use gpui::{App, KeyBinding};

use crate::{DuplicateLine, FormatSql, MoveLineDown, MoveLineUp, OpenCommandPalette, Quit, RedoLastEdit, ToggleAiPanel, ToggleComment, ToggleTheme, UndoLastEdit};

/// Register all default keybindings. Pass the action types via [`KeyBinding::new`].
///
/// This is called once from `main::run` after the actions are defined.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-ctrl-t", ToggleTheme, None),
        KeyBinding::new("cmd-p", OpenCommandPalette, None),
        KeyBinding::new("shift-alt-f", FormatSql, None),
        KeyBinding::new("ctrl-/", ToggleComment, None),
        KeyBinding::new("cmd-shift-a", ToggleAiPanel, None),
        KeyBinding::new("shift-cmd-d", DuplicateLine, None),
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