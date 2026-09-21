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
use dbstudio_core::ai::{ChatMessage, Role, provider_for};
use lsp_types::{CompletionItem, CompletionItemKind};
use std::rc::Rc;

use crate::components::sql_completion::SqlCompletionProvider;
use crate::components::vim::{VimBuf, VimMode};
use crate::state::{AppState, select_database};
use crate::utils::toolbar_divider;

pub enum EditorEvent {
    ExecuteQuery(String),
}

impl EventEmitter<EditorEvent> for Editor {}

pub struct Editor {
    window_id: u64,
    pub input_state: Entity<EditorState>,
    provider: Rc<SqlCompletionProvider>,
    db_select: Entity<SelectState<Vec<SharedString>>>,
    is_executing: bool,
    active_connection: Option<String>,
    /// The session id whose editor buffer is currently loaded, used to detect
    /// tab switches so per-session buffers are swapped in/out.
    current_session_id: Option<u64>,
    /// Whether Vim-style key handling is active for the editor.
    vim_on: bool,
    vim: VimBuf,
    _subscriptions: Vec<Subscription>,
}

impl Editor {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        cx.new(|cx| Self::new(window_id, window, cx))
    }

    fn new(window_id: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                this.is_executing = state.is_executing_for(this.window_id);
                this.active_connection = state.active_connection_name_for(this.window_id).cloned();
                refresh_schema_completions(&this.provider, state.tables_for(this.window_id));
                cx.notify();
            }),
            cx.observe_global_in::<AppState>(window, move |this, win, cx| {
                this.sync_editor_buffer(win, cx);
                let state = cx.global::<AppState>();
                let names: Vec<SharedString> = state
                    .databases_for(this.window_id)
                    .iter()
                    .map(|d| d.name.clone().into())
                    .collect();
                let active_db = state.active_database_for(this.window_id).cloned();
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
        let vim_on = state.vim_mode;
        Self {
            window_id,
            input_state,
            provider,
            db_select,
            is_executing: state.is_executing_for(window_id),
            active_connection: state.active_connection_name_for(window_id).cloned(),
            current_session_id: state.window_state(window_id).active_session,
            vim_on,
            vim: VimBuf::default(),
            _subscriptions,
        }
    }

    /// Restore/save the per-session editor buffer on tab switches.
    ///
    /// The editor is a single entity shared across all sessions; when the
    /// active session changes we save the outgoing buffer and load the
    /// incoming one.
    fn sync_editor_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_id = cx.global::<AppState>().window_state(self.window_id).active_session;
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
            select_database(db.as_ref(), self.window_id, cx);
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

    /// Line indices [start_line, end_line] covered by the current selection
    /// (or the cursor line when the selection is empty).
    fn selected_line_span(&self, cx: &Context<Self>) -> (usize, usize) {
        let state = self.input_state.read(cx);
        let selected_range = state.selected_range();
        let full_text = state.value().to_string();
        let start = selected_range.start.min(full_text.len());
        let end = selected_range.end.min(full_text.len());
        let count_nl = |up_to: usize| full_text[..up_to].bytes().filter(|&b| b == b'\n').count();
        (count_nl(start), count_nl(end))
    }

    /// Replace the editor buffer with `text` (resets selection to the end).
    fn replace_editor_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let entity = self.input_state.clone();
        cx.update_entity(&entity, |i, cx| {
            i.set_value(SharedString::from(text), window, cx);
            cx.notify();
        });
    }

    /// Duplicate the selected lines (or the cursor line) below itself.
    pub fn duplicate_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let full_text = self.input_state.read(cx).value().to_string();
        let (start_line, end_line) = self.selected_line_span(cx);
        let mut lines: Vec<String> = full_text.split('\n').map(str::to_string).collect();
        if lines.is_empty() {
            return;
        }
        let block: Vec<String> = lines[start_line..=end_line].to_vec();
        lines.splice(end_line + 1..end_line + 1, block);
        self.replace_editor_text(lines.join("\n"), window, cx);
    }

    /// Move the selected lines (or the cursor line) up by one line.
    pub fn move_line_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let full_text = self.input_state.read(cx).value().to_string();
        let (start_line, end_line) = self.selected_line_span(cx);
        if start_line == 0 {
            return;
        }
        let lines: Vec<String> = full_text.split('\n').map(str::to_string).collect();
        let block: Vec<String> = lines[start_line..=end_line].to_vec();
        let prev = lines[start_line - 1].clone();
        let mut rebuilt = Vec::with_capacity(lines.len());
        rebuilt.extend(lines[..start_line - 1].iter().cloned());
        rebuilt.extend(block);
        rebuilt.push(prev);
        rebuilt.extend(lines[end_line + 1..].iter().cloned());
        self.replace_editor_text(rebuilt.join("\n"), window, cx);
    }

    /// Move the selected lines (or the cursor line) down by one line.
    pub fn move_line_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let full_text = self.input_state.read(cx).value().to_string();
        let (start_line, end_line) = self.selected_line_span(cx);
        let lines: Vec<String> = full_text.split('\n').map(str::to_string).collect();
        if end_line + 1 >= lines.len() {
            return;
        }
        let block: Vec<String> = lines[start_line..=end_line].to_vec();
        let next = lines[end_line + 1].clone();
        let mut rebuilt = Vec::with_capacity(lines.len());
        rebuilt.extend(lines[..start_line].iter().cloned());
        rebuilt.push(next);
        rebuilt.extend(block);
        rebuilt.extend(lines[end_line + 2..].iter().cloned());
        self.replace_editor_text(rebuilt.join("\n"), window, cx);
    }

    fn on_disconnect(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        crate::state::disconnect(self.window_id, cx);
    }

    /// Ask the configured LLM to continue the SQL at the cursor and insert the
    /// suggested text inline (the plan's 4g "鍏夋爣澶?Tab 瑙﹀彂" inline completion).
    pub fn ai_complete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = cx.global::<AppState>().ai_config.clone();
        let provider = provider_for(&config);
        if !provider.is_configured() {
            return;
        }

        let state = self.input_state.read(cx);
        let full_text = state.value().to_string();
        let cursor = state.selected_range().start.min(full_text.len());
        let schemas = cx.global::<AppState>().table_schemas_for(self.window_id);
        let mut schema_ctx = String::new();
        for (_key, s) in schemas.iter().take(10) {
            let columns: Vec<String> = s
                .columns
                .iter()
                .map(|c| format!("{} {}", c.name, c.data_type))
                .collect();
            schema_ctx.push_str(&format!("TABLE {} ({})\n", s.table_name, columns.join(", ")));
        }
        let prefix = &full_text[..cursor];
        let user_prompt = format!(
            "Schema:\n{}\n\nSQL buffer up to the cursor:\n```sql\n{}\n```\n\n\
             Continue this SQL statement. Output ONLY the characters to insert at the cursor \
             position to complete the statement naturally. No explanations, no markdown fences.",
            if schema_ctx.trim().is_empty() { "No schema loaded." } else { schema_ctx.trim_end() },
            prefix
        );

        let input_weak = self.input_state.downgrade();
        cx.spawn_in(window, async move |_this, cx| {
            let outcome = provider
                .chat(&[
                    ChatMessage {
                        role: Role::System,
                        content: "You are an inline SQL autocompletion engine. Given the schema \
                                  and the SQL written so far, reply with ONLY the text to append \
                                  at the cursor. Never invent table/column names not in the schema."
                            .to_string(),
                    },
                    ChatMessage {
                        role: Role::User,
                        content: user_prompt,
                    },
                ])
                .await;
            let completion = match outcome {
                Ok(resp) => normalize_ai_completion(&resp.content),
                Err(_) => return,
            };
            if completion.is_empty() {
                return;
            }
            let _ = cx.update(|window, app| {
                if let Some(handle) = input_weak.upgrade() {
                    handle.update(app, |state, ecx| {
                        state.insert(SharedString::from(completion), window, ecx);
                        ecx.notify();
                    });
                }
            });
        })
        .detach();
    }

    /// Toggle Vim key handling for the editor and persist the preference.
    pub fn toggle_vim(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.vim_on = !self.vim_on;
        if !self.vim_on {
            self.vim.reset();
            self.vim.mode = VimMode::Normal;
        }
        crate::state::save_vim_mode(self.vim_on, cx);
        cx.notify();
    }

    /// Route a raw key event through the Vim state machine. Returns `true` when
    /// the key was consumed (caller must stop propagation so the native editor
    /// never sees it).
    fn vim_handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if event.is_held {
            return true;
        }
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.to_lowercase();
        let ctrl = modifiers.control;
        let shift = modifiers.shift;
        let alt = modifiers.alt;

        {
            let state = self.input_state.read(cx);
            let text = state.value().to_string();
            let caret = state.selected_range().start;
            self.vim.sync(&text, caret);
        }

        let Some(step) = self.vim.step(&key, ctrl, shift, alt) else {
            // Pass through to the native editor (insert-mode typing, etc.).
            return false;
        };

        let entity = self.input_state.clone();
        cx.update_entity(&entity, |i, cx| {
            if let Some(text) = &step.text {
                i.set_value(SharedString::from(text.clone()), window, cx);
            }
            match step.selection {
                Some((lo, hi)) if lo < hi => i.set_selected_range(lo..hi, cx),
                _ => i.set_selected_range(step.caret.min(i.value().len())..step.caret.min(i.value().len()), cx),
            }
            if step.to_insert {
                i.focus(window, cx);
                cx.notify();
            }
        });
        cx.notify();
        true
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
                    crate::state::save_favorite(&name, &sql, this.window_id, cx);
                }
            }));

        let ai_configured = provider_for(&cx.global::<AppState>().ai_config).is_configured();
        let ai_complete_button = Button::new("editor-ai-complete")
            .icon(Icon::empty().path("icons/sparkles.svg"))
            .small()
            .ghost()
            .tooltip("AI 琛ュ叏锛氱敤宸查厤缃ā鍨嬬画鍐欏厜鏍囧 SQL")
            .disabled(!has_connection || !ai_configured)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.ai_complete(window, cx);
            }));

        let vim_button = Button::new("editor-vim")
            .label("Vim")
            .small()
            .ghost()
            .toggled(self.vim_on)
            .tooltip(if self.vim_on {
                "Vim 妯″紡锛歄N 鈥?Esc 鍥炲埌 Normal锛宨/a/o 杩涘叆鎻掑叆"
            } else {
                "Vim 妯″紡锛歄FF"
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_vim(window, cx);
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
                    .child(vim_button)
                    .child(favorite_button)
                    .child(ai_complete_button)
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
            .when(self.vim_on, |this| {
                this.capture_key_down(cx.listener(
                    |this, event: &KeyDownEvent, window, cx| {
                        if this.vim_handle_key(event, window, cx) {
                            cx.stop_propagation();
                        }
                    },
                ))
            })
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

/// Trim surrounding whitespace and strip ```sql ... ``` fences from an LLM
/// completion payload before splicing it into the buffer.
fn normalize_ai_completion(content: &str) -> String {
    let trimmed = content.trim();
    let stripped = if trimmed.starts_with("```") {
        let body = trimmed
            .trim_start_matches('`')
            .strip_prefix("sql")
            .unwrap_or("");
        body.trim_end_matches('`').trim()
    } else {
        trimmed
    };
    // Also drop a leading "sql" token that gpt-style completions sometimes emit.
    if let Some(body) = stripped.strip_prefix("sql ") {
        body.trim().to_string()
    } else {
        stripped.to_string()
    }
}

fn refresh_schema_completions(provider: &SqlCompletionProvider, tables: &[dbstudio_core::schema::TableInfo]) {
    let items: Vec<CompletionItem> = tables
        .iter()
        .map(|table| {
            let detail = table
                .schema
                .as_deref()
                .map(|schema| format!("{} 路 {}", schema, table.table_type.display_name()))
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
