//! Local Devnet checkout. This binary never opens Wallet or accepts a seed.
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use nightfall_consensus::{Block, Chain};
use nightfall_crypto::{scan_output, ViewKey};
use nightfall_types::NetworkId;
use nightfall_wallet::counter::{self, Incoming, Invoice, Settlement};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;
#[derive(Parser)]
#[command(
    name = "nightfall-till",
    version,
    about = "View-only local Devnet till; no seed or spending commands"
)]
struct Cli {
    #[arg(long, global = true, default_value = "devnet")]
    network: String,
    #[arg(long)]
    datadir: PathBuf,
    #[arg(long)]
    view_key_file: PathBuf,
    #[arg(long)]
    rpc: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Address,
    Invoice {
        #[arg(long)]
        reference: String,
        #[arg(long)]
        darks: u64,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        expires_in: Option<u64>,
    },
    Till {
        #[arg(long, default_value_t = 10)]
        confirmations: u64,
        #[arg(long)]
        json: bool,
    },
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn regular_private(path: &Path, limit: u64) -> Result<fs::File> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > limit {
        bail!("Expected a bounded regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1
        {
            bail!("Use an owner-only file with one link");
        }
        let f = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let opened = f.metadata()?;
        if opened.ino() != meta.ino() || opened.dev() != meta.dev() {
            bail!("File changed during open");
        }
        Ok(f)
    }
    #[cfg(not(unix))]
    {
        Ok(fs::File::open(path)?)
    }
}
fn reference_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && s.as_bytes()[0].is_ascii_alphanumeric()
}
fn rpc(addr: &str, method: &str, params: Value) -> Result<Value> {
    let addr: std::net::SocketAddr = addr.parse()?;
    if !addr.ip().is_loopback() {
        bail!("RPC must be loopback");
    }
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    writeln!(
        stream,
        "{}",
        json!({"method":method,"params":params,"id":1})
    )?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(16 * 1024 * 1024 + 1)
        .read_line(&mut line)?;
    if line.len() > 16 * 1024 * 1024 {
        bail!("Node response too large");
    }
    let v: Value = serde_json::from_str(&line)?;
    if v.get("error").is_some_and(|e| !e.is_null()) {
        bail!("Node refused request");
    }
    v.get("result").cloned().context("Missing node result")
}
fn scan(addr: &str, view: &ViewKey) -> Result<(u64, Vec<Incoming>)> {
    let st = rpc(addr, "status", json!({}))?;
    let mut chain = Chain::new_fair(NetworkId::Devnet)?;
    let genesis = chain.genesis_hash.to_hex();
    if st["genesis"].as_str() != Some(&genesis)
        || st["network"].as_str() != Some("devnet")
        || st["loading"].as_bool() == Some(true)
    {
        bail!("Node identity or readiness mismatch");
    }
    let tip = st["tip_height"].as_u64().context("Missing tip")?;
    if tip > 10_000 || st["pruned"].as_bool() == Some(true) {
        bail!("Prototype needs an unpruned Devnet chain of at most 10000 blocks");
    }
    let mut height = 0;
    let mut payments = Vec::new();
    while height <= tip {
        let blocks: Vec<Block> = serde_json::from_value(rpc(
            addr,
            "get_blocks",
            json!({"from":height,"limit":(tip-height+1).min(128)}),
        )?)?;
        if blocks.is_empty() || blocks.len() > 128 {
            bail!("Incomplete chain page");
        }
        for block in blocks {
            if block.header.height.0 != height || height > tip {
                bail!("Chain changed during scan");
            }
            chain.apply_block(block.clone(), now())?;
            for out in &block.body.outputs {
                if out.features.is_coinbase() {
                    continue;
                }
                if let Some(found) = scan_output(view, out) {
                    payments.push(Incoming {
                        amount_darks: found.value,
                        memo: found.memo,
                        height: Some(height),
                        timestamp: block.header.timestamp_unix,
                        reference_id: hex::encode(out.commit.0),
                    });
                }
            }
            height += 1;
        }
    }
    let end = rpc(addr, "status", json!({}))?;
    let anchor: Vec<Block> =
        serde_json::from_value(rpc(addr, "get_blocks", json!({"from":tip,"limit":1}))?)?;
    if end["genesis"] != st["genesis"]
        || end["network"] != st["network"]
        || end["tip_height"].as_u64().is_none_or(|h| h < tip)
        || anchor.len() != 1
        || anchor[0].hash() != chain.tip_hash()
        || st["tip"].as_str() != Some(&chain.tip_hash().to_hex())
    {
        bail!("Chain changed or does not match its reported tip; retry");
    }
    Ok((tip, payments))
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.network != "devnet" {
        bail!("This view-only checkout prototype supports Devnet only");
    }
    let meta = fs::symlink_metadata(&cli.datadir)?;
    if !meta.is_dir() {
        bail!("Choose an existing dedicated till directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            bail!("Till directory must belong to you and must not be writable by others");
        }
    }
    if fs::read_to_string(cli.datadir.join(".nightfall-checkout-devnet"))? != "devnet\n" {
        bail!("Missing Devnet marker");
    }
    let mut key = Zeroizing::new(String::new());
    regular_private(&cli.view_key_file, 1024)?.read_to_string(&mut key)?;
    let view = ViewKey::decode(&key).map_err(|_| anyhow::anyhow!("Invalid view key"))?;
    if view.address().scan_point().is_none() || view.spend_point().is_none() {
        bail!("Invalid view-key points");
    }
    let dir = cli.datadir.join("invoices");
    if !dir.exists() {
        fs::create_dir(&dir)?;
    }
    if !fs::symlink_metadata(&dir)?.is_dir() {
        bail!("Invalid invoice directory");
    }
    match cli.command {
        Command::Address => println!("{}", view.address().encode()),
        Command::Invoice {
            reference,
            darks,
            description,
            expires_in,
        } => {
            if description.len() > 64 || description.chars().any(char::is_control) {
                bail!("Invalid description");
            }
            if fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
                .count()
                >= counter::MAX_INVOICES
            {
                bail!("Invoice limit reached");
            }
            if !reference_ok(&reference) || expires_in.is_some_and(|n| n == 0 || n > 604800) {
                bail!("Invalid invoice reference or expiry");
            }
            let invoice = Invoice {
                reference: reference.clone(),
                amount_darks: Some(darks),
                description,
                created_unix: now(),
                expires_unix: expires_in.map(|n| now() + n),
                closed_note: None,
            };
            invoice.check().map_err(anyhow::Error::msg)?;
            let path = dir.join(format!("{reference}.json"));
            // Publish a complete immutable file by hard link. Duplicate references
            // fail without overwriting; interrupted staging is never an invoice.
            let stage = dir.join(format!(
                ".stage-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            let result = (|| -> Result<()> {
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut f = options.open(&stage)?;
                f.write_all(&serde_json::to_vec(&invoice)?)?;
                f.sync_all()?;
                fs::hard_link(&stage, &path)?;
                fs::remove_file(&stage)?;
                #[cfg(unix)]
                fs::File::open(&dir)?.sync_all()?;
                Ok(())
            })();
            if stage.exists() {
                let _ = fs::remove_file(stage);
            }
            result?;
            println!(
                "{}",
                counter::request_for(&invoice, view.address(), NetworkId::Devnet).to_uri()
            );
        }
        Command::Till {
            confirmations,
            json: as_json,
        } => {
            if !(1..=10000).contains(&confirmations) {
                bail!("Invalid confirmation policy");
            }
            let mut invoices = Vec::new();
            for entry in fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if invoices.len() >= counter::MAX_INVOICES {
                    bail!("Invoice limit exceeded");
                }
                let mut raw = String::new();
                regular_private(&path, 8192)?.read_to_string(&mut raw)?;
                let invoice: Invoice = serde_json::from_str(&raw)?;
                invoice.check().map_err(anyhow::Error::msg)?;
                if !reference_ok(&invoice.reference)
                    || path.file_stem().and_then(|s| s.to_str()) != Some(&invoice.reference)
                {
                    bail!("Invoice identity mismatch");
                }
                invoices.push(invoice);
            }
            let (tip, payments) = scan(&cli.rpc, &view)?;
            for invoice in invoices {
                let status = counter::status(&invoice, &payments, now());
                let settled = counter::settlement(&status, tip, confirmations);
                let value = match settled {
                    Settlement::NotPaid => "not_paid".into(),
                    Settlement::Unconfirmed => "unconfirmed".into(),
                    Settlement::Confirming { have, need } => format!("confirming:{have}/{need}"),
                    Settlement::Confirmed => "confirmed".into(),
                    Settlement::ClosedByHand => "closed".into(),
                };
                if as_json {
                    println!(
                        "{}",
                        json!({"reference":invoice.reference,"asked_darks":invoice.amount_darks,"received_darks":status.received_darks,"invoice":status.headline(),"settlement":value,"tip_height":tip,"candidates":status.candidates.len(),"view_only":true})
                    );
                } else {
                    println!(
                        "{}: {} — {}",
                        invoice.reference,
                        status.headline(),
                        settled.headline()
                    );
                }
            }
        }
    }
    Ok(())
}
