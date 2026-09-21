mod assets;
mod keybindings;
mod themes;
mod window;
mod workspace;

use gpui::{actions, App, AppContext as _};
use gpui_component::{theme, Root};
use themes::*;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};
use window::*;
use workspace::*;

actions!(
    window,
    [
        NewWindow,
        Quit,
        ToggleTheme,
        OpenCommandPalette,
        FormatSql,
        ToggleComment,
        ToggleAiPanel,
        UndoLastEdit,
        RedoLastEdit,
        DuplicateLine,
        MoveLineUp,
        MoveLineDown
    ]
);

fn open_workspace_window(cx: &mut App) {
    let window_options = get_window_options(cx);
    cx.open_window(window_options, |win, cx| {
        gpui_component::init(cx);
        theme::init(cx);
        dbstudio_ui::state::init(cx);
        load_theme_mode(cx);

        let workspace_view = Workspace::view(win, cx);
        cx.new(|cx| Root::new(workspace_view, win, cx))
    })
    .unwrap();
}

fn init_logging() {
    let debug = std::env::args().any(|arg| arg == "--debug" || arg == "-d");

    let filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true))
        .with(filter)
        .init();
}

fn main() {
    init_logging();

    // Register the dbstudio:// URL scheme (per-user) at startup.
    if let Err(e) = dbstudio_ui::url_scheme_registry::register_url_scheme() {
        tracing::debug!("Failed to register URL scheme: {}", e);
    }

    tracing::info!(
        "Starting {} v{}",
        dbstudio_core::APP_NAME,
        env!("CARGO_PKG_VERSION")
    );

    let application = gpui_platform::application().with_assets(assets::CombinedAssets);

    application.run(|cx: &mut App| {
        cx.on_window_closed(|cx, _window_id| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        open_workspace_window(cx);

        cx.on_action(|_: &NewWindow, cx| {
            open_workspace_window(cx);
        });
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &ToggleTheme, cx| {
            toggle_color_mode(None, cx);
        });
        cx.on_action(|_: &OpenCommandPalette, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &FormatSql, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &ToggleComment, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &ToggleAiPanel, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &UndoLastEdit, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &RedoLastEdit, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &DuplicateLine, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &MoveLineUp, _cx| {
            // Handled by workspace via on_action on the root div
        });
        cx.on_action(|_: &MoveLineDown, _cx| {
            // Handled by workspace via on_action on the root div
        });
        keybindings::init(cx);

        cx.activate(true);
    });
}
