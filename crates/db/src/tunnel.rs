//! Shared SSH tunnel plumbing.
//!
//! SSH settings are stored as flat fields on [`ConnectionConfig`] and turned into
//! a typed [`SshConfig`] by [`ConnectionConfig::ssh_config`]. Every network
//! driver resolves the host/port it should dial through [`Endpoint::resolve`], so
//! [`SshTunnel::open`] is the single place where the SSH client is spawned.
//!
//! # How forwarding works
//!
//! [`SshTunnel::open`] connects to the SSH server (password or key-file auth),
//! binds a local TCP listener on `127.0.0.1:<ephemeral port>`, and for every
//! accepted connection opens a `direct-tcpip` channel through the SSH session to
//! the database endpoint. The caller rewrites `config.host` / `config.port` to
//! the loopback endpoint before dialing, so drivers stay tunnel-agnostic.
//!
//! # Runtime note
//!
//! `russh` (and the `tiberius`/`oracle` drivers) are tokio-based while the rest
//! of the app runs on smol. Tunnels therefore execute on a dedicated tokio
//! runtime owned by a background thread (see [`TOKIO_RUNTIME`]); the smol side
//! only waits on a one-shot channel for the listener to come up. Forwarding
//! tasks are detached and live for the lifetime of the process — one tunnel is
//! created per [`crate::connect`] call and is never torn down explicitly.
//!
//! # Host key verification
//!
//! The server key is checked against the user's `known_hosts` file
//! (trust-on-first-use). Unknown hosts are recorded automatically and
//! subsequent connections with a different key are rejected as a possible
//! man-in-the-middle attack.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result, bail};
use dbstudio_core::models::{ConnectionConfig, SshAuthType, SshConfig};

/// Fallback host for configs that leave `host` empty.
pub const DEFAULT_HOST: &str = "127.0.0.1";

/// A TCP endpoint a driver can dial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// The endpoint a driver should dial for `config`.
    ///
    /// Falls back to [`DEFAULT_HOST`] when the config has no host. Drivers must
    /// not read `config.host` / `config.port` directly: when a tunnel is active
    /// the caller rewrites the config to the tunnel's loopback endpoint before
    /// this is reached.
    pub fn resolve(config: &ConnectionConfig) -> Self {
        let host = if config.host.trim().is_empty() {
            DEFAULT_HOST.to_string()
        } else {
            config.host.clone()
        };
        Self {
            host,
            port: config.port,
        }
    }
}

/// Dedicated tokio runtime that hosts SSH tunnel and MSSQL tasks.
///
/// Both `russh` (SSH) and `tiberius` (MSSQL) require a tokio context.
/// Keeping a single static runtime avoids per-connection runtime creation
/// and lets the two paths share the same event loop when needed.
///
/// `smol` drives the application; sqlx runs on async-std through the smol
/// bridge, so this runtime is only entered for russh/tiberius workloads.
pub(crate) static TOKIO_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        std::thread::Builder::new()
            .name("dbstudio-tokio-runtime".to_string())
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("failed to build shared tokio runtime")
            })
            .expect("failed to spawn tokio runtime thread")
            .join()
            .expect("tokio runtime thread panicked")
    })
}

/// russh client handler that verifies the server host key against the user's
/// `known_hosts` file, adopting unknown hosts on first use (TOFU).
///
/// - Known & matching key -> accept.
/// - Known & changed key  -> reject (possible MITM).
/// - Unknown host         -> record the key (TOFU) and accept.
///
/// `known_hosts` lives in the user's home directory; which path is decided by
/// `russh-keys` (Windows: `~/ssh/known_hosts`, Unix: `~/.ssh/known_hosts`).
struct KnownHostsHandler {
    host: String,
    port: u16,
}

impl KnownHostsHandler {
    fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    fn host_desc(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("[{}]:{}", self.host, self.port)
        }
    }
}

#[async_trait::async_trait]
impl russh::client::Handler for KnownHostsHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let host = self.host.trim();

        match russh_keys::check_known_hosts(host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                // Unknown host: trust-on-first-use. Record it and accept.
                match russh_keys::learn_known_hosts(host, self.port, server_public_key) {
                    Ok(()) => {
                        tracing::info!(
                            "Added {} to known_hosts (trust on first use)",
                            self.host_desc()
                        );
                        Ok(true)
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Could not record {} in known_hosts: {err}; accepting anyway",
                            self.host_desc()
                        );
                        Ok(true)
                    }
                }
            }
            Err(russh_keys::Error::KeyChanged { line }) => {
                bail!(
                    "SSH host key verification failed for {}: the key recorded at \
                     known_hosts line {line} does not match the server key (possible \
                     man-in-the-middle). Remove the stale entry and retry.",
                    self.host_desc()
                );
            }
            Err(err) => {
                tracing::warn!(
                    "Could not verify host key for {}: {err}; accepting anyway",
                    self.host_desc()
                );
                Ok(true)
            }
        }
    }
}

/// Validated SSH settings plus the database endpoint they should forward to.
#[derive(Debug, Clone)]
pub struct SshTunnel {
    ssh: SshConfig,
    target: Endpoint,
}

/// Keeps an SSH port-forward alive for as long as the process-layer handle is
/// held.
///
/// The forward loop runs on the dedicated SSH [`tokio`] runtime; only explicitly
/// aborting its task tears the loop down. The database `Connection` stores this
/// guard for its whole lifetime, so dropping the connection (disconnect, app
/// exit) also stops the tunnel instead of leaking a permanent forward.
#[derive(Debug)]
pub struct TunnelGuard {
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TunnelGuard {
    fn drop(&mut self) {
        self.handle.abort();
        tracing::debug!("SSH tunnel forward task aborted");
    }
}

impl SshTunnel {
    /// Validate the SSH fields of `config`.
    ///
    /// Returns `Ok(None)` when SSH is disabled, otherwise a tunnel descriptor or
    /// an error naming the missing field so the UI can show something
    /// actionable.
    pub fn prepare(config: &ConnectionConfig) -> Result<Option<Self>> {
        let Some(ssh) = config.ssh_config() else {
            return Ok(None);
        };

        if ssh.host.trim().is_empty() {
            bail!("SSH tunnelling is enabled but no SSH host is configured");
        }
        if ssh.username.trim().is_empty() {
            bail!("SSH tunnelling is enabled but no SSH username is configured");
        }
        if matches!(ssh.auth_type, SshAuthType::KeyFile)
            && ssh.key_path.as_deref().unwrap_or("").trim().is_empty()
        {
            bail!("SSH tunnelling uses key file auth but no private key path is configured");
        }

        Ok(Some(Self {
            ssh,
            target: Endpoint::resolve(config),
        }))
    }

    /// The SSH server the tunnel is established with.
    pub fn ssh(&self) -> &SshConfig {
        &self.ssh
    }

    /// The database endpoint the tunnel has to reach.
    pub fn target(&self) -> &Endpoint {
        &self.target
    }

    /// Establish the local port forward and return the loopback endpoint a
    /// driver should dial.
    ///
    /// Connects to the SSH server, authenticates (password or key file), binds a
    /// listener on `127.0.0.1:0` and detaches the forward loop on the SSH tokio
    /// runtime. The returned endpoint stays valid for the lifetime of the
    /// process.
    ///
    /// `ssh_password` is the password for [`SshAuthType::Password`]; it is
    /// deliberately not stored in [`SshConfig`] (which is persisted) but fetched
    /// from the OS keyring at connect time, mirroring the database password.
    ///
    /// Returns the loopback endpoint to dial **and** a [`TunnelGuard`]; the
    /// guard must be kept alive for as long as the tunnel is needed — dropping
    /// it aborts the forward task.
    pub async fn open(
        &self,
        ssh_password: Option<&str>,
        ssh_key_passphrase: Option<&str>,
    ) -> Result<(Endpoint, TunnelGuard)> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tunnel = self.clone();
        let ssh_password = ssh_password.map(|p| p.to_string());
        let ssh_key_passphrase = ssh_key_passphrase.map(|p| p.to_string());

        runtime().spawn(async move {
            let result = establish(&tunnel, ssh_password.as_deref(), ssh_key_passphrase.as_deref())
                .await;
            let _ = tx.send(result);
        });

        // Safe to block: the caller runs on the smol executor, never inside the
        // tokio runtime above.
        rx.blocking_recv()
            .context("SSH tunnel task was cancelled before it finished")?
    }
}

async fn establish(
    tunnel: &SshTunnel,
    ssh_password: Option<&str>,
    ssh_key_passphrase: Option<&str>,
) -> Result<(Endpoint, TunnelGuard)> {
    let ssh_host = tunnel.ssh.host.trim();
    let ssh_port = tunnel.ssh.port;
    let username = tunnel.ssh.username.trim();

    let config = Arc::new(russh::client::Config::default());
    let mut session =
        russh::client::connect(config, (ssh_host, ssh_port), KnownHostsHandler::new(ssh_host, ssh_port))
            .await
            .with_context(|| format!("failed to connect to SSH server {ssh_host}:{ssh_port}"))?;

    match tunnel.ssh.auth_type {
        SshAuthType::Password => {
            let password = ssh_password.unwrap_or("");
            if password.is_empty() {
                bail!(
                    "SSH authentication for user {username} on {ssh_host}:{ssh_port} requires a \
                     password: set the SSH password in the connection form"
                );
            }
            let authenticated = session
                .authenticate_password(username, password)
                .await
                .context("SSH password authentication failed")?;
            if !authenticated {
                bail!(
                    "SSH authentication failed for user {username} on {ssh_host}:{ssh_port}: \
                     check the username and password"
                );
            }
        }
        SshAuthType::KeyFile => {
            let key_path = expand_tilde(tunnel.ssh.key_path.as_deref().unwrap_or_default().trim());
            let key = russh_keys::load_secret_key(&key_path, ssh_key_passphrase)
                .with_context(|| format!("failed to load SSH private key {key_path}"))?;
            let authenticated = session
                .authenticate_publickey(username, Arc::new(key))
                .await
                .context("SSH key authentication failed")?;
            if !authenticated {
                bail!(
                    "SSH authentication failed for user {username} on {ssh_host}:{ssh_port}: \
                     the server rejected the private key"
                );
            }
        }
    }

    let session = Arc::new(session);

    let listener = tokio::net::TcpListener::bind((DEFAULT_HOST, 0))
        .await
        .context("failed to bind local listener for SSH tunnel")?;
    let local_port = listener
        .local_addr()
        .context("failed to read local listener port")?
        .port();

    let target = tunnel.target.clone();
    let target_desc = format!("{}:{}", target.host, target.port);
    let forward = async move {
        loop {
            let (mut inbound, _peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    tracing::warn!("SSH tunnel listener error: {err}");
                    break;
                }
            };

            let session = Arc::clone(&session);
            let target = target.clone();
            tokio::spawn(async move {
                let channel = session
                    .channel_open_direct_tcpip(&target.host, u32::from(target.port), "127.0.0.1", 0)
                    .await;
                let channel = match channel {
                    Ok(channel) => channel,
                    Err(err) => {
                        tracing::warn!(
                            "SSH tunnel could not open channel to {}:{}: {err}",
                            target.host,
                            target.port
                        );
                        return;
                    }
                };
                let mut outbound = channel.into_stream();
                if let Err(err) = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await {
                    tracing::debug!("SSH tunnel stream closed: {err}");
                }
            });
        }
    };
    let forward_handle = tokio::spawn(forward);

    tracing::info!(
        "SSH tunnel established: 127.0.0.1:{local_port} -> {target_desc} via {username}@{ssh_host}:{ssh_port}"
    );
    Ok((
        Endpoint::new(DEFAULT_HOST, local_port),
        TunnelGuard {
            handle: forward_handle,
        },
    ))
}

/// Expand a leading `~` in a key path to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}