use super::*;

fn kind(sql: &str) -> MutationKind {
    classify_sql_mutations(sql).expect("expected a classification")
}

#[test]
fn blank_and_comment_only_input_is_none() {
    assert_eq!(classify_sql_mutations(""), None);
    assert_eq!(classify_sql_mutations("   \t\n"), None);
    assert_eq!(classify_sql_mutations("-- only a comment"), None);
    assert_eq!(classify_sql_mutations("/* only a comment */"), None);
}

#[test]
fn plain_reads_are_read_only() {
    for sql in [
        "SELECT 1",
        "select id from users",
        "SHOW TABLES",
        "DESCRIBE users",
        "EXPLAIN SELECT 1",
        "PRAGMA table_info('users')",
        "VALUES (1)",
    ] {
        assert_eq!(kind(sql), MutationKind::Read, "expected Read for {sql:?}");
    }
}

#[test]
fn writes_leading_the_statement_are_mutations() {
    assert_eq!(kind("INSERT INTO t VALUES (1)"), MutationKind::Data);
    assert_eq!(kind("UPDATE t SET a = 1"), MutationKind::Data);
    assert_eq!(kind("DELETE FROM t"), MutationKind::Data);
    assert_eq!(
        kind("MERGE INTO t USING s ON t.id = s.id"),
        MutationKind::Data
    );
    assert_eq!(kind("REPLACE INTO t VALUES (1)"), MutationKind::Data);
    assert_eq!(kind("TRUNCATE TABLE t"), MutationKind::Data);
}

#[test]
fn schema_and_privilege_changes_are_mutations() {
    for sql in [
        "DROP TABLE t",
        "CREATE TABLE t (a INT)",
        "ALTER TABLE t ADD c INT",
        "GRANT SELECT ON t TO u",
        "REVOKE SELECT ON t FROM u",
        "COMMENT ON TABLE t IS 'x'",
        "VACUUM",
        "REINDEX t",
        "ANALYZE",
        "CALL do_thing()",
        "EXEC do_thing()",
        "RENAME TABLE a TO b",
    ] {
        assert_eq!(
            kind(sql),
            MutationKind::Schema,
            "expected Schema for {sql:?}"
        );
    }
}

#[test]
fn select_for_update_is_a_lock_hint_not_a_write() {
    assert_eq!(kind("SELECT * FROM t FOR UPDATE"), MutationKind::Read);
    assert_eq!(
        kind("SELECT * FROM t FOR UPDATE SKIP LOCKED"),
        MutationKind::Read
    );
    // …but a write keyword elsewhere in the same batch still wins.
    assert_eq!(
        kind("SELECT * FROM t FOR UPDATE; DELETE FROM t"),
        MutationKind::Data
    );
}

#[test]
fn comment_before_a_write_does_not_hide_it() {
    assert_eq!(kind("-- a note\nDROP TABLE t"), MutationKind::Schema);
    assert_eq!(kind("/* note */ DELETE FROM t"), MutationKind::Data);
    // A `WHERE` inside a block comment never scopes an unscoped DELETE.
    assert_eq!(kind("DELETE /* WHERE x = 1 */ FROM t"), MutationKind::Data);
}

#[test]
fn multi_statement_batches_are_scanned_in_full() {
    // The single most important bypass: a read prefix must not legitimise a
    // trailing write.
    assert_eq!(kind("SELECT 1; DROP TABLE users"), MutationKind::Schema);
    assert_eq!(kind("SELECT 1; DELETE FROM users"), MutationKind::Data);
    assert_eq!(
        kind("EXPLAIN SELECT 1; UPDATE t SET a = 1"),
        MutationKind::Data
    );
    assert_eq!(kind("SHOW TABLES; SHOW TABLES"), MutationKind::Read);
}

#[test]
fn data_modifying_cte_is_a_write_despite_the_outer_select() {
    assert_eq!(
        kind("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d"),
        MutationKind::Data
    );
    assert_eq!(
        kind("WITH x AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM x"),
        MutationKind::Data
    );
    // A read-only CTE stays read-only.
    assert_eq!(
        kind("WITH x AS (SELECT 1) SELECT * FROM x"),
        MutationKind::Read
    );
}

#[test]
fn literals_and_identifiers_do_not_look_like_statements() {
    assert_eq!(
        kind("SELECT 'DROP TABLE t', \"UPDATE\" FROM dual"),
        MutationKind::Read
    );
    assert_eq!(
        kind("SELECT * FROM t WHERE note = 'delete me'"),
        MutationKind::Read
    );
    // A column named after a write keyword is not a statement.
    assert_eq!(kind("SELECT deleted FROM t"), MutationKind::Read);
    assert_eq!(kind("SELECT `delete`, 'insert' FROM t"), MutationKind::Read);
    // MySQL backslash escapes inside a literal.
    assert_eq!(
        kind("SELECT 'a\\'b DROP TABLE t' FROM x"),
        MutationKind::Read
    );
    // Bracket identifiers.
    assert_eq!(kind("SELECT [delete] FROM t"), MutationKind::Read);
}

#[test]
fn unknown_leading_keyword_reports_unknown_not_read() {
    assert_eq!(kind("USE mydb"), MutationKind::Unknown);
    assert_eq!(
        kind("SET SESSION max_connections = 10"),
        MutationKind::Unknown
    );
    // Unknown head with a write keyword present: the write wins.
    assert_eq!(kind("USE mydb; DELETE FROM t"), MutationKind::Data);
}

#[test]
fn unknown_is_not_a_mutation_under_the_lenient_rule() {
    assert!(!MutationKind::Unknown.is_mutation());
    assert!(!MutationKind::Read.is_mutation());
    assert!(MutationKind::Data.is_mutation());
    assert!(MutationKind::Schema.is_mutation());
}

#[test]
fn unclosed_comment_neutralises_only_what_follows_it() {
    // An unclosed `--` comment eats to end of line, and an unclosed `/* */`
    // eats to end of input — neither yields a statement.
    assert_eq!(classify_sql_mutations("-- never closed DROP TABLE t"), None);
    // …but a complete statement before the unterminated comment survives.
    assert_eq!(
        classify_sql_mutations("SELECT 1 /* never closed"),
        Some(MutationKind::Read)
    );
    assert_eq!(
        classify_sql_mutations("SELECT 1 -- never closed"),
        Some(MutationKind::Read)
    );
}

#[test]
fn mutation_keywords_reports_which_keywords_fired() {
    let got = mutation_keywords_in("SELECT 1; DROP TABLE t; INSERT INTO u VALUES (1)");
    assert_eq!(got.len(), 2);
    assert!(got.contains("DROP"));
    assert!(got.contains("INSERT"));
    assert!(!got.contains("SELECT"));
}

#[test]
fn strip_keeps_only_statement_structure() {
    let stripped =
        strip_sql_comments_and_literals("SELECT 'a' -- c\n FROM t /* x */ WHERE `d` = 'x'");
    let tokens = tokenize_sql(&stripped);
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    assert_eq!(words, vec!["SELECT", "FROM", "T", "WHERE"]);
}
