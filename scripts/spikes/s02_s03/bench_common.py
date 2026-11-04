# -*- coding: utf-8 -*-
"""S-02/S-03 OCR 时延基准 —— 公共工具（环境采集 / 统计 / 独占锁 / 输出）。

设计原则：
- 只做测量，不在测量期间写大文件/装包/编译；
- 所有统计只用 numpy，P50/P95 采用 numpy.percentile 默认线性插值；
- 独占锁 `.verify/bench.lock` 用 O_CREAT|O_EXCL 创建，内容 owner/pid/startedAt，
  遵循团队约定：已存在且 <40 分钟且 pid 存活 → 等待重试；否则接管。
"""
from __future__ import annotations

import ctypes
import csv
import json
import os
import platform
import time
import winreg
from contextlib import contextmanager
from ctypes import wintypes
from pathlib import Path
from typing import Any, Dict, List

import numpy as np

REPO = Path(__file__).resolve().parents[3]
SPIKE_DIR = REPO / ".verify" / "spikes" / "S-02-S-03"
INPUTS_DIR = SPIKE_DIR / "inputs"
RESULTS_DIR = SPIKE_DIR / "results"
LOCK_PATH = REPO / ".verify" / "bench.lock"
SUMMARY_DIR = REPO / "docs" / "spikes" / "data" / "s02_s03"

LOCK_MAX_AGE_S = 40 * 60  # 超过 40 分钟视为陈旧锁，可接管
LOCK_RETRY_S = 30


# ---------------------------------------------------------------- 统计
def summarize(vals: List[float]) -> Dict[str, float]:
    a = np.asarray(vals, dtype=np.float64)
    return {
        "n": int(a.size),
        "mean": round(float(a.mean()), 3),
        "min": round(float(a.min()), 3),
        "p50": round(float(np.percentile(a, 50)), 3),
        "p95": round(float(np.percentile(a, 95)), 3),
        "p99": round(float(np.percentile(a, 99)), 3),
        "max": round(float(a.max()), 3),
        "std": round(float(a.std(ddof=1)) if a.size > 1 else 0.0, 3),
    }


def save_json(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, ensure_ascii=False, indent=2), encoding="utf-8")


def save_csv(path: Path, rows: List[Dict[str, Any]], fields: List[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8-sig") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow(r)


# ---------------------------------------------------------------- 环境
class MEMORYSTATUSEX(ctypes.Structure):
    _fields_ = [
        ("dwLength", wintypes.DWORD),
        ("dwMemoryLoad", wintypes.DWORD),
        ("ullTotalPhys", ctypes.c_ulonglong),
        ("ullAvailPhys", ctypes.c_ulonglong),
        ("ullTotalPageFile", ctypes.c_ulonglong),
        ("ullAvailPageFile", ctypes.c_ulonglong),
        ("ullTotalVirtual", ctypes.c_ulonglong),
        ("ullAvailVirtual", ctypes.c_ulonglong),
        ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
    ]


def _reg(path: str, name: str) -> str | None:
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path) as k:
            return str(winreg.QueryValueEx(k, name)[0])
    except OSError:
        return None


def env_info() -> Dict[str, Any]:
    mem = MEMORYSTATUSEX()
    mem.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(mem))
    cpu = _reg(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString")
    build = _reg(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
    display = _reg(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "DisplayVersion")
    product = _reg(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "ProductName")
    return {
        "os": f"{product} {display} build {build}",
        "platform": platform.platform(),
        "cpu": (cpu or "").strip(),
        "cpu_logical": os.cpu_count(),
        "ram_total_gib": round(mem.ullTotalPhys / 1024**3, 2),
        "ram_avail_gib_at_start": round(mem.ullAvailPhys / 1024**3, 2),
        "python": platform.python_version(),
        "python_impl": platform.python_implementation(),
        "timestamp_local": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }


def ort_info() -> Dict[str, Any]:
    import onnxruntime

    return {
        "onnxruntime": onnxruntime.__version__,
        "providers": onnxruntime.get_available_providers(),
    }


# ---------------------------------------------------------------- 进程存活 / 独占锁
_PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
_STILL_ACTIVE = 259
_k32 = ctypes.WinDLL("kernel32", use_last_error=True)


def pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    h = _k32.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return False
    try:
        code = wintypes.DWORD()
        ok = _k32.GetExitCodeProcess(h, ctypes.byref(code))
        return bool(ok) and code.value == _STILL_ACTIVE
    finally:
        _k32.CloseHandle(h)


def _read_lock() -> Dict[str, Any] | None:
    """读取 bench.lock：兼容团队两种写法（JSON 对象 / key=value 行），并给出 startedAt_epoch。"""
    try:
        raw = LOCK_PATH.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return None
    info: Dict[str, Any] = {}
    if raw.lstrip().startswith("{"):
        try:
            info = json.loads(raw)
        except json.JSONDecodeError:
            info = {}
    if not info:
        for line in raw.splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                info[k.strip()] = v.strip()
    if not isinstance(info, dict):
        info = {}
    try:
        info["pid"] = int(info.get("pid", 0))
    except (TypeError, ValueError):
        info["pid"] = 0
    # startedAt_epoch：优先显式字段，其次 ISO 时间串，最后退回文件 mtime
    epoch = info.get("startedAt_epoch")
    if epoch is None and info.get("startedAt"):
        try:
            from datetime import datetime

            dt = datetime.fromisoformat(str(info["startedAt"]))
            epoch = dt.timestamp()
        except ValueError:
            epoch = None
    if epoch is None:
        try:
            epoch = LOCK_PATH.stat().st_mtime
        except OSError:
            epoch = 0.0
    try:
        info["startedAt_epoch"] = float(epoch)
    except (TypeError, ValueError):
        info["startedAt_epoch"] = 0.0
    return info


@contextmanager
def bench_lock(owner: str = "ocr-bench", wait: bool = True, timeout_s: int = 900):
    """独占创建 .verify/bench.lock；必要时等待或接管。"""
    LOCK_PATH.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + timeout_s
    waited = 0.0
    while True:
        try:
            fd = os.open(str(LOCK_PATH), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            info = _read_lock()
            now = time.time()
            age = now - info.get("startedAt_epoch", 0.0) if info else LOCK_MAX_AGE_S + 1
            holder = info.get("owner", "?") if info else "?"
            alive = pid_alive(info.get("pid", 0)) if info else False
            if (not alive) or age > LOCK_MAX_AGE_S:
                print(f"[lock] 接管陈旧锁（holder={holder} age={age/60:.1f}min alive={alive}）")
                LOCK_PATH.unlink(missing_ok=True)
                continue
            if not wait:
                raise RuntimeError(
                    f"bench.lock 被占用：holder={holder} pid={info.get('pid')} "
                    f"age={age/60:.1f}min，本次先跳过正式测量"
                )
            if time.monotonic() > deadline:
                raise RuntimeError(f"等待 bench.lock 超时（{timeout_s}s），holder={holder}")
            print(f"[lock] 被 {holder} 占用（age={age/60:.1f}min），{LOCK_RETRY_S}s 后重试…")
            time.sleep(LOCK_RETRY_S)
            waited += LOCK_RETRY_S
        else:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(
                    f"owner={owner}\npid={os.getpid()}\n"
                    f"startedAt={time.strftime('%Y-%m-%dT%H:%M:%S%z')}\n"
                    f"startedAt_epoch={time.time():.3f}\n"
                )
            print(f"[lock] 已获取 bench.lock（owner={owner} pid={os.getpid()}，等待 {waited:.0f}s）")
            break
    try:
        yield
    finally:
        info = _read_lock()
        if info and info.get("pid") == os.getpid():
            LOCK_PATH.unlink(missing_ok=True)
            print("[lock] 已释放 bench.lock")
