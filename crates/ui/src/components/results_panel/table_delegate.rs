use super::*;

pub struct ResultsTableDelegate {
    columns: Vec<Column>,
    rows: Vec<Vec<ResultCell>>,
    /// Display order into `rows` (identity until sorted). Sorting reorders this
    /// vec instead of cloning the rows, so the data is never duplicated.
    order: Vec<usize>,
    /// Currently sorted data column index (0-based, excluding the row-number column).
    sort_col: Option<usize>,
    sort_asc: bool,
    visible_rows: Range<usize>,
    selected_row_cell: Rc<RefCell<Option<usize>>>,
}

impl ResultsTableDelegate {
    pub(super) fn new(selected_row_cell: Rc<RefCell<Option<usize>>>) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            order: Vec::new(),
            sort_col: None,
            sort_asc: true,
            visible_rows: Range::default(),
            selected_row_cell,
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
            Column::new(&col.name, &col.name).sortable().width(px(width))
        }));
        self.columns = cols;
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
    }

    /// Cell at the currently displayed (possibly sorted/reordered) `display_row`,
    /// addressed by the result column's original name.
    ///
    /// Selections are tracked in display coordinates, while edit/delete actions
    /// operate on the original `QueryResult`; resolving cells through this
    /// accessor keeps the two index spaces aligned (and survives column moves).
    pub(super) fn cell_named(&self, display_row: usize, name: &str) -> Option<&ResultCell> {
        let display_col = self.columns.iter().position(|c| c.name.as_ref() == name)?;
        if display_col == 0 {
            return None;
        }
        let row_ix = *self.order.get(display_row)?;
        self.rows.get(row_ix)?.get(display_col - 1)
    }
}

impl TableDelegate for ResultsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.order.len()
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
        // Column 0 is the row-number column; never sort it.
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
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        if col_ix == 0 {
            return Label::new((row_ix + 1).to_string())
                .text_sm()
                .text_color(cx.theme().muted_foreground);
        }
        let row_ix = match self.order.get(row_ix) {
            Some(ix) => *ix,
            None => {
                return Label::new("--").text_sm();
            }
        };
        if let Some(row) = self.rows.get(row_ix) {
            if let Some(cell) = row.get(col_ix - 1) {
                if cell.is_null {
                    return Label::new("NULL")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .italic();
                }
                return Label::new(cell.value.clone()).text_sm();
            }
        }
        Label::new("--").text_sm()
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
