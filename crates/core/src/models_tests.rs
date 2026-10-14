use super::*;
use std::str::FromStr;

#[test]
fn database_type_round_trips_through_as_str() {
    for db_type in DatabaseType::all() {
        let parsed = DatabaseType::from_str(db_type.as_str()).expect("parse");
        assert_eq!(parsed, *db_type);
    }
}

#[test]
fn database_type_parsing_accepts_aliases_case_insensitively() {
    assert_eq!(
        DatabaseType::from_str(" Postgres ").unwrap(),
        DatabaseType::PostgreSQL
    );
    assert_eq!(
        DatabaseType::from_str("PostgreSQL").unwrap(),
        DatabaseType::PostgreSQL
    );
    assert_eq!(
        DatabaseType::from_str("SQLServer").unwrap(),
        DatabaseType::MSSQL
    );
    assert_eq!(
        DatabaseType::from_str("mssql").unwrap(),
        DatabaseType::MSSQL
    );
    assert_eq!(
        DatabaseType::from_str("mariadb").unwrap(),
        DatabaseType::MySQL
    );
    assert!(DatabaseType::from_str("cassandra").is_err());
}

#[test]
fn quote_style_escapes_its_own_delimiter() {
    assert_eq!(DatabaseType::MySQL.quote_identifier("orders"), "`orders`");
    assert_eq!(
        DatabaseType::PostgreSQL.quote_identifier("orders"),
        "\"orders\""
    );
    assert_eq!(DatabaseType::MSSQL.quote_identifier("orders"), "[orders]");
    // Each style doubles the delimiter it needs to escape.
    assert_eq!(DatabaseType::SQLite.quote_identifier("a`b"), "`a``b`");
    assert_eq!(DatabaseType::Oracle.quote_identifier("a\"b"), "\"a\"\"b\"");
    assert_eq!(DatabaseType::MSSQL.quote_identifier("a]b"), "[a]]b]");
    // Brackets only escape the closer, not the opener.
    assert_eq!(DatabaseType::MSSQL.quote_identifier("a[b"), "[a[b]");
}

#[test]
fn sqlite_quotes_identifiers_like_the_rest_of_the_app() {
    // SQLite accepts `"x"`, `` `x` `` and `[x]`, so this is not a
    // compatibility fix - it is the fix for the same identifier being
    // quoted two ways depending on which panel generated the SQL.
    assert_eq!(DatabaseType::SQLite.quote_style(), QuoteStyle::Backtick);
    assert_eq!(
        DatabaseType::SQLite.quote_identifier("items"),
        DatabaseType::MySQL.quote_identifier("items")
    );
}

#[test]
fn preview_statement_uses_a_valid_row_limit_per_engine() {
    assert_eq!(
        DatabaseType::PostgreSQL.preview_statement("\"t\"", 100),
        "SELECT * FROM \"t\" LIMIT 100"
    );
    assert_eq!(
        DatabaseType::MSSQL.preview_statement("[t]", 100),
        "SELECT TOP 100 * FROM [t]"
    );
    // Oracle has no LIMIT clause; browse-table used to send one.
    assert_eq!(
        DatabaseType::Oracle.preview_statement("\"t\"", 100),
        "SELECT * FROM \"t\" FETCH FIRST 100 ROWS ONLY"
    );
    // 0 means unlimited, and no engine gets a clause.
    for db in DatabaseType::all() {
        assert_eq!(db.preview_statement("x", 0), "SELECT * FROM x", "{db:?}");
    }
}

#[test]
fn page_clause_matches_each_engines_offset_syntax() {
    assert_eq!(
        DatabaseType::SQLite.page_clause("SELECT * FROM ( a ) AS _s", 10, 20),
        "SELECT * FROM ( a ) AS _s LIMIT 10 OFFSET 20"
    );
    assert_eq!(
        DatabaseType::Oracle.page_clause("W", 10, 20),
        "W OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY"
    );
    assert_eq!(
        DatabaseType::MSSQL.page_clause("W", 10, 20),
        "W ORDER BY (SELECT NULL) OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY"
    );
}

#[test]
fn write_statements_are_exhaustively_dialect_specific() {
    assert_eq!(
        DatabaseType::SQLite.truncate_statement("`t`"),
        "DELETE FROM `t`"
    );
    assert_eq!(
        DatabaseType::PostgreSQL.truncate_statement("\"t\""),
        "TRUNCATE TABLE \"t\""
    );
    assert_eq!(
        DatabaseType::Oracle.drop_table_statement("\"t\""),
        "DROP TABLE \"t\" PURGE"
    );
    assert_eq!(
        DatabaseType::MySQL.drop_table_statement("`t`"),
        "DROP TABLE IF EXISTS `t`"
    );
    assert_eq!(
        DatabaseType::MySQL.text_cast_expr("`c`"),
        "CAST(`c` AS CHAR)"
    );
    assert_eq!(
        DatabaseType::PostgreSQL.text_cast_expr("\"c\""),
        "CAST(\"c\" AS TEXT)"
    );
    assert_eq!(
        DatabaseType::Oracle.text_cast_expr("\"c\""),
        "TO_CHAR(\"c\")"
    );
    // Oracle takes no AS for derived tables (ORA-00933); everyone else has
    // always sent it and accepts it.
    assert_eq!(
        DatabaseType::Oracle.wrap_derived_table("SELECT 1 FROM dual", "dbstudio_sub"),
        "SELECT * FROM ( SELECT 1 FROM dual ) dbstudio_sub"
    );
    assert_eq!(
        DatabaseType::MSSQL.wrap_derived_table("SELECT 1", "dbstudio_sub"),
        "SELECT * FROM ( SELECT 1 ) AS dbstudio_sub"
    );
}

#[test]
fn ssl_mode_param_only_exists_for_the_two_url_drivers() {
    use SslMode::*;
    assert_eq!(
        DatabaseType::MySQL.ssl_mode_param(Require),
        Some(("ssl-mode", "required"))
    );
    assert_eq!(
        DatabaseType::PostgreSQL.ssl_mode_param(VerifyFull),
        Some(("sslmode", "verify-full"))
    );
    // Not encrypted at the URL level, or no knob at all.
    assert_eq!(DatabaseType::MySQL.ssl_mode_param(Disable), None);
    for db in [
        DatabaseType::SQLite,
        DatabaseType::MSSQL,
        DatabaseType::Oracle,
    ] {
        assert_eq!(db.ssl_mode_param(VerifyCa), None, "{db:?}");
    }
    // Only SQLite has no endpoint to tunnel.
    assert!(DatabaseType::SQLite.is_file_based());
    let file_based: Vec<DatabaseType> = DatabaseType::all()
        .iter()
        .filter(|d| d.is_file_based())
        .copied()
        .collect();
    assert_eq!(file_based, vec![DatabaseType::SQLite]);
}

#[test]
fn ssh_auth_type_from_wire_falls_back_to_password() {
    assert_eq!(SshAuthType::from_wire("key_file"), SshAuthType::KeyFile);
    assert_eq!(SshAuthType::from_wire("KeyFile"), SshAuthType::KeyFile);
    assert_eq!(SshAuthType::from_wire("password"), SshAuthType::Password);
    assert_eq!(SshAuthType::from_wire(""), SshAuthType::Password);
}

#[test]
fn ssh_config_is_none_when_disabled() {
    let config = ConnectionConfig::new(DatabaseType::MySQL, "test".to_string());
    assert!(config.ssh_config().is_none());
}

#[test]
fn ssh_config_applies_defaults() {
    let mut config = ConnectionConfig::new(DatabaseType::MySQL, "test".to_string());
    config.ssh_enabled = true;

    let ssh = config.ssh_config().expect("ssh config");
    assert_eq!(ssh.port, 22);
    assert_eq!(ssh.host, "");
    assert!(matches!(ssh.auth_type, SshAuthType::Password));

    config.ssh_host = Some("bastion".to_string());
    config.ssh_auth_type = Some("key_file".to_string());
    let ssh = config.ssh_config().expect("ssh config");
    assert_eq!(ssh.port, 22);
    assert!(matches!(ssh.auth_type, SshAuthType::KeyFile));
}

#[test]
fn extra_params_round_trip() {
    let mut config = ConnectionConfig::new(DatabaseType::MySQL, "test".to_string());
    config.set_extra_params(HashMap::from([(
        "sslmode".to_string(),
        "require".to_string(),
    )]));

    assert_eq!(
        config.extra_params_map().get("sslmode").map(String::as_str),
        Some("require")
    );

    config.set_extra_params(HashMap::new());
    assert!(config.extra_params.is_none());
    assert!(config.extra_params_map().is_empty());
}

#[test]
fn ssl_mode_wire_and_display() {
    for mode in [
        SslMode::Disable,
        SslMode::Require,
        SslMode::VerifyCa,
        SslMode::VerifyFull,
    ] {
        assert_eq!(SslMode::from_wire(mode.as_str()), mode);
    }
    // Lenient parsing for spellings found in the wild.
    assert_eq!(SslMode::from_wire("prefer"), SslMode::Require);
    assert_eq!(SslMode::from_wire("verify-identity"), SslMode::VerifyFull);
    assert_eq!(SslMode::from_wire(""), SslMode::Disable);
    assert!(!SslMode::Disable.is_encrypted());
    assert!(SslMode::Require.is_encrypted());
}

#[test]
fn new_connection_defaults_to_disabled_ssl() {
    let config = ConnectionConfig::new(DatabaseType::PostgreSQL, "test".to_string());
    assert_eq!(config.ssl_mode, SslMode::Disable);
}
