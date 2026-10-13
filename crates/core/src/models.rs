use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Parse a datetime string with common formats, falling back to current time.
pub fn parse_datetime(s: &str) -> chrono::NaiveDateTime {
    const FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
    ];

    for format in FORMATS {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, format) {
            return dt;
        }
    }

    chrono::Utc::now().naive_utc()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    SQLite,
    MySQL,
    PostgreSQL,
    MSSQL,
    Oracle,
}

/// Identifier quoting style an engine accepts.
///
/// The three spellings are not interchangeable across engines, and the escaping
/// is part of the style (each one doubles its own delimiter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    Backtick,
    DoubleQuote,
    Bracket,
}

impl QuoteStyle {
    pub fn quote(&self, value: &str) -> String {
        // `escaped` is the character that doubles itself: for the bracket style
        // that is only the closing bracket, which is what SQL Server expects.
        let (open, close, escaped) = match self {
            QuoteStyle::Backtick => ('`', '`', '`'),
            QuoteStyle::DoubleQuote => ('"', '"', '"'),
            QuoteStyle::Bracket => ('[', ']', ']'),
        };
        let mut out = String::with_capacity(value.len() + 2);
        out.push(open);
        for c in value.chars() {
            if c == escaped {
                out.push(c);
            }
            out.push(c);
        }
        out.push(close);
        out
    }
}

impl DatabaseType {
    pub fn all() -> &'static [DatabaseType] {
        &[
            DatabaseType::SQLite,
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
            DatabaseType::MSSQL,
            DatabaseType::Oracle,
        ]
    }

    pub fn default_port(&self) -> u16 {
        match self {
            DatabaseType::SQLite => 0,
            DatabaseType::MySQL => 3306,
            DatabaseType::PostgreSQL => 5432,
            DatabaseType::MSSQL => 1433,
            DatabaseType::Oracle => 1521,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            DatabaseType::SQLite => "SQLite",
            DatabaseType::MySQL => "MySQL",
            DatabaseType::PostgreSQL => "PostgreSQL",
            DatabaseType::MSSQL => "SQL Server",
            DatabaseType::Oracle => "Oracle",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DatabaseType::SQLite => "sqlite",
            DatabaseType::MySQL => "mysql",
            DatabaseType::PostgreSQL => "postgresql",
            DatabaseType::MSSQL => "mssql",
            DatabaseType::Oracle => "oracle",
        }
    }

    /// The engine's identifier quoting style.
    ///
    /// SQLite is deliberately `Backtick`, not `DoubleQuote`: it accepts all
    /// three spellings, and both the export path and the SQLite driver's own
    /// catalog queries already quote with backticks. Keeping the UI on double
    /// quotes meant the same identifier was quoted two different ways depending
    /// on which panel generated the SQL.
    pub fn quote_style(&self) -> QuoteStyle {
        match self {
            DatabaseType::SQLite | DatabaseType::MySQL => QuoteStyle::Backtick,
            DatabaseType::PostgreSQL | DatabaseType::Oracle => QuoteStyle::DoubleQuote,
            DatabaseType::MSSQL => QuoteStyle::Bracket,
        }
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        self.quote_style().quote(value)
    }

    /// Does this engine have no network endpoint (so an SSH tunnel is moot)?
    pub fn is_file_based(&self) -> bool {
        matches!(self, DatabaseType::SQLite)
    }

    /// `SELECT` of at most `max_rows` rows from an already-quoted table
    /// reference. `max_rows == 0` means no limit.
    ///
    /// This is the offset-less row-limiting dialect; windowed paging is
    /// [`DatabaseType::page_clause`]. Oracle has no `LIMIT`, which is why the
    /// browse-table action used to fail there.
    pub fn preview_statement(&self, table_ref: &str, max_rows: usize) -> String {
        match self {
            DatabaseType::MSSQL => {
                if max_rows > 0 {
                    format!("SELECT TOP {max_rows} * FROM {table_ref}")
                } else {
                    format!("SELECT * FROM {table_ref}")
                }
            }
            DatabaseType::Oracle if max_rows > 0 => {
                format!("SELECT * FROM {table_ref} FETCH FIRST {max_rows} ROWS ONLY")
            }
            _ if max_rows > 0 => format!("SELECT * FROM {table_ref} LIMIT {max_rows}"),
            _ => format!("SELECT * FROM {table_ref}"),
        }
    }

    /// Windowing clause for an already-wrapped query, in this engine's dialect.
    pub fn page_clause(&self, wrapped: &str, limit: usize, offset: usize) -> String {
        match self {
            // MSSQL requires an ORDER BY before OFFSET/FETCH; `(SELECT NULL)` is
            // the idiomatic "any order" when the caller has none.
            DatabaseType::MSSQL => format!(
                "{wrapped} ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"
            ),
            DatabaseType::Oracle => {
                format!("{wrapped} OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
            }
            DatabaseType::SQLite | DatabaseType::MySQL | DatabaseType::PostgreSQL => {
                format!("{wrapped} LIMIT {limit} OFFSET {offset}")
            }
        }
    }

    /// Expression coercing an already-quoted column to text for `LIKE`/`LOWER`.
    ///
    /// The arms are exhaustive on purpose. `TEXT` is what this app has always
    /// sent for SQLite, MSSQL and PostgreSQL. The Oracle arm was verified
    /// against a live 12.2 EE server: `CAST(x AS TEXT)` fails there with
    /// ORA-00902 (invalid datatype) on every statement, while `TO_CHAR`
    /// accepts VARCHAR2/NVARCHAR2/NCHAR/NUMBER/DATE/TIMESTAMP and CLOB.
    pub fn text_cast_expr(&self, quoted: &str) -> String {
        match self {
            DatabaseType::MySQL => format!("CAST({quoted} AS CHAR)"),
            DatabaseType::Oracle => format!("TO_CHAR({quoted})"),
            DatabaseType::SQLite
            | DatabaseType::PostgreSQL
            | DatabaseType::MSSQL => format!("CAST({quoted} AS TEXT)"),
        }
    }

    /// Wrap an arbitrary query as an aliased derived table.
    ///
    /// Oracle rejects both `AS` for table aliases (ORA-00933) and unquoted
    /// identifiers starting with `_` (ORA-00911), verified on a live 12.2 EE
    /// server, so the wrapper omits `AS` there. Pass an `alias` that is valid
    /// unquoted in every engine (letter-initial) and dialect-specific keyword
    /// handling stays inside this one method.
    pub fn wrap_derived_table(&self, inner: &str, alias: &str) -> String {
        match self {
            DatabaseType::Oracle => format!("SELECT * FROM ( {inner} ) {alias}"),
            DatabaseType::SQLite
            | DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::MSSQL => format!("SELECT * FROM ( {inner} ) AS {alias}"),
        }
    }

    /// Statement that empties a table, in a form the engine accepts.
    pub fn truncate_statement(&self, qualified: &str) -> String {
        match self {
            DatabaseType::SQLite => format!("DELETE FROM {qualified}"),
            DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::MSSQL
            | DatabaseType::Oracle => format!("TRUNCATE TABLE {qualified}"),
        }
    }

    /// `DROP TABLE` for an already-qualified identifier.
    pub fn drop_table_statement(&self, qualified: &str) -> String {
        match self {
            // Without `PURGE` Oracle drops into the recycle bin and a following
            // `CREATE TABLE` of the same name fails.
            DatabaseType::Oracle => format!("DROP TABLE {qualified} PURGE"),
            DatabaseType::SQLite
            | DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::MSSQL => format!("DROP TABLE IF EXISTS {qualified}"),
        }
    }

    /// URL-level TLS knob for a given [`SslMode`], or `None` when the engine has
    /// no such parameter or the spelling differs from the mode named here.
    ///
    /// Each driver expects its own vocabulary: MySQL `ssl-mode=required`,
    /// PostgreSQL `sslmode=require`.
    pub fn ssl_mode_param(&self, mode: SslMode) -> Option<(&'static str, &'static str)> {
        use SslMode::*;
        match (self, mode) {
            (DatabaseType::MySQL, Require) => Some(("ssl-mode", "required")),
            (DatabaseType::MySQL, VerifyCa) => Some(("ssl-mode", "verify_ca")),
            (DatabaseType::MySQL, VerifyFull) => Some(("ssl-mode", "verify_identity")),
            (DatabaseType::PostgreSQL, Require) => Some(("sslmode", "require")),
            (DatabaseType::PostgreSQL, VerifyCa) => Some(("sslmode", "verify-ca")),
            (DatabaseType::PostgreSQL, VerifyFull) => Some(("sslmode", "verify-full")),
            _ => None,
        }
    }
}

/// Error returned when a database type string cannot be recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownDatabaseType(pub String);

impl fmt::Display for UnknownDatabaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown database type: {}", self.0)
    }
}

impl std::error::Error for UnknownDatabaseType {}

/// Parse a database type from its canonical or common alias spelling
/// (case-insensitive). This is the counterpart of [`DatabaseType::as_str`].
impl std::str::FromStr for DatabaseType {
    type Err = UnknownDatabaseType;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_lowercase();
        match normalized.as_str() {
            "sqlite" | "sqlite3" => Ok(DatabaseType::SQLite),
            "mysql" | "mariadb" => Ok(DatabaseType::MySQL),
            "postgresql" | "postgres" | "pg" => Ok(DatabaseType::PostgreSQL),
            "mssql" | "sqlserver" | "sql_server" | "microsoftsqlserver" => Ok(DatabaseType::MSSQL),
            "oracle" | "oracledb" => Ok(DatabaseType::Oracle),
            _ => Err(UnknownDatabaseType(value.to_string())),
        }
    }
}

impl fmt::Display for DatabaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: SshAuthType,
    pub key_path: Option<String>,
    /// Live SSH password for password authentication. Intentionally not
    /// persisted anywhere; the OS keyring is the source of truth and this field
    /// carries the value only between the connection form and the connection
    /// attempt.
    pub ssh_password: Option<String>,
    /// Live passphrase for password-encrypted private keys (key file auth).
    /// Same lifetime rules as [`SshConfig::ssh_password`]: keyring-only, never
    /// persisted in the connection row.
    pub ssh_key_passphrase: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthType {
    #[default]
    Password,
    KeyFile,
}

impl SshAuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SshAuthType::Password => "password",
            SshAuthType::KeyFile => "key_file",
        }
    }

    /// Parse the persisted/wire spelling of the SSH auth type.
    ///
    /// Unrecognized values fall back to [`SshAuthType::Password`], matching the
    /// behaviour of existing stored connections.
    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "key_file" | "keyfile" | "key" => SshAuthType::KeyFile,
            _ => SshAuthType::default(),
        }
    }
}

/// The environment a connection targets.
///
/// Used to drive the Safe Mode guards in the UI: connections marked
/// `Production` (and, optionally, `Staging`) get extra confirmation prompts
/// before destructive statements are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Dev,
    Staging,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Dev => "dev",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }

    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "staging" => Environment::Staging,
            "production" | "prod" => Environment::Production,
            _ => Environment::Dev,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Environment::Dev => "Development",
            Environment::Staging => "Staging",
            Environment::Production => "Production",
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }
}

/// TLS/SSL mode for encrypted connections.
///
/// This is a first-class connection option (mirrored on the connection form),
/// distinct from the free-form `extra_params` passthrough. `Require` and above
/// force encryption; the `Verify*` modes also validate the server certificate
/// (subject/CA) where the driver accepts such parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SslMode {
    #[default]
    Disable,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify_ca",
            SslMode::VerifyFull => "verify_full",
        }
    }

    /// Parse the persisted/wire spelling of the SSL mode.
    ///
    /// Unknown or empty values fall back to [`SslMode::Disable`], keeping old
    /// stored connections (which predate the column) unencrypted.
    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "require" | "required" | "prefer" => SslMode::Require,
            "verify_ca" | "verify-ca" => SslMode::VerifyCa,
            "verify_full" | "verify-full" | "verify_identity" | "verify-identity" | "full" => {
                SslMode::VerifyFull
            }
            _ => SslMode::Disable,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SslMode::Disable => "Disable",
            SslMode::Require => "Require",
            SslMode::VerifyCa => "Verify CA",
            SslMode::VerifyFull => "Verify Full",
        }
    }

    pub fn is_encrypted(&self) -> bool {
        !matches!(self, SslMode::Disable)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub db_type: DatabaseType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub ssh_enabled: bool,
    #[serde(default)]
    pub ssh_host: Option<String>,
    #[serde(default)]
    pub ssh_port: Option<u16>,
    #[serde(default)]
    pub ssh_username: Option<String>,
    #[serde(default)]
    pub ssh_auth_type: Option<String>,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    #[serde(default)]
    pub extra_params: Option<String>,
    /// Name of a loaded driver plugin to route this connection through. When
    /// set, [`dbstudio_db::connect`] opens the connection via the plugin
    /// instead of a built-in engine driver.
    #[serde(default)]
    pub plugin_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl ConnectionConfig {
    pub fn new(db_type: DatabaseType, name: String) -> Self {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            db_type,
            host: "localhost".to_string(),
            port: db_type.default_port(),
            database: String::new(),
            username: String::new(),
            color: None,
            environment: Environment::Dev,
            ssl_mode: SslMode::Disable,
            group: None,
            tags: Vec::new(),
            ssh_enabled: false,
            ssh_host: None,
            ssh_port: None,
            ssh_username: None,
            ssh_auth_type: None,
            ssh_key_path: None,
            extra_params: None,
            plugin_name: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn display_title(&self) -> String {
        if self.database.is_empty() {
            format!("{}@{}", self.username, self.host)
        } else {
            format!("{}@{}/{}", self.username, self.host, self.database)
        }
    }

    pub fn connection_summary(&self) -> String {
        match self.db_type {
            DatabaseType::SQLite => self.database.clone(),
            _ => {
                let port_suffix = if self.port != self.db_type.default_port() {
                    format!(":{}", self.port)
                } else {
                    String::new()
                };
                format!(
                    "{}{}{}",
                    self.host,
                    port_suffix,
                    if self.database.is_empty() {
                        String::new()
                    } else {
                        format!("/{}", self.database)
                    }
                )
            }
        }
    }

    /// Build a typed SshConfig from flat SSH fields.
    pub fn ssh_config(&self) -> Option<SshConfig> {
        if !self.ssh_enabled {
            return None;
        }
        let auth_type = SshAuthType::from_wire(self.ssh_auth_type.as_deref().unwrap_or(""));
        Some(SshConfig {
            enabled: true,
            host: self.ssh_host.clone().unwrap_or_default(),
            port: self.ssh_port.unwrap_or(22),
            username: self.ssh_username.clone().unwrap_or_default(),
            auth_type,
            key_path: self.ssh_key_path.clone(),
            ssh_password: None,
            ssh_key_passphrase: None,
        })
    }

    /// Parse extra_params JSON string into a HashMap.
    pub fn extra_params_map(&self) -> HashMap<String, String> {
        self.extra_params
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }

    /// Set extra_params from a HashMap.
    pub fn set_extra_params(&mut self, params: HashMap<String, String>) {
        self.extra_params = if params.is_empty() {
            None
        } else {
            serde_json::to_string(&params).ok()
        };
    }
}

// Keep type aliases for backward compatibility
pub type DbConnectionConfig = ConnectionConfig;
#[cfg(test)]
mod tests {
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
}
