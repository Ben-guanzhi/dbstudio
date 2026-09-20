use std::sync::Arc;

use anyhow::Result;
use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_core::result::SqlResult;
use dbstudio_db::Connection;
use tokio::sync::RwLock;

use crate::protocol::{CallToolResult, Tool};

/// Shared MCP server state.
pub struct McpState {
    pub connection: RwLock<Option<Arc<Connection>>>,
    pub config: RwLock<Option<ConnectionConfig>>,
    pub password: RwLock<String>,
}

impl McpState {
    pub fn new() -> Self {
        Self {
            connection: RwLock::new(None),
            config: RwLock::new(None),
            password: RwLock::new(String::new()),
        }
    }
}

/// Get the list of available MCP tools.
pub fn list_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "connect".into(),
            description: "Connect to a database. Supported db_type: sqlite, mysql, postgresql, mssql, oracle.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "db_type": { "type": "string", "enum": ["sqlite", "mysql", "postgresql", "mssql", "oracle"] },
                    "host": { "type": "string" },
                    "port": { "type": "integer" },
                    "database": { "type": "string" },
                    "username": { "type": "string" },
                    "password": { "type": "string" }
                },
                "required": ["db_type", "password"]
            }),
        },
        Tool {
            name: "disconnect".into(),
            description: "Disconnect from the current database.".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "list_databases".into(),
            description: "List all databases on the connected server.".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "list_tables".into(),
            description: "List all tables in the current database.".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "list_columns".into(),
            description: "List columns for a table with their types.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": { "type": "string" },
                    "schema": { "type": "string" }
                },
                "required": ["table"]
            }),
        },
        Tool {
            name: "list_indexes".into(),
            description: "List indexes for a table.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": { "type": "string" },
                    "schema": { "type": "string" }
                },
                "required": ["table"]
            }),
        },
        Tool {
            name: "list_foreign_keys".into(),
            description: "List foreign keys for a table.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": { "type": "string" },
                    "schema": { "type": "string" }
                },
                "required": ["table"]
            }),
        },
        Tool {
            name: "get_create_table_sql".into(),
            description: "Get the CREATE TABLE statement for a table.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": { "type": "string" },
                    "schema": { "type": "string" }
                },
                "required": ["table"]
            }),
        },
        Tool {
            name: "execute".into(),
            description: "Execute a SQL query and return results.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string" }
                },
                "required": ["sql"]
            }),
        },
        Tool {
            name: "switch_database".into(),
            description: "Switch to a different database (MySQL/PostgreSQL).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "database": { "type": "string" }
                },
                "required": ["database"]
            }),
        },
        Tool {
            name: "ping".into(),
            description: "Check if the database connection is alive.".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
    ]
}

/// Handle a tool call.
pub async fn call_tool(
    state: &McpState,
    tool_name: &str,
    arguments: Option<serde_json::Value>,
) -> CallToolResult {
    let args = arguments.unwrap_or_default();

    match tool_name {
        "connect" => handle_connect(state, &args).await,
        "disconnect" => handle_disconnect(state).await,
        "list_databases" => handle_list_databases(state).await,
        "list_tables" => handle_list_tables(state).await,
        "list_columns" => handle_list_columns(state, &args).await,
        "list_indexes" => handle_list_indexes(state, &args).await,
        "list_foreign_keys" => handle_list_foreign_keys(state, &args).await,
        "get_create_table_sql" => handle_get_create_table_sql(state, &args).await,
        "execute" => handle_execute(state, &args).await,
        "switch_database" => handle_switch_database(state, &args).await,
        "ping" => handle_ping(state).await,
        _ => CallToolResult::error(format!("Unknown tool: {}", tool_name)),
    }
}

async fn get_connection(state: &McpState) -> Result<Arc<Connection>, CallToolResult> {
    let conn = state.connection.read().await;
    conn.clone().ok_or_else(|| CallToolResult::error("Not connected. Use 'connect' first.".into()))
}

async fn handle_connect(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let db_type_str = match args.get("db_type").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: db_type".into()),
    };
    let password = match args.get("password").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: password".into()),
    };

    let db_type = match db_type_str {
        "sqlite" => DatabaseType::SQLite,
        "mysql" => DatabaseType::MySQL,
        "postgresql" => DatabaseType::PostgreSQL,
        "mssql" => DatabaseType::MSSQL,
        "oracle" => DatabaseType::Oracle,
        _ => return CallToolResult::error(format!("Unsupported db_type: {}", db_type_str)),
    };

    let mut config = ConnectionConfig::new(db_type, "MCP Connection".into());
    config.host = args.get("host").and_then(|v| v.as_str()).unwrap_or("").into();
    config.port = args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    config.database = args.get("database").and_then(|v| v.as_str()).unwrap_or("").into();
    config.username = args.get("username").and_then(|v| v.as_str()).unwrap_or("").into();

    match dbstudio_db::connect(&config, password, None, None).await {
        Ok(conn) => {
            let mut conn_guard = state.connection.write().await;
            let mut config_guard = state.config.write().await;
            let mut pw_guard = state.password.write().await;
            *conn_guard = Some(Arc::new(conn));
            *config_guard = Some(config);
            *pw_guard = password.to_string();
            CallToolResult::success("Connected successfully.".into())
        }
        Err(e) => CallToolResult::error(format!("Connection failed: {}", e)),
    }
}

async fn handle_disconnect(state: &McpState) -> CallToolResult {
    let mut conn = state.connection.write().await;
    *conn = None;
    CallToolResult::success("Disconnected.".into())
}

async fn handle_list_databases(state: &McpState) -> CallToolResult {
    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::list_databases(&conn).await {
        Ok(databases) => {
            let dbs: Vec<String> = databases.iter().map(|db| db.name.clone()).collect();
            CallToolResult::success(serde_json::to_string_pretty(&dbs).unwrap_or_default())
        }
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_list_tables(state: &McpState) -> CallToolResult {
    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::list_tables(&conn, "").await {
        Ok(tables) => {
            let names: Vec<String> = tables.iter().map(|t| t.name.clone()).collect();
            CallToolResult::success(serde_json::to_string_pretty(&names).unwrap_or_default())
        }
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_list_columns(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let table = match args.get("table").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: table".into()),
    };
    let schema = args.get("schema").and_then(|v| v.as_str());

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::list_columns(&conn, table, schema).await {
        Ok(columns) => CallToolResult::success(serde_json::to_string_pretty(&columns).unwrap_or_default()),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_list_indexes(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let table = match args.get("table").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: table".into()),
    };
    let schema = args.get("schema").and_then(|v| v.as_str());

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::list_indexes(&conn, table, schema).await {
        Ok(indexes) => CallToolResult::success(serde_json::to_string_pretty(&indexes).unwrap_or_default()),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_list_foreign_keys(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let table = match args.get("table").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: table".into()),
    };
    let schema = args.get("schema").and_then(|v| v.as_str());

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::list_foreign_keys(&conn, table, schema).await {
        Ok(fks) => CallToolResult::success(serde_json::to_string_pretty(&fks).unwrap_or_default()),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_get_create_table_sql(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let table = match args.get("table").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: table".into()),
    };
    let schema = args.get("schema").and_then(|v| v.as_str());

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match dbstudio_db::get_create_table_sql(&conn, table, schema).await {
        Ok(sql) => CallToolResult::success(sql),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_execute(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let sql = match args.get("sql").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: sql".into()),
    };

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match conn.execute(sql).await {
        Ok(result) => CallToolResult::success(format_result(&result)),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_switch_database(state: &McpState, args: &serde_json::Value) -> CallToolResult {
    let database = match args.get("database").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return CallToolResult::error("Missing required parameter: database".into()),
    };

    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match conn.switch_database(database).await {
        Ok(()) => CallToolResult::success(format!("Switched to database: {}", database)),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

async fn handle_ping(state: &McpState) -> CallToolResult {
    let conn = match get_connection(state).await {
        Ok(c) => c,
        Err(e) => return e,
    };
    match conn.ping().await {
        Ok(()) => CallToolResult::success("Pong!".into()),
        Err(e) => CallToolResult::error(format!("Failed: {}", e)),
    }
}

fn format_result(result: &SqlResult) -> String {
    match result {
        SqlResult::Query(query) => {
            if query.rows.is_empty() {
                return "Empty result set.".into();
            }
            let mut output = String::new();
            // Header
            let headers: Vec<&str> = query.columns.iter().map(|c| c.name.as_str()).collect();
            output.push_str(&headers.join(" | "));
            output.push('\n');
            output.push_str(&"-".repeat(60));
            output.push('\n');
            // Rows
            for row in &query.rows {
                let vals: Vec<&str> = row.iter().map(|c| {
                    if c.is_null { "NULL" } else { &c.value }
                }).collect();
                output.push_str(&vals.join(" | "));
                output.push('\n');
            }
            output.push_str(&format!("\n({} rows)", query.rows.len()));
            output
        }
        SqlResult::Modified(exec) => {
            format!("Statement executed. Rows affected: {}", exec.rows_affected)
        }
        SqlResult::Error(err) => {
            format!("Error: {}", err.message)
        }
    }
}
