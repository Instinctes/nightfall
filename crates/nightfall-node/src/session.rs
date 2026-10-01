//! Live P2P sessions.
//!
//! A node behind NAT cannot be dialled. It can, however, hold a socket open
//! to a seed and receive every block the moment the seed does. The previous
//! design threw that socket away after each handshake and then tried to dial
//! the peer's listen address — which, for NAT, does not exist. Blocks arrived
//! on the next 8-second poll, or not at all.
//!
//! This pool is the socket. Announce writes to it. A drop reconnects.

use nightfall_consensus::Block;
use nightfall_ledger::Transaction;
use nightfall_p2p::{broadcast_block, broadcast_stem_tx, broadcast_tx, write_msg, PeerMsg};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};

/// How many outbound sockets we try to keep up. Seeds are filled first, so a
/// wallet that can reach the network at all is on a live link to it.
/// The supervisor now launches a stay-connected thread per known address;
/// this remains the number we consider "enough" for a healthy node.
#[allow(dead_code)]
pub const TARGET_OUTBOUND: usize = 8;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StemSendFailure {
    pub txid: String,
    pub route: String,
}

/// Hard ceiling on asynchronous delivery-failure metadata.
///
/// A hostile or broken peer must not be able to turn repeated socket failures
/// into an unbounded allocation. Dropping the oldest failure is safe: the
/// corresponding stem remains protected by its randomized embargo and will
/// eventually enter normal diffusion.
pub const STEM_FAILURE_QUEUE_MAX: usize = 10_000;

#[derive(Default)]
struct StemFailureState {
    queue: VecDeque<StemSendFailure>,
    seen: HashSet<StemSendFailure>,
}

#[derive(Clone, Default)]
pub struct StemFailureQueue {
    inner: Arc<Mutex<StemFailureState>>,
}

impl StemFailureQueue {
    pub fn push(&self, failure: StemSendFailure) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        // One socket failure must produce at most one pending repair event.
        if !state.seen.insert(failure.clone()) {
            return;
        }

        if state.queue.len() >= STEM_FAILURE_QUEUE_MAX {
            if let Some(oldest) = state.queue.pop_front() {
                state.seen.remove(&oldest);
            }
        }

        state.queue.push_back(failure);
    }

    pub fn drain(&self) -> Vec<StemSendFailure> {
        let Ok(mut state) = self.inner.lock() else {
            return Vec::new();
        };

        let drained = state.queue.drain(..).collect();
        state.seen.clear();
        drained
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|state| state.queue.len())
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone)]
pub struct SessionHandle {
    /// Unique socket/session key. Inbound and outbound sockets intentionally
    /// remain distinct here.
    pub key: String,
    /// Logical remote peer identity used by privacy routing.
    ///
    /// Multiple sockets to the same remote node share this value.
    pub peer_id: String,
    pub outbound: bool,
    /// True only when the peer explicitly negotiated Dandelion stem v1.
    pub dandelion_stem_v1: bool,
    writer: Arc<Mutex<TcpStream>>,
}

impl SessionHandle {
    pub fn send(&self, msg: &PeerMsg) -> std::io::Result<()> {
        let mut s = self.writer.lock().map_err(|e| {
            std::io::Error::other(format!("session {} lock poisoned: {e}", self.key))
        })?;
        write_msg(&mut s, msg)
    }

    pub fn send_block(&self, block: &Block) -> std::io::Result<()> {
        let mut s = self.writer.lock().map_err(|e| {
            std::io::Error::other(format!("session {} lock poisoned: {e}", self.key))
        })?;
        broadcast_block(&mut s, block)
    }

    pub fn send_tx(&self, tx: &Transaction) -> std::io::Result<()> {
        let mut s = self.writer.lock().map_err(|e| {
            std::io::Error::other(format!("session {} lock poisoned: {e}", self.key))
        })?;
        broadcast_tx(&mut s, tx)
    }

    pub fn send_stem_tx(&self, tx: &Transaction) -> std::io::Result<()> {
        if !self.dandelion_stem_v1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("session {} did not negotiate Dandelion stem v1", self.key),
            ));
        }

        let mut s = self.writer.lock().map_err(|e| {
            std::io::Error::other(format!("session {} lock poisoned: {e}", self.key))
        })?;

        broadcast_stem_tx(&mut s, tx)
    }

    /// Unblock the peer's read loop so the slot frees. Used when a new
    /// inbound needs the chain and every seat is taken by a node that
    /// already has it.
    pub fn disconnect(&self) {
        if let Ok(s) = self.writer.lock() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

pub struct SessionPool {
    inner: Mutex<HashMap<String, SessionHandle>>,
}

impl SessionPool {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, key: String, stream: TcpStream, outbound: bool) -> SessionHandle {
        let peer_id = key.clone();

        self.insert_with_identity(key, stream, outbound, false, peer_id)
    }

    pub fn insert_with_dandelion(
        &self,
        key: String,
        stream: TcpStream,
        outbound: bool,
        dandelion_stem_v1: bool,
    ) -> SessionHandle {
        let peer_id = key.clone();

        self.insert_with_identity(key, stream, outbound, dandelion_stem_v1, peer_id)
    }

    pub fn insert_with_identity(
        &self,
        key: String,
        stream: TcpStream,
        outbound: bool,
        dandelion_stem_v1: bool,
        peer_id: String,
    ) -> SessionHandle {
        let handle = SessionHandle {
            key: key.clone(),
            peer_id,
            outbound,
            dandelion_stem_v1,
            writer: Arc::new(Mutex::new(stream)),
        };

        if let Ok(mut g) = self.inner.lock() {
            g.insert(key, handle.clone());
        }

        handle
    }

    pub fn remove(&self, key: &str) {
        if let Ok(mut g) = self.inner.lock() {
            g.remove(key);
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner
            .lock()
            .map(|g| g.contains_key(key))
            .unwrap_or(false)
    }

    pub fn get(&self, key: &str) -> Option<SessionHandle> {
        self.inner.lock().ok().and_then(|g| g.get(key).cloned())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn outbound_count(&self) -> usize {
        self.inner
            .lock()
            .map(|g| g.values().filter(|s| s.outbound).count())
            .unwrap_or(0)
    }

    pub fn all(&self) -> Vec<SessionHandle> {
        self.inner
            .lock()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn outbound_keys(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|g| {
                g.values()
                    .filter(|s| s.outbound)
                    .map(|s| s.key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Outbound routes that explicitly negotiated Dandelion stem v1.
    pub fn dandelion_outbound_keys(&self) -> Vec<String> {
        self.dandelion_outbound_keys_except_peer(None)
    }

    /// Dandelion-capable outbound sockets excluding every socket that belongs
    /// to the same logical peer.
    pub fn dandelion_outbound_keys_except_peer(
        &self,
        excluded_peer_id: Option<&str>,
    ) -> Vec<String> {
        self.inner
            .lock()
            .map(|g| {
                // The anonymity graph is a graph of logical peers, not TCP
                // sockets. Two transport sessions to the same remote node
                // therefore occupy one Dandelion candidate slot.
                //
                // BTreeMap gives deterministic peer ordering. For duplicate
                // sockets, the lexicographically smallest live session key
                // is the stable representative until that socket disappears.
                let mut by_peer = BTreeMap::<String, String>::new();

                for session in g.values().filter(|session| {
                    session.outbound
                        && session.dandelion_stem_v1
                        && excluded_peer_id != Some(session.peer_id.as_str())
                }) {
                    by_peer
                        .entry(session.peer_id.clone())
                        .and_modify(|route| {
                            if session.key.as_str() < route.as_str() {
                                *route = session.key.clone();
                            }
                        })
                        .or_insert_with(|| session.key.clone());
                }

                by_peer.into_values().collect()
            })
            .unwrap_or_default()
    }

    /// True if we already initiated a live outbound to this dial target.
    pub fn has_outbound_to(&self, addr: &str) -> bool {
        let key = outbound_key(addr);
        self.inner
            .lock()
            .map(|g| g.contains_key(&key))
            .unwrap_or(false)
    }
}

/// Inbound and outbound to the same listen address must not share a map
/// key. A miner that announces by opening a fresh TCP connection would
/// otherwise overwrite the long-lived outbound, then delete it when that
/// short connection closed — leaving a socket that nobody writes to.
pub fn outbound_key(addr: &str) -> String {
    format!("out:{addr}")
}

pub fn inbound_key(label: &str) -> String {
    format!("in:{label}")
}

impl Default for SessionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Fan a block out to every live socket. One thread per session so a stuck
/// write cannot delay the rest — the same lesson as the old dial-per-peer
/// announce, minus the dial.
pub fn fanout_block(sessions: &[SessionHandle], block: &Block) {
    for s in sessions {
        let s = s.clone();
        let block = block.clone();
        std::thread::spawn(move || {
            if let Err(e) = s.send_block(&block) {
                tracing::debug!("session {} block send: {e}", s.key);
            }
        });
    }
}

pub fn fanout_tx(sessions: &[SessionHandle], tx: &Transaction) {
    fluff_tx(sessions, tx, None);
}

/// Broadcast a transaction to every live socket except `exclude`.
pub fn fluff_tx(sessions: &[SessionHandle], tx: &Transaction, exclude: Option<&str>) {
    let excluded_peer_id = exclude.and_then(|key| {
        sessions
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.peer_id.as_str())
    });

    for s in sessions {
        if exclude == Some(s.key.as_str()) || excluded_peer_id == Some(s.peer_id.as_str()) {
            continue;
        }

        let s = s.clone();
        let tx = tx.clone();

        std::thread::spawn(move || {
            if let Err(e) = s.send_tx(&tx) {
                tracing::debug!("session {} tx send: {e}", s.key);
            }
        });
    }
}

/// Send a stem transaction through one exact epoch-selected route.
///
/// The routing decision belongs to the Dandelion privacy graph, not this
/// function. Returning false means the selected session vanished before the
/// send could be queued; the caller can then use its fail-safe fluff path.
pub fn stem_tx_to(sessions: &SessionPool, route: &str, tx: &Transaction) -> bool {
    stem_tx_to_observed(sessions, route, tx, None)
}

/// Queue a stem write without blocking the node-state lock.
///
/// `true` means the write was successfully scheduled. A later socket/write
/// error is reported through `failures`; it does not masquerade as delivery.
pub fn stem_tx_to_observed(
    sessions: &SessionPool,
    route: &str,
    tx: &Transaction,
    failures: Option<&StemFailureQueue>,
) -> bool {
    let Some(s) = sessions.get(route) else {
        return false;
    };

    if !s.dandelion_stem_v1 {
        return false;
    }

    let s = s.clone();
    let tx = tx.clone();
    let txid = tx.txid().to_hex();
    let route = route.to_string();
    let failures = failures.cloned();

    std::thread::spawn(move || {
        if let Err(e) = s.send_stem_tx(&tx) {
            tracing::debug!("session {} stem send failed: {e}", s.key);

            if let Some(queue) = failures {
                queue.push(StemSendFailure { txid, route });
            }
        }
    });

    true
}

/// Pick the Dandelion stem hop: prefer an outbound peer that is not `exclude`.
pub fn pick_stem_peer<'a>(
    sessions: &'a [SessionHandle],
    exclude: Option<&str>,
) -> Option<&'a SessionHandle> {
    let excluded_peer_id = exclude.and_then(|key| {
        sessions
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.peer_id.as_str())
    });

    let outbound: Vec<&SessionHandle> = sessions
        .iter()
        .filter(|s| {
            s.outbound
                && s.dandelion_stem_v1
                && exclude != Some(s.key.as_str())
                && excluded_peer_id != Some(s.peer_id.as_str())
        })
        .collect();

    let pool = if outbound.is_empty() {
        sessions
            .iter()
            .filter(|s| {
                s.dandelion_stem_v1
                    && exclude != Some(s.key.as_str())
                    && excluded_peer_id != Some(s.peer_id.as_str())
            })
            .collect::<Vec<_>>()
    } else {
        outbound
    };

    if pool.is_empty() {
        return None;
    }

    let idx = rand::random::<usize>() % pool.len();
    Some(pool[idx])
}

/// Forward a transaction to exactly one peer. Returns false if nobody is left.
pub fn stem_tx(sessions: &[SessionHandle], tx: &Transaction, exclude: Option<&str>) -> bool {
    let Some(chosen) = pick_stem_peer(sessions, exclude) else {
        return false;
    };
    let s = chosen.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = s.send_stem_tx(&tx) {
            tracing::debug!("session {} stem send: {e}", s.key);
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn insert_and_remove_are_visible() {
        let pool = SessionPool::new();
        let (a, _b) = pair();
        assert!(pool.is_empty());
        pool.insert("seed.example:17891".into(), a, true);
        assert_eq!(pool.len(), 1);
        assert!(pool.contains("seed.example:17891"));
        assert_eq!(pool.outbound_count(), 1);
        pool.remove("seed.example:17891");
        assert!(pool.is_empty());
    }

    #[test]
    fn inbound_sessions_do_not_count_as_outbound() {
        let pool = SessionPool::new();
        let (a, _b) = pair();
        pool.insert(inbound_key("1.2.3.4:9"), a, false);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.outbound_count(), 0);
        assert!(pool.outbound_keys().is_empty());
        assert!(!pool.has_outbound_to("1.2.3.4:9"));
    }

    #[test]
    fn stem_skips_the_peer_we_heard_the_tx_from() {
        let (a, _a2) = pair();
        let (b, _b2) = pair();
        let pool = SessionPool::new();
        pool.insert_with_dandelion(outbound_key("10.0.0.1:1"), a, true, true);
        pool.insert_with_dandelion(inbound_key("10.0.0.2:2"), b, false, true);
        let all = pool.all();
        let picked = pick_stem_peer(&all, Some(&outbound_key("10.0.0.1:1"))).unwrap();
        assert_eq!(picked.key, inbound_key("10.0.0.2:2"));
        assert!(pick_stem_peer(&all, None).is_some());
    }

    #[test]
    fn inbound_and_outbound_to_the_same_listen_addr_do_not_collide() {
        let pool = SessionPool::new();
        let (a, _b) = pair();
        let (c, _d) = pair();
        pool.insert(outbound_key("82.1.2.3:17891"), a, true);
        pool.insert(inbound_key("82.1.2.3:54321"), c, false);
        assert_eq!(pool.len(), 2);
        pool.remove(&inbound_key("82.1.2.3:54321"));
        assert!(pool.has_outbound_to("82.1.2.3:17891"));
        assert_eq!(pool.outbound_count(), 1);
    }

    #[test]
    fn exact_route_sends_a_real_stem_wire_message() {
        let pool = SessionPool::new();
        let (sender, receiver) = pair();

        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();

        let route = outbound_key("10.0.0.9:17891");
        pool.insert_with_dandelion(route.clone(), sender, true, true);

        let tx = Transaction {
            version: 8,
            inputs: Vec::new(),
            outputs: Vec::new(),
            kernels: Vec::new(),
        };

        assert!(stem_tx_to(&pool, &route, &tx));

        let mut reader = std::io::BufReader::new(receiver);
        let msg = nightfall_p2p::read_msg(&mut reader).unwrap();

        match msg {
            PeerMsg::Tx { stem, .. } => assert!(stem),
            other => panic!("expected stem tx, got {other:?}"),
        }
    }

    #[test]
    fn dandelion_routes_exclude_legacy_peers() {
        let pool = SessionPool::new();

        let (legacy, _legacy_peer) = pair();
        let (capable, _capable_peer) = pair();

        let legacy_key = outbound_key("10.0.0.10:17891");
        let capable_key = outbound_key("10.0.0.11:17891");

        pool.insert(legacy_key.clone(), legacy, true);

        pool.insert_with_dandelion(capable_key.clone(), capable, true, true);

        let routes = pool.dandelion_outbound_keys();

        assert_eq!(routes, vec![capable_key.clone()]);
        assert!(!routes.contains(&legacy_key));

        let tx = Transaction {
            version: 8,
            inputs: Vec::new(),
            outputs: Vec::new(),
            kernels: Vec::new(),
        };

        assert!(!stem_tx_to(&pool, &legacy_key, &tx));
    }

    #[test]
    fn asynchronous_stem_write_failure_is_reported() {
        let pool = SessionPool::new();
        let failures = StemFailureQueue::default();

        let (writer, _peer) = pair();
        let route = outbound_key("127.0.0.1:17891");

        let handle = pool.insert_with_dandelion(route.clone(), writer, true, true);

        // Deterministically force send_stem_tx() to fail without relying on
        // TCP timing: poison this session's private writer mutex.
        let writer = Arc::clone(&handle.writer);

        let _ = std::thread::spawn(move || {
            let _guard = writer.lock().unwrap();
            panic!("intentional test poison");
        })
        .join();

        let tx = Transaction {
            version: 8,
            inputs: Vec::new(),
            outputs: Vec::new(),
            kernels: Vec::new(),
        };

        let txid = tx.txid().to_hex();

        assert!(stem_tx_to_observed(&pool, &route, &tx, Some(&failures),));

        for _ in 0..100 {
            if !failures.is_empty() {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let got = failures.drain();

        assert_eq!(got, vec![StemSendFailure { txid, route }]);
    }

    #[test]
    fn stem_failure_queue_deduplicates_identical_events() {
        let queue = StemFailureQueue::default();

        let failure = StemSendFailure {
            txid: "same-tx".into(),
            route: "out:same-peer".into(),
        };

        queue.push(failure.clone());
        queue.push(failure.clone());
        queue.push(failure.clone());

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.drain(), vec![failure]);
        assert!(queue.is_empty());
    }

    #[test]
    fn stem_failure_queue_has_a_hard_memory_bound() {
        let queue = StemFailureQueue::default();

        for i in 0..(STEM_FAILURE_QUEUE_MAX + 128) {
            queue.push(StemSendFailure {
                txid: format!("tx-{i}"),
                route: format!("out:peer-{i}"),
            });
        }

        assert_eq!(queue.len(), STEM_FAILURE_QUEUE_MAX);

        let drained = queue.drain();

        assert_eq!(drained.len(), STEM_FAILURE_QUEUE_MAX);

        // FIFO overflow discards the oldest metadata, not the newest failure.
        assert_eq!(
            drained.last().unwrap(),
            &StemSendFailure {
                txid: format!("tx-{}", STEM_FAILURE_QUEUE_MAX + 127),
                route: format!("out:peer-{}", STEM_FAILURE_QUEUE_MAX + 127),
            }
        );

        assert!(queue.is_empty());
    }

    #[test]
    fn dandelion_never_returns_to_same_logical_peer_via_second_socket() {
        let pool = SessionPool::new();

        let (incoming, _incoming_peer) = pair();
        let (same_out, _same_out_peer) = pair();
        let (other_out, _other_out_peer) = pair();

        let incoming_key = inbound_key("82.1.2.3:54321");

        let same_out_key = outbound_key("82.1.2.3:17891");

        let other_out_key = outbound_key("91.2.3.4:17891");

        let logical_source = "82.1.2.3:17891".to_string();

        pool.insert_with_identity(
            incoming_key.clone(),
            incoming,
            false,
            true,
            logical_source.clone(),
        );

        pool.insert_with_identity(
            same_out_key.clone(),
            same_out,
            true,
            true,
            logical_source.clone(),
        );

        pool.insert_with_identity(
            other_out_key.clone(),
            other_out,
            true,
            true,
            "91.2.3.4:17891".into(),
        );

        let routes = pool.dandelion_outbound_keys_except_peer(Some(logical_source.as_str()));

        assert_eq!(routes, vec![other_out_key.clone()]);
        assert!(!routes.contains(&same_out_key));

        let all = pool.all();

        let picked = pick_stem_peer(&all, Some(&incoming_key)).expect("other logical peer");

        assert_eq!(picked.key, other_out_key);
        assert_ne!(picked.peer_id, logical_source);
    }

    #[test]
    fn dandelion_candidates_are_unique_per_logical_peer() {
        let pool = SessionPool::new();

        let (a1, _a1_peer) = pair();
        let (a2, _a2_peer) = pair();
        let (b, _b_peer) = pair();
        let (legacy, _legacy_peer) = pair();

        // Two different transport addresses deliberately identify the same
        // logical Dandelion peer.
        let peer_a = "203.0.113.10:17891".to_string();

        let a_route_1 = outbound_key("alias-a.example:17891");

        let a_route_2 = outbound_key("203.0.113.10:17891");

        let b_route = outbound_key("198.51.100.20:17891");

        let legacy_route = outbound_key("192.0.2.30:17891");

        pool.insert_with_identity(a_route_1.clone(), a1, true, true, peer_a.clone());

        pool.insert_with_identity(a_route_2.clone(), a2, true, true, peer_a.clone());

        pool.insert_with_identity(b_route.clone(), b, true, true, "198.51.100.20:17891".into());

        pool.insert_with_identity(
            legacy_route.clone(),
            legacy,
            true,
            false,
            "192.0.2.30:17891".into(),
        );

        let routes = pool.dandelion_outbound_keys();

        // Two capable logical peers exist, even though three capable
        // outbound sockets exist.
        assert_eq!(routes.len(), 2);

        let a_routes = routes
            .iter()
            .filter(|route| {
                route.as_str() == a_route_1.as_str() || route.as_str() == a_route_2.as_str()
            })
            .count();

        assert_eq!(
            a_routes, 1,
            "same logical peer occupied multiple Dandelion slots"
        );

        assert!(routes.contains(&b_route));
        assert!(!routes.contains(&legacy_route));

        let unique_peer_ids = routes
            .iter()
            .map(|route| {
                pool.get(route)
                    .expect("selected route remains live")
                    .peer_id
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(
            unique_peer_ids.len(),
            routes.len(),
            "Dandelion candidates are not unique by logical peer"
        );

        let without_a = pool.dandelion_outbound_keys_except_peer(Some(peer_a.as_str()));

        assert_eq!(
            without_a,
            vec![b_route.clone()],
            "excluding logical A must remove every A transport socket"
        );

        // Losing the chosen representative must expose the surviving sibling
        // without ever giving logical A two privacy slots.
        let representative = routes
            .iter()
            .find(|route| {
                route.as_str() == a_route_1.as_str() || route.as_str() == a_route_2.as_str()
            })
            .expect("logical A representative")
            .clone();

        pool.remove(&representative);

        let remaining_a = if representative == a_route_1 {
            a_route_2
        } else {
            a_route_1
        };

        let repaired = pool.dandelion_outbound_keys();

        assert_eq!(repaired.len(), 2);
        assert!(repaired.contains(&remaining_a));
        assert!(repaired.contains(&b_route));

        let repaired_peer_ids = repaired
            .iter()
            .map(|route| {
                pool.get(route)
                    .expect("replacement route remains live")
                    .peer_id
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(
            repaired_peer_ids.len(),
            repaired.len(),
            "representative failover reintroduced duplicate logical peers"
        );
    }
}
