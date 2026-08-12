//! The UTXO commitment set and the global supply invariant.
//!
//! # The invariant
//!
//! Summing the balance equation over every transaction that ever confirmed:
//!
//! ```text
//!   Σ UTXO − Σ kernel_excess  =  (total_minted − total_burned) · G
//! ```
//!
//! Anyone holding the UTXO set and the kernel set can evaluate this in one pass
//! and know, cryptographically, that not a single extra dark exists. If someone
//! ever found a way to inflate, this equation breaks and every node sees it.
//!
//! v4 had no such property. `total_minted_darks` was a plain counter that a
//! forged transaction never touched, and the 90 M cap was checked nowhere in
//! the ledger at all (audit findings C-01, S-01).

use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::traits::Identity;
use nightfall_crypto::{commit_public_value, domain, hash_multi, Commitment};
use nightfall_types::{Hash256, Height, NetworkId, MAX_SUPPLY_DARKS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An unspent output as the chain remembers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtxoEntry {
    /// One-time key that must sign to spend this output.
    pub output_pk: [u8; 32],
    /// Height at which it was created — drives coinbase maturity.
    pub height: u64,
    /// Coinbase outputs are subject to a maturity delay.
    pub is_coinbase: bool,
}

/// Coinbase outputs cannot be spent until this many blocks have passed.
///
/// Without maturity, a reorg that orphans a block also invalidates every
/// transaction that spent its subsidy, cascading arbitrarily far.
pub const COINBASE_MATURITY: u64 = 1_440; // ~6 h at 15 s blocks

/// Maturity per network. Mainnet gets the full delay; devnet gets a short one
/// so the spend path is actually testable without mining for six hours.
pub fn coinbase_maturity(network: NetworkId) -> u64 {
    match network {
        NetworkId::Mainnet => COINBASE_MATURITY,
        NetworkId::Testnet => 60,
        NetworkId::Devnet => 10,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UtxoSet {
    /// Sorted by commitment so the root is canonical.
    pub entries: BTreeMap<[u8; 32], UtxoEntry>,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, commit: &Commitment) -> Option<&UtxoEntry> {
        self.entries.get(&commit.0)
    }

    pub fn contains(&self, commit: &Commitment) -> bool {
        self.entries.contains_key(&commit.0)
    }

    pub fn insert(&mut self, commit: Commitment, entry: UtxoEntry) -> bool {
        self.entries.insert(commit.0, entry).is_none()
    }

    pub fn remove(&mut self, commit: &Commitment) -> Option<UtxoEntry> {
        self.entries.remove(&commit.0)
    }

    /// Σ of all unspent commitments.
    pub fn commitment_sum(&self) -> Option<RistrettoPoint> {
        let mut acc = RistrettoPoint::identity();
        for k in self.entries.keys() {
            acc += Commitment(*k).point()?;
        }
        Some(acc)
    }

    /// Merkle root over the sorted commitment set.
    ///
    /// O(n log n) per block. v4 recomputed an O(n) root after *every single
    /// transaction*, giving O(n²) per block — the chain would have ground to a
    /// halt within days (audit finding N-04). A Merkle Mountain Range would
    /// make this incremental; that is the next optimisation, not a correctness
    /// issue.
    pub fn root(&self) -> Hash256 {
        if self.entries.is_empty() {
            return Hash256::ZERO;
        }
        let mut level: Vec<Hash256> = self
            .entries
            .iter()
            .map(|(commit, e)| {
                hash_multi(
                    domain::MERKLE_LEAF,
                    &[commit, &e.output_pk, &e.height.to_le_bytes()],
                )
            })
            .collect();

        while level.len() > 1 {
            let mut next = Vec::with_capacity(level.len().div_ceil(2));
            for chunk in level.chunks(2) {
                let right = if chunk.len() == 2 { chunk[1] } else { chunk[0] };
                next.push(hash_multi(domain::MERKLE, &[&chunk[0].0, &right.0]));
            }
            level = next;
        }
        level[0]
    }
}

/// Running total of every kernel excess ever accepted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KernelAccumulator {
    /// Compressed running sum.
    pub sum: Commitment,
    pub count: u64,
}

impl Default for KernelAccumulator {
    fn default() -> Self {
        Self {
            sum: Commitment::identity(),
            count: 0,
        }
    }
}

impl KernelAccumulator {
    pub fn add(&mut self, excess: &Commitment) -> Option<()> {
        let acc = self.sum.point()?;
        let e = excess.point()?;
        self.sum = Commitment::from_point(acc + e);
        self.count += 1;
        Some(())
    }

    pub fn point(&self) -> Option<RistrettoPoint> {
        self.sum.point()
    }
}

/// Supply bookkeeping, cryptographically cross-checkable.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SupplyState {
    pub total_minted_darks: u64,
    pub total_burned_darks: u64,
}

impl SupplyState {
    /// Coins that actually exist: minted minus burned fees.
    pub fn circulating(&self) -> u64 {
        self.total_minted_darks
            .saturating_sub(self.total_burned_darks)
    }

    /// Hard cap check. This is the line that was entirely missing in v4.
    pub fn would_exceed_cap(&self, additional_mint: u64) -> bool {
        self.total_minted_darks
            .checked_add(additional_mint)
            .map(|t| t > MAX_SUPPLY_DARKS)
            .unwrap_or(true)
    }
}

/// Verify `Σ UTXO − Σ kernel_excess == circulating·G`.
pub fn verify_supply_invariant(
    utxos: &UtxoSet,
    kernels: &KernelAccumulator,
    supply: &SupplyState,
) -> Result<(), SupplyError> {
    let utxo_sum = utxos.commitment_sum().ok_or(SupplyError::MalformedUtxo)?;
    let kernel_sum = kernels.point().ok_or(SupplyError::MalformedKernelSum)?;
    let expected = commit_public_value(supply.circulating());

    if utxo_sum - kernel_sum != expected {
        return Err(SupplyError::InvariantViolated);
    }
    if supply.total_minted_darks > MAX_SUPPLY_DARKS {
        return Err(SupplyError::CapExceeded);
    }
    Ok(())
}

/// Is a coinbase output old enough to spend?
pub fn is_mature(entry: &UtxoEntry, spend_height: Height, maturity: u64) -> bool {
    if !entry.is_coinbase {
        return true;
    }
    spend_height.0 >= entry.height.saturating_add(maturity)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SupplyError {
    #[error("malformed commitment in UTXO set")]
    MalformedUtxo,
    #[error("malformed kernel excess sum")]
    MalformedKernelSum,
    #[error("SUPPLY INVARIANT VIOLATED — coins exist that were never minted")]
    InvariantViolated,
    #[error("total minted exceeds the 90,000,000 NIGHT hard cap")]
    CapExceeded,
}

#[cfg(test)]
mod tests {
    use super::*;
    use curve25519_dalek::scalar::Scalar;
    use rand::rngs::OsRng;

    fn entry() -> UtxoEntry {
        UtxoEntry {
            output_pk: [0u8; 32],
            height: 0,
            is_coinbase: false,
        }
    }

    #[test]
    fn invariant_holds_for_a_clean_mint() {
        // Mint 100: one output of 100 with blind b, kernel excess b·H.
        let b = Scalar::random(&mut OsRng);
        let mut utxos = UtxoSet::new();
        utxos.insert(Commitment::new(100, &b), entry());

        let mut kernels = KernelAccumulator::default();
        let excess = Commitment::from_point(nightfall_crypto::generator_h() * b);
        kernels.add(&excess).unwrap();

        let supply = SupplyState {
            total_minted_darks: 100,
            total_burned_darks: 0,
        };
        assert!(verify_supply_invariant(&utxos, &kernels, &supply).is_ok());
    }

    #[test]
    fn invariant_catches_phantom_coins() {
        // Same as above but the UTXO secretly holds 1_000_000 instead of 100.
        let b = Scalar::random(&mut OsRng);
        let mut utxos = UtxoSet::new();
        utxos.insert(Commitment::new(1_000_000, &b), entry());

        let mut kernels = KernelAccumulator::default();
        kernels
            .add(&Commitment::from_point(nightfall_crypto::generator_h() * b))
            .unwrap();

        let supply = SupplyState {
            total_minted_darks: 100,
            total_burned_darks: 0,
        };
        assert_eq!(
            verify_supply_invariant(&utxos, &kernels, &supply),
            Err(SupplyError::InvariantViolated),
            "the whole point of the invariant"
        );
    }

    #[test]
    fn fee_burn_is_reflected() {
        // Mint 100, burn 10 as fee: circulating is 90.
        let b1 = Scalar::random(&mut OsRng);
        let b2 = Scalar::random(&mut OsRng);
        let mut utxos = UtxoSet::new();
        utxos.insert(Commitment::new(90, &b2), entry());

        let mut kernels = KernelAccumulator::default();
        // coinbase kernel: excess = b1
        kernels
            .add(&Commitment::from_point(
                nightfall_crypto::generator_h() * b1,
            ))
            .unwrap();
        // spend kernel: excess = b2 − b1
        kernels
            .add(&Commitment::from_point(
                nightfall_crypto::generator_h() * (b2 - b1),
            ))
            .unwrap();

        let supply = SupplyState {
            total_minted_darks: 100,
            total_burned_darks: 10,
        };
        assert!(verify_supply_invariant(&utxos, &kernels, &supply).is_ok());
    }

    #[test]
    fn cap_is_enforced() {
        let s = SupplyState {
            total_minted_darks: MAX_SUPPLY_DARKS,
            total_burned_darks: 0,
        };
        assert!(s.would_exceed_cap(1));
        assert!(!s.would_exceed_cap(0));
    }

    #[test]
    fn root_is_order_independent_and_changes_with_content() {
        let b = Scalar::random(&mut OsRng);
        let c1 = Commitment::new(1, &b);
        let c2 = Commitment::new(2, &b);

        let mut a = UtxoSet::new();
        a.insert(c1, entry());
        a.insert(c2, entry());

        let mut b2 = UtxoSet::new();
        b2.insert(c2, entry());
        b2.insert(c1, entry());

        assert_eq!(
            a.root(),
            b2.root(),
            "root must not depend on insertion order"
        );

        b2.remove(&c1);
        assert_ne!(a.root(), b2.root());
    }

    #[test]
    fn coinbase_maturity_is_enforced() {
        let m = COINBASE_MATURITY;
        let cb = UtxoEntry {
            output_pk: [0u8; 32],
            height: 10,
            is_coinbase: true,
        };
        assert!(!is_mature(&cb, Height(10), m));
        assert!(!is_mature(&cb, Height(10 + m - 1), m));
        assert!(is_mature(&cb, Height(10 + m), m));
        // Regular outputs are always spendable.
        assert!(is_mature(&entry(), Height(0), m));
    }

    #[test]
    fn maturity_differs_per_network() {
        assert_eq!(coinbase_maturity(NetworkId::Mainnet), COINBASE_MATURITY);
        assert!(coinbase_maturity(NetworkId::Devnet) < COINBASE_MATURITY);
        assert!(coinbase_maturity(NetworkId::Testnet) < COINBASE_MATURITY);
    }
}
