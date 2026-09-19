use anyhow::Result;
use async_lock::Mutex;
use dbstudio_core::models::ConnectionConfig;
use dbstudio_core::result::{ResultCell, SqlResult};
use dbstudio_core::schema::ColumnInfo;
use std::sync::Arc;
use std::time::Instant;

use crate::tunnel::Endpoint;
use crate::utils::{finalize_query_result, hex_encode, is_select_query};

pub struct OracleConnection {
    conn: Arc<Mutex<Option<oracle::Connection>>>,
    service_name: String,
}

impl OracleConnection {
    pub async fn open(config: &ConnectionConfig, password: &str) -> Result<Self> {
        let service = if config.database.is_empty() {
            "ORCL".to_string()
        } else {
            config.database.clone()
        };

        let endpoint = Endpoint::resolve(config);
        let connect_string = format!("//{}:{}/{}", endpoint.host, endpoint.port, service);
        let username = config.username.clone();
        let password = password.to_string();

        let conn = smol::unblock(move || {
            oracle::Connection::connect(&username, &password, &connect_string)
                .map_err(|e| anyhow::anyhow!("Oracle connect error: {}", e))
        })
        .await?;

        Ok(Self {
            conn: Arc::new(Mutex::new(Some(conn))),
            service_name: service,
        })
    }

    pub fn service_name(&self) -> String {
        self.service_name.clone()
    }

    pub async fn ping(&self) -> Result<()> {
        self.execute_with_conn(|c| c.query("SELECT 1 FROM dual", &[]).map(|_| ()))
            .await
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        let sql_string = sql.to_string();
        let start = Instant::now();

        let conn = self.conn.clone();
        smol::unblock(move || {
            let guard = conn.lock_blocking();
            let c = guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

            let is_select = is_select_query(&sql_string, &["DESCRIBE"]);

            if is_select {
                let q = c
                    .query(&sql_string, &[])
                    .map_err(|e| anyhow::anyhow!("Oracle query error: {}", e))?;

                let columns: Vec<ColumnInfo> = q
                    .column_info()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        ColumnInfo::new(
                            c.name().to_string(),
                            c.oracle_type().to_string(),
                            i as u32,
                        )
                    })
                    .collect();

                let mut rows = Vec::new();
                for row in q {
                    let row = row.map_err(|e| anyhow::anyhow!("Oracle row error: {}", e))?;
                    let mut cells = Vec::new();
                    for v in row.sql_values() {
                        cells.push(sqlvalue_to_result(v));
                    }
                    rows.push(cells);
                }
                let elapsed = start.elapsed().as_millis();
                Ok(finalize_query_result(columns, rows, sql_string, elapsed))
            } else {
                let stmt = c
                    .execute(&sql_string, &[])
                    .map_err(|e| anyhow::anyhow!("Oracle execute error: {}", e))?;
                let rows_affected = stmt.row_count().unwrap_or(0);
                let elapsed = start.elapsed().as_millis();
                Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
                    rows_affected,
                    execution_time_ms: elapsed,
                    sql: sql_string,
                    message: format!("{} rows affected", rows_affected),
                }))
            }
        })
        .await
    }

    async fn execute_with_conn<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&oracle::Connection) -> Result<(), oracle::Error> + Send + 'static,
    {
        let conn = self.conn.clone();
        smol::unblock(move || {
            let guard = conn.lock_blocking();
            let c = guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
            f(c).map_err(|e| anyhow::anyhow!("Oracle error: {}", e))?;
            Ok(())
        })
        .await
    }

    pub async fn current_database(&self) -> Result<String> {
        let result = self
            .execute("SELECT SYS_CONTEXT('USERENV', 'DB_NAME') FROM dual")
            .await?;
        Ok(result
            .first_cell()
            .map(|c| c.value.clone())
            .unwrap_or_default())
    }

    pub async fn execute_parameterized(&self, sql: &str, params: &[&str]) -> Result<SqlResult> {
        let sql_string = sql.to_string();
        let params: Vec<String> = params.iter().map(|s| s.to_string()).collect();
        let start = Instant::now();

        let conn = self.conn.clone();
        smol::unblock(move || {
            let guard = conn.lock_blocking();
            let c = guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;

            let is_select = is_select_query(&sql_string, &["DESCRIBE"]);

            if is_select {
                let param_refs: Vec<&dyn oracle::sql_type::ToSql> = params
                    .iter()
                    .map(|p| p as &dyn oracle::sql_type::ToSql)
                    .collect();
                let q = c
                    .query(&sql_string, param_refs.as_slice())
                    .map_err(|e| anyhow::anyhow!("Oracle query error: {}", e))?;

                let columns: Vec<ColumnInfo> = q
                    .column_info()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        ColumnInfo::new(
                            c.name().to_string(),
                            c.oracle_type().to_string(),
                            i as u32,
                        )
                    })
                    .collect();

                let mut rows = Vec::new();
                for row in q {
                    let row = row.map_err(|e| anyhow::anyhow!("Oracle row error: {}", e))?;
                    let mut cells = Vec::new();
                    for v in row.sql_values() {
                        cells.push(sqlvalue_to_result(v));
                    }
                    rows.push(cells);
                }
                let elapsed = start.elapsed().as_millis();
                Ok(finalize_query_result(columns, rows, sql_string, elapsed))
            } else {
                let param_refs: Vec<&dyn oracle::sql_type::ToSql> = params
                    .iter()
                    .map(|p| p as &dyn oracle::sql_type::ToSql)
                    .collect();
                let stmt = c
                    .execute(&sql_string, param_refs.as_slice())
                    .map_err(|e| anyhow::anyhow!("Oracle execute error: {}", e))?;
                let rows_affected = stmt.row_count().unwrap_or(0);
                let elapsed = start.elapsed().as_millis();
                Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
                    rows_affected,
                    execution_time_ms: elapsed,
                    sql: sql_string,
                    message: format!("{} rows affected", rows_affected),
                }))
            }
        })
        .await
    }

    pub async fn switch_database(&self, database: &str) -> Result<()> {
        anyhow::bail!(
            "Oracle cannot switch databases on an existing connection; reconnect using database {database:?}"
        );
    }
}

fn sqlvalue_to_result(v: &oracle::SqlValue) -> ResultCell {
    use dbstudio_core::result::CellType;

    if v.is_null().unwrap_or(true) {
        return ResultCell::null();
    }
    if let Ok(s) = v.get::<String>() {
        return ResultCell::text(s);
    }
    if let Ok(i) = v.get::<i64>() {
        return ResultCell::integer(i);
    }
    if let Ok(u) = v.get::<u64>() {
        return ResultCell::decimal(u.to_string(), u as f64);
    }
    if let Ok(f) = v.get::<f64>() {
        return ResultCell::float(f);
    }
    if let Ok(b) = v.get::<Vec<u8>>() {
        return ResultCell::typed(CellType::Binary, format!("0x{}", hex_encode(&b)));
    }
    ResultCell::typed(CellType::Other, "[unsupported]")
}
