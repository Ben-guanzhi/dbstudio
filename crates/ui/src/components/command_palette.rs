use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Icon,
    IconName,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};

use crate::state::AppState;

pub enum CommandPaletteEvent {
    Execute(String),
    SelectTable(String),
    SelectDatabase(String),
    RunCommand(String),
    Close,
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

#[derive(Clone)]
struct PaletteItem {
    label: String,
    detail: Option<String>,
    icon: IconName,
    action: PaletteAction,
}

#[derive(Clone)]
enum PaletteAction {
    ExecuteSql(String),
    SelectTable(String),
    SelectDatabase(String),
    RunCommand(String),
}

pub struct CommandPalette {
    input_state: Entity<InputState>,
    items: Vec<PaletteItem>,
    filtered_items: Vec<usize>,
    selected_index: usize,
    is_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl CommandPalette {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type a command or search...")
                .clean_on_escape()
        });

        let _subscriptions = vec![
            cx.subscribe_in(&input_state, window, Self::on_input_event),
            cx.observe_global::<AppState>(move |this, cx| {
                this.build_items(cx);
                this.filter_items(cx);
                cx.notify();
            }),
        ];

        let mut palette = Self {
            input_state,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected_index: 0,
            is_open: false,
            _subscriptions,
        };
        palette.build_items(cx);
        palette.filter_items(cx);
        palette
    }

    fn build_items(&mut self, cx: &mut Context<Self>) {
        let state = cx.global::<AppState>();
        self.items.clear();

        // Add tables
        for table in state.tables() {
            self.items.push(PaletteItem {
                label: table.name.clone(),
                detail: Some(format!("Table · {}", table.table_type.display_name())),
                icon: IconName::LayoutDashboard,
                action: PaletteAction::SelectTable(table.name.clone()),
            });
        }

        // Add databases
        for db in state.databases() {
            self.items.push(PaletteItem {
                label: db.name.clone(),
                detail: Some("Database".to_string()),
                icon: IconName::HardDrive,
                action: PaletteAction::SelectDatabase(db.name.clone()),
            });
        }

        // Add connections
        for conn in &state.saved_connections {
            self.items.push(PaletteItem {
                label: conn.name.clone(),
                detail: Some(format!("{}@{}", conn.username, conn.host)),
                icon: IconName::Globe,
                action: PaletteAction::RunCommand(format!("connect:{}", conn.id)),
            });
        }

        // A command that carries the SQL text (empty string means "run the
        // current query in the editor"); the workspace resolves it.
        self.items.push(PaletteItem {
            label: "Execute Query".to_string(),
            detail: Some("Execute the current SQL query".to_string()),
            icon: IconName::SquareTerminal,
            action: PaletteAction::ExecuteSql(String::new()),
        });

        // Add common commands
        let commands = vec![
            ("Format SQL", "editor-format", "Format the current SQL query"),
            ("Toggle Theme", "toggle-theme", "Switch between light and dark theme"),
            ("Toggle AI Panel", "toggle-ai", "Show or hide the AI assistant panel"),
            ("Toggle Safe Mode", "toggle-safe-mode", "Require confirmation for all write statements"),
            ("Clear History", "clear-history", "Clear query history"),
            ("Export CSV", "export-csv", "Export results as CSV"),
            ("Export JSON", "export-json", "Export results as JSON"),
        ];

        for (label, cmd, detail) in commands {
            self.items.push(PaletteItem {
                label: label.to_string(),
                detail: Some(detail.to_string()),
                icon: IconName::SquareTerminal,
                action: PaletteAction::RunCommand(cmd.to_string()),
            });
        }
    }

    fn filter_items(&mut self, cx: &mut Context<Self>) {
        let query = if self.is_open {
            self.input_state.read_with(cx, |state: &InputState, _| state.value().to_string())
        } else {
            String::new()
        };

        let query_lower = query.to_lowercase();

        self.filtered_items = if query_lower.is_empty() {
            (0..self.items.len()).collect()
        } else {
            self.items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.label.to_lowercase().contains(&query_lower)
                        || item
                            .detail
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&query_lower))
                            .unwrap_or(false)
                })
                .map(|(ix, _)| ix)
                .collect()
        };

        self.selected_index = 0;
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.is_open = true;
        self.filter_items(cx);
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.is_open = false;
        cx.notify();
    }

    fn selected_item(&self) -> Option<&PaletteItem> {
        self.filtered_items
            .get(self.selected_index)
            .and_then(|&ix| self.items.get(ix))
    }

    fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.filtered_items.is_empty() {
            return;
        }
        let new_index = (self.selected_index as i32 + delta)
            .max(0)
            .min(self.filtered_items.len() as i32 - 1) as usize;
        self.selected_index = new_index;
        cx.notify();
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.confirm_selection(cx);
        }
    }

    fn confirm_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_item().cloned() {
            match &item.action {
                PaletteAction::ExecuteSql(sql) => {
                    cx.emit(CommandPaletteEvent::Execute(sql.clone()));
                }
                PaletteAction::SelectTable(table) => {
                    cx.emit(CommandPaletteEvent::SelectTable(table.clone()));
                }
                PaletteAction::SelectDatabase(db) => {
                    cx.emit(CommandPaletteEvent::SelectDatabase(db.clone()));
                }
                PaletteAction::RunCommand(cmd) => {
                    // Handle built-in commands
                    match cmd.as_str() {
                        "clear-history" => {
                            crate::state::clear_history(cx);
                        }
                        "toggle-safe-mode" => {
                            crate::state::toggle_safe_mode(cx);
                        }
                        _ if cmd.starts_with("connect:") => {
                            let conn_id = &cmd[8..];
                            let conn = cx
                                .global::<AppState>()
                                .saved_connections
                                .iter()
                                .find(|c| c.id == conn_id)
                                .cloned();
                            if let Some(conn) = conn {
                                crate::state::connect(&conn, cx);
                            }
                        }
                        _ => {
                            cx.emit(CommandPaletteEvent::RunCommand(cmd.clone()));
                        }
                    }
                }
            }
            self.close(cx);
        }
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_open {
            return div().into_any_element();
        }

        let items = self.filtered_items.clone();
        let selected = self.selected_index;

        div()
            .id("command-palette-overlay")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close(cx);
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(gpui::black().opacity(0.5)),
            )
            .child(
                v_flex()
                    .id("command-palette")
                    .absolute()
                    .top(px(100.0))
                    .left(px(200.0))
                    .w(px(400.0))
                    .max_h(px(500.0))
                    .rounded(px(8.0))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_e, _window, cx| {
                        cx.stop_propagation();
                    })
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                        match event.keystroke.key.as_ref() {
                            "up" => this.move_selection(-1, cx),
                            "down" => this.move_selection(1, cx),
                            "escape" => this.close(cx),
                            _ => {}
                        }
                    }))
                    .child(
                        h_flex()
                            .id("palette-input")
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                Icon::new(IconName::Search)
                                    .size_4()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(Input::new(&self.input_state).flex_1()),
                    )
                    .child(
                        v_flex()
                            .id("palette-items")
                            .flex_1()
                            .overflow_y_scrollbar()
                            .py_1()
                            .when(items.is_empty(), |this| {
                                this.child(
                                    div()
                                        .p_4()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .text_center()
                                        .child("No matching items"),
                                )
                            })
                            .children(items.into_iter().enumerate().map(|(ix, item_ix)| {
                                let item = &self.items[item_ix];
                                let is_selected = ix == selected;
                                let bg = if is_selected {
                                    cx.theme().list_active
                                } else {
                                    gpui::transparent_black()
                                };

                                h_flex()
                                    .id(("palette-item", item_ix))
                                    .w_full()
                                    .items_center()
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .bg(bg)
                                    .cursor_pointer()
                                    .hover(|this| this.bg(cx.theme().list_hover))
                                    .child(
                                        Icon::new(item.icon.clone())
                                            .size_4()
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .child(item.label.clone()),
                                            )
                                            .when_some(item.detail.clone(), |this, detail| {
                                                this.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(detail),
                                                )
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                        this.selected_index = ix;
                                        this.confirm_selection(cx);
                                    }))
                            })),
                    ),
            )
            .into_any_element()
    }
}
