use super::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn test_pool(tag: &str) -> Pool<Sqlite> {
    let path = std::env::temp_dir().join(format!(
        "dbstudio-ai-settings-test-{}-{}.db",
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

/// True only if the OS keyring really stores and returns a password.
///
/// Headless CI runners have no secret service, so there `set_password` can
/// succeed while `get_password` comes back empty and the API key is silently
/// lost. Probe with a sentinel using the same production functions the test
/// exercises, so the probe can only pass where the test itself can pass.
fn keyring_round_trips() -> bool {
    let sentinel = "dbstudio-keyring-probe";
    let previous = get_ai_api_key();
    let ok = set_ai_api_key(sentinel).is_ok() && get_ai_api_key() == sentinel;
    let _ = set_ai_api_key(&previous);
    ok
}

#[test]
fn ai_config_keyring_round_trip() {
    // The OS keyring is process-global, so the probe and the write/verify/clear
    // steps must not race with another test's keyring use.
    let _keyring = crate::keyring();

    if !keyring_round_trips() {
        eprintln!("skipping: no working OS keyring on this host (no secret service)");
        return;
    }

    async_std::task::block_on(async {
        let pool = test_pool("cfg").await;

        let mut config = LlmConfig {
            provider: "ollama".into(),
            base_url: Some("http://localhost:11434/v1".into()),
            model: Some("llama3.1".into()),
            ..LlmConfig::default()
        };
        config.api_key = Some("top-secret".into());

        save_ai_config(&pool, &config).await.unwrap();

        let loaded = load_ai_config(&pool).await;
        assert_eq!(loaded.provider, "ollama");
        assert_eq!(
            loaded.base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(loaded.model.as_deref(), Some("llama3.1"));
        assert_eq!(loaded.api_key.as_deref(), Some("top-secret"));

        let stored: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
            .bind(AI_CONFIG_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!stored.contains("top-secret"));

        // Saving without a key must clear the keyring credential.
        config.api_key = None;
        save_ai_config(&pool, &config).await.unwrap();
        let loaded = load_ai_config(&pool).await;
        assert!(loaded.api_key.is_none());
    });
}
