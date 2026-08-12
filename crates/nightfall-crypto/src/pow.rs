//! Nighthash-v2 proof of work and 256-bit difficulty arithmetic.
//!
//! # The hash
//!
//! ```text
//!   salt = Blake3("nightfall:nighthash:salt:v2" ‖ header_preimage)[..16]
//!   pow  = Argon2id(pwd = header_preimage ‖ nonce_le, salt, m, t, p) -> 32 bytes
//! ```
//!
//! Argon2id is **memory-hard**: producing one hash requires `memory_kib` of
//! randomly-addressed working memory. Hardware that wants to hash in parallel
//! must supply that much fast RAM per unit, which is the property that keeps
//! mining economical on ordinary CPUs.
//!
//! v4 used `Blake3(header ‖ nonce)`. Blake3 is superb as a hash but is
//! trivially pipelined in silicon and needs no memory at all — precisely the
//! "SHA-256 ASIC day-one" outcome `docs/ATTRIBUTES.md` rules out. See audit
//! finding N-02 and `docs/DECISIONS.md` row 14.
//!
//! # Cost, honestly
//!
//! Memory-hard proofs are symmetric: verifying a block costs the same as one
//! mining attempt (≈ 10 ms at mainnet parameters on an M3 core). Chain
//! validation is therefore far more expensive than on a Blake3 chain, which is
//! the price of ASIC resistance. Initial sync of 100k blocks is roughly 17
//! minutes of pure hashing.
//!
//! # Difficulty
//!
//! A hash `H` (big-endian 256-bit integer) satisfies difficulty `D` when
//! `H · D < 2^256`, the exact equivalent of `H ≤ ⌊(2^256 − 1) / D⌋` without
//! performing a 256-bit division. Work contributed by a block is `D`, so
//! cumulative chain work is an exact `u128` sum.

use crate::hash_multi;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use nightfall_types::{Hash256, PowParams};
use std::sync::atomic::{AtomicU64, Ordering};

pub const NIGHTHASH_DOMAIN: &[u8] = b"nightfall:nighthash:v2";
pub const NIGHTHASH_SALT_DOMAIN: &[u8] = b"nightfall:nighthash:salt:v2";

/// Salt length. Argon2 requires at least 8 bytes.
const SALT_LEN: usize = 16;

fn salt_for(header_preimage: &[u8]) -> [u8; SALT_LEN] {
    let h = hash_multi(NIGHTHASH_SALT_DOMAIN, &[header_preimage]);
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&h.0[..SALT_LEN]);
    salt
}

fn argon(params: PowParams) -> Argon2<'static> {
    let p = Params::new(params.memory_kib, params.iterations, params.lanes, Some(32))
        .expect("pow parameters are compile-time constants and always valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
}

/// Number of Argon2 blocks a parameter set needs.
fn block_count(params: PowParams) -> usize {
    Params::new(params.memory_kib, params.iterations, params.lanes, Some(32))
        .expect("valid pow parameters")
        .block_count()
}

/// A reusable hashing workspace.
///
/// Allocating tens of megabytes per attempt would dominate the hash loop, so a
/// miner creates one of these and reuses it for every nonce.
pub struct PowHasher {
    argon: Argon2<'static>,
    memory: Vec<Block>,
    params: PowParams,
}

impl PowHasher {
    pub fn new(params: PowParams) -> Self {
        Self {
            argon: argon(params),
            memory: vec![Block::default(); block_count(params)],
            params,
        }
    }

    pub fn params(&self) -> PowParams {
        self.params
    }

    pub fn hash(&mut self, header_preimage: &[u8], nonce: u64) -> Hash256 {
        let salt = salt_for(header_preimage);
        let mut pwd = Vec::with_capacity(header_preimage.len() + 8);
        pwd.extend_from_slice(header_preimage);
        pwd.extend_from_slice(&nonce.to_le_bytes());

        let mut out = [0u8; 32];
        self.argon
            .hash_password_into_with_memory(&pwd, &salt, &mut out, &mut self.memory)
            .expect("argon2 parameters are valid and the buffer is correctly sized");
        Hash256(out)
    }
}

/// One-shot PoW hash. Allocates its workspace; use [`PowHasher`] in a loop.
pub fn nighthash(header_preimage: &[u8], nonce: u64, params: PowParams) -> Hash256 {
    PowHasher::new(params).hash(header_preimage, nonce)
}

/// Does `hash` satisfy `difficulty`?
///
/// Computes the 320-bit product `hash · difficulty` limb by limb and checks
/// that nothing spills past bit 255.
pub fn meets_difficulty(hash: Hash256, difficulty: u64) -> bool {
    if difficulty == 0 {
        return true;
    }
    // Big-endian bytes -> little-endian 64-bit limbs.
    let mut limbs = [0u64; 4];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let hi = 32 - i * 8;
        let mut b = [0u8; 8];
        b.copy_from_slice(&hash.0[hi - 8..hi]);
        *limb = u64::from_be_bytes(b);
    }

    let mut carry: u128 = 0;
    for limb in limbs {
        let product = (limb as u128) * (difficulty as u128) + carry;
        carry = product >> 64;
    }
    carry == 0
}

/// Work contributed by a block at the given difficulty.
pub fn block_work(difficulty: u64) -> u128 {
    difficulty.max(1) as u128
}

/// How many hashes between stop-flag checks.
///
/// Much smaller than the Blake3-era value: a single Argon2id hash takes
/// milliseconds, so checking every 8 attempts still costs nothing and keeps
/// the miner responsive to a new tip within ~100 ms.
const CHECK_INTERVAL: u32 = 8;

/// Mine until `difficulty` is satisfied or `should_stop` asks us to give up.
///
/// The cancellation callback is what allows the miner to run **outside** the
/// node's state lock and abandon a stale template the moment a peer's block
/// arrives (audit finding N-03).
pub fn mine_interruptible(
    header_preimage: &[u8],
    difficulty: u64,
    start_nonce: u64,
    params: PowParams,
    should_stop: &dyn Fn() -> bool,
) -> Option<(u64, Hash256)> {
    mine_counted(
        header_preimage,
        difficulty,
        start_nonce,
        params,
        should_stop,
        None,
    )
}

/// As [`mine_interruptible`], but reports progress into `hash_counter` so the
/// UI can display a real hashrate.
pub fn mine_counted(
    header_preimage: &[u8],
    difficulty: u64,
    start_nonce: u64,
    params: PowParams,
    should_stop: &dyn Fn() -> bool,
    hash_counter: Option<&AtomicU64>,
) -> Option<(u64, Hash256)> {
    let mut hasher = PowHasher::new(params);
    let mut nonce = start_nonce;
    let mut since_check = 0u32;

    loop {
        let h = hasher.hash(header_preimage, nonce);
        if let Some(c) = hash_counter {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if meets_difficulty(h, difficulty) {
            return Some((nonce, h));
        }
        nonce = nonce.wrapping_add(1);

        since_check += 1;
        if since_check >= CHECK_INTERVAL {
            since_check = 0;
            if should_stop() {
                return None;
            }
        }
    }
}

/// Blocking mine with no cancellation. Tests and one-shot CLI use only.
pub fn mine(
    header_preimage: &[u8],
    difficulty: u64,
    start_nonce: u64,
    params: PowParams,
) -> (u64, Hash256) {
    mine_interruptible(header_preimage, difficulty, start_nonce, params, &|| false)
        .expect("uninterruptible mine cannot return None")
}

/// Default worker count: leave one core for the node and the interface.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(1)
}

/// Peak memory a parallel miner will hold, in bytes.
///
/// Each worker owns its own Argon2 workspace, so memory scales linearly with
/// thread count. Worth surfacing in a UI before someone starts 16 workers at
/// 32 MiB each.
pub fn mining_memory_bytes(params: PowParams, threads: usize) -> u64 {
    params.memory_bytes() * threads as u64
}

/// Mine across several threads.
///
/// Each worker takes a disjoint nonce stride, so no two workers ever test the
/// same nonce. The first to succeed sets the shared flag and the rest abandon
/// their current attempt at the next check.
///
/// This matters far more under Nighthash-v2 than it did under Blake3: a single
/// core manages roughly 90 hashes per second at mainnet parameters, so a
/// single-threaded miner would leave most of a modern CPU idle.
pub fn mine_parallel(
    header_preimage: &[u8],
    difficulty: u64,
    start_nonce: u64,
    params: PowParams,
    threads: usize,
    should_stop: &(dyn Fn() -> bool + Sync),
    hash_counter: Option<&AtomicU64>,
) -> Option<(u64, Hash256)> {
    let threads = threads.max(1);
    if threads == 1 {
        return mine_counted(
            header_preimage,
            difficulty,
            start_nonce,
            params,
            should_stop,
            hash_counter,
        );
    }

    let found = std::sync::atomic::AtomicBool::new(false);
    let result = std::sync::Mutex::new(None::<(u64, Hash256)>);

    std::thread::scope(|scope| {
        for worker in 0..threads {
            let found = &found;
            let result = &result;
            scope.spawn(move || {
                let mut hasher = PowHasher::new(params);
                // Disjoint stride so workers never collide on a nonce.
                let mut nonce = start_nonce.wrapping_add(worker as u64);
                let mut since_check = 0u32;

                loop {
                    let h = hasher.hash(header_preimage, nonce);
                    if let Some(c) = hash_counter {
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                    if meets_difficulty(h, difficulty) {
                        // Only the first winner records a result.
                        if !found.swap(true, Ordering::SeqCst) {
                            if let Ok(mut slot) = result.lock() {
                                *slot = Some((nonce, h));
                            }
                        }
                        return;
                    }

                    nonce = nonce.wrapping_add(threads as u64);
                    since_check += 1;
                    if since_check >= CHECK_INTERVAL {
                        since_check = 0;
                        if found.load(Ordering::SeqCst) || should_stop() {
                            return;
                        }
                    }
                }
            });
        }
    });

    result.lock().ok().and_then(|r| *r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_types::NetworkId;

    fn cheap() -> PowParams {
        // Smallest legal Argon2 configuration, for fast unit tests.
        PowParams {
            memory_kib: 8,
            iterations: 1,
            lanes: 1,
        }
    }

    #[test]
    fn difficulty_one_accepts_everything() {
        assert!(meets_difficulty(Hash256([0xFF; 32]), 1));
    }

    #[test]
    fn max_hash_fails_any_real_difficulty() {
        assert!(!meets_difficulty(Hash256([0xFF; 32]), 2));
    }

    #[test]
    fn zero_hash_always_passes() {
        assert!(meets_difficulty(Hash256::ZERO, u64::MAX));
    }

    #[test]
    fn boundary_is_exact() {
        // hash = 2^255 exactly. Times 2 == 2^256, which must NOT fit.
        let mut h = [0u8; 32];
        h[0] = 0x80;
        assert!(meets_difficulty(Hash256(h), 1));
        assert!(!meets_difficulty(Hash256(h), 2));

        // hash = 2^255 − 1: times 2 is 2^256 − 2, which fits.
        let mut h2 = [0xFFu8; 32];
        h2[0] = 0x7F;
        assert!(meets_difficulty(Hash256(h2), 2));
        assert!(!meets_difficulty(Hash256(h2), 3));
    }

    #[test]
    fn hash_is_deterministic() {
        let a = nighthash(b"header", 42, cheap());
        let b = nighthash(b"header", 42, cheap());
        assert_eq!(a, b);
    }

    #[test]
    fn nonce_changes_the_hash() {
        assert_ne!(
            nighthash(b"header", 0, cheap()),
            nighthash(b"header", 1, cheap())
        );
    }

    #[test]
    fn header_changes_the_hash() {
        assert_ne!(
            nighthash(b"header-a", 7, cheap()),
            nighthash(b"header-b", 7, cheap())
        );
    }

    #[test]
    fn parameters_are_consensus_critical() {
        // A node running different memory parameters computes a different hash
        // and would fork itself off the network. This test documents that.
        let p1 = cheap();
        let p2 = PowParams {
            memory_kib: 16,
            ..cheap()
        };
        assert_ne!(nighthash(b"h", 1, p1), nighthash(b"h", 1, p2));
    }

    #[test]
    fn reused_hasher_matches_one_shot() {
        let mut hasher = PowHasher::new(cheap());
        assert_eq!(hasher.hash(b"x", 5), nighthash(b"x", 5, cheap()));
        // Reuse must not corrupt the workspace between calls.
        assert_eq!(hasher.hash(b"y", 6), nighthash(b"y", 6, cheap()));
        assert_eq!(hasher.hash(b"x", 5), nighthash(b"x", 5, cheap()));
    }

    #[test]
    fn mining_finds_a_valid_nonce() {
        let (nonce, h) = mine(b"test-header", 40, 0, cheap());
        assert!(meets_difficulty(h, 40));
        assert_eq!(nighthash(b"test-header", nonce, cheap()), h);
    }

    #[test]
    fn mining_is_interruptible() {
        let calls = std::cell::Cell::new(0u32);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert!(mine_interruptible(b"h", u64::MAX, 0, cheap(), &stop).is_none());
    }

    #[test]
    fn network_parameters_are_memory_hard() {
        // Mainnet must demand real memory; devnet is allowed to be cheap.
        let main = NetworkId::Mainnet.pow_params();
        assert!(
            main.memory_bytes() >= 16 * 1024 * 1024,
            "mainnet PoW must require at least 16 MiB per hash"
        );
        assert!(NetworkId::Devnet.pow_params().memory_kib < main.memory_kib);
    }
}
