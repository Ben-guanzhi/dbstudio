use anyhow::Result;
use sqlx::{Pool, Sqlite};
use chrono::{DateTime, Utc};

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
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// A single-connection pool on a throwaway file so every `favorites`
    /// statement hits the same SQLite database.
    async fn test_pool(tag: &str) -> Pool<Sqlite> {
        let path = std::env::temp_dir().join(format!(
            "dbstudio-favorites-test-{}-{}.db",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("DROP TABLE IF EXISTS favorites")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE favorites (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                sql TEXT NOT NULL,
                connection_id TEXT,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn favorites_crud_round_trip() {
        async_std::task::block_on(async {
            let pool = test_pool("crud").await;
            let repo = FavoritesRepository::new(&pool);

            let id = repo
                .save("top customers", "SELECT * FROM customers", Some("c1"))
                .await
                .unwrap();

            let all = repo.load_all().await.unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].id, id);
            assert_eq!(all[0].name, "top customers");
            assert_eq!(all[0].connection_id.as_deref(), Some("c1"));

            repo.update(id, "best customers", "SELECT * FROM customers LIMIT 10")
                .await
                .unwrap();
            let all = repo.load_all().await.unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].name, "best customers");
            assert_eq!(all[0].sql, "SELECT * FROM customers LIMIT 10");

            repo.delete(id).await.unwrap();
            assert!(repo.load_all().await.unwrap().is_empty());
        });
    }

    #[test]
    fn favorites_persist_scoped_and_unscoped_connection_ids() {
        async_std::task::block_on(async {
            let pool = test_pool("ids").await;
            let repo = FavoritesRepository::new(&pool);

            let _a = repo.save("scoped", "SELECT 1", Some("conn-1")).await.unwrap();
            let _b = repo.save("global", "SELECT 2", None).await.unwrap();

            let all = repo.load_all().await.unwrap();
            assert_eq!(all.len(), 2);
            assert!(all.iter().any(|f| f.name == "scoped" && f.connection_id.as_deref() == Some("conn-1")));
            assert!(all.iter().any(|f| f.name == "global" && f.connection_id.is_none()));
        });
    }
}
