use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::schema::ColumnInfo;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const MAX_RESULT_ROWS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SqlResult {
    Query(QueryResult),
    Modified(ExecResult),
    Error(ErrorResult),
}

impl SqlResult {
    /// The first cell of the first row, matching scalar queries such as
    /// `SELECT DB_NAME()`.
    ///
    /// Returns `None` for modified/error results or when no rows came back.
    /// Owned: the grid is shared, so a borrow cannot escape the lock.
    pub fn first_cell(&self) -> Option<ResultCell> {
        match self {
            SqlResult::Query(q) => q.first_cell(),
            _ => None,
        }
    }

    /// The first row of a query result, if any (owned, see `first_cell`).
    pub fn first_row(&self) -> Option<Row> {
        match self {
            SqlResult::Query(q) => q.first_row(),
            _ => None,
        }
    }
}

/// Shared, live grid of a query result.
///
/// The rows and the few result counters that a load-more append mutates live
/// behind one `RwLock`. A page append is an in-place `extend` (Θ(page)), not a
/// copy-on-write deep copy of the whole accumulated grid - `Arc::make_mut` used
/// to pay Θ(total) of cell copies per page (Θ(k²·P) across k pages) because the
/// results panel and the table delegate each held a clone of the `Arc`. Readers
/// (the delegate, the info bar, exports) take a read lock; a writer (the
/// load-more append) takes the write lock briefly.
#[derive(Debug)]
pub struct Grid {
    pub(crate) inner: RwLock<GridInner>,
}

/// The mutable part of a result grid: the rows plus the counters a load-more
/// rewrite. Everything else (`columns`, `sql`) stays immutable on `QueryResult`.
#[derive(Debug)]
pub struct GridInner {
    pub rows: Vec<Vec<ResultCell>>,
    pub row_count: usize,
    pub total_row_count: usize,
    pub execution_time_ms: u128,
    pub truncated: bool,
}

/// What a load-more append records after extending the rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppendMeta {
    pub execution_time_ms: u128,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub sql: String,
    /// The live, shareable grid. Cloning a `QueryResult`/`SqlResult` bumps the
    /// refcount only - the rows are read through the grid, never copied.
    pub grid: Arc<Grid>,
}

impl Grid {
    pub fn new(inner: GridInner) -> Self {
        Self {
            inner: RwLock::new(inner),
        }
    }
    /// A read lock over the grid's mutable part.
    pub fn read(&self) -> RwLockReadGuard<'_, GridInner> {
        self.inner.read().unwrap()
    }
    /// A write lock over the grid's mutable part (the load-more append path).
    pub fn write(&self) -> RwLockWriteGuard<'_, GridInner> {
        self.inner.write().unwrap()
    }
    /// Live row count.
    pub fn len(&self) -> usize {
        self.read().rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.read().rows.is_empty()
    }
}

impl Clone for QueryResult {
    fn clone(&self) -> Self {
        Self {
            columns: self.columns.clone(),
            sql: self.sql.clone(),
            grid: Arc::clone(&self.grid),
        }
    }
}

// Wire compatibility: the plugin C-ABI is the only serde boundary, and its
// shape is the flat 7-field object `{columns, rows, row_count, total_row_count,
// execution_time_ms, sql, truncated}` where `rows` is a 2-D array of the 4-field
// cells. `Serialize`/`Deserialize` are written by hand so that `grid`
// transparently presents exactly that shape: `rows`/counters/truncated are read
// out of the shared grid on the way out, and a deserialized grid is rebuilt from
// them. `ResultCell`/`CellType`/`ColumnInfo` keep their derived
// (de)serialization, so the rest of the wire - including `sort_value: null`,
// `kind: "text"` and the `#[serde(default)]` tolerance for missing
// `kind`/`sort_value` - is unchanged.
impl Serialize for QueryResult {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Serialize through a same-field, same-order proxy so the output is
        // byte-identical to the old derived shape while the mutable fields come
        // out of the live grid.
        #[derive(Serialize)]
        struct Wire<'a> {
            columns: &'a [ColumnInfo],
            rows: &'a [Vec<ResultCell>],
            row_count: &'a usize,
            total_row_count: &'a usize,
            execution_time_ms: &'a u128,
            sql: &'a str,
            truncated: &'a bool,
        }
        let g = self.grid.read();
        Wire {
            columns: &self.columns,
            rows: &g.rows,
            row_count: &g.row_count,
            total_row_count: &g.total_row_count,
            execution_time_ms: &g.execution_time_ms,
            sql: &self.sql,
            truncated: &g.truncated,
        }
        .serialize(ser)
    }
}

#[derive(Deserialize)]
struct QueryResultWire {
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<ResultCell>>,
    row_count: usize,
    total_row_count: usize,
    execution_time_ms: u128,
    sql: String,
    truncated: bool,
}

impl<'de> Deserialize<'de> for QueryResult {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = QueryResultWire::deserialize(d)?;
        Ok(QueryResult::new(
            w.columns,
            w.rows,
            w.row_count,
            w.total_row_count,
            w.execution_time_ms,
            w.sql,
            w.truncated,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub rows_affected: u64,
    pub execution_time_ms: u128,
    pub sql: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResult {
    pub message: String,
    pub sql: String,
    pub execution_time_ms: u128,
}

/// The logical kind of a result cell, as reported by the driver.
///
/// Used to drive sorting and (in the future) type-aware rendering and cell
/// editing. Misses a specific kind fall back to [`CellType::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CellType {
    #[default]
    Text,
    Integer,
    Float,
    Boolean,
    Decimal,
    Date,
    Time,
    DateTime,
    Binary,
    Json,
    Uuid,
    Blob,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultCell {
    pub value: String,
    pub is_null: bool,
    /// The logical type of the cell, as reported by the driver.
    #[serde(default)]
    pub kind: CellType,
    /// A numeric proxy for the value, when one exists (numbers and dates).
    ///
    /// Used for stable type-aware sorting; `None` puts the cell in the text
    /// group, which compares lexically. Never stored for `NULL` cells.
    #[serde(default)]
    pub sort_value: Option<f64>,
}

impl ResultCell {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            is_null: false,
            kind: CellType::Text,
            sort_value: None,
        }
    }

    pub fn integer(value: i64) -> Self {
        Self {
            value: value.to_string(),
            is_null: false,
            kind: CellType::Integer,
            sort_value: Some(value as f64),
        }
    }

    pub fn float(value: f64) -> Self {
        Self {
            value: value.to_string(),
            is_null: false,
            kind: CellType::Float,
            sort_value: Some(value),
        }
    }

    pub fn boolean(value: bool) -> Self {
        Self {
            value: value.to_string(),
            is_null: false,
            kind: CellType::Boolean,
            sort_value: Some(if value { 1.0 } else { 0.0 }),
        }
    }

    /// A numeric cell with an approximate `f64` sort proxy (e.g. `DECIMAL`).
    pub fn decimal(value: impl Into<String>, sort_proxy: f64) -> Self {
        Self {
            value: value.into(),
            is_null: false,
            kind: CellType::Decimal,
            sort_value: Some(sort_proxy),
        }
    }

    /// A non-numeric typed cell: dates, times, blobs, JSON, UUIDs.
    pub fn typed(kind: CellType, value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            is_null: false,
            kind,
            sort_value: None,
        }
    }

    pub fn null() -> Self {
        Self {
            value: String::new(),
            is_null: true,
            kind: CellType::Other,
            sort_value: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.is_null || self.value.is_empty()
    }

    pub fn value_if_not_null(&self) -> Option<String> {
        if self.is_null {
            None
        } else {
            Some(self.value.clone())
        }
    }

    /// Whether the cell carries a numeric orderable proxy.
    pub fn is_ordered(&self) -> bool {
        self.sort_value.is_some()
    }

    /// A stable comparison across [`ResultCell`]s for sorting.
    ///
    /// Total order, in this precedence: non-`NULL` before `NULL`; among
    /// non-`NULL` cells, one backed by a numeric proxy sorts before one that
    /// is not, and cells within each group compare by that group's own order
    /// (`f64` for the proxy group, lexically for the other).
    ///
    /// Keeping the groups disjoint is the point: comparing a proxy-backed
    /// cell against a text-only cell through only their display strings let
    /// `2` < `"10"` (numeric) and `"10"` < `"2"` (lexical) hold at once, so
    /// the relation was not transitive and a sort over it had no defined
    /// result. `f64::total_cmp` also gives `NaN` a definite position instead
    /// of folding it into `Equal` with every other value.
    pub fn sort_cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.is_null, other.is_null) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => match (self.sort_value, other.sort_value) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => self.value.cmp(&other.value),
            },
        }
    }
}

impl From<&str> for ResultCell {
    fn from(s: &str) -> Self {
        Self::text(s)
    }
}

impl From<String> for ResultCell {
    fn from(s: String) -> Self {
        Self::text(s)
    }
}

/// Type alias for a row of result cells.
pub type Row = Vec<ResultCell>;

/// Extension trait for convenient row access patterns.
pub trait RowExt {
    /// Get the string value of a cell by index, or empty string if missing/null.
    fn cell_str(&self, index: usize) -> String;
    /// Get the first cell's string value, or empty string.
    fn first_str(&self) -> String;
    /// Borrowed value of a cell, or `""` when the index is missing or the cell is NULL.
    fn cell(&self, index: usize) -> &str;
    /// `true` when the cell holds a non-NULL value equal to `expected`.
    ///
    /// Missing indices and NULL cells never match, not even against `""`; use
    /// [`RowExt::cell_str`] when NULL should read as an empty string.
    fn cell_is(&self, index: usize, expected: &str) -> bool;
    /// Cell value parsed as `u32`, falling back to `default`.
    fn cell_u32(&self, index: usize, default: u32) -> u32;
}

impl RowExt for Row {
    fn cell_str(&self, index: usize) -> String {
        self.cell(index).to_string()
    }

    fn first_str(&self) -> String {
        self.cell_str(0)
    }

    fn cell(&self, index: usize) -> &str {
        self.get(index)
            .filter(|c| !c.is_null)
            .map(|c| c.value.as_str())
            .unwrap_or_default()
    }

    fn cell_is(&self, index: usize, expected: &str) -> bool {
        self.get(index)
            .map(|c| !c.is_null && c.value == expected)
            .unwrap_or(false)
    }

    fn cell_u32(&self, index: usize, default: u32) -> u32 {
        self.cell(index).parse().unwrap_or(default)
    }
}

impl QueryResult {
    /// Build a result wrapping `rows` in a fresh shared grid. This is the
    /// constructor every driver/export path uses in place of the old struct
    /// literal; it is where the (previously field-level) counters move into the
    /// grid.
    pub fn new(
        columns: Vec<ColumnInfo>,
        rows: Vec<Vec<ResultCell>>,
        row_count: usize,
        total_row_count: usize,
        execution_time_ms: u128,
        sql: String,
        truncated: bool,
    ) -> Self {
        Self {
            grid: Arc::new(Grid::new(GridInner {
                rows,
                row_count,
                total_row_count,
                execution_time_ms,
                truncated,
            })),
            columns,
            sql,
        }
    }

    /// The first row, owned (the grid is shared, so no borrow escapes the lock).
    pub fn first_row(&self) -> Option<Row> {
        self.grid.read().rows.first().cloned()
    }

    /// The first cell of the first row, owned.
    pub fn first_cell(&self) -> Option<ResultCell> {
        self.grid
            .read()
            .rows
            .first()
            .and_then(|r| r.first())
            .cloned()
    }

    pub fn empty(sql: impl Into<String>) -> Self {
        Self::new(Vec::new(), Vec::new(), 0, 0, 0, sql.into(), false)
    }

    /// The shared live grid handle.
    pub fn grid(&self) -> &Arc<Grid> {
        &self.grid
    }

    /// Live row count of the grid (reads the lock; cheap when uncontended).
    pub fn row_len(&self) -> usize {
        self.grid.len()
    }

    pub fn row_count(&self) -> usize {
        self.grid.read().row_count
    }

    pub fn total_row_count(&self) -> usize {
        self.grid.read().total_row_count
    }

    pub fn truncated(&self) -> bool {
        self.grid.read().truncated
    }

    pub fn execution_time_ms(&self) -> u128 {
        self.grid.read().execution_time_ms
    }

    /// A snapshot of the current rows under one read-lock acquisition - what a
    /// background export thread iterates so it sees a stable table.
    pub fn rows_snapshot(&self) -> Vec<Vec<ResultCell>> {
        self.grid.read().rows.clone()
    }

    /// Append a page in place: Θ(page), no copy of the accumulated rows. This is
    /// the load-more path that used to be an `Arc::make_mut` COW deep copy.
    pub fn append_rows(&self, page: Vec<Vec<ResultCell>>, meta: AppendMeta) -> usize {
        let added = page.len();
        let mut g = self.grid.write();
        g.rows.extend(page);
        g.row_count = g.rows.len();
        g.total_row_count = if meta.truncated {
            g.rows.len().saturating_add(1)
        } else {
            g.rows.len()
        };
        g.truncated = meta.truncated;
        g.execution_time_ms = meta.execution_time_ms;
        added
    }

    /// Driver-side cap, run before the grid is shared: rewrite the counters and
    /// drop the overflow. (Now `&self` - it goes through the grid's write lock,
    /// which at build time is uncontended since the grid is the sole owner.)
    pub fn truncate_rows(&self, max_rows: usize) {
        let mut g = self.grid.write();
        g.total_row_count = g.rows.len();
        if g.rows.len() > max_rows {
            g.rows.truncate(max_rows);
            g.row_count = max_rows;
            g.truncated = true;
        } else {
            g.row_count = g.rows.len();
            g.truncated = false;
        }
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
