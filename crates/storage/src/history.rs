use anyhow::Result;
use sqlx::{Pool, Sqlite};

use super::types::QueryHistoryEntry;

pub struct QueryHistoryRepository<'a> {
    pool: &'a Pool<Sqlite>,
}

impl<'a> QueryHistoryRepository<'a> {
    pub fn new(pool: &'a Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub async fn record(
        &self,
        connection_id: &str,
        sql: &str,
        execution_time_ms: u128,
        row_count: Option<usize>,
        is_error: bool,
    ) -> Result<()> {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        sqlx::query(
            "INSERT INTO query_history
             (connection_id, sql, execution_time_ms, row_count, is_error, executed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(connection_id)
        .bind(sql)
        .bind(execution_time_ms as i64)
        .bind(row_count.map(|r| r as i64))
        .bind(is_error as i32)
        .bind(&now)
        .execute(self.pool)
        .await?;

        // Bound table growth: drop the oldest rows per connection after every
        // insert so the local history cannot grow without limit.
        self.prune(500).await?;
        Ok(())
    }

    pub async fn load_for_connection(
        &self,
        connection_id: &str,
        limit: usize,
    ) -> Result<Vec<QueryHistoryEntry>> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT id, connection_id, sql, execution_time_ms, row_count, is_error, executed_at
             FROM query_history
             WHERE connection_id = ?1
             ORDER BY executed_at DESC
             LIMIT ?2",
        )
        .bind(connection_id)
        .bind(limit as i64)
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_entry()).collect())
    }

    pub async fn load_recent(&self, limit: usize) -> Result<Vec<QueryHistoryEntry>> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT id, connection_id, sql, execution_time_ms, row_count, is_error, executed_at
             FROM query_history
             ORDER BY executed_at DESC
             LIMIT ?1",
        )
        .bind(limit as i64)
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_entry()).collect())
    }

    pub async fn clear_for_connection(&self, connection_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM query_history WHERE connection_id = ?1")
            .bind(connection_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune(&self, keep_per_connection: usize) -> Result<()> {
        sqlx::query(&format!(
            "DELETE FROM query_history WHERE id NOT IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY connection_id ORDER BY executed_at DESC) as rn
                    FROM query_history
                ) WHERE rn <= {}
            )",
            keep_per_connection
        ))
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct HistoryRow {
    id: i64,
    connection_id: String,
    sql: String,
    execution_time_ms: i64,
    row_count: Option<i64>,
    is_error: i32,
    executed_at: String,
}

impl HistoryRow {
    fn into_entry(self) -> QueryHistoryEntry {
        QueryHistoryEntry {
            id: self.id,
            connection_id: self.connection_id,
            sql: self.sql,
            execution_time_ms: self.execution_time_ms as u128,
            row_count: self.row_count.map(|r| r as usize),
            is_error: self.is_error != 0,
            executed_at: self.executed_at,
        }
    }
}
