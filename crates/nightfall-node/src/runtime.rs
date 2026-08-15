//! Node state, P2P server, and the mining loop.

use crate::rpc;
use crate::session::{
    fanout_block, fanout_tx, inbound_key, outbound_key, SessionHandle, SessionPool,
};
use nightfall_consensus::{Block, BlockTemplate, Chain, Mempool};
use nightfall_crypto::{default_threads, mine_parallel, Address};
use nightfall_ledger::Transaction;
use nightfall_p2p::{
    broadcast_block, connect_peer, dialable_addr, handshake, read_msg, write_msg, PeerMsg,
    MAX_BLOCKS_PER_REQUEST, MAX_PEERS_PER_MSG,
};
use nightfall_storage::{now_unix, ChainStore};
use nightfall_types::NetworkId;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

pub type SharedState = Arc<Mutex<NodeInner>>;

/// Maximum simultaneous peer connections (live sessions + address book).
const MAX_PEERS: usize = 64;
/// How often live sessions are asked for their tip. Push is the main path;
/// this is the safety net for a missed `InvBlock`.
const STATUS_TICK: Duration = Duration::from_secs(4);

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
    /// Blocks that did not extend our tip but whose parent we hold, keyed by
    /// the parent's hash so a branch can be walked forward and weighed.
    ///
    /// Bounded, and cleared whenever the tip moves — this is a scratch pad for
    /// resolving a fork in progress, not a second chain store.
    pub branch: HashMap<[u8; 32], Block>,
    /// Highest tip height any peer has told us about.
    ///
    /// Learned from handshakes, in both directions. Used to hold mining back
    /// while we are behind, so nobody spends hashes extending a tip the network
    /// has already left. Decays back to our own height when peers vanish, so a
    /// node whose peers went away can still mine rather than stalling forever.
    pub best_peer_height: u64,
    /// When `best_peer_height` was last confirmed by a peer.
    pub best_peer_seen: u64,
    /// When we last made progress towards a peer's reported height.
    ///
    /// Reset whenever our tip moves. If it stops moving while a peer still
    /// claims to be ahead, the gap is a fork rather than lag, and mining
    /// resumes rather than waiting on something unreachable.
    pub behind_since: u64,
    /// Software version each peer announced, by address.
    ///
    /// The handshake has always carried an agent string and the node has always
    /// thrown it away, which meant the one question worth asking during an
    /// incident — *what is everyone else running?* — had no answer anywhere.
    /// A network of a dozen nodes where a known-bad release is still mining is
    /// a very different situation from one where everybody upgraded, and until
    /// now the two were indistinguishable from the inside.
    pub peer_agents: HashMap<String, String>,
    /// Set while a reorg candidate is being rebuilt and verified.
    ///
    /// Rebuilding is measured in tens of seconds on a chain of any size, and
    /// every peer thread that fails to connect a block reaches the same
    /// conclusion at the same moment. Without this they all start, and because
    /// each one finishes by taking the state lock to swap chains, the node
    /// spends minutes doing the same arithmetic n times to arrive at the answer
    /// the first one already had. One at a time; the rest skip the round.
    pub reorg_in_flight: Arc<AtomicBool>,
    /// Unix time of the last side-channel reorg fetch. A peer that is
    /// ahead on a *fork* cannot be caught by `GetBlocks` — those blocks
    /// do not connect to our tip — so we pull their chain on a fresh
    /// socket. Without a throttle, every 4-second Status tick would
    /// start the same download.
    pub last_reorg_fetch: AtomicU64,
    /// Sockets that are open right now. Announce writes here. A wallet behind
    /// NAT stays in real time because its outbound to a seed lives in this
    /// pool, not because anyone can dial it back.
    pub sessions: Arc<SessionPool>,
    /// Generation counter + condvar. Bumped with the tip so a wallet scan
    /// thread can sleep until there is something new to look at, instead of
    /// polling the whole chain every few seconds.
    pub tip_notify: Arc<(Mutex<u64>, Condvar)>,
}

/// Upper bound on buffered branch blocks. A fork deeper than this is past
/// MAX_REORG_DEPTH territory anyway, and an unbounded map is somewhere a peer
/// could put arbitrary data.
const MAX_BRANCH_BLOCKS: usize = 600;

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

    fn bump_tip(&mut self) {
        self.tip_epoch.fetch_add(1, Ordering::SeqCst);
        // Our chain moved, so whatever gap remains is being closed. Anything
        // that stops moving while a peer claims more is a fork, and the mining
        // hold-off gives up on it — see MAX_CATCHUP_WAIT_SECS.
        self.behind_since = now_unix();
        if let Ok(mut gen) = self.tip_notify.0.lock() {
            *gen = gen.wrapping_add(1);
        }
        self.tip_notify.1.notify_all();
    }

    pub fn submit_tx(&mut self, tx: Transaction) -> Result<String, String> {
        self.chain.precheck_tx(&tx).map_err(|e| e.to_string())?;
        let id = tx.txid().to_hex();
        if !self.mempool.insert(tx.clone()) {
            return Ok(id);
        }
        // Live sockets first. A NAT peer that is holding a connection to us
        // hears the transaction immediately; dialling their listen port would
        // just time out.
        fanout_tx(&self.sessions.all(), &tx);
        if self.sessions.is_empty() {
            fallback_dial_tx(&self.dialable_peers(), &tx, self);
        }
        Ok(id)
    }

    pub fn dialable_peers(&self) -> Vec<String> {
        // Official seeds always stay in the set. A genesis-mismatch drop
        // removes a learned address; it must not also forget the compiled-in
        // names, because those are the ones that come back on the right chain
        // after an upgrade.
        self.network
            .seed_nodes()
            .iter()
            .map(|s| s.to_string())
            .chain(self.bootstrap.iter().cloned())
            .chain(self.peer_addrs.iter().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .take(MAX_PEERS)
            .collect()
    }

    /// Stop dialling an address that can never peer with us.
    ///
    /// After a chain reset the old network is still out there: same ports,
    /// same `peers.json` entries, different genesis. Paying a handshake to
    /// each of them every round is how nodes fell a minute behind a miner
    /// on the same LAN while every log line looked healthy.
    fn forget_incompatible(&mut self, addr: &str) {
        self.peer_addrs.remove(addr);
        self.bootstrap.retain(|a| a != addr);
        self.sessions.remove(addr);
        self.sessions.remove(&outbound_key(addr));
        self.sessions.remove(&inbound_key(addr));
        tracing::info!("dropped {addr}: incompatible genesis");
    }

    /// Announce a block on every live socket.
    ///
    /// The old path dialled each listen address from scratch. That is how a
    /// miner behind NAT never heard the next block: nobody could complete the
    /// SYN. Writing the block onto the socket the peer already opened is the
    /// whole difference between "in real time" and "on the next poll".
    pub fn announce_block(&self, block: Block) {
        let live = self.sessions.all();
        if live.is_empty() {
            fallback_dial_block(&self.dialable_peers(), &block, self);
            return;
        }
        fanout_block(&live, &block);
    }
}

/// Last-resort dial when we have no live sessions yet (process just started,
/// or every socket dropped at once). Seeds only — learned NAT addresses will
/// not answer.
fn fallback_dial_block(peers: &[String], block: &Block, inner: &NodeInner) {
    let network = inner.network;
    let genesis = inner.chain.genesis_hash;
    let height = inner.chain.tip_height().map(|h| h.0).unwrap_or(0);
    let tip = inner.chain.tip_hash();
    let port = inner.listen_port;
    for addr in peers.iter().take(4) {
        let block = block.clone();
        let addr = addr.clone();
        thread::spawn(move || {
            if let Ok(mut s) = connect_peer(&addr, 3000) {
                if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                    let _ = broadcast_block(&mut s, &block);
                }
            }
        });
    }
}

fn fallback_dial_tx(peers: &[String], tx: &Transaction, inner: &NodeInner) {
    let network = inner.network;
    let genesis = inner.chain.genesis_hash;
    let height = inner.chain.tip_height().map(|h| h.0).unwrap_or(0);
    let tip = inner.chain.tip_hash();
    let port = inner.listen_port;
    for addr in peers.iter().take(4) {
        let tx = tx.clone();
        let addr = addr.clone();
        thread::spawn(move || {
            if let Ok(mut s) = connect_peer(&addr, 3000) {
                if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                    let _ = nightfall_p2p::broadcast_tx(&mut s, &tx);
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
    /// How far behind the best peer, so the UI can say why mining is paused
    /// instead of appearing to do nothing.
    pub blocks_behind: u64,
    /// Sockets that are actually open, not entries in the address book.
    pub live_peers: usize,
}

pub struct NodeHandle {
    state: SharedState,
    mining_enabled: Arc<AtomicBool>,
    tip_notify: Arc<(Mutex<u64>, Condvar)>,
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
        let sessions = Arc::new(SessionPool::new());
        let tip_notify = Arc::new((Mutex::new(0u64), Condvar::new()));

        let inner = NodeInner {
            chain,
            mempool: Mempool::default(),
            store,
            network: cfg.network,
            // Restored from peers.json so a peer added last session is
            // actually dialled this session. Without this they lived only
            // in `bootstrap`, and the sync loop never looked there — the
            // "peers survive a restart" fix wrote the file and then ignored
            // it on the way back in.
            peer_addrs: load_known_peers(&cfg.datadir).into_iter().collect(),
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
            branch: HashMap::new(),
            best_peer_height: 0,
            best_peer_seen: 0,
            // Not zero: `now − 0` is already older than the catch-up window,
            // so a freshly opened wallet would mine on a stale tip the moment
            // someone pressed Start — the exact case this field exists to stop.
            behind_since: now_unix(),
            peer_agents: HashMap::new(),
            reorg_in_flight: Arc::new(AtomicBool::new(false)),
            last_reorg_fetch: AtomicU64::new(0),
            sessions: Arc::clone(&sessions),
            tip_notify: Arc::clone(&tip_notify),
        };
        let state: SharedState = Arc::new(Mutex::new(inner));

        {
            let st = Arc::clone(&state);
            let listen = cfg.p2p_listen.clone();
            thread::spawn(move || p2p_listen_loop(listen, st));
        }

        rpc::spawn_rpc(cfg.rpc_listen.clone(), Arc::clone(&state));

        spawn_outbound_supervisor(Arc::clone(&state));
        spawn_status_ticker(Arc::clone(&state));

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
            tip_notify,
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
            blocks_behind: catchup_behind(&g).unwrap_or(0),
            live_peers: g.sessions.len(),
        })
    }

    /// Sleep until the tip moves, or `timeout` elapses. Returns the current
    /// generation so the caller can wait again without missing a bump.
    pub fn wait_tip_change(&self, seen: u64, timeout: Duration) -> u64 {
        let (lock, cv) = &*self.tip_notify;
        let Ok(guard) = lock.lock() else {
            return seen;
        };
        if *guard != seen {
            return *guard;
        }
        match cv.wait_timeout(guard, timeout) {
            Ok((g, _)) => *g,
            Err(_) => seen,
        }
    }

    pub fn tip_generation(&self) -> u64 {
        self.tip_notify.0.lock().map(|g| *g).unwrap_or(0)
    }
}

// -------------------------------------------------------- live sessions ---

fn spawn_outbound_supervisor(state: SharedState) {
    thread::spawn(move || {
        let mut launched: HashSet<String> = HashSet::new();
        loop {
            let targets = outbound_targets(&state);
            for addr in targets {
                if launched.contains(&addr) {
                    continue;
                }
                launched.insert(addr.clone());
                let st = Arc::clone(&state);
                thread::spawn(move || stay_connected(st, addr));
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn outbound_targets(state: &SharedState) -> Vec<String> {
    let Ok(g) = state.lock() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in g.network.seed_nodes() {
        out.push((*s).to_string());
    }
    for a in g.dialable_peers() {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

fn stay_connected(state: SharedState, addr: String) {
    let mut backoff = Duration::from_millis(250);
    loop {
        match open_outbound_session(&state, &addr) {
            Ok(()) => {
                backoff = Duration::from_millis(250);
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("genesis mismatch")
                    || msg.contains("network mismatch")
                    || msg.contains("self-dial")
                {
                    if msg.contains("genesis") || msg.contains("network") {
                        if let Ok(mut g) = state.lock() {
                            g.forget_incompatible(&addr);
                        }
                    }
                    tracing::info!("outbound {addr} abandoned: {msg}");
                    return;
                }
                tracing::debug!("outbound {addr}: {e}");
            }
        }
        thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(8));
    }
}

fn is_self_connection(stream: &TcpStream, listen_port: u16) -> bool {
    match (stream.local_addr(), stream.peer_addr()) {
        (Ok(local), Ok(peer)) => local.ip() == peer.ip() && peer.port() == listen_port,
        _ => false,
    }
}

fn open_outbound_session(state: &SharedState, addr: &str) -> anyhow::Result<()> {
    let mut stream = connect_peer(addr, 8_000)?;
    prepare_live_socket(&stream)?;

    let (network, genesis, our_h, next, tip, port) = {
        let g = state.lock().unwrap();
        (
            g.network,
            g.chain.genesis_hash,
            g.chain.tip_height().map(|h| h.0).unwrap_or(0),
            next_needed_height(&g),
            g.chain.tip_hash(),
            g.listen_port,
        )
    };

    if is_self_connection(&stream, port) {
        anyhow::bail!("self-dial");
    }

    let (peer_h, peer_tip) = handshake(&mut stream, network, genesis, our_h, tip, port)?;
    note_peer_height(state, peer_h);
    state.lock().unwrap().peer_addrs.insert(addr.to_string());

    let write_clone = stream.try_clone()?;
    let sess = {
        let g = state.lock().unwrap();
        g.sessions.insert(outbound_key(addr), write_clone, true)
    };
    tracing::info!("outbound session {addr} height={peer_h}");

    // Catch up on this socket before we sit in the read loop. A NAT wallet
    // that just opened Core is often tens of blocks behind; one GetBlocks
    // here, then the loop chains pages if the peer is still ahead.
    if peer_h + 1 > next || peer_tip != tip.to_hex() {
        let _ = sess.send(&PeerMsg::GetBlocks {
            from_height: next,
            limit: MAX_BLOCKS_PER_REQUEST,
        });
    } else if peer_h < our_h {
        // They are behind us. Push from their tip on this same session —
        // do not do request/response on it, the read loop is about to own
        // the socket. A fork is handled when the first pushed block fails
        // to connect and `consider_branch` runs on their side.
        push_blocks_via_session(state, &sess, peer_h, addr);
    }

    let mut reader = BufReader::new(stream);
    let result = peer_io_loop(&mut reader, &sess, state, addr, peer_h);
    if let Ok(g) = state.lock() {
        g.sessions.remove(&outbound_key(addr));
    }
    result
}

/// Height of the next block this chain will accept. 0 on an empty node.
fn next_needed_height(inner: &NodeInner) -> u64 {
    inner
        .chain
        .tip_height()
        .map(|h| h.0.saturating_add(1))
        .unwrap_or(0)
}

const REORG_FETCH_COOLDOWN_SECS: u64 = 15;

/// Pull a peer's chain on a fresh socket and weigh it. Used when live
/// `GetBlocks` cannot extend our tip — we are on a fork, not merely late.
fn kick_reorg_fetch(state: &SharedState, sess: &SessionHandle) {
    if !sess.outbound {
        return;
    }
    let addr = sess
        .key
        .strip_prefix("out:")
        .unwrap_or(&sess.key)
        .to_string();
    if addr.starts_with("in:") {
        return;
    }
    let now = now_unix();
    {
        let Ok(g) = state.lock() else { return };
        let last = g.last_reorg_fetch.load(Ordering::Relaxed);
        if now.saturating_sub(last) < REORG_FETCH_COOLDOWN_SECS {
            return;
        }
        g.last_reorg_fetch.store(now, Ordering::Relaxed);
    }
    tracing::info!("peer {addr} is ahead on a fork — fetching their chain");
    let st = Arc::clone(state);
    thread::spawn(move || {
        if let Err(e) = sync_from_peer(&st, &addr) {
            tracing::debug!("reorg fetch {addr}: {e}");
        }
    });
}

fn spawn_status_ticker(state: SharedState) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        loop {
            let sessions = {
                let Ok(g) = state.lock() else {
                    thread::sleep(STATUS_TICK);
                    continue;
                };
                g.sessions.all()
            };
            for s in sessions {
                if let Err(e) = s.send(&PeerMsg::GetStatus) {
                    tracing::debug!("status tick {}: {e}", s.key);
                }
            }
            thread::sleep(STATUS_TICK);
        }
    });
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
                let live = state.lock().map(|g| g.sessions.len()).unwrap_or(MAX_PEERS);
                if live >= MAX_PEERS {
                    tracing::debug!("session limit reached, dropping connection");
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
    prepare_live_socket(&stream)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut hello_writer = stream.try_clone()?;

    let (network, genesis) = {
        let g = state.lock().unwrap();
        (g.network, g.chain.genesis_hash)
    };

    let (session_key, peer_height) = match read_msg(&mut reader)? {
        PeerMsg::Hello {
            wire,
            network: net,
            genesis: g,
            listen_port,
            height: peer_height,
            agent,
            ..
        } => {
            // Inbound never checked this, only outbound did — so the wire
            // version was a suggestion rather than a gate. An old node simply
            // dialled us instead of being dialled, and was served normally.
            // Refusing on one side only is not refusing.
            if wire != nightfall_types::WIRE_VERSION {
                tracing::info!(
                    "refused {peer_label} running {agent}: wire v{wire}, we need \
                     v{}",
                    nightfall_types::WIRE_VERSION
                );
                if let Some(addr) = dialable_addr(&peer_label, listen_port) {
                    if let Ok(mut st) = state.lock() {
                        st.forget_incompatible(&addr);
                    }
                }
                write_msg(
                    &mut hello_writer,
                    &PeerMsg::Error {
                        message: format!(
                            "wire version mismatch — you speak v{wire}, this network \
                             speaks v{}. Update from https://nightfallcoin.org; \
                             your coins are unaffected.",
                            nightfall_types::WIRE_VERSION
                        ),
                    },
                )?;
                return Ok(());
            }
            if net != network {
                write_msg(
                    &mut hello_writer,
                    &PeerMsg::Error {
                        message: "network mismatch".into(),
                    },
                )?;
                return Ok(());
            }
            if g != genesis.to_hex() {
                // Drop them rather than keeping the address around to dial
                // again every round. After a reset the old network is still
                // out there, still listening, and still costing a socket
                // timeout per attempt for a handshake that cannot succeed.
                if let Some(addr) = dialable_addr(&peer_label, listen_port) {
                    if let Ok(mut st) = state.lock() {
                        st.forget_incompatible(&addr);
                    }
                }
                write_msg(
                    &mut hello_writer,
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
                    tracing::info!("learned peer address {addr} running {agent}");
                }
                if g.peer_agents.len() < MAX_PEERS * 2 {
                    g.peer_agents.insert(addr, agent.clone());
                }
            }
            // Always key inbound sockets by the observed connection, never
            // by the advertised listen address. See `outbound_key`.
            let session_key = inbound_key(&peer_label);
            note_peer_height(&state, peer_height);

            let (height, tip, our_port) = {
                let g = state.lock().unwrap();
                (
                    g.chain.tip_height().map(|h| h.0).unwrap_or(0),
                    g.chain.tip_hash().to_hex(),
                    g.listen_port,
                )
            };
            write_msg(
                &mut hello_writer,
                &PeerMsg::HelloOk {
                    wire: nightfall_types::WIRE_VERSION,
                    network,
                    genesis: genesis.to_hex(),
                    height,
                    tip,
                    listen_port: our_port,
                },
            )?;
            (session_key, peer_height)
        }
        other => {
            write_msg(
                &mut hello_writer,
                &PeerMsg::Error {
                    message: format!("expected hello, got {other:?}"),
                },
            )?;
            return Ok(());
        }
    };

    let sess = {
        let g = state.lock().unwrap();
        g.sessions.insert(session_key.clone(), hello_writer, false)
    };
    let result = peer_io_loop(&mut reader, &sess, &state, &peer_label, peer_height);
    if let Ok(g) = state.lock() {
        g.sessions.remove(&session_key);
    }
    result
}

fn prepare_live_socket(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    // Long read timeout: the peer may be quiet between 15-second blocks.
    // Writes are short so a stuck announce cannot pin a session forever.
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    Ok(())
}

fn peer_io_loop(
    reader: &mut BufReader<TcpStream>,
    sess: &SessionHandle,
    state: &SharedState,
    peer_label: &str,
    mut last_peer_height: u64,
) -> anyhow::Result<()> {
    loop {
        let msg = match read_msg(reader) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                sess.send(&PeerMsg::Ping { nonce: now_unix() })?;
                continue;
            }
            // A peer that sends us garbage or an oversized frame is dropped,
            // not tolerated in a retry loop.
            Err(e) => return Err(e.into()),
        };

        match msg {
            PeerMsg::Ping { nonce } => sess.send(&PeerMsg::Pong { nonce })?,
            PeerMsg::Pong { .. } => {}

            PeerMsg::GetPeers => {
                let addrs: Vec<String> = {
                    let g = state.lock().unwrap();
                    g.dialable_peers()
                        .into_iter()
                        .take(MAX_PEERS_PER_MSG)
                        .collect()
                };
                sess.send(&PeerMsg::Peers { addrs })?;
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
                sess.send(&PeerMsg::Blocks { blocks })?;
            }

            PeerMsg::GetStatus => {
                let g = state.lock().unwrap();
                sess.send(&PeerMsg::Status {
                    height: g.chain.tip_height().map(|h| h.0).unwrap_or(0),
                    tip: g.chain.tip_hash().to_hex(),
                    bits: g.chain.next_difficulty() as u32,
                    peers: g.sessions.len(),
                    mempool: g.mempool.len(),
                })?;
            }

            PeerMsg::Status { height, tip, .. } => {
                last_peer_height = height;
                note_peer_height(state, height);
                let (next, our_tip) = {
                    let g = state.lock().unwrap();
                    (next_needed_height(&g), g.chain.tip_hash().to_hex())
                };
                if height + 1 > next {
                    // Ask for blocks we do not yet have. Starting at
                    // `tip − 1` used to include a block we already hold;
                    // apply_block then returned BadHeight and the rest of
                    // the page was thrown away — a node 120 blocks behind
                    // on a one-block fork sat there forever.
                    sess.send(&PeerMsg::GetBlocks {
                        from_height: next,
                        limit: MAX_BLOCKS_PER_REQUEST,
                    })?;
                } else if tip != our_tip && sess.outbound {
                    kick_reorg_fetch(state, sess);
                }
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
                            g.branch.clear();
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
                } else {
                    // It did not extend our tip. If its parent is a block we
                    // hold, this is a competing branch rather than junk, and
                    // refusing to look at it is how a fork becomes permanent.
                    consider_branch(state, block, peer_label);
                }
            }

            PeerMsg::Tx { tx } => {
                let newly = {
                    let mut g = state.lock().unwrap();
                    match g.chain.precheck_tx(&tx) {
                        Ok(()) => g.mempool.insert(tx.clone()),
                        Err(e) => {
                            tracing::debug!("reject tx: {e}");
                            false
                        }
                    }
                };
                if newly {
                    let g = state.lock().unwrap();
                    fanout_tx(&g.sessions.all(), &tx);
                }
            }

            PeerMsg::InvBlock { height, .. } => {
                last_peer_height = last_peer_height.max(height);
                note_peer_height(state, height);
                let next = {
                    let g = state.lock().unwrap();
                    next_needed_height(&g)
                };
                if height + 1 > next {
                    sess.send(&PeerMsg::GetBlocks {
                        from_height: next,
                        limit: MAX_BLOCKS_PER_REQUEST,
                    })?;
                }
            }

            PeerMsg::Blocks { blocks } => {
                if blocks.len() > MAX_BLOCKS_PER_REQUEST {
                    return Err(anyhow::anyhow!("peer sent an oversized block batch"));
                }
                let n = {
                    let mut g = state.lock().unwrap();
                    match g.chain.try_ingest_blocks(blocks, now_unix()) {
                        Ok(n) => {
                            if n > 0 {
                                g.bump_tip();
                                let _ = g.persist();
                                tracing::info!("extended +{n} from {peer_label}");
                            }
                            n
                        }
                        Err(_) => 0,
                    }
                };
                // Keep pulling on this live socket until we catch the peer.
                // One 128-block page per Status tick would take a minute to
                // close a 2,000-block gap; chaining pages closes it now.
                let next = {
                    let g = state.lock().unwrap();
                    next_needed_height(&g)
                };
                if n > 0 {
                    if last_peer_height + 1 > next {
                        sess.send(&PeerMsg::GetBlocks {
                            from_height: next,
                            limit: MAX_BLOCKS_PER_REQUEST,
                        })?;
                    }
                } else if last_peer_height + 1 > next {
                    // The page did not connect. Either we already have those
                    // heights (a competing block at our tip) or we are on a
                    // lighter fork. GetBlocks cannot fix that — pull and
                    // weigh their chain.
                    kick_reorg_fetch(state, sess);
                }
            }

            _ => {}
        }
    }
}

/// Remember the best tip height a peer has claimed.
///
/// Only ever raised by a peer that is genuinely ahead, and stamped with the
/// time so it can expire. A peer could lie about being ahead — the cost of
/// believing one is that mining pauses, not that anything invalid is accepted,
/// and a node that pauses is strictly safer than one that forks.
fn note_peer_height(state: &SharedState, height: u64) {
    if let Ok(mut g) = state.lock() {
        let now = now_unix();
        // Expire a stale claim, so a peer that disappeared cannot stall mining
        // forever on a height nobody can serve any more.
        if now.saturating_sub(g.best_peer_seen) > PEER_HEIGHT_TTL_SECS {
            g.best_peer_height = 0;
        }
        let ours = g.chain.tip_height().map(|h| h.0).unwrap_or(0);
        let was_behind = g.best_peer_height > ours;
        if height > g.best_peer_height {
            g.best_peer_height = height;
        }
        if height >= g.best_peer_height {
            g.best_peer_seen = now;
        }
        // The catch-up clock starts when we first *learn* we are behind, not
        // when the process started. A slow first handshake would otherwise
        // burn the whole window before anyone had spoken.
        if g.best_peer_height > ours && !was_behind {
            g.behind_since = now;
        }
    }
}

/// How long a peer's claimed height stays believed without confirmation.
const PEER_HEIGHT_TTL_SECS: u64 = 120;

/// Weigh a block that forks from a chain we already hold.
///
/// The receiving side of a push cannot ask for anything: it has one block at a
/// time, arriving in order. So each rejected block whose parent we know is kept
/// by parent hash, the branch is walked forward as far as it goes, and the
/// result is offered to `maybe_reorg_to` — which applies the ordinary rule,
/// cumulative work, and rebuilds from genesis with full validation. Nothing
/// here shortcuts a check; it only gives the existing check something to look
/// at.
fn consider_branch(state: &SharedState, block: Block, peer_label: &str) {
    let mut g = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    // Keep it first, then look for where the branch meets our chain.
    //
    // Only the first block of a branch has a parent we hold; every one after it
    // descends from a block that exists solely in this buffer. Requiring a
    // known parent before storing therefore discarded the entire branch after
    // its first block — the run could never grow past one, and never outweigh
    // anything.
    if g.branch.len() >= MAX_BRANCH_BLOCKS {
        g.branch.clear();
    }
    g.branch.insert(block.header.prev_hash.0, block);

    // The fork root is the highest block of ours that something in the buffer
    // claims as its parent. Searching from the tip backwards finds a recent
    // fork immediately, which is the common case.
    let Some(fork_at) = g
        .chain
        .blocks
        .iter()
        .rposition(|b| g.branch.contains_key(&b.hash().0))
    else {
        return;
    };

    // Walk forward for as far as the run is contiguous. A gap simply ends it;
    // the next push round fills in what is missing.
    let mut candidate: Vec<Block> = g.chain.blocks[..=fork_at].to_vec();
    let mut cursor = g.chain.blocks[fork_at].hash().0;
    let mut added = 0usize;
    while let Some(next) = g.branch.get(&cursor) {
        candidate.push(next.clone());
        cursor = next.hash().0;
        added += 1;
        if added > MAX_BRANCH_BLOCKS {
            break;
        }
    }
    if added == 0 {
        return;
    }

    // Cheap rejection before rebuilding anything.
    let claimed: u128 = candidate.iter().map(|b| b.work()).sum();
    if claimed <= g.chain.total_work {
        return;
    }

    let before = g.chain.block_count();
    let network = g.chain.network;
    let our_work = g.chain.total_work;
    let our_len = g.chain.blocks.len();
    let guard = Arc::clone(&g.reorg_in_flight);
    drop(g);

    let Some(_flight) = ReorgFlight::begin(&guard) else {
        tracing::debug!("branch from {peer_label} skipped — a reorg is already being verified");
        return;
    };

    // Verified with the lock released. See `Chain::evaluate_reorg`.
    let verdict = Chain::evaluate_reorg(network, our_work, our_len, candidate, now_unix());

    let Ok(g) = state.lock() else { return };
    let mut g = g;
    match verdict {
        Ok(Some(chain)) => {
            if g.chain.adopt_reorg(chain) {
                let after = g.chain.block_count();
                tracing::info!(
                    "reorged onto a heavier branch from {peer_label}: {before} -> {after} blocks, \
                     work {}",
                    g.chain.total_work
                );
                g.branch.clear();
                g.bump_tip();
                let _ = g.persist();
            } else {
                tracing::debug!("branch from {peer_label} lost the race — our chain moved on");
            }
        }
        Ok(None) => {}
        Err(e) => tracing::debug!("branch from {peer_label} not adopted: {e}"),
    }
}

/// Marks a reorg verification as running, and clears the mark on any exit.
///
/// A plain `store(false)` at the end of the function is not enough: the paths
/// out of a reorg include several `?` returns and a panic would leave the flag
/// set forever, which would silently disable reorgs for the rest of the
/// process's life. Nothing would log; the node would simply stop reconciling.
struct ReorgFlight(Arc<AtomicBool>);

impl ReorgFlight {
    fn begin(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(Arc::clone(flag)))
    }
}

impl Drop for ReorgFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Highest height at which we and this peer hold the same block.
///
/// Returns their tip height when we already agree there — the ordinary case of
/// a peer that is merely behind. Otherwise walks backwards in doubling steps,
/// asking for one block at each probe, until a hash matches. That is a handful
/// of round trips rather than a linear scan, and it is bounded by
/// `MAX_REORG_DEPTH` because a fork deeper than that cannot be resolved anyway.
///
/// On any failure it falls back to just below the peer's tip: no worse than the
/// behaviour this replaced.
fn common_ancestor(
    state: &SharedState,
    stream: &mut TcpStream,
    peer_h: u64,
    peer_tip: &str,
) -> u64 {
    let our_hash_at = |h: u64| -> Option<String> {
        let g = state.lock().ok()?;
        g.chain.blocks.get(h as usize).map(|b| b.hash().to_hex())
    };

    // Fast path: we agree at their tip, so they are simply behind.
    if our_hash_at(peer_h).as_deref() == Some(peer_tip) {
        return peer_h;
    }

    let mut step = 1u64;
    while step <= nightfall_consensus::MAX_REORG_DEPTH as u64 {
        let probe = match peer_h.checked_sub(step) {
            Some(p) => p,
            None => break,
        };
        let Ok(batch) = nightfall_p2p::request_blocks(stream, probe, 1) else {
            break;
        };
        let Some(theirs) = batch.first() else { break };
        if our_hash_at(probe) == Some(theirs.hash().to_hex()) {
            return probe;
        }
        step = step.saturating_mul(2);
    }

    peer_h.saturating_sub(1)
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
fn push_blocks_to(state: &SharedState, stream: &mut TcpStream, from_height: u64, addr: &str) {
    // `from_height` is the last height we and the peer agree on, so the first
    // block sent is the one immediately after it — the block they can attach.
    let mut from = from_height;
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

fn push_blocks_via_session(
    state: &SharedState,
    sess: &SessionHandle,
    from_height: u64,
    addr: &str,
) {
    let mut from = from_height;
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
            if sess.send_block(&block).is_err() {
                tracing::debug!("push to {addr} ended after {sent} block(s)");
                return;
            }
            sent += 1;
        }
        from = next;
    }
    if sent > 0 {
        tracing::info!("pushed {sent} block(s) to {addr} on live session");
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

    let (peer_h, peer_tip) = match handshake(&mut stream, network, genesis, our_h, tip, port) {
        Ok(v) => v,
        Err(e) => {
            // Inbound already dropped genesis-mismatch peers. Outbound did
            // not, so `peers.json` entries from an abandoned chain were
            // redialled forever — reachable, listening, and useless.
            if e.to_string().contains("genesis mismatch") {
                if let Ok(mut st) = state.lock() {
                    st.forget_incompatible(addr);
                }
            }
            return Err(e.into());
        }
    };
    state.lock().unwrap().peer_addrs.insert(addr.to_string());
    note_peer_height(state, peer_h);

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
        // Where to start pushing from is the whole question.
        //
        // Starting just below their tip is right when they are simply behind on
        // our chain. It is useless when they are on a *branch*: every block we
        // send then has a parent they have never seen, so nothing connects and
        // nothing can be evaluated. That is not theoretical — a two-block fork
        // survived indefinitely this way, with the heavier side pushing into a
        // void every eight seconds.
        //
        // So: find where we last agreed, and push from there.
        let from = common_ancestor(state, &mut stream, peer_h, &peer_tip);
        push_blocks_to(state, &mut stream, from, addr);
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

        // Only one thread verifies a reorg at a time, and the claim is staked
        // before the download rather than after it: eleven peers that all
        // noticed the same divergence would otherwise each pull an entire
        // chain over the wire to prove the same point.
        let (network, our_work, our_len, guard) = {
            let g = state.lock().unwrap();
            (
                g.chain.network,
                g.chain.total_work,
                g.chain.blocks.len(),
                Arc::clone(&g.reorg_in_flight),
            )
        };
        let Some(_flight) = ReorgFlight::begin(&guard) else {
            tracing::debug!("peer {addr} diverges, but a reorg is already being verified");
            break;
        };

        // Pull the peer's chain in full before judging it. Comparing against a
        // single 128-block page can never outweigh a longer local chain, so a
        // node stuck on a lighter fork would never recover.
        let candidate = fetch_full_chain(&mut stream, peer_h, our_len)?;
        if candidate.is_empty() {
            break;
        }

        // Rebuilt and re-verified with the lock released — tens of seconds of
        // work that used to freeze the whole node. See `Chain::evaluate_reorg`.
        let verdict = Chain::evaluate_reorg(network, our_work, our_len, candidate, now_unix());

        let mut g = state.lock().unwrap();
        match verdict {
            Ok(Some(chain)) => {
                if g.chain.adopt_reorg(chain) {
                    g.bump_tip();
                    let _ = g.persist();
                    tracing::info!(
                        "reorged to heavier chain from {addr} — now {} blocks, work {}",
                        g.chain.block_count(),
                        g.chain.total_work
                    );
                } else {
                    tracing::debug!("peer {addr} chain lost the race — ours moved on meanwhile");
                }
            }
            Ok(None) => tracing::debug!("peer {addr} chain is not heavier"),
            Err(e) => tracing::warn!("peer {addr} offered an invalid chain: {e}"),
        }
        break;
    }

    if total_applied > 0 {
        tracing::info!("synced +{total_applied} blocks from {addr}");
    }
    Ok(())
}

/// How many blocks it is worth pulling to judge a peer's chain.
///
/// Enough to hold all of theirs, never more than we would accept anyway.
pub fn reorg_fetch_cap(peer_height: u64, our_len: usize) -> usize {
    let theirs = peer_height.saturating_add(1) as usize;
    theirs.min(our_len.saturating_add(nightfall_consensus::MAX_REORG_DEPTH))
}

/// Download a peer's entire chain, paging until we reach their tip.
///
/// The bound is "as much as could possibly be accepted", and it has to be
/// exactly that. It used to be `MAX_REORG_DEPTH * 4` — a flat 2,000 blocks,
/// chosen when the chain was short enough that the difference never showed.
///
/// The day the chain passed 2,000 blocks, that number became a wall. A node
/// that had diverged asked a peer for its chain, received a truncated 2,000
/// block *prefix* of it, weighed that prefix against its own longer chain,
/// correctly concluded it was lighter, and refused it. Then it did the same
/// thing on the next round, and the next. The peer was not offering a worse
/// chain; we were only ever looking at part of a better one. Two nodes, one
/// plainly heavier, permanently unable to reconcile — and every block mined on
/// the losing side lost with it.
///
/// Found at block 2,057, with a laptop stuck on 2,048 while the seed node it
/// was talking to carried more work and neither would budge.
///
/// `our_len + MAX_REORG_DEPTH` is the right ceiling because it is precisely
/// what [`Chain::evaluate_reorg`] will entertain: anything longer is rejected
/// as too deep, so fetching it would be wasted bandwidth. Anything shorter,
/// including any fixed constant, eventually truncates a legitimate chain.
fn fetch_full_chain(
    stream: &mut TcpStream,
    peer_height: u64,
    our_len: usize,
) -> anyhow::Result<Vec<nightfall_consensus::Block>> {
    let cap = reorg_fetch_cap(peer_height, our_len);

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

/// How long a node will hold mining back while it believes it is behind.
///
/// This is a bound on *how wrong the belief may be*, not a tuning parameter.
/// Peers report a height, not a chain. On a fork the other side reports a
/// number that is unreachable from where we stand — not because we are behind,
/// but because we are somewhere else. Waiting for it means waiting forever, and
/// two nodes on opposite branches will do it to each other simultaneously.
///
/// That is exactly what happened: both chains stopped growing while every node
/// politely waited for the other. So the wait now expires. Mining on a chain
/// that turns out to be the lighter one wastes that miner's own electricity;
/// a network where nobody mines is broken for everyone.
///
/// Raised from 45 seconds to ten minutes in v0.6.0, because 45 was chosen to
/// escape a deadlock that no longer exists. Reorg verification used to run
/// under the node's global lock and could take half a minute on its own, so a
/// node could trivially go 45 seconds without its tip moving while perfectly
/// healthy — and then start mining on a tip the network had already left,
/// producing the fork the timeout was meant to resolve. With verification off
/// the lock, a tip that has not moved in ten minutes while a peer claims more
/// really is stuck, and the escape means what it says.
///
/// The clock measures *time without progress*, not time spent behind:
/// `bump_tip` resets it whenever our chain moves at all. A node that is
/// catching up, however far behind it is, never reaches this.
const MAX_CATCHUP_WAIT_SECS: u64 = 600;

/// How far behind the best peer we are, or `None` if we should mine anyway.
///
/// `None` covers three cases, and the third is the important one:
///
/// - we are level with, or ahead of, every peer;
/// - we have no peers at all — nothing to be behind, and a network has to
///   start somewhere;
/// - we have been waiting too long, which means the gap is a fork rather than
///   lag, and waiting cannot close it.
fn catchup_behind(inner: &NodeInner) -> Option<u64> {
    let ours = inner.chain.tip_height().map(|h| h.0).unwrap_or(0);
    if inner.best_peer_height <= ours {
        return None;
    }
    // Sync is either working, in which case our height keeps moving and this
    // stays fresh, or it is not, in which case waiting achieves nothing.
    if now_unix().saturating_sub(inner.behind_since) > MAX_CATCHUP_WAIT_SECS {
        return None;
    }
    Some(inner.best_peer_height - ours)
}

pub fn blocks_behind(state: &SharedState) -> Option<u64> {
    let g = state.lock().ok()?;
    catchup_behind(&g)
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

        // Do not mine on a chain we know is not the current one.
        //
        // Every block mined while behind a peer is built on a tip that peer has
        // already moved past, so it lands on a branch that loses the moment the
        // two reconcile. The work is not merely wasted, it actively deepens a
        // fork — and it happens exactly when someone is most likely to press
        // Start: right after opening the wallet, before the first sync lands.
        //
        // Mining with *no* peers stays allowed: a network of one has nothing to
        // be behind, and refusing would make a first node impossible. The
        // wallet warns about that case separately, and loudly.
        if let Some(behind) = blocks_behind(&state) {
            tracing::debug!("holding off mining — {behind} block(s) behind a peer");
            thread::sleep(Duration::from_secs(1));
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
