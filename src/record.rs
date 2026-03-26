use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub id: Option<i64>,
    pub timestamp_unix: i64,
    pub timestamp_iso: String,
    pub command: String,
    pub cwd: String,
    pub exit_code: i64,
    pub duration_ms: i64,
    pub shell: String,
    pub user: String,
    pub hostname: String,
    pub session_id: String,
}

impl CommandRecord {
    pub fn new(
        command: String,
        cwd: String,
        exit_code: i64,
        duration_ms: i64,
        shell: String,
        user: String,
        hostname: String,
        session_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: None,
            timestamp_unix: now.timestamp(),
            timestamp_iso: now.to_rfc3339(),
            command,
            cwd,
            exit_code,
            duration_ms,
            shell,
            user,
            hostname,
            session_id,
        }
    }
}
