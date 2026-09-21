use anyhow::Result;
use dbstudio_core::ai::LlmConfig;
use sqlx::{Pool, Sqlite};

const AI_CONFIG_KEY: &str = "ai.config";
const KEYRING_SERVICE: &str = dbstudio_core::NAMESPACE;
const KEYRING_ID: &str = "ai:apikey";

fn keyring_entry() -> keyring::Entry {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ID).unwrap_or_else(|_| {
        keyring::Entry::new_with_target(KEYRING_ID, KEYRING_SERVICE, "")
            .expect("Failed to create keyring entry")
    })
}

/// API keys never touch the SQLite settings table; they live in the OS keyring.
pub fn get_ai_api_key() -> String {
    keyring_entry().get_password().unwrap_or_default()
}

pub fn set_ai_api_key(key: &str) -> Result<()> {
    let entry = keyring_entry();
    if key.is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry.set_password(key)?;
    }
    Ok(())
}

/// Load the persisted AI config, overlaying the keyring-stored API key.
pub async fn load_ai_config(pool: &Pool<Sqlite>) -> LlmConfig {
    let mut config = match sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE key = ?1",
    )
    .bind(AI_CONFIG_KEY)
    .fetch_optional(pool)
    .await
    {
        Ok(Some((json,))) => serde_json::from_str::<LlmConfig>(&json).unwrap_or_default(),
        _ => LlmConfig::default(),
    };
    let key = get_ai_api_key();
    config.api_key = if key.is_empty() { None } else { Some(key) };
    config
}

/// Persist the AI config. The provider/base/model go to the settings table;
/// the API key is stored in the OS keyring and stripped from the stored JSON.
pub async fn save_ai_config(pool: &Pool<Sqlite>, config: &LlmConfig) -> Result<()> {
    set_ai_api_key(config.api_key.as_deref().unwrap_or(""))?;
    let mut stored = config.clone();
    stored.api_key = None;
    let json = serde_json::to_string(&stored)?;
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
        .bind(AI_CONFIG_KEY)
        .bind(&json)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn ai_config_keyring_round_trip() {
        // Single test on purpose: the OS keyring is process-global, so the
        // set/verify/clear steps must not race with another test's keyring use.
        async_std::task::block_on(async {
            let pool = test_pool("cfg").await;

            let mut config = LlmConfig::default();
            config.provider = "ollama".into();
            config.base_url = Some("http://localhost:11434/v1".into());
            config.model = Some("llama3.1".into());
            config.api_key = Some("top-secret".into());

            save_ai_config(&pool, &config).await.unwrap();

            let loaded = load_ai_config(&pool).await;
            assert_eq!(loaded.provider, "ollama");
            assert_eq!(loaded.base_url.as_deref(), Some("http://localhost:11434/v1"));
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
}