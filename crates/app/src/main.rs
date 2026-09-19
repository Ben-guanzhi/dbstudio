mod assets;
mod themes;
mod window;
mod workspace;

use gpui::{App, AppContext as _, KeyBinding, actions};
use gpui_component::{Root, theme};
use themes::*;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};
use window::*;
use workspace::*;

actions!(window, [Quit, ToggleTheme]);

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

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &ToggleTheme, cx| {
            toggle_color_mode(None, cx);
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-ctrl-t", ToggleTheme, None),
        ]);

        cx.activate(true);
    });
}