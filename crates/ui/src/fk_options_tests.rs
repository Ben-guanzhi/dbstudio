use crate::state::build_fk_options_sql;
use dbstudio_core::models::DatabaseType;

#[test]
fn fk_options_sql_selects_distinct_references() {
    let sql = build_fk_options_sql("customers", "id", 100, 0, DatabaseType::PostgreSQL);
    assert!(
        sql.contains("SELECT DISTINCT \"id\" FROM \"customers\""),
        "{sql}"
    );
    assert!(sql.contains("IS NOT NULL"), "{sql}");
    assert!(sql.contains("ORDER BY \"id\""), "{sql}");
    assert!(sql.contains("LIMIT 100"), "{sql}");
}

#[test]
fn fk_options_sql_quotes_per_dialect() {
    let sql = build_fk_options_sql("customers", "id", 50, 0, DatabaseType::MySQL);
    assert!(sql.contains("`id`"), "{sql}");
    assert!(sql.contains("`customers`"), "{sql}");
    assert!(sql.contains("LIMIT 50"), "{sql}");

    let sql = build_fk_options_sql("customers", "id", 50, 0, DatabaseType::MSSQL);
    assert!(sql.contains("[id]"), "{sql}");
}
