//! 本地 GPU 推理命令（契约 v4）。
//!
//! 命令清单：
//! - `local_gpu_probe(refresh?)` → [`GpuProbeResult`]（真实 `--list-devices` 探测，60s 缓存）
//! - `local_engine_status` / `local_engine_start` / `local_engine_stop` → [`EngineStatusPayload`]
//! - `backends_list` / `backends_download` / `backends_cancel` / `backends_remove`（后端组件按需下载）
//!
//! 说明：`backends_download` 的进度/状态复用 `download:progress` / `download:state` 事件，
//! `model` 字段填后端组件 id（`vulkan` / `cuda` / `cpu`），与模型下载共用前端进度组件。

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::app::state::AppState;
use crate::contract::errors::AppResult;
use crate::contract::events::EngineStatusPayload;
use crate::contract::types::{BackendInfo, DownloadId, GpuProbeResult};

/// 打包后 Tauri `resource_dir` 可能不在 `exe_dir` 之下（如 NSIS/安装版布局），
/// 这里补一个**精确**的随包后端根目录（CPU 后端随主安装包分发）。
fn resource_backend_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("backends"))
}

/// 后端根目录候选（`SNIP_ASK_BACKENDS_DIR` → data_dir → exe_dir → 打包 resource_dir）。
fn backend_roots(app: &AppHandle, state: &AppState) -> Vec<PathBuf> {
    let mut roots = state.backends.search_roots();
    if let Some(root) = resource_backend_root(app) {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// 命令：探测 GPU 与后端可用性（`refresh` 缺省 false，命中 60s 缓存）。
#[tauri::command]
pub async fn local_gpu_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> AppResult<GpuProbeResult> {
    let config = state.snapshot_config();
    let model_vram_mb = state
        .models
        .entry(&config.local.model)
        .map(|entry| entry.vram_mb)
        .unwrap_or(0);
    let backends_roots = backend_roots(&app, &state);
    let cache = state.gpu_probe.clone();
    let force = refresh.unwrap_or(false);
    let result = tauri::async_runtime::spawn_blocking(move || {
        cache.probe_roots(&backends_roots, model_vram_mb, force)
    })
    .await
    .map_err(|err| {
        crate::contract::errors::AppError::io_failed(format!("GPU 探测任务失败：{err}"))
    })?;
    Ok(result)
}

/// 命令：引擎运行态快照。
#[tauri::command]
pub async fn local_engine_status(state: State<'_, AppState>) -> AppResult<EngineStatusPayload> {
    Ok(state.local_engine.snapshot())
}

/// 命令：显式启动本地引擎（按 `local.device` / `local.gpuBackend` 走 Vulkan → CUDA → CPU）。
#[tauri::command]
pub async fn local_engine_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<EngineStatusPayload> {
    let config = state.snapshot_config();
    let engine = state.local_engine.clone();
    let status = engine.start(&app, &config).await?;
    crate::events::emit_engine_status_payload(&app, &status);
    *state.engine.write() = status.local;
    Ok(status)
}

/// 命令：显式停止本地引擎（按 PID/进程树回收）。
#[tauri::command]
pub async fn local_engine_stop(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<EngineStatusPayload> {
    let engine = state.local_engine.clone();
    engine.stop().await?;
    let status = engine.snapshot();
    crate::events::emit_engine_status_payload(&app, &status);
    *state.engine.write() = status.local;
    Ok(status)
}

/// 命令：后端组件清单（Vulkan 默认 GPU 组件 / CUDA 可选 / CPU）。
#[tauri::command]
pub async fn backends_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<BackendInfo>> {
    let roots = backend_roots(&app, &state);
    state.backends.list_with_roots(&roots)
}

/// 命令：下载后端组件（按需，不塞进主安装包）。
#[tauri::command]
pub async fn backends_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<DownloadId> {
    let download = state.backends.download(&app, &id)?;
    // 安装完成后 GPU 可用性变化：清缓存 + 广播引擎状态。
    state.gpu_probe.invalidate();
    crate::events::emit_engine_status_payload(&app, &state.local_engine.snapshot());
    Ok(download)
}

/// 命令：取消后端组件下载。
#[tauri::command]
pub async fn backends_cancel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.backends.cancel(&id)
}

/// 命令：删除已安装的后端组件。
#[tauri::command]
pub async fn backends_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    state.backends.remove(&id)?;
    state.gpu_probe.invalidate();
    crate::events::emit_engine_status_payload(&app, &state.local_engine.snapshot());
    Ok(())
}
