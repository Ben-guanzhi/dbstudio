use gpui::{App, KeyBinding};

use crate::{
    DuplicateLine, FormatSql, MoveLineDown, MoveLineUp, NewWindow, OpenCommandPalette,
    OpenSettings, Quit, RedoLastEdit, ShowHelp, ToggleAiPanel, ToggleComment, ToggleTheme,
    UndoLastEdit,
};

/// Register all default keybindings. Pass the action types via [`KeyBinding::new`].
///
/// This is called once from `main::run` after the actions are defined.
///
/// The `secondary` prefix is gpui's cross-platform primary modifier — `Cmd` on
/// macOS and `Ctrl` on Windows/Linux — so a single binding covers both
/// platforms (bare `cmd-` would bind the Windows/Super key instead).
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewWindow, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-ctrl-t", ToggleTheme, None),
        // Quick Open / command palette: Cmd+P & Cmd+K on macOS, Ctrl+P & Ctrl+K elsewhere.
        KeyBinding::new("secondary-p", OpenCommandPalette, None),
        KeyBinding::new("secondary-k", OpenCommandPalette, None),
        KeyBinding::new("f1", ShowHelp, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("shift-alt-f", FormatSql, None),
        KeyBinding::new("ctrl-/", ToggleComment, None),
        KeyBinding::new("secondary-shift-a", ToggleAiPanel, None),
        KeyBinding::new("secondary-shift-d", DuplicateLine, None),
        KeyBinding::new("shift-alt-up", MoveLineUp, None),
        KeyBinding::new("shift-alt-down", MoveLineDown, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-z", UndoLastEdit, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-z", RedoLastEdit, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-z", UndoLastEdit, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", RedoLastEdit, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-z", RedoLastEdit, None),
    ]);
}
