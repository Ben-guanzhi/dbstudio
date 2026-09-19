use anyhow::Result;
use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_core::result::SqlResult;
use dbstudio_core::schema::*;

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

impl Connection {
    pub fn db_type(&self) -> DatabaseType {
        match &self.inner {
            ConnectionInner::Sqlite(_) => DatabaseType::SQLite,
            ConnectionInner::MySql(_) => DatabaseType::MySQL,
            ConnectionInner::Postgres(_) => DatabaseType::PostgreSQL,
            ConnectionInner::Mssql(_) => DatabaseType::MSSQL,
            ConnectionInner::Oracle(_) => DatabaseType::Oracle,
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

pub async fn connect(
    config: &ConnectionConfig,
    password: &str,
    ssh_password: Option<&str>,
    ssh_key_passphrase: Option<&str>,
) -> Result<Connection> {
    let mut config = config.clone();

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