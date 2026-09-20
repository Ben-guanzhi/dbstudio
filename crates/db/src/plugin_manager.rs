use std::sync::OnceLock;

use dbstudio_plugin::loader::PluginManager;

static PLUGIN_MANAGER: OnceLock<PluginManager> = OnceLock::new();

/// The process-wide plugin manager, initialized lazily on first access.
///
/// Scans the default plugin directories for driver libraries. Load failures
/// are logged but never fatal: a missing or broken plugin simply means the
/// connection form offers no plugin-driven driver for that engine.
pub fn plugin_manager() -> &'static PluginManager {
    PLUGIN_MANAGER.get_or_init(|| {
        let mut manager = PluginManager::new();
        if let Err(e) = manager.load_all() {
            tracing::warn!("Failed to scan plugin directories: {e}");
        }
        manager
    })
}