//! `nightfall-nodekit` — the operator's command line.
//!
//! Every command reads, or writes one new file the operator named. None
//! needs root, none touches a running node's data, none contacts anything but
//! the node address it is given.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use nightfall_nodekit as kit;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "nightfall-nodekit",
    version,
    about = "Diagnose, verify and set up a NIGHTFALLCOIN node; destructive legacy upgrades require explicit --force"
)]
struct Cli {
    /// Network to check against. Its genesis is computed by this build's
    /// consensus code, not typed in.
    #[arg(long, global = true, default_value = "mainnet")]
    network: String,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Inspect a data directory. Opens files read-only; safe beside a running node.
    Doctor {
        /// Data directory. Defaults to the platform location for the network.
        #[arg(long)]
        datadir: Option<PathBuf>,
    },
    /// Ask a running node who it is and whether it is keeping up.
    Status {
        /// RPC address. Defaults to 127.0.0.1 on the network's RPC port.
        #[arg(long)]
        rpc: Option<String>,
        #[arg(long, default_value_t = 5)]
        timeout_secs: u64,
    },
    /// Check downloaded files against a published SHA256SUMS list.
    Verify {
        /// The checksum list, e.g. SHA256SUMS-1.0.5.txt.
        #[arg(long)]
        sums: PathBuf,
        /// Directory holding the downloads. Defaults to the list's directory.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Check only this file name.
        #[arg(long)]
        file: Option<String>,
    },
    /// Write a systemd or launchd unit to a new file for review. Never installs it.
    Service {
        #[arg(long, value_enum)]
        kind: Kind,
        /// Absolute path of the nightfalld binary.
        #[arg(long)]
        binary: PathBuf,
        /// Absolute path of the node's data directory.
        #[arg(long)]
        datadir: PathBuf,
        /// Unprivileged account to run as (systemd only).
        #[arg(long)]
        user: Option<String>,
        /// Accept inbound peers on all interfaces. RPC stays on loopback regardless.
        #[arg(long)]
        public: bool,
        /// Also serve the light API for a self-hosted web wallet on
        /// 127.0.0.1:<PORT> (conventionally 17888). Loopback only.
        #[arg(long)]
        light_port: Option<u16>,
        /// Where to write the unit. Must not exist; system directories are refused.
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a Caddyfile serving the web wallet and your node from your own
    /// domain, same-origin. Never installs it.
    Selfhost {
        /// Your hostname, e.g. wallet.example.org.
        #[arg(long)]
        domain: String,
        /// Absolute path of the web wallet's static files.
        #[arg(long)]
        wallet_dir: PathBuf,
        /// The node's --mobile-listen port on 127.0.0.1.
        #[arg(long, default_value_t = 17888)]
        light_port: u16,
        /// Where to write the Caddyfile. Must not exist.
        #[arg(long)]
        out: PathBuf,
    },
    /// Explicitly upgrade old pruned 1.1.x chain data by backing it up
    /// and resetting only chain state for a fully verified resync.
    /// Never auto-seals history whose deleted bodies cannot be validated.
    UpgradePruned {
        /// Existing datadir; no implicit default for a destructive operation.
        #[arg(long)]
        datadir: PathBuf,
        /// Acknowledge that the preserved chain backup will need resync.
        #[arg(long)]
        force: bool,
    },
    /// Print the genesis hash this build expects.
    Genesis,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Systemd,
    Launchd,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let network = kit::parse_network(&cli.network)?;
    match cli.cmd {
        Cmd::UpgradePruned { datadir, force } => {
            if !force {
                anyhow::bail!(
                    "Legacy pruned 1.1.x state cannot be safely sealed without its deleted historical bodies. Stop the node, back up the wallet, and re-run upgrade-pruned with --force to preserve the old chain in a recovery backup and start a fresh verified resync. No files were changed."
                );
            }
            let network_explicit = std::env::args()
                .skip(1)
                .any(|arg| arg == "--network" || arg.starts_with("--network="));
            let options = nightfall_node::recovery::RecoverOpts {
                datadir: datadir.clone(),
                network,
                force,
            };
            // The recovery preflight runs BEFORE taking a lock: no accidental
            // datadir creation and no symlink traversal on malformed inputs.
            nightfall_node::recovery::precheck(&options, network_explicit, true)?;

            let validate_legacy = || -> Result<()> {
                let meta: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(datadir.join("chain-meta.json"))?)?;
                let is_pruned = meta.get("pruned").and_then(|v| v.as_bool()) == Some(true)
                    && meta
                        .get("first_height")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        > 0;
                let seal_horizon = meta
                    .get("validated_horizon_fingerprint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let seal_headers = meta
                    .get("validated_headers_fingerprint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !is_pruned || (!seal_horizon.is_empty() && !seal_headers.is_empty()) {
                    anyhow::bail!(
                        "upgrade-pruned only accepts legacy pruned data without complete seals; this datadir is not such a case"
                    );
                }
                Ok(())
            };

            validate_legacy()?;
            // Do not touch running nodes or desktop wallets.
            let _lock = nightfall_storage::dirlock::acquire(&datadir)?;
            // Recheck under the one-writer lock to avoid an intervening change.
            validate_legacy()?;

            let report = nightfall_node::recovery::recover_stalled_fork(&options)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "network": network.as_str(),
                        "backup": report.backup_dir,
                        "old_tip": report.previous_tip,
                        "resync_required": true,
                        "wallet_preserved": true
                    })
                );
            } else {
                println!(
                    "Legacy pruned chain archived in {}",
                    report.backup_dir.display()
                );
                println!("Wallet preserved. Restart the node with mining off until the whole chain is independently verified and synchronized.");
                println!("No unverified old storage seals were trusted or minted.");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Genesis => {
            let g = kit::pinned_genesis(network)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"network": network.as_str(), "genesis": g})
                );
            } else {
                println!("{g}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Doctor { datadir } => {
            let dir = datadir.unwrap_or_else(|| nightfall_storage::default_data_dir(network));
            let report = kit::doctor(network, &dir)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "nightfall-nodekit doctor · {} · {}",
                    report.network, report.datadir
                );
                println!("expected genesis {}\n", report.pinned_genesis);
                for f in &report.findings {
                    let tag = match f.level {
                        kit::Level::Ok => "  ok  ",
                        kit::Level::Note => "  ·   ",
                        kit::Level::Warn => " WARN ",
                        kit::Level::Fail => " FAIL ",
                    };
                    println!("{tag} {:<12} {}", f.check, f.detail);
                }
            }
            Ok(match report.worst() {
                kit::Level::Fail => ExitCode::from(1),
                _ => ExitCode::SUCCESS,
            })
        }
        Cmd::Status { rpc, timeout_secs } => {
            let addr = rpc.unwrap_or_else(|| format!("127.0.0.1:{}", network.default_rpc_port()));
            let s = kit::status(network, &addr, Duration::from_secs(timeout_secs))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                println!(
                    "node at {addr} · {} · genesis matches this build",
                    s.network
                );
                println!("  tip        {}", s.tip_height);
                if let Some(b) = s.best_peer_height {
                    println!("  best peer  {b}  ({} behind)", s.lag.unwrap_or(0));
                }
                if let Some(p) = s.peers {
                    println!(
                        "  peers      {p}{}",
                        if p == 0 {
                            "  — mining alone risks a private fork"
                        } else {
                            ""
                        }
                    );
                }
                if s.loading {
                    println!("  loading    still verifying its saved chain");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Verify { sums, dir, file } => {
            let dir = dir.unwrap_or_else(|| {
                sums.parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
            });
            let results = kit::verify_sums(&sums, &dir, file.as_deref())?;
            let mut bad = false;
            let mut any_checked = false;
            for (name, v) in &results {
                match v {
                    kit::Verdict::Ok => {
                        any_checked = true;
                        if !cli.json {
                            println!("  OK        {name}");
                        }
                    }
                    kit::Verdict::Missing => {
                        if !cli.json {
                            println!("  not here  {name}");
                        }
                    }
                    kit::Verdict::Mismatch { .. } => {
                        bad = true;
                        any_checked = true;
                        if !cli.json {
                            println!("  MISMATCH  {name}  — do not run this file");
                        }
                    }
                }
            }
            if cli.json {
                let rows: Vec<_> = results
                    .iter()
                    .map(|(n, v)| serde_json::json!({"file": n, "result": v}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
            if !any_checked && !bad {
                eprintln!("none of the listed files are in {}", dir.display());
                return Ok(ExitCode::from(1));
            }
            Ok(if bad {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::Service {
            kind,
            binary,
            datadir,
            user,
            public,
            light_port,
            out,
        } => {
            let spec = kit::ServiceSpec {
                kind: match kind {
                    Kind::Systemd => kit::ServiceKind::Systemd,
                    Kind::Launchd => kit::ServiceKind::Launchd,
                },
                network,
                binary,
                datadir,
                user,
                public_p2p: public,
                light_port,
            };
            let unit = kit::render_service(&spec)?;
            kit::write_new_file(&out, &unit)?;
            println!(
                "wrote {}. Read it, then install it yourself using the lines at its top.",
                out.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Selfhost {
            domain,
            wallet_dir,
            light_port,
            out,
        } => {
            let caddy = kit::render_caddy(&kit::SelfHostSpec {
                domain,
                wallet_dir,
                light_port,
            })?;
            kit::write_new_file(&out, &caddy)?;
            println!("wrote {}. Read it, then install it yourself; docs/MY-NODE.md explains what your node will see.", out.display());
            Ok(ExitCode::SUCCESS)
        }
    }
}
