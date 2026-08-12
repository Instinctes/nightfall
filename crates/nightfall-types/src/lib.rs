//! Core types for **NIGHTFALLCOIN** (`NIGHT`).

use serde::{Deserialize, Serialize};
use std::fmt;

pub const COIN_NAME: &str = "NIGHTFALLCOIN";
pub const TICKER: &str = "NIGHT";
pub const PROTOCOL_NAME: &str = "nightfall";

/// Protocol v5 — "Nightproof-β".
///
/// v4 is **consensus-broken and deprecated**: its balance proof was a
/// tautology, allowing anyone to mint arbitrary NIGHT. v5 nodes will not peer
/// with v4 nodes and the genesis commitment differs, so the chains cannot mix.
/// See `docs/AUDIT-2026-08-12.md`.
pub const PROTOCOL_VERSION: u32 = 5;

/// Wire protocol version for P2P/RPC. Bumped in lockstep with the v5 fork.
pub const WIRE_VERSION: u32 = 2;

/// 1 NIGHT = 10^8 darks.
pub const DARKS_PER_NIGHT: u64 = 100_000_000;
pub const MAX_SUPPLY_NIGHT: u64 = 90_000_000;
pub const MAX_SUPPLY_DARKS: u64 = MAX_SUPPLY_NIGHT * DARKS_PER_NIGHT;
pub const INITIAL_BLOCK_REWARD_NIGHT: u64 = 20;
pub const HALVING_INTERVAL_BLOCKS: u64 = 2_250_000;
pub const TARGET_BLOCK_TIME_SECS: u64 = 15;
pub const NIGHT_ASSET_ID: u32 = 0;

/// Difficulty is retargeted on **every** block using a linearly-weighted moving
/// average over this window (Zawy LWMA-1).
///
/// v4 retargeted once every 240 blocks by ±1 "leading zero bit", i.e. the
/// difficulty could only double or halve per hour and was hard-capped at 28
/// bits ≈ 2.7·10⁸ hashes. A laptop could out-mine the entire network and
/// rewrite history. See audit finding N-02.
pub const DIFFICULTY_WINDOW: u64 = 90;

/// Difficulty may never drop below this, so a chain can never be rewritten for
/// free after a hashrate collapse.
///
/// Calibrated for Nighthash-v2: one CPU core produces roughly 90 hashes per
/// second at mainnet parameters, so this floor still costs ~20 seconds of
/// single-core work per block even if all other miners vanish.
pub const MIN_DIFFICULTY: u64 = 2_000;

/// Proof-of-work parameters for Nighthash-v2 (Argon2id).
///
/// Memory-hardness is the point: an attacker must supply `memory_kib` of fast
/// RAM per parallel hashing unit, which is what makes purpose-built hardware
/// uneconomical relative to ordinary CPUs. Blake3 — used by v4 — offered no
/// such barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowParams {
    /// Memory per hash, in KiB.
    pub memory_kib: u32,
    /// Passes over memory.
    pub iterations: u32,
    /// Parallel lanes.
    pub lanes: u32,
}

impl PowParams {
    pub fn memory_bytes(&self) -> u64 {
        self.memory_kib as u64 * 1024
    }
}

/// Max timestamp drift into the future (seconds).
///
/// v4 allowed ±2 h on a 15-second chain, which is 480 block intervals of
/// slack — ample room to manipulate the retarget (time-warp).
pub const MAX_FUTURE_DRIFT_SECS: u64 = 120;

/// Number of blocks used for the median-time-past rule. A block's timestamp
/// must be strictly greater than the median of the previous `MTP_WINDOW`.
pub const MTP_WINDOW: usize = 11;

/// Solve-time clamps for LWMA, expressed in multiples of the target.
pub const LWMA_MIN_SOLVETIME_FACTOR: i64 = -5;
pub const LWMA_MAX_SOLVETIME_FACTOR: i64 = 6;

/// Hard ceiling on serialized block size, enforced before deserialization.
pub const MAX_BLOCK_BYTES: usize = 2 * 1024 * 1024;
/// Hard ceiling on any single P2P message.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkId {
    Mainnet,
    Testnet,
    Devnet,
}

impl NetworkId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Devnet => "devnet",
        }
    }

    /// Starting difficulty. Work per block equals the difficulty value, and a
    /// hash is valid when `hash · difficulty < 2^256`.
    pub fn initial_difficulty(self) -> u64 {
        match self {
            // Measured with `cargo run -p nightfall-crypto --example powbench`
            // on an Apple M3 Pro (10 mining threads):
            //   devnet  22,268 H/s · testnet 2,351 H/s · mainnet 489 H/s
            // Difficulty ≈ hashrate × 15 s target.
            Self::Devnet => 60, // deliberately trivial; tests must be fast
            Self::Testnet => 30_000,
            Self::Mainnet => 5_000, // ≈ 10 s here, ≈ 25 s on a modest laptop
        }
    }

    /// Floor below which difficulty is never allowed to fall.
    pub fn min_difficulty(self) -> u64 {
        match self {
            Self::Devnet => 1,
            Self::Testnet => 1_000,
            Self::Mainnet => MIN_DIFFICULTY,
        }
    }

    /// Domain separation context bound into range proofs, so a proof made for
    /// one network can never be replayed on another.
    pub fn proof_context(self) -> &'static [u8] {
        match self {
            Self::Mainnet => b"nightfall:mainnet:v5",
            Self::Testnet => b"nightfall:testnet:v5",
            Self::Devnet => b"nightfall:devnet:v5",
        }
    }

    /// Addresses every node dials on startup, in addition to any peer the user
    /// adds by hand or via `SEED_NODE`.
    ///
    /// Seed nodes exist to solve exactly one problem: a fresh install has no
    /// way to find the network, and a miner with no peers silently builds a
    /// private fork that gets discarded the moment it connects to anyone.
    ///
    /// **To publish a seed node**, put its `host:port` here and ship a new
    /// build. A DNS name is preferable to a bare IP — it survives the address
    /// changing, which a home connection eventually will:
    ///
    /// ```ignore
    /// Self::Mainnet => &["seed.nightfall.cash:17891"],
    /// ```
    ///
    /// Hostnames are resolved at dial time, so dynamic DNS works.
    pub fn seed_nodes(self) -> &'static [&'static str] {
        match self {
            // No public seed node yet. Until one is listed here, every node has
            // to be pointed at a peer manually — see docs/MAINNET.md.
            Self::Mainnet => &[],
            Self::Testnet => &[],
            Self::Devnet => &[],
        }
    }

    /// Proof-of-work parameters. Consensus-critical: a node using different
    /// values computes different hashes and forks itself off the network.
    pub fn pow_params(self) -> PowParams {
        match self {
            // 32 MiB per hash. A dedicated ASIC would need that much fast RAM
            // per parallel core, which is where the economics stop working.
            Self::Mainnet => PowParams {
                memory_kib: 32 * 1024,
                iterations: 1,
                lanes: 1,
            },
            Self::Testnet => PowParams {
                memory_kib: 8 * 1024,
                iterations: 1,
                lanes: 1,
            },
            // Deliberately cheap so the test suite and local development do not
            // spend their lives hashing. Devnet has no economic security.
            Self::Devnet => PowParams {
                memory_kib: 1024,
                iterations: 1,
                lanes: 1,
            },
        }
    }

    pub fn default_p2p_port(self) -> u16 {
        match self {
            Self::Mainnet => 17891,
            Self::Testnet => 17892,
            Self::Devnet => 17893,
        }
    }

    pub fn default_rpc_port(self) -> u16 {
        match self {
            Self::Mainnet => 17881,
            Self::Testnet => 17882,
            Self::Devnet => 17883,
        }
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Amount(pub u64);

impl Amount {
    pub const ZERO: Self = Self(0);

    pub fn from_night(whole: u64) -> Option<Self> {
        whole.checked_mul(DARKS_PER_NIGHT).map(Self)
    }

    pub fn from_darks(darks: u64) -> Self {
        Self(darks)
    }

    pub fn darks(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / DARKS_PER_NIGHT;
        let frac = self.0 % DARKS_PER_NIGHT;
        write!(f, "{whole}.{frac:08} {TICKER}")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub const ZERO: Self = Self([0u8; 32]);

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, TypesError> {
        let bytes = hex::decode(s).map_err(|_| TypesError::InvalidHex)?;
        if bytes.len() != 32 {
            return Err(TypesError::InvalidHashLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Leading zero bits (MSB first) — used by Nighthash difficulty.
    pub fn leading_zero_bits(self) -> u32 {
        let mut bits = 0u32;
        for byte in self.0 {
            if byte == 0 {
                bits += 8;
            } else {
                bits += byte.leading_zeros();
                break;
            }
        }
        bits
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Height(pub u64);

impl Height {
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisConfig {
    pub network: NetworkId,
    pub chain_name: String,
    pub coin_name: String,
    pub ticker: String,
    pub allocations: Vec<GenesisAllocation>,
    pub protocol_version: u32,
    pub max_supply_night: u64,
    pub initial_reward_night: u64,
    pub halving_interval: u64,
}

impl GenesisConfig {
    pub fn fair_launch(network: NetworkId) -> Self {
        Self {
            network,
            chain_name: PROTOCOL_NAME.to_string(),
            coin_name: COIN_NAME.to_string(),
            ticker: TICKER.to_string(),
            allocations: Vec::new(),
            protocol_version: PROTOCOL_VERSION,
            max_supply_night: MAX_SUPPLY_NIGHT,
            initial_reward_night: INITIAL_BLOCK_REWARD_NIGHT,
            halving_interval: HALVING_INTERVAL_BLOCKS,
        }
    }

    pub fn assert_fair(&self) -> Result<(), TypesError> {
        if !self.allocations.is_empty() {
            return Err(TypesError::PremineForbidden {
                count: self.allocations.len(),
            });
        }
        if self.coin_name != COIN_NAME || self.ticker != TICKER {
            return Err(TypesError::IdentityMismatch);
        }
        if self.max_supply_night != MAX_SUPPLY_NIGHT {
            return Err(TypesError::SupplyMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenesisAllocation {
    pub label: String,
    pub amount: Amount,
}

#[derive(Debug, thiserror::Error)]
pub enum TypesError {
    #[error("invalid hex")]
    InvalidHex,
    #[error("hash must be 32 bytes")]
    InvalidHashLength,
    #[error("fair launch violated: {count} genesis allocation(s)")]
    PremineForbidden { count: usize },
    #[error("coin identity mismatch")]
    IdentityMismatch,
    #[error("max supply mismatch")]
    SupplyMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometric_cap() {
        assert_eq!(
            2 * INITIAL_BLOCK_REWARD_NIGHT * HALVING_INTERVAL_BLOCKS,
            MAX_SUPPLY_NIGHT
        );
    }

    #[test]
    fn fair_genesis() {
        let g = GenesisConfig::fair_launch(NetworkId::Devnet);
        assert!(g.assert_fair().is_ok());
    }
}
