use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use dbstudio_core::result::{ExecResult, ErrorResult, QueryResult, ResultCell, SqlResult};
use dbstudio_core::schema::{ColumnInfo, TableSchema};
use dbstudio_db::utils::quote_string_literal;
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
    input::{Input, InputState},
    label::Label,
    scroll::ScrollableElement as _,
    table::{Column, ColumnSort, DataTable, TableDelegate, TableState},
    v_flex,
};

use crate::state::AppState;

mod actions;
mod export;
mod table_delegate;
mod views;

use table_delegate::ResultsTableDelegate;

#[derive(Clone, Copy, PartialEq)]
enum ResultsTab {
    Data,
    Schema,
}

pub struct ResultsPanel {
    result: Option<Arc<SqlResult>>,
    table: Entity<TableState<ResultsTableDelegate>>,
    selected_schema: Option<TableSchema>,
    pending_table: Option<String>,
    current_table: Option<String>,
    active_tab: ResultsTab,
    selected_row_cell: Rc<RefCell<Option<usize>>>,
    show_insert_modal: bool,
    editing_row: Option<usize>,
    edit_original_row: Option<Vec<ResultCell>>,
    insert_inputs: Vec<Entity<InputState>>,
    insert_columns: Vec<ColumnInfo>,
    _subscriptions: Vec<Subscription>,
}

impl ResultsPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let selected_row_cell = Rc::new(RefCell::new(None));
        let table = cx.new(|cx| {
            TableState::new(
                ResultsTableDelegate::new(selected_row_cell.clone()),
                window,
                cx,
            )
            .sortable(true)
        });

        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();

            let state_result_ptr = state.last_result.as_ref().map(|r| Arc::as_ptr(r));
            let current_result_ptr = this.result.as_ref().map(|r| Arc::as_ptr(r));
            let result_changed = state_result_ptr != current_result_ptr;

            let pending_schema = if let Some(ref t) = this.pending_table {
                state.table_schemas.get(t).cloned()
            } else {
                None
            };

            if result_changed {
                this.result = state.last_result.clone();

                match this.result.as_deref() {
                    Some(SqlResult::Query(query)) => {
                        this.active_tab = ResultsTab::Data;
                        let query = query.clone();
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().update(&query);
                            table.refresh(cx);
                        });
                    }
                    _ => {
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().update(&QueryResult::empty(""));
                            table.refresh(cx);
                        });
                    }
                }
            }

            let got_schema = pending_schema.is_some();
            if let Some(schema) = pending_schema {
                this.selected_schema = Some(schema);
                this.pending_table = None;
            }

            if result_changed || got_schema {
                cx.notify();
            }
        })];

        Self {
            result: cx.global::<AppState>().last_result.clone(),
            table,
            selected_schema: None,
            pending_table: None,
            current_table: None,
            active_tab: ResultsTab::Data,
            selected_row_cell,
            show_insert_modal: false,
            editing_row: None,
            edit_original_row: None,
            insert_inputs: Vec::new(),
            insert_columns: Vec::new(),
            _subscriptions,
        }
    }

    pub fn select_table(&mut self, table_name: String, schema: Option<TableSchema>, cx: &mut Context<Self>) {
        self.current_table = Some(table_name.clone());
        self.pending_table = Some(table_name);
        self.selected_schema = schema;
        self.result = None;
        self.active_tab = ResultsTab::Data;
        *self.selected_row_cell.borrow_mut() = None;
        cx.update_global::<AppState, _>(|state, _cx| {
            state.last_result = None;
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().update(&QueryResult::empty(""));
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }

    fn set_active_tab(&mut self, tab: ResultsTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

}

fn empty_hint(message: &'static str, cx: &Context<ResultsPanel>) -> AnyElement {
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message)
        .into_any_element()
}

/// Small muted badge/label used across the schema view.
fn muted_label(text: impl Into<SharedString>, cx: &Context<ResultsPanel>) -> Label {
    Label::new(text).text_xs().text_color(cx.theme().muted_foreground)
}

/// Section heading inside the schema view.
fn section_header(title: &'static str, cx: &Context<ResultsPanel>) -> Label {
    Label::new(title)
        .text_sm()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
}

fn render_error(cx: &Context<ResultsPanel>, err: &ErrorResult) -> impl IntoElement {
    v_flex()
        .gap_1()
        .p_4()
        .child(div().text_base().font_bold().text_color(gpui::red()).child("Query Error"))
        .child(div().text_sm().text_color(cx.theme().foreground).child(err.message.clone()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(err.sql.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} ms", err.execution_time_ms)),
        )
}

fn render_modified(cx: &Context<ResultsPanel>, exec: &ExecResult) -> impl IntoElement {
    v_flex()
        .gap_1()
        .p_4()
        .child(div().text_sm().font_medium().child(exec.message.clone()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} rows affected in {} ms", exec.rows_affected, exec.execution_time_ms)),
        )
}

fn export_path(ext: &str) -> std::path::PathBuf {
    let dir = dirs::desktop_dir().or_else(dirs::document_dir).unwrap_or_default();
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    dir.join(format!("export_{}.{}", ts, ext))
}

fn spawn_export_result(
    result: std::thread::JoinHandle<std::io::Result<()>>,
    path_display: String,
    cx: &mut Context<ResultsPanel>,
) {
    cx.spawn(async move |_this, cx| {
        let msg = match result.join() {
            Ok(Ok(())) => format!("Exported to {}", path_display),
            Ok(Err(e)) => format!("Export failed: {}", e),
            Err(_) => "Export failed: thread error".to_string(),
        };
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = msg;
        });
    })
    .detach();
}

impl Render for ResultsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_query_results = matches!(self.result.as_deref(), Some(SqlResult::Query(_)));
        let has_schema = self.selected_schema.is_some();
        let has_selection = self.selected_row_cell.borrow().is_some();
        let has_table = self.current_table.is_some();

        let mut root = v_flex()
            .id("results-panel")
            .size_full()
            .p_2()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(self.render_tab_bar(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(self.render_export_buttons(has_query_results, cx))
                            .child(self.render_info(cx)),
                    ),
            );

        if self.active_tab == ResultsTab::Data {
            root = root
                .child(self.render_toolbar(has_table, has_schema, has_selection, cx))
                .child(self.render_data_content(has_table, cx));
        } else {
            root = root.child(self.render_schema_view(cx));
        }

        if self.show_insert_modal {
            root = root.child(self.render_insert_modal(cx));
        }

        root
    }
}
