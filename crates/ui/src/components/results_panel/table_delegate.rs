use dbstudio_core::result::CellType;

use super::*;

/// Comparison operator for a per-column result filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Contains,
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Like,
    StartsWith,
    IsEmpty,
    NotEmpty,
}

impl FilterOp {
    pub fn label(&self) -> &'static str {
        match self {
            FilterOp::Contains => "contains",
            FilterOp::Equals => "equals",
            FilterOp::NotEquals => "not equals",
            FilterOp::GreaterThan => ">",
            FilterOp::LessThan => "<",
            FilterOp::Like => "like",
            FilterOp::StartsWith => "starts with",
            FilterOp::IsEmpty => "is empty",
            FilterOp::NotEmpty => "not empty",
        }
    }

    /// Whether this operator needs a comparison value.
    pub fn needs_value(&self) -> bool {
        !matches!(self, FilterOp::IsEmpty | FilterOp::NotEmpty)
    }
}

/// One active per-column filter applied to the result grid.
#[derive(Debug, Clone)]
pub struct ColumnFilter {
    /// Data column index (0-based into the result column list, excluding the
    /// leading row-number column the grid adds).
    pub column: usize,
    pub op: FilterOp,
    pub value: String,
}

impl ColumnFilter {
    pub fn matches(&self, cell: Option<&ResultCell>) -> bool {
        cell_matches_filter(cell, self.op, &self.value)
    }
}

/// Whether `cell` satisfies `op` against `value`. NULL cells only match the
/// emptiness operators; numeric cells compare via their sort proxy.
pub fn cell_matches_filter(cell: Option<&ResultCell>, op: FilterOp, value: &str) -> bool {
    let Some(cell) = cell else {
        return false;
    };
    if cell.is_null {
        return matches!(op, FilterOp::IsEmpty);
    }
    match op {
        FilterOp::Contains => cell.value.to_lowercase().contains(&value.to_lowercase()),
        FilterOp::StartsWith => cell.value.to_lowercase().starts_with(&value.to_lowercase()),
        FilterOp::Equals => cell.value.eq_ignore_ascii_case(value),
        FilterOp::NotEquals => !cell.value.eq_ignore_ascii_case(value),
        FilterOp::Like => like_match(&cell.value, value),
        FilterOp::GreaterThan => filter_num(cell, value, |a, b| a > b),
        FilterOp::LessThan => filter_num(cell, value, |a, b| a < b),
        FilterOp::IsEmpty => cell.value.is_empty(),
        FilterOp::NotEmpty => !cell.value.is_empty(),
    }
}

/// SQL `LIKE` matching: `%` matches any run, `_` matches a single character.
pub fn like_match(value: &str, pattern: &str) -> bool {
    let v: Vec<char> = value.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let (m, n) = (v.len(), p.len());
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;
    for j in 1..=n {
        if p[j - 1] == '%' {
            dp[0][j] = dp[0][j - 1];
        }
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = match p[j - 1] {
                '%' => dp[i - 1][j] || dp[i][j - 1],
                '_' => dp[i - 1][j - 1],
                c => dp[i - 1][j - 1] && v[i - 1] == c,
            };
        }
    }
    dp[m][n]
}

/// Numeric or text comparison against a typed cell. Falls back to a substring
/// match when either side is not a number, so `>` on a text column still shows
/// something instead of hiding every row.
fn filter_num(cell: &ResultCell, expected: &str, cmp: impl Fn(f64, f64) -> bool) -> bool {
    let Some(exp) = expected.trim().parse::<f64>().ok() else {
        return cell.value.to_lowercase().contains(&expected.to_lowercase());
    };
    match cell
        .sort_value
        .or_else(|| cell.value.trim().parse::<f64>().ok())
    {
        Some(actual) => cmp(actual, exp),
        None => false,
    }
}

/// Default operator set offered per logical cell type.
pub fn operators_for(kind: CellType) -> &'static [FilterOp] {
    match kind {
        CellType::Integer | CellType::Float | CellType::Decimal | CellType::Boolean => &[
            FilterOp::Equals,
            FilterOp::NotEquals,
            FilterOp::GreaterThan,
            FilterOp::LessThan,
            FilterOp::IsEmpty,
            FilterOp::NotEmpty,
        ],
        CellType::Date | CellType::Time | CellType::DateTime => &[
            FilterOp::GreaterThan,
            FilterOp::LessThan,
            FilterOp::Equals,
            FilterOp::NotEquals,
            FilterOp::IsEmpty,
            FilterOp::NotEmpty,
        ],
        _ => &[
            FilterOp::Contains,
            FilterOp::Equals,
            FilterOp::NotEquals,
            FilterOp::StartsWith,
            FilterOp::Like,
            FilterOp::IsEmpty,
            FilterOp::NotEmpty,
        ],
    }
}

pub struct ResultsTableDelegate {
    columns: Vec<Column>,
    rows: Vec<Vec<ResultCell>>,
    order: Vec<usize>,
    sort_col: Option<usize>,
    sort_asc: bool,
    visible_rows: Range<usize>,
    selected_row_cell: Rc<RefCell<Option<usize>>>,
    selected_cell: Rc<RefCell<Option<(usize, usize)>>>,
    last_click: Rc<RefCell<Option<(usize, usize, Instant)>>>,
    panel_handle: Rc<RefCell<Option<Entity<ResultsPanel>>>>,
    filter: String,
    column_filters: Vec<ColumnFilter>,
    filtered_order: Vec<usize>,
}

impl ResultsTableDelegate {
    pub(super) fn new(
        selected_row_cell: Rc<RefCell<Option<usize>>>,
        selected_cell: Rc<RefCell<Option<(usize, usize)>>>,
        last_click: Rc<RefCell<Option<(usize, usize, Instant)>>>,
        panel_handle: Rc<RefCell<Option<Entity<ResultsPanel>>>>,
    ) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            order: Vec::new(),
            sort_col: None,
            sort_asc: true,
            visible_rows: Range::default(),
            selected_row_cell,
            selected_cell,
            last_click,
            panel_handle,
            filter: String::new(),
            column_filters: Vec::new(),
            filtered_order: Vec::new(),
        }
    }

    pub(super) fn update(&mut self, result: &QueryResult) {
        self.rows = result.rows.clone();
        self.order = (0..self.rows.len()).collect();
        self.sort_col = None;
        self.sort_asc = true;
        let mut cols = vec![Column::new("#", "#").width(px(60.0))];
        cols.extend(result.columns.iter().enumerate().map(|(col_ix, col)| {
            let header_len = col.name.chars().count().max(1);
            let cell_len = self
                .rows
                .iter()
                .filter_map(|row| row.get(col_ix))
                .map(|cell| cell.value.chars().count())
                .max()
                .unwrap_or(0);
            let width = (header_len.max(cell_len) as f32 * 7.5 + 24.0).clamp(80.0, 3000.0);
            Column::new(&col.name, &col.name)
                .sortable()
                .width(px(width))
        }));
        self.columns = cols;
        self.rebuild_filtered_order();
    }

    pub(super) fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.rebuild_filtered_order();
    }

    pub(super) fn set_column_filters(&mut self, filters: &[ColumnFilter]) {
        self.column_filters = filters.to_vec();
        self.rebuild_filtered_order();
    }

    pub(super) fn clear_filters(&mut self) {
        self.filter.clear();
        self.column_filters.clear();
        self.rebuild_filtered_order();
    }

    fn rebuild_filtered_order(&mut self) {
        if self.filter.is_empty() && self.column_filters.is_empty() {
            self.filtered_order = self.order.clone();
            return;
        }
        let query = self.filter.to_lowercase();
        self.filtered_order = self
            .order
            .iter()
            .copied()
            .filter(|&row_ix| self.row_matches(row_ix, &query))
            .collect();
    }

    fn row_matches(&self, row_ix: usize, query: &str) -> bool {
        let Some(row) = self.rows.get(row_ix) else {
            return false;
        };
        let text_ok = query.is_empty()
            || row
                .iter()
                .any(|cell| cell.value.to_lowercase().contains(query));
        if !text_ok {
            return false;
        }
        self.column_filters
            .iter()
            .all(|f| f.matches(row.get(f.column)))
    }

    fn apply_sort(&mut self) {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        if let Some(col) = self.sort_col {
            order.sort_by(|&a, &b| {
                let ca = self.rows[a].get(col);
                let cb = self.rows[b].get(col);
                let ord = match (ca, cb) {
                    (Some(a), Some(b)) => a.sort_cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (None, None) => std::cmp::Ordering::Equal,
                };
                if self.sort_asc {
                    ord
                } else {
                    ord.reverse()
                }
            });
        }
        self.order = order;
        self.rebuild_filtered_order();
    }

    pub(super) fn cell_named(&self, display_row: usize, name: &str) -> Option<&ResultCell> {
        let display_col = self.columns.iter().position(|c| c.name.as_ref() == name)?;
        if display_col == 0 {
            return None;
        }
        let row_ix = *self.filtered_order.get(display_row)?;
        self.rows.get(row_ix)?.get(display_col - 1)
    }
}

impl ResultsTableDelegate {
    /// Whether the cell at (display row, display col) is currently being edited.
    fn is_editing_cell(&self, cx: &App, display_row: usize, display_col: usize) -> bool {
        match self.panel_handle.borrow().as_ref() {
            Some(panel) => {
                let p = panel.read(cx);
                p.editing_cell == Some((display_row, display_col))
            }
            None => false,
        }
    }

    fn render_edit_input(
        &mut self,
        col_ix: usize,
        cx: &mut Context<TableState<Self>>,
    ) -> AnyElement {
        let input = self
            .panel_handle
            .borrow()
            .as_ref()
            .map(|panel| panel.read(cx).editing_input.clone());
        let width = self
            .columns
            .get(col_ix)
            .map(|c| c.width)
            .unwrap_or(px(120.0));
        let panel_handle = self.panel_handle.clone();
        match input {
            Some(input) => div()
                .id("cell-edit-input")
                .h_full()
                .w(width)
                .on_key_down(move |event, _window, cx| {
                    if event.keystroke.key == "escape" {
                        if let Some(panel) = panel_handle.borrow().clone() {
                            panel.update(cx, |panel, cx| panel.cancel_cell_edit(cx));
                        }
                    }
                })
                .child(Input::new(&input).small())
                .into_any_element(),
            None => Label::new("--").text_sm().into_any_element(),
        }
    }
}

impl TableDelegate for ResultsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.filtered_order.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        if col_ix == 0 || col_ix > self.columns.len().saturating_sub(1) {
            return;
        }
        let data_col = col_ix - 1;
        match sort {
            ColumnSort::Default => {
                self.sort_col = None;
                self.sort_asc = true;
            }
            ColumnSort::Ascending => {
                self.sort_col = Some(data_col);
                self.sort_asc = true;
            }
            ColumnSort::Descending => {
                self.sort_col = Some(data_col);
                self.sort_asc = false;
            }
        }
        self.apply_sort();
        self.selected_row_cell.borrow_mut().take();
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let col = self.column(col_ix, cx);
        div().child(col.name)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let selected = *self.selected_row_cell.borrow() == Some(row_ix);
        let cell = self.selected_row_cell.clone();
        div()
            .id(row_ix)
            .cursor_pointer()
            .when(selected, |this| this.bg(gpui::blue().opacity(0.1)))
            .on_click(move |_event, _window, _app| {
                *cell.borrow_mut() = Some(row_ix);
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if col_ix == 0 {
            return Label::new((row_ix + 1).to_string())
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .into_any_element();
        }
        let display_row = row_ix;
        let row_ix = match self.filtered_order.get(row_ix) {
            Some(ix) => *ix,
            None => {
                return Label::new("--").text_sm().into_any_element();
            }
        };

        // Render the input in place while this cell is being edited.
        if self.is_editing_cell(cx, display_row, col_ix) {
            return self.render_edit_input(col_ix, cx);
        }

        if let Some(row) = self.rows.get(row_ix) {
            if let Some(cell) = row.get(col_ix - 1) {
                let value = if cell.is_null {
                    String::new()
                } else {
                    cell.value.clone()
                };
                let is_null = cell.is_null;
                let selected_cell = self.selected_cell.clone();
                let selected_row_cell = self.selected_row_cell.clone();
                let last_click = self.last_click.clone();
                let panel_handle = self.panel_handle.clone();
                let click_target = div()
                    .id(format!("cell-{}-{}", display_row, col_ix))
                    .cursor_pointer()
                    .on_click(move |_event, window, cx| {
                        let now = Instant::now();
                        let is_double = last_click
                            .borrow()
                            .map(|(r, c, t)| {
                                r == display_row
                                    && c == col_ix
                                    && now.duration_since(t) < Duration::from_millis(350)
                            })
                            .unwrap_or(false);
                        *last_click.borrow_mut() = Some((display_row, col_ix, now));
                        *selected_row_cell.borrow_mut() = Some(display_row);
                        *selected_cell.borrow_mut() = Some((display_row, col_ix));
                        if is_double {
                            if let Some(panel) = panel_handle.borrow().clone() {
                                panel.update(cx, |panel, cx| {
                                    panel.start_cell_edit(
                                        display_row,
                                        col_ix,
                                        value.clone(),
                                        is_null,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }
                    });
                if cell.is_null {
                    return click_target
                        .child(
                            Label::new("NULL")
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .italic(),
                        )
                        .into_any_element();
                }
                return click_target
                    .child(Label::new(cell.value.clone()).text_sm())
                    .into_any_element();
            }
        }
        Label::new("--").text_sm().into_any_element()
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let col = self.columns.remove(col_ix);
        let clamped = to_ix.min(self.columns.len());
        self.columns.insert(clamped, col);

        for row in &mut self.rows {
            if col_ix < row.len() {
                let cell = row.remove(col_ix);
                let insert_at = clamped.min(row.len());
                row.insert(insert_at, cell);
            }
        }
    }

    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.visible_rows = visible_range;
    }
}
