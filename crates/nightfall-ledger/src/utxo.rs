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

    /// Running Σ of every entry's commitment point.
    ///
    /// `commitment_sum` used to walk the whole map and decompress every
    /// commitment, and `verify_supply_invariant` calls it once per block. That
    /// is O(n) point decompressions per block and O(n²) over a chain — with n
    /// growing, so it got worse every week. Measured on a running node: 1730
    /// of 2302 stack samples inside `verify_supply_invariant`, 1599 of those
    /// in `CompressedRistretto::decompress`. Proof of work, which the log line
    /// blamed, did not appear in the profile at all.
    ///
    /// Carried forward instead: add on insert, subtract on remove. Every
    /// mutation goes through those two methods — `entries` is only ever read
    /// from outside this file — so the running value cannot drift behind the
    /// map's back.
    ///
    /// Not serialised. A sum read from a file is a sum somebody else computed,
    /// and this one stands between the chain and coins minted out of nowhere.
    #[serde(skip)]
    sum: RistrettoPoint,

    /// Entries whose commitment does not decompress to a point.
    ///
    /// Such an entry has no contribution to add, so the running sum would
    /// silently be a sum over a *different* set than the one on disk. Counting
    /// them lets `commitment_sum` report None exactly as the walking version
    /// did, rather than returning a confident wrong answer.
    #[serde(skip)]
    malformed: usize,

    /// How many entries the running sum accounts for.
    ///
    /// This is what makes the optimisation safe to get wrong. The obvious
    /// design — rebuild the sum after loading — depends on every caller
    /// remembering to, and the one that forgets does not crash: it computes
    /// the supply invariant against a sum of zero entries, which is the exact
    /// check that stands between this chain and invented coins. A guard that
    /// fails silently in the direction of "no verification" is not a guard.
    ///
    /// So the count travels with the sum, and `commitment_sum` compares it to
    /// the map. Deserialisation leaves it at zero while `entries` is full, the
    /// two disagree, and the walking version runs. Being slow is recoverable;
    /// being confidently wrong here is not.
    #[serde(skip)]
    summed: usize,
}

impl UtxoSet {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            sum: RistrettoPoint::identity(),
            malformed: 0,
            summed: 0,
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
        let fresh = self.entries.insert(commit.0, entry).is_none();
        // Only a genuinely new commitment moves the sum. Re-inserting an
        // existing one replaces the entry's metadata, not the point, and
        // adding it twice would make the invariant reject an honest chain.
        if fresh {
            self.summed += 1;
            match commit.point() {
                Some(p) => self.sum += p,
                None => self.malformed += 1,
            }
        }
        fresh
    }

    pub fn remove(&mut self, commit: &Commitment) -> Option<UtxoEntry> {
        let gone = self.entries.remove(&commit.0);
        if gone.is_some() {
            self.summed = self.summed.saturating_sub(1);
            match commit.point() {
                Some(p) => self.sum -= p,
                None => self.malformed = self.malformed.saturating_sub(1),
            }
        }
        gone
    }

    /// Σ of all unspent commitments.
    ///
    /// O(1) whenever the running sum covers the whole map, which is every
    /// path that built the set through `insert`/`remove` — that is, all of
    /// them today. Falls back to walking when the two disagree, which is what
    /// a deserialised set looks like.
    pub fn commitment_sum(&self) -> Option<RistrettoPoint> {
        if self.summed != self.entries.len() {
            return self.commitment_sum_by_walking();
        }
        if self.malformed > 0 {
            return None;
        }
        Some(self.sum)
    }

    /// The definition: Σ over the map, decompressing as it goes.
    ///
    /// Kept, not deleted. It is the fallback above, and it is what the tests
    /// pin the running sum against — an optimisation with no independent
    /// statement of what it should equal is an optimisation nobody can check.
    fn commitment_sum_by_walking(&self) -> Option<RistrettoPoint> {
        let mut acc = RistrettoPoint::identity();
        for k in self.entries.keys() {
            acc += Commitment(*k).point()?;
        }
        Some(acc)
    }

    /// Bring the running sum up to date with the entries.
    ///
    /// An optimisation, never a correctness requirement: `commitment_sum`
    /// already gives the right answer without it, just slowly. Call it after
    /// loading a set that did not come through `insert`.
    pub fn rebuild_sum(&mut self) {
        let mut acc = RistrettoPoint::identity();
        let mut bad = 0usize;
        for k in self.entries.keys() {
            match Commitment(*k).point() {
                Some(p) => acc += p,
                None => bad += 1,
            }
        }
        self.sum = acc;
        self.malformed = bad;
        self.summed = self.entries.len();
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

    /// The running sum must equal the definition after any mix of operations.
    ///
    /// Not a smoke test. `commitment_sum` is one side of the equation that
    /// decides whether coins were minted from nothing, and it was just
    /// replaced by a value carried forward across thousands of mutations. If
    /// it drifts, the invariant still "passes" — against the wrong number.
    #[test]
    fn the_running_sum_equals_walking_the_set_after_any_sequence() {
        use rand::Rng;
        let mut rng = OsRng;
        let mut utxos = UtxoSet::new();
        let mut live: Vec<Commitment> = Vec::new();

        for step in 0..400 {
            // Insert, remove, re-insert, and remove things that were never
            // there — every path that touches the counters.
            let roll: u8 = rng.gen_range(0..100);
            if roll < 55 || live.is_empty() {
                let c = Commitment::new(rng.gen_range(0..1_000u64), &Scalar::random(&mut rng));
                utxos.insert(c, entry());
                live.push(c);
            } else if roll < 75 {
                let i = rng.gen_range(0..live.len());
                let c = live.swap_remove(i);
                utxos.remove(&c);
            } else if roll < 90 {
                // Re-insert something already present: must not double-count.
                let c = live[rng.gen_range(0..live.len())];
                utxos.insert(c, entry());
            } else {
                // Remove something absent: must not touch the sum.
                let c = Commitment::new(7, &Scalar::random(&mut rng));
                utxos.remove(&c);
            }

            assert_eq!(
                utxos.commitment_sum().map(|p| p.compress()),
                utxos.commitment_sum_by_walking().map(|p| p.compress()),
                "running sum drifted from the definition at step {step}",
            );
            assert_eq!(utxos.summed, utxos.entries.len(), "counter drifted");
        }
    }

    /// A deserialised set has no running sum, and must still answer correctly.
    ///
    /// This is the trap the counter exists for. Without it the sum would come
    /// back as the identity — a supply invariant computed over zero entries,
    /// which is the check silently switching itself off.
    #[test]
    fn a_deserialised_set_still_reports_the_real_sum() {
        let mut utxos = UtxoSet::new();
        for v in 1..25u64 {
            utxos.insert(Commitment::new(v, &Scalar::random(&mut OsRng)), entry());
        }
        let want = utxos.commitment_sum().expect("well-formed").compress();

        // Exactly what serde leaves behind: the map is restored, the three
        // #[serde(skip)] fields are at their defaults. Built by hand rather
        // than through a serialiser because the map is keyed by [u8; 32] —
        // JSON refuses it outright — and the point of the test is the state,
        // not the format that produces it.
        let round = UtxoSet {
            entries: utxos.entries.clone(),
            sum: RistrettoPoint::identity(),
            malformed: 0,
            summed: 0,
        };
        assert_eq!(round.entries.len(), utxos.entries.len());
        assert_eq!(
            round.commitment_sum().expect("still computable").compress(),
            want,
            "a set that came back from a file reported a different supply",
        );

        // And after rebuilding it takes the fast path with the same answer.
        let mut rebuilt = round;
        rebuilt.rebuild_sum();
        assert_eq!(rebuilt.summed, rebuilt.entries.len());
        assert_eq!(rebuilt.commitment_sum().unwrap().compress(), want);
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
