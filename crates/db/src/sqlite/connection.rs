use anyhow::Result;
use async_lock::Mutex;
use dbstudio_core::models::ConnectionConfig;
use dbstudio_core::result::{ResultCell, SqlResult};
use dbstudio_core::schema::*;
use rusqlite::params;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::utils::{finalize_query_result, is_select_query};

pub struct SqliteConnection {
    pub(crate) conn: Arc<Mutex<Option<rusqlite::Connection>>>,
    database: Arc<RwLock<String>>,
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
                ResultCell::typed(
                    dbstudio_core::result::CellType::Blob,
                    format!("blob[{} bytes]", bytes.len()),
                )
            }
            Err(_) => ResultCell::text("[error]"),
        };
        cells.push(cell);
    }
    cells
}

impl SqliteConnection {
    pub async fn open(config: &ConnectionConfig) -> Result<Self> {
        let path = config.database.clone();
        let open_path = path.clone();
        let conn = smol::unblock(move || {
            let conn = rusqlite::Connection::open_with_flags(
                &open_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            )?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA busy_timeout=5000;",
            )?;
            Ok::<_, anyhow::Error>(conn)
        })
        .await?;

        Ok(Self {
            conn: Arc::new(Mutex::new(Some(conn))),
            database: Arc::new(RwLock::new(path)),
        })
    }

    pub fn database(&self) -> String {
        self.database.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn ping(&self) -> Result<()> {
        let conn = self.conn.clone();
        smol::unblock(move || {
            let guard = conn.lock_blocking();
            let c = guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
            c.execute_batch("SELECT 1")?;
            Ok(())
        })
        .await
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        let conn = self.conn.clone();
        let sql = sql.to_string();
        let start = Instant::now();

        smol::unblock(move || {
            let guard = conn.lock_blocking();
            let c = guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

            if is_select_query(&sql, &["PRAGMA", "EXPLAIN"]) {
                let mut stmt = c.prepare(&sql)?;
                let columns: Vec<ColumnInfo> = stmt
                    .column_names()
                    .iter()
                    .enumerate()
                    .map(|(i, name)| ColumnInfo::new(*name, "TEXT", i as u32))
                    .collect();

                let mut rows_iter = stmt.query(params![])?;
                let mut rows = Vec::new();
                while let Some(row) = rows_iter.next()? {
                    rows.push(row_to_cells(row, columns.len()));
                }

                let elapsed = start.elapsed().as_millis();
                Ok(finalize_query_result(columns, rows, sql, elapsed))
            } else {
                let rows_affected = c.execute(&sql, params![])? as u64;
                let elapsed = start.elapsed().as_millis();
                Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
                    rows_affected,
                    execution_time_ms: elapsed,
                    sql,
                    message: format!("{} rows affected", rows_affected),
                }))
            }
        })
        .await
    }

    pub async fn current_database(&self) -> Result<String> {
        Ok(self.database())
    }

    pub async fn switch_database(&self, database: &str) -> Result<()> {
        let conn = self.conn.clone();
        let path = database.to_string();
        let open_path = path.clone();
        smol::unblock(move || {
            let new_conn = rusqlite::Connection::open_with_flags(
                &open_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
            )?;
            let mut guard = conn.lock_blocking();
            *guard = Some(new_conn);
            Ok::<_, anyhow::Error>(())
        })
        .await?;
        if let Ok(mut g) = self.database.write() {
            *g = path;
        }
        Ok(())
    }
}
