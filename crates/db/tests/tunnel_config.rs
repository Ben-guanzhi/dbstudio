use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use dbstudio_db::utils::quote_string_literal;
use dbstudio_db::{Endpoint, SshTunnel};

fn config(db_type: DatabaseType, host: &str, port: u16) -> ConnectionConfig {
    let mut config = ConnectionConfig::new(db_type, "test".to_string());
    config.host = host.to_string();
    config.port = port;
    config
}

fn ssh_config() -> ConnectionConfig {
    let mut config = config(DatabaseType::MySQL, "db.internal", 3306);
    config.ssh_enabled = true;
    config.ssh_host = Some("bastion.example.com".to_string());
    config.ssh_username = Some("deploy".to_string());
    config
}

#[test]
fn endpoint_resolution_defaults_empty_host() {
    let empty = config(DatabaseType::PostgreSQL, "   ", 5432);
    assert_eq!(Endpoint::resolve(&empty), Endpoint::new("127.0.0.1", 5432));

    let explicit = config(DatabaseType::MySQL, "db.internal", 3307);
    assert_eq!(
        Endpoint::resolve(&explicit),
        Endpoint::new("db.internal", 3307)
    );
}

#[test]
fn ssh_disabled_yields_no_tunnel() {
    let config = config(DatabaseType::MySQL, "db.internal", 3306);
    assert!(SshTunnel::prepare(&config).expect("prepare").is_none());
}

#[test]
fn ssh_tunnel_requires_host_and_username() {
    let mut config = config(DatabaseType::MySQL, "db.internal", 3306);
    config.ssh_enabled = true;

    let err = SshTunnel::prepare(&config).unwrap_err().to_string();
    assert!(err.contains("no SSH host"), "unexpected error: {}", err);

    config.ssh_host = Some("bastion.example.com".to_string());
    let err = SshTunnel::prepare(&config).unwrap_err().to_string();
    assert!(err.contains("no SSH username"), "unexpected error: {}", err);
}

#[test]
fn ssh_tunnel_key_auth_requires_key_path() {
    let mut config = ssh_config();
    config.ssh_auth_type = Some("key_file".to_string());

    let err = SshTunnel::prepare(&config).unwrap_err().to_string();
    assert!(err.contains("private key path"), "unexpected error: {}", err);

    config.ssh_key_path = Some("~/.ssh/id_ed25519".to_string());
    let tunnel = SshTunnel::prepare(&config)
        .expect("prepare")
        .expect("tunnel");
    assert_eq!(tunnel.ssh().port, 22);
    assert_eq!(tunnel.target(), &Endpoint::new("db.internal", 3306));
}

#[test]
fn ssh_tunnel_open_reports_connection_failure() {
    let mut config = ssh_config();
    // Unroutable loopback port: fails fast without any network access.
    config.host = "127.0.0.1".to_string();
    config.port = 1;
    config.ssh_host = Some("127.0.0.1".to_string());
    config.ssh_port = Some(1);
    let tunnel = SshTunnel::prepare(&config)
        .expect("prepare")
        .expect("tunnel");

    let err = smol::block_on(tunnel.open(Some("secret"), None)).unwrap_err().to_string();
    assert!(
        err.contains("failed to connect to SSH server"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn sqlite_connect_ignores_ssh_settings() {
    let dir = std::env::temp_dir().join(format!("dbstudio-tunnel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ssh-ignored.db");
    let _ = std::fs::remove_file(&path);

    let mut config = config(DatabaseType::SQLite, "", 0);
    config.database = path.to_string_lossy().to_string();
    config.ssh_enabled = true;
    config.ssh_host = Some("bastion.example.com".to_string());
    config.ssh_username = Some("deploy".to_string());

    let conn = smol::block_on(dbstudio_db::connect(&config, "", None, None)).expect("sqlite connect");
    assert!(smol::block_on(conn.ping()).is_ok());
}

#[test]
fn string_literals_escape_single_quotes() {
    assert_eq!(quote_string_literal("people"), "'people'");
    assert_eq!(quote_string_literal("o'brien"), "'o''brien'");
}