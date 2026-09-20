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
            "verify_full" | "verify-full" | "verify_identity" | "verify-identity" | "full" => SslMode::VerifyFull,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
                format!("{}{}{}",
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
        assert_eq!(DatabaseType::from_str(" Postgres ").unwrap(), DatabaseType::PostgreSQL);
        assert_eq!(DatabaseType::from_str("PostgreSQL").unwrap(), DatabaseType::PostgreSQL);
        assert_eq!(DatabaseType::from_str("SQLServer").unwrap(), DatabaseType::MSSQL);
        assert_eq!(DatabaseType::from_str("mssql").unwrap(), DatabaseType::MSSQL);
        assert_eq!(DatabaseType::from_str("mariadb").unwrap(), DatabaseType::MySQL);
        assert!(DatabaseType::from_str("cassandra").is_err());
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
        config.set_extra_params(HashMap::from([("sslmode".to_string(), "require".to_string())]));

        assert_eq!(config.extra_params_map().get("sslmode").map(String::as_str), Some("require"));

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
        assert!(SslMode::Disable.is_encrypted() == false);
        assert!(SslMode::Require.is_encrypted());
    }

    #[test]
    fn new_connection_defaults_to_disabled_ssl() {
        let config = ConnectionConfig::new(DatabaseType::PostgreSQL, "test".to_string());
        assert_eq!(config.ssl_mode, SslMode::Disable);
    }
}
