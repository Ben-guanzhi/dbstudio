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
    /// Used for stable type-aware sorting. `None` falls back to a text
    /// comparison. Never stored for `NULL` cells.
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
        if self.is_null { None } else { Some(self.value.clone()) }
    }

    /// Whether the cell carries a numeric orderable proxy.
    pub fn is_ordered(&self) -> bool {
        self.sort_value.is_some()
    }

    /// A stable comparison across [`ResultCell`]s for sorting.
    ///
    /// `NULL` always sorts last; ordered cells compare by their numeric proxy,
    /// everything else falls back to a plain string comparison.
    pub fn sort_cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.is_null, other.is_null) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => match (self.sort_value, other.sort_value) {
                (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
                _ => self.value.cmp(&other.value),
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
mod tests {
    use super::*;

    fn row() -> Row {
        vec![
            ResultCell::text("alice"),
            ResultCell::null(),
            ResultCell::text("42"),
        ]
    }

    #[test]
    fn row_ext_accessors_handle_missing_and_null_cells() {
        let row = row();
        assert_eq!(row.first_str(), "alice");
        assert_eq!(row.cell_str(2), "42");
        // NULL and out-of-range indices both read as empty
        assert_eq!(row.cell(1), "");
        assert_eq!(row.cell(99), "");
        assert_eq!(row.cell_str(99), "");
        assert_eq!(row.first_str(), "alice");
    }

    #[test]
    fn null_payload_is_ignored_by_row_accessors() {
        let row = vec![
            ResultCell { value: "42".to_string(), is_null: true, kind: CellType::Other, sort_value: None },
            ResultCell::text(""),
            ResultCell::text("42"),
        ];
        assert_eq!(row.cell(0), "");
        assert_eq!(row.cell_str(0), "");
        assert_eq!(row.first_str(), "");
        assert_eq!(row.cell_u32(0, 7), 7);
        assert!(!row.cell_is(0, "42"));
        assert!(!row.cell_is(0, ""));
        assert!(row.cell_is(1, ""));
        assert_eq!(row.cell_u32(2, 7), 42);
    }

    #[test]
    fn row_ext_predicates_and_number_parsing() {
        let row = row();
        assert!(row.cell_is(0, "alice"));
        assert!(!row.cell_is(0, "bob"));
        // NULL never matches, not even the empty string
        assert!(!row.cell_is(1, ""));
        assert!(!row.cell_is(1, "alice"));
        assert!(!row.cell_is(99, ""));
        assert_eq!(row.cell_u32(2, 7), 42);
        assert_eq!(row.cell_u32(0, 7), 7);
        assert_eq!(row.cell_u32(99, 7), 7);
    }

    #[test]
    fn value_if_not_null_filters_nulls() {
        assert_eq!(ResultCell::text("x").value_if_not_null().as_deref(), Some("x"));
        assert_eq!(ResultCell::null().value_if_not_null(), None);
    }

    #[test]
    fn first_cell_and_first_row_cover_all_result_shapes() {
        let query = SqlResult::Query(QueryResult {
            columns: Vec::new(),
            rows: vec![vec![ResultCell::text("db_name"), ResultCell::text("extra")]],
            row_count: 1,
            total_row_count: 1,
            execution_time_ms: 1,
            sql: "SELECT DB_NAME()".to_string(),
            truncated: false,
        });
        assert_eq!(query.first_cell().map(|c| c.value.as_str()), Some("db_name"));
        assert_eq!(query.first_row().map(|r| r.cell_str(1)), Some("extra".to_string()));

        let empty = SqlResult::Query(QueryResult::empty("SELECT 1"));
        assert!(empty.first_cell().is_none());
        assert!(empty.first_row().is_none());

        let modified = SqlResult::Modified(ExecResult {
            rows_affected: 1,
            execution_time_ms: 1,
            sql: "DELETE FROM t".to_string(),
            message: "1 rows affected".to_string(),
        });
        assert!(modified.first_cell().is_none());
        assert!(modified.first_row().is_none());
    }

    #[test]
    fn sort_cmp_orders_by_type_and_null_last() {
        use std::cmp::Ordering;
        assert_eq!(ResultCell::integer(2).sort_cmp(&ResultCell::integer(10)), Ordering::Less);
        assert_eq!(ResultCell::float(2.5).sort_cmp(&ResultCell::float(1.5)), Ordering::Greater);
        assert_eq!(ResultCell::integer(2).sort_cmp(&ResultCell::float(2.0)), Ordering::Equal);
        assert_eq!(ResultCell::text("a").sort_cmp(&ResultCell::text("b")), Ordering::Less);
        assert_eq!(ResultCell::null().sort_cmp(&ResultCell::integer(1)), Ordering::Greater);
        assert_eq!(ResultCell::integer(1).sort_cmp(&ResultCell::null()), Ordering::Less);
        assert_eq!(ResultCell::null().sort_cmp(&ResultCell::null()), Ordering::Equal);
    }

    #[test]
    fn truncate_rows_reports_totals() {
        let mut query = QueryResult {
            columns: Vec::new(),
            rows: vec![vec![ResultCell::text("a")]; 5],
            row_count: 0,
            total_row_count: 0,
            execution_time_ms: 1,
            sql: "SELECT a".to_string(),
            truncated: false,
        };

        query.truncate_rows(2);
        assert_eq!(query.rows.len(), 2);
        assert_eq!(query.row_count, 2);
        assert_eq!(query.total_row_count, 5);
        assert!(query.truncated);

        query.truncate_rows(10);
        assert_eq!(query.row_count, 2);
        assert_eq!(query.total_row_count, 2);
        assert!(!query.truncated);
    }
}
