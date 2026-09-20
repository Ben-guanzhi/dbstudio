use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_db::export::{export_database, import_database, ExportConfig};

fn temp_db_path(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("dbstudio-export-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().to_string()
}

fn sqlite_config(path: &str) -> ConnectionConfig {
    let mut config = ConnectionConfig::new(DatabaseType::SQLite, "test".to_string());
    config.database = path.to_string();
    config
}

fn setup_source_db(source: &str) {
    let config = sqlite_config(source);
    smol::block_on(async {
        let conn = dbstudio_db::connect(&config, "", None, None).await.expect("connect");
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL)")
            .await
            .expect("create");
        conn.execute("INSERT INTO items (name, price) VALUES ('apple', 1.5), ('banana', 2.25), ('cherry', 3.0)")
            .await
            .expect("insert");
        // Edge case: value containing a single quote, needs escaping
        conn.execute("INSERT INTO items (name, price) VALUES ('o''brien', 4.0)")
            .await
            .expect("insert escaped quote");
    });
}

#[test]
fn export_import_round_trip_preserves_data() {
    let source = temp_db_path("source.db");
    let dest = temp_db_path("dest.db");
    let dump = temp_db_path("dump.sql");
    let _ = std::fs::remove_file(&dump);

    setup_source_db(&source);

    let source_config = sqlite_config(&source);
    let smol_source = source_config.clone();
    let dump_path = dump.clone();

    smol::block_on(async {
        let conn = dbstudio_db::connect(&smol_source, "", None, None).await.expect("connect");
        let config = ExportConfig::default();
        let stats = export_database(&conn, std::path::Path::new(&dump_path), &config, None)
            .await
            .expect("export");

        assert_eq!(stats.tables_exported, 1);
        assert_eq!(stats.rows_exported, 4);
    });

    let dump_content = std::fs::read_to_string(&dump).expect("read dump");
    assert!(dump_content.contains("DROP TABLE IF EXISTS `items`"));
    assert!(dump_content.contains("CREATE TABLE items"), "dump should preserve schema SQL: {}", dump_content);
    assert!(dump_content.contains("INSERT INTO `items`"));
    assert!(dump_content.contains("'o''brien'"), "dump must escape single quotes");

    // Import into a fresh database
    let dest_config = sqlite_config(&dest);
    let smol_dest = dest_config.clone();
    let dump_path = dump.clone();

    smol::block_on(async {
        let conn = dbstudio_db::connect(&smol_dest, "", None, None).await.expect("connect");
        let stats = import_database(&conn, std::path::Path::new(&dump_path), None)
            .await
            .expect("import");

        assert_eq!(stats.statements_executed, 6); // DROP + CREATE + 4 INSERTs
        assert_eq!(stats.rows_imported, 4);
        assert_eq!(stats.errors, 0);

        let result = conn
            .execute("SELECT id, name, price FROM items ORDER BY id")
            .await
            .expect("select");
        let dbstudio_core::result::SqlResult::Query(q) = result else {
            panic!("expected query");
        };
        assert_eq!(q.row_count, 4, "all rows should round-trip");
        assert_eq!(q.rows[0][1].value, "apple");
        assert_eq!(q.rows[3][1].value, "o'brien", "escaped quote should round-trip");
    });
}

#[test]
fn export_data_only_config() {
    let source = temp_db_path("data_only.db");
    let dump = temp_db_path("data_only.sql");
    let _ = std::fs::remove_file(&dump);

    setup_source_db(&source);
    let config = sqlite_config(&source);
    let dump_path = dump.clone();

    smol::block_on(async {
        let conn = dbstudio_db::connect(&config, "", None, None).await.expect("connect");
        let export_config = ExportConfig {
            include_schema: false,
            include_data: true,
            include_drop: false,
            tables: vec!["items".to_string()],
            ..Default::default()
        };
        let stats = export_database(&conn, std::path::Path::new(&dump_path), &export_config, None)
            .await
            .expect("export");
        assert_eq!(stats.rows_exported, 4);
    });

    let dump_content = std::fs::read_to_string(&dump).expect("read dump");
    assert!(!dump_content.contains("DROP TABLE"), "data-only must not emit DROP");
    assert!(!dump_content.contains("CREATE TABLE"), "data-only must not emit CREATE");
    assert!(dump_content.contains("INSERT INTO `items`"));
}

#[test]
fn export_schema_only_config() {
    let source = temp_db_path("schema_only.db");
    let dump = temp_db_path("schema_only.sql");
    let _ = std::fs::remove_file(&dump);

    setup_source_db(&source);
    let config = sqlite_config(&source);
    let dump_path = dump.clone();

    smol::block_on(async {
        let conn = dbstudio_db::connect(&config, "", None, None).await.expect("connect");
        let export_config = ExportConfig {
            include_schema: true,
            include_data: false,
            include_drop: true,
            ..Default::default()
        };
        let stats = export_database(&conn, std::path::Path::new(&dump_path), &export_config, None)
            .await
            .expect("export");
        assert_eq!(stats.rows_exported, 0);
        assert_eq!(stats.tables_exported, 0);
    });

    let dump_content = std::fs::read_to_string(&dump).expect("read dump");
    assert!(dump_content.contains("CREATE TABLE items"), "schema-only should emit CREATE: {}", dump_content);
    assert!(!dump_content.contains("INSERT INTO `items`"), "schema-only must not emit INSERT");
}