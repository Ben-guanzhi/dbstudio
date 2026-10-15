use super::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn test_pool(tag: &str) -> Pool<Sqlite> {
    let path = std::env::temp_dir().join(format!(
        "dbstudio-connections-test-{}-{}.db",
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
    sqlx::query(
        "CREATE TABLE connections (
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
            environment TEXT NOT NULL DEFAULT 'dev',
            ssl_mode TEXT NOT NULL DEFAULT 'disable',
            group_name TEXT,
            tags TEXT,
            plugin_name TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

/// Run `f` while holding the keyring lock.
///
/// The OS credential store is process-global, so a keyring read or write must
/// not run alongside another test's — concurrent calls can leave a credential
/// unreadable even when the tests use different ids.
fn under_keyring<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    let _keyring = crate::keyring();
    f()
}

/// True only if the OS keyring really stores and returns a credential.
///
/// Headless CI runners have no secret service, so there `set_password` can
/// succeed while `get_password` comes back empty. `ensure_link_nonce` is the
/// same production path the deep-link handler uses, so probing it means the
/// probe can only pass where the test itself can pass.
fn keyring_probe(repo: &ConnectionsRepository) -> bool {
    under_keyring(|| {
        repo.ensure_link_nonce("test-nonce-probe").is_ok()
            && !repo.get_link_nonce("test-nonce-probe").is_empty()
    })
}

#[test]
fn test_link_nonce_is_generated_and_delete_revokes_it() {
    async_std::task::block_on(async {
        let pool = test_pool("nonce").await;
        let repo = ConnectionsRepository::new(&pool);
        let id = "test-nonce-stable";

        if !keyring_probe(&repo) {
            eprintln!("skipped: no working OS keyring");
            return;
        }

        let nonce = under_keyring(|| {
            let first = repo.ensure_link_nonce(id).unwrap();
            assert!(!first.is_empty());
            // 再取一次必须返回同一个值——深链不能每打开一次就失效。
            assert_eq!(repo.ensure_link_nonce(id).unwrap(), first);
            first
        });

        // 删除连接必须清掉 keyring 里的 nonce，否则被删连接仍能被深链唤起。
        repo.delete(id).await.unwrap();
        assert!(
            under_keyring(|| repo.get_link_nonce(id)).is_empty(),
            "deleting a connection must revoke its deep-link nonce {nonce:?}"
        );
    });
}

#[test]
fn test_no_nonce_for_an_unknown_connection() {
    async_std::task::block_on(async {
        let pool = test_pool("missing").await;
        let repo = ConnectionsRepository::new(&pool);

        if !keyring_probe(&repo) {
            eprintln!("skipped: no working OS keyring");
            return;
        }

        let id = format!("test-nonce-missing-{}", std::process::id());
        assert!(
            under_keyring(|| repo.get_link_nonce(&id)).is_empty(),
            "an unsaved connection must not have a deep-link nonce"
        );
    });
}

#[test]
fn test_saving_a_connection_mints_its_link_nonce() {
    async_std::task::block_on(async {
        let pool = test_pool("save").await;
        let repo = ConnectionsRepository::new(&pool);

        if !keyring_probe(&repo) {
            eprintln!("skipped: no working OS keyring");
            return;
        }

        // 老版本保存的连接没有 nonce；save() 负责惰性补上，不用用户重新保存。
        let mut info = ConnectionConfig::new(DatabaseType::MySQL, "saved".to_string());
        info.id = "test-nonce-save".to_string();

        assert!(
            under_keyring(|| repo.get_link_nonce(&info.id)).is_empty(),
            "nothing saved yet, so no nonce exists"
        );
        repo.save(&info, "").await.unwrap();
        assert!(
            !under_keyring(|| repo.get_link_nonce(&info.id)).is_empty(),
            "save() must mint a deep-link nonce"
        );

        repo.delete(&info.id).await.unwrap();
    });
}
