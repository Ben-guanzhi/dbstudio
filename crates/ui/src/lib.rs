#![recursion_limit = "256"]

pub mod components;
pub mod state;
pub mod url_scheme;
pub mod url_scheme_registry;
pub mod utils;

#[cfg(test)]
mod filter_tests {
    use dbstudio_core::result::{CellType, ResultCell};
    use crate::components::results_panel::{cell_matches_filter, like_match, operators_for, FilterOp};

    #[test]
    fn equality_and_contains_are_case_insensitive() {
        assert!(cell_matches_filter(Some(&ResultCell::text("Alice")), FilterOp::Equals, "alice"));
        assert!(!cell_matches_filter(Some(&ResultCell::text("Alice")), FilterOp::Equals, "bob"));
        assert!(cell_matches_filter(Some(&ResultCell::text("Hello World")), FilterOp::Contains, "WORLD"));
        assert!(!cell_matches_filter(Some(&ResultCell::text("Hello World")), FilterOp::Contains, "foo"));
        assert!(cell_matches_filter(Some(&ResultCell::text("Hello World")), FilterOp::StartsWith, "hello"));
    }

    #[test]
    fn null_cells_only_match_emptiness_operators() {
        let null = ResultCell::null();
        assert!(cell_matches_filter(Some(&null), FilterOp::IsEmpty, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::NotEmpty, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::Equals, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::Contains, "x"));
        assert!(!cell_matches_filter(None, FilterOp::IsEmpty, ""));
    }

    #[test]
    fn like_supports_percent_and_underscore() {
        assert!(like_match("hello world", "hello%"));
        assert!(like_match("hello world", "%world"));
        assert!(like_match("hello world", "%llo w%"));
        assert!(like_match("abc", "a_c"));
        assert!(!like_match("abc", "a__c"));
        assert!(like_match("hello world", "%o_w%"));
    }

    #[test]
    fn numeric_operators_use_sort_proxy() {
        assert!(cell_matches_filter(Some(&ResultCell::integer(10)), FilterOp::GreaterThan, "5"));
        assert!(cell_matches_filter(Some(&ResultCell::integer(10)), FilterOp::LessThan, "11"));
        assert!(!cell_matches_filter(Some(&ResultCell::float(1.5)), FilterOp::GreaterThan, "2"));
        // A non-numeric expected value falls back to a substring match.
        assert!(cell_matches_filter(Some(&ResultCell::text("x10y")), FilterOp::GreaterThan, "10y"));
        // Numeric ops against non-numeric cells with numeric values match nothing.
        assert!(!cell_matches_filter(Some(&ResultCell::text("x10y")), FilterOp::GreaterThan, "10"));
    }

    #[test]
    fn operators_for_group_by_cell_type() {
        let numeric = operators_for(CellType::Integer);
        assert!(numeric.contains(&FilterOp::GreaterThan));
        assert!(!numeric.contains(&FilterOp::Contains));
        let text = operators_for(CellType::Text);
        assert!(text.contains(&FilterOp::Contains));
        assert!(!text.contains(&FilterOp::GreaterThan));
    }
}

#[cfg(test)]
mod pagination_tests {
    use dbstudio_core::models::DatabaseType;
    use crate::state::operations::paginate_query;

    #[test]
    fn simple_dialects_use_limit_offset() {
        for db in [DatabaseType::SQLite, DatabaseType::MySQL, DatabaseType::PostgreSQL] {
            assert_eq!(
                paginate_query("SELECT * FROM t ORDER BY id", 100, 200, db),
                "SELECT * FROM ( SELECT * FROM t ORDER BY id ) AS _dbstudio_sub LIMIT 100 OFFSET 200"
            );
        }
    }

    #[test]
    fn trailing_semicolons_are_stripped() {
        assert_eq!(
            paginate_query("SELECT * FROM t;  ", 10, 0, DatabaseType::SQLite),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub LIMIT 10 OFFSET 0"
        );
    }

    #[test]
    fn mssql_uses_offset_fetch() {
        assert_eq!(
            paginate_query("SELECT * FROM t", 100, 200, DatabaseType::MSSQL),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub ORDER BY (SELECT NULL) OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
    }

    #[test]
    fn oracle_uses_offset_fetch() {
        assert_eq!(
            paginate_query("SELECT * FROM t", 100, 200, DatabaseType::Oracle),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
    }
}

#[cfg(test)]
mod guard_tests {
    use dbstudio_core::models::Environment;
    use crate::state::guard::{WriteKind, classify_sql, requires_confirmation};

    #[test]
    fn classifies_common_statements() {
        assert_eq!(classify_sql("  SELECT * FROM t"), Some(WriteKind::Select));
        assert_eq!(classify_sql("select id from users"), Some(WriteKind::Select));
        assert_eq!(classify_sql("INSERT INTO t VALUES (1)"), Some(WriteKind::Insert));
        assert_eq!(classify_sql("UPDATE t SET a = 1 WHERE id = 2"), Some(WriteKind::Update));
        assert_eq!(classify_sql("UPDATE t SET a = 1"), Some(WriteKind::UpdateNoWhere));
        assert_eq!(classify_sql("DELETE FROM t WHERE id = 2"), Some(WriteKind::Delete));
        assert_eq!(classify_sql("DELETE FROM t"), Some(WriteKind::DeleteNoWhere));
        assert_eq!(classify_sql("DROP TABLE t"), Some(WriteKind::Drop));
        assert_eq!(classify_sql("TRUNCATE TABLE t"), Some(WriteKind::Truncate));
        assert_eq!(classify_sql("ALTER TABLE t ADD COLUMN c INT"), Some(WriteKind::Alter));
        assert_eq!(classify_sql("CREATE TABLE t (id INT)"), Some(WriteKind::Create));
        assert_eq!(classify_sql("VACUUM"), Some(WriteKind::OtherDdl));
    }

    #[test]
    fn classify_ignores_unknown_and_empty() {
        assert_eq!(classify_sql(""), None);
        assert_eq!(classify_sql("   "), None);
        assert_eq!(classify_sql("NOT SQL AT ALL"), None);
    }

    #[test]
    fn safe_mode_confirms_every_write() {
        assert!(requires_confirmation(Some(WriteKind::Insert), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::Update), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::UpdateNoWhere), Environment::Staging, true));
        assert!(requires_confirmation(Some(WriteKind::Delete), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::DeleteNoWhere), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::Drop), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::Alter), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::Create), Environment::Dev, true));
        assert!(requires_confirmation(Some(WriteKind::OtherDdl), Environment::Dev, true));
        assert!(!requires_confirmation(Some(WriteKind::Select), Environment::Dev, true));
        assert!(!requires_confirmation(Some(WriteKind::Select), Environment::Production, true));
    }

    #[test]
    fn without_safe_mode_environments_apply() {
        assert!(!requires_confirmation(Some(WriteKind::Update), Environment::Dev, false));
        assert!(!requires_confirmation(Some(WriteKind::Delete), Environment::Staging, false));
        assert!(requires_confirmation(
            Some(WriteKind::DeleteNoWhere),
            Environment::Dev,
            false
        ));
        assert!(requires_confirmation(Some(WriteKind::Insert), Environment::Production, false));
        assert!(!requires_confirmation(
            Some(WriteKind::Select),
            Environment::Production,
            false
        ));
    }
}