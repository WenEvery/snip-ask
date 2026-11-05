//! 模型清单解析与安装状态合并（说明书 M12 / 契约 §10）。
//!
//! 纯逻辑：清单解析、静态条目 + 本地安装状态（`.installed` 标记）合并为 `ModelInfo`。
//! 文件系统扫描在 `services::model_manager` 中完成。

use std::collections::HashMap;

use crate::contract::errors::AppResult;
use crate::contract::types::{
    DownloadState, ModelInfo, ModelKind, ModelManifest, ModelManifestEntry,
};

/// 解析内置模型清单 JSON。
pub fn parse_manifest(json: &str) -> AppResult<ModelManifest> {
    Ok(serde_json::from_str(json)?)
}

/// 由清单条目 + 运行时状态构造 `ModelInfo`。
pub fn entry_to_info(
    entry: &ModelManifestEntry,
    installed_bytes: Option<u64>,
    download_state: DownloadState,
) -> ModelInfo {
    let installed = installed_bytes.is_some();
    ModelInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: entry.kind.unwrap_or(ModelKind::Gguf),
        size_bytes: entry.size_bytes,
        sha256: entry.sha256.clone(),
        license: entry.license.clone(),
        min_ram_mb: entry.min_ram_mb,
        vram_mb: entry.vram_mb,
        urls: entry.urls.clone(),
        params: entry.params.clone(),
        installed,
        installed_bytes: installed_bytes.unwrap_or(0),
        download_state: match download_state {
            // 已安装：无论状态是 idle/queued 都标记为已安装。
            DownloadState::Idle | DownloadState::Queued if installed => DownloadState::Installed,
            other => other,
        },
    }
}

/// 合并已安装状态：`installed` 为 `<models_dir>/<id>/.installed` 扫描结果（id → 字节数）。
pub fn merge_installed(
    manifest: &ModelManifest,
    installed: &HashMap<String, u64>,
    states: &HashMap<String, DownloadState>,
) -> Vec<ModelInfo> {
    manifest
        .models
        .iter()
        .map(|entry| {
            let download_state = states
                .get(&entry.id)
                .copied()
                .unwrap_or(DownloadState::Idle);
            entry_to_info(entry, installed.get(&entry.id).copied(), download_state)
        })
        .collect()
}

/// 按 id 查清单条目。
pub fn find_entry<'a>(manifest: &'a ModelManifest, id: &str) -> Option<&'a ModelManifestEntry> {
    manifest.models.iter().find(|entry| entry.id == id)
}

/// 已安装模型数量。
pub fn installed_count(infos: &[ModelInfo]) -> u32 {
    infos.iter().filter(|info| info.installed).count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "version": 1,
        "verified": false,
        "notes": "测试清单",
        "models": [
            {
                "id": "hy-mt-1.8b-q4",
                "name": "HY-MT 1.8B",
                "kind": "gguf",
                "sizeBytes": 1258291200,
                "sha256": "0000",
                "license": "Tencent Hunyuan Community License",
                "minRamMB": 4096,
                "urls": ["https://example.com/a.gguf"],
                "params": { "quant": "Q4_K_M", "context": 8192 }
            },
            {
                "id": "opus-mt-zh-en",
                "name": "OPUS-MT",
                "kind": "ct2",
                "sizeBytes": 314572800,
                "sha256": "0000",
                "license": "CC-BY-4.0",
                "minRamMB": 2048,
                "urls": [],
                "params": {}
            }
        ]
    }"#;

    #[test]
    fn parses_manifest_and_ignores_unknown_fields() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.models.len(), 2);
        assert_eq!(manifest.models[0].id, "hy-mt-1.8b-q4");
        assert_eq!(manifest.models[1].kind, Some(ModelKind::Ct2));
        assert_eq!(manifest.models[0].params["quant"], "Q4_K_M");
    }

    #[test]
    fn merges_installed_state() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        let mut installed = HashMap::new();
        installed.insert("hy-mt-1.8b-q4".to_string(), 1_258_291_200);
        let infos = merge_installed(&manifest, &installed, &HashMap::new());
        assert!(infos[0].installed);
        assert_eq!(infos[0].installed_bytes, 1_258_291_200);
        assert_eq!(infos[0].download_state, DownloadState::Installed);
        assert!(!infos[1].installed);
        assert_eq!(infos[1].download_state, DownloadState::Idle);
        assert_eq!(installed_count(&infos), 1);
        // 契约 v4：清单的 vramMB 透传到 ModelInfo（缺省 0，表示不作为 GPU 预算约束）。
        assert_eq!(infos[0].vram_mb, 0);
    }

    #[test]
    fn explicit_download_state_wins_over_queued() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        let mut states = HashMap::new();
        states.insert("hy-mt-1.8b-q4".to_string(), DownloadState::Failed);
        let infos = merge_installed(&manifest, &HashMap::new(), &states);
        assert_eq!(infos[0].download_state, DownloadState::Failed);
        assert!(!infos[0].installed);
    }

    #[test]
    fn find_entry_by_id() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        assert!(find_entry(&manifest, "opus-mt-zh-en").is_some());
        assert!(find_entry(&manifest, "missing").is_none());
    }
}
