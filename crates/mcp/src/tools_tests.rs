use super::*;

/// Regression tests for the safe-mode gate. Each `is_write_statement` case here
/// was reachable before the gate was moved to the shared classifier: the old
/// check looked only at the first whitespace-separated word, so any of these
/// passed straight through.
#[test]
fn gate_blocks_the_bypasses_the_first_keyword_check_missed() {
    // A read prefix legitimising a trailing write.
    assert!(is_write_statement("SELECT 1; DROP TABLE users"));
    assert!(is_write_statement("SELECT 1; DELETE FROM users"));
    assert!(is_write_statement("EXPLAIN SELECT 1; UPDATE t SET a = 1"));
    // A leading comment hiding the verb.
    assert!(is_write_statement("-- housekeeping\nDELETE FROM users"));
    assert!(is_write_statement("/* note */ DELETE FROM users"));
    // A data-modifying CTE under an outer SELECT.
    assert!(is_write_statement(
        "WITH d AS (DELETE FROM users RETURNING *) SELECT * FROM d"
    ));
    assert!(is_write_statement(
        "WITH x AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM x"
    ));
}

#[test]
fn gate_allows_genuine_reads() {
    for sql in [
        "SELECT 1",
        "select id, name from users where active = 1",
        "SELECT * FROM t FOR UPDATE",
        "PRAGMA table_info('users')",
        "SHOW TABLES",
        "EXPLAIN SELECT 1",
        "SELECT 'DROP TABLE t' AS note",
        "SELECT deleted FROM t",
        "",
    ] {
        assert!(!is_write_statement(sql), "expected read-only for {sql:?}");
    }
}

#[test]
fn gate_blocks_writes_at_the_front() {
    for sql in [
        "INSERT INTO t VALUES (1)",
        "UPDATE t SET a = 1 WHERE id = 2",
        "DELETE FROM t",
        "DROP TABLE t",
        "CREATE TABLE t (a INT)",
        "ALTER TABLE t ADD c INT",
        "TRUNCATE TABLE t",
        "GRANT ALL ON t TO u",
        "CALL wipe_all()",
    ] {
        assert!(is_write_statement(sql), "expected mutation for {sql:?}");
    }
}

#[test]
fn gate_blocks_unknown_statements_too() {
    // The MCP is LLM-facing: when the classifier cannot decide, refuse rather
    // than assume read-only. `switch_database` exists for the USE case.
    assert!(is_write_statement("USE mydb"));
    assert!(is_write_statement("SET SESSION max_connections = 10"));
}

#[test]
fn mutation_reason_names_the_keywords_that_fired() {
    assert_eq!(
        mutation_reason("SELECT 1; DROP TABLE t; INSERT INTO u VALUES (1)"),
        "DROP, INSERT"
    );
    // No write keyword survived the strip: report the fallback phrase.
    assert_eq!(mutation_reason("USE mydb"), "an unrecognised statement");
}
