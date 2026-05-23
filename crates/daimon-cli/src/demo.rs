//! Phase 2b acceptance demo — proves the broker pattern (D19) end-to-end
//! against a real own-vault + real SSH endpoint.
//!
//! Phase 2c update: vault/inventory/audit are Postgres-backed via the
//! production stack assembler (`daimon-broker::production`). The demo
//! therefore needs a running Postgres + a known tenant. Default
//! configuration points at the dev `daimon` database with tenant `default`.
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
//! D19 invariant proved: the agent code path NEVER touches the raw SSH key
//! — we only call broker.execute().

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use clap::Parser;
use daimon_broker::production::{build_production_broker, BootConfig, MasterKeyHandle};
use daimon_broker::{
    AuditFilter, Credential, ExecRequest, ManagedTarget, Op, OpResult, TargetKind, TargetRef,
    TransportKind,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "daimon-demo",
    about = "Phase 2b/2c acceptance: broker → vault → ssh-transport end-to-end against Postgres."
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

    /// Path to a known_hosts file. Default `./daimon-data/known_hosts`.
    #[arg(long, default_value = "./daimon-data/known_hosts")]
    known_hosts: String,

    /// Tenant slug (must exist in public.tenants). Default `default`.
    #[arg(long, default_value = "default", env = "DAIMON_TENANT_SLUG")]
    tenant_slug: String,

    /// Postgres connection URL. Defaults to $DAIMON_PG_URL or
    /// postgres://$USER@localhost:5432/daimon.
    #[arg(long, env = "DAIMON_PG_URL")]
    pg_url: Option<String>,

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

    info!("==> dAImon Phase 2c demo — Postgres-backed in-tree vault + real russh SSH");

    // ---- 1. Build the credential from CLI inputs -----------------------------
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

    // ---- 2. Assemble the production broker against Postgres ------------------
    info!("==> Assembling production broker (Postgres-backed)");
    let pg_url = args
        .pg_url
        .clone()
        .or_else(|| std::env::var("DAIMON_PG_URL").ok())
        .unwrap_or_else(|| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
            format!("postgres://{user}@localhost:5432/daimon")
        });
    let master_key = MasterKeyHandle::from_systemd_or_dev_env()
        .context("load master key (set DAIMON_MASTER_KEY_FILE for dev)")?;
    let broker = build_production_broker(BootConfig {
        pg_url,
        tenant_slug: args.tenant_slug.clone(),
        known_hosts_path: args.known_hosts.clone().into(),
        master_key,
        kill_path: std::path::PathBuf::from("./daimon-data/KILL"),
        policy_path: std::path::PathBuf::from("./daimon-data/policy.toml"),
    })
    .await
    .context("build_production_broker")?;

    // ---- 3. Operator workflow via the admin proxy (D22/D23/D24) -------------
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

    // ---- 4. The actual broker.execute call — agent code path -----------------
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

    // ---- 5. Print the audit log ----------------------------------------------
    info!("==> Audit log (recent first):");
    let events = broker
        .audit_query(&args.actor, &AuditFilter::default(), 50, 0)
        .await
        .context("audit_query")?;
    for ev in events.iter() {
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
