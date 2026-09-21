use anyhow::Result;
use sqlx::{Pool, Sqlite};

/// Read a raw string value from the `settings` table.
pub async fn get_setting(pool: &Pool<Sqlite>, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Read a boolean setting with a default for missing/unparseable values.
pub async fn get_setting_bool(pool: &Pool<Sqlite>, key: &str, default: bool) -> bool {
    match get_setting(pool, key).await {
        Some(v) => v == "true" || v == "1",
        None => default,
    }
}

/// Write a raw string value to the `settings` table.
pub async fn set_setting(pool: &Pool<Sqlite>, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// Write a boolean setting to the `settings` table.
pub async fn set_setting_bool(pool: &Pool<Sqlite>, key: &str, value: bool) -> Result<()> {
    set_setting(pool, key, if value { "true" } else { "false" }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn test_pool() -> Pool<Sqlite> {
        let path = std::env::temp_dir().join(format!(
            "dbstudio-settings-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("DROP TABLE IF EXISTS settings")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn settings_round_trip() {
        async_std::task::block_on(async {
            let pool = test_pool().await;

            assert!(get_setting_bool(&pool, "app.safe_mode", true).await);
            set_setting_bool(&pool, "app.safe_mode", false).await.unwrap();
            assert!(!get_setting_bool(&pool, "app.safe_mode", true).await);

            set_setting(&pool, "app.last_theme", "dark").await.unwrap();
            assert_eq!(
                get_setting(&pool, "app.last_theme").await.as_deref(),
                Some("dark")
            );
            assert_eq!(get_setting(&pool, "missing.key").await, None);
        });
    }
}