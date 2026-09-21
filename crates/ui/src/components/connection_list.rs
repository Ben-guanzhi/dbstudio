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
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    Sizable as _,
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
    editing_id: Option<String>,
    group_input: Entity<InputState>,
    tags_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionList {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let group_input = {
            let window = &mut *window;
            cx.new(move |cx| {
                InputState::new(window, cx).placeholder("Group (optional)")
            })
        };
        let tags_input = {
            let window = &mut *window;
            cx.new(move |cx| {
                InputState::new(window, cx).placeholder("Tags, comma separated")
            })
        };
        cx.new(|cx| ConnectionList::new(cx, group_input, tags_input, &*window))
    }

    fn new(
        cx: &mut Context<Self>,
        group_input: Entity<InputState>,
        tags_input: Entity<InputState>,
        window: &Window,
    ) -> Self {
        let mut _subscriptions = vec![
            cx.observe_global::<AppState>(move |this, cx| {
                let state = cx.global::<AppState>();
                this.connections = state.saved_connections.clone();
                if let Some(selected) = &this.selected_id {
                    if !this.connections.iter().any(|c| &c.id == selected) {
                        this.selected_id = None;
                    }
                }
                if let Some(editing) = &this.editing_id {
                    if !this.connections.iter().any(|c| &c.id == editing) {
                        this.editing_id = None;
                    }
                }
                cx.notify();
            }),
        ];

        _subscriptions.extend([
            cx.subscribe_in(&group_input, window, Self::on_tag_input_event),
            cx.subscribe_in(&tags_input, window, Self::on_tag_input_event),
        ]);

        Self {
            connections: cx.global::<AppState>().saved_connections.clone(),
            selected_id: None,
            collapsed_groups: HashSet::new(),
            editing_id: None,
            group_input,
            tags_input,
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

    fn on_tag_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.save_tag_edit(window, cx);
        }
    }

    /// Start inline editing of a connection's group/tags (right-click).
    fn open_tag_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.connections.iter().find(|c| c.id == id) else {
            return;
        };
        let group = info.group.clone().unwrap_or_default();
        let tags = info.tags.join(", ");
        let group_input = self.group_input.clone();
        let tags_input = self.tags_input.clone();
        cx.update_entity(&group_input, |i, cx| {
            i.set_value(group, window, cx);
            cx.notify();
        });
        cx.update_entity(&tags_input, |i, cx| {
            i.set_value(tags, window, cx);
            cx.notify();
        });
        self.editing_id = Some(id.to_string());
        cx.notify();
    }

    fn save_tag_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing_id.clone() else {
            return;
        };
        let Some(mut updated) = self.connections.iter().find(|c| c.id == id).cloned() else {
            self.editing_id = None;
            return;
        };
        let group = self.group_input.read(cx).value().to_string();
        let tags = self.tags_input.read(cx).value().to_string();
        updated.group = if group.trim().is_empty() {
            None
        } else {
            Some(group.trim().to_string())
        };
        updated.tags = tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        updated.updated_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.editing_id = None;
        crate::state::save_connection(&updated, "", "", "", cx);
    }

    fn cancel_tag_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.editing_id = None;
        cx.notify();
    }

    fn render_item(&mut self, ix: usize, info: &ConnectionInfo, cx: &mut Context<Self>) -> impl IntoElement {
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
        let info_for_edit = info.clone();
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
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.open_tag_edit(&info_for_edit.id, window, cx);
                }),
            )
    }

    fn render_edit_item(&mut self, info: &ConnectionInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let env = info.environment;
        v_flex()
            .id(("conn-edit", 0usize))
            .w_full()
            .gap_2()
            .px_3()
            .py_2()
            .bg(cx.theme().list_even)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(info.name.clone()),
                    )
                    .child(self.env_badge(env, cx)),
            )
            .child(Input::new(&self.group_input).flex_1().rounded(cx.theme().radius))
            .child(Input::new(&self.tags_input).flex_1().rounded(cx.theme().radius))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("tag-save")
                            .label("Save")
                            .small()
                            .ghost()
                            .primary()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.save_tag_edit(window, cx);
                            })),
                    )
                    .child(
                        Button::new("tag-cancel")
                            .label("Cancel")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_tag_edit(window, cx);
                            })),
                    ),
            )
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

        // Group connections by group field (from an owned snapshot so the item
        // renderers below can borrow `self` mutably).
        let conns = self.connections.clone();
        let mut grouped: BTreeMap<String, Vec<&ConnectionInfo>> = BTreeMap::new();
        let mut ungrouped: Vec<&ConnectionInfo> = Vec::new();

        for info in &conns {
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
            let is_editing = self.editing_id.as_deref() == Some(info.id.as_str());
            items = items.child(if is_editing {
                self.render_edit_item(info, cx).into_any_element()
            } else {
                self.render_item(idx, info, cx).into_any_element()
            });
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
                    let is_editing = self.editing_id.as_deref() == Some(info.id.as_str());
                    items = items.child(if is_editing {
                        self.render_edit_item(info, cx).into_any_element()
                    } else {
                        self.render_item(idx, info, cx).into_any_element()
                    });
                    idx += 1;
                }
            }
        }

        items
    }
}