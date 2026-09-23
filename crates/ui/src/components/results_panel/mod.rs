use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dbstudio_core::result::{
    CellType, ErrorResult, ExecResult, QueryResult, ResultCell, SqlResult,
};
use dbstudio_core::schema::{ColumnInfo, TableSchema};
use dbstudio_db::utils::quote_string_literal;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    label::Label,
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectItem, SelectState},
    table::{Column, ColumnSort, DataTable, TableDelegate, TableState},
    v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _,
    StyledExt as _,
};

use crate::state::{AppState, build_result_page_sql, fetch_result_page};

mod actions;
mod export;
mod table_delegate;
mod views;

use table_delegate::ResultsTableDelegate;
pub use table_delegate::{cell_matches_filter, like_match, operators_for, ColumnFilter, FilterOp};

#[derive(Clone, Copy, PartialEq)]
enum ResultsTab {
    Data,
    Schema,
}

/// Filter-column dropdown option: one entry per result column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterColOption {
    pub index: usize,
    pub name: String,
}

impl SelectItem for FilterColOption {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.index
    }
}

/// Filter-operator dropdown option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterOpOption(pub FilterOp);

impl FilterOpOption {
    fn from_wire(value: &str) -> FilterOp {
        match value {
            "contains" => FilterOp::Contains,
            "equals" => FilterOp::Equals,
            "not-equals" => FilterOp::NotEquals,
            "greater-than" => FilterOp::GreaterThan,
            "less-than" => FilterOp::LessThan,
            "like" => FilterOp::Like,
            "starts-with" => FilterOp::StartsWith,
            "is-empty" => FilterOp::IsEmpty,
            _ => FilterOp::NotEmpty,
        }
    }
}

impl SelectItem for FilterOpOption {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        self.0.label().into()
    }

    fn value(&self) -> &Self::Value {
        match self.0 {
            FilterOp::Contains => &"contains",
            FilterOp::Equals => &"equals",
            FilterOp::NotEquals => &"not-equals",
            FilterOp::GreaterThan => &"greater-than",
            FilterOp::LessThan => &"less-than",
            FilterOp::Like => &"like",
            FilterOp::StartsWith => &"starts-with",
            FilterOp::IsEmpty => &"is-empty",
            FilterOp::NotEmpty => &"not-empty",
        }
    }
}

fn filter_op_options_for(kind: CellType) -> Vec<FilterOpOption> {
    operators_for(kind)
        .iter()
        .map(|op| FilterOpOption(*op))
        .collect()
}

pub struct ResultsPanel {
    window_id: u64,
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
    /// Back-reference to this panel so the table delegate can trigger edits.
    panel_handle: Rc<RefCell<Option<Entity<Self>>>>,
    /// Cell currently being edited in the grid, as (display row, display col).
    editing_cell: Option<(usize, usize)>,
    /// Input used for in-place grid editing.
    editing_input: Entity<InputState>,
    /// Original text + NULL flag of the cell being edited.
    editing_original: Option<(String, bool)>,
    /// Whether the pending-changes (diff review) modal is open.
    show_review_modal: bool,
    /// Text filter for filtering result rows across all columns.
    filter_input: Entity<InputState>,
    filter_text: String,
    /// Type-aware per-column filters (AND-combined with the text filter).
    column_filters: Vec<ColumnFilter>,
    /// Composer (column/op/value) visibility for adding a column filter.
    show_filter_composer: bool,
    /// Data column index selected in the composer.
    filter_col_index: usize,
    filter_col_select: Entity<SelectState<Vec<FilterColOption>>>,
    filter_op: FilterOp,
    filter_op_select: Entity<SelectState<Vec<FilterOpOption>>>,
    filter_value_input: Entity<InputState>,
    /// Server-side page size; the cycle button picks one of these values.
    page_size: usize,
    /// Byte/row offset of the current page within the base query.
    page_offset: usize,
    /// Accumulated offsets of previous pages (for Back navigation).
    page_history: Vec<usize>,
    /// The user's own SELECT before paging/filter wrapping; `None` for
    /// non-query results (writes, DDL, errors).
    base_sql: Option<String>,
    /// SQL of the last page we fetched ourselves, used to distinguish our own
    /// page fetches from fresh user queries in the result subscription.
    last_sent_sql: Option<String>,
    /// The active server-side keyword filter, if any.
    server_filter: Option<String>,
    /// Whether the current page fetch indicated more rows may exist.
    more_pages: bool,
    _subscriptions: Vec<Subscription>,
}

impl ResultsPanel {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let window_id = window.window_handle().window_id().as_u64();
        let panel = cx.new(|cx| Self::new(window_id, window, cx));
        panel
            .read(cx)
            .panel_handle
            .borrow_mut()
            .replace(panel.clone());
        panel
    }

    fn new(window_id: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let selected_row_cell = Rc::new(RefCell::new(None));
        let selected_cell = Rc::new(RefCell::new(None));
        let last_click = Rc::new(RefCell::new(None));
        let panel_handle = Rc::new(RefCell::new(None));
        let table = cx.new(|cx| {
            TableState::new(
                ResultsTableDelegate::new(
                    selected_row_cell.clone(),
                    selected_cell.clone(),
                    last_click.clone(),
                    panel_handle.clone(),
                ),
                window,
                cx,
            )
            .sortable(true)
        });

        let editing_input = cx.new(|cx| InputState::new(window, cx).placeholder("Edit value..."));

        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter rows...")
                .clean_on_escape()
        });

        let filter_value_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value...")
                .clean_on_escape()
        });

        let filter_col_select =
            cx.new(|cx| SelectState::new(Vec::<FilterColOption>::new(), None, window, cx));
        cx.subscribe_in(&filter_col_select, window, Self::on_filter_col_change)
            .detach();

        let filter_op_select =
            cx.new(|cx| SelectState::new(Vec::<FilterOpOption>::new(), None, window, cx));
        cx.subscribe_in(&filter_op_select, window, Self::on_filter_op_change)
            .detach();

        let editing_input_sub = editing_input.clone();
        cx.subscribe_in(
            &editing_input_sub,
            window,
            |this, _emitter, event: &gpui_component::input::InputEvent, win, cx| match event {
                gpui_component::input::InputEvent::PressEnter { .. }
                | gpui_component::input::InputEvent::Blur => this.commit_cell_edit(win, cx),
                _ => {}
            },
        )
        .detach();

        let filter_input_sub = filter_input.clone();
        let _subscriptions = vec![cx.observe_global::<AppState>(move |this, cx| {
            let state = cx.global::<AppState>();

            let state_result_ptr = state
                .last_result_for(this.window_id)
                .map(Arc::as_ptr);
            let current_result_ptr = this.result.as_ref().map(Arc::as_ptr);
            let result_changed = state_result_ptr != current_result_ptr;

            let pending_schema = if let Some(ref t) = this.pending_table {
                state.table_schemas_for(this.window_id).get(t).cloned()
            } else {
                None
            };

            if result_changed {
                let prev_columns = match this.result.as_deref() {
                    Some(SqlResult::Query(q)) => Some(q.columns.clone()),
                    _ => None,
                };
                this.result = state.last_result_for(this.window_id).cloned();
                // Keep the text/column filters when re-running or paging
                // over the same result shape; clear them otherwise.
                let shape_same = matches!(
                    (&prev_columns, this.result.as_deref()),
                    (Some(a), Some(SqlResult::Query(b))) if a == &b.columns
                );
                if !shape_same {
                    this.filter_text.clear();
                    this.column_filters.clear();
                    this.show_filter_composer = false;
                }

                match this.result.as_deref() {
                    Some(SqlResult::Query(query)) => {
                        this.active_tab = ResultsTab::Data;

                        // Track server-side paging: our own page fetch advances
                        // the window; any other query starts a fresh base.
                        if this.last_sent_sql.as_deref() == Some(query.sql.as_str()) {
                            this.last_sent_sql = None;
                            this.more_pages = query.rows.len() >= this.page_size;
                        } else {
                            this.base_sql = Some(query.sql.clone());
                            this.page_offset = 0;
                            this.page_history.clear();
                            this.server_filter = None;
                            this.more_pages = query.truncated;
                        }

                        let query = query.clone();
                        let filter_text = this.filter_text.clone();
                        let column_filters = this.column_filters.clone();
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().update(&query);
                            table.delegate_mut().set_filter(&filter_text);
                            table.delegate_mut().set_column_filters(&column_filters);
                            table.refresh(cx);
                        });
                    }
                    _ => {
                        this.filter_text.clear();
                        this.column_filters.clear();
                        this.show_filter_composer = false;
                        this.last_sent_sql = None;
                        if !matches!(this.result.as_deref(), Some(SqlResult::Error(_))) {
                            // A write/DDL result replaced the query: drop paging.
                            this.base_sql = None;
                            this.page_offset = 0;
                            this.page_history.clear();
                            this.server_filter = None;
                            this.more_pages = false;
                        }
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().update(&QueryResult::empty(""));
                            table.delegate_mut().clear_filters();
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

        cx.subscribe_in(
            &filter_input_sub,
            window,
            |this, _, _event: &gpui_component::input::InputEvent, _win, cx| {
                this.filter_text = this.filter_input.read(cx).value().to_string();
                this.apply_filter(cx);
            },
        )
        .detach();

        let initial_result = cx.global::<AppState>().last_result_for(window_id).cloned();
        let initial_base = match initial_result.as_deref() {
            Some(SqlResult::Query(q)) => Some(q.sql.clone()),
            _ => None,
        };
        let initial_more_pages =
            matches!(initial_result.as_deref(), Some(SqlResult::Query(q)) if q.truncated);

        Self {
            window_id,
            result: initial_result,
            base_sql: initial_base,
            last_sent_sql: None,
            page_size: 100,
            page_offset: 0,
            page_history: Vec::new(),
            server_filter: None,
            more_pages: initial_more_pages,
            table,
            selected_schema: None,
            pending_table: None,
            current_table: None,
            active_tab: ResultsTab::Data,
            selected_row_cell,
            panel_handle,
            editing_cell: None,
            editing_input,
            editing_original: None,
            show_review_modal: false,
            show_insert_modal: false,
            editing_row: None,
            edit_original_row: None,
            insert_inputs: Vec::new(),
            insert_columns: Vec::new(),
            filter_input,
            filter_text: String::new(),
            column_filters: Vec::new(),
            show_filter_composer: false,
            filter_col_index: 0,
            filter_col_select,
            filter_op: FilterOp::Contains,
            filter_op_select,
            filter_value_input,
            _subscriptions,
        }
    }

    /// Rebuild the filter-column dropdown from the active query result and
    /// reset the composer selection to the first column.
    fn update_filter_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let columns = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.columns.clone(),
            _ => Vec::new(),
        };
        if columns.is_empty() {
            return;
        }
        let items: Vec<FilterColOption> = columns
            .iter()
            .enumerate()
            .map(|(index, col)| FilterColOption {
                index,
                name: col.name.clone(),
            })
            .collect();
        self.filter_col_index = 0;
        self.filter_col_select.update(cx, |this, cx| {
            this.set_items(items, window, cx);
            this.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.rebuild_filter_op_options(window, cx);
    }

    /// Rebuild the operator dropdown for the currently selected column's type.
    fn rebuild_filter_op_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.composer_cell_type(cx);
        let items = filter_op_options_for(kind);
        self.filter_op = items[0].0;
        self.filter_op_select.update(cx, |this, cx| {
            this.set_items(items, window, cx);
            this.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
    }

    /// The logical type of the composer's selected column, sampled from the
    /// first non-NULL cell, falling back to `Text` for unknown columns.
    fn composer_cell_type(&self, _cx: &mut Context<Self>) -> CellType {
        match self.result.as_deref() {
            Some(SqlResult::Query(q)) => {
                let col = self.filter_col_index.min(q.columns.len().saturating_sub(1));
                q.rows
                    .iter()
                    .find_map(|row| row.get(col).filter(|c| !c.is_null).map(|c| c.kind))
                    .unwrap_or(CellType::Text)
            }
            _ => CellType::Text,
        }
    }

    /// Apply the current column filters to the table delegate.
    fn apply_column_filters(&mut self, cx: &mut Context<Self>) {
        let filters = self.column_filters.clone();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_column_filters(&filters);
            table.refresh(cx);
        });
    }

    fn on_filter_col_change(
        &mut self,
        _: &Entity<SelectState<Vec<FilterColOption>>>,
        event: &SelectEvent<Vec<FilterColOption>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.filter_col_index = *value;
            self.rebuild_filter_op_options(window, cx);
            cx.notify();
        }
    }

    fn on_filter_op_change(
        &mut self,
        _: &Entity<SelectState<Vec<FilterOpOption>>>,
        event: &SelectEvent<Vec<FilterOpOption>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(value)) = event {
            self.filter_op = FilterOpOption::from_wire(value);
            cx.notify();
        }
    }

    fn toggle_filter_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_filter_composer = !self.show_filter_composer;
        if self.show_filter_composer {
            self.update_filter_columns(window, cx);
        }
        cx.notify();
    }

    fn add_column_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.filter_value_input.read(cx).value().to_string();
        if self.filter_op.needs_value() && value.trim().is_empty() {
            self.status(cx, "Filter requires a value".to_string());
            return;
        }
        self.column_filters.push(ColumnFilter {
            column: self.filter_col_index,
            op: self.filter_op,
            value,
        });
        self.filter_value_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.apply_column_filters(cx);
        self.show_filter_composer = false;
        cx.notify();
    }

    fn remove_column_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.column_filters.len() {
            self.column_filters.remove(index);
        }
        self.apply_column_filters(cx);
        cx.notify();
    }

    fn clear_column_filters(&mut self, cx: &mut Context<Self>) {
        self.column_filters.clear();
        self.apply_column_filters(cx);
        cx.notify();
    }

    /// Fetch one server-side page of `base_sql` (optionally keyword-filtered)
    /// and display it. `push` records the current offset for Back navigation.
    ///
    /// Clears `last_sent_sql` first so the result subscription can tell this
    /// fetch apart from a fresh user query.
    fn fetch_page(&mut self, offset: usize, push: bool, cx: &mut Context<Self>) {
        let Some(base) = self.base_sql.clone() else {
            self.status(cx, "No query to page through".to_string());
            cx.notify();
            return;
        };
        let filter = self.server_filter.clone().unwrap_or_default();
        let columns: Vec<String> = match self.result.as_deref() {
            Some(SqlResult::Query(q)) => q.columns.iter().map(|c| c.name.clone()).collect(),
            _ => Vec::new(),
        };
        if push {
            self.page_history.push(self.page_offset);
        }
        self.last_sent_sql = None;
        let sql = build_result_page_sql(
            &base,
            &filter,
            &columns,
            offset,
            self.page_size,
            self.window_id,
            cx,
        );
        self.last_sent_sql = Some(sql.clone());
        self.page_offset = offset;
        fetch_result_page(sql, self.window_id, cx);
        cx.notify();
    }

    /// Run `filter_text` as a server-side keyword filter from offset 0.
    fn apply_server_filter(&mut self, cx: &mut Context<Self>) {
        if self.base_sql.is_none() {
            self.status(cx, "No query to filter".to_string());
            cx.notify();
            return;
        }
        self.server_filter = Some(self.filter_text.trim().to_string());
        self.page_history.clear();
        self.fetch_page(0, false, cx);
    }

    /// Cycle the server-side page size between 100 / 500 / 1000 rows.
    fn cycle_page_size(&mut self, cx: &mut Context<Self>) {
        self.page_size = match self.page_size {
            100 => 500,
            500 => 1000,
            _ => 100,
        };
        self.page_history.clear();
        self.fetch_page(0, false, cx);
    }

    /// Apply the text filter to the table delegate.
    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter_text.clone();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_filter(&filter);
            table.refresh(cx);
        });
        cx.notify();
    }

    pub fn select_table(
        &mut self,
        table_name: String,
        schema: Option<TableSchema>,
        cx: &mut Context<Self>,
    ) {
        self.current_table = Some(table_name.clone());
        self.pending_table = Some(table_name);
        self.selected_schema = schema;
        self.result = None;
        self.active_tab = ResultsTab::Data;
        self.filter_text.clear();
        self.column_filters.clear();
        self.show_filter_composer = false;
        *self.selected_row_cell.borrow_mut() = None;
        cx.update_global::<AppState, _>(|state, _cx| {
            if let Some(s) = state.active_session_mut_for(self.window_id) {
                s.last_result = None;
            }
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().update(&QueryResult::empty(""));
            table.delegate_mut().clear_filters();
            table.delegate_mut().set_filter("");
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }

    fn set_active_tab(&mut self, tab: ResultsTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    /// Begin in-place editing of the cell at (display row, display col).
    /// `value`/`is_null` describe the current cell contents.
    pub fn start_cell_edit(
        &mut self,
        display_row: usize,
        display_col: usize,
        value: String,
        is_null: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_cell = Some((display_row, display_col));
        self.editing_original = Some((value.clone(), is_null));
        let initial = if is_null { String::new() } else { value };
        self.editing_input.update(cx, |input, cx| {
            input.set_value(initial, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Cancel the current grid edit without staging anything.
    pub fn cancel_cell_edit(&mut self, cx: &mut Context<Self>) {
        self.editing_cell = None;
        self.editing_original = None;
        cx.notify();
    }

    /// Commit the active grid edit: stage an UPDATE (or nothing when the cell
    /// did not change) into the session's pending-edit buffer.
    pub fn commit_cell_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((display_row, display_col)) = self.editing_cell.take() else {
            return;
        };
        let original = self.editing_original.take();
        let old = match &original {
            Some((value, is_null)) => {
                if *is_null {
                    None
                } else {
                    Some(value.clone())
                }
            }
            None => None,
        };
        let new = actions::cell_value_str(self.editing_input.read(cx).value().as_str());

        if old == new {
            cx.notify();
            return;
        }

        let table = match &self.current_table {
            Some(t) => t.clone(),
            None => {
                cx.notify();
                return;
            }
        };

        let table_entity = self.table.clone();
        // Column at the display position (display col 0 is the row number).
        let col_name = table_entity
            .read(cx)
            .delegate()
            .column(display_col, cx)
            .name
            .to_string();
        if col_name.is_empty() {
            cx.notify();
            return;
        }

        let Some(schema) = self.selected_schema.clone() else {
            cx.notify();
            return;
        };
        let pk_columns: Vec<ColumnInfo> = schema
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .cloned()
            .collect();
        if pk_columns.is_empty() {
            self.status(cx, "Cannot edit: the table has no primary key".to_string());
            cx.notify();
            return;
        }

        let conditions: Vec<String> = pk_columns
            .iter()
            .filter_map(|pk| {
                let col_q = crate::state::quote_ident(&pk.name, self.window_id, cx);
                let cell = table_entity
                    .read(cx)
                    .delegate()
                    .cell_named(display_row, &pk.name)?;
                if cell.is_null {
                    Some(format!("{} IS NULL", col_q))
                } else {
                    Some(format!("{} = {}", col_q, quote_string_literal(&cell.value)))
                }
            })
            .collect();
        if conditions.is_empty() {
            cx.notify();
            return;
        }

        let table_q = crate::state::quote_ident(&table, self.window_id, cx);
        let col_q = crate::state::quote_ident(&col_name, self.window_id, cx);
        let where_clause = conditions.join(" AND ");
        let sets = format!("{} = {}", col_q, actions::quoted_or_null(new.as_deref()));
        let sql = format!("UPDATE {} SET {} WHERE {};", table_q, sets, where_clause);
        let inverse_set = format!("{} = {}", col_q, actions::quoted_or_null(old.as_deref()));
        let inverse_sql = format!(
            "UPDATE {} SET {} WHERE {};",
            table_q, inverse_set, where_clause
        );

        let diff = Some(crate::state::guard::PendingDiff {
            table: table.clone(),
            op: crate::state::guard::DiffOp::Update,
            row_key: where_clause.clone(),
            cells: vec![crate::state::guard::CellDiff {
                column: col_name,
                old_value: old,
                new_value: new,
            }],
        });

        self.editing_input.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        self.editing_original = None;
        // Highlight the cell as modified pending review
        self.table.update(cx, |table, cx| {
            table.delegate_mut().mark_modified(display_row, display_col);
            cx.notify();
        });
        self.stage_edit(sql, inverse_sql, "UPDATE".to_string(), diff, cx);
    }

    fn status(&mut self, cx: &mut Context<Self>, message: String) {
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = message;
        });
    }

    /// Toggle the pending-changes (diff review) modal.
    pub fn toggle_review_modal(&mut self, cx: &mut Context<Self>) {
        self.show_review_modal = !self.show_review_modal;
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
    Label::new(text)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
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
        .child(
            div()
                .text_base()
                .font_bold()
                .text_color(gpui::red())
                .child("Query Error"),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(err.message.clone()),
        )
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
                .child(format!(
                    "{} rows affected in {} ms",
                    exec.rows_affected, exec.execution_time_ms
                )),
        )
}

fn export_path(ext: &str) -> std::path::PathBuf {
    let dir = dirs::desktop_dir()
        .or_else(dirs::document_dir)
        .unwrap_or_default();
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
        let has_filter = !self.filter_text.is_empty();

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
                            .child(self.render_load_more_button(cx))
                            .child(self.render_export_buttons(has_query_results, cx))
                            .child(self.render_info(cx)),
                    ),
            );

        if self.active_tab == ResultsTab::Data {
            root = root
                .child(
                    v_flex()
                        .gap_1()
                        .child(self.render_text_filter_bar(has_filter, cx))
                        .child(self.render_filter_bar(cx)),
                )
                .child(self.render_toolbar(has_table, has_schema, has_selection, cx))
                .child(self.render_data_content(has_table, cx));
        } else {
            root = root.child(self.render_schema_view(cx));
        }

        if self.show_insert_modal {
            root = root.child(self.render_insert_modal(cx));
        }

        if self.show_review_modal {
            root = root.child(self.render_review_modal(cx));
        }

        root
    }
}
