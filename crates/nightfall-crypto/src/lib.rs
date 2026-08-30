//! Nightfall cryptography.
//!
//! Protocol v5 ("Nightproof-β") replaces the v4 `Nightproof-α` construction,
//! which was unsound: its balance proof was a tautology, it had no range
//! proofs, and it published the recipient of every output in cleartext. The
//! full analysis is in `docs/AUDIT-2026-08-12.md`.
//!
//! What is here now:
//!
//! | Module | Role |
//! |--------|------|
//! | [`commit`] | Pedersen commitments on Bulletproof-compatible generators |
//! | [`rangeproof`] | Bulletproofs — every output value is provably in `[0, 2^64)` |
//! | [`schnorr`] | Schnorr signatures, generator-parameterised |
//! | [`kernel`] | Excess signature over `H` — the actual no-inflation proof |
//! | [`stealth`] | One-sided unlinkable outputs (MWEB-style) |
//! | [`keys`] | Scan / spend key split, addresses, view keys |
//! | [`pow`] | Nighthash proof of work |

mod commit;
mod kernel;
mod keys;
mod mnemonic;
mod pow;
mod rangeproof;
mod schnorr;
mod stealth;

pub use commit::*;
pub use kernel::*;
pub use keys::*;
pub use mnemonic::{MnemonicError, MNEMONIC_WORDS};
pub use pow::*;
pub use rangeproof::{RangeError, RangeProofBytes, RANGE_BITS};
pub use schnorr::{SchnorrSig, SCHNORR_DOMAIN};
pub use stealth::*;

/// Cross-curve DLEQ (Ristretto ↔ secp256k1). Experimental — swap only.
pub mod dleq;
/// Shared outputs for atomic swaps. Experimental — see the module docs.
pub mod swap;

pub mod rangeproofs {
    pub use crate::rangeproof::{prove, verify};
}

pub mod sig {
    pub use crate::schnorr::{sign, verify};
}

use nightfall_types::Hash256;

pub mod domain {
    pub const GENESIS: &[u8] = b"nightfall:genesis:v2";
    pub const BLOCK: &[u8] = b"nightfall:block:v2";
    pub const TX: &[u8] = b"nightfall:tx:v2";
    pub const TXBODY: &[u8] = b"nightfall:txbody:v2";
    pub const MERKLE: &[u8] = b"nightfall:merkle:v2";
    pub const MERKLE_LEAF: &[u8] = b"nightfall:merkle:leaf:v2";
    pub const KERNEL: &[u8] = b"nightfall:kernel:v2";
    pub const UTXO_ROOT: &[u8] = b"nightfall:utxoroot:v2";
    pub const INPUT: &[u8] = b"nightfall:input:v2";
}

/// Length-prefixed, domain-separated Blake3. The length prefixes make the
/// encoding injective, so no two distinct inputs can collide by concatenation.
pub fn hash_domain(domain: &[u8], data: &[u8]) -> Hash256 {
    hash_multi(domain, &[data])
}

pub fn hash_multi(domain: &[u8], parts: &[&[u8]]) -> Hash256 {
    let mut h = blake3::Hasher::new();
    h.update(&(domain.len() as u64).to_le_bytes());
    h.update(domain);
    h.update(&(parts.len() as u64).to_le_bytes());
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p);
    }
    Hash256(*h.finalize().as_bytes())
}

pub fn genesis_commitment(canonical_bytes: &[u8]) -> Hash256 {
    hash_domain(domain::GENESIS, canonical_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_separation() {
        assert_ne!(
            hash_domain(domain::TX, b"x"),
            hash_domain(domain::BLOCK, b"x")
        );
    }

    #[test]
    fn length_prefixing_prevents_concat_collisions() {
        // Without length prefixes these two would hash identically.
        assert_ne!(
            hash_multi(domain::TX, &[b"ab", b"c"]),
            hash_multi(domain::TX, &[b"a", b"bc"])
        );
    }
}
