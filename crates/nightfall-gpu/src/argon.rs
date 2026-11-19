//! The CPU half of Argon2id (v1.3) for Nighthash-v2, one lane, one pass.
//!
//! The GPU only does the memory-hard middle: filling blocks 2..m'-1. The cheap
//! ends — H0, the first two blocks and the final tag — stay here, written
//! straight from RFC 9106 so they can be checked line by line, and tested
//! against the `argon2` crate the consensus code uses.

use blake2::digest::{Update, VariableOutput};
use blake2::{Blake2b512, Blake2bVar, Digest};
use nightfall_crypto::hash_multi;
use nightfall_crypto::NIGHTHASH_SALT_DOMAIN;
use nightfall_types::{Hash256, PowParams};

pub const BLOCK_BYTES: usize = 1024;
pub const BLOCK_WORDS: usize = 128;

const VERSION: u32 = 0x13;
const TYPE_ID: u32 = 2;
const TAG_LEN: u32 = 32;

/// Number of 1 KiB blocks in the single lane, m' = 4·⌊m/4⌋ for p = 1.
pub fn lane_blocks(params: PowParams) -> usize {
    (params.memory_kib as usize / 4) * 4
}

/// The kernel only implements what Nighthash-v2 uses.
pub fn supported(params: PowParams) -> bool {
    params.lanes == 1 && params.iterations == 1 && lane_blocks(params) >= 8
}

/// Same derivation as `nightfall_crypto::pow`: 16 bytes of the domain hash.
pub fn salt_for(header_preimage: &[u8]) -> [u8; 16] {
    let h = hash_multi(NIGHTHASH_SALT_DOMAIN, &[header_preimage]);
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&h.0[..16]);
    salt
}

/// H0 (RFC 9106 §3.2), for password = header_preimage ‖ nonce_le.
pub fn h0(params: PowParams, header_preimage: &[u8], nonce: u64, salt: &[u8; 16]) -> [u8; 64] {
    let pwd_len = (header_preimage.len() + 8) as u32;
    let mut h = Blake2b512::new();
    Digest::update(&mut h, 1u32.to_le_bytes()); // lanes
    Digest::update(&mut h, TAG_LEN.to_le_bytes());
    Digest::update(&mut h, params.memory_kib.to_le_bytes());
    Digest::update(&mut h, params.iterations.to_le_bytes());
    Digest::update(&mut h, VERSION.to_le_bytes());
    Digest::update(&mut h, TYPE_ID.to_le_bytes());
    Digest::update(&mut h, pwd_len.to_le_bytes());
    Digest::update(&mut h, header_preimage);
    Digest::update(&mut h, nonce.to_le_bytes());
    Digest::update(&mut h, (salt.len() as u32).to_le_bytes());
    Digest::update(&mut h, salt);
    Digest::update(&mut h, 0u32.to_le_bytes()); // secret
    Digest::update(&mut h, 0u32.to_le_bytes()); // associated data
    h.finalize().into()
}

/// H' with a 1024-byte output (RFC 9106 §3.3).
fn h_prime_1024(parts: &[&[u8]], out: &mut [u8; BLOCK_BYTES]) {
    let mut h = Blake2b512::new();
    Digest::update(&mut h, (BLOCK_BYTES as u32).to_le_bytes());
    for p in parts {
        Digest::update(&mut h, p);
    }
    let mut v: [u8; 64] = h.finalize().into();
    out[..32].copy_from_slice(&v[..32]);
    // r = ceil(1024/32) - 2 = 30 blocks of 32 bytes, then one full 64.
    for i in 1..30 {
        v = Blake2b512::digest(v).into();
        out[i * 32..i * 32 + 32].copy_from_slice(&v[..32]);
    }
    v = Blake2b512::digest(v).into();
    out[960..].copy_from_slice(&v);
}

/// Blocks 0 and 1 of the lane, as little-endian words.
pub fn first_blocks(h0: &[u8; 64], out: &mut [u64]) {
    debug_assert!(out.len() >= 2 * BLOCK_WORDS);
    let mut bytes = [0u8; BLOCK_BYTES];
    for j in 0..2u32 {
        h_prime_1024(&[h0, &j.to_le_bytes(), &0u32.to_le_bytes()], &mut bytes);
        for (w, chunk) in out[j as usize * BLOCK_WORDS..(j as usize + 1) * BLOCK_WORDS]
            .iter_mut()
            .zip(bytes.as_chunks::<8>().0)
        {
            *w = u64::from_le_bytes(*chunk);
        }
    }
}

/// The tag from the last block: H'^32, which for T ≤ 64 is a single BLAKE2b
/// with a 32-byte output length — not a truncated BLAKE2b-512.
pub fn final_tag(last_block: &[u64]) -> Hash256 {
    let mut h = Blake2bVar::new(TAG_LEN as usize).unwrap();
    h.update(&TAG_LEN.to_le_bytes());
    for w in &last_block[..BLOCK_WORDS] {
        h.update(&w.to_le_bytes());
    }
    let mut out = [0u8; 32];
    h.finalize_variable(&mut out).unwrap();
    Hash256(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_params_are_supported() {
        let p = nightfall_types::NetworkId::Mainnet.pow_params();
        assert!(supported(p));
        assert_eq!(lane_blocks(p), 32_768);
    }
}
