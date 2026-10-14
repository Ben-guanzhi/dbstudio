use std::sync::Arc;

use anyhow::Result;
use dbstudio_core::error::DbError;
use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::*;
use dbstudio_plugin::DatabasePlugin;

use crate::mssql;
use crate::mysql;
use crate::oracle;
use crate::postgres;
use crate::sqlite;
use crate::tunnel::{SshTunnel, TunnelGuard};

/// Macro to delegate method calls to the underlying connection type.
/// This eliminates the repetitive match patterns in the Connection enum.
///
/// ARCHITECTURAL DEBT (known, accepted): Rust 1.91 does not support
/// `async fn` in traits as dyn-compatible, so `ConnectionInner` cannot
/// hold `Arc<dyn SqlDriver>` and dispatch through a trait object. This
/// macro is the explicit-dispatch fallback: each call site expands to a
/// 6-arm match, which is Θ(1) jump-table cost but duplicates the dispatch
/// shape 11 times. When the toolchain gains native async-trait dyn support,
/// replace this macro with a single `Arc<dyn SqlDriver>` field and delete
/// the match arms. Do not add new dispatch sites — extend the macro.
macro_rules! delegate_to_driver {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match &$self.inner {
            ConnectionInner::Sqlite(c) => c.$method($($arg),*).await,
            ConnectionInner::MySql(c) => c.$method($($arg),*).await,
            ConnectionInner::Postgres(c) => c.$method($($arg),*).await,
            ConnectionInner::Mssql(c) => c.$method($($arg),*).await,
            ConnectionInner::Oracle(c) => c.$method($($arg),*).await,
            ConnectionInner::Plugin(c) => c.$method($($arg),*).await,
        }
    };
}

/// Enum-based connection dispatch, avoiding trait objects
/// (native `async fn` in traits is not dyn-compatible on Rust 1.91).
enum ConnectionInner {
    Sqlite(sqlite::SqliteConnection),
    MySql(mysql::MySqlConnection),
    Postgres(postgres::PostgresConnection),
    Mssql(mssql::MssqlConnection),
    Oracle(oracle::OracleConnection),
    Plugin(PluginConnection),
}

/// A live database connection.
///
/// Wraps the driver-specific [`ConnectionInner`] plus an optional SSH tunnel
/// guard. The guard is kept only for the lifetime of this handle, so dropping
/// the connection tears down its port-forward instead of leaking it for the
/// rest of the process.
pub struct Connection {
    inner: ConnectionInner,
    /// Kept to pin the SSH tunnel for the connection's lifetime; never read.
    #[allow(dead_code)]
    _tunnel: Option<TunnelGuard>,
}

/// Wrapper for plugin-based connections.
struct PluginConnection {
    plugin: Arc<dyn DatabasePlugin>,
    handle: String,
    /// Database type declared by the plugin (`PluginInfo::db_type`), e.g.
    /// `"mysql"` for wrappers around built-in engines, or a free-form string
    /// for custom engines.
    declared_type: String,
}

impl PluginConnection {
    fn new(plugin: Arc<dyn DatabasePlugin>, handle: String) -> Self {
        let declared_type = plugin.info().db_type.clone();
        Self {
            plugin,
            handle,
            declared_type,
        }
    }

    /// Resolve the declared plugin type to a built-in [`DatabaseType`] when it
    /// names one of the known engines, so dialect handling (identifier quoting,
    /// SELECT shape, export) behaves correctly.
    fn effective_db_type(&self) -> DatabaseType {
        if let Ok(t) = self.declared_type.parse() {
            return t;
        }
        // Custom engines have no built-in dialect; SQLite's backtick quoting
        // and `LIMIT`-based SELECT shape is the least-wrong default.
        DatabaseType::SQLite
    }
}

impl Drop for PluginConnection {
    fn drop(&mut self) {
        let _ = self.plugin.close(&self.handle);
    }
}

impl Connection {
    pub fn db_type(&self) -> DatabaseType {
        match &self.inner {
            ConnectionInner::Sqlite(_) => DatabaseType::SQLite,
            ConnectionInner::MySql(_) => DatabaseType::MySQL,
            ConnectionInner::Postgres(_) => DatabaseType::PostgreSQL,
            ConnectionInner::Mssql(_) => DatabaseType::MSSQL,
            ConnectionInner::Oracle(_) => DatabaseType::Oracle,
            ConnectionInner::Plugin(c) => c.effective_db_type(),
        }
    }

    /// The raw database type declared by the plugin, for connections opened via
    /// [`connect_plugin`]. Returns `None` for built-in drivers.
    pub fn plugin_db_type(&self) -> Option<&str> {
        match &self.inner {
            ConnectionInner::Plugin(c) => Some(&c.declared_type),
            _ => None,
        }
    }

    pub async fn ping(&self) -> std::result::Result<(), DbError> {
        delegate_to_driver!(self, ping).map_err(|e| DbError::from_anyhow(&e))
    }

    pub async fn execute(&self, sql: &str) -> std::result::Result<SqlResult, DbError> {
        delegate_to_driver!(self, execute, sql).map_err(|e| DbError::from_anyhow(&e))
    }

    /// Run a SELECT for an export, without the interactive display cap where
    /// the driver can support it. SQLite streams every row; the other engines
    /// still come through the capped [`execute`], and export turns their
    /// `truncated` flag into a `partial` dump rather than a silent lie.
    pub async fn execute_export(&self, sql: &str) -> std::result::Result<SqlResult, DbError> {
        let result = match &self.inner {
            ConnectionInner::Sqlite(c) => c.execute_export(sql).await,
            _ => delegate_to_driver!(self, execute, sql),
        };
        result.map_err(|e| DbError::from_anyhow(&e))
    }

    pub async fn current_database(&self) -> std::result::Result<String, DbError> {
        delegate_to_driver!(self, current_database).map_err(|e| DbError::from_anyhow(&e))
    }

    pub async fn switch_database(&self, database: &str) -> std::result::Result<(), DbError> {
        delegate_to_driver!(self, switch_database, database).map_err(|e| DbError::from_anyhow(&e))
    }
}

/// Create a connection from a plugin.
pub fn connect_plugin(
    plugin: Arc<dyn DatabasePlugin>,
    config: &ConnectionConfig,
    password: &str,
) -> Result<Connection> {
    let config_json = serde_json::to_string(config)?;
    let handle = plugin.connect(&config_json, password)?;

    Ok(Connection {
        inner: ConnectionInner::Plugin(PluginConnection::new(plugin, handle)),
        _tunnel: None,
    })
}

/// Emit one JSON-forwarding catalog method on `PluginConnection`: call the
/// same-named method on the plugin trait and decode the reply into `$ret`.
/// The `schema_opt` arm unwraps the engine-side `Option<&str>` schema to the
/// empty string the plugin wire format uses.
macro_rules! plugin_catalog {
    // `schema_opt` sits outside the argument group so the two argument shapes
    // parse without ambiguity (a trailing flag inside the group would be
    // indistinguishable from one more `&str` argument).
    ($method:ident, $plugin_method:ident, $ret:ty, schema_opt, ($($arg:ident),+)) => {
        pub async fn $method(&self, $($arg: &str),+, schema: Option<&str>) -> Result<Vec<$ret>> {
            let result_json = self
                .plugin
                .$plugin_method(&self.handle, $($arg),+, schema.unwrap_or(""))?;
            serde_json::from_str(&result_json).map_err(Into::into)
        }
    };
    ($method:ident, $plugin_method:ident, $ret:ty, ($($arg:ident),*)) => {
        pub async fn $method(&self, $($arg: &str),*) -> Result<Vec<$ret>> {
            let result_json = self.plugin.$plugin_method(&self.handle, $($arg),*)?;
            serde_json::from_str(&result_json).map_err(Into::into)
        }
    };
}

impl PluginConnection {
    pub async fn ping(&self) -> Result<()> {
        self.plugin.ping(&self.handle)?;
        Ok(())
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        let result_json = self.plugin.execute(&self.handle, sql)?;
        let result: SqlResult = serde_json::from_str(&result_json)?;
        Ok(result)
    }

    pub async fn current_database(&self) -> Result<String> {
        // Delegate to the plugin. Built-in engines implement their own
        // dialect-specific query in `DatabasePlugin::current_database`;
        // plugins that don't override it report an empty string.
        self.plugin.current_database(&self.handle)
    }

    pub async fn switch_database(&self, database: &str) -> Result<()> {
        self.plugin.switch_database(&self.handle, database)?;
        Ok(())
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        // Plugins don't support schemas by default
        Ok(Vec::new())
    }

    // The five catalog endpoints share one wire shape (call → JSON reply →
    // decode); the macro keeps them in lockstep with the plugin trait.
    plugin_catalog!(list_databases, list_databases, DatabaseInfo, ());
    plugin_catalog!(list_tables, list_tables, TableInfo, (schema));
    plugin_catalog!(list_columns, list_columns, ColumnInfo, schema_opt, (table));
    plugin_catalog!(list_indexes, list_indexes, IndexInfo, schema_opt, (table));
    plugin_catalog!(
        list_foreign_keys,
        list_foreign_keys,
        ForeignKeyInfo,
        schema_opt,
        (table)
    );
    pub async fn get_create_table_sql(&self, table: &str, schema: Option<&str>) -> Result<String> {
        let schema_str = schema.unwrap_or("");
        let sql = self
            .plugin
            .get_create_table_sql(&self.handle, table, schema_str)?;
        Ok(sql)
    }
}

pub async fn connect(
    config: &ConnectionConfig,
    password: &str,
    ssh_password: Option<&str>,
    ssh_key_passphrase: Option<&str>,
) -> std::result::Result<Connection, DbError> {
    connect_inner(config, password, ssh_password, ssh_key_passphrase)
        .await
        .map_err(|e| DbError::connect(e.to_string()))
}

async fn connect_inner(
    config: &ConnectionConfig,
    password: &str,
    ssh_password: Option<&str>,
    ssh_key_passphrase: Option<&str>,
) -> Result<Connection> {
    let mut config = config.clone();

    // Route through a driver plugin when the connection names one. Plugins
    // receive the full config JSON and manage the connection themselves, so
    // the SSH tunnel path is skipped for plugin-backed connections.
    if let Some(plugin_name) = &config.plugin_name {
        let plugin = crate::plugin_manager::plugin_manager()
            .get_plugin_by_name(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("driver plugin '{plugin_name}' is not loaded"))?;
        return connect_plugin(plugin, &config, password);
    }

    // SQLite is file-based, so there is no endpoint to forward.
    let mut tunnel_guard = None;
    if !config.db_type.is_file_based() {
        if let Some(tunnel) = SshTunnel::prepare(&config)? {
            // Route the driver through the tunnel's loopback listener instead of
            // dialing the remote host directly. `Endpoint::resolve` picks the
            // rewritten host/port up, so drivers need no tunnel awareness.
            let (local, guard) = tunnel.open(ssh_password, ssh_key_passphrase).await?;
            config.host = local.host;
            config.port = local.port;
            tunnel_guard = Some(guard);
        }
    }

    let connection = match config.db_type {
        DatabaseType::SQLite => {
            ConnectionInner::Sqlite(sqlite::SqliteConnection::open(&config).await?)
        }
        DatabaseType::MySQL => {
            ConnectionInner::MySql(mysql::MySqlConnection::open(&config, password).await?)
        }
        DatabaseType::PostgreSQL => {
            ConnectionInner::Postgres(postgres::PostgresConnection::open(&config, password).await?)
        }
        DatabaseType::MSSQL => {
            ConnectionInner::Mssql(mssql::MssqlConnection::open(&config, password).await?)
        }
        DatabaseType::Oracle => {
            ConnectionInner::Oracle(oracle::OracleConnection::open(&config, password).await?)
        }
    };
    Ok(Connection {
        inner: connection,
        _tunnel: tunnel_guard,
    })
}

pub async fn list_databases(conn: &Connection) -> std::result::Result<Vec<DatabaseInfo>, DbError> {
    delegate_to_driver!(conn, list_databases).map_err(|e| DbError::from_anyhow(&e))
}

pub async fn list_schemas(
    conn: &Connection,
    database: &str,
) -> std::result::Result<Vec<SchemaInfo>, DbError> {
    delegate_to_driver!(conn, list_schemas, database).map_err(|e| DbError::from_anyhow(&e))
}

pub async fn list_tables(
    conn: &Connection,
    schema: &str,
) -> std::result::Result<Vec<TableInfo>, DbError> {
    delegate_to_driver!(conn, list_tables, schema).map_err(|e| DbError::from_anyhow(&e))
}

pub async fn list_columns(
    conn: &Connection,
    table: &str,
    schema: Option<&str>,
) -> std::result::Result<Vec<ColumnInfo>, DbError> {
    delegate_to_driver!(conn, list_columns, table, schema).map_err(|e| DbError::from_anyhow(&e))
}

pub async fn list_indexes(
    conn: &Connection,
    table: &str,
    schema: Option<&str>,
) -> std::result::Result<Vec<IndexInfo>, DbError> {
    delegate_to_driver!(conn, list_indexes, table, schema).map_err(|e| DbError::from_anyhow(&e))
}

pub async fn list_foreign_keys(
    conn: &Connection,
    table: &str,
    schema: Option<&str>,
) -> std::result::Result<Vec<ForeignKeyInfo>, DbError> {
    delegate_to_driver!(conn, list_foreign_keys, table, schema)
        .map_err(|e| DbError::from_anyhow(&e))
}

pub async fn get_create_table_sql(
    conn: &Connection,
    table: &str,
    schema: Option<&str>,
) -> std::result::Result<String, DbError> {
    delegate_to_driver!(conn, get_create_table_sql, table, schema)
        .map_err(|e| DbError::from_anyhow(&e))
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
