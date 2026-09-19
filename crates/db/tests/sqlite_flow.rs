use dbstudio_core::models::{DatabaseType, ConnectionConfig};
use dbstudio_core::result::SqlResult;

fn temp_db_path(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("dbclient-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().to_string()
}

fn sqlite_config(path: &str) -> ConnectionConfig {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    ConnectionConfig {
        id: "test".to_string(),
        name: "test".to_string(),
        db_type: DatabaseType::SQLite,
        host: String::new(),
        port: 0,
        database: path.to_string(),
        username: String::new(),
        color: None,
        ssh_enabled: false,
        ssh_host: None,
        ssh_port: None,
        ssh_username: None,
        ssh_auth_type: None,
        ssh_key_path: None,
        extra_params: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[test]
fn sqlite_full_flow() {
    let path = temp_db_path("full.db");
    let config = sqlite_config(&path);

    smol::block_on(async {
        let conn = dbstudio_db::connect(&config, "", None, None).await.expect("connect");
        conn.ping().await.expect("ping");

        // DDL + DML round trip
        let missing = dbstudio_db::list_columns(&conn, "people", None).await.expect("columns");
        assert!(missing.is_empty(), "missing table should yield no columns");

        let _ = conn
            .execute("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .await
            .expect("create table");

        let modified = conn
            .execute("INSERT INTO people (name) VALUES ('alice'), ('bob')")
            .await
            .expect("insert");
        assert!(matches!(modified, SqlResult::Modified(exec) if exec.rows_affected == 2));

        let result = conn
            .execute("SELECT id, name FROM people ORDER BY id")
            .await
            .expect("select");
        let SqlResult::Query(q) = result else {
            panic!("expected query result");
        };
        assert_eq!(q.columns.len(), 2);
        assert_eq!(q.row_count, 2);
        assert_eq!(q.rows[0][1].value, "alice");
        assert_eq!(q.rows[1][1].value, "bob");

        // Introspection
        let tables = dbstudio_db::list_tables(&conn, "").await.expect("tables");
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "people");

        let columns = dbstudio_db::list_columns(&conn, "people", None).await.expect("columns");
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].name, "id");
        assert!(columns[0].is_primary_key, "id should be PK");

        let _indexes = dbstudio_db::list_indexes(&conn, "people", None).await.expect("indexes");

        let create_sql = dbstudio_db::get_create_table_sql(&conn, "people", None).await;
        assert!(create_sql.is_ok());
        assert!(create_sql.unwrap_or_default().to_lowercase().contains("people"));

        // Error path
        let err = conn.execute("SELECT * FROM missing_table").await;
        assert!(err.is_err());
    });
}

#[test]
fn sqlite_switch_database_and_current() {
    let first = temp_db_path("first.db");
    let second = temp_db_path("second.db");
    let config = sqlite_config(&first);
    smol::block_on(async {
        let conn = dbstudio_db::connect(&config, "", None, None).await.expect("connect");
        assert_eq!(conn.current_database().await.unwrap_or_default(), first);

        conn.switch_database(&second).await.expect("switch");
        assert_eq!(conn.current_database().await.unwrap_or_default(), second);
    });
}