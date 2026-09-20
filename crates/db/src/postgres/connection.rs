use anyhow::Result;
use dbstudio_core::models::ConnectionConfig;
use crate::utils::{build_query_result_from_rows, finalize_query_result};
use dbstudio_core::result::{CellType, ResultCell, SqlResult, MAX_RESULT_ROWS};
use dbstudio_core::schema::ColumnInfo;
use sqlx::{Arguments, Column, Executor, PgPool, Row, TypeInfo, ValueRef};
use std::sync::RwLock;
use std::time::Instant;

use futures::{StreamExt, TryStreamExt};

use rust_decimal::prelude::ToPrimitive;

use crate::tunnel::Endpoint;
use crate::utils::{extra_params_query, is_select_query, urlencode_credentials};

/// Whether the pool should be replaced on a live reconnection.
///
/// `PostgreSQL` has no `USE database` equivalent: switching databases means
/// dialing a fresh pool bound to the new database name. The old pool is dropped
/// only after the new one is up, so a failed switch leaves the connection intact.
pub struct PostgresConnection {
    pool: RwLock<PgPool>,
    database: RwLock<String>,
    connect: RwLock<ConnectionConfig>,
    password: RwLock<String>,
}

impl PostgresConnection {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: RwLock::new(pool),
            database: RwLock::new(String::new()),
            connect: RwLock::new(ConnectionConfig::new(
                dbstudio_core::models::DatabaseType::PostgreSQL,
                "unused".to_string(),
            )),
            password: RwLock::new(String::new()),
        }
    }

    /// Build the connection URL for `config` and resolve it to a pool.
    async fn create_pool(config: &ConnectionConfig, password: &str) -> Result<PgPool> {
        let endpoint = Endpoint::resolve(config);

        let (user, pwd) = (
            urlencode_credentials(&config.username),
            urlencode_credentials(password),
        );
        let url = format!(
            "postgres://{}:{}@{}:{}/{}",
            user,
            pwd,
            endpoint.host,
            endpoint.port,
            urlencode_credentials(&config.database)
        );
        let extra = extra_params_query(config);
        let extra = {
            let ssl = crate::utils::ssl_mode_query(config);
            if ssl.is_empty() {
                extra
            } else if extra.is_empty() {
                ssl
            } else {
                format!("{extra}&{ssl}")
            }
        };
        let url = if extra.is_empty() {
            url
        } else if url.contains('?') {
            format!("{url}&{extra}")
        } else {
            format!("{url}?{extra}")
        };

        Ok(sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(&url)
            .await?)
    }

    pub async fn open(config: &ConnectionConfig, password: &str) -> Result<Self> {
        let pool = Self::create_pool(config, password).await?;
        Ok(Self {
            pool: RwLock::new(pool),
            database: RwLock::new(config.database.clone()),
            connect: RwLock::new(config.clone()),
            password: RwLock::new(password.to_string()),
        })
    }

    /// The underlying pool. `.clone()` is cheap (pool handles are `Arc`s).
    fn current_pool(&self) -> Result<PgPool> {
        self.pool
            .read()
            .map(|g| g.clone())
            .map_err(|_| anyhow::anyhow!("PostgreSQL pool lock poisoned"))
    }

    pub fn pool(&self) -> Result<PgPool> {
        self.current_pool()
    }

    pub async fn ping(&self) -> Result<()> {
        let pool = self.current_pool()?;
        sqlx::query("SELECT 1").execute(&pool).await?;
        Ok(())
    }

    pub async fn execute(&self, sql: &str) -> Result<SqlResult> {
        let pool = self.current_pool()?;
        let sql = sql.to_string();
        let start = Instant::now();

        if is_select_query(&sql, &[]) {
            let statement = sqlx::query(&sql);
            // Stream at most MAX_RESULT_ROWS + 1 rows so a huge result set is
            // never materialized in full by `fetch_all`; the +1 lets the
            // client-side cap below report `truncated` correctly.
            let rows: Vec<sqlx::postgres::PgRow> = statement
                .fetch(&pool)
                .take(MAX_RESULT_ROWS + 1)
                .try_collect()
                .await?;
            let elapsed = start.elapsed().as_millis();
            if rows.is_empty() {
                if let Ok(desc) = pool.describe(&sql).await {
                    let columns: Vec<ColumnInfo> = desc
                        .columns()
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            ColumnInfo::new(
                                c.name().to_string(),
                                c.type_info().name().to_string(),
                                i as u32,
                            )
                        })
                        .collect();
                    return Ok(finalize_query_result(columns, Vec::new(), sql, elapsed));
                }
            }
            return Ok(build_query_result_from_rows(rows, sql, elapsed, extract_pg_cell));
        }

        let result = sqlx::query(&sql).execute(&pool).await?;
        let elapsed = start.elapsed().as_millis();
        let rows_affected = result.rows_affected();
        Ok(SqlResult::Modified(dbstudio_core::result::ExecResult {
            rows_affected,
            execution_time_ms: elapsed,
            sql,
            message: format!("{} rows affected", rows_affected),
        }))
    }

    pub async fn execute_parameterized(&self, sql: &str, params: &[&str]) -> Result<SqlResult> {
        let pool = self.current_pool()?;
        let sql_owned = sql.to_string();
        let start = Instant::now();

        let mut args = sqlx::postgres::PgArguments::default();
        for p in params {
            args.add(*p).map_err(anyhow::Error::msg)?;
        }
        let statement = sqlx::query_with(&sql_owned, args);
        let rows: Vec<sqlx::postgres::PgRow> = statement
            .fetch(&pool)
            .take(MAX_RESULT_ROWS + 1)
            .try_collect()
            .await?;
        let elapsed = start.elapsed().as_millis();
        if rows.is_empty() {
            if let Ok(desc) = pool.describe(&sql_owned).await {
                let columns: Vec<ColumnInfo> = desc
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        ColumnInfo::new(
                            c.name().to_string(),
                            c.type_info().name().to_string(),
                            i as u32,
                        )
                    })
                    .collect();
                return Ok(finalize_query_result(columns, Vec::new(), sql_owned, elapsed));
            }
        }
        Ok(build_query_result_from_rows(rows, sql_owned, elapsed, extract_pg_cell))
    }

    pub async fn current_database(&self) -> Result<String> {
        let pool = self.current_pool()?;
        let row: (String,) = sqlx::query_as("SELECT current_database()")
            .fetch_one(&pool)
            .await?;
        Ok(row.0)
    }

    /// Switch database by dialing a fresh pool bound to `database`.
    ///
    /// The new pool is created before the old one is dropped; on failure the
    /// current pool stays usable so the caller can keep working.
    pub async fn switch_database(&self, database: &str) -> Result<()> {
        let db = database.trim().to_string();
        if db.is_empty() {
            return Err(anyhow::anyhow!("Database name must not be empty"));
        }

        let config = self
            .connect
            .read()
            .map(|g| g.clone())
            .map_err(|_| anyhow::anyhow!("PostgreSQL config lock poisoned"))?;
        let password = self
            .password
            .read()
            .map(|g| g.clone())
            .map_err(|_| anyhow::anyhow!("PostgreSQL password lock poisoned"))?;

        let mut new_config = config;
        new_config.database = db.clone();
        let new_pool = Self::create_pool(&new_config, &password).await?;

        let mut pool_guard = self
            .pool
            .write()
            .map_err(|_| anyhow::anyhow!("PostgreSQL pool lock poisoned"))?;
        *pool_guard = new_pool;
        drop(pool_guard);

        let mut config_guard = self
            .connect
            .write()
            .map_err(|_| anyhow::anyhow!("PostgreSQL config lock poisoned"))?;
        config_guard.database = db.clone();
        drop(config_guard);

        if let Ok(mut g) = self.database.write() {
            *g = db;
        }
        Ok(())
    }
}

fn extract_pg_cell(row: &sqlx::postgres::PgRow, index: usize) -> ResultCell {
    let raw = row.try_get_raw(index);
    let is_null = raw.map(|r| r.is_null()).unwrap_or(true);
    if is_null {
        return ResultCell::null();
    }

    if let Ok(v) = row.try_get::<String, _>(index) {
        return ResultCell::text(v);
    }
    if let Ok(v) = row.try_get::<i32, _>(index) {
        return ResultCell::integer(v as i64);
    }
    if let Ok(v) = row.try_get::<i64, _>(index) {
        return ResultCell::integer(v);
    }
    if let Ok(v) = row.try_get::<f64, _>(index) {
        return ResultCell::float(v);
    }
    if let Ok(v) = row.try_get::<bool, _>(index) {
        return ResultCell::boolean(v);
    }
    if let Ok(v) = row.try_get::<sqlx::types::JsonValue, _>(index) {
        return ResultCell::typed(CellType::Json, v.to_string());
    }
    if let Ok(v) = row.try_get::<sqlx::types::Uuid, _>(index) {
        return ResultCell::typed(CellType::Uuid, v.to_string());
    }
    if let Ok(v) = row.try_get::<rust_decimal::Decimal, _>(index) {
        return ResultCell::decimal(v.to_string(), v.to_f64().unwrap_or(0.0));
    }
    if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(index) {
        return ResultCell::typed(CellType::Date, v.to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(index) {
        return ResultCell::typed(CellType::Time, v.to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(index) {
        return ResultCell::typed(CellType::DateTime, v.to_string());
    }

    ResultCell::typed(CellType::Other, "[unsupported]")
}
