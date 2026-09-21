use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
};

use dbstudio_core::models::Environment;

use crate::state::{AppState, ConnectionStatus};
use crate::utils::toolbar_divider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderEvent {
    ToggleTheme,
    NewConnection,
    ToggleTables,
    ToggleHistory,
    ToggleAi,
    OpenPalette,
    NewWindow,
}

impl EventEmitter<HeaderEvent> for HeaderBar {}

pub struct HeaderBar {
    window_id: u64,
    connection_state: ConnectionStatus,
    active_connection_name: Option<String>,
    active_database: Option<String>,
    environment: Environment,
    safe_mode: bool,
    show_tables: bool,
    show_history: bool,
    _subscriptions: Vec<Subscription>,
}

impl HeaderBar {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| Self::new(window_id, cx))
    }

    fn new(window_id: u64, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            this.refresh_from(state);
            cx.notify();
        })];

        let mut this = Self {
            window_id,
            connection_state: ConnectionStatus::Disconnected,
            active_connection_name: None,
            active_database: None,
            environment: Environment::default(),
            safe_mode: false,
            show_tables: true,
            show_history: false,
            _subscriptions,
        };
        let state = cx.global::<AppState>();
        this.refresh_from(state);
        this
    }

    fn refresh_from(&mut self, state: &AppState) {
        let window_id = self.window_id;
        self.connection_state = state.connection_state_for(window_id);
        self.active_connection_name = state.active_connection_name_for(window_id).cloned();
        self.active_database = state.active_database_for(window_id).cloned();
        self.environment = state
            .active_session_for(window_id)
            .map(|s| s.environment)
            .unwrap_or_default();
        self.safe_mode = state.safe_mode;
        let ws = state.window_state(window_id);
        self.show_tables = ws.show_tables;
        self.show_history = ws.show_history;
    }

    fn env_badge(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, color) = match self.environment {
            Environment::Dev => ("DEV", cx.theme().button_success),
            Environment::Staging => ("STAGING", cx.theme().button_warning),
            Environment::Production => ("PRODUCTION", cx.theme().danger),
        };
        div()
            .flex()
            .items_center()
            .px_1()
            .rounded(px(4.0))
            .bg(color.opacity(0.15))
            .child(div().text_xs().font_bold().text_color(color).child(label))
    }
}

impl Render for HeaderBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.connection_state == ConnectionStatus::Connected;

        let tables_button = Button::new("header-tables")
            .icon(Icon::new(if self.show_tables {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeft
            }))
            .small()
            .ghost()
            .tooltip("Toggle Tables Panel")
            .disabled(!connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::ToggleTables);
            }));

        let history_button = Button::new("header-history")
            .icon(Icon::new(if self.show_history {
                IconName::PanelRightClose
            } else {
                IconName::PanelRight
            }))
            .small()
            .ghost()
            .tooltip("Toggle History Panel")
            .disabled(!connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::ToggleHistory);
            }));

        let search_button = Button::new("header-search")
            .icon(Icon::new(IconName::Search))
            .small()
            .ghost()
            .tooltip("Search / Open Quickly")
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::OpenPalette);
            }));

        let new_window_button = Button::new("header-new-window")
            .icon(Icon::new(IconName::Plus))
            .small()
            .ghost()
            .tooltip("New Window")
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::NewWindow);
            }));

        let ai_button = Button::new("header-ai")
            .icon(Icon::new(IconName::Bot))
            .small()
            .ghost()
            .tooltip("Toggle AI Panel")
            .disabled(!connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::ToggleAi);
            }));

        let theme_button = Button::new("header-theme")
            .icon(Icon::new(IconName::Sun))
            .small()
            .ghost()
            .tooltip("Toggle Theme")
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                cx.emit(HeaderEvent::ToggleTheme);
            }));

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

        // Center information pill (TablePro titlebar style):
        // [env badge] connection name - dbbadge - [SAFE]
        let name = self
            .active_connection_name
            .clone()
            .unwrap_or_else(|| dbstudio_core::APP_NAME.to_string());
        let db = self.active_database.clone();
        let safe = self.safe_mode;
        let info_pill = h_flex()
            .id("header-info-pill")
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded(px(6.0))
            .bg(cx.theme().muted)
            .when(connected, |d| {
                d.child(self.env_badge(cx))
                    .child(div().text_xs().font_medium().child(name.clone()))
                    .when_some(db.clone(), |d, db| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("-"),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::HardDrive)
                                        .size_3()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(div().text_xs().child(db)),
                        )
                    })
                    .when(safe, |d| {
                        d.child(
                            div()
                                .px_1()
                                .rounded(px(4.0))
                                .bg(cx.theme().accent.opacity(0.15))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_bold()
                                        .text_color(cx.theme().accent)
                                        .child("SAFE"),
                                ),
                        )
                    })
            })
            .when(!connected, |d| d.child(div().text_xs().child(name)));

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
                    .gap_0()
                    .child(tables_button)
                    .child(history_button),
            )
            .child(info_pill)
            .child(
                h_flex()
                    .items_center()
                    .gap_0()
                    .child(search_button)
                    .child(new_window_button)
                    .child(ai_button)
                    .child(theme_button)
                    .child(toolbar_divider(cx))
                    .child(minimize_button)
                    .child(maximize_button)
                    .child(close_button),
            )
    }
}
