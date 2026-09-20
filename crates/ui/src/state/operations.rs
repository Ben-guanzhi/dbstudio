use std::sync::Arc;

use dbstudio_core::models::{DatabaseType, ConnectionConfig};
use dbstudio_core::result::{QueryResult, SqlResult, MAX_RESULT_ROWS};
use dbstudio_core::schema::{DatabaseInfo, TableSchema};
use dbstudio_db::utils::{quote_backtick, quote_bracket, quote_double_quote};
use dbstudio_storage::types::{ConnectionInfo, ConnectionStatus, QueryHistoryEntry};
use dbstudio_storage::AppStore;
use gpui::*;

use super::guard::{classify_sql, requires_confirmation, WriteKind};
use super::{AppState, ConnectionSession};

/// Create a new empty session and make it the active one. Returns its id.
pub fn new_session(name: &str, cx: &mut App) -> u64 {
    let mut id = 0u64;
    cx.update_global::<AppState, _>(|state, _cx| {
        id = state.next_session_id;
        state.next_session_id += 1;
        state.sessions.insert(0, ConnectionSession::new(id, name));
        state.active_session = Some(id);
    });
    id
}

/// Switch the active session to `id`, restoring its editor buffer.
pub fn switch_session(id: u64, cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        if state.session(id).is_some() {
            state.active_session = Some(id);
        }
    });
}

/// Close a session, dropping its connection (and releasing its SSH tunnel).
/// Removes it from the tab list and activates a neighbouring tab if any.
pub fn close_session(id: u64, cx: &mut App) {
    let mut next_active = None;
    cx.update_global::<AppState, _>(|state, _cx| {
        let Some(pos) = state.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        state.sessions.remove(pos);

        if state.sessions.is_empty() {
            state.active_session = None;
            state.status_message = "Disconnected".to_string();
            return;
        }

        // Activate the tab that took the removed one's place, or the last one.
        let new_pos = pos.min(state.sessions.len() - 1);
        next_active = Some(state.sessions[new_pos].id);
        state.active_session = next_active;
        state.status_message = String::new();
    });
}

/// Sync the editor buffer for the active session (called whenever the editor changes).
pub fn set_active_editor_text(text: &str, cx: &mut App) {
    update_active_session(cx, |session, _cx| {
        session.editor_text = text.to_string();
    });
}

/// Run `f` against the active session (if any), then notifies observers.
fn update_active_session<F>(cx: &mut App, f: F)
where
    F: FnOnce(&mut ConnectionSession, &mut App),
{
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(session) = state.active_session_mut() {
            f(session, _cx);
        }
    });
}

/// Initiate a connection asynchronously. Updates `AppState` along the way.
pub fn connect(conn_info: &ConnectionInfo, cx: &mut App) {
    let info = conn_info.clone();
    let id = new_session(&info.name, cx);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.session_mut(id) {
            s.connection_state = ConnectionStatus::Connecting;
        }
        state.status_message = format!("Connecting to {}...", info.name);
    });

    cx.spawn(async move |cx| connect_async(id, info, cx).await).detach();
}

/// Disconnect the active session (keeps the tab open, ready to reconnect).
pub fn disconnect(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut() {
            s.connection_state = ConnectionStatus::Disconnecting;
        }
    });

    cx.spawn(async move |cx| {
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.active_session_mut() {
                s.connection = None;
                s.connection_state = ConnectionStatus::Disconnected;
                s.active_database = None;
                s.databases.clear();
                s.tables.clear();
                s.table_schemas.clear();
                s.last_result = None;
                s.is_executing = false;
            }
            state.status_message = "Disconnected".to_string();
        });
    })
    .detach();
}

/// Run a SQL query / statement against the active session.
pub fn execute_query(sql: String, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection().cloned() {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let conn_id = cx
        .global::<AppState>()
        .active_session()
        .and_then(|s| s.connection_id.clone())
        .unwrap_or_default();

    // Safe-mode guard: classify the SQL and check the session's environment.
    let kind = classify_sql(&sql);
    let env = cx
        .global::<AppState>()
        .active_session()
        .map(|s| s.environment)
        .unwrap_or_default();
    if requires_confirmation(kind.clone(), env, cx.global::<AppState>().safe_mode) {
        cx.update_global::<AppState, _>(|state, _cx| {
            state.pending_dangerous_query = Some((sql, kind.unwrap_or(WriteKind::OtherDdl)));
            state.status_message = "Confirm this query before execution".to_string();
        });
        return;
    }

    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut() {
            s.is_executing = true;
            s.last_result = None;
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
            if let Some(s) = state.active_session_mut() {
                s.is_executing = false;
            }
            match &result {
                Ok(result) => {
                    let (exec_ms, rc, err) = match result {
                        SqlResult::Query(q) => (q.execution_time_ms, Some(q.row_count), false),
                        SqlResult::Modified(m) => (m.execution_time_ms, None, false),
                        SqlResult::Error(_) => (0, None, true),
                    };
                    if let Some(s) = state.active_session_mut() {
                        s.last_result = Some(Arc::new(result.clone()));
                    }
                    record_history(state, &sql_clone, exec_ms, rc, err);
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
                    record_history(state, &sql_clone, 0, None, true);
                    is_error = true;
                    state.status_message = format!("Query failed: {}", e);
                    if let Some(s) = state.active_session_mut() {
                        s.last_result = Some(Arc::new(SqlResult::Error(
                            dbstudio_core::result::ErrorResult {
                                message: e.to_string(),
                                sql: sql_clone.clone(),
                                execution_time_ms: 0,
                            },
                        )));
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

/// Confirm and execute a previously-intercepted dangerous query.
pub fn confirm_dangerous_query(cx: &mut App) {
    let pending = cx
        .update_global::<AppState, _>(|state, _cx| state.pending_dangerous_query.take());
    if let Some((sql, _kind)) = pending {
        // Re-execute: this time the guard won't intercept because we
        // bypass it by calling execute_raw_query.
        execute_raw_query(sql, cx);
    }
}

/// Dismiss the pending dangerous query without executing it.
pub fn reject_dangerous_query(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.pending_dangerous_query = None;
        state.status_message = "Query cancelled".to_string();
    });
}

/// Toggle the global safe-mode flag. When enabled, write statements are
/// confirmed before execution in every environment.
pub fn toggle_safe_mode(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.safe_mode = !state.safe_mode;
        state.status_message = format!(
            "Safe Mode {}",
            if state.safe_mode { "enabled" } else { "disabled" }
        );
    });
}

/// Execute a SQL query without the safe-mode guard. Used internally for
/// confirmed dangerous queries and programmatic execution (data edits).
pub fn execute_raw_query(sql: String, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection().cloned() {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let conn_id = cx
        .global::<AppState>()
        .active_session()
        .and_then(|s| s.connection_id.clone())
        .unwrap_or_default();

    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut() {
            s.is_executing = true;
            s.last_result = None;
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
            if let Some(s) = state.active_session_mut() {
                s.is_executing = false;
            }
            match &result {
                Ok(result) => {
                    let (exec_ms, rc, err) = match result {
                        SqlResult::Query(q) => (q.execution_time_ms, Some(q.row_count), false),
                        SqlResult::Modified(m) => (m.execution_time_ms, None, false),
                        SqlResult::Error(_) => (0, None, true),
                    };
                    if let Some(s) = state.active_session_mut() {
                        s.last_result = Some(Arc::new(result.clone()));
                    }
                    record_history(state, &sql_clone, exec_ms, rc, err);
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
                    record_history(state, &sql_clone, 0, None, true);
                    is_error = true;
                    state.status_message = format!("Query failed: {}", e);
                    if let Some(s) = state.active_session_mut() {
                        s.last_result = Some(Arc::new(SqlResult::Error(
                            dbstudio_core::result::ErrorResult {
                                message: e.to_string(),
                                sql: sql_clone.clone(),
                                execution_time_ms: 0,
                            },
                        )));
                    }
                    None
                }
            }
        });

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

/// Fetch the next page of a truncated result set and append it in place.
///
/// Runs the original query wrapped as a derived table with the active
/// database's LIMIT/OFFSET dialect, offset by the rows already loaded. The
/// merged result replaces `last_result` so the results panel observes the new
/// row window.
pub fn load_more_rows(cx: &mut App) {
    let Some((sql, offset, db_type, conn)) = cx.update_global::<AppState, _>(|state, _cx| {
        let conn = state.active_connection();
        let session = state.active_session();
        let conn = conn?;
        let session = session?;
        let query = match session.last_result.as_deref() {
            Some(SqlResult::Query(q)) if q.truncated => q,
            _ => return None,
        };
        Some((
            query.sql.clone(),
            query.rows.len(),
            conn.db_type(),
            conn.clone(),
        ))
    }) else {
        AppState::update_status(cx, "No more results to load".to_string());
        return;
    };

    let paged = paginate_query(&sql, MAX_RESULT_ROWS, offset, db_type);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.active_session_mut() {
            s.is_executing = true;
        }
    });

    cx.spawn(async move |cx| {
        match conn.execute(&paged).await {
            Ok(SqlResult::Query(page)) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    let loaded = apply_load_more(state, page);
                    if let Some(s) = state.active_session_mut() {
                        s.is_executing = false;
                    }
                    state.status_message = format!("Loaded {} more rows", loaded);
                });
            }
            Ok(_) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    if let Some(s) = state.active_session_mut() {
                        s.is_executing = false;
                    }
                    state.status_message = "Load more returned no rows".to_string();
                });
            }
            Err(e) => {
                cx.update_global::<AppState, _>(|state, _cx| {
                    if let Some(s) = state.active_session_mut() {
                        s.is_executing = false;
                    }
                    state.status_message = format!("Load more failed: {}", e);
                });
            }
        }
    })
    .detach();
}

/// Append `page` to the active session's truncated result; returns rows added.
fn apply_load_more(state: &mut AppState, page: QueryResult) -> usize {
    let Some(s) = state.active_session_mut() else {
        return 0;
    };
    let Some(SqlResult::Query(existing)) = s.last_result.as_deref() else {
        return 0;
    };
    let mut merged = existing.clone();
    let added = page.rows.len();
    merged.rows.extend(page.rows);
    merged.row_count = merged.rows.len();
    merged.total_row_count = if page.truncated {
        merged.rows.len().saturating_add(1)
    } else {
        merged.rows.len()
    };
    merged.truncated = page.truncated;
    merged.execution_time_ms = page.execution_time_ms;
    s.last_result = Some(Arc::new(SqlResult::Query(merged)));
    added
}

fn record_history(
    state: &mut AppState,
    sql: &str,
    execution_time_ms: u128,
    row_count: Option<usize>,
    is_error: bool,
) {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let entry = QueryHistoryEntry {
        id: state.next_history_id,
        connection_id: state
            .active_session()
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
pub fn save_connection(config: &ConnectionConfig, password: &str, ssh_password: &str, ssh_key_passphrase: &str, cx: &mut App) {
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
                let _ = store.connections().set_ssh_password(&info.id, &ssh_password);
                let _ = store.connections().set_ssh_passphrase(&info.id, &ssh_key_passphrase);
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

/// Switch the active database / schema for the active session and reload its tables.
pub fn select_database(database: &str, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection().cloned() {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let db = database.to_string();
    let session_id = cx.global::<AppState>().active_session;

    cx.spawn(async move |cx| {
        if let Err(e) = conn.switch_database(&db).await {
            cx.update_global::<AppState, _>(|state, _cx| {
                state.status_message = format!("Failed to switch database: {}", e);
            });
            return;
        }
        let databases = dbstudio_db::list_databases(&conn).await.unwrap_or_default();
        let tables = dbstudio_db::list_tables(&conn, &db).await.unwrap_or_default();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.session_mut(session_id.unwrap_or(0)) {
                s.databases = databases;
                s.tables = tables;
                s.active_database = Some(db);
            }
        });
    })
    .detach();
}

/// Save then connect using a freshly built config (used by the form's Connect button).
pub fn connect_config(config: &ConnectionConfig, password: &str, ssh_password: &str, ssh_key_passphrase: &str, cx: &mut App) {
    let info = config.clone();
    let password = password.to_string();
    let ssh_password = ssh_password.to_string();
    let ssh_key_passphrase = ssh_key_passphrase.to_string();

    let id = new_session(&info.name, cx);
    cx.update_global::<AppState, _>(|state, _cx| {
        if let Some(s) = state.session_mut(id) {
            s.connection_id = Some(info.id.clone());
        }
    });

    cx.spawn(async move |cx| {
        if let Ok(store) = AppStore::singleton().await {
            let _ = store.connections().save(&info, &password).await;
            let _ = store.connections().set_ssh_password(&info.id, &ssh_password);
            let _ = store.connections().set_ssh_passphrase(&info.id, &ssh_key_passphrase);
        }
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = format!("Connecting to {}...", info.name);
        });
        connect_async(id, info, cx).await;
    })
    .detach();
}

/// Refresh the tables list for the active session.
pub fn refresh_tables(cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection().cloned() {
        Some(conn) => conn,
        None => return,
    };
    let db = cx.global::<AppState>().active_database().cloned();
    let session_id = cx.global::<AppState>().active_session;

    cx.spawn(async move |cx| {
        let tables = dbstudio_db::list_tables(&conn, db.as_deref().unwrap_or(""))
            .await
            .unwrap_or_default();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.session_mut(session_id.unwrap_or(0)) {
                s.tables = tables;
            }
        });
    })
    .detach();
}

/// Toggle the tables panel visibility.
pub fn toggle_tables(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.show_tables = !state.show_tables;
    });
}

/// Toggle the history panel visibility.
pub fn toggle_history(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.show_history = !state.show_history;
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

/// Load the full schema (columns/indexes/fks) for a table in the active session.
pub fn load_table_schema(table: &str, schema: Option<&str>, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection().cloned() {
        Some(conn) => conn,
        None => return,
    };
    let table = table.to_string();
    let schema = schema.map(|s| s.to_string());
    let session_id = cx.global::<AppState>().active_session;

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
        };
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.session_mut(session_id.unwrap_or(0)) {
                s.table_schemas.insert(table, table_schema);
            }
        });
    })
    .detach();
}

/// Build a SELECT query with identifier quoting appropriate for the active database type.
pub fn build_select_query(table: &str, schema: Option<&str>, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection()
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

    if db_type == DatabaseType::MSSQL {
        format!("SELECT TOP 100 * FROM {};", table_ref)
    } else {
        format!("SELECT * FROM {} LIMIT 100;", table_ref)
    }
}

/// Quote a SQL identifier (table/column name) for a specific database type.
pub fn quote_ident_for(db_type: DatabaseType, name: &str) -> String {
    match db_type {
        DatabaseType::MySQL => quote_backtick(name),
        DatabaseType::MSSQL => quote_bracket(name),
        _ => quote_double_quote(name),
    }
}

/// Build a paginated SELECT for the given (single-statement) query.
///
/// Wraps the original SQL as a derived table and applies the active database's
/// row-limiting dialect so "Load more" can page through truncated result sets
/// one window at a time.
pub fn paginate_query(sql: &str, limit: usize, offset: usize, db_type: DatabaseType) -> String {
    let inner = sql.trim().trim_end_matches(';').trim();
    let wrapped = format!("SELECT * FROM ( {inner} ) AS _dbstudio_sub");
    match db_type {
        DatabaseType::MSSQL => format!(
            "{wrapped} ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"
        ),
        DatabaseType::Oracle => format!("{wrapped} OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"),
        _ => format!("{wrapped} LIMIT {limit} OFFSET {offset}"),
    }
}

/// Quote a SQL identifier (table/column name) based on the active database type.
pub fn quote_ident(name: &str, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection()
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

    match dbstudio_db::connect(&info, &password, ssh_password.as_deref(), ssh_key_passphrase.as_deref()).await {
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
                .or_else(|| databases.iter().find(|d| d.name.eq_ignore_ascii_case(&info.database)))
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
                    s.tables = tables;
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

/// Generate a best-effort inverse SQL for an applied edit.
/// This is a simplified approach that handles common cases.
pub fn generate_inverse(sql: &str, _label: &str) -> String {
    let upper = sql.to_uppercase();
    if upper.starts_with("INSERT") {
        // INSERT → DELETE with same WHERE (best effort: delete by all columns)
        // This is a simplified inverse; a full implementation would need row data
        format!("-- undo: {}", sql.trim_end_matches(';'))
    } else if upper.starts_with("UPDATE") {
        // UPDATE → can't easily invert without knowing old values
        // Mark as needing manual review
        format!("-- undo: {}", sql.trim_end_matches(';'))
    } else if upper.starts_with("DELETE") {
        // DELETE → can't easily invert without knowing deleted row data
        format!("-- undo: {}", sql.trim_end_matches(';'))
    } else {
        format!("-- undo: {}", sql.trim_end_matches(';'))
    }
}

/// Undo the last applied edit by executing its inverse SQL.
pub fn undo_last_edit(cx: &mut App) {
    let record = cx.update_global::<AppState, _>(|state, _cx| {
        match state.active_session_mut() {
            Some(s) if !s.undo_stack.is_empty() => {
                let record = s.undo_stack.pop().unwrap();
                s.redo_stack.push(crate::state::guard::EditRecord {
                    forward_sql: record.forward_sql.clone(),
                    inverse_sql: record.inverse_sql.clone(),
                    label: record.label.clone(),
                });
                Some(record)
            }
            _ => None,
        }
    });
    if let Some(record) = record {
        // Only execute if the inverse is actual SQL (not a comment placeholder).
        if !record.inverse_sql.is_empty() && !record.inverse_sql.starts_with("--") {
            execute_raw_query(record.inverse_sql, cx);
            AppState::update_status(cx, format!("Undid {}", record.label));
        } else {
            AppState::update_status(cx, format!("Undo not available for: {}", record.label));
        }
    } else {
        AppState::update_status(cx, "Nothing to undo".to_string());
    }
}

/// Redo the last undone edit by re-executing its forward SQL.
pub fn redo_last_edit(cx: &mut App) {
    let record = cx.update_global::<AppState, _>(|state, _cx| {
        match state.active_session_mut() {
            Some(s) if !s.redo_stack.is_empty() => {
                let record = s.redo_stack.pop().unwrap();
                s.undo_stack.push(crate::state::guard::EditRecord {
                    forward_sql: record.forward_sql.clone(),
                    inverse_sql: record.inverse_sql.clone(),
                    label: record.label.clone(),
                });
                Some(record)
            }
            _ => None,
        }
    });
    if let Some(record) = record {
        execute_raw_query(record.forward_sql, cx);
        AppState::update_status(cx, format!("Redid {}", record.label));
    } else {
        AppState::update_status(cx, "Nothing to redo".to_string());
    }
}

/// Save a query as a favorite.
pub fn save_favorite(name: &str, sql: &str, cx: &mut App) {
    let connection_id = cx
        .global::<AppState>()
        .active_connection_id()
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

/// Export the current database to a SQL dump file.
pub fn export_database(path: &str, cx: &mut AsyncApp) {
    let path = path.to_string();
    cx.spawn(async move |cx| {
        let conn_result = cx.update_global::<AppState, _>(|state, _cx| {
            state.active_session()
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
                        "Exported {} tables ({} rows) to {}",
                        stats.tables_exported,
                        stats.rows_exported,
                        path.display()
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

/// Import a SQL dump file into the current database.
pub fn import_database(path: &str, cx: &mut AsyncApp) {
    let path = path.to_string();
    cx.spawn(async move |cx| {
        let conn_result = cx.update_global::<AppState, _>(|state, _cx| {
            state.active_session()
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
                        stats.statements_executed,
                        stats.rows_imported,
                        stats.errors
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
