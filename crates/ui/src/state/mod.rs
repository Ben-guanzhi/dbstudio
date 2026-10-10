pub mod guard;
pub mod ledger;
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
#[derive(Debug, Clone, PartialEq)]
pub struct FavoriteEntry {
    pub id: i64,
    pub name: String,
    pub sql: String,
    pub connection_id: Option<String>,
}

/// Which view of the currently-open database object the center pane shows.
///
/// Mirrors the TablePlus-style object tabs (Data / Structure / Query /
/// History). Stored per session so each connection tab remembers where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectSegment {
    #[default]
    Data,
    Structure,
    Query,
    History,
}

impl ObjectSegment {
    pub const ALL: [ObjectSegment; 4] = [
        ObjectSegment::Data,
        ObjectSegment::Structure,
        ObjectSegment::Query,
        ObjectSegment::History,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ObjectSegment::Data => "Data",
            ObjectSegment::Structure => "Structure",
            ObjectSegment::Query => "Query",
            ObjectSegment::History => "History",
        }
    }
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
    /// The table list, behind a handle so observers can tell "the schema
    /// changed" from "some other field of `AppState` was written" in O(1).
    ///
    /// Every mutable access to `AppState` notifies *all* its observers, and two
    /// of them mirror this list (the SQL editor's completion source, the tables
    /// tree). Sharing one `Arc<Vec<_>>` instead of re-cloning it per observer
    /// also means a writer can no longer mutate the list in place without
    /// changing its address: while an observer holds a clone the refcount is
    /// above one, so `Arc::make_mut` copy-on-writes onto a fresh address.
    pub tables: Arc<Vec<TableInfo>>,
    pub table_schemas: HashMap<String, TableSchema>,
    pub last_result: Option<Arc<SqlResult>>,
    pub is_executing: bool,
    /// The SQL editor buffer for this session, restored when the tab is switched to.
    pub editor_text: String,
    /// Accumulated data edits, tracked across their whole lifecycle
    /// (pending → applied/undoable → reverted/redoable) by a single ledger.
    pub edits: ledger::EditLedger,
    /// Which object segment (Data/Structure/Query/History) the center pane shows.
    pub active_object: ObjectSegment,
    /// The table currently open in the object tabs: (name, schema).
    pub current_object: Option<(String, Option<String>)>,
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
            tables: Arc::new(Vec::new()),
            table_schemas: HashMap::new(),
            last_result: None,
            is_executing: false,
            editor_text: String::new(),
            edits: ledger::EditLedger::new(),
            active_object: ObjectSegment::default(),
            current_object: None,
        }
    }
}

/// A window-level interaction awaiting a user decision. Replaces the old bare
/// `Option<(String, WriteKind)>`: the payload is named, and `None` is an
/// explicit variant so "nothing pending" and "a query awaits confirmation"
/// are the only two representable states.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Interaction {
    #[default]
    Nothing,
    /// A dangerous query awaiting confirmation before execution.
    ConfirmQuery { sql: String, kind: WriteKind },
}

impl Interaction {
    pub fn is_pending(&self) -> bool {
        matches!(self, Interaction::ConfirmQuery { .. })
    }
}

/// Window-local UI state: which session this window displays, plus window-only
/// toggles. Multiple windows share the global `sessions` catalog but each keeps
/// its own active session pointer, so two windows can be on different
/// connections/databases without disturbing one another.
#[derive(Debug, Clone, Default)]
pub struct WindowState {
    pub active_session: Option<u64>,
    pub interaction: Interaction,
    pub show_tables: bool,
    pub show_history: bool,
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
        self.tables_arc_for(window_id).as_slice()
    }

    /// The table list as a shared handle. Observers that only need to know
    /// *whether it changed* compare this with [`Arc::ptr_eq`] instead of
    /// re-scanning every table on every `AppState` notification.
    pub fn tables_arc_for(&self, window_id: u64) -> &Arc<Vec<TableInfo>> {
        match self.active_session_for(window_id) {
            Some(s) => &s.tables,
            None => empty_tables(),
        }
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

    /// The object segment the center pane of `window_id` is showing.
    pub fn active_object_for(&self, window_id: u64) -> ObjectSegment {
        self.active_session_for(window_id)
            .map(|s| s.active_object)
            .unwrap_or_default()
    }

    /// The table currently open in `window_id`'s object tabs, if any.
    pub fn current_object_for(&self, window_id: u64) -> Option<(String, Option<String>)> {
        self.active_session_for(window_id)
            .and_then(|s| s.current_object.clone())
    }
}

static EMPTY_SCHEMAS: std::sync::OnceLock<HashMap<String, TableSchema>> =
    std::sync::OnceLock::new();
static EMPTY_WINDOW: std::sync::OnceLock<WindowState> = std::sync::OnceLock::new();
static EMPTY_TABLES: std::sync::OnceLock<Arc<Vec<TableInfo>>> = std::sync::OnceLock::new();

fn empty_schemas() -> &'static HashMap<String, TableSchema> {
    EMPTY_SCHEMAS.get_or_init(HashMap::new)
}

/// One shared empty list, so a window with no session has a stable address for
/// its observers to compare against instead of a fresh allocation per call.
fn empty_tables() -> &'static Arc<Vec<TableInfo>> {
    EMPTY_TABLES.get_or_init(|| Arc::new(Vec::new()))
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

#[cfg(test)]
mod tests {
    use super::{empty_tables, ConnectionSession};
    use dbstudio_core::schema::{TableInfo, TableType};
    use std::sync::Arc;

    fn table(name: &str) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            schema: Some("public".to_string()),
            database: Some("app".to_string()),
            table_type: TableType::Table,
            row_count: Some(1),
            comment: None,
        }
    }

    /// The premise the observers' `Arc::ptr_eq` fast path rests on: as long as a
    /// consumer holds the list the refcount is above one, so a writer that
    /// mutates in place through `Arc::make_mut` is pushed onto a *new* address
    /// and the consumer's own view is left untouched.
    #[test]
    fn a_shared_table_list_copy_on_writes_onto_a_new_address() {
        let mut tables = Arc::new(vec![table("orders")]);
        let mirror = Arc::clone(&tables);
        let before = Arc::as_ptr(&tables);

        Arc::make_mut(&mut tables).push(table("users"));

        assert_ne!(Arc::as_ptr(&tables), before, "make_mut reused the address");
        assert_ne!(Arc::as_ptr(&tables), Arc::as_ptr(&mirror));
        assert_eq!(
            mirror.len(),
            1,
            "the mirrored list was mutated through the handle"
        );
        assert_eq!(tables.len(), 2);
    }

    /// The other half of the same argument, kept honest: in-place mutation *is*
    /// possible while the refcount is one. That is only reachable when no
    /// observer mirrors this list, and then the observer's handle is a different
    /// allocation anyway - so the pointer probe says "look", and content still
    /// decides. A pointer fast path can therefore skip work, never a change.
    #[test]
    fn an_unshared_table_list_mutates_in_place() {
        let mut tables = Arc::new(vec![table("orders")]);
        let before = Arc::as_ptr(&tables);

        Arc::make_mut(&mut tables).push(table("users"));

        assert_eq!(Arc::as_ptr(&tables), before);
        assert_eq!(tables.len(), 2);
    }

    #[test]
    fn a_session_starts_on_its_own_empty_handle() {
        let session = ConnectionSession::new(1, "main");

        assert!(session.tables.is_empty());
        // Distinct from the shared no-session handle, so an observer that
        // settled on the empty one re-checks content when a session appears.
        assert_ne!(Arc::as_ptr(&session.tables), Arc::as_ptr(empty_tables()));
    }

    #[test]
    fn the_shared_empty_handle_does_not_move() {
        assert!(empty_tables().is_empty());
        assert_eq!(Arc::as_ptr(empty_tables()), Arc::as_ptr(empty_tables()));
    }
}
