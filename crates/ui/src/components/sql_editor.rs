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
                this.is_executing = state.is_executing;
                this.active_connection = state.active_connection_name.clone();
                refresh_schema_completions(&this.provider, &state.tables);
                cx.notify();
            }),
            cx.observe_global_in::<AppState>(window, move |this, win, cx| {
                let state = cx.global::<AppState>();
                let names: Vec<SharedString> = state
                    .databases
                    .iter()
                    .map(|d| d.name.clone().into())
                    .collect();
                let active_db = state.active_database.clone();
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
            is_executing: state.is_executing,
            active_connection: state.active_connection_name.clone(),
            _subscriptions,
        }
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
        let sql = self.input_state.read(cx).value().to_string();
        if !sql.trim().is_empty() {
            cx.emit(EditorEvent::ExecuteQuery(sql.trim().into()));
        }
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
            .tooltip("Format")
            .disabled(!has_connection)
            .on_click(|_, _, _| {});

        let execute_button = Button::new("editor-execute")
            .icon(Icon::empty().path("icons/play.svg"))
            .small()
            .primary()
            .ghost()
            .tooltip(if self.is_executing { "Executing..." } else { "Execute" })
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
