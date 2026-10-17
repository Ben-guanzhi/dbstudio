use std::sync::Arc;

use dbstudio_core::ai::LlmConfig;
use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_core::result::{AppendMeta, QueryResult, SqlResult, MAX_RESULT_ROWS};
use dbstudio_core::schema::{DatabaseInfo, TableSchema};
use dbstudio_db::Connection;
use dbstudio_storage::types::{ConnectionInfo, ConnectionStatus, QueryHistoryEntry};
use dbstudio_storage::AppStore;
use gpui::*;

use super::guard::{self, classify_sql, requires_confirmation, WriteKind};
use super::{AppState, ConnectionSession, Interaction, ObjectSegment};

/// Create a new empty session and make it the active one in `window_id`.
/// Returns its id.
pub fn new_session(name: &str, window_id: u64, cx: &mut App) -> u64 {
    let mut id = 0u64;
    cx.update_global::<AppState, _>(|state, _cx| {
        id = state.next_session_id;
        state.next_session_id += 1;
        state.sessions.insert(0, ConnectionSession::new(id, name));
        state.set_active_session(window_id, Some(id));
    });
    id
}

/// Switch the active session in `window_id` to `id`, restoring its editor buffer.
pub fn switch_session(id: u64, window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        if state.session(id).is_some() {
            state.set_active_session(window_id, Some(id));
        }
    });
}

/// Close a session, dropping its connection (and releasing its SSH tunnel).
/// Removes it from the tab list; every window that pointed at it gets a
/// neighbouring tab instead (or none if the catalog is now empty).
pub fn close_session(id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        let Some(pos) = state.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        state.sessions.remove(pos);

        if state.sessions.is_empty() {
            for win in state.windows.values_mut() {
                win.active_session = None;
            }
            state.status_message = "Disconnected".to_string();
            return;
        }

        // Activate the tab that took the removed one's place, or the last one.
        let new_pos = pos.min(state.sessions.len() - 1);
        let next = state.sessions[new_pos].id;
        for win in state.windows.values_mut() {
            if win.active_session == Some(id) {
                win.active_session = Some(next);
            }
        }
        state.status_message = String::new();
    });
}

/// Sync the editor buffer for the active session of `window_id` (called whenever the editor changes).
pub fn set_active_editor_text(text: &str, window_id: u64, cx: &mut App) {
    update_active_session(window_id, cx, |session, _cx| {
        session.editor_text = text.to_string();
    });
}

/// Run `f` against the active session of `window_id` (if any), then notifies observers.
fn update_active_session<F>(window_id: u64, cx: &mut App, f: F)
where
    F: FnOnce(&mut ConnectionSession, &mut App),
{
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(session) = state.active_session_mut_for(window_id) {
            f(session, _cx);
        }
    });
}

/// Initiate a connection asynchronously in `window_id`. Updates `AppState` along the way.
pub fn connect(conn_info: &ConnectionInfo, window_id: u64, cx: &mut App) {
    let info = conn_info.clone();
    let id = new_session(&info.name, window_id, cx);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.session_mut(id) {
            s.connection_state = ConnectionStatus::Connecting;
        }
        state.status_message = format!("Connecting to {}...", info.name);
    });

    cx.spawn(async move |cx| connect_async(id, info, cx).await)
        .detach();
}

/// Disconnect the active session of `window_id` (keeps the tab open, ready to reconnect).
pub fn disconnect(window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut_for(window_id) {
            s.connection_state = ConnectionStatus::Disconnecting;
        }
    });

    cx.spawn(async move |cx| {
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.active_session_mut_for(window_id) {
                s.connection = None;
                s.connection_state = ConnectionStatus::Disconnected;
                s.active_database = None;
                s.databases.clear();
                s.tables = Arc::new(Vec::new());
                s.table_schemas.clear();
                s.last_result = None;
                s.is_executing = false;
            }
            state.status_message = "Disconnected".to_string();
        });
    })
    .detach();
}

/// Connection context resolved once per user action and shared by the
/// execution helpers below.
struct ExecContext {
    conn: Arc<Connection>,
    conn_id: String,
    session_id: Option<u64>,
}

impl ExecContext {
    fn for_window(window_id: u64, cx: &App) -> Option<Self> {
        let state = cx.global::<AppState>();
        Some(Self {
            conn: state.active_connection_for(window_id)?.clone(),
            conn_id: state
                .active_connection_id_for(window_id)
                .cloned()
                .unwrap_or_default(),
            session_id: state.active_session_for(window_id).map(|s| s.id),
        })
    }
}

/// Shared execution backend for the guarded and raw paths: flips the session
/// into the executing state, runs the SQL, lands the result on the session,
/// and records history. The two callers only differ in their pre-execution
/// guard.
fn spawn_execution(exec: ExecContext, sql: String, cx: &mut App) {
    let ExecContext {
        conn,
        conn_id,
        session_id,
    } = exec;

    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(sid) = session_id {
            if let Some(s) = state.session_mut(sid) {
                s.is_executing = true;
                s.last_result = None;
            }
        }
        state.status_message = "Executing...".to_string();
    });

    cx.spawn(async move |cx| {
        let result = conn.execute(&sql).await;
        let mut exec_time = 0u128;
        let mut row_count = None;
        let mut is_error = false;

        let sql_clone = sql.clone();
        let row_count_for_record = cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(sid) = session_id {
                if let Some(s) = state.session_mut(sid) {
                    s.is_executing = false;
                }
            }
            match result {
                // The outcome is only ever *stored*: the session takes
                // ownership of the grid, so a landed query costs no deep copy
                // of up to 10 000 rows on its way behind the `Arc`.
                Ok(result) => {
                    let (exec_ms, rc, err) = match &result {
                        SqlResult::Query(q) => (q.execution_time_ms(), Some(q.row_count()), false),
                        SqlResult::Modified(m) => (m.execution_time_ms, None, false),
                        SqlResult::Error(_) => (0, None, true),
                    };
                    if let Some(sid) = session_id {
                        if let Some(s) = state.session_mut(sid) {
                            s.last_result = Some(Arc::new(result));
                        }
                    }
                    record_history(state, session_id, &sql_clone, exec_ms, rc, err);
                    state.status_message = if err {
                        "Query failed".to_string()
                    } else {
                        "Query executed".to_string()
                    };
                    exec_time = exec_ms;
                    row_count = rc;
                    is_error = err;
                    rc
                }
                Err(e) => {
                    record_history(state, session_id, &sql_clone, 0, None, true);
                    is_error = true;
                    let lost = e.is_connection_lost();
                    state.status_message = if lost {
                        format!("Connection lost: {}", e)
                    } else {
                        format!("Query failed: {}", e)
                    };
                    if let Some(sid) = session_id {
                        if let Some(s) = state.session_mut(sid) {
                            if lost {
                                // The handle is dead: drop it and mark the session
                                // disconnected so the UI offers a reconnect instead
                                // of continuing to poke a stale connection.
                                s.connection = None;
                                s.connection_state = ConnectionStatus::Disconnected;
                            }
                            s.last_result = Some(Arc::new(SqlResult::Error(
                                dbstudio_core::result::ErrorResult {
                                    message: e.to_string(),
                                    sql: sql_clone.clone(),
                                    execution_time_ms: 0,
                                },
                            )));
                        }
                    }
                    None
                }
            }
        });

        // Persist to storage (fire-and-forget)
        cx.spawn(async move |_cx| {
            if let Ok(store) = AppStore::singleton().await {
                let _rows = row_count_for_record;
                let _ = store
                    .history()
                    .record(&conn_id, &sql_clone, exec_time, row_count, is_error)
                    .await;
            }
        })
        .detach();
    })
    .detach();
}

/// Run a SQL query / statement against the active session of `window_id`.
pub fn execute_query(sql: String, window_id: u64, cx: &mut App) {
    let Some(exec) = ExecContext::for_window(window_id, cx) else {
        AppState::update_status(cx, "Not connected".to_string());
        return;
    };

    // Safe-mode guard: classify the SQL and check the session's environment.
    let kind = classify_sql(&sql);
    let env = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.environment)
        .unwrap_or_default();
    if requires_confirmation(kind.clone(), env, cx.global::<AppState>().safe_mode) {
        cx.update_global::<AppState, _>(|state, _cx| {
            state.window_state_mut(window_id).interaction = Interaction::ConfirmQuery {
                sql,
                kind: kind.unwrap_or(WriteKind::OtherDdl),
            };
            state.status_message = "Confirm this query before execution".to_string();
        });
        return;
    }

    spawn_execution(exec, sql, cx);
}

/// Switch which object segment (Data/Structure/Query/History) the center pane
/// of `window_id` shows.
pub fn set_object_segment(segment: ObjectSegment, window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut_for(window_id) {
            s.active_object = segment;
        }
    });
}

/// Open a table object in `segment`: load its schema, remember it as the
/// session's current object, and switch the center pane to `segment`.
///
/// This only touches state; the caller (workspace) drives the editor/results
/// entities for segments that need a query run.
pub fn open_table_object(
    table: &str,
    schema: Option<&str>,
    segment: ObjectSegment,
    window_id: u64,
    cx: &mut App,
) {
    load_table_schema(table, schema, window_id, cx);

    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut_for(window_id) {
            s.active_object = segment;
            s.current_object = Some((table.to_string(), schema.map(|s| s.to_string())));
        }
    });
}

/// Confirm and execute a previously-intercepted dangerous query in `window_id`.
pub fn confirm_dangerous_query(window_id: u64, cx: &mut App) {
    let pending = cx.update_global::<AppState, _>(|state, _cx| {
        std::mem::take(&mut state.window_state_mut(window_id).interaction)
    });
    if let Interaction::ConfirmQuery { sql, kind: _ } = pending {
        // Re-execute: this time the guard won't intercept because we
        // bypass it by calling execute_raw_query.
        execute_raw_query(sql, window_id, cx);
    }
}

/// Dismiss the pending dangerous query in `window_id` without executing it.
pub fn reject_dangerous_query(window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.window_state_mut(window_id).interaction = Interaction::Nothing;
        state.status_message = "Query cancelled".to_string();
    });
}

/// Toggle the global safe-mode flag. When enabled, write statements are
/// confirmed before execution in every environment.
pub fn toggle_safe_mode(cx: &mut App) {
    let new_value = cx.update_global::<AppState, _>(|state, _cx| {
        state.safe_mode = !state.safe_mode;
        state.status_message = format!(
            "Safe Mode {}",
            if state.safe_mode {
                "enabled"
            } else {
                "disabled"
            }
        );
        state.safe_mode
    });

    cx.spawn(async move |_cx| {
        if let Ok(store) = AppStore::singleton().await {
            let _ = dbstudio_storage::settings::set_setting_bool(
                store.pool(),
                "app.safe_mode",
                new_value,
            )
            .await;
        }
    })
    .detach();
}

/// Persist the SQL editor's Vim-mode preference (editor preference in the
/// settings table) and publish it into the global state.
pub fn save_vim_mode(on: bool, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.vim_mode = on;
        state.status_message = format!("Vim mode {}", if on { "enabled" } else { "disabled" });
    });
    cx.spawn(async move |_cx| {
        if let Ok(store) = AppStore::singleton().await {
            let _ = dbstudio_storage::settings::set_setting_bool(store.pool(), "app.vim_mode", on)
                .await;
        }
    })
    .detach();
}

/// Persist the AI provider configuration (settings table + OS keyring) and
/// publish it back into the global state for the AI panel.
pub fn save_ai_config(config: LlmConfig, cx: &mut App) {
    cx.spawn(async move |cx| {
        let result = async {
            let store = AppStore::singleton().await?;
            dbstudio_storage::ai_settings::save_ai_config(store.pool(), &config).await
        }
        .await;
        cx.update_global::<AppState, _>(|state, _cx| match result {
            Ok(()) => {
                state.ai_config = config;
                state.status_message = "AI configuration saved".to_string();
            }
            Err(e) => state.status_message = format!("Failed to save AI configuration: {e}"),
        });
    })
    .detach();
}

/// Execute a SQL query without the safe-mode guard. Used internally for
/// confirmed dangerous queries and programmatic execution (data edits).
pub fn execute_raw_query(sql: String, window_id: u64, cx: &mut App) {
    match ExecContext::for_window(window_id, cx) {
        Some(exec) => spawn_execution(exec, sql, cx),
        None => AppState::update_status(cx, "Not connected".to_string()),
    }
}

/// Fetch the next page of a truncated result set and append it in place.
///
/// Runs the original query wrapped as a derived table with the active
/// database's LIMIT/OFFSET dialect, offset by the rows already loaded. The
/// merged result replaces `last_result` so the results panel observes the new
/// row window.
pub fn load_more_rows(window_id: u64, cx: &mut App) {
    let Some((sql, offset, db_type, conn)) = cx.update_global::<AppState, _>(|state, _cx| {
        let conn = state.active_connection_for(window_id)?;
        let session = state.active_session_for(window_id)?;
        let query = match session.last_result.as_deref() {
            Some(SqlResult::Query(q)) if q.truncated() => q,
            _ => return None,
        };
        Some((
            query.sql.clone(),
            query.row_len(),
            conn.db_type(),
            conn.clone(),
        ))
    }) else {
        AppState::update_status(cx, "No more results to load".to_string());
        return;
    };
    let session_id = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.id);

    let paged = paginate_query(&sql, MAX_RESULT_ROWS, offset, db_type);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(sid) = session_id {
            if let Some(s) = state.session_mut(sid) {
                s.is_executing = true;
            }
        }
    });

    cx.spawn(async move |cx| match conn.execute(&paged).await {
        Ok(SqlResult::Query(page)) => {
            cx.update_global::<AppState, _>(|state, _cx| {
                let loaded = apply_load_more(state, session_id, page);
                if let Some(sid) = session_id {
                    if let Some(s) = state.session_mut(sid) {
                        s.is_executing = false;
                    }
                }
                state.status_message = format!("Loaded {} more rows", loaded);
            });
        }
        Ok(_) => {
            cx.update_global::<AppState, _>(|state, _cx| {
                if let Some(sid) = session_id {
                    if let Some(s) = state.session_mut(sid) {
                        s.is_executing = false;
                    }
                }
                state.status_message = "Load more returned no rows".to_string();
            });
        }
        Err(e) => {
            cx.update_global::<AppState, _>(|state, _cx| {
                if let Some(sid) = session_id {
                    if let Some(s) = state.session_mut(sid) {
                        s.is_executing = false;
                    }
                }
                state.status_message = format!("Load more failed: {}", e);
            });
        }
    })
    .detach();
}

/// Append `page` to the session `session_id`'s truncated result; returns rows added.
fn apply_load_more(state: &mut AppState, session_id: Option<u64>, page: QueryResult) -> usize {
    let Some(sid) = session_id else {
        return 0;
    };
    let Some(s) = state.session_mut(sid) else {
        return 0;
    };
    let result = match s.last_result.as_ref() {
        Some(r) => r,
        None => return 0,
    };
    let SqlResult::Query(existing) = result.as_ref() else {
        return 0;
    };
    // In-place append into the session's shared grid (Θ(page)): the panel and the
    // table delegate read the same `Arc<Grid>` live, so a page append extends the
    // existing rows instead of copy-on-write cloning the whole accumulated grid
    // (the old `Arc::make_mut` paid Θ(total) of that per page, Θ(k²·P) across k).
    // The handle address stays put - the panel's observer detects the growth by
    // the grid's live row count (see `ResultsPanel`), not by a new address.
    existing.append_rows(
        page.rows_snapshot(),
        AppendMeta {
            execution_time_ms: page.execution_time_ms(),
            truncated: page.truncated(),
        },
    )
}

fn record_history(
    state: &mut AppState,
    session_id: Option<u64>,
    sql: &str,
    execution_time_ms: u128,
    row_count: Option<usize>,
    is_error: bool,
) {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let entry = QueryHistoryEntry {
        id: state.next_history_id,
        connection_id: session_id
            .and_then(|sid| state.session(sid))
            .and_then(|s| s.connection_id.clone())
            .unwrap_or_default(),
        sql: sql.to_string(),
        execution_time_ms,
        row_count,
        is_error,
        executed_at: now,
    };
    state.next_history_id += 1;
    state.query_history.insert(0, entry);
    if state.query_history.len() > 200 {
        state.query_history.truncate(200);
    }
}

/// Save (create or update) a connection in the store.
pub fn save_connection(
    config: &ConnectionConfig,
    password: &str,
    ssh_password: &str,
    ssh_key_passphrase: &str,
    cx: &mut App,
) {
    let info = config.clone();
    let password = password.to_string();
    let ssh_password = ssh_password.to_string();
    let ssh_key_passphrase = ssh_key_passphrase.to_string();

    cx.spawn(async move |cx| {
        let store = match AppStore::singleton().await {
            Ok(store) => store,
            Err(e) => {
                tracing::error!("Failed to get app store: {}", e);
                return;
            }
        };

        match store.connections().save(&info, &password).await {
            Ok(_) => {
                let _ = store
                    .connections()
                    .set_ssh_password(&info.id, &ssh_password);
                let _ = store
                    .connections()
                    .set_ssh_passphrase(&info.id, &ssh_key_passphrase);
                if let Ok(connections) = store.connections().load_all().await {
                    cx.update_global::<AppState, _>(|state, _cx| {
                        state.saved_connections = connections;
                        state.status_message = "Connection saved".to_string();
                    });
                }
            }
            Err(e) => {
                tracing::error!("Failed to save connection: {}", e);
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!("Failed to save connection: {}", e);
                });
            }
        }
    })
    .detach();
}

/// Delete a saved connection.
pub fn delete_connection(id: String, cx: &mut App) {
    cx.spawn(async move |cx| {
        let store = match AppStore::singleton().await {
            Ok(store) => store,
            Err(e) => {
                tracing::error!("Failed to get app store: {}", e);
                return;
            }
        };

        match store.connections().delete(&id).await {
            Ok(_) => {
                if let Ok(connections) = store.connections().load_all().await {
                    cx.update_global::<AppState, _>(|state, _cx| {
                        state.saved_connections = connections;
                        state.status_message = "Connection deleted".to_string();
                    });
                }
            }
            Err(e) => {
                tracing::error!("Failed to delete connection: {}", e);
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!("Failed to delete connection: {}", e);
                });
            }
        }
    })
    .detach();
}

/// Switch the active database / schema for the active session of `window_id` and reload its tables.
pub fn select_database(database: &str, window_id: u64, cx: &mut App) {
    let conn = match cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .cloned()
    {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let db = database.to_string();
    let session_id = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.id);

    cx.spawn(async move |cx| {
        if let Err(e) = conn.switch_database(&db).await {
            cx.update_global::<AppState, _>(|state, _cx| {
                state.status_message = format!("Failed to switch database: {}", e);
            });
            return;
        }
        let databases = dbstudio_db::list_databases(&conn).await.unwrap_or_default();
        let tables = dbstudio_db::list_tables(&conn, &db)
            .await
            .unwrap_or_default();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(sid) = session_id {
                if let Some(s) = state.session_mut(sid) {
                    s.databases = databases;
                    s.tables = Arc::new(tables);
                    s.active_database = Some(db);
                }
            }
        });
    })
    .detach();
}

/// Save then connect using a freshly built config (used by the form's Connect button).
pub fn connect_config(
    config: &ConnectionConfig,
    password: &str,
    ssh_password: &str,
    ssh_key_passphrase: &str,
    window_id: u64,
    cx: &mut App,
) {
    let info = config.clone();
    let password = password.to_string();
    let ssh_password = ssh_password.to_string();
    let ssh_key_passphrase = ssh_key_passphrase.to_string();

    let id = new_session(&info.name, window_id, cx);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.session_mut(id) {
            s.connection_id = Some(info.id.clone());
        }
    });

    cx.spawn(async move |cx| {
        if let Ok(store) = AppStore::singleton().await {
            let _ = store.connections().save(&info, &password).await;
            let _ = store
                .connections()
                .set_ssh_password(&info.id, &ssh_password);
            let _ = store
                .connections()
                .set_ssh_passphrase(&info.id, &ssh_key_passphrase);
        }
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = format!("Connecting to {}...", info.name);
        });
        connect_async(id, info, cx).await;
    })
    .detach();
}

/// Refresh the tables list for the active session of `window_id`.
pub fn refresh_tables(window_id: u64, cx: &mut App) {
    let conn = match cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .cloned()
    {
        Some(conn) => conn,
        None => return,
    };
    let db = cx
        .global::<AppState>()
        .active_database_for(window_id)
        .cloned();
    let session_id = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.id);

    cx.spawn(async move |cx| {
        let tables = dbstudio_db::list_tables(&conn, db.as_deref().unwrap_or(""))
            .await
            .unwrap_or_default();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(sid) = session_id {
                if let Some(s) = state.session_mut(sid) {
                    s.tables = Arc::new(tables);
                }
            }
        });
    })
    .detach();
}

/// Toggle the tables panel visibility in `window_id`.
pub fn toggle_tables(window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        let ws = state.window_state_mut(window_id);
        ws.show_tables = !ws.show_tables;
    });
}

/// Toggle the history panel visibility in `window_id`.
pub fn toggle_history(window_id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        let ws = state.window_state_mut(window_id);
        ws.show_history = !ws.show_history;
    });
}

/// Clear all query history from state and storage.
pub fn clear_history(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.query_history.clear();
    });

    cx.spawn(async move |cx| {
        if let Ok(store) = AppStore::singleton().await {
            if let Ok(connections) = store.connections().load_all().await {
                for conn in &connections {
                    let _ = store.history().clear_for_connection(&conn.id).await;
                }
            }
        }
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = "History cleared".to_string();
        });
    })
    .detach();
}

/// Load the full schema (columns/indexes/fks) for a table in the active session of `window_id`.
pub fn load_table_schema(table: &str, schema: Option<&str>, window_id: u64, cx: &mut App) {
    let conn = match cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .cloned()
    {
        Some(conn) => conn,
        None => return,
    };
    let table = table.to_string();
    let schema = schema.map(|s| s.to_string());
    let session_id = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.id);

    cx.spawn(async move |cx| {
        let columns = dbstudio_db::list_columns(&conn, &table, schema.as_deref())
            .await
            .unwrap_or_default();
        let indexes = dbstudio_db::list_indexes(&conn, &table, schema.as_deref())
            .await
            .unwrap_or_default();
        let foreign_keys = dbstudio_db::list_foreign_keys(&conn, &table, schema.as_deref())
            .await
            .unwrap_or_default();
        let create_sql = dbstudio_db::get_create_table_sql(&conn, &table, schema.as_deref())
            .await
            .unwrap_or_default();

        let table_schema = TableSchema {
            table_name: table.clone(),
            columns,
            indexes,
            foreign_keys,
            create_sql: if create_sql.is_empty() {
                None
            } else {
                Some(create_sql)
            },
            ddl_quality: conn.db_type().ddl_quality(),
        };
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(sid) = session_id {
                if let Some(s) = state.session_mut(sid) {
                    s.table_schemas.insert(table, table_schema);
                }
            }
        });
    })
    .detach();
}

/// Rows fetched when the user browses a table from the tree.
const BROWSE_ROW_LIMIT: usize = 100;

/// Build a SELECT query with identifier quoting appropriate for the database
/// type of the active connection in `window_id`.
pub fn build_select_query(table: &str, schema: Option<&str>, window_id: u64, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .map(|c| c.db_type())
        .unwrap_or(DatabaseType::SQLite);

    let table_ref = match schema {
        Some(s) if !s.is_empty() => format!(
            "{}.{}",
            quote_ident_for(db_type, s),
            quote_ident_for(db_type, table)
        ),
        _ => quote_ident_for(db_type, table),
    };

    format!(
        "{};",
        db_type.preview_statement(&table_ref, BROWSE_ROW_LIMIT)
    )
}

/// Quote a SQL identifier (table/column name) for a specific database type.
pub fn quote_ident_for(db_type: DatabaseType, name: &str) -> String {
    db_type.quote_identifier(name)
}

/// Build a paginated SELECT for the given (single-statement) query.
///
/// Wraps the original SQL as a derived table and applies the active database's
/// row-limiting dialect so "Load more" can page through truncated result sets
/// one window at a time.
pub fn paginate_query(sql: &str, limit: usize, offset: usize, db_type: DatabaseType) -> String {
    let inner = sql.trim().trim_end_matches(';').trim();
    let wrapped = db_type.wrap_derived_table(inner, "dbstudio_sub");
    db_type.page_clause(&wrapped, limit, offset)
}

/// Build the SQL for one server-side page of `base`, optionally filtered.
///
/// The keyword filter is pushed into SQL as a case-insensitive `LIKE` across
/// every result column (mirroring the client-side text filter's semantics),
/// then `offset`/`page_size` windowing is applied in the database's dialect
/// via [`paginate_query`].
pub fn build_result_page_sql(
    base: &str,
    filter: &str,
    columns: &[String],
    offset: usize,
    page_size: usize,
    window_id: u64,
    cx: &App,
) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .map(|c| c.db_type())
        .unwrap_or(DatabaseType::SQLite);
    let trimmed = base.trim().trim_end_matches(';').trim();
    let keyword = filter.trim();
    let filtered = if keyword.is_empty() || columns.is_empty() {
        trimmed.to_string()
    } else {
        let esc = keyword.to_lowercase().replace('\'', "''");
        let conds: Vec<String> = columns
            .iter()
            .map(|col| {
                let quoted = quote_ident(col, window_id, cx);
                let cast = db_type.text_cast_expr(&quoted);
                format!("LOWER({cast}) LIKE '%{esc}%'")
            })
            .collect();
        format!(
            "{} WHERE {}",
            db_type.wrap_derived_table(trimmed, "dbstudio_filter"),
            conds.join(" OR ")
        )
    };
    paginate_query(&filtered, page_size, offset, db_type)
}

/// Fetch one results page without recording history.
///
/// Used by the results panel's server-side pagination: the SQL is built
/// internally by [`build_result_page_sql`], so it skips the safe-mode
/// classifier and the history/persistence bookkeeping that [`execute_query`]
/// performs for user-typed statements. The outcome is stored as the session's
/// latest result so the results panel picks it up like any other query.
pub fn fetch_result_page(sql: String, window_id: u64, cx: &mut App) {
    let conn = match cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .cloned()
    {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let session_id = cx
        .global::<AppState>()
        .active_session_for(window_id)
        .map(|s| s.id);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(sid) = session_id {
            if let Some(s) = state.session_mut(sid) {
                s.is_executing = true;
            }
        }
        state.status_message = "Loading page...".to_string();
    });

    cx.spawn(async move |cx| {
        let result = conn.execute(&sql).await;
        let sql_for_error = sql.clone();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(sid) = session_id {
                if let Some(s) = state.session_mut(sid) {
                    s.is_executing = false;
                }
            }
            match result {
                Ok(result) => {
                    let ok = matches!(result, SqlResult::Query(_));
                    if let Some(sid) = session_id {
                        if let Some(s) = state.session_mut(sid) {
                            s.last_result = Some(Arc::new(result));
                        }
                    }
                    state.status_message = if ok {
                        "Page loaded".to_string()
                    } else {
                        "Page fetch failed".to_string()
                    };
                }
                Err(e) => {
                    if let Some(sid) = session_id {
                        if let Some(s) = state.session_mut(sid) {
                            s.last_result = Some(Arc::new(SqlResult::Error(
                                dbstudio_core::result::ErrorResult {
                                    message: e.to_string(),
                                    sql: sql_for_error,
                                    execution_time_ms: 0,
                                },
                            )));
                        }
                    }
                    state.status_message = format!("Page fetch failed: {e}");
                }
            }
        });
    })
    .detach();
}

/// Build the candidate-value query for a foreign-key cell dropdown: distinct
/// non-null references from `target_table`, windowed per dialect by
/// [`paginate_query`] so every driver gets its native row limit.
pub fn build_fk_options_sql(
    target_table: &str,
    target_column: &str,
    limit: usize,
    offset: usize,
    db_type: DatabaseType,
) -> String {
    let q_table = quote_ident_for(db_type, target_table);
    let q_col = quote_ident_for(db_type, target_column);
    let base = format!(
        "SELECT DISTINCT {q_col} FROM {q_table} WHERE {q_col} IS NOT NULL ORDER BY {q_col}"
    );
    paginate_query(&base, limit, offset, db_type)
}

/// Quote a SQL identifier (table/column name) based on the database type of the
/// active connection in `window_id`.
pub fn quote_ident(name: &str, window_id: u64, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection_for(window_id)
        .map(|c| c.db_type())
        .unwrap_or(DatabaseType::SQLite);

    quote_ident_for(db_type, name)
}

async fn connect_async(id: u64, info: ConnectionInfo, cx: &mut AsyncApp) {
    let mut password = String::new();
    let mut ssh_password = None;
    let mut ssh_key_passphrase = None;
    if let Ok(store) = AppStore::singleton().await {
        password = store.connections().get_password(&info.id);
        let stored_ssh = store.connections().get_ssh_password(&info.id);
        if !stored_ssh.is_empty() {
            ssh_password = Some(stored_ssh);
        }
        let stored_key = store.connections().get_ssh_passphrase(&info.id);
        if !stored_key.is_empty() {
            ssh_key_passphrase = Some(stored_key);
        }
    }

    match dbstudio_db::connect(
        &info,
        &password,
        ssh_password.as_deref(),
        ssh_key_passphrase.as_deref(),
    )
    .await
    {
        Ok(conn) => {
            let mut databases = dbstudio_db::list_databases(&conn).await.unwrap_or_default();
            if databases.is_empty() {
                if let Ok(current) = conn.current_database().await {
                    if !current.is_empty() {
                        databases.push(DatabaseInfo {
                            name: current,
                            is_current: true,
                        });
                    }
                }
            }

            let active_database = databases
                .iter()
                .find(|d| d.name == info.database)
                .or_else(|| {
                    databases
                        .iter()
                        .find(|d| d.name.eq_ignore_ascii_case(&info.database))
                })
                .map(|d| d.name.clone())
                .or_else(|| databases.first().map(|d| d.name.clone()));

            if let Some(db) = &active_database {
                let _ = conn.switch_database(db).await;
            }
            let tables = dbstudio_db::list_tables(&conn, active_database.as_deref().unwrap_or(""))
                .await
                .unwrap_or_default();

            let conn = Arc::new(conn);
            cx.update_global::<AppState, _>(|state, _cx| {
                if let Some(s) = state.session_mut(id) {
                    s.connection = Some(conn);
                    s.connection_state = ConnectionStatus::Connected;
                    s.environment = info.environment;
                    s.active_database = active_database;
                    s.databases = databases;
                    s.tables = Arc::new(tables);
                }
                state.status_message = format!("Connected to {}", info.name);
            });
        }
        Err(e) => {
            tracing::warn!("Connection failed: {:?}", e);
            cx.update_global::<AppState, _>(|state, _cx| {
                if let Some(s) = state.session_mut(id) {
                    s.connection_state = ConnectionStatus::Disconnected;
                }
                state.status_message = format!("Connection failed: {:?}", e);
            });
        }
    }
}

/// Execute one SQL statement and report the awaited outcome back to the
/// caller — success and failure callbacks both run inside the async result
/// handler, so stack bookkeeping always reflects what actually happened.
/// Records history either way.
fn run_edit(
    exec: ExecContext,
    sql: String,
    label: String,
    on_success: Box<dyn FnOnce(&mut AppState) + 'static>,
    on_failure: Box<dyn FnOnce(&mut AppState) + 'static>,
    cx: &mut App,
) {
    let ExecContext {
        conn,
        conn_id,
        session_id,
    } = exec;
    cx.spawn(async move |cx| {
        let result = conn.execute(&sql).await;
        let (exec_ms, row_count, is_error, failure, lost) = match &result {
            Ok(SqlResult::Query(q)) => (
                q.execution_time_ms(),
                Some(q.row_count()),
                false,
                None,
                false,
            ),
            Ok(SqlResult::Modified(m)) => (m.execution_time_ms, None, false, None, false),
            Ok(SqlResult::Error(e)) => (0, None, true, Some(e.message.clone()), false),
            Err(e) => (0, None, true, Some(e.to_string()), e.is_connection_lost()),
        };
        // Convert any failure into a SqlResult the state update can store.
        // The awaited `result` is used nowhere else, so the snapshot takes
        // it by value: a Query outcome lands in the session without ever
        // deep-copying its grid.
        let result_snapshot = match result {
            Ok(r) => r,
            Err(e) => SqlResult::Error(dbstudio_core::result::ErrorResult {
                message: e.to_string(),
                sql: sql.clone(),
                execution_time_ms: 0,
            }),
        };

        let sql_for_history = sql.clone();
        cx.update_global::<AppState, _>(|state, _cx| {
            record_history(
                state,
                session_id,
                &sql_for_history,
                exec_ms,
                row_count,
                is_error,
            );
            if is_error {
                // Bookkeeping always runs: a failed undo/redo puts its record
                // back on the right stack regardless of *why* it failed.
                on_failure(state);
                if lost {
                    if let Some(sid) = session_id {
                        if let Some(s) = state.session_mut(sid) {
                            s.connection = None;
                            s.connection_state = ConnectionStatus::Disconnected;
                        }
                    }
                    state.status_message =
                        format!("Connection lost: {}", failure.unwrap_or_default());
                } else {
                    state.status_message =
                        format!("{} failed: {}", label, failure.unwrap_or_default());
                }
            } else {
                if let Some(sid) = session_id {
                    if let Some(s) = state.session_mut(sid) {
                        s.last_result = Some(Arc::new(result_snapshot));
                    }
                }
                on_success(state);
                state.status_message = format!("Applied {}", label);
            }
        });

        // Persist to storage (fire-and-forget).
        cx.spawn(async move |_cx| {
            if let Ok(store) = AppStore::singleton().await {
                let _ = store
                    .history()
                    .record(&conn_id, &sql, exec_ms, row_count, is_error)
                    .await;
            }
        })
        .detach();
    })
    .detach();
}

/// Undo the last applied edit in the active session of `window_id`.
///
/// The inverse runs first; the redo entry is only recorded when the inverse
/// actually went through. A failed undo puts the record back on the undo
/// stack instead of arming a redo that would replay an edit that never left.
pub fn undo_last_edit(window_id: u64, cx: &mut App) {
    let Some(exec) = ExecContext::for_window(window_id, cx) else {
        AppState::update_status(cx, "Not connected".to_string());
        return;
    };

    let popped = cx.update_global::<AppState, _>(|state, _cx| {
        state
            .active_session_mut_for(window_id)
            .and_then(|s| s.edits.undo_top())
    });
    let Some(record) = popped else {
        AppState::update_status(cx, "Nothing to undo".to_string());
        return;
    };
    if record.inverse_sql.is_empty() {
        // Records without a derivable inverse are never pushed; reaching here
        // means a legacy entry. Surface it and move on.
        AppState::update_status(cx, format!("Undo not available for: {}", record.label));
        return;
    }

    let redo_record = record.clone();
    let undo_record = record.clone();
    run_edit(
        exec,
        record.inverse_sql.clone(),
        format!("undo of {}", record.label),
        Box::new(move |state| {
            if let Some(s) = state.active_session_mut_for(window_id) {
                s.edits.undo_succeeded(&redo_record);
            }
        }),
        Box::new(move |state| {
            if let Some(s) = state.active_session_mut_for(window_id) {
                s.edits.undo_failed(undo_record);
            }
        }),
        cx,
    );
}

/// Redo the last undone edit in the active session of `window_id`.
///
/// Mirrors [`undo_last_edit`]: the forward statement runs first and the undo
/// entry is only recorded when it actually went through.
pub fn redo_last_edit(window_id: u64, cx: &mut App) {
    let Some(exec) = ExecContext::for_window(window_id, cx) else {
        AppState::update_status(cx, "Not connected".to_string());
        return;
    };

    let popped = cx.update_global::<AppState, _>(|state, _cx| {
        state
            .active_session_mut_for(window_id)
            .and_then(|s| s.edits.redo_top())
    });
    let Some(record) = popped else {
        AppState::update_status(cx, "Nothing to redo".to_string());
        return;
    };

    let undo_record = record.clone();
    let redo_record = record.clone();
    run_edit(
        exec,
        record.forward_sql.clone(),
        format!("redo of {}", record.label),
        Box::new(move |state| {
            if let Some(s) = state.active_session_mut_for(window_id) {
                s.edits.redo_succeeded(&undo_record);
            }
        }),
        Box::new(move |state| {
            if let Some(s) = state.active_session_mut_for(window_id) {
                s.edits.redo_failed(redo_record);
            }
        }),
        cx,
    );
}

/// Apply staged edits in order, each leaving the pending buffer and entering
/// the undo stack only when its execution succeeded. Stops at the first
/// failure; the failed edit and everything after it stay pending for retry.
pub fn apply_edits_sequentially(edits: Vec<guard::PendingWrite>, window_id: u64, cx: &mut App) {
    if edits.is_empty() {
        return;
    }
    let Some(exec) = ExecContext::for_window(window_id, cx) else {
        AppState::update_status(cx, "Not connected".to_string());
        return;
    };
    let ExecContext {
        conn,
        conn_id,
        session_id,
    } = exec;

    cx.spawn(async move |cx| {
        for edit in edits {
            let result = conn.execute(&edit.sql).await;
            let (exec_ms, row_count, is_error, failure) = match &result {
                Ok(SqlResult::Query(q)) => {
                    (q.execution_time_ms(), Some(q.row_count()), false, None)
                }
                Ok(SqlResult::Modified(m)) => (m.execution_time_ms, None, false, None),
                Ok(SqlResult::Error(e)) => (0, None, true, Some(e.message.clone())),
                Err(e) => (0, None, true, Some(e.to_string())),
            };
            let failed = is_error;
            let sql = edit.sql.clone();
            let label = edit.label.clone();
            // anyhow::Error is not Clone, so the state update works on a
            // SqlResult snapshot (transport errors become SqlResult::Error).
            // The result is used nowhere else in this iteration, so the
            // snapshot moves: a Query outcome reaches the session without a
            // deep copy of its grid.
            let result_snapshot = match result {
                Ok(r) => r,
                Err(e) => SqlResult::Error(dbstudio_core::result::ErrorResult {
                    message: e.to_string(),
                    sql: sql.clone(),
                    execution_time_ms: 0,
                }),
            };

            cx.update_global::<AppState, _>(|state, _cx| {
                record_history(state, session_id, &sql, exec_ms, row_count, is_error);
                if let Some(sid) = session_id {
                    if let Some(s) = state.session_mut(sid) {
                        s.last_result = Some(Arc::new(result_snapshot));
                        if failed {
                            // The edit (and the rest) stay pending for retry.
                        } else {
                            // Leave the pending queue and enter the undo side;
                            // the ledger owns the "new apply clears redo" and
                            // "no inverse -> not undoable" rules.
                            s.edits.apply_success(&sql);
                        }
                    }
                }
                state.status_message = if failed {
                    format!("{} failed: {}", label, failure.unwrap_or_default())
                } else {
                    format!("Applied {}", label)
                };
            });

            // Persist to storage (fire-and-forget).
            let conn_id = conn_id.clone();
            cx.spawn(async move |_cx| {
                if let Ok(store) = AppStore::singleton().await {
                    let _ = store
                        .history()
                        .record(&conn_id, &sql, exec_ms, row_count, is_error)
                        .await;
                }
            })
            .detach();

            if failed {
                return;
            }
        }
    })
    .detach();
}

/// Save a query as a favorite.
pub fn save_favorite(name: &str, sql: &str, window_id: u64, cx: &mut App) {
    let connection_id = cx
        .global::<AppState>()
        .active_connection_id_for(window_id)
        .cloned();
    let name = name.to_string();
    let sql = sql.to_string();

    cx.spawn(async move |_cx| {
        if let Ok(store) = AppStore::singleton().await {
            if let Err(e) = store
                .favorites()
                .save(&name, &sql, connection_id.as_deref())
                .await
            {
                tracing::error!("Failed to save favorite: {}", e);
            }
        }
    })
    .detach();
}

/// Load all favorites.
pub fn load_favorites(cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Ok(store) = AppStore::singleton().await {
            if let Ok(favorites) = store.favorites().load_all().await {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.favorites = favorites
                        .into_iter()
                        .map(|f| crate::state::FavoriteEntry {
                            id: f.id,
                            name: f.name,
                            sql: f.sql,
                            connection_id: f.connection_id,
                        })
                        .collect();
                });
            }
        }
    })
    .detach();
}

/// Delete a favorite by id.
pub fn delete_favorite(id: i64, cx: &mut App) {
    cx.spawn(async move |_cx| {
        if let Ok(store) = AppStore::singleton().await {
            if let Err(e) = store.favorites().delete(id).await {
                tracing::error!("Failed to delete favorite: {}", e);
            }
        }
    })
    .detach();
}

/// Trailing warning for an export status line when the dump is incomplete.
fn partial_export_suffix(stats: &dbstudio_db::export::ExportStats) -> &'static str {
    if stats.partial {
        " — warning: the result limit was hit, so the dump is missing rows"
    } else {
        ""
    }
}

/// Export the current database (of the active session in `window_id`) to a SQL dump file.
pub fn export_database(path: &str, window_id: u64, cx: &mut AsyncApp) {
    let path = path.to_string();
    cx.spawn(async move |cx| {
        let conn_result = cx.update_global::<AppState, _>(|state, _cx| {
            state
                .active_session_for(window_id)
                .and_then(|s| s.connection.as_ref().map(|c| c.clone()))
        });

        let Some(conn) = conn_result else {
            return;
        };

        let path = std::path::PathBuf::from(path);
        let config = dbstudio_db::export::ExportConfig::default();

        match dbstudio_db::export::export_database(&conn, &path, &config, None).await {
            Ok(stats) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!(
                        "Exported {} tables ({} rows) to {}{}",
                        stats.tables_exported,
                        stats.rows_exported,
                        path.display(),
                        partial_export_suffix(&stats)
                    );
                });
            }
            Err(e) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!("Export failed: {}", e);
                });
            }
        }
    })
    .detach();
}

/// Export a single table (schema + data) of the active session in `window_id`
/// to a SQL dump file. Used by the tables-tree "Export Table…" menu item.
pub fn export_table(path: &str, table: &str, window_id: u64, cx: &mut AsyncApp) {
    let path = path.to_string();
    let table = table.to_string();
    cx.spawn(async move |cx| {
        let conn_result = cx.update_global::<AppState, _>(|state, _cx| {
            state
                .active_session_for(window_id)
                .and_then(|s| s.connection.clone())
        });

        let Some(conn) = conn_result else {
            return;
        };

        let path = std::path::PathBuf::from(path);
        let config = dbstudio_db::export::ExportConfig {
            tables: vec![table.clone()],
            ..dbstudio_db::export::ExportConfig::default()
        };

        match dbstudio_db::export::export_database(&conn, &path, &config, None).await {
            Ok(stats) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!(
                        "Exported {} ({} rows) to {}{}",
                        table,
                        stats.rows_exported,
                        path.display(),
                        partial_export_suffix(&stats)
                    );
                });
            }
            Err(e) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!("Export failed: {}", e);
                });
            }
        }
    })
    .detach();
}

/// Import a SQL dump file into the current database (of the active session in `window_id`).
pub fn import_database(path: &str, window_id: u64, cx: &mut AsyncApp) {
    let path = path.to_string();
    cx.spawn(async move |cx| {
        let conn_result = cx.update_global::<AppState, _>(|state, _cx| {
            state
                .active_session_for(window_id)
                .and_then(|s| s.connection.clone())
        });

        let Some(conn) = conn_result else {
            return;
        };

        let path = std::path::PathBuf::from(path);

        match dbstudio_db::export::import_database(&conn, &path, None).await {
            Ok(stats) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!(
                        "Imported {} statements ({} rows, {} errors)",
                        stats.statements_executed, stats.rows_imported, stats.errors
                    );
                });
            }
            Err(e) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.status_message = format!("Import failed: {}", e);
                });
            }
        }
    })
    .detach();
}
