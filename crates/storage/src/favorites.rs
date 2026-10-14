use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};

#[derive(Debug, Clone)]
pub struct FavoriteQuery {
    pub id: i64,
    pub name: String,
    pub sql: String,
    pub connection_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct FavoritesRepository<'a> {
    pool: &'a Pool<Sqlite>,
}

impl<'a> FavoritesRepository<'a> {
    pub fn new(pool: &'a Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub async fn save(&self, name: &str, sql: &str, connection_id: Option<&str>) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO favorites (name, sql, connection_id, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(name)
        .bind(sql)
        .bind(connection_id)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn load_all(&self) -> Result<Vec<FavoriteQuery>> {
        let rows: Vec<(i64, String, String, Option<String>, String)> =
            sqlx::query_as("SELECT id, name, sql, connection_id, created_at FROM favorites ORDER BY created_at DESC")
                .fetch_all(self.pool)
                .await?;

        Ok(rows
            .into_iter()
            .map(|(id, name, sql, connection_id, created_at)| FavoriteQuery {
                id,
                name,
                sql,
                connection_id,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM favorites WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn update(&self, id: i64, name: &str, sql: &str) -> Result<()> {
        sqlx::query("UPDATE favorites SET name = ?, sql = ? WHERE id = ?")
            .bind(name)
            .bind(sql)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "favorites_tests.rs"]
mod tests;
