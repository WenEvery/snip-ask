//! ID 生成与解析（UUID v4 小写字符串，契约 §1.4）。

use uuid::Uuid;

use crate::contract::types::{DownloadId, SessionId, TaskId};

/// 生成新的会话 ID。
pub fn new_session_id() -> SessionId {
    SessionId(Uuid::new_v4().to_string())
}

/// 生成新的任务 ID。
pub fn new_task_id() -> TaskId {
    TaskId(Uuid::new_v4().to_string())
}

/// 生成新的下载 ID。
pub fn new_download_id() -> DownloadId {
    DownloadId(Uuid::new_v4().to_string())
}

/// 判断字符串是否为合法的 UUID v4 小写形式。
pub fn is_valid_uuid(value: &str) -> bool {
    match Uuid::parse_str(value) {
        Ok(uuid) => uuid.get_version_num() == 4 && uuid.to_string() == value,
        Err(_) => false,
    }
}

impl SessionId {
    /// 生成新的会话 ID。
    pub fn new_v4() -> Self {
        new_session_id()
    }
}

impl TaskId {
    /// 生成新的任务 ID。
    pub fn new_v4() -> Self {
        new_task_id()
    }
}

impl DownloadId {
    /// 生成新的下载 ID。
    pub fn new_v4() -> Self {
        new_download_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_uuid_v4_lowercase() {
        let session = new_session_id();
        assert_eq!(session.as_str().len(), 36);
        assert_eq!(session.as_str(), session.as_str().to_lowercase());
        assert!(is_valid_uuid(session.as_str()));

        let task = TaskId::new_v4();
        assert!(is_valid_uuid(task.as_str()));

        let download = DownloadId::new_v4();
        assert!(is_valid_uuid(download.as_str()));
    }

    #[test]
    fn invalid_uuid_is_rejected() {
        assert!(!is_valid_uuid("not-a-uuid"));
        assert!(!is_valid_uuid(""));
        assert!(!is_valid_uuid("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
    }
}
