//! `nightfalld` — full node: P2P, RPC, optional mining.

use clap::{Parser, Subcommand, ValueEnum};
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_p2p::network_magic;
use nightfall_storage::{default_data_dir, now_unix, ChainStore};
use nightfall_types::{
    NetworkId, COIN_NAME, HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_NIGHT, MAX_SUPPLY_NIGHT,
    TICKER,
};
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "nightfalld",
    about = "NIGHTFALLCOIN full node — fair launch, 90M cap, privacy by default",
    version
)]
struct Cli {
    #[arg(long, global = true)]
    datadir: Option<PathBuf>,

    #[arg(long, value_enum, global = true, default_value_t = Net::Devnet)]
    network: Net,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Banner,
    Init,
    Status,
    Tip,
    Path,
    /// Mine N blocks once (offline convenience)
    Mine {
        #[arg(long, default_value_t = 1)]
        blocks: u32,
        #[arg(long, default_value = "miner.seed")]
        miner_seed: PathBuf,
    },
    /// Run full node daemon: P2P + RPC + optional continuous mining
    Run {
        #[arg(long)]
        listen: Option<String>,
        #[arg(long)]
        rpc_listen: Option<String>,
        /// Connect to peers (repeatable), host:port
        #[arg(long = "connect")]
        connect: Vec<String>,
        #[arg(long, default_value_t = false)]
        mine: bool,
        #[arg(long, default_value = "miner.seed")]
        miner_seed: PathBuf,
        /// SOCKS5 proxy for outbound P2P. Tor: `127.0.0.1:9050`.
        #[arg(long)]
        proxy: Option<String>,
        /// Public phone API (HTTP). Only status/scan_feed/submit_tx.
        /// Example: 0.0.0.0:17888
        #[arg(long)]
        mobile_listen: Option<String>,
        /// Run as a pure introducer: answer the handshake, hand over the
        /// address book, hang up. Holds no inbound session, so one machine
        /// can point thousands of new installs at the network instead of
        /// the 128 a normal node has seats for. Still syncs and dials out.
        #[arg(long, default_value_t = false)]
        introducer: bool,
        /// Drop block bodies older than the 500-block reorg window.
        /// UTXO and headers stay. Cannot rescan from genesis afterwards.
        /// Seeds that serve IBD or --mobile-listen must not set this.
        #[arg(long, default_value_t = false)]
        prune: bool,
    },
    /// Write `blocks.jsonl` + `snapshot.json`. The importer still verifies PoW.
    ExportSnapshot {
        #[arg(long)]
        out: PathBuf,
    },
    /// Rewrite the chain file in the compact binary encoding.
    ///
    /// Measured on mainnet at 31,288 blocks: 152.3 MiB as JSON, 42.2 MiB
    /// binary. The same chain, 72.3 % less disk, because serde_json renders
    /// every hash and range proof as an array of decimal numbers. Storage
    /// only: block hashes are computed over raw field bytes and the P2P wire
    /// is untouched, so a converted node speaks to every other node exactly
    /// as before.
    ///
    /// The old file is kept as blocks.jsonl.pre-binary. Delete blocks.bin to
    /// go back.
    MigrateStorage,
    /// Load a snapshot directory. Every block is re-checked; this is not a trust shortcut.
    ImportSnapshot {
        #[arg(long)]
        from: PathBuf,
    },
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
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let network: NetworkId = cli.network.into();
    let datadir = cli.datadir.unwrap_or_else(|| default_data_dir(network));
    let store = ChainStore::new(&datadir);

    match cli.command {
        Commands::Banner => {
            println!("{COIN_NAME} ({TICKER})");
            println!("Money that refuses to snitch.");
            println!(
                "privacy=default · premine=0 · max_supply={MAX_SUPPLY_NIGHT} · no_tail · fee_burn=100%"
            );
        }
        Commands::Init => {
            let chain = store.load_or_new(network)?;
            store.save(&chain)?;
            println!("datadir........ {}", datadir.display());
            println!("network........ {network}");
            println!("genesis_hash... {}", chain.genesis_hash);
            println!("max_supply..... {MAX_SUPPLY_NIGHT} {TICKER}");
            println!("blocks......... {}", chain.block_count());
            println!("p2p_port....... {}", network.default_p2p_port());
            println!("rpc_port....... {}", network.default_rpc_port());
            println!("status......... ready — use `nightfalld run` to start");
        }
        Commands::Mine { blocks, miner_seed } => {
            let mut chain = store.load_or_new(network)?;
            let seed_path = resolve(&datadir, &miner_seed);
            let miner = load_or_create_miner(&seed_path)?;
            println!("miner.......... {}", miner.address().encode());
            for _ in 0..blocks {
                let block = chain.mine_block(&miner.address(), vec![], now_unix())?;
                println!(
                    "mined height={} hash={} reward={}",
                    block.header.height.0,
                    block.hash(),
                    nightfall_types::Amount(block.header.reward_darks),
                );
            }
            store.save(&chain)?;
            println!(
                "chain blocks={} minted={}",
                chain.block_count(),
                nightfall_types::Amount(chain.total_minted())
            );
        }
        Commands::Status => {
            let chain = store.load_or_new(network)?;
            print_status(&chain, &datadir, network, 0, 0);
        }
        Commands::Tip => {
            let chain = store.load_or_new(network)?;
            if let Some(b) = chain.blocks.last() {
                println!("{}", serde_json::to_string_pretty(b)?);
            } else {
                println!("null");
            }
        }
        Commands::Path => {
            println!("{}", store.blocks_path().display());
        }
        Commands::ExportSnapshot { out } => {
            let snap = store.export_snapshot(&out)?;
            println!("wrote.......... {}", out.display());
            println!("blocks......... {}", snap.blocks);
            println!("tip............ {}", snap.tip);
            println!("genesis........ {}", snap.genesis);
            println!("network........ {:?}", snap.network);
            println!("verify......... importer re-checks PoW and supply");
        }
        Commands::MigrateStorage => {
            // Rewriting the chain file under a running node would have the
            // daemon appending to a file this command is about to rename.
            // There is no lock to take, so say it plainly.
            println!("datadir........ {}", datadir.display());
            println!("note........... stop `nightfalld run` first");
            let (before, after) = store.migrate_to_binary()?;
            let saved = before.saturating_sub(after);
            let pct = if before > 0 {
                saved as f64 * 100.0 / before as f64
            } else {
                0.0
            };
            println!("before......... {:.1} MB", before as f64 / 1_048_576.0);
            println!("after.......... {:.1} MB", after as f64 / 1_048_576.0);
            println!(
                "saved.......... {:.1} MB ({pct:.1} %)",
                saved as f64 / 1_048_576.0
            );
            println!("verified....... every block hash matched before the swap");
            println!("kept........... blocks.jsonl.pre-binary — delete when happy");
        }
        Commands::ImportSnapshot { from } => {
            println!("importing...... {}", from.display());
            println!("verify......... full PoW + supply (not trusted)");
            let chain = store.import_snapshot(&from, network)?;
            print_status(&chain, &datadir, network, 0, 0);
        }
        Commands::Run {
            listen,
            rpc_listen,
            connect,
            mine,
            miner_seed,
            proxy,
            mobile_listen,
            introducer,
            prune,
        } => {
            let listen =
                listen.unwrap_or_else(|| format!("0.0.0.0:{}", network.default_p2p_port()));
            let rpc_listen =
                rpc_listen.unwrap_or_else(|| format!("127.0.0.1:{}", network.default_rpc_port()));
            let seed_path = resolve(&datadir, &miner_seed);
            let miner = if mine {
                Some(load_or_create_miner(&seed_path)?.address())
            } else {
                None
            };
            if let Some(ref m) = miner {
                println!("miner.......... {}", m.encode());
            }

            let proxy = proxy.or_else(|| std::env::var("NIGHTFALL_PROXY").ok());
            println!(
                "proxy.......... {}",
                proxy.as_deref().unwrap_or("127.0.0.1:9050 (Tor default)")
            );
            let cfg = NodeConfig {
                network,
                datadir: datadir.clone(),
                p2p_listen: listen.clone(),
                rpc_listen: rpc_listen.clone(),
                connect,
                mine,
                miner,
                proxy,
                mobile_listen: mobile_listen.clone(),
                peers_url: std::env::var("NIGHTFALL_PEERS_URL").ok(),
                introducer,
                prune,
            };

            println!("{COIN_NAME} node starting");
            println!("network........ {network}");
            println!("datadir........ {}", datadir.display());
            println!("p2p............ {listen}");
            println!("rpc............ {rpc_listen}");
            if let Some(ref m) = mobile_listen {
                println!("mobile......... {m}");
            }
            println!("mine........... {mine}");
            println!(
                "prune.......... {}",
                if prune {
                    "yes — bodies older than 500 blocks will be dropped"
                } else {
                    "no — full archive"
                }
            );

            let handle = NodeHandle::start(cfg)?;
            println!("genesis........ {}", handle.genesis_hex());
            println!("node online — Ctrl+C to stop");

            // Stay alive; periodic status
            loop {
                thread::sleep(Duration::from_secs(30));
                if let Ok(s) = handle.status_snapshot() {
                    tracing::info!(
                        blocks = s.blocks,
                        tip = %s.tip,
                        peers = s.peers,
                        mempool = s.mempool,
                        mining = s.mining,
                        "heartbeat"
                    );
                }
            }
        }
    }
    Ok(())
}

fn resolve(datadir: &std::path::Path, p: &PathBuf) -> PathBuf {
    if p.is_absolute() {
        p.clone()
    } else {
        datadir.join(p)
    }
}

fn print_status(
    chain: &nightfall_consensus::Chain,
    datadir: &std::path::Path,
    network: NetworkId,
    peers: usize,
    mempool: usize,
) {
    let magic = network_magic(network);
    println!("coin........... {COIN_NAME} ({TICKER})");
    println!("network........ {network}");
    println!("datadir........ {}", datadir.display());
    println!("blocks......... {}", chain.block_count());
    if chain.is_pruned() {
        println!(
            "pruned......... yes — bodies from height {}",
            chain.first_height
        );
    }
    println!(
        "tip_height..... {}",
        chain
            .tip_height()
            .map(|h| h.0.to_string())
            .unwrap_or_else(|| "none".into())
    );
    println!("tip............ {}", chain.tip_hash());
    println!("genesis........ {}", chain.genesis_hash);
    println!("difficulty..... {}", chain.next_difficulty());
    println!(
        "minted......... {}",
        nightfall_types::Amount(chain.total_minted())
    );
    println!(
        "burned_fees.... {}",
        nightfall_types::Amount(chain.ledger.supply.total_burned_darks)
    );
    println!(
        "circulating.... {}",
        nightfall_types::Amount(chain.ledger.supply.circulating())
    );
    println!("utxos.......... {}", chain.ledger.utxos.len());
    println!("kernels........ {}", chain.ledger.kernels.count);
    println!("utxo_root...... {}", chain.ledger.utxo_root());
    println!(
        "supply_proof... {}",
        match chain.verify_supply() {
            Ok(()) => "OK — Σ UTXO − Σ excess == circulating·G".to_string(),
            Err(e) => format!("FAILED: {e}"),
        }
    );
    println!("total_work..... {}", chain.total_work);
    println!("max_supply..... {MAX_SUPPLY_NIGHT} {TICKER}");
    println!("tail........... none");
    println!("reward_era0.... {INITIAL_BLOCK_REWARD_NIGHT} {TICKER}");
    println!("halving_every.. {HALVING_INTERVAL_BLOCKS}");
    println!("peers.......... {peers}");
    println!("mempool........ {mempool}");
    println!(
        "p2p_magic...... {:02x}{:02x}{:02x}{:02x}",
        magic[0], magic[1], magic[2], magic[3]
    );
}

fn load_or_create_miner(path: &PathBuf) -> anyhow::Result<WalletKeys> {
    if path.exists() {
        let hex_seed = fs::read_to_string(path)?;
        let bytes = hex::decode(hex_seed.trim())?;
        if bytes.len() != 32 {
            anyhow::bail!("miner seed must be 32 bytes hex");
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        Ok(WalletKeys::from_seed(seed))
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let keys = WalletKeys::generate();
        // Owner-only permissions: this file is the private key to every coin
        // the miner ever earns.
        nightfall_storage::write_secret_file(path, &hex::encode(keys.seed))?;
        println!("wrote new miner seed {} (mode 0600)", path.display());
        Ok(keys)
    }
}
