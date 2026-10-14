use crate::state::operations::paginate_query;
use dbstudio_core::models::DatabaseType;

#[test]
fn simple_dialects_use_limit_offset() {
    for db in [
        DatabaseType::SQLite,
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
    ] {
        assert_eq!(
            paginate_query("SELECT * FROM t ORDER BY id", 100, 200, db),
            "SELECT * FROM ( SELECT * FROM t ORDER BY id ) AS dbstudio_sub LIMIT 100 OFFSET 200"
        );
    }
}

#[test]
fn trailing_semicolons_are_stripped() {
    assert_eq!(
        paginate_query("SELECT * FROM t;  ", 10, 0, DatabaseType::SQLite),
        "SELECT * FROM ( SELECT * FROM t ) AS dbstudio_sub LIMIT 10 OFFSET 0"
    );
}

#[test]
fn mssql_uses_offset_fetch() {
    assert_eq!(
        paginate_query("SELECT * FROM t", 100, 200, DatabaseType::MSSQL),
        "SELECT * FROM ( SELECT * FROM t ) AS dbstudio_sub ORDER BY (SELECT NULL) OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
    );
}

#[test]
fn oracle_uses_offset_fetch() {
    assert_eq!(
        paginate_query("SELECT * FROM t", 100, 200, DatabaseType::Oracle),
        "SELECT * FROM ( SELECT * FROM t ) dbstudio_sub OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
    );
}
