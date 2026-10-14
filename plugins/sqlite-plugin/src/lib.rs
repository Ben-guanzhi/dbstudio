//! Reference dbstudio driver plugin: a `rusqlite` engine exported over the C
//! ABI defined in `dbstudio-plugin::abi`.
//!
//! Everything the host needs crosses the boundary as NUL-terminated UTF-8 plus
//! a `u32` status word. No Rust type appears in a slot signature, which is what
//! makes the ABI survive a host/plugin toolchain split.
use std::collections::HashMap;
use std::ffi::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock, Mutex,
};
use std::time::Instant;

use dbstudio_core::models::ConnectionConfig;
use dbstudio_core::result::{
    CellType, ExecResult, QueryResult, ResultCell, SqlResult, MAX_RESULT_ROWS,
};
use dbstudio_core::schema::{
    ColumnInfo, DatabaseInfo, ForeignKeyInfo, IndexInfo, TableInfo, TableType,
};
use dbstudio_plugin::abi::{
    fail, free_foreign, read_foreign, ship_json, ship_string, PluginString, PluginTable,
};
use dbstudio_plugin::{PluginInfo, PLUGIN_ABI_VERSION};

/// An open database and the file it came from.
struct Connection {
    db: rusqlite::Connection,
    database: String,
}

/// Every connection this process has opened, keyed by the handle token handed
/// back to the host. A `Mutex` rather than an `RwLock`: queries are short and
/// serialising them is the honest thing to do for a single-file engine.
fn connections() -> &'static LazyLock<Mutex<HashMap<String, Connection>>> {
    static CONNECTIONS: LazyLock<Mutex<HashMap<String, Connection>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &CONNECTIONS
}

/// Lock, recovering the guard if a previous holder panicked. The mutex never
/// needs to be poisoned for the host's sake: a dropped connection is just an
/// unknown handle on the next call.
fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Connection>> {
    connections()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn new_handle() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("sqlite-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The statements `rusqlite` will run as a query with a result set. Everything
/// else goes through `execute`, which reports rows affected.
fn is_query(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(
        head.as_str(),
        "SELECT" | "PRAGMA" | "EXPLAIN" | "WITH" | "VALUES"
    )
}

/// Quote an identifier the way SQLite expects.
fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn cell_str(row: &[ResultCell], index: usize) -> String {
    row.get(index)
        .filter(|c| !c.is_null)
        .map(|c| c.value.clone())
        .unwrap_or_default()
}

fn cell_is(row: &[ResultCell], index: usize, expected: &str) -> bool {
    row.get(index)
        .map(|c| !c.is_null && c.value == expected)
        .unwrap_or(false)
}

fn row_to_cells(row: &rusqlite::Row, count: usize) -> Vec<ResultCell> {
    use rusqlite::types::ValueRef;
    let mut cells = Vec::with_capacity(count);
    for i in 0..count {
        let cell = match row.get_ref(i) {
            Ok(ValueRef::Null) => ResultCell::null(),
            Ok(ValueRef::Integer(n)) => ResultCell::integer(n),
            Ok(ValueRef::Real(f)) => ResultCell::float(f),
            Ok(ValueRef::Text(bytes)) => {
                ResultCell::text(String::from_utf8_lossy(bytes).into_owned())
            }
            Ok(ValueRef::Blob(bytes)) => {
                ResultCell::typed(CellType::Blob, format!("blob[{} bytes]", bytes.len()))
            }
            Err(_) => ResultCell::text("[error]"),
        };
        cells.push(cell);
    }
    cells
}

// ── Operations ───────────────────────────────────────────────────────────────

fn connect_inner(config_json: &str, password: &str) -> anyhow::Result<String> {
    // SQLite has no password, so the parameter is deliberately unread rather
    // than reported as an error: a host sharing one connect path across engines
    // sends it for all of them.
    let _ = password;
    let config: ConnectionConfig = serde_json::from_str(config_json)?;
    let db = rusqlite::Connection::open_with_flags(
        &config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;

    let connection = Connection {
        db,
        database: config.database.clone(),
    };
    let token = new_handle();
    lock().insert(token.clone(), connection);
    Ok(token)
}

fn execute_inner(handle: &str, sql: &str) -> anyhow::Result<SqlResult> {
    let start = Instant::now();
    let guard = lock();
    let conn = guard
        .get(handle)
        .ok_or_else(|| anyhow::anyhow!("unknown connection handle"))?;

    if is_query(sql) {
        let mut stmt = conn.db.prepare(sql)?;
        let columns: Vec<ColumnInfo> = stmt
            .column_names()
            .iter()
            .enumerate()
            .map(|(i, name)| ColumnInfo::new(*name, "TEXT", i as u32))
            .collect();

        let mut rows_iter = stmt.query(rusqlite::params![])?;
        let mut rows = Vec::new();
        while let Some(row) = rows_iter.next()? {
            rows.push(row_to_cells(row, columns.len()));
            // rusqlite streams rows lazily, so stopping here bounds memory while
            // still letting `truncate_rows` flag that more were available.
            if rows.len() > MAX_RESULT_ROWS {
                break;
            }
        }

        let mut result = QueryResult {
            columns,
            rows,
            row_count: 0,
            total_row_count: 0,
            execution_time_ms: start.elapsed().as_millis(),
            sql: sql.to_string(),
            truncated: false,
        };
        result.truncate_rows(MAX_RESULT_ROWS);
        Ok(SqlResult::Query(result))
    } else {
        let affected = conn.db.execute(sql, rusqlite::params![])?;
        Ok(SqlResult::Modified(ExecResult {
            rows_affected: affected as u64,
            execution_time_ms: start.elapsed().as_millis(),
            sql: sql.to_string(),
            message: format!("{} rows affected", affected),
        }))
    }
}

fn list_databases_inner() -> anyhow::Result<Vec<DatabaseInfo>> {
    // A file engine has exactly one database, which is already reported by
    // `current_database`; an empty list keeps the UI from showing a bogus
    // switchable entry.
    Ok(Vec::new())
}

fn list_tables_inner(handle: &str, _schema: &str) -> anyhow::Result<Vec<TableInfo>> {
    let result = execute_inner(
        handle,
        "SELECT name, type FROM sqlite_master
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    match result {
        SqlResult::Query(q) => Ok(q
            .rows
            .into_iter()
            .map(|row| TableInfo {
                name: cell_str(&row, 0),
                schema: None,
                database: None,
                table_type: if row.get(1).map(|c| c.value.as_str()) == Some("view") {
                    TableType::View
                } else {
                    TableType::Table
                },
                row_count: None,
                comment: None,
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn list_columns_inner(handle: &str, table: &str, _schema: &str) -> anyhow::Result<Vec<ColumnInfo>> {
    let result = execute_inner(
        handle,
        &format!("PRAGMA table_info({})", quote_ident(table)),
    )?;
    match result {
        SqlResult::Query(q) => Ok(q
            .rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| ColumnInfo {
                name: cell_str(&row, 1),
                data_type: cell_str(&row, 2),
                ordinal: i as u32,
                nullable: !cell_is(&row, 3, "1"),
                is_primary_key: cell_is(&row, 5, "1"),
                default_value: row.get(4).and_then(|c| c.value_if_not_null()),
                comment: None,
                table_name: Some(table.to_string()),
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn list_indexes_inner(handle: &str, table: &str, _schema: &str) -> anyhow::Result<Vec<IndexInfo>> {
    let result = execute_inner(
        handle,
        &format!("PRAGMA index_list({})", quote_ident(table)),
    )?;
    let mut indexes = Vec::new();
    if let SqlResult::Query(q) = result {
        for row in q.rows {
            let name = cell_str(&row, 0);
            if name.is_empty() {
                continue;
            }
            // sqlite_autoindex_* entries are the primary key surfaced twice;
            // the built-in driver keeps them out of the index list too.
            if name.starts_with("sqlite_autoindex_") {
                continue;
            }
            let columns = execute_inner(
                handle,
                &format!("PRAGMA index_info({})", quote_ident(&name)),
            )
            .map(|r| match r {
                SqlResult::Query(q) => q.rows.into_iter().map(|c| cell_str(&c, 2)).collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();

            indexes.push(IndexInfo {
                name,
                table_name: table.to_string(),
                columns,
                is_unique: cell_is(&row, 1, "1"),
                is_primary: false,
                index_type: None,
            });
        }
    }
    Ok(indexes)
}

fn list_foreign_keys_inner(
    handle: &str,
    table: &str,
    _schema: &str,
) -> anyhow::Result<Vec<ForeignKeyInfo>> {
    let result = execute_inner(
        handle,
        &format!("PRAGMA foreign_key_list({})", quote_ident(table)),
    )?;
    match result {
        SqlResult::Query(q) => Ok(q
            .rows
            .into_iter()
            .map(|row| ForeignKeyInfo {
                name: String::new(),
                source_table: table.to_string(),
                source_columns: vec![cell_str(&row, 3)],
                target_table: cell_str(&row, 2),
                target_columns: vec![cell_str(&row, 4)],
                on_delete: row.get(5).and_then(|c| c.value_if_not_null()),
                on_update: row.get(6).and_then(|c| c.value_if_not_null()),
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn get_create_table_sql_inner(handle: &str, table: &str, _schema: &str) -> anyhow::Result<String> {
    let result = execute_inner(
        handle,
        &format!(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = '{}'",
            table.replace('\'', "''")
        ),
    )?;
    Ok(match result {
        SqlResult::Query(q) => q
            .rows
            .first()
            .and_then(|r| r.first())
            .map(|c| c.value.clone())
            .unwrap_or_default(),
        _ => String::new(),
    })
}

fn ping_inner(handle: &str) -> anyhow::Result<()> {
    let guard = lock();
    let conn = guard
        .get(handle)
        .ok_or_else(|| anyhow::anyhow!("unknown connection handle"))?;
    conn.db.execute_batch("SELECT 1")?;
    Ok(())
}

fn close_inner(handle: &str) -> anyhow::Result<()> {
    lock()
        .remove(handle)
        .map(|_| ())
        .ok_or_else(|| anyhow::anyhow!("unknown connection handle"))
}

fn switch_database_inner(handle: &str, database: &str) -> anyhow::Result<()> {
    let mut guard = lock();
    let slot = guard
        .get_mut(handle)
        .ok_or_else(|| anyhow::anyhow!("unknown connection handle"))?;
    let db = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    slot.db = db;
    slot.database = database.to_string();
    Ok(())
}

fn current_database_inner(handle: &str) -> anyhow::Result<String> {
    let guard = lock();
    guard
        .get(handle)
        .map(|c| c.database.clone())
        .ok_or_else(|| anyhow::anyhow!("unknown connection handle"))
}

// ── Slot bodies ──────────────────────────────────────────────────────────────
//
// A panic escaping an `extern "C"` frame aborts the host process, so every slot
// runs its body under `catch_unwind` and reports the outcome through the wire
// instead. These two helpers keep that one decision in one place.

fn ship<T: serde::Serialize>(
    body: impl FnOnce() -> anyhow::Result<T>,
    error: *mut PluginString,
) -> PluginString {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => ship_json(&value, error),
        Ok(Err(e)) => fail(error, e.to_string()),
        Err(_) => fail(error, "plugin panicked".to_string()),
    }
}

/// The JSON-free counterpart of [`ship`].
///
/// Three slots cross a bare string rather than a JSON document: `connect`
/// returns an opaque handle, and `current_database` and `get_create_table_sql`
/// return a single value. The host stores and compares the handle verbatim, so
/// JSON-encoding it would hand it to the next call wrapped in quotes.
fn ship_raw(
    body: impl FnOnce() -> anyhow::Result<String>,
    error: *mut PluginString,
) -> PluginString {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => ship_string(value),
        Ok(Err(e)) => fail(error, e.to_string()),
        Err(_) => fail(error, "plugin panicked".to_string()),
    }
}

fn status(body: impl FnOnce() -> anyhow::Result<()>, error: *mut PluginString) -> u32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => 0,
        Ok(Err(e)) => {
            fail(error, e.to_string());
            1
        }
        Err(_) => {
            fail(error, "plugin panicked".to_string());
            1
        }
    }
}

fn read(ptr: *const c_char) -> String {
    read_foreign(ptr).into_owned()
}

unsafe extern "C" fn slot_info(error: *mut PluginString) -> PluginString {
    ship(
        || {
            Ok(PluginInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: "SQLite driver plugin, exported over the C ABI".to_string(),
                author: "dbstudio".to_string(),
                db_type: "sqlite".to_string(),
            })
        },
        error,
    )
}

unsafe extern "C" fn slot_can_handle(db_type: *const c_char) -> u32 {
    if read(db_type) == "sqlite" {
        1
    } else {
        0
    }
}

unsafe extern "C" fn slot_connect(
    config_json: *const c_char,
    password: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (config, password) = (read(config_json), read(password));
    ship_raw(|| connect_inner(&config, &password), error)
}

unsafe extern "C" fn slot_execute(
    handle: *const c_char,
    sql: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, sql) = (read(handle), read(sql));
    ship(|| execute_inner(&handle, &sql), error)
}

unsafe extern "C" fn slot_list_databases(
    _handle: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    ship(list_databases_inner, error)
}

unsafe extern "C" fn slot_list_tables(
    handle: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, schema) = (read(handle), read(schema));
    ship(|| list_tables_inner(&handle, &schema), error)
}

unsafe extern "C" fn slot_list_columns(
    handle: *const c_char,
    table: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, table, schema) = (read(handle), read(table), read(schema));
    ship(|| list_columns_inner(&handle, &table, &schema), error)
}

unsafe extern "C" fn slot_list_indexes(
    handle: *const c_char,
    table: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, table, schema) = (read(handle), read(table), read(schema));
    ship(|| list_indexes_inner(&handle, &table, &schema), error)
}

unsafe extern "C" fn slot_list_foreign_keys(
    handle: *const c_char,
    table: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, table, schema) = (read(handle), read(table), read(schema));
    ship(|| list_foreign_keys_inner(&handle, &table, &schema), error)
}

unsafe extern "C" fn slot_get_create_table_sql(
    handle: *const c_char,
    table: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let (handle, table, schema) = (read(handle), read(table), read(schema));
    ship_raw(
        || get_create_table_sql_inner(&handle, &table, &schema),
        error,
    )
}

unsafe extern "C" fn slot_ping(handle: *const c_char, error: *mut PluginString) -> u32 {
    let handle = read(handle);
    status(|| ping_inner(&handle), error)
}

unsafe extern "C" fn slot_close(handle: *const c_char, error: *mut PluginString) -> u32 {
    let handle = read(handle);
    status(|| close_inner(&handle), error)
}

unsafe extern "C" fn slot_switch_database(
    handle: *const c_char,
    database: *const c_char,
    error: *mut PluginString,
) -> u32 {
    let (handle, database) = (read(handle), read(database));
    status(|| switch_database_inner(&handle, &database), error)
}

unsafe extern "C" fn slot_current_database(
    handle: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    let handle = read(handle);
    ship_raw(|| current_database_inner(&handle), error)
}

/// The table the loader reads. Field order is the ABI; do not reorder.
///
/// A `static` rather than a `const`: `addr_of!` on a const would materialise a
/// temporary, and each call would hand the host a different address.
static TABLE: PluginTable = PluginTable {
    free_string: free_foreign,
    info: slot_info,
    can_handle: slot_can_handle,
    connect: slot_connect,
    execute: slot_execute,
    list_databases: slot_list_databases,
    list_tables: slot_list_tables,
    list_columns: slot_list_columns,
    list_indexes: slot_list_indexes,
    list_foreign_keys: slot_list_foreign_keys,
    get_create_table_sql: slot_get_create_table_sql,
    ping: slot_ping,
    close: slot_close,
    switch_database: slot_switch_database,
    current_database: slot_current_database,
    _reserved: 0,
};

#[unsafe(no_mangle)]
pub extern "C" fn dbstudio_plugin_abi_version() -> u32 {
    PLUGIN_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn dbstudio_plugin_entry() -> *const PluginTable {
    std::ptr::addr_of!(TABLE)
}
