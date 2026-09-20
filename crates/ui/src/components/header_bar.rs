use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Icon,
    IconName,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
};

use crate::state::{AppState, ConnectionStatus};
use crate::utils::toolbar_divider;

pub enum HeaderEvent {
    ToggleTheme,
    NewConnection,
}

impl EventEmitter<HeaderEvent> for HeaderBar {}

pub struct HeaderBar {
    connection_state: ConnectionStatus,
    active_connection_name: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl HeaderBar {
    pub fn view(_window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            this.connection_state = state.connection_state();
            this.active_connection_name = state.active_connection_name().cloned();
            cx.notify();
        })];

        let state = cx.global::<AppState>();
        Self {
            connection_state: state.connection_state(),
            active_connection_name: state.active_connection_name().cloned(),
            _subscriptions,
        }
    }

    fn on_toggle_theme(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(HeaderEvent::ToggleTheme);
    }
}

impl Render for HeaderBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .active_connection_name
            .clone()
            .unwrap_or_else(|| dbstudio_core::APP_NAME.to_string());

        let theme_button = Button::new("header-theme")
            .icon(Icon::new(IconName::Sun))
            .small()
            .ghost()
            .on_click(cx.listener(Self::on_toggle_theme));

        let github_button = Button::new("header-github")
            .icon(Icon::new(IconName::Github))
            .small()
            .ghost()
            .on_click(|_, _, cx| cx.open_url("https://github.com"));

        let minimize_button = Button::new("window-minimize")
            .icon(Icon::new(IconName::WindowMinimize))
            .small()
            .ghost()
            .on_click(|_, window, _| window.minimize_window());

        let maximize_button = Button::new("window-maximize")
            .icon(Icon::new(IconName::WindowMaximize))
            .small()
            .ghost()
            .on_click(|_, window, _| window.zoom_window());

        let close_button = Button::new("window-close")
            .icon(Icon::new(IconName::WindowClose))
            .small()
            .ghost()
            .on_click(|_, _, cx| cx.quit());

        div()
            .id("header-bar")
            .flex()
            .h_flex()
            .justify_between()
            .items_center()
            .px_2()
            .h(px(36.0))
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Label::new(title).text_xs()),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_0()
                    .child(theme_button)
                    .child(github_button)
                    .child(toolbar_divider(cx))
                    .child(minimize_button)
                    .child(maximize_button)
                    .child(close_button),
            )
    }
}
