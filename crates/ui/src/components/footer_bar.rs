use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Icon,
    IconName,
    Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

use crate::state::AppState;

pub struct FooterBar {
    connection_state: crate::state::ConnectionStatus,
    active_database: Option<String>,
    status_message: String,
    show_tables: bool,
    show_history: bool,
    _subscriptions: Vec<Subscription>,
}

impl FooterBar {
    pub fn view(_window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            this.connection_state = state.connection_state;
            this.active_database = state.active_database.clone();
            this.status_message = state.status_message.clone();
            this.show_tables = state.show_tables;
            this.show_history = state.show_history;
            cx.notify();
        })];

        let state = cx.global::<AppState>();
        Self {
            connection_state: state.connection_state,
            active_database: state.active_database.clone(),
            status_message: state.status_message.clone(),
            show_tables: state.show_tables,
            show_history: state.show_history,
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
                IconName::PanelLeftOpen
            }))
            .small()
            .ghost()
            .tooltip("Toggle Tables Panel")
            .disabled(!is_connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                crate::state::toggle_tables(cx);
            }));

        let history_button = Button::new("footer-history")
            .icon(Icon::new(if self.show_history {
                IconName::PanelRightClose
            } else {
                IconName::PanelRightOpen
            }))
            .small()
            .ghost()
            .tooltip("Toggle History Panel")
            .disabled(!is_connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                crate::state::toggle_history(cx);
            }));

        let agent_button = Button::new("footer-agent")
            .icon(Icon::new(IconName::Bot))
            .small()
            .ghost()
            .tooltip("Toggle Agent Panel")
            .disabled(!is_connected)
            .on_click(cx.listener(|_this, _: &ClickEvent, _, _cx| {
            }));

        let db_label = self.active_database.clone().unwrap_or_default();

        div()
            .id("footer-bar")
            .flex()
            .h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .text_xs()
            .bg(cx.theme().title_bar)
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .when(is_connected, |d| d.child(tables_button))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.status_message.clone()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(db_label),
                    )
                    .when(is_connected, |d| {
                        d.child(history_button).child(agent_button)
                    }),
            )
    }
}