use dbstudio_storage::types::ConnectionInfo;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    StyledExt as _,
    h_flex,
    v_flex,
};

use crate::state::AppState;

pub enum ConnectionListEvent {
    Selected(ConnectionInfo),
}

impl EventEmitter<ConnectionListEvent> for ConnectionList {}

pub struct ConnectionList {
    connections: Vec<ConnectionInfo>,
    selected_id: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionList {
    pub fn view(_window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            this.connections = state.saved_connections.clone();
            if let Some(selected) = &this.selected_id {
                if !this.connections.iter().any(|c| &c.id == selected) {
                    this.selected_id = None;
                }
            }
            cx.notify();
        })];

        Self {
            connections: cx.global::<AppState>().saved_connections.clone(),
            selected_id: None,
            _subscriptions,
        }
    }

    pub fn set_selected(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected_id = id;
        cx.notify();
    }

    fn on_select(&mut self, info: &ConnectionInfo, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = Some(info.id.clone());
        cx.emit(ConnectionListEvent::Selected(info.clone()));
        cx.notify();
    }

    fn render_item(&self, ix: usize, info: &ConnectionInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let is_selected = self.selected_id.as_deref() == Some(info.id.as_str());
        let text_color = if is_selected {
            cx.theme().accent_foreground
        } else {
            cx.theme().foreground
        };
        let bg_color = if is_selected {
            cx.theme().list_active
        } else if ix.is_multiple_of(2) {
            cx.theme().colors.list
        } else {
            cx.theme().list_even
        };
        let detail = if info.host.is_empty() {
            info.database.clone()
        } else {
            format!(
                "{}@{}:{}/{}",
                info.username, info.host, info.port, info.database
            )
        };
        let info = info.clone();

        h_flex()
            .id(("conn", ix))
            .w_full()
            .overflow_x_hidden()
            .items_center()
            .gap_3()
            .text_color(text_color)
            .px_3()
            .py_2()
            .bg(bg_color)
            .border_1()
            .border_color(bg_color)
            .when(is_selected, |this| this.border_color(cx.theme().list_active_border))
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .child(
                v_flex()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .whitespace_nowrap()
                            .child(info.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_nowrap()
                            .text_color(text_color.opacity(0.6))
                            .child(detail),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.on_select(&info, window, cx)
            }))
    }
}

impl Render for ConnectionList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut items = v_flex().id("connection-list").flex_1().gap_1();

        if self.connections.is_empty() {
            items = items.child(
                v_flex()
                    .items_center()
                    .justify_center()
                    .py_8()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("No saved connections"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground.opacity(0.7))
                            .child("Click the + button to create one"),
                    ),
            );
            return items;
        }

        for (i, info) in self.connections.iter().enumerate() {
            let item = self.render_item(i, info, cx);
            items = items.child(item);
        }

        items
    }
}