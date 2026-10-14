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
#[path = "settings_tests.rs"]
mod tests;
