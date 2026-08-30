//! Chain observation. A failed query is not a height and is not zero confirms.
//!
//! Spec v0.3 driver contract. The fake exists so tests can force reorgs,
//! outages and "already known" broadcasts without a node.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WatchError {
    #[error("chain query failed: {0}")]
    Unavailable(String),
    /// The node is replaying from disk. Acting on its tip would treat a
    /// catching-up wallet as a stalled chain.
    #[error("chain is still loading from disk")]
    Loading,
    #[error("our height {ours} is behind peer {peer}")]
    Behind { ours: u64, peer: u64 },
    /// The node cannot look up confirmed transactions at all.
    ///
    /// Distinct from every other variant on purpose. Without `-txindex`,
    /// `getrawtransaction` answers error −5 for *every* confirmed
    /// transaction, including ones the node itself mined. Folding that into
    /// "unknown" would leave a swap reporting "node unreachable" while H₁
    /// runs out; folding it into "unavailable" would have the driver retry
    /// forever against a node that will never answer. It is a configuration
    /// problem, and the only useful response is to say so.
    #[error("this node cannot see confirmed transactions — start bitcoind with -txindex=1")]
    NeedsTxIndex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRef {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutRef {
    pub txid: String,
    pub vout: u32,
}

/// Both chains, same questions. Confirmations: `Some(0)` is mempool,
/// `None` is unknown, `Err` is "we could not ask". Those three are not
/// interchangeable.
pub trait ChainWatch {
    fn height(&self) -> Result<u64, WatchError>;
    fn confirmations(&self, tx: &TxRef) -> Result<Option<u64>, WatchError>;
    fn is_unspent(&self, out: &OutRef) -> Result<bool, WatchError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BroadcastResult {
    Accepted { txid: String },
    AlreadyKnown { txid: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MempoolAccept {
    Ok,
    Reject { reason: String },
}

pub trait Broadcaster {
    fn test_accept(&self, raw_hex: &str) -> Result<MempoolAccept, WatchError>;
    fn broadcast(&self, raw_hex: &str) -> Result<BroadcastResult, WatchError>;
}

/// In-memory chain. Heights, confirms, spends and outages are set by the test.
#[derive(Clone, Debug, Default)]
pub struct FakeWatch {
    pub height: u64,
    pub loading: bool,
    pub best_peer_height: u64,
    /// When set, every query returns this error. Distinct from height 0.
    pub outage: Option<WatchError>,
    /// A *partial* outage: the tip still reads, but transaction lookups fail.
    /// This is the realistic bitcoind failure — `getblockcount` answers from
    /// cache while `getrawtransaction` hits a index that is rebuilding or a
    /// connection that has dropped. Without it, no test can reach the code
    /// that reads confirmations, because `height()` errors first.
    pub confs_outage: Option<WatchError>,
    /// Present = known (0 = mempool). Absent = unknown. Never use 0 for "we
    /// could not ask".
    pub confs: HashMap<String, u64>,
    pub unspent: HashMap<(String, u32), bool>,
    pub sent: Vec<String>,
    pub reject_next: Option<String>,
}

impl FakeWatch {
    pub fn new(height: u64) -> Self {
        Self {
            height,
            ..Self::default()
        }
    }

    /// Drop `depth` blocks and strip that many confirms from every tx.
    /// A test that claims to handle reorgs must call this, not just lower
    /// `height` — otherwise confirms and tip disagree.
    pub fn reorg(&mut self, depth: u64) {
        self.height = self.height.saturating_sub(depth);
        for conf in self.confs.values_mut() {
            *conf = conf.saturating_sub(depth);
        }
    }

    pub fn set_confs(&mut self, txid: &str, n: u64) {
        self.confs.insert(txid.to_string(), n);
    }
}

impl ChainWatch for FakeWatch {
    fn height(&self) -> Result<u64, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        if self.loading {
            return Err(WatchError::Loading);
        }
        if self.best_peer_height > self.height {
            return Err(WatchError::Behind {
                ours: self.height,
                peer: self.best_peer_height,
            });
        }
        Ok(self.height)
    }

    fn confirmations(&self, tx: &TxRef) -> Result<Option<u64>, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        if let Some(e) = &self.confs_outage {
            return Err(e.clone());
        }
        if self.loading {
            return Err(WatchError::Loading);
        }
        Ok(self.confs.get(&tx.id).copied())
    }

    fn is_unspent(&self, out: &OutRef) -> Result<bool, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        Ok(*self
            .unspent
            .get(&(out.txid.clone(), out.vout))
            .unwrap_or(&true))
    }
}

impl Broadcaster for FakeWatch {
    fn test_accept(&self, _raw_hex: &str) -> Result<MempoolAccept, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        if let Some(r) = &self.reject_next {
            return Ok(MempoolAccept::Reject { reason: r.clone() });
        }
        Ok(MempoolAccept::Ok)
    }

    fn broadcast(&self, raw_hex: &str) -> Result<BroadcastResult, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        // Interior mutability would be nicer; tests use a Cell wrapper.
        let _ = raw_hex;
        Err(WatchError::Unavailable(
            "use FakeWatch::broadcast_mut in tests".into(),
        ))
    }
}

impl FakeWatch {
    pub fn broadcast_mut(
        &mut self,
        txid: &str,
        raw_hex: &str,
    ) -> Result<BroadcastResult, WatchError> {
        if let Some(e) = &self.outage {
            return Err(e.clone());
        }
        if self.sent.iter().any(|s| s == txid) {
            return Ok(BroadcastResult::AlreadyKnown {
                txid: txid.to_string(),
            });
        }
        self.sent.push(txid.to_string());
        self.confs.insert(txid.to_string(), 0);
        let _ = raw_hex;
        Ok(BroadcastResult::Accepted {
            txid: txid.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outage_is_not_zero_confirmations() {
        let mut w = FakeWatch::new(10);
        w.set_confs("abc", 3);
        w.outage = Some(WatchError::Unavailable("rpc down".into()));
        let q = w.confirmations(&TxRef { id: "abc".into() });
        assert!(matches!(q, Err(WatchError::Unavailable(_))));
        assert_ne!(q.ok().flatten(), Some(0));
    }

    #[test]
    fn a_reorg_of_50_drops_height_and_confirms() {
        let mut w = FakeWatch::new(80);
        w.set_confs("lock", 60);
        w.reorg(50);
        assert_eq!(w.height().unwrap(), 30);
        assert_eq!(
            w.confirmations(&TxRef { id: "lock".into() }).unwrap(),
            Some(10)
        );
    }

    #[test]
    fn loading_is_not_a_height() {
        let mut w = FakeWatch::new(12);
        w.loading = true;
        assert_eq!(w.height(), Err(WatchError::Loading));
    }

    #[test]
    fn behind_a_peer_is_not_synced() {
        let mut w = FakeWatch::new(10);
        w.best_peer_height = 40;
        assert!(matches!(
            w.height(),
            Err(WatchError::Behind { ours: 10, peer: 40 })
        ));
    }
}
