pub mod operations;

use std::collections::HashMap;
use std::sync::Arc;

use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::{DatabaseInfo, TableInfo, TableSchema};
use dbstudio_db::Connection;
use dbstudio_storage::types::QueryHistoryEntry;
use gpui::*;

pub use dbstudio_storage::types::{ConnectionInfo, ConnectionStatus};
pub use operations::*;

/// Global application state shared across the UI.
pub struct AppState {
    pub saved_connections: Vec<ConnectionInfo>,
    pub active_connection: Option<Arc<Connection>>,
    pub connection_state: ConnectionStatus,
    pub status_message: String,
    pub active_connection_name: Option<String>,
    pub active_connection_id: Option<String>,
    pub active_database: Option<String>,
    pub databases: Vec<DatabaseInfo>,
    pub tables: Vec<TableInfo>,
    pub table_schemas: HashMap<String, TableSchema>,
    pub last_result: Option<Arc<SqlResult>>,
    pub is_executing: bool,
    pub query_history: Vec<QueryHistoryEntry>,
    pub next_history_id: i64,
    pub show_tables: bool,
    pub show_history: bool,
}

impl Global for AppState {}

impl AppState {
    pub fn init(cx: &mut App) {
        cx.set_global(AppState {
            saved_connections: Vec::new(),
            active_connection: None,
            connection_state: ConnectionStatus::Disconnected,
            status_message: "Not connected".to_string(),
            active_connection_name: None,
            active_connection_id: None,
            active_database: None,
            databases: Vec::new(),
            tables: Vec::new(),
            table_schemas: HashMap::new(),
            last_result: None,
            is_executing: false,
            query_history: Vec::new(),
            next_history_id: 1,
            show_tables: true,
            show_history: false,
        });

        cx.spawn(async move |cx| {
            match dbstudio_storage::AppStore::singleton().await {
                Ok(store) => {
                    if let Ok(connections) = store.connections().load_all().await {
                        cx
                            .update_global::<AppState, _>(|app_state, _cx| {
                                app_state.saved_connections = connections;
                            });
                    }
                    // Load recent query history across all connections
                    if let Ok(history) = store.history().load_recent(200).await {
                        cx.update_global::<AppState, _>(|app_state, _cx| {
                            app_state.query_history = history;
                            if let Some(max_id) = app_state.query_history.iter().map(|e| e.id).max() {
                                app_state.next_history_id = max_id + 1;
                            }
                        });
                    }
                }
                Err(e) => tracing::error!("Failed to init storage: {}", e),
            }
        })
        .detach();
    }

    pub fn update_status(cx: &mut App, message: impl Into<String>) {
        cx.update_global::<AppState, _>(|app_state, _cx| {
            app_state.status_message = message.into();
        });
    }
}

/// Initialize all global UI state.
pub fn init(cx: &mut App) {
    AppState::init(cx);
}