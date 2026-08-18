//! Node state, P2P server, and the mining loop.

use crate::rpc;
use crate::session::{
    fanout_block, fluff_tx, inbound_key, outbound_key, stem_tx, SessionHandle, SessionPool,
};
use nightfall_consensus::{Block, BlockTemplate, Chain, Mempool};
use nightfall_crypto::{default_threads, mine_parallel, Address};
use nightfall_ledger::Transaction;
use nightfall_p2p::{
    broadcast_block, connect_peer_via, dialable_addr, handshake, is_directory_addr,
    looks_like_dial_target, read_msg, write_msg, PeerMsg, SocksProxy, DEFAULT_TOR_PROXY,
    MAX_BLOCKS_PER_REQUEST, MAX_PEERS_PER_MSG,
};
use nightfall_storage::{now_unix, ChainStore};
use nightfall_types::NetworkId;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

pub type SharedState = Arc<Mutex<NodeInner>>;

/// Soft cap on live sessions. A seed that sits at this number and then
/// refuses Hello is how new wallets freeze on genesis: they never learn
/// another address. 128 still fits a 1 GiB VPS; overflow is handled by
/// evicting a caught-up inbound, not by dropping the newcomer.
const MAX_PEERS: usize = 128;
/// Extra inbound Hellos accepted while at `MAX_PEERS` so a genesis node
/// can introduce itself. After Hello we evict a synced session. Without
/// this burst the next TCP is discarded before we know they need the chain.
const IBD_ACCEPT_BURST: usize = 32;
/// A peer this many blocks from our tip is treated as caught up and can
/// give up its seat for someone who is not.
pub const IBD_BEHIND: u64 = 8;
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
    /// SOCKS5 proxy for outbound P2P (`127.0.0.1:9050` for Tor).
    pub proxy: Option<String>,
    /// Public light-client HTTP API. Empty / None = off.
    pub mobile_listen: Option<String>,
    /// HTTPS directory of listening nodes. `None` uses the compiled mainnet
    /// default. `Some("off")` disables the fetch.
    pub peers_url: Option<String>,
}

/// Where a new install finds listening nodes when the compiled-in seeds
/// are full or filtered. Served by the website Worker over 443.
pub const DEFAULT_PEERS_DIRECTORY: &str = "https://nightfallcoin.org/peers";
/// After an inbound peer is at our tip, wait this long (they get a peer
/// list and a last page of blocks) and then hang up. The seed is a
/// doorbell, not a living room.
pub const INTRO_GRACE_SECS: u64 = 30;

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
    /// Last advertised tip height per live session key (`in:…` / `out:…`).
    session_height: HashMap<String, u64>,
    /// Listen addresses we ourselves completed an outbound handshake to.
    /// Those are the only IPs we publish: we know they answer.
    reachable_listen: HashSet<String>,
    /// Replay from disk is still running. P2P waits; RPC/light already answer.
    pub(crate) loading: bool,
    pub(crate) preview_blocks: u64,
    pub(crate) preview_tip: String,
    pub(crate) last_dial_error: Option<String>,
    /// Next height a catch-up page should request, so two peers do not
    /// fetch the same 128-block slice.
    ibd_from: u64,
    ibd_buffer: BTreeMap<u64, Vec<Block>>,
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
    /// Last time a 1-second ticker stored the wall clock. A jump larger than
    /// [`SLEEP_GAP_SECS`] means the process was frozen (lid closed). Mining
    /// must not resume on the stale tip.
    pub last_wall_tick: AtomicU64,
    /// Sockets that are open right now. Announce writes here. A wallet behind
    /// NAT stays in real time because its outbound to a seed lives in this
    /// pool, not because anyone can dial it back.
    pub sessions: Arc<SessionPool>,
    /// Generation counter + condvar. Bumped with the tip so a wallet scan
    /// thread can sleep until there is something new to look at, instead of
    /// polling the whole chain every few seconds.
    pub tip_notify: Arc<(Mutex<u64>, Condvar)>,
    /// Outbound dials go through this when set. Inbound listen is unchanged.
    pub proxy: Option<SocksProxy>,
    /// Last successful outbound went through the SOCKS proxy.
    pub last_tor_ok: Arc<AtomicBool>,
    /// Transactions in the Dandelion stem phase, waiting to fluff.
    ///
    /// Keyed by txid. The embargo is a few tens of seconds so a stem that
    /// dies mid-path still reaches the network.
    pub stem_embargo: HashMap<String, u64>,
    /// Addresses that completed a handshake this process. Gossip from
    /// `GetPeers` is not enough — that is how a `peers.json` filled with
    /// Tor exits and produced 51 hung SYN_SENT while the seed sat one
    /// socket away.
    pub confirmed_peers: HashSet<String>,
    /// Set when a peer is ahead and their blocks do not connect to our tip.
    /// Mining on that tip deepens a fork; this does not expire with the
    /// catch-up window.
    pub stalled_on_fork: AtomicBool,
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
        let seeds = self
            .network
            .seed_nodes()
            .iter()
            .map(|s| s.to_string())
            .chain(self.bootstrap.iter().cloned());
        let mut all = peers_to_remember(seeds, self.confirmed_peers.iter().cloned());
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
        self.stalled_on_fork.store(false, Ordering::SeqCst);
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
        // Origin always stems: the first hop is one random peer, not a
        // broadcast that paints this node as the sender.
        propagate_from_inner(self, &tx, None, true);
        Ok(id)
    }

    /// Addresses a stranger may be told to dial. Seeds plus nodes that
    /// completed an outbound handshake with us and whose address is
    /// globally routable. This is the protocol directory — not "everyone
    /// who ever connected".
    pub fn publishable_peers(&self) -> Vec<String> {
        merge_directory_peers(
            self.network.seed_nodes().iter().map(|s| s.to_string()),
            self.reachable_listen.iter().cloned(),
            MAX_PEERS_PER_MSG,
        )
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
        self.confirmed_peers.remove(addr);
        self.bootstrap.retain(|a| a != addr);
        self.sessions.remove(addr);
        self.sessions.remove(&outbound_key(addr));
        self.sessions.remove(&inbound_key(addr));
        tracing::info!("dropped {addr}: incompatible genesis");
    }

    fn mark_confirmed(&mut self, addr: String) {
        self.confirmed_peers.insert(addr.clone());
        self.peer_addrs.insert(addr);
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
fn parse_proxy_cfg(raw: Option<&str>) -> anyhow::Result<Option<SocksProxy>> {
    match raw.map(str::trim) {
        None | Some("") => SocksProxy::parse(DEFAULT_TOR_PROXY)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{e}")),
        Some("off") | Some("none") | Some("clearnet") => Ok(None),
        Some(s) => SocksProxy::parse(s)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{e}")),
    }
}

fn fallback_dial_block(peers: &[String], block: &Block, inner: &NodeInner) {
    let network = inner.network;
    let genesis = inner.chain.genesis_hash;
    let height = inner.chain.tip_height().map(|h| h.0).unwrap_or(0);
    let tip = inner.chain.tip_hash();
    let port = inner.listen_port;
    for addr in peers.iter().take(4) {
        let block = block.clone();
        let addr = addr.clone();
        let proxy = inner.proxy.clone();
        thread::spawn(move || {
            if let Ok((mut s, _tor)) = connect_peer_via(&addr, 3000, proxy.as_ref()) {
                if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                    let _ = broadcast_block(&mut s, &block);
                }
            }
        });
    }
}

fn fallback_stem_tx(peers: &[String], tx: &Transaction, inner: &NodeInner) {
    let Some(addr) = peers.iter().next().cloned() else {
        return;
    };
    let network = inner.network;
    let genesis = inner.chain.genesis_hash;
    let height = inner.chain.tip_height().map(|h| h.0).unwrap_or(0);
    let tip = inner.chain.tip_hash();
    let port = inner.listen_port;
    let proxy = inner.proxy.clone();
    let tx = tx.clone();
    thread::spawn(move || {
        if let Ok((mut s, _tor)) = connect_peer_via(&addr, 3000, proxy.as_ref()) {
            if handshake(&mut s, network, genesis, height, tip, port).is_ok() {
                let _ = nightfall_p2p::broadcast_tx(&mut s, &tx);
            }
        }
    });
}

/// Probability a relay stays in the stem phase (Dandelion++).
const DANDELION_STEM_P: f64 = 0.90;
const DANDELION_EMBARGO_MIN: u64 = 12;
const DANDELION_EMBARGO_MAX: u64 = 28;

fn embargo_deadline() -> u64 {
    let span = DANDELION_EMBARGO_MAX - DANDELION_EMBARGO_MIN + 1;
    now_unix() + DANDELION_EMBARGO_MIN + (rand::random::<u64>() % span)
}

fn propagate_from_inner(
    inner: &mut NodeInner,
    tx: &Transaction,
    from_key: Option<&str>,
    origin: bool,
) {
    let stem = origin || rand::random::<f64>() < DANDELION_STEM_P;
    let live = inner.sessions.all();
    if stem && stem_tx(&live, tx, from_key) {
        inner
            .stem_embargo
            .insert(tx.txid().to_hex(), embargo_deadline());
        return;
    }
    if live.is_empty() {
        fallback_stem_tx(&inner.dialable_peers(), tx, inner);
        return;
    }
    fluff_tx(&live, tx, from_key);
    inner.stem_embargo.remove(&tx.txid().to_hex());
}

fn propagate_tx(state: &SharedState, tx: &Transaction, from_key: Option<&str>, origin: bool) {
    if let Ok(mut g) = state.lock() {
        propagate_from_inner(&mut g, tx, from_key, origin);
    }
}

fn spawn_dandelion_fluff(state: SharedState) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(2));
        let now = now_unix();
        let due: Vec<Transaction> = {
            let Ok(mut g) = state.lock() else {
                continue;
            };
            let ids: Vec<String> = g
                .stem_embargo
                .iter()
                .filter(|(_, t)| **t <= now)
                .map(|(id, _)| id.clone())
                .collect();
            let mut txs = Vec::new();
            for id in ids {
                g.stem_embargo.remove(&id);
                if let Some(tx) = g.mempool.txs.get(&id).cloned() {
                    txs.push(tx);
                }
            }
            txs
        };
        for tx in due {
            if let Ok(g) = state.lock() {
                fluff_tx(&g.sessions.all(), &tx, None);
            }
        }
    });
}

#[derive(Clone)]
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
    /// Outbound P2P is going through SOCKS5 (typically Tor).
    pub tor_proxy: bool,
    /// Transaction relay uses Dandelion-class stem/fluff.
    pub dandelion: bool,
    /// Why the last outbound dial failed, if we have no live peers.
    pub last_dial_error: Option<String>,
    /// True while the chain is still being replayed from disk.
    pub loading: bool,
}

pub struct NodeHandle {
    state: SharedState,
    mining_enabled: Arc<AtomicBool>,
    tip_notify: Arc<(Mutex<u64>, Condvar)>,
}

impl NodeHandle {
    pub fn start(cfg: NodeConfig) -> anyhow::Result<Self> {
        let store = ChainStore::new(&cfg.datadir);
        // Own file, already validated: replay without proofs (seconds).
        // File changed or no record: full verify in the background, RPC/light
        // up immediately so a seed restart is not five minutes of 520s.
        let stored = store.blocks_path().exists();
        let trusted = store.is_own_file_trusted();
        let need_slow_replay = stored && !trusted;
        let preview = store.peek_meta();
        let chain = if need_slow_replay {
            Chain::new_fair(cfg.network)?
        } else {
            store.load_or_new(cfg.network)?
        };
        let genesis_hex = preview
            .as_ref()
            .map(|(_, _, g)| g.clone())
            .unwrap_or_else(|| chain.genesis_hash.to_hex());
        if !stored {
            store.save(&chain)?;
        }
        let (preview_blocks, preview_tip) = preview
            .map(|(n, t, _)| (n, t))
            .unwrap_or_else(|| (chain.block_count(), chain.tip_hash().to_hex()));

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
            session_height: HashMap::new(),
            reachable_listen: HashSet::new(),
            loading: need_slow_replay,
            preview_blocks,
            preview_tip,
            last_dial_error: None,
            ibd_from: 0,
            ibd_buffer: BTreeMap::new(),
            reorg_in_flight: Arc::new(AtomicBool::new(false)),
            last_reorg_fetch: AtomicU64::new(0),
            last_wall_tick: AtomicU64::new(now_unix()),
            confirmed_peers: HashSet::new(),
            stalled_on_fork: AtomicBool::new(false),
            sessions: Arc::clone(&sessions),
            tip_notify: Arc::clone(&tip_notify),
            proxy: parse_proxy_cfg(cfg.proxy.as_deref())?,
            last_tor_ok: Arc::new(AtomicBool::new(false)),
            stem_embargo: HashMap::new(),
        };
        let state: SharedState = Arc::new(Mutex::new(inner));

        rpc::spawn_rpc(cfg.rpc_listen.clone(), Arc::clone(&state));
        if let Some(addr) = cfg.mobile_listen.clone().filter(|s| !s.is_empty()) {
            crate::mobile::spawn_mobile(addr, Arc::clone(&state));
        }

        // HTTP directory only — fills the address book. Must not open P2P
        // sockets until the real chain is loaded, or we handshake at genesis.
        spawn_directory_bootstrap(
            Arc::clone(&state),
            cfg.network,
            cfg.peers_url.clone(),
            genesis_hex,
        );

        {
            let st = Arc::clone(&state);
            let datadir = cfg.datadir.clone();
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(30));
                if let Ok(g) = st.lock() {
                    if !g.loading {
                        if let Err(e) = g.persist() {
                            tracing::warn!("persist: {e}");
                        }
                    }
                    g.persist_peers(&datadir);
                }
            });
        }

        {
            // Heartbeat for lid-close detection. Frozen with the rest of the
            // process during sleep; the first iteration after wake sees the
            // jump and clears the stale peer height before mining resumes.
            let st = Arc::clone(&state);
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(1));
                let now = now_unix();
                if let Ok(mut g) = st.lock() {
                    let last = g.last_wall_tick.load(Ordering::Relaxed);
                    if now.saturating_sub(last) > SLEEP_GAP_SECS {
                        apply_clock_jump(&mut g);
                    } else {
                        g.last_wall_tick.store(now, Ordering::Relaxed);
                    }
                }
            });
        }

        if need_slow_replay {
            let st = Arc::clone(&state);
            let listen = cfg.p2p_listen.clone();
            let mine = cfg.miner.is_some();
            let network = cfg.network;
            let datadir = cfg.datadir.clone();
            thread::spawn(move || {
                tracing::info!("loading chain from disk — RPC and the light API are already up");
                let progress = {
                    let st = Arc::clone(&st);
                    move |done: u64, _total: u64| {
                        if let Ok(mut g) = st.lock() {
                            if g.loading {
                                g.preview_blocks = done.max(1);
                            }
                        }
                    }
                };
                match ChainStore::new(&datadir).load_or_new_with_progress(network, progress) {
                    Ok(loaded) => {
                        let blocks = loaded.block_count();
                        let tip = loaded.tip_hash().to_hex();
                        if let Ok(mut g) = st.lock() {
                            g.chain = loaded;
                            g.loading = false;
                            g.preview_blocks = blocks;
                            g.preview_tip = tip.clone();
                            g.ibd_from = next_needed_height(&g);
                            g.bump_tip();
                            let _ = g.persist();
                        }
                        tracing::info!("chain ready ({blocks} blocks, tip {tip}) — opening P2P");
                        spawn_p2p_plane(st, listen, mine);
                    }
                    Err(e) => {
                        tracing::error!("chain load failed: {e}");
                        if let Ok(mut g) = st.lock() {
                            g.last_dial_error = Some(format!("chain load: {e}"));
                        }
                    }
                }
            });
        } else {
            spawn_p2p_plane(
                Arc::clone(&state),
                cfg.p2p_listen.clone(),
                cfg.miner.is_some(),
            );
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
        if !looks_like_dial_target(addr) {
            anyhow::bail!("enter an address as host:port");
        }
        // Through Tor we must not touch the system resolver — that is the
        // DNS leak Dandelion cannot fix. Clearnet still fails fast here.
        let using_proxy = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.proxy.clone())
            .is_some();
        if !using_proxy {
            use std::net::ToSocketAddrs;
            let resolved = addr
                .to_socket_addrs()
                .map_err(|e| anyhow::anyhow!("{addr} is not reachable: {e}"))?
                .next()
                .ok_or_else(|| anyhow::anyhow!("{addr} resolved to nothing"))?;
            let _ = resolved;
        }

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
        let loading = g.loading;
        let blocks = if loading {
            g.preview_blocks
        } else {
            g.chain.block_count()
        };
        let tip = if loading && !g.preview_tip.is_empty() {
            g.preview_tip.clone()
        } else {
            g.chain.tip_hash().to_hex()
        };
        let tip_height = if loading {
            g.preview_blocks.saturating_sub(1)
        } else {
            g.chain.tip_height().map(|h| h.0).unwrap_or(0)
        };
        Ok(StatusSnap {
            blocks,
            tip,
            peers: g.peer_addrs.len(),
            mempool: g.mempool.len(),
            mining: g.mining_enabled.load(Ordering::SeqCst),
            minted: g.chain.ledger.supply.total_minted_darks,
            burned_fees: g.chain.ledger.supply.total_burned_darks,
            difficulty: g.chain.next_difficulty(),
            total_work: g.chain.total_work,
            utxos: g.chain.ledger.utxos.len(),
            utxo_root: g.chain.ledger.utxo_root().to_hex(),
            supply_ok: !loading && g.chain.verify_supply().is_ok(),
            hashes_total: g.hashes_total.load(Ordering::Relaxed),
            blocks_found: g.blocks_found.load(Ordering::Relaxed),
            tip_height,
            coinbase_maturity: g.chain.ledger.coinbase_maturity,
            kernels: g.chain.ledger.kernels.count,
            started_at: g.started_at,
            blocks_behind: if loading {
                0
            } else {
                catchup_behind(&g).unwrap_or(0)
            },
            live_peers: g.sessions.len(),
            tor_proxy: g.proxy.is_some() && g.last_tor_ok.load(Ordering::Relaxed),
            dandelion: true,
            last_dial_error: g.last_dial_error.clone(),
            loading,
        })
    }

    /// Change the outbound SOCKS5 proxy. Existing sockets stay up; new dials
    /// use the new value. Empty string clears it.
    pub fn set_proxy(&self, proxy: Option<&str>) -> anyhow::Result<()> {
        let parsed = parse_proxy_cfg(proxy)?;
        let mut g = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
        g.proxy = parsed;
        Ok(())
    }

    pub fn proxy_addr(&self) -> Option<String> {
        self.state
            .lock()
            .ok()
            .and_then(|g| g.proxy.as_ref().map(|p| p.addr.clone()))
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

/// How many non-seed addresses we will dial at once. A `peers.json` stuffed
/// with Tor exits used to launch one thread per line and leave 50+ sockets
/// in SYN_SENT — the seed, one hop away, starved.
pub const MAX_OUTBOUND_EXTRA: usize = 6;
const DIAL_GIVE_UP: u32 = 5;

fn spawn_outbound_supervisor(state: SharedState) {
    thread::spawn(move || {
        let launched: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            let targets = outbound_targets(&state);
            for addr in targets {
                {
                    let mut live = launched.lock().unwrap();
                    if live.contains(&addr) {
                        continue;
                    }
                    live.insert(addr.clone());
                }
                let st = Arc::clone(&state);
                let slot = Arc::clone(&launched);
                thread::spawn(move || {
                    stay_connected(st, addr.clone());
                    if let Ok(mut live) = slot.lock() {
                        live.remove(&addr);
                    }
                });
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn outbound_targets(state: &SharedState) -> Vec<String> {
    let Ok(g) = state.lock() else {
        return Vec::new();
    };
    let seeds: Vec<String> = g
        .network
        .seed_nodes()
        .iter()
        .map(|s| s.to_string())
        .chain(g.bootstrap.iter().cloned())
        .collect();
    let confirmed: Vec<String> = g.confirmed_peers.iter().cloned().collect();
    let gossip: Vec<String> = g.peer_addrs.iter().cloned().collect();
    outbound_dial_list(&seeds, &confirmed, &gossip, MAX_OUTBOUND_EXTRA)
}

/// Seeds first, then peers that already shook hands, then a short gossip
/// tail. Unbounded gossip is how a laptop spent its sockets on relays that
/// will never speak Nightfall.
pub fn outbound_dial_list(
    seeds: &[String],
    confirmed: &[String],
    gossip: &[String],
    max_extra: usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in seeds {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    let mut extra = 0usize;
    for a in confirmed.iter().chain(gossip.iter()) {
        if extra >= max_extra {
            break;
        }
        if out.contains(a) {
            continue;
        }
        out.push(a.clone());
        extra += 1;
    }
    out
}

/// Addresses worth writing to disk: compiled-in seeds and peers that
/// actually completed a handshake. Gossip is kept in memory for this
/// session only.
pub fn peers_to_remember(
    seeds: impl IntoIterator<Item = String>,
    confirmed: impl IntoIterator<Item = String>,
) -> Vec<String> {
    seeds
        .into_iter()
        .chain(confirmed)
        .collect::<HashSet<_>>()
        .into_iter()
        .take(MAX_PEERS)
        .collect()
}

fn stay_connected(state: SharedState, addr: String) {
    let mut backoff = Duration::from_millis(250);
    let mut fails = 0u32;
    let is_seed = {
        let Ok(g) = state.lock() else {
            return;
        };
        g.network.seed_nodes().iter().any(|s| *s == addr) || g.bootstrap.iter().any(|s| s == &addr)
    };
    loop {
        match open_outbound_session(&state, &addr) {
            Ok(()) => {
                backoff = Duration::from_millis(250);
                fails = 0;
                if let Ok(mut g) = state.lock() {
                    g.last_dial_error = None;
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("self-dial") {
                    if let Ok(mut g) = state.lock() {
                        g.last_dial_error = Some(format!("{addr}: {msg}"));
                    }
                }
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
                fails = fails.saturating_add(1);
                if !is_seed && fails >= DIAL_GIVE_UP {
                    if let Ok(mut g) = state.lock() {
                        g.peer_addrs.remove(&addr);
                    }
                    tracing::debug!("outbound {addr}: giving up after {fails} failures");
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
    let proxy = state.lock().ok().and_then(|g| g.proxy.clone());
    let (mut stream, used_tor) = connect_peer_via(addr, 8_000, proxy.as_ref())?;
    if let Ok(g) = state.lock() {
        g.last_tor_ok.store(used_tor, Ordering::Relaxed);
    }
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
    {
        let mut g = state.lock().unwrap();
        g.mark_confirmed(addr.to_string());
        if is_directory_addr(addr) {
            g.reachable_listen.insert(addr.to_string());
        }
    }

    let write_clone = stream.try_clone()?;
    let sess = {
        let mut g = state.lock().unwrap();
        let key = outbound_key(addr);
        g.session_height.insert(key.clone(), peer_h);
        g.sessions.insert(key, write_clone, true)
    };
    tracing::info!("outbound session {addr} height={peer_h}");

    // Catch up on this socket before we sit in the read loop. A NAT wallet
    // that just opened Core is often tens of blocks behind; one GetBlocks
    // here, then the loop chains pages if the peer is still ahead. Claim a
    // unique slice so two new outbound peers do not fetch the same page.
    if peer_h + 1 > next || peer_tip != tip.to_hex() {
        let from_height = {
            let mut g = state.lock().unwrap();
            claim_ibd_from(&mut g)
        };
        let _ = sess.send(&PeerMsg::GetBlocks {
            from_height,
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
    if let Ok(mut g) = state.lock() {
        let key = outbound_key(addr);
        g.sessions.remove(&key);
        g.session_height.remove(&key);
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

const IBD_PAGE: u64 = MAX_BLOCKS_PER_REQUEST as u64;
/// Do not run more than this many heights ahead of our tip. Lost pages
/// rewind the claim pointer by re-requesting `next`.
const IBD_MAX_AHEAD: u64 = IBD_PAGE * 4;

/// Next GetBlocks start height. Two live peers therefore fetch different
/// 128-block slices instead of the same one twice.
fn next_ibd_claim(next_needed: u64, ibd_from: &mut u64) -> u64 {
    if *ibd_from < next_needed {
        *ibd_from = next_needed;
    }
    if *ibd_from >= next_needed.saturating_add(IBD_MAX_AHEAD) {
        return next_needed;
    }
    let from = *ibd_from;
    *ibd_from = from.saturating_add(IBD_PAGE);
    from
}

fn claim_ibd_from(inner: &mut NodeInner) -> u64 {
    let next = next_needed_height(inner);
    next_ibd_claim(next, &mut inner.ibd_from)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IbdPage {
    Applied(usize),
    Buffered,
    Duplicate,
    Rejected,
}

fn apply_ibd_page(inner: &mut NodeInner, blocks: Vec<Block>, now: u64) -> IbdPage {
    let Some(start) = blocks.first().map(|b| b.header.height.0) else {
        return IbdPage::Duplicate;
    };
    let next = next_needed_height(inner);
    if start > next {
        inner.ibd_buffer.insert(start, blocks);
        while inner.ibd_buffer.len() > 8 {
            if let Some(k) = inner.ibd_buffer.keys().next_back().copied() {
                inner.ibd_buffer.remove(&k);
            }
        }
        return IbdPage::Buffered;
    }
    let last_h = blocks.last().map(|b| b.header.height.0).unwrap_or(start);
    if last_h < next {
        return IbdPage::Duplicate;
    }
    let mut applied = match inner.chain.try_ingest_blocks(blocks, now) {
        Ok(n) => n,
        Err(_) => 0,
    };
    loop {
        let n = next_needed_height(inner);
        let Some(page) = inner.ibd_buffer.remove(&n) else {
            break;
        };
        match inner.chain.try_ingest_blocks(page, now) {
            Ok(k) if k > 0 => applied += k,
            _ => break,
        }
    }
    let now_next = next_needed_height(inner);
    if now_next > inner.ibd_from {
        inner.ibd_from = now_next;
    }
    if applied > 0 {
        IbdPage::Applied(applied)
    } else if now_next > next {
        IbdPage::Duplicate
    } else if start == next {
        IbdPage::Rejected
    } else {
        IbdPage::Duplicate
    }
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

/// Seeds plus reachable listeners, filtered for the public directory.
pub fn merge_directory_peers(
    seeds: impl IntoIterator<Item = String>,
    reachable: impl IntoIterator<Item = String>,
    cap: usize,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for addr in seeds.into_iter().chain(reachable) {
        if !is_directory_addr(&addr) {
            continue;
        }
        if !seen.insert(addr.clone()) {
            continue;
        }
        out.push(addr);
        if out.len() >= cap {
            break;
        }
    }
    out
}

fn spawn_p2p_plane(state: SharedState, listen: String, mine: bool) {
    {
        let st = Arc::clone(&state);
        thread::spawn(move || p2p_listen_loop(listen, st));
    }
    spawn_outbound_supervisor(Arc::clone(&state));
    spawn_status_ticker(Arc::clone(&state));
    spawn_dandelion_fluff(Arc::clone(&state));
    if mine {
        let st = Arc::clone(&state);
        thread::spawn(move || mining_loop(st));
    }
}

fn spawn_directory_bootstrap(
    state: SharedState,
    network: NetworkId,
    configured: Option<String>,
    genesis_hex: String,
) {
    let url = match configured.as_deref().map(str::trim) {
        Some("off") | Some("none") | Some("false") => return,
        Some(s) if !s.is_empty() => s.to_string(),
        _ if network == NetworkId::Mainnet => DEFAULT_PEERS_DIRECTORY.to_string(),
        _ => return,
    };
    thread::spawn(move || match fetch_directory_peers(&url, &genesis_hex) {
        Ok(peers) if !peers.is_empty() => {
            if let Ok(mut g) = state.lock() {
                let mut added = 0usize;
                for addr in peers {
                    if !looks_like_dial_target(&addr) || !is_directory_addr(&addr) {
                        continue;
                    }
                    if g.peer_addrs.insert(addr.clone()) {
                        g.bootstrap.push(addr);
                        added += 1;
                    }
                }
                if added > 0 {
                    tracing::info!("directory {url}: {added} listening node(s) to dial");
                }
            }
        }
        Ok(_) => tracing::debug!("directory {url}: empty"),
        Err(e) => tracing::info!("directory {url}: {e}"),
    });
}

fn fetch_directory_peers(url: &str, genesis_hex: &str) -> anyhow::Result<Vec<String>> {
    let body: serde_json::Value = ureq::get(url)
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .into_json()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(got) = body.get("genesis").and_then(|v| v.as_str()) {
        if !got.is_empty() && !genesis_hex.is_empty() && !genesis_hex.eq_ignore_ascii_case(got) {
            anyhow::bail!("directory genesis {got} does not match ours");
        }
    }
    let list = body
        .get("peers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(list
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .filter(|a| is_directory_addr(a))
        .take(MAX_PEERS_PER_MSG)
        .collect())
}

fn finish_introduction(
    state: &SharedState,
    sess: &SessionHandle,
    peer_height: u64,
    started: std::time::Instant,
    label: &str,
) -> bool {
    let our = state
        .lock()
        .ok()
        .and_then(|g| g.chain.tip_height().map(|h| h.0))
        .unwrap_or(0);
    if peer_height.saturating_add(IBD_BEHIND) < our {
        return false;
    }
    if started.elapsed() < Duration::from_secs(INTRO_GRACE_SECS) {
        return false;
    }
    if let Ok(g) = state.lock() {
        let _ = sess.send(&PeerMsg::Peers {
            addrs: g.publishable_peers(),
        });
    }
    tracing::info!("introduced {label} — hanging up, they have the tip");
    true
}

/// One live session we might disconnect to free a seat.
#[derive(Debug, Clone)]
pub struct EvictionCandidate {
    pub key: String,
    pub height: Option<u64>,
    pub outbound: bool,
}

/// Pick a session that already has the chain. A seed's job is to introduce
/// newcomers, not to hold 128 miners who finished IBD an hour ago.
///
/// Returns `None` when every inbound is still catching up — stealing their
/// seat would recreate the "stuck at genesis" bug for someone else.
pub fn pick_eviction_victim(
    candidates: &[EvictionCandidate],
    our_tip: u64,
    protect: &str,
) -> Option<String> {
    let mut synced: Vec<&EvictionCandidate> = candidates
        .iter()
        .filter(|c| !c.outbound && c.key != protect)
        .filter(|c| match c.height {
            Some(h) => h.saturating_add(IBD_BEHIND) >= our_tip,
            None => true,
        })
        .collect();
    if synced.is_empty() {
        return None;
    }
    synced.sort_by_key(|c| std::cmp::Reverse(c.height.unwrap_or(u64::MAX)));
    Some(synced[0].key.clone())
}

fn evict_synced_inbound(state: &SharedState, protect: &str) -> bool {
    let (our_tip, candidates) = {
        let Ok(g) = state.lock() else {
            return false;
        };
        let tip = g.chain.tip_height().map(|h| h.0).unwrap_or(0);
        let list: Vec<EvictionCandidate> = g
            .sessions
            .all()
            .into_iter()
            .map(|s| EvictionCandidate {
                height: g.session_height.get(&s.key).copied(),
                key: s.key,
                outbound: s.outbound,
            })
            .collect();
        (tip, list)
    };
    let Some(key) = pick_eviction_victim(&candidates, our_tip, protect) else {
        return false;
    };
    let Some(sess) = state.lock().ok().and_then(|g| g.sessions.get(&key)) else {
        return false;
    };
    tracing::info!("evicted {key} (caught up) to make room for a new inbound");
    sess.disconnect();
    true
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
                // Hard cap only. Soft cap is MAX_PEERS; overflow seats exist
                // so Hello can run and a synced miner can be asked to leave.
                if live >= MAX_PEERS.saturating_add(IBD_ACCEPT_BURST) {
                    tracing::debug!("session burst full, dropping connection");
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
                    g.mark_confirmed(addr.clone());
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
            if let Ok(mut g) = state.lock() {
                g.session_height.insert(session_key.clone(), peer_height);
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

    if state
        .lock()
        .map(|g| g.sessions.len() >= MAX_PEERS)
        .unwrap_or(false)
    {
        evict_synced_inbound(&state, &session_key);
    }
    let sess = {
        let g = state.lock().unwrap();
        g.sessions.insert(session_key.clone(), hello_writer, false)
    };
    // First gift: who else answers. A new install that only knows the
    // seed must leave this socket with somewhere else to dial.
    let intro = {
        let g = state.lock().unwrap();
        g.publishable_peers()
    };
    let _ = sess.send(&PeerMsg::Peers { addrs: intro });
    let result = peer_io_loop(&mut reader, &sess, &state, &peer_label, peer_height);
    if let Ok(mut g) = state.lock() {
        g.sessions.remove(&session_key);
        g.session_height.remove(&session_key);
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
    let introduced_at = std::time::Instant::now();
    loop {
        if !sess.outbound
            && finish_introduction(state, sess, last_peer_height, introduced_at, peer_label)
        {
            return Ok(());
        }
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
                    g.publishable_peers()
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
                let (want, our_tip) = {
                    let mut g = state.lock().unwrap();
                    g.session_height.insert(sess.key.clone(), height);
                    let next = next_needed_height(&g);
                    let our_tip = g.chain.tip_hash().to_hex();
                    let want = if height + 1 > next {
                        Some(claim_ibd_from(&mut g))
                    } else {
                        None
                    };
                    (want, our_tip)
                };
                if let Some(from_height) = want {
                    // Distinct pages per peer. Starting at `tip − 1` used
                    // to include a block we already hold; apply_block then
                    // returned BadHeight and the rest of the page was
                    // thrown away.
                    sess.send(&PeerMsg::GetBlocks {
                        from_height,
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
                    propagate_tx(state, &tx, Some(sess.key.as_str()), false);
                }
            }

            PeerMsg::InvBlock { height, .. } => {
                last_peer_height = last_peer_height.max(height);
                note_peer_height(state, height);
                let want = {
                    let mut g = state.lock().unwrap();
                    if height + 1 > next_needed_height(&g) {
                        Some(claim_ibd_from(&mut g))
                    } else {
                        None
                    }
                };
                if let Some(from_height) = want {
                    sess.send(&PeerMsg::GetBlocks {
                        from_height,
                        limit: MAX_BLOCKS_PER_REQUEST,
                    })?;
                }
            }

            PeerMsg::Blocks { blocks } => {
                if blocks.len() > MAX_BLOCKS_PER_REQUEST {
                    return Err(anyhow::anyhow!("peer sent an oversized block batch"));
                }
                let outcome = {
                    let mut g = state.lock().unwrap();
                    let outcome = apply_ibd_page(&mut g, blocks, now_unix());
                    if let IbdPage::Applied(n) = outcome {
                        g.stalled_on_fork.store(false, Ordering::SeqCst);
                        g.bump_tip();
                        let _ = g.persist();
                        tracing::info!("extended +{n} from {peer_label}");
                    }
                    outcome
                };
                // Keep pulling. Parallel peers fetch different pages;
                // out-of-order ones sit in ibd_buffer until the hole fills.
                let (want, reject) = {
                    let mut g = state.lock().unwrap();
                    let next = next_needed_height(&g);
                    let behind = last_peer_height + 1 > next;
                    match outcome {
                        IbdPage::Applied(_) | IbdPage::Buffered if behind => {
                            (Some(claim_ibd_from(&mut g)), false)
                        }
                        IbdPage::Rejected if behind => (None, true),
                        _ => (None, false),
                    }
                };
                if let Some(from_height) = want {
                    sess.send(&PeerMsg::GetBlocks {
                        from_height,
                        limit: MAX_BLOCKS_PER_REQUEST,
                    })?;
                } else if reject {
                    if let Ok(g) = state.lock() {
                        g.stalled_on_fork.store(true, Ordering::SeqCst);
                    }
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
pub const PEER_HEIGHT_TTL_SECS: u64 = 120;

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
    let our_hashes: Vec<_> = g.chain.blocks.iter().map(|b| b.hash()).collect();
    let guard = Arc::clone(&g.reorg_in_flight);
    drop(g);

    let Some(_flight) = ReorgFlight::begin(&guard) else {
        tracing::debug!("branch from {peer_label} skipped — a reorg is already being verified");
        return;
    };

    // Verified with the lock released. See `Chain::evaluate_reorg`.
    let verdict = Chain::evaluate_reorg(network, our_work, &our_hashes, candidate, now_unix());

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
    let proxy = state.lock().ok().and_then(|g| g.proxy.clone());
    let (mut stream, used_tor) = connect_peer_via(addr, 15_000, proxy.as_ref())?;
    // Connect is 15s. Pages of 128 blocks over Tor need the live-session
    // budget or a mid-chain fetch dies and looks like "sync is stuck".
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(120)));
    if let Ok(g) = state.lock() {
        g.last_tor_ok.store(used_tor, Ordering::Relaxed);
    }
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
    state.lock().unwrap().mark_confirmed(addr.to_string());
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

    // Fetch forward from the next height we can attach, not from tip − 1.
    // Starting at tip − 1 includes a block we already hold; apply_block
    // then returns BadHeight, applied stays 0, and every catch-up is
    // misread as a fork — which is how a node that was merely late ended
    // up in the reorg path after the lid opened.
    //
    // `our_h + 1` is wrong on an empty node: tip_height is then 0 by
    // default, and we would skip genesis.
    let mut from = {
        let g = state.lock().unwrap();
        next_needed_height(&g)
    };
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

        // Nothing connected. Either we already have those heights (a competing
        // block at our tip) or we are on a lighter fork. Find where we last
        // agreed and pull only the suffix — not the peer's chain from genesis,
        // which used to hit `our_len + MAX_REORG_DEPTH` and refuse a laptop
        // that had slept for a few hours.
        if peer_tip == {
            let g = state.lock().unwrap();
            g.chain.tip_hash().to_hex()
        } {
            break;
        }

        let ancestor = common_ancestor(state, &mut stream, peer_h, &peer_tip);
        let (network, our_work, our_hashes, prefix, rewind, guard) = {
            let g = state.lock().unwrap();
            let our_tip = g.chain.tip_height().map(|h| h.0).unwrap_or(0);
            let rewind = our_tip.saturating_sub(ancestor) as usize;
            let prefix_end = (ancestor as usize)
                .saturating_add(1)
                .min(g.chain.blocks.len());
            (
                g.chain.network,
                g.chain.total_work,
                g.chain.blocks.iter().map(|b| b.hash()).collect::<Vec<_>>(),
                g.chain.blocks[..prefix_end].to_vec(),
                rewind,
                Arc::clone(&g.reorg_in_flight),
            )
        };

        if rewind > nightfall_consensus::MAX_REORG_DEPTH {
            tracing::warn!(
                "peer {addr} diverges {rewind} blocks back — past MAX_REORG_DEPTH, not adopting"
            );
            break;
        }

        let Some(_flight) = ReorgFlight::begin(&guard) else {
            tracing::debug!("peer {addr} diverges, but a reorg is already being verified");
            break;
        };

        let suffix = fetch_blocks_from(&mut stream, ancestor.saturating_add(1), peer_h)?;
        if suffix.is_empty() && rewind == 0 {
            break;
        }
        let mut candidate = prefix;
        candidate.extend(suffix);
        if candidate.is_empty() {
            break;
        }

        // Rebuilt and re-verified with the lock released — tens of seconds of
        // work that used to freeze the whole node. See `Chain::evaluate_reorg`.
        let verdict = Chain::evaluate_reorg(network, our_work, &our_hashes, candidate, now_unix());

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

/// Hard ceiling on a single reorg pull. A peer advertising `u64::MAX` must
/// not make us allocate forever. ~100k blocks is more than two weeks at the
/// 15-second target — long enough for a closed laptop, short enough that a
/// liar is cut off.
pub const MAX_REORG_FETCH: usize = 100_000;

/// How many blocks to pull in `[from_height, peer_height]`, inclusive.
///
/// This used to be `min(peer_height + 1, our_len + MAX_REORG_DEPTH)` and was
/// fetched from genesis. A node 700 blocks behind on a one-block fork then
/// asked for a chain the acceptance rule would refuse as "too deep". The
/// caller now supplies the first height *after* the common ancestor, so the
/// cap is the suffix, not the whole history.
pub fn reorg_fetch_cap(peer_height: u64, from_height: u64) -> usize {
    if from_height > peer_height {
        return 0;
    }
    let want = (peer_height - from_height) as usize + 1;
    want.min(MAX_REORG_FETCH)
}

/// Download blocks `[from_height ..= peer_height]` in pages.
fn fetch_blocks_from(
    stream: &mut TcpStream,
    from_height: u64,
    peer_height: u64,
) -> anyhow::Result<Vec<nightfall_consensus::Block>> {
    let cap = reorg_fetch_cap(peer_height, from_height);
    let mut all = Vec::with_capacity(cap.min(4096));
    let mut from = from_height;

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
        if n < MAX_BLOCKS_PER_REQUEST || from > peer_height {
            break;
        }
    }
    if all.len() > cap {
        all.truncate(cap);
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
pub const MAX_CATCHUP_WAIT_SECS: u64 = 600;

/// Wall-clock gap that means the process was frozen (laptop lid).
///
/// A healthy ticker writes once a second. Sixty seconds of silence is not a
/// slow hash, it is sleep. Shorter than [`PEER_HEIGHT_TTL_SECS`] so we notice
/// the lid before we trust a stale peer height and mine on it.
pub const SLEEP_GAP_SECS: u64 = 60;

/// How far behind the best peer we are, or `None` if we should mine anyway.
fn catchup_behind(inner: &NodeInner) -> Option<u64> {
    // A competing tip must not be extended. This is not the catch-up
    // timeout: waiting that out and mining again is how a one-block fork
    // became a stranded chain.
    if inner.stalled_on_fork.load(Ordering::SeqCst) || inner.reorg_in_flight.load(Ordering::SeqCst)
    {
        return Some(1);
    }
    mining_should_wait(
        inner.chain.block_count(),
        inner.chain.tip_height().map(|h| h.0).unwrap_or(0),
        inner.best_peer_height,
        inner.best_peer_seen,
        inner.behind_since,
        now_unix(),
    )
}

/// Whether mining must wait, and by how many blocks if a peer is actually
/// ahead. Pure so the sleep/startup cases can be tested without a node.
///
/// A chain we already have, with no fresh peer, must not be extended. That is
/// the lid-close fork: the process wakes, `best_peer_height` still equals our
/// tip, sockets are dead, and the first block we mine is a branch the seed
/// will never take. Genesis (empty chain) may mine — a network of one has to
/// start. After [`MAX_CATCHUP_WAIT_SECS`] we mine anyway so an isolated node
/// is not bricked.
pub fn mining_should_wait(
    block_count: u64,
    our_height: u64,
    best_peer_height: u64,
    best_peer_seen: u64,
    behind_since: u64,
    now: u64,
) -> Option<u64> {
    let peer_fresh =
        best_peer_seen > 0 && now.saturating_sub(best_peer_seen) <= PEER_HEIGHT_TTL_SECS;

    if !peer_fresh {
        if block_count == 0 {
            return None;
        }
        if now.saturating_sub(behind_since) > MAX_CATCHUP_WAIT_SECS {
            return None;
        }
        return Some(1);
    }

    if best_peer_height <= our_height {
        return None;
    }
    if now.saturating_sub(behind_since) > MAX_CATCHUP_WAIT_SECS {
        return None;
    }
    Some(best_peer_height - our_height)
}

/// Drop cached peer height and abort the current template. Called when the
/// wall clock jumps — the process was frozen and every socket is suspect.
fn apply_clock_jump(inner: &mut NodeInner) {
    if inner.chain.block_count() == 0 {
        inner.last_wall_tick.store(now_unix(), Ordering::Relaxed);
        return;
    }
    tracing::info!("wall clock jumped — holding mining until a peer confirms the tip");
    inner.tip_epoch.fetch_add(1, Ordering::SeqCst);
    inner.best_peer_height = 0;
    inner.best_peer_seen = 0;
    inner.behind_since = now_unix();
    inner.last_wall_tick.store(now_unix(), Ordering::Relaxed);
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
        let (enabled, tip_epoch, epoch_now, last_tick) = {
            let mut g = state.lock().unwrap();
            let now = now_unix();
            let last = g.last_wall_tick.load(Ordering::Relaxed);
            if now.saturating_sub(last) > SLEEP_GAP_SECS {
                apply_clock_jump(&mut g);
            }
            (
                g.mining_enabled.load(Ordering::SeqCst),
                Arc::clone(&g.tip_epoch),
                g.tip_epoch.load(Ordering::SeqCst),
                g.last_wall_tick.load(Ordering::Relaxed),
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
        // An existing chain with no *fresh* peer is the same trap: after the
        // lid opens, sockets are dead and the cached height still matches, so
        // "no one is ahead" is a lie. Genesis may mine (a network of one has
        // to start). After MAX_CATCHUP_WAIT_SECS an isolated node mines too.
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

        let should_stop = || {
            if now_unix().saturating_sub(last_tick) > SLEEP_GAP_SECS {
                return true;
            }
            tip_epoch.load(Ordering::SeqCst) != epoch_now || !mining_flag.load(Ordering::SeqCst)
        };

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
        let now = now_unix();
        if now.saturating_sub(g.last_wall_tick.load(Ordering::Relaxed)) > SLEEP_GAP_SECS {
            apply_clock_jump(&mut g);
            tracing::debug!("discarding block found after a clock jump");
            continue;
        }
        if catchup_behind(&g).is_some() {
            tracing::debug!("discarding block found while we should not have been mining");
            continue;
        }
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

#[cfg(test)]
mod ibd_claim_tests {
    use super::{next_ibd_claim, IBD_MAX_AHEAD, IBD_PAGE};

    #[test]
    fn distinct_pages_then_rewind_at_the_cap() {
        let mut ibd_from = 0u64;
        let next = 100u64;
        let a = next_ibd_claim(next, &mut ibd_from);
        let b = next_ibd_claim(next, &mut ibd_from);
        let c = next_ibd_claim(next, &mut ibd_from);
        assert_eq!(a, 100);
        assert_eq!(b, 100 + IBD_PAGE);
        assert_eq!(c, 100 + IBD_PAGE * 2);
        assert_eq!(b - a, IBD_PAGE);

        // Past the ahead-cap we re-request the hole instead of running away.
        ibd_from = next + IBD_MAX_AHEAD;
        let rewind = next_ibd_claim(next, &mut ibd_from);
        assert_eq!(rewind, next);
    }

    #[test]
    fn claim_pointer_catches_up_after_ingest() {
        let mut ibd_from = 50;
        let advanced = next_ibd_claim(200, &mut ibd_from);
        assert_eq!(advanced, 200);
        assert_eq!(ibd_from, 200 + IBD_PAGE);
    }
}
