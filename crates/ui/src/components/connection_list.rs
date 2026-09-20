use std::collections::{BTreeMap, HashSet};

use dbstudio_core::models::Environment;
use dbstudio_storage::types::ConnectionInfo;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Icon,
    IconName,
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
    collapsed_groups: HashSet<String>,
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
            collapsed_groups: HashSet::new(),
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

    fn toggle_group(&mut self, group: &str, cx: &mut Context<Self>) {
        if self.collapsed_groups.contains(group) {
            self.collapsed_groups.remove(group);
        } else {
            self.collapsed_groups.insert(group.to_string());
        }
        cx.notify();
    }

    fn env_badge(&self, env: Environment, cx: &Context<Self>) -> impl IntoElement {
        let (label, color) = match env {
            Environment::Dev => ("DEV", cx.theme().success),
            Environment::Staging => ("STG", cx.theme().warning),
            Environment::Production => ("PROD", cx.theme().danger),
        };
        div()
            .text_xs()
            .font_bold()
            .px_1()
            .rounded(px(2.0))
            .bg(color.opacity(0.15))
            .text_color(color)
            .child(label)
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
        } else if ix % 2 == 0 {
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
        let env = info.environment;

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
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .whitespace_nowrap()
                                    .child(info.name.clone()),
                            )
                            .child(self.env_badge(env, cx)),
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

        // Group connections by group field
        let mut grouped: BTreeMap<String, Vec<&ConnectionInfo>> = BTreeMap::new();
        let mut ungrouped: Vec<&ConnectionInfo> = Vec::new();

        for info in &self.connections {
            match &info.group {
                Some(g) if !g.is_empty() => {
                    grouped.entry(g.clone()).or_default().push(info);
                }
                _ => ungrouped.push(info),
            }
        }

        // Render ungrouped connections first
        let mut idx = 0;
        for info in &ungrouped {
            items = items.child(self.render_item(idx, info, cx));
            idx += 1;
        }

        // Render grouped connections with collapsible headers
        for (group_name, group_conns) in &grouped {
            let is_collapsed = self.collapsed_groups.contains(group_name);
            let group = group_name.clone();
            items = items.child(
                h_flex()
                    .id(format!("group-{}", group_name))
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_group(&group, cx);
                    }))
                    .child(
                        Icon::new(if is_collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                            .size_3()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} ({})", group_name, group_conns.len())),
                    ),
            );
            if !is_collapsed {
                for info in group_conns {
                    items = items.child(self.render_item(idx, info, cx));
                    idx += 1;
                }
            }
        }

        items
    }
}
