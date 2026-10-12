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
/// v3 moves the compatibility check in front of construction: plugins must
/// export the plain `dbstudio_plugin_abi_version` C symbol, which the loader
/// reads before calling `create_plugin()`.
pub const PLUGIN_ABI_VERSION: u32 = 3;

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
    ///
    /// Informational only: the loader verifies the version through the
    /// `dbstudio_plugin_abi_version` C symbol before construction, so this
    /// method is never what makes the compatibility decision.
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

    // ── Catalog/metadata operations ──
    // Every one of these carries a default impl so growing the trait does not
    // break the vtable again (v2 already bumped ABI for exactly this reason):
    // a plugin built against an older ABI keeps loading, and an operation it
    // does not override surfaces as a clean "unsupported" error instead of a
    // link failure. Override what the wrapped engine can actually serve.

    /// List databases on the server.
    fn list_databases(&self, handle: &str) -> anyhow::Result<String> {
        let _ = handle;
        Err(anyhow::anyhow!(
            "list_databases not supported by this plugin"
        ))
    }

    /// List tables in the current database.
    fn list_tables(&self, handle: &str, schema: &str) -> anyhow::Result<String> {
        let _ = (handle, schema);
        Err(anyhow::anyhow!("list_tables not supported by this plugin"))
    }

    /// List columns for a table.
    fn list_columns(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String> {
        let _ = (handle, table, schema);
        Err(anyhow::anyhow!("list_columns not supported by this plugin"))
    }

    /// List indexes for a table.
    fn list_indexes(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String> {
        let _ = (handle, table, schema);
        Err(anyhow::anyhow!("list_indexes not supported by this plugin"))
    }

    /// List foreign keys for a table.
    fn list_foreign_keys(&self, handle: &str, table: &str, schema: &str) -> anyhow::Result<String> {
        let _ = (handle, table, schema);
        Err(anyhow::anyhow!(
            "list_foreign_keys not supported by this plugin"
        ))
    }

    /// Get CREATE TABLE SQL for a table.
    fn get_create_table_sql(
        &self,
        handle: &str,
        table: &str,
        schema: &str,
    ) -> anyhow::Result<String> {
        let _ = (handle, table, schema);
        Err(anyhow::anyhow!(
            "get_create_table_sql not supported by this plugin"
        ))
    }

    /// Switch to a different database.
    fn switch_database(&self, handle: &str, database: &str) -> anyhow::Result<()> {
        let _ = (handle, database);
        Err(anyhow::anyhow!(
            "switch_database not supported by this plugin"
        ))
    }

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

/// C-ABI entry point reporting the plugin's ABI version. Plugins export it as:
///
/// ```text
/// #[no_mangle]
/// pub extern "C" fn dbstudio_plugin_abi_version() -> u32 {
///     dbstudio_plugin::PLUGIN_ABI_VERSION
/// }
/// ```
///
/// The loader reads the version through this symbol *before* calling
/// `create_plugin()`. Asking the constructed trait object for `abi_version()`
/// would dispatch through the plugin's own vtable — the very table that is
/// mislaid when the versions differ — so that check was undefined behavior
/// before it could ever fail.
pub type PluginAbiVersionFn = extern "C" fn() -> u32;

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
///
/// Only called after `dbstudio_plugin_abi_version` has matched, so the vtable
/// the returned object points at is trusted by then.
pub type CreatePluginFn = fn() -> Box<dyn DatabasePlugin>;
