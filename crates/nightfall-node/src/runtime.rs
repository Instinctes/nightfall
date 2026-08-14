//! Node state, P2P server, and the mining loop.

use crate::rpc;
use nightfall_consensus::{Block, BlockTemplate, Chain, Mempool};
use nightfall_crypto::{default_threads, mine_parallel, Address};
use nightfall_ledger::Transaction;
use nightfall_p2p::{
    broadcast_block, broadcast_tx, connect_peer, dialable_addr, handshake, read_msg, write_msg,
    PeerMsg, MAX_BLOCKS_PER_REQUEST, MAX_PEERS_PER_MSG,
};
use nightfall_storage::{now_unix, ChainStore};
use nightfall_types::NetworkId;
use std::collections::HashSet;
use std::fs;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub type SharedState = Arc<Mutex<NodeInner>>;

/// Maximum simultaneous peer connections.
const MAX_PEERS: usize = 64;
/// How often to re-sync from peers. Short, because two miners that drift
/// apart must reconverge before they waste much work.
const SYNC_INTERVAL: Duration = Duration::from_secs(8);

pub struct NodeConfig {
    pub network: NetworkId,
    pub datadir: PathBuf,
    pub p2p_listen: String,
    pub rpc_listen: String,
    pub connect: Vec<String>,
    pub mine: bool,
    pub miner: Option<Address>,
}

pub struct NodeInner {
    pub chain: Chain,
    pub mempool: Mempool,
    pub store: ChainStore,
    pub network: NetworkId,
    /// Addresses we can dial back (peers' advertised listen addresses).
    pub peer_addrs: HashSet<String>,
    pub miner: Option<Address>,
    pub bootstrap: Vec<String>,
    /// Port we accept connections on, advertised in every handshake.
    pub listen_port: u16,
    pub mining_enabled: Arc<AtomicBool>,
    /// Bumped whenever the tip changes. The miner watches this to abandon a
    /// stale template instantly.
    pub tip_epoch: Arc<AtomicU64>,
    /// Total hashes computed by this node's miner. The UI samples it to show a
    /// real hashrate instead of a guess.
    pub hashes_total: Arc<AtomicU64>,
    /// Blocks this node mined itself.
    pub blocks_found: Arc<AtomicU64>,
    /// Unix time the node started, for uptime display.
    pub started_at: u64,
}

/// Where known peer addresses are kept between runs.
fn peers_file(datadir: &std::path::Path) -> PathBuf {
    datadir.join("peers.json")
}

/// Addresses to dial on the next start.
///
/// Without this, a peer added by hand lives only as long as the process. The
/// wallet tells someone with no peers to add one; they do; they quit; and the
/// next launch is back to knowing nobody — mining alone again, which is the
/// failure this whole network has already paid for twice. Anything learned or
/// entered is worth keeping.
fn load_known_peers(datadir: &std::path::Path) -> Vec<String> {
    let Ok(raw) = fs::read_to_string(peers_file(datadir)) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&raw)
        .unwrap_or_default()
        .into_iter()
        // Re-validate on the way in: the file is editable, and an address that
        // does not parse should be dropped rather than dialled.
        .filter(|a| a.parse::<std::net::SocketAddr>().is_ok() || a.contains(':'))
        .take(MAX_PEERS)
        .collect()
}

impl NodeInner {
    pub fn persist(&self) -> anyhow::Result<()> {
        self.store.save(&self.chain)
    }

    /// Write known peers out, so the next start is not back to zero.
    pub fn persist_peers(&self, datadir: &std::path::Path) {
        let mut all: Vec<String> = self.dialable_peers();
        all.sort();
        if let Ok(json) = serde_json::to_string_pretty(&all) {
            let path = peers_file(datadir);
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }

    fn bump_tip(&self) {
        self.tip_epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn submit_tx(&mut self, tx: Transaction) -> Result<String, String> {
        self.chain.precheck_tx(&tx).map_err(|e| e.to_string())?;
        let id = tx.txid().to_hex();
        if !self.mempool.insert(tx.clone()) {
            return Ok(id);
        }
        let peers: Vec<String> = self.dialable_peers();
        let (network, genesis, height, tip, port) = (
            self.network,
            self.chain.genesis_hash,
            self.chain.tip_height().map(|h| h.0).unwrap_or(0),
            self.chain.tip_hash(),
            self.listen_port,
        );
        thread::spawn(move || {
            for addr in peers {
                if let Ok(mut s) = connect_peer(&addr, 3000) {
                    if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                        let _ = broadcast_tx(&mut s, &tx);
                    }
                }
            }
        });
        Ok(id)
    }

    pub fn dialable_peers(&self) -> Vec<String> {
        self.bootstrap
            .iter()
            .chain(self.peer_addrs.iter())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .take(MAX_PEERS)
            .collect()
    }

    /// Announce a block. Runs off-thread so block propagation never blocks the
    /// state lock — v4 dialled every peer synchronously while holding it.
    pub fn announce_block(&self, block: Block) {
        let peers = self.dialable_peers();
        let network = self.network;
        let genesis = self.chain.genesis_hash;
        let height = self.chain.tip_height().map(|h| h.0).unwrap_or(0);
        let tip = self.chain.tip_hash();
        let port = self.listen_port;

        thread::spawn(move || {
            for addr in peers {
                if let Ok(mut s) = connect_peer(&addr, 5000) {
                    if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                        let _ = broadcast_block(&mut s, &block);
                    }
                }
            }
        });
    }
}

pub struct StatusSnap {
    pub blocks: u64,
    pub tip: String,
    pub peers: usize,
    pub mempool: usize,
    pub mining: bool,
    pub minted: u64,
    pub burned_fees: u64,
    pub difficulty: u64,
    pub total_work: u128,
    pub utxos: usize,
    pub utxo_root: String,
    pub supply_ok: bool,
    pub hashes_total: u64,
    pub blocks_found: u64,
    pub tip_height: u64,
    pub coinbase_maturity: u64,
    pub kernels: u64,
    pub started_at: u64,
}

pub struct NodeHandle {
    state: SharedState,
    mining_enabled: Arc<AtomicBool>,
}

impl NodeHandle {
    pub fn start(cfg: NodeConfig) -> anyhow::Result<Self> {
        let store = ChainStore::new(&cfg.datadir);
        let chain = store.load_or_new(cfg.network)?;
        store.save(&chain)?;

        let mining_enabled = Arc::new(AtomicBool::new(cfg.mine));
        let tip_epoch = Arc::new(AtomicU64::new(0));
        let hashes_total = Arc::new(AtomicU64::new(0));
        let blocks_found = Arc::new(AtomicU64::new(0));

        let inner = NodeInner {
            chain,
            mempool: Mempool::default(),
            store,
            network: cfg.network,
            peer_addrs: HashSet::new(),
            miner: cfg.miner,
            // Built-in seeds first, then whatever the user configured, then
            // whatever this node knew last time it ran.
            bootstrap: cfg
                .network
                .seed_nodes()
                .iter()
                .map(|s| s.to_string())
                .chain(cfg.connect.iter().cloned())
                .chain(load_known_peers(&cfg.datadir))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect(),
            listen_port: cfg
                .p2p_listen
                .rsplit(':')
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(|| cfg.network.default_p2p_port()),
            mining_enabled: Arc::clone(&mining_enabled),
            tip_epoch: Arc::clone(&tip_epoch),
            hashes_total: Arc::clone(&hashes_total),
            blocks_found: Arc::clone(&blocks_found),
            started_at: now_unix(),
        };
        let state: SharedState = Arc::new(Mutex::new(inner));

        {
            let st = Arc::clone(&state);
            let listen = cfg.p2p_listen.clone();
            thread::spawn(move || p2p_listen_loop(listen, st));
        }

        rpc::spawn_rpc(cfg.rpc_listen.clone(), Arc::clone(&state));

        {
            let st = Arc::clone(&state);
            let peers: Vec<String> = cfg
                .network
                .seed_nodes()
                .iter()
                .map(|s| s.to_string())
                .chain(cfg.connect.iter().cloned())
                .collect();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(200));
                loop {
                    let targets: Vec<String> = {
                        let g = st.lock().unwrap();
                        let mut v = peers.clone();
                        v.extend(g.peer_addrs.iter().cloned());
                        v.into_iter().collect::<HashSet<_>>().into_iter().collect()
                    };
                    for addr in targets {
                        if let Err(e) = sync_from_peer(&st, &addr) {
                            tracing::debug!("sync {addr}: {e}");
                        }
                    }
                    thread::sleep(SYNC_INTERVAL);
                }
            });
        }

        if cfg.miner.is_some() {
            let st = Arc::clone(&state);
            thread::spawn(move || mining_loop(st));
        }

        {
            let st = Arc::clone(&state);
            let datadir = cfg.datadir.clone();
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(30));
                if let Ok(g) = st.lock() {
                    if let Err(e) = g.persist() {
                        tracing::warn!("persist: {e}");
                    }
                    g.persist_peers(&datadir);
                }
            });
        }

        Ok(Self {
            state,
            mining_enabled,
        })
    }

    pub fn genesis_hex(&self) -> String {
        self.state.lock().unwrap().chain.genesis_hash.to_hex()
    }

    pub fn set_mining(&self, on: bool) {
        self.mining_enabled.store(on, Ordering::SeqCst);
    }

    pub fn is_mining(&self) -> bool {
        self.mining_enabled.load(Ordering::SeqCst)
    }

    pub fn shared(&self) -> SharedState {
        Arc::clone(&self.state)
    }

    /// Add a peer to dial. Returns an error for anything that is not a usable
    /// `host:port`, so a typo surfaces immediately instead of silently doing
    /// nothing.
    pub fn add_peer(&self, addr: &str) -> anyhow::Result<()> {
        let addr = addr.trim();
        if addr.is_empty() {
            anyhow::bail!("enter an address as host:port");
        }
        // Accept either a literal socket address or a resolvable hostname.
        use std::net::ToSocketAddrs;
        let resolved = addr
            .to_socket_addrs()
            .map_err(|e| anyhow::anyhow!("{addr} is not reachable: {e}"))?
            .next()
            .ok_or_else(|| anyhow::anyhow!("{addr} resolved to nothing"))?;
        let _ = resolved;

        let mut g = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
        if g.peer_addrs.len() >= MAX_PEERS {
            anyhow::bail!("peer limit reached");
        }
        g.peer_addrs.insert(addr.to_string());
        g.bootstrap.push(addr.to_string());
        tracing::info!("peer added: {addr}");
        Ok(())
    }

    /// Addresses this node will dial.
    pub fn peers(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|g| {
                let mut v = g.dialable_peers();
                v.sort();
                v
            })
            .unwrap_or_default()
    }

    pub fn status_snapshot(&self) -> anyhow::Result<StatusSnap> {
        let g = self.state.lock().unwrap();
        Ok(StatusSnap {
            blocks: g.chain.block_count(),
            tip: g.chain.tip_hash().to_hex(),
            peers: g.peer_addrs.len(),
            mempool: g.mempool.len(),
            mining: g.mining_enabled.load(Ordering::SeqCst),
            minted: g.chain.ledger.supply.total_minted_darks,
            burned_fees: g.chain.ledger.supply.total_burned_darks,
            difficulty: g.chain.next_difficulty(),
            total_work: g.chain.total_work,
            utxos: g.chain.ledger.utxos.len(),
            utxo_root: g.chain.ledger.utxo_root().to_hex(),
            supply_ok: g.chain.verify_supply().is_ok(),
            hashes_total: g.hashes_total.load(Ordering::Relaxed),
            blocks_found: g.blocks_found.load(Ordering::Relaxed),
            tip_height: g.chain.tip_height().map(|h| h.0).unwrap_or(0),
            coinbase_maturity: g.chain.ledger.coinbase_maturity,
            kernels: g.chain.ledger.kernels.count,
            started_at: g.started_at,
        })
    }
}

// -------------------------------------------------------------------- p2p ---

fn p2p_listen_loop(addr: String, state: SharedState) {
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("p2p bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("p2p listening on {addr}");

    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let peer_count = state.lock().map(|g| g.peer_addrs.len()).unwrap_or(0);
                if peer_count >= MAX_PEERS {
                    tracing::debug!("peer limit reached, dropping connection");
                    continue;
                }
                let st = Arc::clone(&state);
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                thread::spawn(move || {
                    if let Err(e) = handle_peer(stream, st, peer.clone()) {
                        tracing::debug!("peer {peer}: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("p2p accept: {e}"),
        }
    }
}

fn handle_peer(stream: TcpStream, state: SharedState, peer_label: String) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    let (network, genesis) = {
        let g = state.lock().unwrap();
        (g.network, g.chain.genesis_hash)
    };

    match read_msg(&mut reader)? {
        PeerMsg::Hello {
            network: net,
            genesis: g,
            listen_port,
            ..
        } => {
            if net != network {
                write_msg(
                    &mut writer,
                    &PeerMsg::Error {
                        message: "network mismatch".into(),
                    },
                )?;
                return Ok(());
            }
            if g != genesis.to_hex() {
                write_msg(
                    &mut writer,
                    &PeerMsg::Error {
                        message: "genesis mismatch".into(),
                    },
                )?;
                return Ok(());
            }
            // Learn an address we can dial back. Without this the peer can
            // reach us but we can never reach them, so our blocks never
            // propagate outward and both nodes fork apart while mining.
            if let Some(addr) = dialable_addr(&peer_label, listen_port) {
                let mut g = state.lock().unwrap();
                if g.peer_addrs.len() < MAX_PEERS {
                    g.peer_addrs.insert(addr.clone());
                    tracing::info!("learned peer address {addr}");
                }
            }

            let (height, tip, our_port) = {
                let g = state.lock().unwrap();
                (
                    g.chain.tip_height().map(|h| h.0).unwrap_or(0),
                    g.chain.tip_hash().to_hex(),
                    g.listen_port,
                )
            };
            write_msg(
                &mut writer,
                &PeerMsg::HelloOk {
                    wire: nightfall_types::WIRE_VERSION,
                    network,
                    genesis: genesis.to_hex(),
                    height,
                    tip,
                    listen_port: our_port,
                },
            )?;
        }
        other => {
            write_msg(
                &mut writer,
                &PeerMsg::Error {
                    message: format!("expected hello, got {other:?}"),
                },
            )?;
            return Ok(());
        }
    }

    loop {
        let msg = match read_msg(&mut reader) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                write_msg(&mut writer, &PeerMsg::Ping { nonce: now_unix() })?;
                continue;
            }
            // A peer that sends us garbage or an oversized frame is dropped,
            // not tolerated in a retry loop.
            Err(e) => return Err(e.into()),
        };

        match msg {
            PeerMsg::Ping { nonce } => write_msg(&mut writer, &PeerMsg::Pong { nonce })?,
            PeerMsg::Pong { .. } => {}

            PeerMsg::GetPeers => {
                let addrs: Vec<String> = {
                    let g = state.lock().unwrap();
                    g.dialable_peers()
                        .into_iter()
                        .take(MAX_PEERS_PER_MSG)
                        .collect()
                };
                write_msg(&mut writer, &PeerMsg::Peers { addrs })?;
            }

            PeerMsg::Peers { addrs } => {
                let mut g = state.lock().unwrap();
                for a in addrs.into_iter().take(MAX_PEERS_PER_MSG) {
                    // Only accept things that parse as a socket address, so a
                    // peer cannot feed us arbitrary strings to dial.
                    if a.parse::<std::net::SocketAddr>().is_ok() && g.peer_addrs.len() < MAX_PEERS {
                        g.peer_addrs.insert(a);
                    }
                }
            }

            PeerMsg::GetBlocks { from_height, limit } => {
                let blocks = {
                    let g = state.lock().unwrap();
                    g.chain
                        .blocks_from(from_height, limit.min(MAX_BLOCKS_PER_REQUEST))
                };
                write_msg(&mut writer, &PeerMsg::Blocks { blocks })?;
            }

            PeerMsg::GetStatus => {
                let g = state.lock().unwrap();
                write_msg(
                    &mut writer,
                    &PeerMsg::Status {
                        height: g.chain.tip_height().map(|h| h.0).unwrap_or(0),
                        tip: g.chain.tip_hash().to_hex(),
                        bits: g.chain.next_difficulty() as u32,
                        peers: g.peer_addrs.len(),
                        mempool: g.mempool.len(),
                    },
                )?;
            }

            PeerMsg::Block { block } => {
                let height = block.header.height.0;
                let applied = {
                    let mut g = state.lock().unwrap();
                    match g.chain.apply_block(block.clone(), now_unix()) {
                        Ok(()) => {
                            g.mempool.remove_included(&block);
                            g.bump_tip();
                            let _ = g.persist();
                            true
                        }
                        Err(e) => {
                            tracing::debug!("block {height} rejected: {e}");
                            false
                        }
                    }
                };
                if applied {
                    tracing::info!("accepted block {height} from {peer_label}");
                    let g = state.lock().unwrap();
                    g.announce_block(block);
                }
                // Deliberately NOT triggering a full chain download here.
                // v4 responded to every rejected block by pulling the peer's
                // entire chain — free bandwidth amplification for any attacker.
                // Divergence is resolved by the periodic sync task instead.
            }

            PeerMsg::Tx { tx } => {
                let mut g = state.lock().unwrap();
                match g.chain.precheck_tx(&tx) {
                    Ok(()) => {
                        g.mempool.insert(tx);
                    }
                    Err(e) => tracing::debug!("reject tx: {e}"),
                }
            }

            PeerMsg::InvBlock { height, .. } => {
                let our_height = {
                    let g = state.lock().unwrap();
                    g.chain.tip_height().map(|h| h.0).unwrap_or(0)
                };
                if height > our_height {
                    write_msg(
                        &mut writer,
                        &PeerMsg::GetBlocks {
                            from_height: our_height.saturating_sub(1),
                            limit: MAX_BLOCKS_PER_REQUEST,
                        },
                    )?;
                }
            }

            PeerMsg::Blocks { blocks } => {
                if blocks.len() > MAX_BLOCKS_PER_REQUEST {
                    return Err(anyhow::anyhow!("peer sent an oversized block batch"));
                }
                let mut g = state.lock().unwrap();
                if let Ok(n) = g.chain.try_ingest_blocks(blocks, now_unix()) {
                    if n > 0 {
                        g.bump_tip();
                        let _ = g.persist();
                        tracing::info!("extended +{n} from {peer_label}");
                    }
                }
            }

            _ => {}
        }
    }
}

/// How many blocks to push to a lagging peer in one sync round.
///
/// Bounded on purpose. Uploading an unlimited run of blocks to whoever asks is
/// bandwidth amplification, and the periodic sync task comes back shortly, so a
/// peer that is far behind catches up over several rounds rather than in one
/// burst that a stranger could trigger at will.
const PUSH_BATCH: usize = 256;

/// Send a lagging peer the blocks it is missing, over a connection we already
/// have open.
///
/// The receiving side applies each block through its normal inbound path, so
/// nothing here bypasses validation: every block is checked against its own
/// rules on arrival, and any that does not connect is simply rejected.
fn push_blocks_to(state: &SharedState, stream: &mut TcpStream, peer_h: u64, addr: &str) {
    // Start one below their tip so a peer sitting on a short fork receives the
    // block that reconnects them, rather than a run they will all reject.
    let mut from = peer_h.saturating_sub(1);
    let mut sent = 0usize;

    while sent < PUSH_BATCH {
        let batch = {
            let Ok(g) = state.lock() else { return };
            g.chain
                .blocks_from(from, (PUSH_BATCH - sent).min(MAX_BLOCKS_PER_REQUEST))
        };
        if batch.is_empty() {
            break;
        }
        let next = batch
            .last()
            .map(|b| b.header.height.0 + 1)
            .unwrap_or(from + 1);

        for block in batch {
            if broadcast_block(stream, &block).is_err() {
                // A peer that hangs up mid-push is not an error worth
                // shouting about; the next round picks up where this stopped.
                tracing::debug!("push to {addr} ended after {sent} block(s)");
                return;
            }
            sent += 1;
        }
        from = next;
    }

    if sent > 0 {
        tracing::info!("pushed {sent} block(s) to {addr}, which was behind");
    }
}

fn sync_from_peer(state: &SharedState, addr: &str) -> anyhow::Result<()> {
    let mut stream = connect_peer(addr, 15_000)?;
    let (network, genesis, our_h, tip, port) = {
        let g = state.lock().unwrap();
        (
            g.network,
            g.chain.genesis_hash,
            g.chain.tip_height().map(|h| h.0).unwrap_or(0),
            g.chain.tip_hash(),
            g.listen_port,
        )
    };

    let (peer_h, peer_tip) = handshake(&mut stream, network, genesis, our_h, tip, port)?;
    state.lock().unwrap().peer_addrs.insert(addr.to_string());

    // Learn about the rest of the network from this peer.
    if write_msg(&mut stream, &PeerMsg::GetPeers).is_ok() {
        let mut reader = std::io::BufReader::new(stream.try_clone()?);
        if let Ok(PeerMsg::Peers { addrs }) = read_msg(&mut reader) {
            let mut g = state.lock().unwrap();
            for a in addrs.into_iter().take(MAX_PEERS_PER_MSG) {
                if a.parse::<std::net::SocketAddr>().is_ok() && g.peer_addrs.len() < MAX_PEERS {
                    g.peer_addrs.insert(a);
                }
            }
        }
    }

    // Nothing to do if we agree.
    if peer_tip == tip.to_hex() {
        return Ok(());
    }

    // They are behind us. Feed them over the connection we already have, and
    // do not bother asking for anything back — there is nothing there to want.
    //
    // Without this, a node can only catch up by dialling, and a peer behind NAT
    // can dial out but cannot be dialled. A fresh seed node whose only contact
    // is a miner behind a router therefore sits at height 0 indefinitely: it
    // has a peer, it reports healthy, it learns a dial-back address it can
    // never reach, and it retries that address forever while the miner keeps
    // mining. Nothing in either log says anything is wrong. Found by standing
    // up the first real seed node and watching it stay empty while connected.
    //
    // This has to happen before the pull loop below: that loop asks the peer
    // for blocks, gets an empty answer from a peer that has none, and breaks
    // immediately — so anything placed inside it never runs in exactly the
    // case that matters.
    if peer_h < our_h {
        push_blocks_to(state, &mut stream, peer_h, addr);
        return Ok(());
    }

    // Fetch forward from just below our tip rather than replaying from genesis.
    // v4 downloaded the whole chain from every peer every 20 seconds.
    let mut from = our_h.saturating_sub(1);
    let mut total_applied = 0usize;

    for _ in 0..64 {
        let batch = nightfall_p2p::request_blocks(&mut stream, from, MAX_BLOCKS_PER_REQUEST)?;
        if batch.is_empty() {
            break;
        }
        let next_from = batch
            .last()
            .map(|b| b.header.height.0 + 1)
            .unwrap_or(from + 1);

        let mut g = state.lock().unwrap();
        let applied = g.chain.try_ingest_blocks(batch, now_unix()).unwrap_or(0);
        if applied > 0 {
            total_applied += applied;
            g.bump_tip();
            let _ = g.persist();
            from = next_from;
            continue;
        }
        drop(g);

        // Nothing connected. Either we are already ahead, or we are on a
        // different fork. Only the second case is worth work.
        if peer_h <= our_h {
            break;
        }

        // Pull the peer's chain in full before judging it. Comparing against a
        // single 128-block page can never outweigh a longer local chain, so a
        // node stuck on a lighter fork would never recover.
        let candidate = fetch_full_chain(&mut stream, peer_h)?;
        if candidate.is_empty() {
            break;
        }

        let mut g = state.lock().unwrap();
        match g.chain.maybe_reorg_to(candidate, now_unix()) {
            Ok(true) => {
                g.bump_tip();
                let _ = g.persist();
                tracing::info!(
                    "reorged to heavier chain from {addr} — now {} blocks, work {}",
                    g.chain.block_count(),
                    g.chain.total_work
                );
            }
            Ok(false) => tracing::debug!("peer {addr} chain is not heavier"),
            Err(e) => tracing::warn!("peer {addr} offered an invalid chain: {e}"),
        }
        break;
    }

    if total_applied > 0 {
        tracing::info!("synced +{total_applied} blocks from {addr}");
    }
    Ok(())
}

/// Download a peer's entire chain, paging until we reach their tip.
///
/// Bounded by [`nightfall_consensus::MAX_REORG_DEPTH`] beyond our own height so
/// a hostile peer cannot make us buffer an unbounded chain.
fn fetch_full_chain(
    stream: &mut TcpStream,
    peer_height: u64,
) -> anyhow::Result<Vec<nightfall_consensus::Block>> {
    let limit = peer_height.saturating_add(1) as usize;
    let cap = limit.min(nightfall_consensus::MAX_REORG_DEPTH * 4);

    let mut all = Vec::with_capacity(cap.min(4096));
    let mut from = 0u64;

    while all.len() < cap {
        let batch = nightfall_p2p::request_blocks(stream, from, MAX_BLOCKS_PER_REQUEST)?;
        if batch.is_empty() {
            break;
        }
        let n = batch.len();
        from = batch
            .last()
            .map(|b| b.header.height.0 + 1)
            .unwrap_or(from + 1);
        all.extend(batch);
        if n < MAX_BLOCKS_PER_REQUEST {
            break;
        }
    }
    Ok(all)
}

fn mining_loop(state: SharedState) {
    let threads = std::env::var("NF_MINING_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(default_threads);

    {
        let params = state
            .lock()
            .map(|g| g.chain.pow_params())
            .unwrap_or_else(|_| nightfall_types::NetworkId::Devnet.pow_params());
        tracing::info!(
            "mining loop started — {threads} thread(s), {} MiB per thread, {} MiB total",
            params.memory_kib / 1024,
            nightfall_crypto::mining_memory_bytes(params, threads) / (1024 * 1024)
        );
    }

    loop {
        let (enabled, tip_epoch, epoch_now) = {
            let g = state.lock().unwrap();
            (
                g.mining_enabled.load(Ordering::SeqCst),
                Arc::clone(&g.tip_epoch),
                g.tip_epoch.load(Ordering::SeqCst),
            )
        };

        if !enabled {
            thread::sleep(Duration::from_millis(300));
            continue;
        }

        // --- build template under the lock (cheap) ---
        let template: Option<BlockTemplate> = {
            let g = state.lock().unwrap();
            match &g.miner {
                None => None,
                Some(miner) => {
                    let txs = g
                        .mempool
                        .select_for_block(nightfall_consensus::MAX_TXS_PER_BLOCK - 1);
                    match g.chain.build_template(miner, txs, now_unix()) {
                        Ok(t) => Some(t),
                        Err(e) => {
                            tracing::warn!("template: {e}");
                            None
                        }
                    }
                }
            }
        };

        let Some(template) = template else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        // --- hash WITHOUT the lock ---
        let difficulty = template.header.difficulty;
        let preimage = template.header.pow_preimage();
        let pow_params = {
            let g = state.lock().unwrap();
            g.chain.pow_params()
        };
        let (mining_flag, hash_counter, found_counter) = {
            let g = state.lock().unwrap();
            (
                Arc::clone(&g.mining_enabled),
                Arc::clone(&g.hashes_total),
                Arc::clone(&g.blocks_found),
            )
        };

        let should_stop =
            || tip_epoch.load(Ordering::SeqCst) != epoch_now || !mining_flag.load(Ordering::SeqCst);

        let start_nonce: u64 = rand::random();
        let Some((nonce, _hash)) = mine_parallel(
            &preimage,
            difficulty,
            start_nonce,
            pow_params,
            threads,
            &should_stop,
            Some(hash_counter.as_ref()),
        ) else {
            // Tip moved or mining was switched off — rebuild on the new tip.
            continue;
        };

        let block = template.clone().seal(nonce);

        // --- submit under the lock (cheap) ---
        let mut g = state.lock().unwrap();
        if g.chain.tip_hash() != template.built_on {
            tracing::debug!("template went stale before submission, discarding");
            continue;
        }
        match g.chain.apply_block(block.clone(), now_unix()) {
            Ok(()) => {
                g.mempool.remove_included(&block);
                g.bump_tip();
                found_counter.fetch_add(1, Ordering::Relaxed);
                let _ = g.persist();
                tracing::info!(
                    "mined block {} hash={} kernels={} difficulty={}",
                    block.header.height.0,
                    block.hash(),
                    block.body.kernels.len(),
                    difficulty
                );
                g.announce_block(block);
            }
            Err(e) => tracing::warn!("our own block was rejected: {e}"),
        }
    }
}
