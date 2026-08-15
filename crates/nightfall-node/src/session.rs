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
use nightfall_p2p::{broadcast_block, broadcast_tx, write_msg, PeerMsg};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

/// How many outbound sockets we try to keep up. Seeds are filled first, so a
/// wallet that can reach the network at all is on a live link to it.
/// The supervisor now launches a stay-connected thread per known address;
/// this remains the number we consider "enough" for a healthy node.
#[allow(dead_code)]
pub const TARGET_OUTBOUND: usize = 8;

#[derive(Clone)]
pub struct SessionHandle {
    pub key: String,
    pub outbound: bool,
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
        let handle = SessionHandle {
            key: key.clone(),
            outbound,
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
    for s in sessions {
        let s = s.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = s.send_tx(&tx) {
                tracing::debug!("session {} tx send: {e}", s.key);
            }
        });
    }
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
}
