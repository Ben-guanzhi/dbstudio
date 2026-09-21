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

#[cfg(test)]
mod vim_tests {
    use crate::components::vim::{VimBuf, VimMode};

    fn buf(text: &str) -> VimBuf {
        let mut v = VimBuf::default();
        v.text = text.to_string();
        v.caret = 0;
        v
    }

    #[test]
    fn motions_are_byte_aware_and_utf8_safe() {
        let mut v = buf("héllo wörld");
        v.step("w", false, false, false);
        assert_eq!(v.caret, 7); // skips the multi-byte 'é'
        v.step("b", false, false, false);
        assert_eq!(v.caret, 0);
        v.step("$", false, false, false);
        assert_eq!(v.caret, 13);
    }

    #[test]
    fn numeric_count_composition() {
        let mut v = buf("abcdef");
        v.step("3", false, false, false);
        v.step("l", false, false, false);
        assert_eq!(v.caret, 3);
    }

    #[test]
    fn insert_roundtrip_after_a() {
        let mut v = buf("ab\ncd");
        v.caret = 1;
        let step = v.step("a", false, false, false).unwrap();
        assert!(step.to_insert);
        assert!(matches!(v.mode, VimMode::Insert));
        let back = v.step("escape", false, false, false).unwrap();
        assert!(!back.to_insert);
        assert!(matches!(v.mode, VimMode::Normal));
    }

    #[test]
    fn open_line_above_and_below_land_on_blank_line() {
        let mut o = buf("a\nb\nc");
        o.caret = 2; // on 'b'
        let step = o.step("o", false, false, false).unwrap();
        assert!(matches!(o.mode, VimMode::Insert));
        assert_eq!(step.caret, 4);
        assert_eq!(o.text, "a\nb\n\nc");

        let mut big = buf("a\nb\nc");
        big.caret = 2;
        let step = big.step("O", false, false, false).unwrap();
        assert_eq!(big.text, "a\n\nb\nc");
        assert_eq!(step.caret, 2);

        let mut tail = buf("a");
        tail.step("o", false, false, false).unwrap();
        assert_eq!(tail.text, "a\n");
        assert_eq!(tail.caret, 2);
    }

    #[test]
    fn x_deletes_char_with_undo_and_redo() {
        let mut v = buf("abc");
        v.caret = 1;
        let step = v.step("x", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ac"));
        assert_eq!(step.caret, 1);
        v.step("u", false, false, false);
        assert_eq!(v.text, "abc");
        v.step("r", true, false, false); // ctrl-r redo
        assert_eq!(v.text, "ac");
    }

    #[test]
    fn dd_deletes_current_line() {
        let mut v = buf("a\nb\nc");
        v.caret = 2;
        v.step("d", false, false, false);
        assert!(v.operator.is_some());
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("a\nc"));
    }

    #[test]
    fn visual_char_delete_removes_selected_range() {
        let mut v = buf("abcd ef");
        v.caret = 2;
        v.step("v", false, false, false);
        assert!(matches!(v.mode, VimMode::VisualChar));
        v.step("l", false, false, false); // extend to 'cd'
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ab ef"));
    }

    #[test]
    fn visual_block_deletes_characters_per_line() {
        let mut v = buf("abc\ndef\nghi");
        v.caret = 1; // 'b'
        v.step("v", true, false, false);
        assert!(matches!(v.mode, VimMode::VisualBlock));
        v.step("j", false, false, false); // anchor the second line
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ac\ndf\nghi"));
    }

    #[test]
    fn yank_motion_then_paste_repeats_word() {
        let mut v = buf("one two");
        v.caret = 0;
        v.step("y", false, false, false);
        v.step("w", false, false, false);
        assert_eq!(v.register, "one "); // range runs to the next word start
        v.step("p", false, false, false);
        assert_eq!(v.text, "oone ne two");
    }

    #[test]
    fn replace_pending_r_swaps_one_char() {
        let mut v = buf("abc");
        v.caret = 1;
        v.step("r", false, false, false);
        let step = v.step("z", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("azc"));
        assert_eq!(v.register, "b");
    }

    #[test]
    fn join_lines_replaces_newline_with_space() {
        let mut v = buf("select *\nfrom t");
        v.caret = 0;
        let step = v.step("J", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("select * from t"));
    }
}