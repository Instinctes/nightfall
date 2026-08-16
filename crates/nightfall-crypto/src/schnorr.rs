//! Schnorr signatures on Ristretto, parameterised by generator.
//!
//! Two generators are in play across the protocol:
//!
//! * `G` — used for ordinary key signatures (spending an output).
//! * `H` — used for **kernel excess signatures**. Signing under `H` is what
//!   proves the excess point carries no `G` component, i.e. that the
//!   transaction minted nothing.
//!
//! Challenge is `e = H(R ‖ P ‖ msg)` with domain separation, computed by
//! wide reduction so it is uniform over the scalar field.

use crate::hash_multi;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

pub const SCHNORR_DOMAIN: &[u8] = b"nightfall:schnorr:v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchnorrSig {
    /// Compressed nonce point `R`.
    pub r: [u8; 32],
    /// Response scalar `s`.
    pub s: [u8; 32],
}

fn challenge(r: &CompressedRistretto, p: &CompressedRistretto, msg: &[u8]) -> Scalar {
    let a = hash_multi(SCHNORR_DOMAIN, &[r.as_bytes(), p.as_bytes(), msg, b"c0"]);
    let b = hash_multi(SCHNORR_DOMAIN, &[r.as_bytes(), p.as_bytes(), msg, b"c1"]);
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a.0);
    wide[32..].copy_from_slice(&b.0);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// Sign `msg` proving knowledge of `secret` where `P = secret·generator`.
///
/// The nonce is derived deterministically from the secret and the message
/// (RFC6979-style) *and* mixed with fresh randomness. Deterministic-only
/// nonces leak the key if the same nonce is ever reused across two messages;
/// random-only nonces fail catastrophically on a bad RNG. Mixing both means an
/// attacker must break both to recover the key.
pub fn sign(secret: &Scalar, generator: &RistrettoPoint, msg: &[u8]) -> SchnorrSig {
    let public = (generator * secret).compress();

    let mut entropy = [0u8; 32];
    use rand::RngCore;
    // Do not panic if the OS RNG is briefly unavailable (Safari wasm).
    // The nonce still mixes the secret and the message.
    let _ = OsRng.try_fill_bytes(&mut entropy);

    let k = {
        let a = hash_multi(
            b"nightfall:schnorr:nonce",
            &[secret.as_bytes(), msg, &entropy, b"k0"],
        );
        let b = hash_multi(
            b"nightfall:schnorr:nonce",
            &[secret.as_bytes(), msg, &entropy, b"k1"],
        );
        let mut wide = [0u8; 64];
        wide[..32].copy_from_slice(&a.0);
        wide[32..].copy_from_slice(&b.0);
        Scalar::from_bytes_mod_order_wide(&wide)
    };

    let r_point = (generator * k).compress();
    let e = challenge(&r_point, &public, msg);
    let s = k + e * secret;

    SchnorrSig {
        r: r_point.to_bytes(),
        s: s.to_bytes(),
    }
}

/// Verify that `sig` proves knowledge of the discrete log of `public`
/// with respect to `generator`.
pub fn verify(
    public: &RistrettoPoint,
    generator: &RistrettoPoint,
    msg: &[u8],
    sig: &SchnorrSig,
) -> bool {
    let Some(r_point) = CompressedRistretto(sig.r).decompress() else {
        return false;
    };
    // Reject non-canonical scalars — a malleable `s` would make signatures
    // (and therefore txids) non-unique.
    let Some(s) = Option::<Scalar>::from(Scalar::from_canonical_bytes(sig.s)) else {
        return false;
    };

    let p_compressed = public.compress();
    let e = challenge(&CompressedRistretto(sig.r), &p_compressed, msg);

    // s·Gen == R + e·P
    generator * s == r_point + public * e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::{generator_g, generator_h};

    #[test]
    fn sign_verify_roundtrip() {
        let sk = Scalar::random(&mut OsRng);
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"hello");
        assert!(verify(&pk, &generator_g(), b"hello", &sig));
    }

    #[test]
    fn rejects_wrong_message() {
        let sk = Scalar::random(&mut OsRng);
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"hello");
        assert!(!verify(&pk, &generator_g(), b"goodbye", &sig));
    }

    #[test]
    fn rejects_wrong_generator() {
        // A signature valid under G must not verify under H. This is the
        // property the excess signature depends on.
        let sk = Scalar::random(&mut OsRng);
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"m");
        assert!(!verify(&pk, &generator_h(), b"m", &sig));
    }

    #[test]
    fn rejects_tampered_s() {
        let sk = Scalar::random(&mut OsRng);
        let pk = generator_g() * sk;
        let mut sig = sign(&sk, &generator_g(), b"m");
        sig.s[0] ^= 1;
        assert!(!verify(&pk, &generator_g(), b"m", &sig));
    }

    #[test]
    fn nonces_do_not_repeat() {
        let sk = Scalar::random(&mut OsRng);
        let a = sign(&sk, &generator_g(), b"m");
        let b = sign(&sk, &generator_g(), b"m");
        assert_ne!(
            a.r, b.r,
            "nonce reuse across identical messages leaks the key"
        );
    }
}
