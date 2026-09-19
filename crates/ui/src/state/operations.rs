use std::collections::HashMap;
use std::sync::Arc;

use dbstudio_core::models::{DatabaseType, ConnectionConfig};
use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::{DatabaseInfo, TableSchema};
use dbstudio_db::utils::{quote_backtick, quote_bracket, quote_double_quote};
use dbstudio_storage::types::{ConnectionInfo, ConnectionStatus, QueryHistoryEntry};
use dbstudio_storage::AppStore;
use gpui::*;

use super::AppState;

/// Initiate a connection asynchronously. Updates `AppState` along the way.
pub fn connect(conn_info: &ConnectionInfo, cx: &mut App) {
    let info = conn_info.clone();
    cx.update_global::<AppState, _>(|state, _cx| {
        state.connection_state = ConnectionStatus::Connecting;
        state.status_message = format!("Connecting to {}...", info.name);
    });

    cx.spawn(async move |cx| connect_async(info, cx).await).detach();
}

/// Disconnect from the active database.
pub fn disconnect(cx: &mut App) {
    cx.update_global::<AppState, _>(|state, _cx| {
        state.connection_state = ConnectionStatus::Disconnecting;
    });

    cx.spawn(async move |cx| {
        cx.update_global::<AppState, _>(|state, _cx| {
            state.connection_state = ConnectionStatus::Disconnected;
            state.active_connection = None;
            state.active_connection_name = None;
            state.active_connection_id = None;
            state.active_database = None;
            state.databases = Vec::new();
            state.tables = Vec::new();
            state.table_schemas = HashMap::new();
            state.last_result = None;
            state.is_executing = false;
            state.status_message = "Disconnected".to_string();
        });
    })
    .detach();
}

/// Run a SQL query / statement against the active connection.
pub fn execute_query(sql: String, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection.clone() {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };

    cx.update_global::<AppState, _>(|state, _cx| {
        state.is_executing = true;
        state.last_result = None;
        state.status_message = "Executing...".to_string();
    });

    cx.spawn(async move |cx| {
        let result = conn.execute(&sql).await;
        let mut conn_id = String::new();
        let mut exec_time = 0u128;
        let mut row_count = None;
        let mut is_error = false;

        cx.update_global::<AppState, _>(|state, _cx| {
            state.is_executing = false;
            match &result {
                Ok(result) => {
                    let (exec_ms, rc, err) = match result {
                        SqlResult::Query(q) => (q.execution_time_ms, Some(q.row_count), false),
                        SqlResult::Modified(m) => (m.execution_time_ms, None, false),
                        SqlResult::Error(_) => (0, None, true),
                    };
                    record_history(state, &sql, exec_ms, rc, err);
                    conn_id = state.active_connection_id.clone().unwrap_or_default();
                    exec_time = exec_ms;
                    row_count = rc;
                    is_error = err;
                    state.last_result = Some(Arc::new(result.clone()));
                    state.status_message = "Query executed".to_string();
                }
                Err(e) => {
                    record_history(state, &sql, 0, None, true);
                    conn_id = state.active_connection_id.clone().unwrap_or_default();
                    is_error = true;
                    state.status_message = format!("Query failed: {}", e);
                    state.last_result = Some(Arc::new(SqlResult::Error(
                        dbstudio_core::result::ErrorResult {
                            message: e.to_string(),
                            sql: sql.clone(),
                            execution_time_ms: 0,
                        },
                    )));
                }
            }
        });

        // Persist to storage (fire-and-forget)
        let sql_clone = sql.clone();
        cx.spawn(async move |_cx| {
            if let Ok(store) = AppStore::singleton().await {
                let _ = store.history().record(&conn_id, &sql_clone, exec_time, row_count, is_error).await;
            }
        })
        .detach();
    })
    .detach();
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
        connection_id: state.active_connection_id.clone().unwrap_or_default(),
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

/// Switch the active database / schema for the connection and reload its tables.
pub fn select_database(database: &str, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection.clone() {
        Some(conn) => conn,
        None => {
            AppState::update_status(cx, "Not connected".to_string());
            return;
        }
    };
    let db = database.to_string();

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
            state.databases = databases;
            state.tables = tables;
            state.active_database = Some(db);
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
    cx.spawn(async move |cx| {
        if let Ok(store) = AppStore::singleton().await {
            let _ = store.connections().save(&info, &password).await;
            let _ = store.connections().set_ssh_password(&info.id, &ssh_password);
            let _ = store.connections().set_ssh_passphrase(&info.id, &ssh_key_passphrase);
        }
        cx.update_global::<AppState, _>(|state, _cx| {
            state.connection_state = ConnectionStatus::Connecting;
            state.status_message = format!("Connecting to {}...", info.name);
        });
        connect_async(info, cx).await;
    })
    .detach();
}

/// Refresh the tables list for the active connection.
pub fn refresh_tables(cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection.clone() {
        Some(conn) => conn,
        None => return,
    };
    let db = cx.global::<AppState>().active_database.clone();

    cx.spawn(async move |cx| {
        let tables = dbstudio_db::list_tables(&conn, db.as_deref().unwrap_or(""))
            .await
            .unwrap_or_default();
        cx.update_global::<AppState, _>(|state, _cx| {
            state.tables = tables;
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

/// Load the full schema (columns/indexes/fks) for a table.
pub fn load_table_schema(table: &str, schema: Option<&str>, cx: &mut App) {
    let conn = match cx.global::<AppState>().active_connection.clone() {
        Some(conn) => conn,
        None => return,
    };
    let table = table.to_string();
    let schema = schema.map(|s| s.to_string());

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
            state.table_schemas.insert(table, table_schema);
        });
    })
    .detach();
}

/// Build a SELECT query with identifier quoting appropriate for the active database type.
pub fn build_select_query(table: &str, schema: Option<&str>, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection
        .as_ref()
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

/// Quote a SQL identifier (table/column name) based on the active database type.
pub fn quote_ident(name: &str, cx: &App) -> String {
    let db_type = cx
        .global::<AppState>()
        .active_connection
        .as_ref()
        .map(|c| c.db_type())
        .unwrap_or(DatabaseType::SQLite);

    quote_ident_for(db_type, name)
}

async fn connect_async(info: ConnectionInfo, cx: &mut AsyncApp) {
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

            let active_database = databases.iter()
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
                state.active_connection = Some(conn);
                state.active_connection_name = Some(info.name.clone());
                state.active_connection_id = Some(info.id.clone());
                state.connection_state = ConnectionStatus::Connected;
                state.active_database = active_database;
                state.databases = databases;
                state.tables = tables;
                state.status_message = format!("Connected to {}", info.name);
            });
        }
        Err(e) => {
            tracing::warn!("Connection failed: {:?}", e);
            cx.update_global::<AppState, _>(|state, _cx| {
                state.connection_state = ConnectionStatus::Disconnected;
                state.status_message = format!("Connection failed: {:?}", e);
            });
        }
    }
}


