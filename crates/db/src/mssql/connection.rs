use anyhow::Result;
use async_lock::Mutex;
use dbstudio_core::models::ConnectionConfig;
use dbstudio_core::result::{ResultCell, SqlResult};
use dbstudio_core::schema::ColumnInfo;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::tunnel::{Endpoint, runtime};
use crate::utils::{finalize_query_result, hex_encode, is_select_query};

type TdsClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// MSSQL connection backed by the shared global tokio runtime.
///
/// The global runtime (see [`crate::tunnel::TOKIO_RUNTIME`]) is created once
/// and reused for both SSH tunnel and MSSQL workloads. Per-connection runtimes
/// were removed to avoid spawning a new event loop for every connection.
pub struct MssqlConnection {
    client: Arc<Mutex<Option<TdsClient>>>,
    database: Arc<RwLock<String>>,
}

impl MssqlConnection {
    pub async fn open(config: &ConnectionConfig, password: &str) -> Result<Self> {
        let endpoint = Endpoint::resolve(config);

        let mut cfg = tiberius::Config::new();
        cfg.host(endpoint.host.clone());
        cfg.port(endpoint.port);
        cfg.authentication(tiberius::AuthMethod::sql_server(
            config.username.clone(),
            password.to_string(),
        ));
        cfg.database(config.database.clone());
        // Require TLS encryption on the wire so credentials and data are not
        // sent in cleartext. `trust_cert` (as before) skips certificate-chain
        // validation, which keeps self-signed/private-CA servers working while
        // still preventing passive eavesdropping.
        cfg.trust_cert();
        cfg.encryption(tiberius::EncryptionLevel::Required);

        let host = endpoint.host.clone();
        let port = endpoint.port;
        // Reuse the global shared tokio runtime instead of creating a per-
        // connection runtime. `handle().spawn()` is safe here because the
        // future captures only `Arc`-wrapped references that outlive the task.
        let handle = runtime().handle().clone();
        let client = handle
            .spawn(async move {
                let addr = format!("{}:{}", host, port);
                let tcp = tokio::net::TcpStream::connect(addr).await?;
                tiberius::Client::connect(cfg, tcp.compat())
                    .await
                    .map_err(anyhow::Error::from)
            })
            .await
            .map_err(|e| anyhow::anyhow!("MSSQL connect task panicked: {}", e))??;

        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
            database: Arc::new(RwLock::new(config.database.clone())),
        })
    }

    pub fn database(&self) -> String {
        self.database.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn ping(&self) -> Result<()> {
        match self.execute("SELECT 1").await? {
            SqlResult::Query(_) => Ok(()),
            _ => Err(anyhow::anyhow!("Ping failed")),
        }
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        let sql = sql.to_string();
        let start = Instant::now();
        let is_select = is_select_query(&sql, &[]);

        let client = self.client.clone();
        let handle = runtime().handle().clone();

        let result = handle
            .spawn(async move {
                let mut guard = client.lock().await;
                let tc = guard
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

                if is_select {
                    let mut result = tc
                        .query(&sql, &[])
                        .await
                        .map_err(anyhow::Error::from)?;

                    let columns: Vec<ColumnInfo> = {
                        let cols = result
                            .columns()
                            .await
                            .map_err(anyhow::Error::from)?
                            .unwrap_or(&[]);
                        cols.iter()
                            .enumerate()
                            .map(|(i, c)| {
                                ColumnInfo::new(
                                    c.name().to_string(),
                                    format!("{:?}", c.column_type()),
                                    i as u32,
                                )
                            })
                            .collect()
                    };

                    let result_stream = result
                        .into_first_result()
                        .await
                        .map_err(anyhow::Error::from)?;

                    let mut rows = Vec::new();
                    for row in result_stream.iter() {
                        let mut cells = Vec::new();
                        for i in 0..row.len() {
                            cells.push(extract_tds_cell(row, i));
                        }
                        rows.push(cells);
                    }

                    let elapsed = start.elapsed().as_millis();
                    Ok(finalize_query_result(columns, rows, sql, elapsed))
                } else {
                    let done = tc
                        .execute(&sql, &[])
                        .await
                        .map_err(anyhow::Error::from)?;
                    let rows_affected: u64 = done.rows_affected().iter().sum();
                    let elapsed = start.elapsed().as_millis();
                    Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
                        rows_affected,
                        execution_time_ms: elapsed,
                        sql,
                        message: format!("{} rows affected", rows_affected),
                    }))
                }
            })
            .await;

        match result {
            Ok(inner) => inner,
            Err(join_err) => {
                if join_err.is_panic() {
                    let payload = join_err.into_panic();
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    Err(anyhow::anyhow!("MSSQL query panicked: {}", msg))
                } else {
                    Err(anyhow::anyhow!("MSSQL query task cancelled"))
                }
            }
        }
    }

    pub async fn current_database(&self) -> Result<String> {
        let result = self.execute("SELECT DB_NAME()").await?;
        Ok(result
            .first_cell()
            .map(|c| c.value.clone())
            .unwrap_or_else(|| self.database()))
    }

    pub async fn execute_parameterized(&self, sql: &str, params: &[&str]) -> Result<SqlResult> {
        let sql = sql.to_string();
        let params: Vec<String> = params.iter().map(|s| s.to_string()).collect();
        let start = Instant::now();
        let is_select = is_select_query(&sql, &[]);

        let client = self.client.clone();
        let handle = runtime().handle().clone();

        let result = handle
            .spawn(async move {
                let mut guard = client.lock().await;
                let tc = guard
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

                if is_select {
                    let param_refs: Vec<&dyn tiberius::ToSql> = params
                        .iter()
                        .map(|p| p as &dyn tiberius::ToSql)
                        .collect();
                    let mut result = tc
                        .query(&sql, &param_refs)
                        .await
                        .map_err(anyhow::Error::from)?;

                    let columns: Vec<ColumnInfo> = {
                        let cols = result
                            .columns()
                            .await
                            .map_err(anyhow::Error::from)?
                            .unwrap_or(&[]);
                        cols.iter()
                            .enumerate()
                            .map(|(i, c)| {
                                ColumnInfo::new(
                                    c.name().to_string(),
                                    format!("{:?}", c.column_type()),
                                    i as u32,
                                )
                            })
                            .collect()
                    };

                    let result_stream = result
                        .into_first_result()
                        .await
                        .map_err(anyhow::Error::from)?;

                    let mut rows = Vec::new();
                    for row in result_stream.iter() {
                        let mut cells = Vec::new();
                        for i in 0..row.len() {
                            cells.push(extract_tds_cell(row, i));
                        }
                        rows.push(cells);
                    }

                    let elapsed = start.elapsed().as_millis();
                    Ok(finalize_query_result(columns, rows, sql, elapsed))
                } else {
                    let param_refs: Vec<&dyn tiberius::ToSql> = params
                        .iter()
                        .map(|p| p as &dyn tiberius::ToSql)
                        .collect();
                    let done = tc
                        .execute(&sql, &param_refs)
                        .await
                        .map_err(anyhow::Error::from)?;
                    let rows_affected: u64 = done.rows_affected().iter().sum();
                    let elapsed = start.elapsed().as_millis();
                    Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
                        rows_affected,
                        execution_time_ms: elapsed,
                        sql,
                        message: format!("{} rows affected", rows_affected),
                    }))
                }
            })
            .await;

        match result {
            Ok(inner) => inner,
            Err(join_err) => {
                if join_err.is_panic() {
                    let payload = join_err.into_panic();
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    Err(anyhow::anyhow!("MSSQL query panicked: {}", msg))
                } else {
                    Err(anyhow::anyhow!("MSSQL query task cancelled"))
                }
            }
        }
    }

    pub async fn switch_database(&self, database: &str) -> Result<()> {
        self.execute(&format!("USE {}", crate::utils::quote_bracket(database)))
            .await?;
        if let Ok(mut g) = self.database.write() {
            *g = database.to_string();
        }
        Ok(())
    }
}

fn extract_tds_cell(row: &tiberius::Row, index: usize) -> ResultCell {
    use tiberius::ColumnData;

    use dbstudio_core::result::CellType;

    let value = row.cells().nth(index).map(|(_, v)| v).cloned();

    match value {
        Some(ColumnData::U8(v)) => v.map(|v| ResultCell::integer(v as i64)),
        Some(ColumnData::I16(v)) => v.map(|v| ResultCell::integer(v as i64)),
        Some(ColumnData::I32(v)) => v.map(|v| ResultCell::integer(v as i64)),
        Some(ColumnData::I64(v)) => v.map(ResultCell::integer),
        Some(ColumnData::F32(v)) => v.map(|v| ResultCell::float(v as f64)),
        Some(ColumnData::F64(v)) => v.map(ResultCell::float),
        Some(ColumnData::Bit(v)) => v.map(ResultCell::boolean),
        Some(ColumnData::String(v)) => v.map(ResultCell::text),
        Some(ColumnData::Guid(v)) => v.map(|g| ResultCell::typed(CellType::Uuid, g.to_string())),
        Some(ColumnData::Binary(v)) => v.as_ref().map(|b| ResultCell::typed(CellType::Binary, format!("0x{}", hex_encode(b)))),
        Some(ColumnData::Numeric(v)) => v.map(|v| {
            let approx = v.to_string().parse::<f64>().unwrap_or(0.0);
            ResultCell::decimal(v.to_string(), approx)
        }),
        Some(ColumnData::Xml(v)) => v.map(|v| ResultCell::typed(CellType::Other, format!("{:?}", v))),
        Some(ColumnData::DateTime(v)) => v.map(|v| ResultCell::typed(CellType::DateTime, tds_datetime(v.days(), v.seconds_fragments()))),
        Some(ColumnData::SmallDateTime(v)) => v.map(|v| ResultCell::typed(CellType::DateTime, tds_smalldatetime(v.days(), v.seconds_fragments()))),
        Some(ColumnData::Date(v)) => v.map(|v| ResultCell::typed(CellType::Date, tds_date(v.days()))),
        Some(ColumnData::Time(v)) => v.map(|v| ResultCell::typed(CellType::Time, tds_time(v.increments(), v.scale()))),
        Some(ColumnData::DateTime2(v)) => v.map(|v| ResultCell::typed(CellType::DateTime, tds_datetime2(v.date(), v.time()))),
        Some(ColumnData::DateTimeOffset(v)) => {
            v.map(|v| ResultCell::typed(CellType::DateTime, tds_datetimeoffset(v.datetime2(), v.offset())))
        }
        _ => None,
    }
    .unwrap_or_else(ResultCell::null)
}

fn tds_epoch_date(epoch_year: i32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(epoch_year, 1, 1).unwrap()
}

fn tds_midnight() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
}

fn tds_datetime(days: i32, seconds_fragments: u32) -> String {
    let date = tds_epoch_date(1900) + chrono::Duration::days(days as i64);
    let ns = seconds_fragments as i64 * 1_000_000_000 / 300;
    let time = tds_midnight() + chrono::Duration::nanoseconds(ns);
    format!("{} {}", date, time)
}

fn tds_smalldatetime(days: u16, minutes: u16) -> String {
    let date = tds_epoch_date(1900) + chrono::Duration::days(days as i64);
    let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(minutes as u32 * 60, 0)
        .unwrap_or_else(tds_midnight);
    format!("{} {}", date, time)
}

fn tds_date(days: u32) -> String {
    (tds_epoch_date(1) + chrono::Duration::days(days as i64)).to_string()
}

fn tds_time(increments: u64, scale: u8) -> String {
    let ns = (increments as i128) * 10i128.pow(9 - scale.min(9) as u32);
    (tds_midnight() + chrono::Duration::nanoseconds(ns as i64)).to_string()
}

fn tds_datetime2(date: tiberius::time::Date, time: tiberius::time::Time) -> String {
    format!(
        "{} {}",
        tds_date(date.days()),
        tds_time(time.increments(), time.scale())
    )
}

fn tds_datetimeoffset(datetime2: tiberius::time::DateTime2, offset: i16) -> String {
    let base = tds_datetime2(datetime2.date(), datetime2.time());
    let sign = if offset < 0 { "-" } else { "+" };
    let abs = offset.unsigned_abs();
    format!("{} {}{:02}:{:02}", base, sign, abs / 60, abs % 60)
}