use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Theme;
use gpui_component::ThemeMode;
use std::path::PathBuf;

fn config_path() -> PathBuf {
    dirs::config_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(dbstudio_core::NAMESPACE)
        .join("theme.txt")
}

/// Path of the theme file written by pre-rename `dbclient` installs.
fn legacy_config_path() -> PathBuf {
    dirs::config_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(dbstudio_core::LEGACY_NAMESPACE)
        .join("theme.txt")
}

pub fn toggle_color_mode(window: Option<&mut Window>, cx: &mut App) {
    let mode = cx.theme().mode;
    let new_mode = if mode.is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(new_mode, window, cx);
    save_theme_mode(new_mode);
}

fn save_theme_mode(mode: ThemeMode) {
    let mode_str = if mode.is_dark() { "dark" } else { "light" };
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, mode_str);
}

pub fn load_theme_mode(cx: &mut App) {
    let config = config_path();
    let mode_str = match std::fs::read_to_string(&config) {
        Ok(mode_str) => mode_str,
        // One-time migration: no theme saved in the current namespace yet, so
        // copy over the pre-rename `dbclient` choice if one exists.
        Err(_) => {
            let legacy = legacy_config_path();
            if !legacy.exists() {
                return;
            }
            match std::fs::read_to_string(&legacy) {
                Ok(mode_str) => {
                    if let Some(parent) = config.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&config, &mode_str);
                    mode_str
                }
                Err(_) => return,
            }
        }
    };

    let mode = match mode_str.trim() {
        "dark" => ThemeMode::Dark,
        "light" => ThemeMode::Light,
        _ => return,
    };
    Theme::change(mode, None, cx);
}
