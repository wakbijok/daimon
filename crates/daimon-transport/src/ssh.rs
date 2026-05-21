//! Real SSH transport via `russh` (D7, D14, D15).
//!
//! Supports both `Credential::SshKey` (public-key auth) and
//! `Credential::SshPassword` (password auth). Only `Op::ShellCommand` is
//! valid — other ops return `TransportError::OpMismatch`.
//!
//! Host-key policy: defaults to `KnownHosts(/var/lib/daimon/known_hosts)`.
//! Uses `russh-keys::check_known_hosts_path` for verification (battle-tested
//! parser handling hashed hostnames + bracketed ports + comma-separated
//! host lists). `AcceptAny` is available as an explicit opt-in constructor
//! for first-bootstrap scenarios; it logs the server fingerprint at WARN
//! every time it's used.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use daimon_vault::Credential;
use russh::client::{self, Handle, Handler};
use russh::keys::{check_known_hosts_path, decode_secret_key, key};
use russh::{ChannelMsg, Disconnect};
use tokio::time::timeout;
use tracing::{debug, instrument, warn};

use crate::op::{Op, OpResult, TransportError};
use crate::transport::{Transport, TransportTarget};

const TRANSPORT_ID: &str = "ssh";
const DEFAULT_KNOWN_HOSTS_PATH: &str = "/var/lib/daimon/known_hosts";

/// Policy for verifying the server's host key on connect.
#[derive(Debug, Clone)]
pub enum HostKeyPolicy {
    /// Verify the server's key against an OpenSSH-format `known_hosts` file
    /// (default `/var/lib/daimon/known_hosts`). Uses `russh-keys`'s built-in
    /// parser — handles hashed hostnames, bracketed ports, comma-separated
    /// host lists. Rejects connection if the host:port pair has no matching
    /// entry. Production default.
    KnownHosts {
        path: PathBuf,
        host: String,
        port: u16,
    },
    /// Accept any server key. The fingerprint is logged at WARN level.
    /// **Security downgrade** — only acceptable for first-bootstrap
    /// scenarios. Use [`SshTransport::with_accept_any`] to opt in explicitly.
    AcceptAny,
}

/// SSH transport. Construct via [`SshTransport::new`] (production default,
/// known_hosts at the default path) or [`SshTransport::with_known_hosts_path`]
/// (custom known_hosts file) or [`SshTransport::with_accept_any`] (explicit
/// security downgrade for bootstrap).
#[derive(Clone)]
pub struct SshTransport {
    config: Arc<client::Config>,
    known_hosts_path: Option<PathBuf>,
    accept_any: bool,
}

impl Default for SshTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl SshTransport {
    /// Production default: KnownHosts at `/var/lib/daimon/known_hosts`.
    pub fn new() -> Self {
        Self::with_known_hosts_path(PathBuf::from(DEFAULT_KNOWN_HOSTS_PATH))
    }

    /// KnownHosts with a custom path.
    pub fn with_known_hosts_path(path: PathBuf) -> Self {
        Self {
            config: Arc::new(make_config()),
            known_hosts_path: Some(path),
            accept_any: false,
        }
    }

    /// **SECURITY DOWNGRADE** — accept any server key. Use only for first-
    /// bootstrap scenarios where you intend to record the fingerprint and
    /// transition to KnownHosts immediately after. Emits a WARN log every
    /// time it accepts a key.
    pub fn with_accept_any() -> Self {
        Self {
            config: Arc::new(make_config()),
            known_hosts_path: None,
            accept_any: true,
        }
    }

    fn policy_for(&self, host: &str, port: u16) -> HostKeyPolicy {
        if self.accept_any {
            HostKeyPolicy::AcceptAny
        } else {
            HostKeyPolicy::KnownHosts {
                path: self
                    .known_hosts_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_KNOWN_HOSTS_PATH)),
                host: host.to_owned(),
                port,
            }
        }
    }

    #[instrument(skip(self, cred), fields(host = %target.host, port = target.port))]
    async fn exec_shell(
        &self,
        target: &TransportTarget,
        command: &str,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        let handler = ClientHandler {
            policy: self.policy_for(&target.host, target.port),
        };

        // Connect.
        let mut handle: Handle<ClientHandler> = client::connect(
            self.config.clone(),
            (target.host.as_str(), target.port),
            handler,
        )
        .await
        .map_err(|e| TransportError::Connect(format!("ssh connect: {e}")))?;

        // Authenticate.
        match cred {
            Credential::SshKey {
                username,
                private_key_pem,
                passphrase,
            } => {
                let keypair = decode_secret_key(private_key_pem, passphrase.as_deref())
                    .map_err(|e| TransportError::Auth(format!("decode key: {e}")))?;
                let ok = handle
                    .authenticate_publickey(username.as_str(), Arc::new(keypair))
                    .await
                    .map_err(|e| TransportError::Auth(format!("publickey auth: {e}")))?;
                if !ok {
                    return Err(TransportError::Auth(
                        "public-key auth rejected by server".into(),
                    ));
                }
            }
            Credential::SshPassword { username, password } => {
                let ok = handle
                    .authenticate_password(username.as_str(), password.as_str())
                    .await
                    .map_err(|e| TransportError::Auth(format!("password auth: {e}")))?;
                if !ok {
                    return Err(TransportError::Auth(
                        "password auth rejected by server".into(),
                    ));
                }
            }
            other => {
                return Err(TransportError::OpMismatch {
                    op: "ssh_shell_command".into(),
                    transport: format!("credential_kind={:?}", other.kind()),
                });
            }
        }

        // Open a session channel + exec.
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| TransportError::Io(format!("open channel: {e}")))?;

        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| TransportError::Io(format!("exec: {e}")))?;

        // Drain the channel.
        let mut stdout = Vec::<u8>::new();
        let mut stderr = Vec::<u8>::new();
        let mut exit_status: i32 = 0;
        let mut got_exit = false;

        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        stderr.extend_from_slice(data);
                    } else {
                        debug!(ext, len = data.len(), "ssh: dropping unknown extended data");
                    }
                }
                ChannelMsg::ExitStatus { exit_status: s } => {
                    exit_status = s as i32;
                    got_exit = true;
                }
                ChannelMsg::Eof => {
                    // Server done sending output; wait for Close.
                }
                ChannelMsg::Close => {
                    break;
                }
                _ => {
                    // ExitSignal / WindowAdjusted / etc. — ignored for shell exec.
                }
            }
        }

        // Politely tear down.
        let _ = handle
            .disconnect(Disconnect::ByApplication, "session complete", "")
            .await;

        if !got_exit {
            warn!("ssh: channel closed without ExitStatus, returning exit_status=0");
        }

        Ok(OpResult::ShellCommand {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            exit_status,
        })
    }
}

#[async_trait]
impl Transport for SshTransport {
    fn id(&self) -> &str {
        TRANSPORT_ID
    }

    async fn execute(
        &self,
        target: &TransportTarget,
        op: &Op,
        cred: &Credential,
    ) -> Result<OpResult, TransportError> {
        match op {
            Op::ShellCommand {
                command,
                timeout_secs,
            } => {
                let dur = Duration::from_secs(*timeout_secs as u64);
                timeout(dur, self.exec_shell(target, command, cred))
                    .await
                    .map_err(|_| TransportError::Timeout(*timeout_secs))?
            }
            other => Err(TransportError::OpMismatch {
                op: op_name(other).into(),
                transport: TRANSPORT_ID.into(),
            }),
        }
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::ShellCommand { .. } => "shell_command",
        Op::Http { .. } => "http",
        Op::SnmpGet { .. } => "snmp_get",
        Op::SnmpSet { .. } => "snmp_set",
        Op::SnmpWalk { .. } => "snmp_walk",
    }
}

fn make_config() -> client::Config {
    client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        ..Default::default()
    }
}

/// russh `Handler` for the client side.
struct ClientHandler {
    policy: HostKeyPolicy,
}

#[async_trait]
impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match &self.policy {
            HostKeyPolicy::KnownHosts { path, host, port } => {
                match check_known_hosts_path(host, *port, server_public_key, path) {
                    Ok(true) => {
                        debug!(host = %host, port = *port, "ssh: known_hosts match");
                        Ok(true)
                    }
                    Ok(false) => {
                        let fp = server_public_key.fingerprint();
                        warn!(
                            host = %host,
                            port = *port,
                            server_fingerprint = %fp,
                            known_hosts = %path.display(),
                            "ssh: known_hosts verification FAILED — entry mismatch. Refusing connection."
                        );
                        Ok(false)
                    }
                    Err(russh::keys::Error::KeyChanged { line }) => {
                        let fp = server_public_key.fingerprint();
                        warn!(
                            host = %host,
                            port = *port,
                            server_fingerprint = %fp,
                            known_hosts = %path.display(),
                            existing_line = line,
                            "ssh: KEY CHANGED — existing known_hosts entry has a different key. Refusing connection."
                        );
                        Ok(false)
                    }
                    Err(e) => {
                        warn!(
                            host = %host,
                            port = *port,
                            known_hosts = %path.display(),
                            error = %e,
                            "ssh: known_hosts read/parse error — refusing connection",
                        );
                        Ok(false)
                    }
                }
            }
            HostKeyPolicy::AcceptAny => {
                let fp = server_public_key.fingerprint();
                warn!(
                    server_fingerprint = %fp,
                    "ssh: AcceptAny policy in effect — accepting server key without verification. \
                     This is a security downgrade; switch to KnownHosts before production use."
                );
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Op;
    use std::collections::BTreeMap;

    fn fake_cred() -> Credential {
        Credential::SshPassword {
            username: "u".into(),
            password: "p".into(),
        }
    }

    fn target(host: &str, port: u16) -> TransportTarget {
        TransportTarget {
            host: host.into(),
            port,
        }
    }

    #[tokio::test]
    async fn id_is_ssh() {
        let t = SshTransport::new();
        assert_eq!(t.id(), "ssh");
    }

    #[tokio::test]
    async fn default_constructor_uses_known_hosts_policy() {
        let t = SshTransport::new();
        let policy = t.policy_for("h", 22);
        assert!(matches!(policy, HostKeyPolicy::KnownHosts { .. }));
    }

    #[tokio::test]
    async fn accept_any_constructor_opt_in_only() {
        let t = SshTransport::with_accept_any();
        let policy = t.policy_for("h", 22);
        assert!(matches!(policy, HostKeyPolicy::AcceptAny));
    }

    #[tokio::test]
    async fn http_op_returns_mismatch() {
        let t = SshTransport::with_accept_any();
        let err = t
            .execute(
                &target("127.0.0.1", 22),
                &Op::Http {
                    method: crate::op::HttpMethod::Get,
                    path: "/".into(),
                    headers: BTreeMap::new(),
                    body: None,
                },
                &fake_cred(),
            )
            .await
            .unwrap_err();
        match err {
            TransportError::OpMismatch { op, transport } => {
                assert_eq!(op, "http");
                assert_eq!(transport, "ssh");
            }
            other => panic!("expected OpMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn snmp_op_returns_mismatch() {
        let t = SshTransport::with_accept_any();
        let err = t
            .execute(
                &target("127.0.0.1", 22),
                &Op::SnmpGet { oid: "1.3.6.1".into() },
                &fake_cred(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::OpMismatch { .. }));
    }

    #[tokio::test]
    async fn connect_to_closed_port_returns_connect_error() {
        let t = SshTransport::with_accept_any();
        let err = t
            .execute(
                &target("127.0.0.1", 1),
                &Op::ShellCommand {
                    command: "true".into(),
                    timeout_secs: 2,
                },
                &fake_cred(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            TransportError::Connect(_) | TransportError::Io(_) | TransportError::Timeout(_)
        ));
    }

    // Live SSH integration test — env-gated, skipped in normal runs.
    //
    //   DAIMON_SSH_TEST_HOST=hostname.example
    //   DAIMON_SSH_TEST_USER=arif
    //   DAIMON_SSH_TEST_KEY=/path/to/private_key   (optional, default: $HOME/.ssh/arif)
    //   DAIMON_SSH_TEST_PORT=22                      (optional)
    //   DAIMON_SSH_TEST_PASSPHRASE=...               (optional)
    //   DAIMON_SSH_TEST_ACCEPT_ANY=1                 (optional, skips known_hosts check)
    //
    //   cargo test -p daimon-transport --features for-broker -- --ignored
    #[tokio::test]
    #[ignore = "requires real SSH endpoint via DAIMON_SSH_TEST_HOST"]
    async fn live_ssh_runs_uname() {
        let Ok(host) = std::env::var("DAIMON_SSH_TEST_HOST") else {
            eprintln!("skipping: DAIMON_SSH_TEST_HOST not set");
            return;
        };
        let user = std::env::var("DAIMON_SSH_TEST_USER")
            .unwrap_or_else(|_| std::env::var("USER").unwrap_or_else(|_| "root".into()));
        let port: u16 = std::env::var("DAIMON_SSH_TEST_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(22);
        let key_path = std::env::var("DAIMON_SSH_TEST_KEY").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME unset");
            format!("{home}/.ssh/arif")
        });
        let passphrase = std::env::var("DAIMON_SSH_TEST_PASSPHRASE").ok();
        let accept_any = std::env::var("DAIMON_SSH_TEST_ACCEPT_ANY").is_ok();

        let pem = std::fs::read_to_string(&key_path)
            .unwrap_or_else(|e| panic!("read key {key_path}: {e}"));

        let cred = Credential::SshKey {
            username: user,
            private_key_pem: pem,
            passphrase,
        };

        let transport = if accept_any {
            SshTransport::with_accept_any()
        } else {
            SshTransport::new()
        };

        let result = transport
            .execute(
                &target(&host, port),
                &Op::ShellCommand {
                    command: "uname -a".into(),
                    timeout_secs: 10,
                },
                &cred,
            )
            .await
            .expect("ssh exec");

        match result {
            OpResult::ShellCommand {
                stdout,
                stderr,
                exit_status,
            } => {
                assert_eq!(exit_status, 0, "stderr={stderr}");
                assert!(!stdout.is_empty(), "stdout was empty");
                eprintln!("live ssh stdout: {stdout}");
            }
            other => panic!("expected ShellCommand result, got {other:?}"),
        }
    }
}
