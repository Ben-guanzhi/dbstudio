use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    Disableable as _,
    Icon,
    IconName,
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Enter, Editor as CodeEditor, EditorState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use lsp_types::{CompletionItem, CompletionItemKind};
use std::rc::Rc;

use crate::components::sql_completion::SqlCompletionProvider;
use crate::state::{AppState, select_database};
use crate::utils::toolbar_divider;

pub enum EditorEvent {
    ExecuteQuery(String),
}

impl EventEmitter<EditorEvent> for Editor {}

pub struct Editor {
    pub input_state: Entity<EditorState>,
    provider: Rc<SqlCompletionProvider>,
    db_select: Entity<SelectState<Vec<SharedString>>>,
    is_executing: bool,
    active_connection: Option<String>,
    /// The session id whose editor buffer is currently loaded, used to detect
    /// tab switches so per-session buffers are swapped in/out.
    current_session_id: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

impl Editor {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = Rc::new(SqlCompletionProvider::new());

        let provider_for_input = provider.clone();
        let input_state = {
            let window = &mut *window;
            cx.new(move |cx| {
                let mut state = EditorState::new(window, cx)
                    .language("sql")
                    .line_number(true)
                    .placeholder("Enter your SQL query here...")
                    .default_value("SELECT 1;");
                state.lsp_mut().completion_provider = Some(provider_for_input);
                state
            })
        };

        let db_select = cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));

        let _subscriptions = vec![
            cx.observe_global::<AppState>(move |this, cx| {
                let state = cx.global::<AppState>();
                this.is_executing = state.is_executing();
                this.active_connection = state.active_connection_name().cloned();
                refresh_schema_completions(&this.provider, state.tables());
                cx.notify();
            }),
            cx.observe_global_in::<AppState>(window, move |this, win, cx| {
                this.sync_editor_buffer(win, cx);
                let state = cx.global::<AppState>();
                let names: Vec<SharedString> = state
                    .databases()
                    .iter()
                    .map(|d| d.name.clone().into())
                    .collect();
                let active_db = state.active_database().cloned();
                this.db_select.update(cx, |select, cx| {
                    select.set_items(names, win, cx);
                    if let Some(db) = &active_db {
                        let value = SharedString::from(db.clone());
                        select.set_selected_value(&value, win, cx);
                    }
                    cx.notify();
                });
            }),
        ];

        cx.subscribe_in(&db_select, window, Self::on_select_database_event)
            .detach();

        let state = cx.global::<AppState>();
        Self {
            input_state,
            provider,
            db_select,
            is_executing: state.is_executing(),
            active_connection: state.active_connection_name().cloned(),
            current_session_id: state.active_session,
            _subscriptions,
        }
    }

    /// Restore/save the per-session editor buffer on tab switches.
    ///
    /// The editor is a single entity shared across all sessions; when the
    /// active session changes we save the outgoing buffer and load the
    /// incoming one.
    fn sync_editor_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_id = cx.global::<AppState>().active_session;
        if new_id == self.current_session_id {
            return;
        }
        self.save_editor_text(cx);
        self.current_session_id = new_id;
        let text = new_id
            .and_then(|id| cx.global::<AppState>().session(id))
            .map(|s| s.editor_text.clone())
            .unwrap_or_default();
        let entity = self.input_state.clone();
        cx.update_entity(&entity, |i, cx| {
            i.set_value(SharedString::from(text), window, cx);
            cx.notify();
        });
    }

    fn save_editor_text(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current_session_id else {
            return;
        };
        let text = self.input_state.read(cx).value().to_string();
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.session_mut(id) {
                s.editor_text = text;
            }
        });
    }

    fn on_select_database_event(
        &mut self,
        _: &Entity<SelectState<Vec<SharedString>>>,
        event: &SelectEvent<Vec<SharedString>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(db)) = event {
            select_database(db.as_ref(), cx);
        }
    }

    fn on_execute(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_query(cx);
    }

    fn run_query(&mut self, cx: &mut Context<Self>) {
        if self.is_executing {
            return;
        }
        let sql = self.get_query_to_execute(cx);
        if !sql.trim().is_empty() {
            cx.emit(EditorEvent::ExecuteQuery(sql));
        }
    }

    /// Get the query to execute: selected text if there's a selection, otherwise all text.
    fn get_query_to_execute(&self, cx: &mut Context<Self>) -> String {
        let state = self.input_state.read(cx);
        let selected_range = state.selected_range();
        let full_text = state.value().to_string();

        if selected_range.is_empty() {
            full_text
        } else {
            let start = selected_range.start.min(full_text.len());
            let end = selected_range.end.min(full_text.len());
            full_text[start..end].to_string()
        }
    }

    /// Format the SQL in the editor using sqlformat.
    pub fn format_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.input_state.read(cx).value().to_string();
        if sql.trim().is_empty() {
            return;
        }
        let formatted = sqlformat::format(
            &sql,
            &sqlformat::QueryParams::None,
            &sqlformat::FormatOptions::default(),
        );
        let entity = self.input_state.clone();
        cx.update_entity(&entity, |i, cx| {
            i.set_value(SharedString::from(formatted), window, cx);
            cx.notify();
        });
    }

    /// Toggle comment on selected lines (add/remove -- prefix).
    pub fn toggle_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.input_state.read(cx);
        let selected_range = state.selected_range();
        let full_text = state.value().to_string();

        if selected_range.is_empty() {
            return;
        }

        let start = selected_range.start.min(full_text.len());
        let end = selected_range.end.min(full_text.len());

        // Find line boundaries
        let line_start = full_text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = full_text[end..].find('\n').map(|i| end + i).unwrap_or(full_text.len());

        let selected_lines = &full_text[line_start..line_end];
        let all_commented = selected_lines.lines().all(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("--") || trimmed.is_empty()
        });

        let new_lines: String = selected_lines
            .lines()
            .map(|line| {
                if all_commented {
                    // Remove comment prefix
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("-- ") {
                        format!("{}{}", &line[..line.len() - trimmed.len()], &trimmed[3..])
                    } else if trimmed.starts_with("--") {
                        format!("{}{}", &line[..line.len() - trimmed.len()], &trimmed[2..])
                    } else {
                        line.to_string()
                    }
                } else {
                    // Add comment prefix
                    format!("-- {}", line)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let new_text = format!("{}{}{}", &full_text[..line_start], new_lines, &full_text[line_end..]);
        let entity = self.input_state.clone();
        cx.update_entity(&entity, |i, cx| {
            i.set_value(SharedString::from(new_text), window, cx);
            cx.notify();
        });
    }

    fn on_disconnect(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        crate::state::disconnect(cx);
    }

    pub fn set_query(&mut self, query: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
        let query = query.into();
        cx.update_entity(&self.input_state, |i, cx| {
            i.set_value(query, window, cx);
            cx.notify();
        });
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_connection = self.active_connection.is_some();

        let format_button = Button::new("editor-format")
            .icon(Icon::empty().path("icons/align-start-vertical.svg"))
            .small()
            .ghost()
            .tooltip("Format SQL (Shift-Alt-F)")
            .disabled(!has_connection)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.format_sql(window, cx);
            }));

        let comment_button = Button::new("editor-comment")
            .icon(Icon::empty().path("icons/message-square.svg"))
            .small()
            .ghost()
            .tooltip("Toggle Comment (Ctrl-/)")
            .disabled(!has_connection)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_comment(window, cx);
            }));

        let favorite_button = Button::new("editor-favorite")
            .icon(Icon::empty().path("icons/star.svg"))
            .small()
            .ghost()
            .tooltip("Save as Favorite")
            .disabled(!has_connection)
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                let sql = this.input_state.read(cx).value().to_string();
                if !sql.trim().is_empty() {
                    // For now, use a default name; a dialog could be added later
                    let name = format!("Query {}", chrono::Local::now().format("%H:%M:%S"));
                    crate::state::save_favorite(&name, &sql, cx);
                }
            }));

        let execute_button = Button::new("editor-execute")
            .icon(Icon::empty().path("icons/play.svg"))
            .small()
            .primary()
            .ghost()
            .tooltip(if self.is_executing { "Executing..." } else { "Execute (Ctrl-Enter)" })
            .loading(self.is_executing)
            .disabled(!has_connection)
            .on_click(cx.listener(Self::on_execute));

        let disconnect_button = Button::new("editor-disconnect")
            .icon(Icon::empty().path("icons/power.svg"))
            .small()
            .danger()
            .ghost()
            .tooltip("Disconnect")
            .disabled(!has_connection)
            .on_click(cx.listener(Self::on_disconnect));

        let toolbar = h_flex()
            .id("editor-toolbar")
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .when(has_connection, |el| {
                el.child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(IconName::LayoutDashboard)
                                .size_4()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            Select::new(&self.db_select)
                                .appearance(false)
                                .menu_width(px(200.0)),
                        ),
                )
            })
            .when(!has_connection, |el| el.child(div()))
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(format_button)
                    .child(comment_button)
                    .child(favorite_button)
                    .child(execute_button)
                    .child(toolbar_divider(cx))
                    .child(disconnect_button),
            );

        v_flex()
            .id("sql-editor")
            .capture_action(cx.listener(
                |this, action: &Enter, _window: &mut Window, cx: &mut Context<Self>| {
                    if action.secondary {
                        cx.stop_propagation();
                        this.run_query(cx);
                    }
                },
            ))
            .size_full()
            .child(toolbar)
            .child(
                div()
                    .id("editor-content")
                    .bg(cx.theme().background)
                    .w_full()
                    .flex_1()
                    .px_2()
                    .pb_2()
                    .child(
                        CodeEditor::new(&self.input_state)
                            .w_full()
                            .h_full()
                            .rounded(cx.theme().radius),
                    ),
            )
    }
}

fn refresh_schema_completions(provider: &SqlCompletionProvider, tables: &[dbstudio_core::schema::TableInfo]) {
    let items: Vec<CompletionItem> = tables
        .iter()
        .map(|table| {
            let detail = table
                .schema
                .as_deref()
                .map(|schema| format!("{} · {}", schema, table.table_type.display_name()))
                .unwrap_or_else(|| table.table_type.display_name().to_string());
            CompletionItem {
                label: table.name.clone(),
                kind: Some(CompletionItemKind::CLASS),
                detail: Some(detail),
                ..Default::default()
            }
        })
        .collect();
    provider.set_schema_completions(items);
}
