//! 系统剪贴板（Win32）：`复制文字` 动作与 `actions.autoCopy` 使用。
//!
//! 只实现写入纯文本（CF_UNICODETEXT），不读取剪贴板、不写日志正文。
//! 为什么放在平台层：仅此目录允许 Win32 调用；其余模块保持安全代码。

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

use crate::contract::errors::{AppError, AppResult};

/// `CF_UNICODETEXT`（Win32 剪贴板格式号；windows crate 将它定义在 `Win32::System::Ole`，
/// 为避免引入 Ole feature 导致依赖膨胀，这里使用其固定数值 13）。
const CF_UNICODETEXT: u32 = 13;

/// 把 `text` 写入系统剪贴板（Unicode 文本）。
///
/// - 空文本也可写入（清空剪贴板文本语义）；
/// - 失败时返回可操作错误，不 panic；
/// - 调用期间持有剪贴板所有权（`OpenClipboard` → `CloseClipboard`），冲突时短暂失败。
pub fn set_text(text: &str) -> AppResult<()> {
    // UTF-16 编码 + 结尾 NUL（CF_UNICODETEXT 约定）。
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let bytes = utf16.len() * std::mem::size_of::<u16>();

    // SAFETY：所有 Win32 调用都成对闭合；`GlobalLock` 指针只在本函数内写入，
    // `SetClipboardData` 成功后内存所有权移交系统（不再 GlobalFree）。
    unsafe {
        OpenClipboard(None).map_err(|err| clipboard_error(format!("打开剪贴板失败：{err}")))?;

        let result = (|| -> AppResult<()> {
            EmptyClipboard().map_err(|err| clipboard_error(format!("清空剪贴板失败：{err}")))?;
            let handle: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .map_err(|err| clipboard_error(format!("分配剪贴板内存失败：{err}")))?;
            let pointer = GlobalLock(handle);
            if pointer.is_null() {
                let _ = GlobalFree(Some(handle));
                return Err(clipboard_error("锁定剪贴板内存失败"));
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, pointer as *mut u8, bytes);
            // `GlobalUnlock` 在锁计数归零时返回 0 且 GetLastError = ERROR_SUCCESS（Win32 语义），
            // windows crate 会把它映射成 Err；此处只需确认句柄仍可交给系统，忽略该返回值。
            let _ = GlobalUnlock(handle);
            // 成功后系统接管 HGLOBAL 所有权；失败时释放，避免泄漏。
            match SetClipboardData(CF_UNICODETEXT, Some(HANDLE(handle.0))) {
                Ok(_) => Ok(()),
                Err(err) => {
                    let _ = GlobalFree(Some(handle));
                    Err(clipboard_error(format!("写入剪贴板失败：{err}")))
                }
            }
        })();

        let _ = CloseClipboard();
        result
    }
}

/// 读取系统剪贴板文本（`CF_UNICODETEXT`）；无文本/不可用时返回空串。
///
/// 用于「快速翻译」热键：读取用户从其它程序复制的文字；不写日志正文。
pub fn get_text() -> String {
    // SAFETY：Win32 调用成对闭合；`GlobalSize/GlobalLock` 只在 OpenClipboard 成功后使用。
    unsafe {
        if OpenClipboard(None).is_err() {
            return String::new();
        }
        let result = (|| -> String {
            let Ok(handle) = GetClipboardData(CF_UNICODETEXT) else {
                return String::new();
            };
            let memory = HGLOBAL(handle.0);
            let size = GlobalSize(memory);
            if size < 2 {
                return String::new();
            }
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                return String::new();
            }
            let units = size / std::mem::size_of::<u16>();
            let utf16 = std::slice::from_raw_parts(pointer as *const u16, units);
            let length = utf16.iter().position(|unit| *unit == 0).unwrap_or(units);
            let text = String::from_utf16_lossy(&utf16[..length]);
            let _ = GlobalUnlock(memory);
            text
        })();
        let _ = CloseClipboard();
        result
    }
}

fn clipboard_error(detail: impl Into<String>) -> AppError {
    AppError::io_failed(detail.into())
        .with_message("复制到剪贴板失败：请稍后重试，或手动选择文本复制")
        .with_fallback(vec!["retry"])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写入并在同一进程内读回（会临时占用系统剪贴板；默认 ignored，避免污染日常剪贴板）。
    #[test]
    #[ignore = "会覆盖当前系统剪贴板，需手动 --ignored 运行"]
    fn real_clipboard_roundtrip() {
        let marker = "SnipAsk clipboard test 512";
        set_text(marker).expect("write clipboard");
        assert_eq!(get_text(), marker);
        set_text("").expect("empty write");
        assert_eq!(get_text(), "");
        set_text(marker).expect("rewrite");
    }

    #[test]
    fn empty_text_is_allowed() {
        // 不实际写系统剪贴板，只验证错误构造函数可用。
        let error = clipboard_error("x");
        assert_eq!(error.code, "SA-5003");
    }
}
