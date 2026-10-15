use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use dbstudio_plugin::loader::{PluginFile, PluginManager};
use dbstudio_plugin::{DatabasePlugin, PluginInfo};

static PLUGIN_MANAGER: OnceLock<Mutex<PluginManager>> = OnceLock::new();

/// The process-wide plugin manager, initialized lazily on first access.
///
/// Scans the default plugin directories for driver libraries. Load failures
/// are logged but never fatal: a missing or broken plugin simply means the
/// connection form offers no plugin-driven driver for that engine.
///
/// `load_all` needs `&mut`, so the manager sits behind a `Mutex` rather than
/// being handed out by reference — otherwise there would be no way to rescan
/// after a plugin gets trusted at runtime. Callers go through the helpers
/// below, which hold the lock only as long as the single call they wrap.
fn manager() -> &'static Mutex<PluginManager> {
    PLUGIN_MANAGER.get_or_init(|| {
        let mut manager = PluginManager::new();
        if let Err(e) = manager.load_all() {
            tracing::warn!("Failed to scan plugin directories: {e}");
        }
        Mutex::new(manager)
    })
}

/// Run `f` under the plugin manager lock.
fn with_manager<R>(f: impl FnOnce(&mut PluginManager) -> R) -> R {
    let mut guard = manager()
        .lock()
        .expect("the plugin manager mutex was poisoned");
    f(&mut guard)
}

pub fn list_plugins() -> Vec<PluginInfo> {
    with_manager(|m| m.list_plugins())
}

pub fn get_plugin_by_name(name: &str) -> Option<Arc<dyn DatabasePlugin>> {
    with_manager(|m| m.get_plugin_by_name(name))
}

/// Every plugin library on disk, with its trust decision. Drives the plugins
/// section of the settings card, which is how a blocked plugin becomes visible.
pub fn scan_plugins() -> Vec<PluginFile> {
    with_manager(|m| m.scan_plugins())
}

/// Where plugin libraries are looked up, so the UI can point at a real folder.
pub fn plugin_dirs() -> Vec<PathBuf> {
    with_manager(|m| m.plugin_dirs().to_vec())
}

/// Register a plugin library, then rescan so it is usable without a restart.
pub fn trust_plugin(path: &Path) -> Result<String> {
    let hash = with_manager(|m| m.trust_plugin(path))?;
    reload_plugins()?;
    Ok(hash)
}

/// Deregister a plugin library, then rescan so the plugin goes away at once.
pub fn revoke_plugin(path: &Path) -> Result<()> {
    with_manager(|m| m.revoke_plugin(path))?;
    reload_plugins()
}

/// Rescan the plugin directories and replace the loaded set.
///
/// A trust change is not enough on its own: `load_all` ran at startup, so the
/// library would otherwise stay absent (or stay present after a revoke) until
/// the next launch. Failures are logged and returned — `load_all` only ever
/// adds and never clears the map first, so a half-broken scan leaves the
/// plugins that were already loaded in place rather than dropping them.
pub fn reload_plugins() -> Result<()> {
    let mut manager = manager()
        .lock()
        .expect("the plugin manager mutex was poisoned");
    manager.load_all()
}
