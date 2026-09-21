pub mod guard;
pub mod operations;

use std::collections::HashMap;
use std::sync::Arc;

use dbstudio_core::ai::LlmConfig;
use dbstudio_core::models::Environment;
use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::{DatabaseInfo, TableInfo, TableSchema};
use dbstudio_db::Connection;
use dbstudio_storage::types::QueryHistoryEntry;
use gpui::*;

use guard::WriteKind;

pub use dbstudio_storage::types::{ConnectionInfo, ConnectionStatus};
pub use operations::*;

/// A saved favorite query.
#[derive(Debug, Clone)]
pub struct FavoriteEntry {
    pub id: i64,
    pub name: String,
    pub sql: String,
    pub connection_id: Option<String>,
}

/// A single live database connection tab.
///
/// Each session keeps the connection handle plus all connection-scoped state
/// (current database, tables, results, editor buffer) so that multiple
/// connections can be open at once and switched between without disturbing one
/// another.
pub struct ConnectionSession {
    pub id: u64,
    pub name: String,
    /// The stored connection-config id (used to persist query history).
    pub connection_id: Option<String>,
    /// The environment of this connection, used for safe-mode guards.
    pub environment: Environment,
    pub connection: Option<Arc<Connection>>,
    pub connection_state: ConnectionStatus,
    pub active_database: Option<String>,
    pub databases: Vec<DatabaseInfo>,
    pub tables: Vec<TableInfo>,
    pub table_schemas: HashMap<String, TableSchema>,
    pub last_result: Option<Arc<SqlResult>>,
    pub is_executing: bool,
    /// The SQL editor buffer for this session, restored when the tab is switched to.
    pub editor_text: String,
    /// Accumulated data edits waiting for review (INSERT / UPDATE / DELETE rows).
    pub pending_edits: Vec<guard::PendingWrite>,
    /// Applied edits for undo support.
    pub undo_stack: Vec<guard::EditRecord>,
    /// Redone edits for redo support.
    pub redo_stack: Vec<guard::EditRecord>,
}

impl ConnectionSession {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            connection_id: None,
            environment: Environment::Dev,
            connection: None,
            connection_state: ConnectionStatus::Disconnected,
            active_database: None,
            databases: Vec::new(),
            tables: Vec::new(),
            table_schemas: HashMap::new(),
            last_result: None,
            is_executing: false,
            editor_text: String::new(),
            pending_edits: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

/// Window-local UI state: which session this window displays, plus window-only
/// toggles. Multiple windows share the global `sessions` catalog but each keeps
/// its own active session pointer, so two windows can be on different
/// connections/databases without disturbing one another.
#[derive(Debug, Clone)]
pub struct WindowState {
    pub active_session: Option<u64>,
    /// A dangerous query awaiting user confirmation before execution.
    pub pending_dangerous_query: Option<(String, WriteKind)>,
    pub show_tables: bool,
    pub show_history: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            active_session: None,
            pending_dangerous_query: None,
            show_tables: true,
            show_history: false,
        }
    }
}

/// Global application state shared across the UI.
///
/// Connection-scoped state lives in [`ConnectionSession`]s (one shared catalog);
/// the session visible in *each window* is tracked per window id in `windows`.
/// Fields not tied to a connection (saved connections, history, panels, prefs)
/// live here directly.
pub struct AppState {
    /// Every open session, across all windows.
    pub sessions: Vec<ConnectionSession>,
    /// The active session per window (keyed by `WindowId::as_u64()`).
    pub windows: HashMap<u64, WindowState>,
    pub next_session_id: u64,
    pub saved_connections: Vec<ConnectionInfo>,
    pub status_message: String,
    pub query_history: Vec<QueryHistoryEntry>,
    pub next_history_id: i64,
    /// Saved favorite queries.
    pub favorites: Vec<FavoriteEntry>,
    /// Global safe-mode toggle. When enabled, every write statement requires
    /// confirmation before execution regardless of the connection environment.
    pub safe_mode: bool,
    /// Whether Vim-style key handling is enabled for the SQL editor.
    pub vim_mode: bool,
    /// Configured LLM provider used by the AI panel (openai / ollama / mock).
    pub ai_config: LlmConfig,
}

impl AppState {
    /// The session currently displayed in a given window, if any.
    pub fn active_session_for(&self, window_id: u64) -> Option<&ConnectionSession> {
        let id = self.window_state(window_id).active_session?;
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn active_session_mut_for(&mut self, window_id: u64) -> Option<&mut ConnectionSession> {
        let id = self.window_state_mut(window_id).active_session?;
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    pub fn session(&self, id: u64) -> Option<&ConnectionSession> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn session_mut(&mut self, id: u64) -> Option<&mut ConnectionSession> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Window-local state, materializing a default (disconnected) one on first use.
    pub fn window_state(&self, window_id: u64) -> &WindowState {
        self.windows
            .get(&window_id)
            .unwrap_or_else(|| EMPTY_WINDOW.get_or_init(WindowState::default))
    }

    pub fn window_state_mut(&mut self, window_id: u64) -> &mut WindowState {
        self.windows.entry(window_id).or_default()
    }

    /// Set (or clear) the active session pointer for a window.
    pub fn set_active_session(&mut self, window_id: u64, id: Option<u64>) {
        self.window_state_mut(window_id).active_session = id;
    }

    // ---- Proxy accessors to the active session *in a window* ----

    pub fn active_connection_for(&self, window_id: u64) -> Option<&Arc<Connection>> {
        self.active_session_for(window_id)
            .and_then(|s| s.connection.as_ref())
    }

    pub fn connection_state_for(&self, window_id: u64) -> ConnectionStatus {
        self.active_session_for(window_id)
            .map(|s| s.connection_state)
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    pub fn is_connected_for(&self, window_id: u64) -> bool {
        self.connection_state_for(window_id) == ConnectionStatus::Connected
    }

    pub fn active_database_for(&self, window_id: u64) -> Option<&String> {
        self.active_session_for(window_id)
            .and_then(|s| s.active_database.as_ref())
    }

    pub fn databases_for(&self, window_id: u64) -> &[DatabaseInfo] {
        self.active_session_for(window_id)
            .map(|s| s.databases.as_slice())
            .unwrap_or(&[])
    }

    pub fn tables_for(&self, window_id: u64) -> &[TableInfo] {
        self.active_session_for(window_id)
            .map(|s| s.tables.as_slice())
            .unwrap_or(&[])
    }

    pub fn table_schemas_for(&self, window_id: u64) -> &HashMap<String, TableSchema> {
        match self.active_session_for(window_id) {
            Some(s) => &s.table_schemas,
            None => empty_schemas(),
        }
    }

    pub fn last_result_for(&self, window_id: u64) -> Option<&Arc<SqlResult>> {
        self.active_session_for(window_id)
            .and_then(|s| s.last_result.as_ref())
    }

    pub fn is_executing_for(&self, window_id: u64) -> bool {
        self.active_session_for(window_id)
            .map(|s| s.is_executing)
            .unwrap_or(false)
    }

    pub fn active_connection_name_for(&self, window_id: u64) -> Option<&String> {
        self.active_session_for(window_id).map(|s| &s.name)
    }

    pub fn active_connection_id_for(&self, window_id: u64) -> Option<&String> {
        self.active_session_for(window_id)
            .and_then(|s| s.connection_id.as_ref())
    }

    /// Whether any window currently has this session open (used by close logic).
    pub fn is_session_open_in_window(&self, session_id: u64) -> bool {
        self.windows
            .values()
            .any(|w| w.active_session == Some(session_id))
    }
}

static EMPTY_SCHEMAS: std::sync::OnceLock<HashMap<String, TableSchema>> =
    std::sync::OnceLock::new();
static EMPTY_WINDOW: std::sync::OnceLock<WindowState> = std::sync::OnceLock::new();

fn empty_schemas() -> &'static HashMap<String, TableSchema> {
    EMPTY_SCHEMAS.get_or_init(HashMap::new)
}

impl Global for AppState {}

impl AppState {
    pub fn init(cx: &mut App) {
        if cx.has_global::<AppState>() {
            return;
        }
        cx.set_global(AppState {
            sessions: Vec::new(),
            windows: HashMap::new(),
            next_session_id: 1,
            saved_connections: Vec::new(),
            status_message: "Not connected".to_string(),
            query_history: Vec::new(),
            next_history_id: 1,
            favorites: Vec::new(),
            safe_mode: true,
            vim_mode: false,
            ai_config: LlmConfig::default(),
        });

        cx.spawn(async move |cx| {
            match dbstudio_storage::AppStore::singleton().await {
                Ok(store) => {
                    if let Ok(connections) = store.connections().load_all().await {
                        cx.update_global::<AppState, _>(|app_state, _cx| {
                            app_state.saved_connections = connections;
                        });
                    }
                    // Load recent query history across all connections
                    if let Ok(history) = store.history().load_recent(200).await {
                        cx.update_global::<AppState, _>(|app_state, _cx| {
                            app_state.query_history = history;
                            if let Some(max_id) = app_state.query_history.iter().map(|e| e.id).max()
                            {
                                app_state.next_history_id = max_id + 1;
                            }
                        });
                    }
                    let ai_config =
                        dbstudio_storage::ai_settings::load_ai_config(store.pool()).await;
                    let safe_mode = dbstudio_storage::settings::get_setting_bool(
                        store.pool(),
                        "app.safe_mode",
                        true,
                    )
                    .await;
                    let vim_mode = dbstudio_storage::settings::get_setting_bool(
                        store.pool(),
                        "app.vim_mode",
                        false,
                    )
                    .await;
                    cx.update_global::<AppState, _>(|app_state, _cx| {
                        app_state.ai_config = ai_config;
                        app_state.safe_mode = safe_mode;
                        app_state.vim_mode = vim_mode;
                    });
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
