pub mod loader;


/// Plugin metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub db_type: String,
}

/// Plugin ABI version for compatibility checking.
///
/// v2 adds `DatabasePlugin::current_database` to the trait (vtable change).
pub const PLUGIN_ABI_VERSION: u32 = 2;

/// Trait that database driver plugins must implement.
///
/// # Safety
///
/// The implementor must ensure that the plugin遵守 the ABI contract:
/// - `abi_version()` must return `PLUGIN_ABI_VERSION`
/// - All function pointers must be valid for the plugin's lifetime
/// - The plugin must not violate Rust's safety guarantees
pub unsafe trait DatabasePlugin: Send + Sync {
    /// Get the ABI version this plugin was built against.
    fn abi_version(&self) -> u32;

    /// Get plugin metadata.
    fn info(&self) -> PluginInfo;

    /// Check if the plugin can handle the given database type.
    fn can_handle(&self, db_type: &str) -> bool;

    /// Connect to a database.
    ///
    /// # Arguments
    /// * `config` - JSON-encoded connection configuration
    /// * `password` - Database password
    ///
    /// # Returns
    /// JSON-encoded connection handle or error
    fn connect(&self, config: &str, password: &str) -> anyhow::Result<String>;

    /// Execute a SQL query.
    ///
    /// # Arguments
    /// * `handle` - Connection handle from `connect()`
    /// * `sql` - SQL query to execute
    ///
    /// # Returns
    /// JSON-encoded query result
    fn execute(&self, handle: &str, sql: &str) -> anyhow::Result<String>;

    /// List databases on the server.
    fn list_databases(&self, handle: &str) -> anyhow::Result<String>;

    /// List tables in the current database.
    fn list_tables(&self, handle: &str, schema: &str) -> anyhow::Result<String>;

    /// List columns for a table.
    fn list_columns(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String>;

    /// List indexes for a table.
    fn list_indexes(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String>;

    /// List foreign keys for a table.
    fn list_foreign_keys(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String>;

    /// Get CREATE TABLE SQL for a table.
    fn get_create_table_sql(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String>;

    /// Switch to a different database.
    fn switch_database(&self, handle: &str, database: &str) -> anyhow::Result<()>;

    /// Get the name of the currently selected database.
    ///
    /// Defaults to an empty string. Plugins wrapping an engine should report
    /// the real current database (or return an error) instead of relying on a
    /// dialect-specific query such as `SELECT DATABASE()`.
    fn current_database(&self, handle: &str) -> anyhow::Result<String> {
        let _ = handle;
        Ok(String::new())
    }

    /// Check if the connection is alive.
    fn ping(&self, handle: &str) -> anyhow::Result<()>;

    /// Close a connection.
    fn close(&self, handle: &str) -> anyhow::Result<()>;
}

/// Plugin entry point function type.
///
/// The plugin library must export a function with this signature:
///
/// ```text
/// #[no_mangle]
/// pub extern "Rust" fn create_plugin() -> Box<dyn DatabasePlugin> {
///     // create and return your plugin instance
/// }
/// ```
pub type CreatePluginFn = fn() -> Box<dyn DatabasePlugin>;
