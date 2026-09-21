use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
};

use crate::state::AppState;

pub enum FooterEvent {
    ExportDatabase,
    ImportDatabase,
}

impl EventEmitter<FooterEvent> for FooterBar {}

pub struct FooterBar {
    window_id: u64,
    connection_state: crate::state::ConnectionStatus,
    active_database: Option<String>,
    status_message: String,
    show_tables: bool,
    show_history: bool,
    vim_mode: bool,
    safe_mode: bool,
    _subscriptions: Vec<Subscription>,
}

impl FooterBar {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| Self::new(window_id, cx))
    }

    fn new(window_id: u64, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            let ws = state.window_state(this.window_id);
            this.connection_state = state.connection_state_for(this.window_id);
            this.active_database = state.active_database_for(this.window_id).cloned();
            this.status_message = state.status_message.clone();
            this.show_tables = ws.show_tables;
            this.show_history = ws.show_history;
            this.vim_mode = state.vim_mode;
            this.safe_mode = state.safe_mode;
            cx.notify();
        })];

        let state = cx.global::<AppState>();
        let ws = state.window_state(window_id);
        Self {
            window_id,
            connection_state: state.connection_state_for(window_id),
            active_database: state.active_database_for(window_id).cloned(),
            status_message: state.status_message.clone(),
            show_tables: ws.show_tables,
            show_history: ws.show_history,
            vim_mode: state.vim_mode,
            safe_mode: state.safe_mode,
            _subscriptions,
        }
    }
}

impl Render for FooterBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_connected = self.connection_state == crate::state::ConnectionStatus::Connected;

        let tables_button = Button::new("footer-tables")
            .icon(Icon::new(if self.show_tables {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeft
            }))
            .small()
            .ghost()
            .tooltip("Toggle Tables Panel")
            .disabled(!is_connected)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                crate::state::toggle_tables(this.window_id, cx);
            }));

        let history_button = Button::new("footer-history")
            .icon(Icon::new(if self.show_history {
                IconName::PanelRightClose
            } else {
                IconName::PanelRight
            }))
            .small()
            .ghost()
            .tooltip("Toggle History Panel")
            .disabled(!is_connected)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                crate::state::toggle_history(this.window_id, cx);
            }));

        let export_button = Button::new("footer-export")
            .icon(Icon::new(IconName::ArrowDown))
            .small()
            .ghost()
            .tooltip("Export Database")
            .disabled(!is_connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                cx.emit(FooterEvent::ExportDatabase);
            }));

        let import_button = Button::new("footer-import")
            .icon(Icon::new(IconName::ArrowUp))
            .small()
            .ghost()
            .tooltip("Import Database")
            .disabled(!is_connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                cx.emit(FooterEvent::ImportDatabase);
            }));

        // Status readout: connection status + database + message
        let status_left = {
            let dot_color = match self.connection_state {
                crate::state::ConnectionStatus::Connected => cx.theme().button_success,
                crate::state::ConnectionStatus::Connecting
                | crate::state::ConnectionStatus::Disconnecting => cx.theme().button_warning,
                crate::state::ConnectionStatus::Disconnected => cx.theme().muted_foreground,
            };
            h_flex()
                .items_center()
                .gap_1()
                .child(div().size(px(7.0)).rounded_full().bg(dot_color))
                .when(is_connected, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{}",
                                self.active_database.clone().unwrap_or_default()
                            )),
                    )
                })
                .when(!self.status_message.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(" · {}", self.status_message)),
                    )
                })
        };

        // Right cluster: vim/safe badges + panel toggles + export/import
        let right_cluster = if is_connected {
            h_flex()
                .items_center()
                .gap_1()
                .when(self.safe_mode, |d| {
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
                .when(self.vim_mode, |d| {
                    d.child(
                        div()
                            .px_1()
                            .rounded(px(4.0))
                            .bg(cx.theme().muted.opacity(0.3))
                            .child(div().text_xs().font_bold().child("VIM")),
                    )
                })
                .child(tables_button)
                .child(history_button)
                .child(import_button)
                .child(export_button)
        } else {
            h_flex().gap_1()
        };

        div()
            .id("footer-bar")
            .w_full()
            .h(px(28.0))
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(cx.theme().title_bar)
            .border_t_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(status_left)
            .child(right_cluster)
    }
}
