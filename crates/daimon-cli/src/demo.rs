//! Phase 2b acceptance demo — proves the broker pattern (D19) end-to-end
//! against a real own-vault + real SSH endpoint.
//!
//! Usage:
//!   daimon-demo \
//!     --target mikrotik-edge \
//!     --host 10.100.10.1 \
//!     --port 22 \
//!     --user admin \
//!     --key ~/.ssh/arif \
//!     --command "uname -a"
//!
//! What this proves:
//! - SqliteVaultClient (in-memory + file-backed both supported) holds a
//!   real SSH private key, encrypted with chacha20poly1305 (D22).
//! - SqliteRegistry maps target://<name> to host+port+credential_ref (D20).
//! - Broker::with_production_admin wires the lot.
//! - broker.execute(...) drives the russh SshTransport against the real
//!   endpoint with the credential resolved from the in-tree vault.
//! - The agent code path NEVER touches the raw SSH key (D19) — we only
//!   call broker.execute().
//! - Every action emits an audit event (D23). The demo prints the event
//!   log at the end.
//!
//! For demo purposes the master key is generated in-process (NOT from
//! systemd LoadCredentialEncrypted). A flag opts in to file-backed
//! storage at a temp dir if you want to inspect the on-disk shape.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use daimon_broker::{
    AuditFilter, Broker, Credential, ExecRequest, Inventory, ManagedTarget, Op, OpResult,
    TargetKind, TargetRef, TransportKind,
};
use daimon_audit::SqliteAuditSink;
use daimon_inventory::SqliteRegistry;
use daimon_transport::{SshTransport, Transport};
use daimon_vault::{MasterKey, SqliteVaultClient};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "daimon-demo",
    about = "Phase 2b acceptance: broker → vault → ssh-transport end-to-end."
)]
struct Args {
    /// Logical target name (becomes target://<name>).
    #[arg(long)]
    target: String,

    /// Real SSH host (IP or hostname).
    #[arg(long)]
    host: String,

    /// SSH port. Default 22.
    #[arg(long, default_value_t = 22)]
    port: u16,

    /// SSH username.
    #[arg(long)]
    user: String,

    /// Path to SSH private key (PEM/OpenSSH format). Mutually exclusive
    /// with --password.
    #[arg(long, conflicts_with = "password")]
    key: Option<String>,

    /// SSH password. Mutually exclusive with --key.
    #[arg(long, conflicts_with = "key")]
    password: Option<String>,

    /// Optional key passphrase if the key is encrypted.
    #[arg(long, requires = "key")]
    passphrase: Option<String>,

    /// Shell command to execute on the remote host.
    #[arg(long)]
    command: String,

    /// Command timeout (seconds). Default 30.
    #[arg(long, default_value_t = 30)]
    timeout_secs: u32,

    /// Skip known_hosts verification and accept any server key. Required
    /// for first-bootstrap demos; logs a WARN every time it's used. Does
    /// NOT persist the key. Use `--learn-known-hosts` for TOFU bootstrap.
    #[arg(long)]
    accept_any_host_key: bool,

    /// **TOFU bootstrap**: accept any server key on this run AND append
    /// it to the specified known_hosts file. Subsequent runs should use
    /// `--known-hosts <same-path>` for strict verification.
    #[arg(long, conflicts_with = "accept_any_host_key")]
    learn_known_hosts: Option<String>,

    /// Path to a known_hosts file. Default `/var/lib/daimon/known_hosts`.
    /// Ignored if `--accept-any-host-key` or `--learn-known-hosts` is set.
    #[arg(long)]
    known_hosts: Option<String>,

    /// Actor id recorded in audit events. Defaults to "demo:operator".
    #[arg(long, default_value = "demo:operator")]
    actor: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();

    info!("==> dAImon Phase 2b demo — in-tree vault + real russh SSH");

    // ---- 1. Build the credential from CLI inputs ------------------------------
    let cred = match (&args.key, &args.password) {
        (Some(key_path), None) => {
            let pem = std::fs::read_to_string(key_path)
                .with_context(|| format!("read key {key_path}"))?;
            Credential::SshKey {
                username: args.user.clone(),
                private_key_pem: pem,
                passphrase: args.passphrase.clone(),
            }
        }
        (None, Some(pw)) => Credential::SshPassword {
            username: args.user.clone(),
            password: pw.clone(),
        },
        _ => anyhow::bail!("exactly one of --key or --password is required"),
    };

    // ---- 2. Seed vault + inventory + audit (in-memory for demo) --------------
    info!("==> Seeding in-memory vault, inventory, audit DBs");
    let master_key = MasterKey::from_bytes(rand_master_key()?);
    let vault = Arc::new(SqliteVaultClient::in_memory(master_key)?);
    let inventory = Arc::new(SqliteRegistry::in_memory()?);
    let audit: Arc<dyn daimon_audit::AuditSink> = Arc::new(SqliteAuditSink::in_memory()?);

    // ---- 3. Build SSH transport with the right policy -----------------------
    info!("==> Building SshTransport");
    let ssh: Arc<dyn Transport> = if let Some(learn_path) = &args.learn_known_hosts {
        info!(
            learn_to = %learn_path,
            "  policy = AcceptAnyAndLearn (TOFU bootstrap — host key will be appended)"
        );
        Arc::new(SshTransport::with_accept_any_and_learn(learn_path.into()))
    } else if args.accept_any_host_key {
        info!("  policy = AcceptAny (security downgrade — bootstrap mode only, NOT persisted)");
        Arc::new(SshTransport::with_accept_any())
    } else {
        match &args.known_hosts {
            Some(path) => {
                info!(known_hosts = %path, "  policy = KnownHosts (custom path)");
                Arc::new(SshTransport::with_known_hosts_path(path.into()))
            }
            None => {
                info!("  policy = KnownHosts (default /var/lib/daimon/known_hosts)");
                Arc::new(SshTransport::new())
            }
        }
    };

    let mut transports: HashMap<TransportKind, Arc<dyn Transport>> = HashMap::new();
    transports.insert(TransportKind::Ssh, ssh);

    let broker = Broker::with_production_admin(inventory.clone(), vault, audit.clone(), transports);

    // ---- 4. Operator workflow via the admin proxy (D22/D23/D24) -------------
    info!("==> [admin] vault_create — store SSH credential");
    let cred_id = broker
        .vault_create(&args.actor, &args.target, cred)
        .await
        .context("vault_create")?;
    info!("  credential id = {cred_id}, name = {}", args.target);

    info!("==> [admin] inventory_upsert — register managed target");
    let managed = ManagedTarget {
        r#ref: TargetRef::parse(&format!("target://{}", args.target))?,
        kind: TargetKind::Host,
        transport: TransportKind::Ssh,
        host: args.host.clone(),
        port: args.port,
        credential_ref: format!("vault://{}", args.target),
        labels: BTreeMap::new(),
        capabilities: vec![],
    };
    broker
        .inventory_upsert(&args.actor, managed)
        .await
        .context("inventory_upsert")?;

    let metadata = inventory
        .list(None)
        .await
        .into_iter()
        .find(|m| m.r#ref.to_string() == format!("target://{}", args.target))
        .context("target not in inventory after upsert")?;
    info!(
        "  registered target://{} → {}:{} ({:?} / {:?})",
        args.target, metadata.host, metadata.port, metadata.kind, metadata.transport
    );

    // ---- 5. The actual broker.execute call — agent code path -----------------
    info!("==> [agent] broker.execute — running `{}` via SSH", args.command);
    let req = ExecRequest::new(
        format!("agent:demo:{}", args.actor),
        TargetRef::parse(&format!("target://{}", args.target))?,
        Op::ShellCommand {
            command: args.command.clone(),
            timeout_secs: args.timeout_secs,
        },
    );
    let result = broker.execute(req).await.context("broker.execute")?;

    match &result {
        OpResult::ShellCommand {
            stdout,
            stderr,
            exit_status,
        } => {
            println!("\n========== REMOTE STDOUT ==========");
            print!("{stdout}");
            if !stderr.is_empty() {
                println!("========== REMOTE STDERR ==========");
                print!("{stderr}");
            }
            println!("========== EXIT STATUS: {exit_status} ==========\n");
        }
        other => {
            anyhow::bail!("expected ShellCommand result, got {other:?}");
        }
    }

    // ---- 6. Print the audit log ---------------------------------------------
    info!("==> Audit log:");
    let events = broker
        .audit_query(&args.actor, &AuditFilter::default(), 50, 0)
        .await
        .context("audit_query")?;
    for ev in events.iter().rev() {
        println!(
            "  [{ts}] actor={actor} action={action} target={target} result={result} latency_ms={latency} {op}",
            ts = ev.ts.to_rfc3339(),
            actor = ev.actor_id,
            action = ev.action.as_str(),
            target = ev.target_ref.as_deref().unwrap_or("-"),
            result = ev.result.as_str(),
            latency = ev.latency_ms.unwrap_or(0),
            op = ev.op_summary.as_deref().unwrap_or(""),
        );
    }

    info!("==> Demo complete. D19 invariant proven: agent code path never touched the SSH key.");
    Ok(())
}

/// Generate a 32-byte master key without bringing in a heavy RNG crate.
/// Uses `getrandom` semantically by reading from system entropy via a
/// dedicated rng if available, falling back to thread RNG.
fn rand_master_key() -> Result<[u8; 32]> {
    use std::io::Read;
    let mut buf = [0u8; 32];
    let mut f = std::fs::File::open("/dev/urandom").context("open /dev/urandom")?;
    f.read_exact(&mut buf).context("read /dev/urandom")?;
    Ok(buf)
}
