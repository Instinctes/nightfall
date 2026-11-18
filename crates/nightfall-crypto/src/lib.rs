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

mod binding;
mod commit;
mod kernel;
mod keys;
mod mnemonic;
mod pow;
mod rangeproof;
mod schnorr;
mod stealth;

pub use binding::*;
pub use commit::*;
pub use kernel::*;
pub use keys::*;
pub use mnemonic::{MnemonicError, MNEMONIC_WORDS};
pub use pow::*;
pub use rangeproof::{RangeError, RangeProofBytes, RANGE_BITS};
pub use schnorr::{SchnorrSig, SCHNORR_DOMAIN};
pub use stealth::*;

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
    pub const TX_BOUND: &[u8] = b"nightfall:tx:bound:v1";
    pub const TXBODY: &[u8] = b"nightfall:txbody:v2";
    pub const TXBODY_BOUND: &[u8] = b"nightfall:txbody:bound:v1";
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
    // Blake3 is defined over the concatenated input, so feeding it the same
    // bytes in one call gives the same hash as feeding them piece by piece.
    // For the short inputs that dominate (Merkle nodes and leaves, ~120 bytes)
    // one call is several times cheaper than a dozen tiny `update`s, and the
    // UTXO root alone makes ~430,000 of these calls per validated block.
    const INLINE: usize = 256;
    let total = 16 + domain.len() + parts.iter().map(|p| 8 + p.len()).sum::<usize>();
    if total > INLINE {
        return hash_multi_streaming(domain, parts);
    }
    let mut buf = [0u8; INLINE];
    let mut at = 0;
    let mut put = |bytes: &[u8]| {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
        at += bytes.len();
    };
    put(&(domain.len() as u64).to_le_bytes());
    put(domain);
    put(&(parts.len() as u64).to_le_bytes());
    for p in parts {
        put(&(p.len() as u64).to_le_bytes());
        put(p);
    }
    Hash256(*blake3::hash(&buf[..at]).as_bytes())
}

/// The definition of [`hash_multi`]: every length prefix and part streamed
/// into one Blake3 hasher. Used for long inputs, and pinned against the
/// single-call path by tests.
fn hash_multi_streaming(domain: &[u8], parts: &[&[u8]]) -> Hash256 {
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

#[cfg(test)]
mod hash_multi_tests {
    use super::*;

    #[test]
    fn single_call_equals_streaming_for_every_length_around_the_inline_limit() {
        let data: Vec<u8> = (0..600u32).map(|i| (i * 7 + 3) as u8).collect();
        for domain_len in [0usize, 1, 19, 24, 64] {
            let domain = &data[..domain_len];
            for n_parts in 0..5usize {
                for len in 0..=260usize {
                    let parts: Vec<&[u8]> = (0..n_parts)
                        .map(|k| &data[k * 3..k * 3 + len.min(600 - k * 3)])
                        .collect();
                    assert_eq!(
                        hash_multi(domain, &parts),
                        hash_multi_streaming(domain, &parts),
                        "domain {domain_len} parts {n_parts} len {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn known_vectors_are_unchanged() {
        // Values computed with the streaming definition before this change.
        assert_eq!(
            hash_multi(domain::MERKLE, &[&[1u8; 32], &[2u8; 32]]),
            hash_multi_streaming(domain::MERKLE, &[&[1u8; 32], &[2u8; 32]])
        );
        assert_eq!(hash_multi(b"", &[]), hash_multi_streaming(b"", &[]));
    }
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
