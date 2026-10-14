use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::{
    abi::PluginTable, bridge::PluginBridge, DatabasePlugin, PluginAbiVersionFn, PluginEntryFn,
    PluginInfo, PLUGIN_ABI_VERSION,
};

/// A loaded plugin with its library handle.
struct LoadedPlugin {
    _library: libloading::Library,
    plugin: Arc<dyn DatabasePlugin>,
}

/// Plugin manager that handles loading and managing database driver plugins.
pub struct PluginManager {
    plugins: HashMap<String, LoadedPlugin>,
    plugin_dirs: Vec<PathBuf>,
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new() -> Self {
        let mut plugin_dirs = Vec::new();

        // Add default plugin directories
        if let Some(app_dir) = dirs::data_local_dir() {
            plugin_dirs.push(app_dir.join("dbstudio").join("plugins"));
        }
        if let Some(config_dir) = dirs::config_dir() {
            plugin_dirs.push(config_dir.join("dbstudio").join("plugins"));
        }

        Self {
            plugins: HashMap::new(),
            plugin_dirs,
        }
    }

    /// Add a directory to search for plugins.
    pub fn add_plugin_dir(&mut self, dir: PathBuf) {
        self.plugin_dirs.push(dir);
    }

    /// Load all plugins from the configured directories.
    pub fn load_all(&mut self) -> Result<()> {
        let dirs: Vec<PathBuf> = self.plugin_dirs.clone();

        for dir in &dirs {
            if !dir.exists() {
                continue;
            }

            let entries = std::fs::read_dir(dir)
                .with_context(|| format!("Failed to read plugin directory: {}", dir.display()))?;

            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                if is_plugin_library(&path) {
                    match self.load_plugin(&path) {
                        Ok(info) => {
                            info!("Loaded plugin: {} v{}", info.name, info.version);
                        }
                        Err(e) => {
                            warn!("Failed to load plugin {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single plugin from a path.
    pub fn load_plugin(&mut self, path: &Path) -> Result<PluginInfo> {
        unsafe {
            let library = libloading::Library::new(path)
                .with_context(|| format!("Failed to load library: {}", path.display()))?;

            // Probe the version through a plain `fn() -> u32` before touching
            // any layout. Reading it off a constructed trait object would
            // dispatch through the plugin's own vtable — the very thing that is
            // mislaid when the versions differ — so that check was undefined
            // behaviour before it could ever fail.
            let abi_version: libloading::Symbol<PluginAbiVersionFn> = library
                .get(b"dbstudio_plugin_abi_version")
                .with_context(|| {
                    "Plugin does not export 'dbstudio_plugin_abi_version'; rebuild it against the \
                     current plugin ABI (v4 reads a C function table)"
                })?;
            if abi_version() != PLUGIN_ABI_VERSION {
                return Err(anyhow::anyhow!(
                    "Plugin ABI version mismatch: expected {}, got {}",
                    PLUGIN_ABI_VERSION,
                    abi_version()
                ));
            }

            let entry: libloading::Symbol<PluginEntryFn> = library
                .get(b"dbstudio_plugin_entry")
                .with_context(|| "Plugin does not export 'dbstudio_plugin_entry' function")?;

            let table = validate_table(entry())?;

            // The table is read by value here: `PluginTable` holds only
            // function pointers, and the library handle is kept alive by
            // `LoadedPlugin`, so they stay valid for the plugin's lifetime.
            let plugin: Arc<dyn DatabasePlugin> = Arc::new(PluginBridge { table });

            let info = plugin.info();
            let plugin_name = info.name.clone();

            let loaded = LoadedPlugin {
                _library: library,
                plugin,
            };

            self.plugins.insert(plugin_name, loaded);

            Ok(info)
        }
    }

    /// Get a plugin by database type.
    pub fn get_plugin(&self, db_type: &str) -> Option<Arc<dyn DatabasePlugin>> {
        for loaded in self.plugins.values() {
            if loaded.plugin.can_handle(db_type) {
                return Some(loaded.plugin.clone());
            }
        }
        None
    }

    /// Get a plugin by name.
    pub fn get_plugin_by_name(&self, name: &str) -> Option<Arc<dyn DatabasePlugin>> {
        self.plugins.get(name).map(|p| p.plugin.clone())
    }

    /// List all loaded plugins.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.plugins.values().map(|p| p.plugin.info()).collect()
    }

    /// Check if any plugin can handle the given database type.
    pub fn can_handle(&self, db_type: &str) -> bool {
        self.plugins.values().any(|p| p.plugin.can_handle(db_type))
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Read and validate a table pointer returned by a plugin's entry function.
///
/// Split out from `load_plugin` so the checks that protect against a broken
/// plugin module can be exercised without `libloading`.
pub(crate) fn validate_table(table_ptr: *const PluginTable) -> Result<PluginTable> {
    if table_ptr.is_null() {
        return Err(anyhow::anyhow!(
            "Plugin entry returned a null table pointer"
        ));
    }
    let table = unsafe { *table_ptr };
    if table.free_string as usize == 0 || table.info as usize == 0 || table.can_handle as usize == 0
    {
        return Err(anyhow::anyhow!(
            "Plugin table is missing mandatory slots (free_string, info, can_handle)"
        ));
    }
    Ok(table)
}

/// Check if a file path looks like a plugin library.
fn is_plugin_library(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match std::env::consts::OS {
        "windows" => ext == "dll",
        "macos" => ext == "dylib",
        _ => ext == "so",
    }
}
