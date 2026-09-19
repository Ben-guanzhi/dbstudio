use serde::{Deserialize, Serialize};

pub use dbstudio_core::models::ConnectionConfig as ConnectionInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryHistoryEntry {
    pub id: i64,
    pub connection_id: String,
    pub sql: String,
    pub execution_time_ms: u128,
    pub row_count: Option<usize>,
    pub is_error: bool,
    pub executed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}
