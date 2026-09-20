use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::{DatabasePlugin, PluginInfo, PLUGIN_ABI_VERSION};

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

            let create_plugin: libloading::Symbol<crate::CreatePluginFn> = library
                .get(b"create_plugin")
                .with_context(|| "Plugin does not export 'create_plugin' function")?;

            let plugin = create_plugin();

            // Check ABI version
            if plugin.abi_version() != PLUGIN_ABI_VERSION {
                return Err(anyhow::anyhow!(
                    "Plugin ABI version mismatch: expected {}, got {}",
                    PLUGIN_ABI_VERSION,
                    plugin.abi_version()
                ));
            }

            let info = plugin.info();
            let plugin_name = info.name.clone();

            let loaded = LoadedPlugin {
                _library: library,
                plugin: Arc::from(plugin),
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

/// Check if a file path looks like a plugin library.
fn is_plugin_library(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match std::env::consts::OS {
        "windows" => ext == "dll",
        "macos" => ext == "dylib",
        _ => ext == "so",
    }
}

