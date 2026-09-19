use anyhow::Result;
use async_lock::OnceCell;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::PathBuf;

use crate::connections::ConnectionsRepository;
use crate::history::QueryHistoryRepository;

static STORE: OnceCell<AppStore> = OnceCell::new();

pub struct AppStore {
    pool: Pool<Sqlite>,
}

impl AppStore {
    pub async fn singleton() -> Result<&'static Self> {
        STORE
            .get_or_try_init(|| async { Self::new().await })
            .await
    }

    async fn new() -> Result<Self> {
        let db_path = Self::db_path();
        Self::migrate_legacy(&db_path);

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    /// The SQLite store under the current [`dbstudio_core::NAMESPACE`].
    fn db_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(dbstudio_core::NAMESPACE)
            .join(dbstudio_core::STORE_FILE_NAME)
    }

    /// The SQLite store of pre-rename installs under
    /// [`dbstudio_core::LEGACY_NAMESPACE`].
    fn legacy_db_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(dbstudio_core::LEGACY_NAMESPACE)
            .join(dbstudio_core::LEGACY_STORE_FILE_NAME)
    }

    /// One-time migration: copy the legacy `dbclient` store into the new
    /// namespace when no migrated store exists yet.
    ///
    /// The legacy file is left untouched so a failed or partial migration can
    /// always be retried from the original.
    fn migrate_legacy(db_path: &std::path::Path) {
        if db_path.exists() {
            return;
        }
        if let Err(err) = copy_if_missing(&Self::legacy_db_path(), db_path) {
            tracing::warn!("legacy dbclient store migration failed: {err}");
        }
    }

    async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS connections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                db_type TEXT NOT NULL,
                host TEXT NOT NULL DEFAULT '',
                port INTEGER NOT NULL DEFAULT 0,
                database TEXT NOT NULL DEFAULT '',
                username TEXT NOT NULL DEFAULT '',
                ssh_enabled INTEGER NOT NULL DEFAULT 0,
                ssh_host TEXT,
                ssh_port INTEGER,
                ssh_username TEXT,
                ssh_auth_type TEXT,
                ssh_key_path TEXT,
                extra_params TEXT,
                color TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        // Migration: databases created by older versions have no
        // `extra_params` column yet; add it on first run when missing.
        let has_col: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pragma_table_info('connections')
             WHERE name = 'extra_params'",
        )
        .fetch_one(&self.pool)
        .await?;
        if has_col.0 == 0 {
            sqlx::query("ALTER TABLE connections ADD COLUMN extra_params TEXT")
                .execute(&self.pool)
                .await?;
        }

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS query_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                connection_id TEXT NOT NULL,
                sql TEXT NOT NULL,
                execution_time_ms INTEGER NOT NULL DEFAULT 0,
                row_count INTEGER,
                is_error INTEGER NOT NULL DEFAULT 0,
                executed_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_query_history_connection
             ON query_history(connection_id, executed_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub fn connections(&self) -> ConnectionsRepository<'_> {
        ConnectionsRepository::new(&self.pool)
    }

    pub fn history(&self) -> QueryHistoryRepository<'_> {
        QueryHistoryRepository::new(&self.pool)
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }
}

/// Copy `from` to `to` if `to` does not exist yet, creating parent directories.
/// A missing `from` is not an error (fresh installs have no legacy data).
fn copy_if_missing(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if to.exists() || !from.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    tracing::info!(
        "migrated legacy store {} -> {}",
        from.display(),
        to.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dbstudio-store-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_if_missing_migrates_legacy_store() {
        let dir = temp_dir("migrate");
        let legacy = dir.join("legacy").join("dbclient.db");
        let target = dir.join("new").join("dbstudio.db");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"store").unwrap();

        copy_if_missing(&legacy, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"store");

        // Legacy original is kept so migrations can be retried.
        assert!(legacy.exists());

        // Idempotent: an existing target is never overwritten.
        std::fs::write(&target, b"newer").unwrap();
        copy_if_missing(&legacy, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"newer");
    }

    #[test]
    fn copy_if_missing_tolerates_missing_legacy_store() {
        let dir = temp_dir("fresh");
        let target = dir.join("new").join("dbstudio.db");
        copy_if_missing(&dir.join("does-not-exist.db"), &target).unwrap();
        assert!(!target.exists());
    }
}
