//! NIGHTFALLCOIN CLI wallet. Talks to a local `nightfalld` over RPC.

use clap::{Parser, Subcommand, ValueEnum};
use nightfall_consensus::Block;
use nightfall_crypto::Address;
use nightfall_storage::default_data_dir;
use nightfall_types::{Amount, NetworkId, COIN_NAME, DARKS_PER_NIGHT, TICKER};
use nightfall_wallet::Wallet;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

/// Default fee: 0.001 NIGHT.
const DEFAULT_FEE_DARKS: u64 = DARKS_PER_NIGHT / 1_000;

#[derive(Parser, Debug)]
#[command(name = "nightfall-wallet", about = "NIGHTFALLCOIN wallet", version)]
struct Cli {
    #[arg(long, global = true)]
    datadir: Option<PathBuf>,

    #[arg(long, value_enum, global = true, default_value_t = Net::Devnet)]
    network: Net,

    #[arg(long, global = true, default_value = "wallet.seed")]
    seed_file: String,

    /// nightfalld RPC host:port (default 127.0.0.1:<network rpc port>)
    #[arg(long, global = true)]
    rpc: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    About,
    /// Create the wallet if it does not exist and print the address.
    Init,
    /// Print the receive address.
    Address,
    /// Print the watch-only view key. Reveals amounts and memos; cannot spend.
    ExportViewKey,
    /// Pull blocks from the node and find outputs belonging to this wallet.
    Sync,
    /// Discard local scan state and re-scan from genesis.
    Rescan,
    Balance,
    /// List owned outputs.
    Outputs,
    Send {
        /// Recipient address (nf1…).
        #[arg(long)]
        to: String,
        /// Amount in whole NIGHT unless --darks is given.
        #[arg(long)]
        amount: u64,
        #[arg(long, default_value_t = false)]
        darks: bool,
        #[arg(long, default_value_t = DEFAULT_FEE_DARKS)]
        fee_darks: u64,
        #[arg(long, default_value = "")]
        memo: String,
    },
    NodeStatus,
    /// Ask the node to re-prove the global supply invariant.
    VerifySupply,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Net {
    Mainnet,
    Testnet,
    Devnet,
}

impl From<Net> for NetworkId {
    fn from(n: Net) -> Self {
        match n {
            Net::Mainnet => NetworkId::Mainnet,
            Net::Testnet => NetworkId::Testnet,
            Net::Devnet => NetworkId::Devnet,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let network: NetworkId = cli.network.into();
    let datadir = cli.datadir.unwrap_or_else(|| default_data_dir(network));
    let rpc = cli
        .rpc
        .unwrap_or_else(|| format!("127.0.0.1:{}", network.default_rpc_port()));

    match cli.command {
        Commands::About => {
            println!("{COIN_NAME} ({TICKER}) wallet");
            println!("Protocol v{}", nightfall_types::PROTOCOL_VERSION);
            println!("Shielded by default · 100% fee burn · RPC {rpc}");
            return Ok(());
        }
        Commands::NodeStatus => {
            let res = rpc_call(&rpc, "status", json!({}))?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            return Ok(());
        }
        Commands::VerifySupply => {
            let res = rpc_call(&rpc, "verify_supply", json!({}))?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            return Ok(());
        }
        _ => {}
    }

    let mut wallet = Wallet::open(&datadir, network, &cli.seed_file)?;

    match cli.command {
        Commands::Init => {
            println!("wallet......... {}", wallet.seed_path.display());
            println!("network........ {network}");
            println!("address........ {}", wallet.address_string());
            println!();
            println!("Back up the seed file. Losing it loses the coins permanently.");
        }

        Commands::Address => println!("{}", wallet.address_string()),

        Commands::ExportViewKey => {
            println!("{}", wallet.view_key_string());
            eprintln!();
            eprintln!("This key reveals every amount and memo you send or receive.");
            eprintln!("It cannot spend. Share it only with someone you want to have that view.");
        }

        Commands::Rescan => {
            wallet.reset_scan()?;
            let n = sync(&mut wallet, &rpc, 0)?;
            println!("rescanned from genesis, found {n} output(s)");
            println!("balance........ {}", wallet.balance());
        }

        Commands::Sync => {
            let from = wallet.scan_from();
            let n = sync(&mut wallet, &rpc, from)?;
            println!("scanned to height {}", wallet.scanned_to());
            println!("new outputs.... {n}");
            println!("balance........ {}", wallet.balance());
        }

        Commands::Balance => {
            println!("{}", wallet.balance());
            println!("outputs........ {}", wallet.spendable_count());
        }

        Commands::Outputs => {
            for o in wallet.outputs() {
                println!(
                    "{} {:>18}  height={:<8} {}{}",
                    if o.spent { "spent " } else { "unspent" },
                    Amount(o.value).to_string(),
                    o.height,
                    &o.commit.to_hex()[..16],
                    if o.memo.is_empty() {
                        String::new()
                    } else {
                        format!("  memo: {}", o.memo)
                    }
                );
            }
        }

        Commands::Send {
            to,
            amount,
            darks,
            fee_darks,
            memo,
        } => {
            let to_addr = Address::decode(&to)
                .map_err(|e| anyhow::anyhow!("recipient address rejected: {e}"))?;
            let amount_darks = if darks {
                amount
            } else {
                amount
                    .checked_mul(DARKS_PER_NIGHT)
                    .ok_or_else(|| anyhow::anyhow!("amount overflow"))?
            };

            let tx = wallet.create_payment(&to_addr, amount_darks, fee_darks, &memo)?;
            let res = rpc_call(&rpc, "submit_tx", json!({ "tx": tx }))?;

            wallet.mark_pending_spend(&tx)?;

            println!("sent........... {}", Amount(amount_darks));
            println!("fee (burned)... {}", Amount(fee_darks));
            println!("txid........... {}", tx.txid().to_hex());
            println!("node........... {}", serde_json::to_string(&res)?);
            println!();
            println!("The transaction is in the mempool until a block is mined.");
        }

        Commands::About | Commands::NodeStatus | Commands::VerifySupply => unreachable!(),
    }

    Ok(())
}

/// Pull blocks from the node in batches and scan them.
fn sync(wallet: &mut Wallet, rpc: &str, from: u64) -> anyhow::Result<u32> {
    let mut height = from;
    let mut total = 0u32;

    loop {
        let res = rpc_call(rpc, "get_blocks", json!({ "from": height, "limit": 128 }))?;
        let blocks: Vec<Block> = serde_json::from_value(res)?;
        if blocks.is_empty() {
            break;
        }
        let last = blocks.last().map(|b| b.header.height.0).unwrap_or(height);
        total += wallet.scan_blocks(&blocks)?;
        if last < height + 1 {
            break;
        }
        height = last + 1;
    }
    Ok(total)
}

fn rpc_call(
    addr: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let stream = TcpStream::connect(addr)
        .map_err(|e| anyhow::anyhow!("cannot reach nightfalld at {addr}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    let req = json!({ "method": method, "params": params, "id": 1 });
    writeln!(writer, "{req}")?;
    writer.flush()?;

    let mut line = String::new();
    reader.read_line(&mut line)?;

    let res: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| anyhow::anyhow!("bad response from node: {e}"))?;

    if let Some(e) = res.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("node error: {e}");
    }
    Ok(res.get("result").cloned().unwrap_or(json!(null)))
}
