//! Confirmation depths, derived rather than guessed.
//!
//! NIGHT: a node adopts a reorg up to [`nightfall_consensus::MAX_REORG_DEPTH`]
//! (500 blocks, ~2 hours at 15 s). Waiting less than that before an
//! irreversible step (Alice redeeming BTC after the NIGHT lock confirms)
//! means a deeper reorg is theft. We wait the bound.
//!
//! Bitcoin: no equivalent bound. 6 blocks is the conventional residual
//! (~1 hour). H₁ must sit well after the NIGHT wait in wall-clock time:
//! 500 × 15 s ≈ 12.5 BTC blocks, so H₁ = 144 (~24 h) leaves margin.
//! Alice must not redeem if fewer than [`Self::btc_redeem_margin`] blocks
//! remain before H₁.

use nightfall_consensus::MAX_REORG_DEPTH;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Depths {
    /// NIGHT confirmations before treating a lock as irreversible.
    pub night: u64,
    /// Bitcoin confirmations before treating TX_lock / TX_redeem as final.
    pub bitcoin: u32,
    /// Relative CSV on TX_cancel (blocks after TX_lock).
    pub cancel: u32,
    /// Relative CSV on TX_punish (blocks after TX_cancel).
    pub punish: u32,
    /// Alice refuses to redeem if fewer than this many BTC blocks remain to H₁.
    pub btc_redeem_margin: u32,
}

impl Depths {
    /// Production numbers. Residual risk is written in the wallet view.
    pub fn mainnet() -> Self {
        Self {
            night: MAX_REORG_DEPTH as u64,
            bitcoin: 6,
            cancel: 144,
            punish: 144,
            btc_redeem_margin: 12,
        }
    }

    pub fn testnet() -> Self {
        Self {
            night: 60,
            bitcoin: 3,
            cancel: 12,
            punish: 12,
            btc_redeem_margin: 3,
        }
    }

    /// Tiny depths so tests can drive every abort without mining hundreds of blocks.
    pub fn testdrive() -> Self {
        Self {
            night: 2,
            bitcoin: 1,
            cancel: 4,
            punish: 4,
            btc_redeem_margin: 2,
        }
    }

    /// Local / regtest. Blocks arrive on demand, so public-testnet CSVs
    /// (12 blocks ≈ two hours of wall time on testnet, seconds here) are
    /// the wrong unit. Same numbers as [`Self::testdrive`]: still BIP68-valid
    /// and still a redeem window after the Bitcoin lock confirms.
    pub fn devnet() -> Self {
        Self::testdrive()
    }

    /// Spec §9.2: do not begin phase 3 if the remaining cancel window is
    /// thinner than the margin.
    pub fn may_redeem(&self, lock_confirmations: u32) -> bool {
        lock_confirmations + self.btc_redeem_margin < self.cancel
    }

    /// Alice's check on Bob's opening depths. He chooses them; she can
    /// only refuse. A `cancel` that is already inside the redeem margin
    /// after the lock confirms means she can never take the Bitcoin —
    /// she would lock NIGHT into a swap she cannot finish.
    ///
    /// CSV values above 65535 wrap in BIP68 (`csv_height`); refuse those
    /// here so the handshake fails before anything is signed.
    pub fn alice_can_finish(&self) -> bool {
        self.night >= 1
            && self.bitcoin >= 1
            && self.btc_redeem_margin >= 1
            && self.punish >= 1
            && self.cancel <= 65535
            && self.punish <= 65535
            && self.may_redeem(self.bitcoin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_night_depth_is_the_reorg_bound() {
        assert_eq!(Depths::mainnet().night, MAX_REORG_DEPTH as u64);
    }

    #[test]
    fn redeem_cutoff_fires_inside_the_margin() {
        let d = Depths::testdrive();
        assert!(d.may_redeem(0));
        assert!(d.may_redeem(1));
        assert!(!d.may_redeem(2));
        assert!(!d.may_redeem(4));
    }

    #[test]
    fn honest_presets_leave_alice_a_redeem_window() {
        assert!(Depths::mainnet().alice_can_finish());
        assert!(Depths::testnet().alice_can_finish());
        assert!(Depths::testdrive().alice_can_finish());
        assert!(Depths::devnet().alice_can_finish());
    }

    #[test]
    fn a_cancel_inside_the_margin_is_not_safe_for_alice() {
        let mut d = Depths::testdrive();
        d.cancel = d.bitcoin + d.btc_redeem_margin; // may_redeem(bitcoin) is false
        assert!(!d.alice_can_finish());
        d.cancel = 0;
        assert!(!d.alice_can_finish());
        d = Depths::testdrive();
        d.cancel = 70_000; // would wrap in BIP68
        assert!(!d.alice_can_finish());
    }
}
