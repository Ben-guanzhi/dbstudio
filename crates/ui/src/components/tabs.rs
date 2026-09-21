use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};

use crate::state::{AppState, ConnectionStatus};

#[derive(Debug, Clone)]
pub enum TabsEvent {
    /// Activate the session tab with this id in the current window.
    Select(u64),
    /// Close (disconnect + drop) the session tab with this id.
    Close(u64),
    /// Open the new-connection form for a fresh tab.
    NewTab,
}

struct TabItem {
    id: u64,
    name: String,
    database: Option<String>,
    status: ConnectionStatus,
}

/// Horizontal session tab strip (TablePro editor-tab-strip proportions:
/// 36px band, 28px capsule track, 24px capsule tabs).
pub struct TabsBar {
    window_id: u64,
    sessions: Vec<TabItem>,
    active: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TabsEvent> for TabsBar {}

impl TabsBar {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| Self::new(window_id, cx))
    }

    fn new(window_id: u64, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();
            this.sessions = state
                .sessions
                .iter()
                .map(|s| TabItem {
                    id: s.id,
                    name: s.name.clone(),
                    database: s.active_database.clone(),
                    status: s.connection_state,
                })
                .collect();
            this.active = state.window_state(this.window_id).active_session;
            cx.notify();
        })];

        let state = cx.global::<AppState>();
        Self {
            window_id,
            sessions: state
                .sessions
                .iter()
                .map(|s| TabItem {
                    id: s.id,
                    name: s.name.clone(),
                    database: s.active_database.clone(),
                    status: s.connection_state,
                })
                .collect(),
            active: state.window_state(window_id).active_session,
            _subscriptions,
        }
    }

    fn status_color(
        status: ConnectionStatus,
        success: Hsla,
        warning: Hsla,
        disconnected_fg: Hsla,
    ) -> Hsla {
        match status {
            ConnectionStatus::Connected => success,
            ConnectionStatus::Connecting | ConnectionStatus::Disconnecting => warning,
            ConnectionStatus::Disconnected => disconnected_fg,
        }
    }
}

impl Render for TabsBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.sessions.is_empty() {
            return div().id("tabs-bar-empty").into_any_element();
        }

        // Pass 1: snapshot per-tab visual data (immutable borrows only).
        struct TabView {
            id: u64,
            label: String,
            is_active: bool,
            dot: Hsla,
        }
        let muted_fg = cx.theme().muted_foreground;
        let success = cx.theme().button_success;
        let warning = cx.theme().button_warning;
        let views: Vec<TabView> = self
            .sessions
            .iter()
            .map(|tab| {
                let is_active = self.active == Some(tab.id);
                let dot = TabsBar::status_color(tab.status, success, warning, muted_fg);
                let label = match &tab.database {
                    Some(db) if !db.is_empty() => format!("{} · {}", tab.name, db),
                    _ => tab.name.clone(),
                };
                TabView {
                    id: tab.id,
                    label,
                    is_active,
                    dot,
                }
            })
            .collect();

        // Pass 2: build elements (mutable borrows via listeners, sequential).
        let mut track = div();
        for v in views {
            let id = v.id;
            let bg = if v.is_active {
                cx.theme().background
            } else {
                cx.theme().title_bar
            };
            let fg = if v.is_active {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            };
            let muted = cx.theme().muted_foreground;
            let hover = cx.theme().list_hover;
            let tab = div()
                .id(("session-tab", id))
                .flex()
                .items_center()
                .gap_1()
                .h(px(24.0))
                .px_3()
                .min_w(px(120.0))
                .max_w(px(220.0))
                .flex_shrink_0()
                .rounded_full()
                .bg(bg)
                .when(!v.is_active, move |d| d.hover(move |s| s.bg(hover)))
                .when(v.is_active, |d| {
                    d.border_1().border_color(cx.theme().border)
                })
                .child(div().size(px(7.0)).flex_shrink_0().rounded_full().bg(v.dot))
                .child(
                    div()
                        .text_xs()
                        .overflow_hidden()
                        .text_ellipsis()
                        .when(v.is_active, |d| d.font_medium())
                        .text_color(fg)
                        .child(v.label),
                )
                .child(
                    div()
                        .id(("tab-close", id))
                        .ml_auto()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|s| s.text_color(cx.theme().foreground))
                        .child(Icon::new(IconName::Close).size_2().text_color(muted))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            cx.emit(TabsEvent::Close(id));
                        })),
                )
                .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                    cx.emit(TabsEvent::Select(id));
                }));
            track = track.child(tab);
        }

        h_flex()
            .id("tabs-bar")
            .w_full()
            .h(px(36.0))
            .px_2()
            .items_center()
            .gap_1()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .id("tabs-track")
                    .flex_1()
                    .min_w_0()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .overflow_x_scroll()
                    .bg(cx.theme().muted)
                    .rounded_full()
                    .child(track),
            )
            .child(
                Button::new("tabs-new")
                    .icon(Icon::new(IconName::Plus).size_3())
                    .ghost()
                    .small()
                    .tooltip("New connection tab")
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(TabsEvent::NewTab);
                    })),
            )
            .into_any_element()
    }
}
