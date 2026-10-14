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

    fn get_create_table_sql(
        &self,
        _handle: &str,
        _table: &str,
        _schema: &str,
    ) -> anyhow::Result<String> {
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
