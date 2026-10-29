//! Transaction binding (protocol 1.1.0).
//!
//! Each input, output and kernel carries a key whose discrete log only its
//! creator knows, and one block-wide equation requires those keys to balance.
//! This is the construction Litecoin MWEB uses for its stealth offsets.
//!
//! * Input: a fresh input key `Ki = ki·G` and a signature under
//!   `Ki + h·Ko`, `h = Hs(Ki ‖ Ko)`, over `H(commit ‖ Ki)`. The coefficient
//!   `h` prevents related-key choices of `Ki`.
//! * Output: the ephemeral key `Ke = r·G`, already signed by `r`.
//! * Kernel: a stealth excess `X = x·G`, signed by `x` over the kernel
//!   message and `X`.
//! * Block: `Σ Ki − Σ Ke = Σ X + s·G`, with `s` the summed stealth offsets.

use crate::commit::{generator_g, Commitment};
use crate::hash_multi;
use crate::schnorr::{self, SchnorrSig};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::rngs::OsRng;
use rand::RngCore;

pub const INPUT_KEY_COEFF_DOMAIN: &[u8] = b"nightfall:input:key:coeff:v1";
pub const INPUT_BIND_DOMAIN: &[u8] = b"nightfall:input:bind:v1";
pub const KERNEL_STEALTH_DOMAIN: &[u8] = b"nightfall:kernel:stealth:v1";

pub const INPUT_SECRET_DOMAIN: &[u8] = b"nightfall:input:secret:v1";

/// A random scalar for **public** blinding only: the offsets.
///
/// Never use this for a secret. If the OS RNG is unavailable (it can be,
/// briefly, in Safari's wasm) it returns zero rather than panicking, which
/// costs privacy for one transaction and nothing else.
pub fn random_scalar() -> Scalar {
    let mut wide = [0u8; 64];
    if OsRng.try_fill_bytes(&mut wide).is_err() {
        return Scalar::ZERO;
    }
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// The fresh input secret `ki` for spending `commit`.
///
/// Derived from the one-time spend secret, the coin and fresh randomness, so
/// it remains secret and unique per coin even if the RNG is unavailable.
pub fn derive_input_secret(ko: &Scalar, commit: &Commitment) -> Scalar {
    let mut entropy = [0u8; 32];
    let _ = OsRng.try_fill_bytes(&mut entropy);
    let a = hash_multi(
        INPUT_SECRET_DOMAIN,
        &[ko.as_bytes(), &commit.0, &entropy, b"0"],
    );
    let b = hash_multi(
        INPUT_SECRET_DOMAIN,
        &[ko.as_bytes(), &commit.0, &entropy, b"1"],
    );
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a.0);
    wide[32..].copy_from_slice(&b.0);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// `h = Hs(Ki ‖ Ko)`, by wide reduction.
pub fn input_key_coefficient(input_key: &[u8; 32], output_pk: &[u8; 32]) -> Scalar {
    let a = hash_multi(INPUT_KEY_COEFF_DOMAIN, &[input_key, output_pk, b"0"]);
    let b = hash_multi(INPUT_KEY_COEFF_DOMAIN, &[input_key, output_pk, b"1"]);
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a.0);
    wide[32..].copy_from_slice(&b.0);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// What the input key signature covers: the coin and the input key.
pub fn input_bind_message(commit: &Commitment, input_key: &[u8; 32]) -> Vec<u8> {
    hash_multi(INPUT_BIND_DOMAIN, &[&commit.0, input_key])
        .0
        .to_vec()
}

/// Sign an input key. `ki` is the fresh input secret, `ko` the one-time
/// spend secret of the output being spent.
pub fn sign_input_key(
    ki: &Scalar,
    ko: &Scalar,
    commit: &Commitment,
    output_pk: &[u8; 32],
) -> ([u8; 32], SchnorrSig) {
    let input_key = (generator_g() * ki).compress().to_bytes();
    let h = input_key_coefficient(&input_key, output_pk);
    let combined = ki + h * ko;
    let sig = schnorr::sign(
        &combined,
        &generator_g(),
        &input_bind_message(commit, &input_key),
    );
    (input_key, sig)
}

/// Verify an input key signature against the `Ko` recorded in the UTXO set.
pub fn verify_input_key(
    commit: &Commitment,
    input_key: &[u8; 32],
    output_pk: &[u8; 32],
    sig: &SchnorrSig,
) -> bool {
    let Some(ki) = CompressedRistretto(*input_key).decompress() else {
        return false;
    };
    let Some(ko) = CompressedRistretto(*output_pk).decompress() else {
        return false;
    };
    let h = input_key_coefficient(input_key, output_pk);
    let combined: RistrettoPoint = ki + ko * h;
    schnorr::verify(
        &combined,
        &generator_g(),
        &input_bind_message(commit, input_key),
        sig,
    )
}

/// What a kernel's stealth signature covers.
pub fn kernel_stealth_message(kernel_message: &[u8], stealth_excess: &[u8; 32]) -> Vec<u8> {
    hash_multi(KERNEL_STEALTH_DOMAIN, &[kernel_message, stealth_excess])
        .0
        .to_vec()
}

/// Decode a public scalar (an offset). Non-canonical encodings are refused so
/// a block body has exactly one byte representation.
pub fn scalar_from_bytes(bytes: &[u8; 32]) -> Option<Scalar> {
    Option::<Scalar>::from(Scalar::from_canonical_bytes(*bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_key_signature_needs_the_spend_secret() {
        let ko = random_scalar();
        let output_pk = (generator_g() * ko).compress().to_bytes();
        let commit = Commitment::new(5, &random_scalar());
        let ki = random_scalar();
        let (input_key, sig) = sign_input_key(&ki, &ko, &commit, &output_pk);
        assert!(verify_input_key(&commit, &input_key, &output_pk, &sig));

        // Same input key, wrong spend secret.
        let (_, forged) = sign_input_key(&ki, &random_scalar(), &commit, &output_pk);
        assert!(!verify_input_key(&commit, &input_key, &output_pk, &forged));

        // A valid signature does not transfer to another coin.
        let other = Commitment::new(6, &random_scalar());
        assert!(!verify_input_key(&other, &input_key, &output_pk, &sig));
    }

    #[test]
    fn offsets_must_be_canonical() {
        assert!(scalar_from_bytes(&[0xff; 32]).is_none());
        assert!(scalar_from_bytes(&random_scalar().to_bytes()).is_some());
    }
}
