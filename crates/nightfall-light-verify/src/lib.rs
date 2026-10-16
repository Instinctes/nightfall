//! Independent header-chain and proof-of-work verifier for Nightfall.
//!
//! # Scope
//!
//! This crate re-implements header-chain verification without calling
//! into `nightfall-consensus`. It walks a chain of headers on its own
//! and checks the rules a light client needs: block linkage, proof of
//! work against declared difficulty, height continuity, difficulty
//! floor, and cumulative work.
//!
//! It does **not** read the UTXO set, the supply counters, or any
//! wallet state.
//!
//! # What this crate shares
//!
//! It shares the cryptographic primitives `nighthash`, `meets_difficulty`,
//! `hash_multi`, and the domain-separation constants from `nightfall-crypto`.
//! Re-implementing Argon2id would not add security value — if the
//! primitive is broken, no second walker helps. What matters is that
//! the *rules* on top of the primitive are implemented twice.
//!
//! # What this crate does not import
//!
//! - `nightfall-consensus` (chain logic)
//! - `nightfall-ledger` (UTXO / supply state)
//! - `nightfall-node`, `nightfall-storage`, `nightfall-wallet`, `nightfall-p2p`
//!
//! # Design intent
//!
//! If the consensus crate has a bug in header linkage, work
//! accumulation, or PoW checking, this crate should still detect it,
//! because it re-derives every one of those facts from the raw header
//! fields. See RFC `docs/RFC-LIGHT-VERIFY.md` for the roadmap.

use nightfall_crypto::{domain, hash_multi, meets_difficulty, nighthash, Commitment};
use nightfall_types::{Hash256, PowParams};

/// A block header as it appears on the wire.
///
/// Deliberately a separate type from
/// `nightfall_consensus::BlockHeader`. If the two layouts diverge,
/// that is a bug — the wire format is consensus data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderView {
    pub version: u32,
    pub height: u64,
    pub prev_hash: Hash256,
    pub utxo_root: Hash256,
    pub kernel_sum: Commitment,
    pub body_root: Hash256,
    pub timestamp_unix: u64,
    pub difficulty: u64,
    pub nonce: u64,
    pub reward_darks: u64,
}

impl HeaderView {
    /// Bytes hashed for proof of work. Excludes the nonce.
    pub fn pow_preimage(&self) -> Vec<u8> {
        hash_multi(
            domain::BLOCK,
            &[
                &self.version.to_le_bytes(),
                &self.height.to_le_bytes(),
                &self.prev_hash.0,
                &self.utxo_root.0,
                &self.kernel_sum.0,
                &self.body_root.0,
                &self.timestamp_unix.to_le_bytes(),
                &self.difficulty.to_le_bytes(),
                &self.reward_darks.to_le_bytes(),
            ],
        )
        .0
        .to_vec()
    }

    /// Proof-of-work hash under the given parameters.
    pub fn pow_hash(&self, params: PowParams) -> Hash256 {
        nighthash(&self.pow_preimage(), self.nonce, params)
    }

    /// Canonical header identity — includes the nonce, unlike the
    /// PoW preimage.
    pub fn hash(&self) -> Hash256 {
        hash_multi(
            domain::BLOCK,
            &[&self.pow_preimage(), &self.nonce.to_le_bytes()],
        )
    }

    /// Work contributed by this header. Equals the declared difficulty.
    pub fn work(&self) -> u128 {
        block_work(self.difficulty)
    }
}

/// Work contributed by a block of the given difficulty.
pub fn block_work(difficulty: u64) -> u128 {
    u128::from(difficulty)
}

/// A verified header chain. The only facts a light client needs after
/// the walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedChain {
    pub tip_hash: Hash256,
    pub tip_height: u64,
    pub cumulative_work: u128,
}

/// Errors surfaced during header-chain verification.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("empty header chain")]
    EmptyChain,

    #[error("header {index} has height {got}, expected {expected}")]
    HeightDiscontinuity {
        index: usize,
        expected: u64,
        got: u64,
    },

    #[error("header {index} does not link to its predecessor")]
    BrokenLink { index: usize },

    #[error("header {index} proof of work does not meet its declared difficulty")]
    PowNotMet { index: usize },

    #[error("header {index} difficulty {got} is below the network floor {min}")]
    DifficultyBelowFloor { index: usize, got: u64, min: u64 },

    #[error("header {index} timestamp is not strictly increasing")]
    NonIncreasingTimestamp { index: usize },

    #[error("cumulative work overflowed u128")]
    WorkOverflow,
}

/// Verifier configuration.
#[derive(Clone, Copy, Debug)]
pub struct VerifierConfig {
    /// Proof-of-work parameters. These are consensus data, not a
    /// local choice — a node using different parameters forks off.
    pub pow_params: PowParams,
    /// Network difficulty floor. Headers below this are rejected.
    pub min_difficulty: u64,
    /// Whether to require strictly increasing timestamps.
    pub enforce_monotonic_time: bool,
}

/// Verify a chain of headers with no external state.
///
/// The walk is purely mechanical:
///
/// 1. The first header must satisfy its declared difficulty and the
///    network floor. Its linkage to a genesis block is the caller's
///    responsibility, because only the caller knows the expected
///    genesis hash for the network.
/// 2. Every subsequent header must:
///    - sit at exactly `prev.height + 1`
///    - carry `prev_hash = prev.hash()`
///    - satisfy its declared difficulty and the network floor
///    - (optionally) carry a strictly larger timestamp
///
/// Work is summed exactly. Nothing here calls into `nightfall-consensus`.
pub fn verify_chain(
    headers: &[HeaderView],
    config: VerifierConfig,
) -> Result<VerifiedChain, VerifyError> {
    let Some(first) = headers.first() else {
        return Err(VerifyError::EmptyChain);
    };

    if first.difficulty < config.min_difficulty {
        return Err(VerifyError::DifficultyBelowFloor {
            index: 0,
            got: first.difficulty,
            min: config.min_difficulty,
        });
    }
    if !meets_difficulty(first.pow_hash(config.pow_params), first.difficulty) {
        return Err(VerifyError::PowNotMet { index: 0 });
    }

    let mut cumulative_work: u128 = first.work();
    let mut prev = first;
    let mut prev_height = first.height;

    for (i, header) in headers.iter().enumerate().skip(1) {
        let expected_height = prev_height
            .checked_add(1)
            .ok_or(VerifyError::WorkOverflow)?;

        if header.height != expected_height {
            return Err(VerifyError::HeightDiscontinuity {
                index: i,
                expected: expected_height,
                got: header.height,
            });
        }

        if header.prev_hash != prev.hash() {
            return Err(VerifyError::BrokenLink { index: i });
        }

        if config.enforce_monotonic_time && header.timestamp_unix <= prev.timestamp_unix {
            return Err(VerifyError::NonIncreasingTimestamp { index: i });
        }

        if header.difficulty < config.min_difficulty {
            return Err(VerifyError::DifficultyBelowFloor {
                index: i,
                got: header.difficulty,
                min: config.min_difficulty,
            });
        }

        if !meets_difficulty(header.pow_hash(config.pow_params), header.difficulty) {
            return Err(VerifyError::PowNotMet { index: i });
        }

        cumulative_work = cumulative_work
            .checked_add(header.work())
            .ok_or(VerifyError::WorkOverflow)?;

        prev = header;
        prev_height = header.height;
    }

    Ok(VerifiedChain {
        tip_hash: prev.hash(),
        tip_height: prev_height,
        cumulative_work,
    })
}

/// Does `a` carry strictly more work than `b`?
///
/// Light clients use this to decide whether a fresh peer's chain is
/// worth switching to. It is the only comparison the header-level
/// verifier needs to expose.
pub fn chain_is_heavier(a: &VerifiedChain, b: &VerifiedChain) -> bool {
    a.cumulative_work > b.cumulative_work
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> PowParams {
        // Minimum Argon2id parameters so tests run fast. This is not
        // the network setting — it is only used inside unit tests
        // where both the miner and the verifier agree on it.
        PowParams {
            memory_kib: 8,
            iterations: 1,
            lanes: 1,
        }
    }

    /// Mine a header that satisfies `difficulty` under `params`.
    fn mined_header(
        height: u64,
        prev_hash: Hash256,
        difficulty: u64,
        params: PowParams,
        timestamp: u64,
    ) -> HeaderView {
        let mut header = HeaderView {
            version: 1,
            height,
            prev_hash,
            utxo_root: Hash256([0u8; 32]),
            kernel_sum: Commitment::identity(),
            body_root: Hash256([0u8; 32]),
            timestamp_unix: timestamp,
            difficulty,
            nonce: 0,
            reward_darks: 0,
        };

        for nonce in 0..10_000_000u64 {
            header.nonce = nonce;
            if meets_difficulty(header.pow_hash(params), difficulty) {
                return header;
            }
        }
        panic!("failed to mine header at difficulty {difficulty}");
    }

    fn default_config(params: PowParams) -> VerifierConfig {
        VerifierConfig {
            pow_params: params,
            min_difficulty: 1,
            enforce_monotonic_time: true,
        }
    }

    #[test]
    fn empty_chain_is_rejected() {
        let err = verify_chain(&[], default_config(test_params())).unwrap_err();
        assert_eq!(err, VerifyError::EmptyChain);
    }

    #[test]
    fn single_header_is_accepted() {
        let p = test_params();
        let g = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let chain = verify_chain(&[g], default_config(p)).unwrap();
        assert_eq!(chain.tip_height, 0);
        assert_eq!(chain.cumulative_work, 1);
    }

    #[test]
    fn two_header_chain_links_correctly() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let h1 = mined_header(1, h0.hash(), 1, p, 1001);
        let chain = verify_chain(&[h0, h1], default_config(p)).unwrap();
        assert_eq!(chain.tip_height, 1);
        assert_eq!(chain.cumulative_work, 2);
    }

    #[test]
    fn broken_link_is_rejected() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let h1 = mined_header(1, Hash256([0xFFu8; 32]), 1, p, 1001);
        let err = verify_chain(&[h0, h1], default_config(p)).unwrap_err();
        assert_eq!(err, VerifyError::BrokenLink { index: 1 });
    }

    #[test]
    fn height_gap_is_rejected() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let h2 = mined_header(2, h0.hash(), 1, p, 1002);
        let err = verify_chain(&[h0, h2], default_config(p)).unwrap_err();
        assert_eq!(
            err,
            VerifyError::HeightDiscontinuity {
                index: 1,
                expected: 1,
                got: 2,
            }
        );
    }

    #[test]
    fn non_monotonic_timestamp_is_rejected() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let h1 = mined_header(1, h0.hash(), 1, p, 1000);
        let err = verify_chain(&[h0, h1], default_config(p)).unwrap_err();
        assert_eq!(err, VerifyError::NonIncreasingTimestamp { index: 1 });
    }

    #[test]
    fn monotonic_time_check_can_be_disabled() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let h1 = mined_header(1, h0.hash(), 1, p, 1000);
        let cfg = VerifierConfig {
            pow_params: p,
            min_difficulty: 1,
            enforce_monotonic_time: false,
        };
        assert!(verify_chain(&[h0, h1], cfg).is_ok());
    }

    #[test]
    fn difficulty_below_floor_is_rejected() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let cfg = VerifierConfig {
            pow_params: p,
            min_difficulty: 2,
            enforce_monotonic_time: true,
        };
        let err = verify_chain(&[h0], cfg).unwrap_err();
        assert!(matches!(err, VerifyError::DifficultyBelowFloor { .. }));
    }

    #[test]
    fn work_accumulates_across_difficulties() {
        let p = test_params();
        let h0 = mined_header(0, Hash256([0u8; 32]), 10, p, 1000);
        let h1 = mined_header(1, h0.hash(), 20, p, 1001);
        let h2 = mined_header(2, h1.hash(), 30, p, 1002);
        let chain = verify_chain(&[h0, h1, h2], default_config(p)).unwrap();
        assert_eq!(chain.cumulative_work, 60);
    }

    #[test]
    fn heavier_chain_wins() {
        let p = test_params();

        let short = {
            let h0 = mined_header(0, Hash256([0u8; 32]), 5, p, 1000);
            verify_chain(&[h0], default_config(p)).unwrap()
        };

        let long = {
            let h0 = mined_header(0, Hash256([0u8; 32]), 5, p, 1000);
            let h1 = mined_header(1, h0.hash(), 5, p, 1001);
            verify_chain(&[h0, h1], default_config(p)).unwrap()
        };

        assert!(chain_is_heavier(&long, &short));
        assert!(!chain_is_heavier(&short, &long));
        assert!(!chain_is_heavier(&short, &short));
    }

    #[test]
    fn chain_id_binding_via_first_header_height_is_explicit() {
        // The verifier deliberately does not know the genesis hash.
        // Callers must check `headers[0].hash()` against the network's
        // expected genesis. This test documents that contract.
        let p = test_params();
        let g = mined_header(0, Hash256([0u8; 32]), 1, p, 1000);
        let chain = verify_chain(std::slice::from_ref(&g), default_config(p)).unwrap();
        assert_eq!(chain.tip_hash, g.hash());
    }
}
