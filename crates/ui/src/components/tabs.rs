use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::ContextMenuExt,
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};
use serde::Deserialize;

use crate::state::{AppState, ConnectionStatus};

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = tabs, no_json)]
pub struct TabClose {
    pub id: u64,
}

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = tabs, no_json)]
pub struct TabCloseOthers {
    pub id: u64,
}

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = tabs, no_json)]
pub struct TabDuplicateInNewWindow {
    pub id: u64,
}

#[derive(Debug, Clone)]
pub enum TabsEvent {
    /// Activate the session tab with this id in the current window.
    Select(u64),
    /// Close (disconnect + drop) the session tab with this id.
    Close(u64),
    /// Close all other tabs except this one.
    CloseOthers(u64),
    /// Duplicate the session in a new window.
    DuplicateInNewWindow(u64),
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

        let muted_fg = cx.theme().muted_foreground;
        let success = cx.theme().button_success;
        let warning = cx.theme().button_warning;

        let mut track = div();
        for tab in self.sessions.iter() {
            let id = tab.id;
            let is_active = self.active == Some(id);
            let dot = TabsBar::status_color(tab.status, success, warning, muted_fg);
            let label = match &tab.database {
                Some(db) if !db.is_empty() => format!("{} · {}", tab.name, db),
                _ => tab.name.clone(),
            };
            let bg = if is_active {
                cx.theme().background
            } else {
                cx.theme().title_bar
            };
            let fg = if is_active {
                cx.theme().foreground
            } else {
                cx.theme().muted_foreground
            };
            let muted = cx.theme().muted_foreground;
            let hover = cx.theme().list_hover;
            let tab_element = div()
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
                .when(!is_active, move |d| d.hover(move |s| s.bg(hover)))
                .when(is_active, |d| d.border_1().border_color(cx.theme().border))
                .child(div().size(px(7.0)).flex_shrink_0().rounded_full().bg(dot))
                .child(
                    div()
                        .text_xs()
                        .overflow_hidden()
                        .text_ellipsis()
                        .when(is_active, |d| d.font_medium())
                        .text_color(fg)
                        .child(label),
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
                }))
                .context_menu(move |menu, _window, _cx| {
                    menu.menu("Close", Box::new(TabClose { id }))
                        .menu("Close Others", Box::new(TabCloseOthers { id }))
                        .menu(
                            "Duplicate in New Window",
                            Box::new(TabDuplicateInNewWindow { id }),
                        )
                });
            track = track.child(tab_element);
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
