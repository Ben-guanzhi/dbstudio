use super::*;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn test_pool() -> Pool<Sqlite> {
    let path =
        std::env::temp_dir().join(format!("dbstudio-settings-test-{}.db", std::process::id()));
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
        set_setting_bool(&pool, "app.safe_mode", false)
            .await
            .unwrap();
        assert!(!get_setting_bool(&pool, "app.safe_mode", true).await);

        set_setting(&pool, "app.last_theme", "dark").await.unwrap();
        assert_eq!(
            get_setting(&pool, "app.last_theme").await.as_deref(),
            Some("dark")
        );
        assert_eq!(get_setting(&pool, "missing.key").await, None);
    });
}
