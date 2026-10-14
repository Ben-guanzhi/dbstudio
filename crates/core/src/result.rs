use crate::schema::ColumnInfo;
use serde::{Deserialize, Serialize};

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
    pub fn first_cell(&self) -> Option<&ResultCell> {
        match self {
            SqlResult::Query(q) => q.first_cell(),
            _ => None,
        }
    }

    /// The first row of a query result, if any.
    pub fn first_row(&self) -> Option<&Row> {
        match self {
            SqlResult::Query(q) => q.first_row(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<ResultCell>>,
    pub row_count: usize,
    pub total_row_count: usize,
    pub execution_time_ms: u128,
    pub sql: String,
    pub truncated: bool,
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
    /// The first row, if any rows came back.
    pub fn first_row(&self) -> Option<&Row> {
        self.rows.first()
    }

    /// The first cell of the first row, if any rows came back.
    pub fn first_cell(&self) -> Option<&ResultCell> {
        self.rows.first().and_then(|r| r.first())
    }

    pub fn empty(sql: impl Into<String>) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            total_row_count: 0,
            execution_time_ms: 0,
            sql: sql.into(),
            truncated: false,
        }
    }

    pub fn truncate_rows(&mut self, max_rows: usize) {
        self.total_row_count = self.rows.len();
        if self.rows.len() > max_rows {
            self.rows.truncate(max_rows);
            self.row_count = max_rows;
            self.truncated = true;
        } else {
            self.row_count = self.rows.len();
            self.truncated = false;
        }
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
