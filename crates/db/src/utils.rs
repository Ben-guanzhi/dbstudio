use dbstudio_core::models::ConnectionConfig;
use dbstudio_core::result::{QueryResult, ResultCell, SqlResult, MAX_RESULT_ROWS};
use dbstudio_core::schema::ColumnInfo;
use sqlx::{Column, TypeInfo};

/// Extract rows from a SqlResult and map them to a target type.
///
/// Returns `Ok(vec)` on success, or `Ok(vec![])` if the result is not a query.
pub fn map_query_result<T>(result: SqlResult, mapper: impl FnMut(Vec<ResultCell>) -> T) -> Vec<T> {
    match result {
        SqlResult::Query(q) => q.rows.into_iter().map(mapper).collect(),
        _ => vec![],
    }
}

/// Extract rows from a SqlResult, applying a fallible mapper.
///
/// Returns `Ok(vec)` on success, or `Ok(vec![])` if the result is not a query.
pub fn try_map_query_result<T>(
    result: SqlResult,
    mapper: impl FnMut(Vec<ResultCell>) -> anyhow::Result<T>,
) -> anyhow::Result<Vec<T>> {
    match result {
        SqlResult::Query(q) => q.rows.into_iter().map(mapper).collect(),
        _ => Ok(vec![]),
    }
}

/// One column's catalog facts in the shape every engine's listing query can
/// produce. This is the uniform row layer the metadata operations are (and
/// future op x type tables would be) built on: engines map their result cells
/// onto it, and [`column_info`] performs the single engine-agnostic assembly.
pub struct RawColumn {
    pub name: String,
    pub data_type: String,
    pub ordinal: u32,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
}

/// Assemble the engine-agnostic `ColumnInfo` from a driver's raw facts.
///
/// Kept in exactly one place: a new `ColumnInfo` field is now a change here
/// plus the per-engine cell extraction, not five hand-copied struct literals.
pub fn column_info(raw: RawColumn, table: &str) -> ColumnInfo {
    ColumnInfo {
        table_name: Some(table.to_string()),
        comment: None,
        name: raw.name,
        data_type: raw.data_type,
        ordinal: raw.ordinal,
        nullable: raw.nullable,
        is_primary_key: raw.is_primary_key,
        default_value: raw.default_value,
    }
}

/// Quote a SQL identifier with backticks (MySQL, SQLite).
pub fn quote_backtick(value: &str) -> String {
    dbstudio_core::models::QuoteStyle::Backtick.quote(value)
}

/// Quote a SQL identifier with double quotes (Postgres, Oracle, standard SQL).
pub fn quote_double_quote(value: &str) -> String {
    dbstudio_core::models::QuoteStyle::DoubleQuote.quote(value)
}

/// Quote a SQL identifier with brackets (MSSQL).
pub fn quote_bracket(value: &str) -> String {
    dbstudio_core::models::QuoteStyle::Bracket.quote(value)
}

/// Escape a value for use as a SQL string literal (single quotes).
///
/// This is the counterpart of the identifier helpers above: value comparisons
/// such as `WHERE name = 'people'` need a *literal*, while `name = `people``,
/// `name = "people"` and `name = [people]` are parsed as identifiers by the
/// respective engines and therefore fail or match nothing.
pub fn quote_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// URL-encode a string for use in database connection URLs.
pub fn urlencode_credentials(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
}

/// Encode bytes as a hexadecimal string.
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

/// Build a `k=v&k=v` query-string from a connection's `extra_params` JSON
/// object for drivers that compose connection URLs.
///
/// Non-string primitives (numbers, booleans) are stringified; anything else is
/// skipped. An empty or invalid payload yields the empty string, so callers can
/// append `?`/`&` unconditionally.
pub fn extra_params_query(config: &ConnectionConfig) -> String {
    let Some(raw) = config.extra_params.as_deref() else {
        return String::new();
    };
    let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw) else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in map {
        let value = match v {
            serde_json::Value::String(s) => s,
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => continue,
        };
        parts.push(format!(
            "{}={}",
            urlencode_credentials(&k),
            urlencode_credentials(&value)
        ));
    }
    parts.join("&")
}

/// Build the SSL query-string segment for drivers that compose connection URLs,
/// based on the connection's first-class `ssl_mode` field.
///
/// Returns the empty string when encryption is disabled or the engine has no
/// URL-level SSL knob. The values use each driver's expected spelling:
/// MySQL wants `ssl-mode=required|verify_ca|verify_identity`, PostgreSQL wants
/// `sslmode=require|verify-ca|verify-full`. If the user already pinned the same
/// key via `extra_params`, the explicit field loses (explicit JSON wins).
pub fn ssl_mode_query(config: &ConnectionConfig) -> String {
    if !config.ssl_mode.is_encrypted() {
        return String::new();
    }
    let Some((key, value)) = config.db_type.ssl_mode_param(config.ssl_mode) else {
        return String::new();
    };

    // If the user explicitly configured the same key through extra_params,
    // respect their choice instead of appending a conflicting duplicate.
    if config.extra_params_map().contains_key(key) {
        return String::new();
    }
    format!("{key}={value}")
}

/// Check if a SQL query is a SELECT statement (or other read-only statement).
///
/// SQL comments before the leading keyword are stripped first, so a `-- ...`
/// or `/* ... */` preamble does not push the real keyword out of the prefix.
pub fn is_select_query(sql: &str, extra_keywords: &[&str]) -> bool {
    let cleaned: String = {
        let mut chars = sql.chars().peekable();
        let mut out = String::with_capacity(sql.len());
        while let Some(c) = chars.next() {
            match c {
                '-' if chars.peek() == Some(&'-') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    let mut prev = None;
                    for c in chars.by_ref() {
                        if prev == Some('*') && c == '/' {
                            break;
                        }
                        prev = Some(c);
                    }
                }
                _ => out.push(c),
            }
        }
        out
    };

    let upper = cleaned.trim().to_uppercase();
    upper.starts_with("SELECT")
        || upper.starts_with("WITH")
        || extra_keywords.iter().any(|kw| upper.starts_with(kw))
}

/// Build a QueryResult from sqlx rows using a cell extraction function.
///
/// This is the shared implementation used by MySQL and Postgres drivers.
pub fn build_query_result_from_rows<R>(
    rows: Vec<R>,
    sql: String,
    elapsed_ms: u128,
    extract_cell: impl Fn(&R, usize) -> ResultCell,
) -> SqlResult
where
    R: sqlx::Row,
    <R::Database as sqlx::Database>::Column: sqlx::Column,
{
    if rows.is_empty() {
        return SqlResult::Query(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            total_row_count: 0,
            execution_time_ms: elapsed_ms,
            sql,
            truncated: false,
        });
    }

    let columns: Vec<ColumnInfo> = rows[0]
        .columns()
        .iter()
        .enumerate()
        .map(|(i, col)| {
            ColumnInfo::new(
                col.name().to_string(),
                col.type_info().name().to_string(),
                i as u32,
            )
        })
        .collect();

    let num_cols = columns.len();
    let mut result_rows: Vec<Vec<ResultCell>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut cells = Vec::with_capacity(num_cols);
        for i in 0..num_cols {
            cells.push(extract_cell(row, i));
        }
        result_rows.push(cells);
    }

    finalize_query_result(columns, result_rows, sql, elapsed_ms)
}

/// Build a QueryResult from pre-extracted columns and rows, applying truncation.
///
/// Used by all five drivers to avoid duplicating the construction + truncation boilerplate.
pub fn finalize_query_result(
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<ResultCell>>,
    sql: String,
    elapsed_ms: u128,
) -> SqlResult {
    finalize_query_result_capped(columns, rows, sql, elapsed_ms, Some(MAX_RESULT_ROWS))
}

/// Same as [`finalize_query_result`], but with the row budget supplied by the
/// caller. `None` means "keep every row already fetched" — used by export,
/// which must not inherit the interactive display cap.
pub fn finalize_query_result_capped(
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<ResultCell>>,
    sql: String,
    elapsed_ms: u128,
    max_rows: Option<usize>,
) -> SqlResult {
    let mut result = QueryResult {
        columns,
        rows,
        row_count: 0,
        total_row_count: 0,
        execution_time_ms: elapsed_ms,
        sql,
        truncated: false,
    };
    match max_rows {
        Some(cap) => result.truncate_rows(cap),
        None => {
            result.row_count = result.rows.len();
            result.total_row_count = result.rows.len();
        }
    }
    SqlResult::Query(result)
}

/// Create a null ResultCell.
pub fn null_cell() -> ResultCell {
    ResultCell::null()
}

/// Create a text ResultCell.
pub fn text_cell(value: impl Into<String>) -> ResultCell {
    ResultCell::text(value)
}
