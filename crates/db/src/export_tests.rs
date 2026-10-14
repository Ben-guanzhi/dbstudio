use super::*;

#[test]
fn quoting_matches_dialect() {
    struct Case {
        db: DatabaseType,
        expected: &'static str,
    }
    let cases = [
        Case {
            db: DatabaseType::SQLite,
            expected: "`items`",
        },
        Case {
            db: DatabaseType::MySQL,
            expected: "`items`",
        },
        Case {
            db: DatabaseType::PostgreSQL,
            expected: "\"items\"",
        },
        Case {
            db: DatabaseType::Oracle,
            expected: "\"items\"",
        },
        Case {
            db: DatabaseType::MSSQL,
            expected: "[items]",
        },
    ];
    for case in cases {
        assert_eq!(quote_ident(&case.db, "items"), case.expected);
    }
}

#[test]
fn qualified_name_prefers_schema_field() {
    let info = TableInfo {
        name: "users".to_string(),
        schema: Some("public".to_string()),
        database: None,
        table_type: TableType::Table,
        row_count: None,
        comment: None,
    };
    assert_eq!(
        qualified_name(&DatabaseType::PostgreSQL, &info),
        "\"public\".\"users\""
    );
    assert_eq!(
        qualified_name(&DatabaseType::MSSQL, &info),
        "[public].[users]"
    );
    // No schema is reported: the identifier stays unqualified.
    let plain = TableInfo {
        name: "users".to_string(),
        schema: None,
        database: None,
        table_type: TableType::Table,
        row_count: None,
        comment: None,
    };
    assert_eq!(qualified_name(&DatabaseType::SQLite, &plain), "`users`");
}

#[test]
fn select_limit_is_dialect_specific() {
    assert_eq!(
        select_statement(&DatabaseType::MySQL, "`t`", 10),
        "SELECT * FROM `t` LIMIT 10"
    );
    assert_eq!(
        select_statement(&DatabaseType::Oracle, "\"t\"", 10),
        "SELECT * FROM \"t\" FETCH FIRST 10 ROWS ONLY"
    );
    assert_eq!(
        select_statement(&DatabaseType::MSSQL, "[t]", 10),
        "SELECT TOP 10 * FROM [t]"
    );
    assert_eq!(
        select_statement(&DatabaseType::MSSQL, "[t]", 0),
        "SELECT * FROM [t]"
    );
}

#[test]
fn drop_statement_oracle_purges() {
    assert_eq!(
        drop_statement(&DatabaseType::Oracle, "\"t\""),
        "DROP TABLE \"t\" PURGE;"
    );
    assert_eq!(
        drop_statement(&DatabaseType::PostgreSQL, "\"t\""),
        "DROP TABLE IF EXISTS \"t\";"
    );
}
