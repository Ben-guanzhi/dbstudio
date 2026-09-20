use std::sync::Arc;

use anyhow::Result;
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

    pub async fn ping(&self) -> Result<()> {
        delegate_to_driver!(self, ping)
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        delegate_to_driver!(self, execute, sql)
    }

    pub async fn current_database(&self) -> Result<String> {
        delegate_to_driver!(self, current_database)
    }

    pub async fn switch_database(&self, database: &str) -> Result<()> {
        delegate_to_driver!(self, switch_database, database)
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

    pub async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let result_json = self.plugin.list_databases(&self.handle)?;
        let dbs: Vec<DatabaseInfo> = serde_json::from_str(&result_json)?;
        Ok(dbs)
    }

    pub async fn list_schemas(&self, _database: &str) -> Result<Vec<SchemaInfo>> {
        // Plugins don't support schemas by default
        Ok(Vec::new())
    }

    pub async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let result_json = self.plugin.list_tables(&self.handle, schema)?;
        let tables: Vec<TableInfo> = serde_json::from_str(&result_json)?;
        Ok(tables)
    }

    pub async fn list_columns(&self, table: &str, schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
        let schema_str = schema.unwrap_or("");
        let result_json = self.plugin.list_columns(&self.handle, table, schema_str)?;
        let columns: Vec<ColumnInfo> = serde_json::from_str(&result_json)?;
        Ok(columns)
    }

    pub async fn list_indexes(&self, table: &str, schema: Option<&str>) -> Result<Vec<IndexInfo>> {
        let schema_str = schema.unwrap_or("");
        let result_json = self.plugin.list_indexes(&self.handle, table, schema_str)?;
        let indexes: Vec<IndexInfo> = serde_json::from_str(&result_json)?;
        Ok(indexes)
    }

    pub async fn list_foreign_keys(&self, table: &str, schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
        let schema_str = schema.unwrap_or("");
        let result_json = self.plugin.list_foreign_keys(&self.handle, table, schema_str)?;
        let fks: Vec<ForeignKeyInfo> = serde_json::from_str(&result_json)?;
        Ok(fks)
    }

    pub async fn get_create_table_sql(&self, table: &str, schema: Option<&str>) -> Result<String> {
        let schema_str = schema.unwrap_or("");
        let sql = self.plugin.get_create_table_sql(&self.handle, table, schema_str)?;
        Ok(sql)
    }
}

pub async fn connect(
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
    if config.db_type != DatabaseType::SQLite {
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
        DatabaseType::SQLite => ConnectionInner::Sqlite(sqlite::SqliteConnection::open(&config).await?),
        DatabaseType::MySQL => ConnectionInner::MySql(mysql::MySqlConnection::open(&config, password).await?),
        DatabaseType::PostgreSQL => ConnectionInner::Postgres(postgres::PostgresConnection::open(&config, password).await?),
        DatabaseType::MSSQL => ConnectionInner::Mssql(mssql::MssqlConnection::open(&config, password).await?),
        DatabaseType::Oracle => ConnectionInner::Oracle(oracle::OracleConnection::open(&config, password).await?),
    };
    Ok(Connection {
        inner: connection,
        _tunnel: tunnel_guard,
    })
}

pub async fn list_databases(conn: &Connection) -> Result<Vec<DatabaseInfo>> {
    delegate_to_driver!(conn, list_databases)
}

pub async fn list_schemas(conn: &Connection, database: &str) -> Result<Vec<SchemaInfo>> {
    delegate_to_driver!(conn, list_schemas, database)
}

pub async fn list_tables(conn: &Connection, schema: &str) -> Result<Vec<TableInfo>> {
    delegate_to_driver!(conn, list_tables, schema)
}

pub async fn list_columns(conn: &Connection, table: &str, schema: Option<&str>) -> Result<Vec<ColumnInfo>> {
    delegate_to_driver!(conn, list_columns, table, schema)
}

pub async fn list_indexes(conn: &Connection, table: &str, schema: Option<&str>) -> Result<Vec<IndexInfo>> {
    delegate_to_driver!(conn, list_indexes, table, schema)
}

pub async fn list_foreign_keys(conn: &Connection, table: &str, schema: Option<&str>) -> Result<Vec<ForeignKeyInfo>> {
    delegate_to_driver!(conn, list_foreign_keys, table, schema)
}

pub async fn get_create_table_sql(conn: &Connection, table: &str, schema: Option<&str>) -> Result<String> {
    delegate_to_driver!(conn, get_create_table_sql, table, schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbstudio_core::models::ConnectionConfig;
    use dbstudio_plugin::{PluginInfo, PLUGIN_ABI_VERSION};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc as StdArc;

    struct MockPlugin {
        declared: String,
        closed: AtomicBool,
    }

    unsafe impl DatabasePlugin for MockPlugin {
        fn abi_version(&self) -> u32 {
            PLUGIN_ABI_VERSION
        }

        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: "mock".to_string(),
                version: "0.1.0".to_string(),
                description: "test plugin".to_string(),
                author: "test".to_string(),
                db_type: self.declared.clone(),
            }
        }

        fn can_handle(&self, db_type: &str) -> bool {
            self.declared == db_type
        }

        fn connect(&self, _config: &str, _password: &str) -> anyhow::Result<String> {
            Ok("handle-1".to_string())
        }

        fn execute(&self, _handle: &str, _sql: &str) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn list_databases(&self, _handle: &str) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn list_tables(&self, _handle: &str, _schema: &str) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn list_columns(&self, _handle: &str, _table: &str, _schema: &str) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn list_indexes(&self, _handle: &str, _table: &str, _schema: &str) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn list_foreign_keys(
            &self,
            _handle: &str,
            _table: &str,
            _schema: &str,
        ) -> anyhow::Result<String> {
            Ok("[]".to_string())
        }

        fn get_create_table_sql(&self, _handle: &str, _table: &str, _schema: &str) -> anyhow::Result<String> {
            Ok(String::new())
        }

        fn switch_database(&self, _handle: &str, _database: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn current_database(&self, handle: &str) -> anyhow::Result<String> {
            Ok(format!("db-{}", handle))
        }

        fn ping(&self, _handle: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn close(&self, _handle: &str) -> anyhow::Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn mock_plugin(declared: &str) -> StdArc<MockPlugin> {
        StdArc::new(MockPlugin {
            declared: declared.to_string(),
            closed: AtomicBool::new(false),
        })
    }

    #[test]
    fn resolves_known_engine_from_declared_type() {
        let plugin = mock_plugin("mysql");
        let config = ConnectionConfig::new(DatabaseType::SQLite, "mock".to_string());
        let conn = connect_plugin(plugin, &config, "pw").unwrap();

        assert_eq!(conn.db_type(), DatabaseType::MySQL);
        assert_eq!(conn.plugin_db_type(), Some("mysql"));
    }

    #[test]
    fn falls_back_to_sqlite_for_unknown_engine() {
        let plugin = mock_plugin("cassandra");
        let config = ConnectionConfig::new(DatabaseType::SQLite, "mock".to_string());
        let conn = connect_plugin(plugin, &config, "pw").unwrap();

        assert_eq!(conn.db_type(), DatabaseType::SQLite);
        assert_eq!(conn.plugin_db_type(), Some("cassandra"));
    }

    #[test]
    fn current_database_delegates_to_plugin() {
        let plugin = mock_plugin("mysql");
        let config = ConnectionConfig::new(DatabaseType::SQLite, "mock".to_string());
        let conn = connect_plugin(plugin, &config, "pw").unwrap();

        assert_eq!(
            smol::block_on(conn.current_database()).unwrap(),
            "db-handle-1"
        );
    }

    #[test]
    fn close_called_on_drop() {
        let plugin = mock_plugin("mysql");
        let config = ConnectionConfig::new(DatabaseType::SQLite, "mock".to_string());
        {
            let _conn = connect_plugin(plugin.clone(), &config, "pw").unwrap();
        }
        assert!(plugin.closed.load(Ordering::SeqCst));
    }

    #[test]
    fn connect_routes_plugin_connections_by_name() {
        let mut config = ConnectionConfig::new(DatabaseType::SQLite, "mock".to_string());
        config.plugin_name = Some("unloaded-plugin".to_string());
        let err = match smol::block_on(connect(&config, "pw", None, None)) {
        Ok(_) => panic!("expected plugin routing error"),
        Err(e) => e,
    };
        assert!(
            err.to_string().contains("is not loaded"),
            "unexpected error: {err}"
        );

        // Built-in connections ignore plugin routing entirely.
        config.plugin_name = None;
        config.database = std::env::temp_dir()
            .join(format!("dbstudio-plugin-routing-{}.db", std::process::id()))
            .to_string_lossy()
            .to_string();
        let result = smol::block_on(connect(&config, "", None, None));
        assert!(result.is_ok());
    }
}