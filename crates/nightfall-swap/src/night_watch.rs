//! NIGHT observation from an in-process node snapshot.
//!
//! `tip_height` while `loading` is a lie. A peer height ahead of us is lag,
//! not a stalled chain. Both must fail closed.

use crate::watch::{ChainWatch, OutRef, TxRef, WatchError};

#[derive(Clone, Debug, Default)]
pub struct NightSnap {
    pub loading: bool,
    pub height: u64,
    pub best_peer_height: u64,
    /// Commitment hex → confirmations of that output (tip - created + 1).
    pub confs: std::collections::HashMap<String, u64>,
    pub unspent: std::collections::HashMap<String, bool>,
}

pub struct NightWatch {
    pub snap: NightSnap,
}

impl ChainWatch for NightWatch {
    fn height(&self) -> Result<u64, WatchError> {
        if self.snap.loading {
            return Err(WatchError::Loading);
        }
        if self.snap.best_peer_height > self.snap.height {
            return Err(WatchError::Behind {
                ours: self.snap.height,
                peer: self.snap.best_peer_height,
            });
        }
        Ok(self.snap.height)
    }

    fn confirmations(&self, tx: &TxRef) -> Result<Option<u64>, WatchError> {
        if self.snap.loading {
            return Err(WatchError::Loading);
        }
        Ok(self.snap.confs.get(&tx.id).copied())
    }

    fn is_unspent(&self, out: &OutRef) -> Result<bool, WatchError> {
        if self.snap.loading {
            return Err(WatchError::Loading);
        }
        Ok(*self.snap.unspent.get(&out.txid).unwrap_or(&true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_is_an_error_not_a_stale_tip() {
        let w = NightWatch {
            snap: NightSnap {
                loading: true,
                height: 99_999,
                ..NightSnap::default()
            },
        };
        assert_eq!(w.height(), Err(WatchError::Loading));
        assert_eq!(
            w.confirmations(&TxRef { id: "x".into() }),
            Err(WatchError::Loading)
        );
    }
}
