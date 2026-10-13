use std::collections::{HashMap, HashSet};

use dbstudio_core::result::{CellType, QueryResult, SqlResult};

use crate::fold::{contains_ignore_case, fold_into_chars, starts_with_ignore_case};

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

/// Whether `cell` satisfies `op` against `value`. NULL cells only match the
/// emptiness operators; numeric cells compare via their sort proxy.
///
/// `value` here is the raw, unfolded filter text. Callers that check many cells
/// per pass go through [`PreparedFilter`] instead, which folds it once and - for
/// `>` / `<` - decides once per pass whether the operand is a number at all.
pub fn cell_matches_filter(cell: Option<&ResultCell>, op: FilterOp, value: &str) -> bool {
    let Some(cell) = cell else {
        return false;
    };
    if cell.is_null {
        return matches!(op, FilterOp::IsEmpty);
    }
    match op {
        FilterOp::Contains => contains_ignore_case(&cell.value, &value.to_lowercase()),
        FilterOp::StartsWith => starts_with_ignore_case(&cell.value, &value.to_lowercase()),
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
    LikePattern::new(pattern).matches(value)
}

/// A `LIKE` pattern with its case folding and scan done once.
///
/// Building this per cell was the cost: the matcher used to allocate a
/// lowercased copy of the value *and* an `(m+1) x (n+1)` dynamic-programming
/// table - one `Vec` per row of it - for every cell of every row on each
/// keystroke. Preparing the pattern once per filter pass and scanning with two
/// pointers accepts exactly the language that table did, at O(1) extra space.
pub struct LikePattern {
    chars: Vec<char>,
}

impl LikePattern {
    pub fn new(pattern: &str) -> Self {
        Self {
            chars: fold_into_chars(pattern),
        }
    }

    /// Does `value` match the prepared pattern?
    ///
    /// The classic last-star backtracking scan: `star` is the pattern index
    /// just after the most recent `%`, `star_j` the value index that `%` was
    /// made to consume up to. A mismatch after a `%` replays from `star` with
    /// the star one character longer, so no cell needs a search table.
    pub fn matches(&self, value: &str) -> bool {
        let v = fold_into_chars(value);
        let (mut i, mut j) = (0usize, 0usize);
        let mut star: Option<(usize, usize)> = None;

        while i < v.len() {
            if j < self.chars.len() && (self.chars[j] == '_' || self.chars[j] == v[i]) {
                i += 1;
                j += 1;
            } else if j < self.chars.len() && self.chars[j] == '%' {
                star = Some((j + 1, i));
                j += 1;
            } else if let Some((replay_j, replay_i)) = star {
                star = Some((replay_j, replay_i + 1));
                j = replay_j;
                i = replay_i + 1;
            } else {
                return false;
            }
        }

        while j < self.chars.len() && self.chars[j] == '%' {
            j += 1;
        }
        j == self.chars.len()
    }
}

/// A column filter with everything that does not depend on the cell hoisted out
/// of the per-cell loop. See [`LikePattern`] for why that mattered.
pub struct PreparedFilter {
    column: usize,
    kind: PreparedOp,
}

enum PreparedOp {
    Like(LikePattern),
    /// Case-insensitive text ops, with the pattern text folded once for the
    /// pass. A `>` / `<` whose operand is not a number lands here too: the
    /// fallback branch of `filter_num` returns before it ever reaches its
    /// comparison closure, so both operators mean "the operand occurs in the
    /// cell", which is exactly `op: Contains` against the folded operand.
    Folded {
        op: FilterOp,
        needle: String,
    },
    /// `>` / `<` with a numeric operand: the operand is parsed once for the
    /// pass, only the cell's own numeric proxy is read per cell.
    Number {
        gt: bool,
        expected: f64,
    },
    Direct {
        op: FilterOp,
        value: String,
    },
}

// Thread-local tally of the pass-level operand preparations the `>` / `<`
// route makes. Test builds only, exactly like `fold.rs`'s `SCRATCH_FOLDS`: the
// `#[cfg(test)]` line inside each helper below compiles away in the product,
// so hoisting costs nothing for being observable. (A `//` comment, not a doc
// comment: doc comments on a macro invocation are a warning, not rustdoc.)
//
// Mind what it pins: it counts **calls**, not allocations. The allocation
// account (`N` folds per pass down to none) is a measurement made on the bench
// side with its own counting allocator; a call counter is corroborating
// structure, not the same evidence, and the two must not be quoted as one.
#[cfg(test)]
thread_local! {
    static OPERAND_PREPARATIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Zero the pass-level operand preparation tally. Test builds only.
#[cfg(test)]
fn reset_operand_preparations() {
    OPERAND_PREPARATIONS.with(|cell| cell.set(0));
}

/// Operand preparations counted since the last reset. Test builds only.
#[cfg(test)]
fn operand_preparations() -> u32 {
    OPERAND_PREPARATIONS.with(|cell| cell.get())
}

/// Is `operand` a number? `operand.trim().parse::<f64>().ok()`, counted - the
/// judgement `filter_num` runs at the head of its body, character for
/// character. That sameness *is* the equivalence argument, so a second
/// spelling of it (a `trim` on one side, a different error handling on the
/// other) would silently break the pass-level decision's claim to stand in for
/// the per-cell one. Overflow is not an error: `"1e400"` parses as `+inf`.
#[inline(always)]
fn parse_operand(operand: &str) -> Option<f64> {
    #[cfg(test)]
    OPERAND_PREPARATIONS.with(|cell| cell.set(cell.get() + 1));
    operand.trim().parse::<f64>().ok()
}

/// `operand.to_lowercase()`, counted: the fold `filter_num`'s fallback branch
/// used to redo for every cell, one heap allocation among them.
#[inline(always)]
fn fold_operand(operand: &str) -> String {
    #[cfg(test)]
    OPERAND_PREPARATIONS.with(|cell| cell.set(cell.get() + 1));
    operand.to_lowercase()
}

impl PreparedFilter {
    pub fn prepare(filter: &ColumnFilter) -> Self {
        let kind = match filter.op {
            FilterOp::Like => PreparedOp::Like(LikePattern::new(&filter.value)),
            FilterOp::Contains | FilterOp::StartsWith => PreparedOp::Folded {
                op: filter.op,
                needle: filter.value.to_lowercase(),
            },
            FilterOp::GreaterThan | FilterOp::LessThan => {
                let gt = filter.op == FilterOp::GreaterThan;
                match parse_operand(&filter.value) {
                    Some(expected) => PreparedOp::Number { gt, expected },
                    None => PreparedOp::Folded {
                        // The comparison closure of `filter_num` is never
                        // reached on this branch - it returns first - so `>`
                        // and `<` are the same predicate here: the operand
                        // occurs in the cell.
                        op: FilterOp::Contains,
                        needle: fold_operand(&filter.value),
                    },
                }
            }
            _ => PreparedOp::Direct {
                op: filter.op,
                value: filter.value.clone(),
            },
        };
        Self {
            column: filter.column,
            kind,
        }
    }

    pub fn matches(&self, cell: Option<&ResultCell>) -> bool {
        match &self.kind {
            PreparedOp::Direct { op, value } => cell_matches_filter(cell, *op, value),
            PreparedOp::Number { gt, expected } => match cell {
                Some(cell) if !cell.is_null => match cell
                    .sort_value
                    .or_else(|| cell.value.trim().parse::<f64>().ok())
                {
                    Some(actual) => {
                        if *gt {
                            actual > *expected
                        } else {
                            actual < *expected
                        }
                    }
                    None => false,
                },
                // Same NULL rule as `cell_matches_filter`: a value-bearing
                // operator matches no NULL cell. Dropping this guard turns
                // `NULL > ""` from false into true, because the folded arm's
                // `contains_ignore_case` answers an empty needle with `true`
                // (measured - see the test that pins it).
                _ => false,
            },
            PreparedOp::Folded { op, needle } => match cell {
                // Same NULL rule as `cell_matches_filter`; none of the operators
                // routed here - `contains`, `starts with`, and a non-numeric
                // `>` / `<` alike - is an emptiness operator, so a NULL cell
                // never passes.
                Some(cell) if !cell.is_null => match *op {
                    FilterOp::Contains => contains_ignore_case(&cell.value, needle),
                    _ => starts_with_ignore_case(&cell.value, needle),
                },
                _ => false,
            },
            // A NULL cell satisfies no value-bearing operator, `LIKE` included.
            PreparedOp::Like(pattern) => match cell {
                Some(cell) if !cell.is_null => pattern.matches(&cell.value),
                _ => false,
            },
        }
    }
}

/// Numeric or text comparison against a typed cell. Falls back to a substring
/// match when either side is not a number, so `>` on a text column still shows
/// something instead of hiding every row.
fn filter_num(cell: &ResultCell, expected: &str, cmp: impl Fn(f64, f64) -> bool) -> bool {
    let Some(exp) = expected.trim().parse::<f64>().ok() else {
        return contains_ignore_case(&cell.value, &expected.to_lowercase());
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

/// Row sample for measuring a column's width, and the character count past
/// which measuring tells us nothing: the width clamp at 3000px takes over at
/// ~400 characters, and rows past the sample cannot widen a column the first
/// rows already filled.
const WIDTH_SAMPLE_ROWS: usize = 100;
const MAX_MEASURED_CHARS: usize = 400;

pub struct ResultsTableDelegate {
    columns: Vec<Column>,
    /// Header name to display column, kept in sync with `columns` so a
    /// by-name lookup costs one hash instead of a scan - building an
    /// UPDATE touched every column by name, and each one paid for the
    /// whole header list.
    column_ix: HashMap<String, usize>,
    /// The shared result handle the grid reads its cells from. The panel
    /// used to deep-copy up to 10 000 rows × every column into a private
    /// `Vec<Vec<ResultCell>>` on each result landing (and again per page
    /// append); the grid is a read-only view of the session's result, so
    /// it holds the same `Arc` and a landing costs one refcount bump.
    /// A copy-on-write through the session handle (load-more, rerun)
    /// mints a new address and the panel pushes it in via `update`, so
    /// the delegate never sees a mutated grid under its feet - the same
    /// snapshot semantics the deep copy gave, at none of the cost.
    source: Option<Arc<SqlResult>>,
    /// Display slot (0-based over the data columns, i.e. `col_ix - 1` in
    /// the grid's coordinates) → index into the row as stored in
    /// `source`. Identity until a column drag reorders it; `move_column`
    /// used to physically reorder every row for this, Θ(rows × cols)
    /// memmove per drag on the private copy. Permuting this one `Vec`
    /// answers exactly the reads the physical reorder produced.
    data_col: Vec<usize>,
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
    /// Cells staged as pending edits (display_row, display_col), tinted like
    /// TablePro's modified-cell highlight while awaiting review.
    modified_cells: HashSet<(usize, usize)>,
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
            column_ix: HashMap::new(),
            source: None,
            data_col: Vec::new(),
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
            modified_cells: HashSet::new(),
        }
    }

    /// The query inside the shared handle; `None` for non-query results and
    /// for the reset state, both of which render an empty grid.
    fn query(&self) -> Option<&QueryResult> {
        match self.source.as_deref() {
            Some(SqlResult::Query(q)) => Some(q),
            _ => None,
        }
    }

    /// The rows of the shared handle, in storage order (see `data_col`
    /// for how display slots map into it).
    fn rows(&self) -> &[Vec<ResultCell>] {
        match self.query() {
            Some(q) => q.rows.as_slice(),
            None => &[],
        }
    }

    /// Storage index of display slot `slot`; slots the permutation does not
    /// cover (out-of-range, or rows longer than the column list) read
    /// themselves, which is what the physical layout did for them.
    fn mapped(&self, slot: usize) -> usize {
        self.data_col.get(slot).copied().unwrap_or(slot)
    }

    pub(super) fn update(&mut self, source: Option<Arc<SqlResult>>) {
        self.source = source;
        self.order = (0..self.rows().len()).collect();
        self.sort_col = None;
        self.sort_asc = true;
        self.modified_cells.clear();
        let mut cols = vec![Column::new("#", "#").width(px(60.0))];
        cols.extend(
            self.query()
                .into_iter()
                .flat_map(|q| q.columns.iter())
                .enumerate()
                .map(|(col_ix, col)| {
                    let header_len = col.name.chars().count().max(1);
                    let cell_len = self
                        .rows()
                        .iter()
                        .take(WIDTH_SAMPLE_ROWS)
                        .filter_map(|row| row.get(col_ix))
                        .map(|cell| cell.value.chars().take(MAX_MEASURED_CHARS).count())
                        .max()
                        .unwrap_or(0);
                    let width = (header_len.max(cell_len) as f32 * 7.5 + 24.0).clamp(80.0, 3000.0);
                    Column::new(&col.name, &col.name)
                        .sortable()
                        .width(px(width))
                }),
        );
        self.columns = cols;
        // A fresh result resets the display order, as replacing the whole
        // physical row copy used to.
        self.data_col = (0..self.query().map_or(0, |q| q.columns.len())).collect();
        self.rebuild_column_index();
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

    /// Tint a cell as modified-pending-review.
    pub(super) fn mark_modified(&mut self, display_row: usize, display_col: usize) {
        self.modified_cells.insert((display_row, display_col));
    }

    /// Drop all modified-cell tints (after apply / discard / reconnect).
    pub(super) fn clear_modified(&mut self) {
        self.modified_cells.clear();
    }

    fn rebuild_filtered_order(&mut self) {
        if self.filter.is_empty() && self.column_filters.is_empty() {
            self.filtered_order = self.order.clone();
            return;
        }
        let query = self.filter.to_lowercase();
        // Prepared once for the whole pass rather than per cell: every cell a
        // row check touches used to rebuild the same `LIKE` pattern.
        let prepared: Vec<PreparedFilter> = self
            .column_filters
            .iter()
            .map(PreparedFilter::prepare)
            .collect();
        self.filtered_order = self
            .order
            .iter()
            .copied()
            .filter(|&row_ix| self.row_matches(row_ix, &query, &prepared))
            .collect();
    }

    fn row_matches(&self, row_ix: usize, query: &str, prepared: &[PreparedFilter]) -> bool {
        let Some(row) = self.rows().get(row_ix) else {
            return false;
        };
        let text_ok = query.is_empty()
            || row
                .iter()
                .any(|cell| contains_ignore_case(&cell.value, query));
        if !text_ok {
            return false;
        }
        // `f.column` indexes the physical display order, as it did when a
        // column drag physically reordered every row; the permutation says
        // which stored cell sits at that position now.
        prepared
            .iter()
            .all(|f| f.matches(row.get(self.mapped(f.column))))
    }

    fn apply_sort(&mut self) {
        let mut order: Vec<usize> = (0..self.rows().len()).collect();
        if let Some(col) = self.sort_col {
            let rows = self.rows();
            let cell_col = self.mapped(col);
            let asc = self.sort_asc;
            order.sort_by(|&a, &b| {
                let ca = rows[a].get(cell_col);
                let cb = rows[b].get(cell_col);
                let ord = match (ca, cb) {
                    (Some(a), Some(b)) => a.sort_cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (None, None) => std::cmp::Ordering::Equal,
                };
                if asc {
                    ord
                } else {
                    ord.reverse()
                }
            });
        }
        self.order = order;
        self.rebuild_filtered_order();
    }

    fn rebuild_column_index(&mut self) {
        let mut index = HashMap::with_capacity(self.columns.len());
        for (ix, col) in self.columns.iter().enumerate() {
            // A self-join can repeat a header; position() found the first,
            // so the first keeps the name.
            index.entry(col.name.to_string()).or_insert(ix);
        }
        self.column_ix = index;
    }

    pub(super) fn cell_named(&self, display_row: usize, name: &str) -> Option<&ResultCell> {
        let display_col = *self.column_ix.get(name)?;
        if display_col == 0 {
            return None;
        }
        let row_ix = *self.filtered_order.get(display_row)?;
        self.rows().get(row_ix)?.get(self.mapped(display_col - 1))
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
        let panel_state = self.panel_handle.borrow().as_ref().map(|panel| {
            let p = panel.read(cx);
            (
                p.editing_input.clone(),
                p.editing_fk_select.clone(),
                p.editing_fk.as_ref().map(|fk| fk.loaded).unwrap_or(false),
            )
        });
        let width = self
            .columns
            .get(col_ix)
            .map(|c| c.width)
            .unwrap_or(px(120.0));
        let panel_handle = self.panel_handle.clone();
        match panel_state {
            Some((input, fk_select, fk_ready)) => {
                let root = div().id("cell-edit-input").h_full().w(width).on_key_down(
                    move |event, _window, cx| {
                        if event.keystroke.key == "escape" {
                            if let Some(panel) = panel_handle.borrow().clone() {
                                panel.update(cx, |panel, cx| panel.cancel_cell_edit(cx));
                            }
                        }
                    },
                );
                // Foreign-key columns get a dropdown of referenced values;
                // everything else keeps the plain text input.
                if fk_ready {
                    root.child(
                        Select::new(&fk_select)
                            .small()
                            .menu_width(px(220.0))
                            .placeholder("Select value…"),
                    )
                    .into_any_element()
                } else {
                    root.child(Input::new(&input).small()).into_any_element()
                }
            }
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
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let selected = *self.selected_row_cell.borrow() == Some(row_ix);
        let striped = row_ix % 2 == 1;
        let cell = self.selected_row_cell.clone();
        div()
            .id(row_ix)
            .cursor_pointer()
            .when(selected, |this| this.bg(gpui::blue().opacity(0.1)))
            .when(!selected && striped, |this| {
                this.bg(cx.theme().muted.opacity(0.35))
            })
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

        if let Some(row) = self.rows().get(row_ix) {
            if let Some(cell) = row.get(self.mapped(col_ix - 1)) {
                let value = if cell.is_null {
                    String::new()
                } else {
                    cell.value.clone()
                };
                let is_null = cell.is_null;
                let is_modified = self.modified_cells.contains(&(display_row, col_ix));
                let selected_cell = self.selected_cell.clone();
                let selected_row_cell = self.selected_row_cell.clone();
                let last_click = self.last_click.clone();
                let panel_handle = self.panel_handle.clone();
                let click_target = div()
                    .id(format!("cell-{}-{}", display_row, col_ix))
                    .cursor_pointer()
                    .when(is_modified, |d| d.bg(gpui::yellow().opacity(0.22)))
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
        self.reorder_column(col_ix, to_ix);
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

impl ResultsTableDelegate {
    /// The column-drag permutation, split from the trait method so it is
    /// testable without a window. Reorders the display columns and the
    /// display-slot permutation together; the stored rows never move. The
    /// old code physically removed/inserted one cell in every row (under
    /// `col_ix < row.len()`); `mapped` reproduces exactly which stored cell
    /// each display slot then showed, slot order for slot order.
    fn reorder_column(&mut self, col_ix: usize, to_ix: usize) {
        let col = self.columns.remove(col_ix);
        let clamped = to_ix.min(self.columns.len());
        self.columns.insert(clamped, col);
        self.rebuild_column_index();

        if col_ix < self.data_col.len() {
            let slot = self.data_col.remove(col_ix);
            let insert_at = clamped.min(self.data_col.len());
            self.data_col.insert(insert_at, slot);
        }
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports, not `use super::*`: this module's own glob already
    // re-exports the panel's whole import surface, and re-globbing it in a
    // child module sends rustc's resolver over its stack limit on Windows.
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use dbstudio_core::result::{QueryResult, ResultCell, SqlResult};
    use dbstudio_core::schema::ColumnInfo;
    use gpui::px;

    use super::{
        operand_preparations, reset_operand_preparations, ColumnFilter, FilterOp, PreparedFilter,
        ResultsTableDelegate, WIDTH_SAMPLE_ROWS,
    };

    fn delegate() -> ResultsTableDelegate {
        ResultsTableDelegate::new(
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
        )
    }

    fn grid(columns: &[&str], rows: Vec<Vec<String>>) -> QueryResult {
        QueryResult {
            columns: columns
                .iter()
                .enumerate()
                .map(|(ix, name)| ColumnInfo::new(*name, "text", ix as u32))
                .collect(),
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(ResultCell::text).collect())
                .collect(),
            row_count: 0,
            total_row_count: 0,
            execution_time_ms: 0,
            sql: String::new(),
            truncated: false,
        }
    }

    fn result(columns: &[&str], rows: Vec<Vec<String>>) -> Option<Arc<SqlResult>> {
        Some(Arc::new(SqlResult::Query(grid(columns, rows))))
    }

    // The name index has to answer exactly what the header scan answered:
    // first of duplicates, nothing for the row-number column, nothing for
    // names the result does not have.
    #[test]
    fn cell_named_resolves_headers_like_the_scan_did() {
        let mut d = delegate();
        d.update(result(
            &["id", "name", "id"],
            vec![vec![
                String::from("1"),
                String::from("a"),
                String::from("9"),
            ]],
        ));
        let found = |d: &ResultsTableDelegate, name: &str, row: usize| {
            d.cell_named(row, name).map(|c| c.value.clone())
        };
        assert_eq!(found(&d, "id", 0).as_deref(), Some("1"));
        assert_eq!(found(&d, "name", 0).as_deref(), Some("a"));
        assert_eq!(found(&d, "#", 0), None);
        assert_eq!(found(&d, "missing", 0), None);
        assert_eq!(found(&d, "name", 1), None);
        for col in d.columns.iter() {
            let first = d.columns.iter().position(|c| c.name == col.name).unwrap();
            assert_eq!(d.column_ix[&col.name.to_string()], first);
        }
    }

    #[test]
    fn width_measurement_stops_at_the_sample_and_the_clamp() {
        let mut rows: Vec<Vec<String>> = (0..WIDTH_SAMPLE_ROWS)
            .map(|_| vec![String::from("short")])
            .collect();
        // The one wide cell sits past the sample, so it must not widen the
        // column any more than a row that was never there.
        rows.push(vec!["x".repeat(5000)]);
        let mut d = delegate();
        d.update(result(&["v"], rows));
        assert_eq!(d.columns[1].width, px(80.0));

        // A wide cell inside the sample still reaches the clamp, and a cell
        // of five thousand characters is measured no harder than one of
        // four hundred.
        let mut d = delegate();
        d.update(result(&["v"], vec![vec!["x".repeat(5000)]]));
        assert_eq!(d.columns[1].width, px(3000.0));
    }

    /// What this round changed, pinned as *call counts*: the two things `>` /
    /// `<` used to redo for every cell from the operand alone - deciding whether
    /// the operand is a number, and folding it for the fallback - are now
    /// decided where a pass decides them. Constant per pass, so a bigger page
    /// must not move the tally. See the counters' doc for why this is not the
    /// allocation measurement re-run.
    #[test]
    fn the_operand_is_prepared_once_per_pass_not_once_per_cell() {
        let cells: Vec<ResultCell> = (0..1000)
            .map(|ix| ResultCell::text(format!("row {ix}")))
            .collect();
        let filter = |op: FilterOp, value: &str| ColumnFilter {
            column: 0,
            op,
            value: value.to_string(),
        };

        // The pass shape of `row_matches`: one prepared filter, then one
        // `matches` per cell. A non-numeric operand pays one judgement plus one
        // fold for the whole pass, and nothing more per cell.
        reset_operand_preparations();
        let loose = PreparedFilter::prepare(&filter(FilterOp::GreaterThan, "2026-01-01"));
        assert_eq!(
            operand_preparations(),
            2,
            "one parse judgement + one fold, once per pass"
        );
        for cell in &cells {
            assert!(!loose.matches(Some(cell)));
        }
        assert_eq!(
            operand_preparations(),
            2,
            "1,000 cells must not add operand work"
        );

        // A numeric operand pays the judgement alone: it reaches `Number`, whose
        // per-cell work is the cell's own proxy, not the operand's.
        reset_operand_preparations();
        let strict = PreparedFilter::prepare(&filter(FilterOp::LessThan, "12"));
        assert_eq!(
            operand_preparations(),
            1,
            "the fold is only paid when the parse fails"
        );
        for cell in &cells {
            assert!(!strict.matches(Some(cell)));
        }
        assert_eq!(operand_preparations(), 1);

        // And through the real per-pass entry: `set_column_filters` prepares the
        // filter list once and then answers every row of the page.
        let rows: Vec<Vec<String>> = (0..500).map(|ix| vec![format!("row {ix}")]).collect();
        let mut d = delegate();
        d.update(result(&["v"], rows));
        reset_operand_preparations();
        d.set_column_filters(&[filter(FilterOp::GreaterThan, "2026-01-01")]);
        assert_eq!(
            operand_preparations(),
            2,
            "a 500-row page is one pass, not 500"
        );
        d.set_column_filters(&[]);
        assert_eq!(operand_preparations(), 2, "clearing prepares nothing");
    }

    /// The share, pinned as a *must-hit*: ingesting a result hands the grid
    /// the session's own buffer - same `Arc`, same rows `Vec` allocation -
    /// instead of copying up to 10 000 rows. If `update` regressed to the old
    /// `result.rows.clone()`, both pointer assertions below go red (that is
    /// the negative control: a deep copy never aliases the source buffer).
    #[test]
    fn ingesting_a_result_shares_the_handle_instead_of_copying_the_grid() {
        let rows: Vec<Vec<String>> = (0..500)
            .map(|r| vec![format!("r{r}c0"), format!("r{r}c1")])
            .collect();
        let arc = Arc::new(SqlResult::Query(grid(&["a", "b"], rows)));
        let mut d = delegate();
        d.update(Some(Arc::clone(&arc)));

        assert!(Arc::ptr_eq(d.source.as_ref().unwrap(), &arc));
        assert_eq!(Arc::strong_count(&arc), 2, "one handle, two owners");
        let SqlResult::Query(q) = arc.as_ref() else {
            unreachable!("the fixture is a Query");
        };
        assert!(std::ptr::eq(d.rows().as_ptr(), q.rows.as_ptr()));
        assert_eq!(d.rows().len(), 500);
    }

    /// The other half of the equivalence claim: sharing must still *read*
    /// like the old deep copy did - a snapshot. Mutating the session's
    /// handle (a load-more append, which copy-on-writes precisely because
    /// the grid still holds the old one) must not change what the grid
    /// shows until the panel pushes the new handle in through `update`.
    #[test]
    fn the_shared_handle_still_reads_as_a_snapshot() {
        let rows: Vec<Vec<String>> = (0..50)
            .map(|r| vec![format!("r{r}c0"), format!("r{r}c1")])
            .collect();
        let mut session = Arc::new(SqlResult::Query(grid(&["a", "b"], rows)));
        let mut d = delegate();
        d.update(Some(Arc::clone(&session)));

        // The append path: `Arc::make_mut` on a shared handle.
        if let SqlResult::Query(q) = Arc::make_mut(&mut session) {
            q.rows
                .push(vec![ResultCell::text("late"), ResultCell::text("later")]);
        }
        assert_eq!(d.rows().len(), 50, "the snapshot was not disturbed");

        d.update(Some(Arc::clone(&session)));
        assert_eq!(d.rows().len(), 51, "re-ingest sees the appended row");
        assert_eq!(d.rows()[50][0].value, "late");
    }

    /// The column drag used to physically remove/insert one cell in every
    /// row, *at the display index* while display slot `d` reads physical
    /// `d-1` - a legacy off-by-one that paired a dragged header with the
    /// neighbour's cells. The permutation must answer, slot for slot, the
    /// same stored cell the physical reorder left on screen, quirk
    /// included - and sort over a dragged slot must order rows by that
    /// same cell.
    #[test]
    fn a_column_drag_permutes_instead_of_rewriting_rows() {
        let rows: Vec<Vec<String>> = (0..5)
            .map(|r| vec![format!("a{r}"), format!("b{r}"), format!("c{r}")])
            .collect();
        let mut d = delegate();
        d.update(result(&["a", "b", "c"], rows));

        // The old call shape: display index 1 (header "a") moved to 2.
        // Old effect: headers became ["b","a","c"], while each physical
        // row went [a,b,c] -> [a,c,b]; display slot 1 now shows physical 0
        // under header "a" at position 2... recomputed slot by slot, the
        // name -> value pairs the old grid displayed were:
        d.reorder_column(1, 2);
        assert_eq!(d.cell_named(3, "a").map(|c| c.value.as_str()), Some("c3"));
        assert_eq!(d.cell_named(3, "b").map(|c| c.value.as_str()), Some("a3"));
        assert_eq!(d.cell_named(3, "c").map(|c| c.value.as_str()), Some("b3"));

        // Sorting display slot 2 (which after the drag shows the stored
        // `b` cells) must order rows by the b-values, as the physically
        // reordered copy did.
        d.sort_col = Some(2);
        d.sort_asc = true;
        d.apply_sort();
        let by_b: Vec<String> = d
            .filtered_order
            .iter()
            .map(|&ix| d.rows()[ix][1].value.clone())
            .collect();
        assert_eq!(by_b, vec!["b0", "b1", "b2", "b3", "b4"]);
    }

    /// Times the copies the run-query path used to make against the shared
    /// ingest that replaced them. `#[ignore]`d: prints numbers, asserts
    /// nothing. Same-process, interleaved rounds, per-path medians.
    #[test]
    #[ignore = "bench: grid-copy timings"]
    fn bench_grid_copy_paths() {
        use std::time::Instant;
        const ROWS: usize = 10_000;
        const COLS: usize = 15;
        let q = QueryResult {
            columns: (0..COLS)
                .map(|ix| ColumnInfo::new(format!("c{ix}"), "text", ix as u32))
                .collect(),
            rows: (0..ROWS)
                .map(|r| {
                    (0..COLS)
                        .map(|c| ResultCell::text(format!("v{r}-{c}")))
                        .collect()
                })
                .collect(),
            row_count: ROWS,
            total_row_count: ROWS,
            execution_time_ms: 0,
            sql: String::new(),
            truncated: false,
        };
        let sqlr = Arc::new(SqlResult::Query(q.clone()));
        let mut t_result_clone = Vec::new();
        let mut t_rows_clone = Vec::new();
        let mut t_update = Vec::new();
        for _ in 0..7 {
            // What the old path cost per fresh query: one SqlResult clone in
            // `spawn_execution`, one QueryResult clone in the panel observer
            // (≈ the `rows.clone` measured here plus headers), one rows clone
            // in `delegate.update`.
            let t = Instant::now();
            let a = SqlResult::clone(&sqlr);
            t_result_clone.push(t.elapsed());
            std::hint::black_box(&a);
            let t = Instant::now();
            let c = q.rows.clone();
            t_rows_clone.push(t.elapsed());
            std::hint::black_box(&c);
            // The new path: hand the same handle to the delegate.
            let mut d = delegate();
            let t = Instant::now();
            d.update(Some(Arc::clone(&sqlr)));
            t_update.push(t.elapsed());
        }
        let med = |mut v: Vec<std::time::Duration>| {
            v.sort();
            v[v.len() / 2]
        };
        let m_result_clone = med(t_result_clone);
        let m_rows_clone = med(t_rows_clone);
        let old = m_result_clone + m_rows_clone * 2;
        let new = med(t_update);
        println!(
            "bench: result.clone={:?} rows.clone={:?} old-per-run(sum)={:?} new.update={:?}",
            m_result_clone, m_rows_clone, old, new,
        );
    }
}
