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

#[cfg(test)]
mod schnorr_properties {
    use super::*;
    use crate::commit::generator_g;
    use curve25519_dalek::traits::Identity;
    use rand::RngCore;

    fn random_scalar() -> Scalar {
        Scalar::random(&mut OsRng)
    }

    fn random_32() -> [u8; 32] {
        let mut b = [0u8; 32];
        let mut rng = OsRng;
        rng.fill_bytes(&mut b);
        b
    }

    #[test]
    fn every_bit_flip_in_s_invalidates_signature() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"m");
        assert!(verify(&pk, &generator_g(), b"m", &sig));
        for byte_idx in 0..32 {
            for bit in 0..8u8 {
                let mut tampered = sig;
                tampered.s[byte_idx] ^= 1u8 << bit;
                assert!(!verify(&pk, &generator_g(), b"m", &tampered));
            }
        }
    }

    #[test]
    fn every_bit_flip_in_r_invalidates_signature() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"m");
        assert!(verify(&pk, &generator_g(), b"m", &sig));
        for byte_idx in 0..32 {
            for bit in 0..8u8 {
                let mut tampered = sig;
                tampered.r[byte_idx] ^= 1u8 << bit;
                assert!(!verify(&pk, &generator_g(), b"m", &tampered));
            }
        }
    }

    #[test]
    fn non_canonical_s_is_rejected() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let mut sig = sign(&sk, &generator_g(), b"m");
        let l_bytes: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x10,
        ];
        sig.s = l_bytes;
        assert!(!verify(&pk, &generator_g(), b"m", &sig));
        let mut l_plus_one = l_bytes;
        l_plus_one[0] = l_plus_one[0].wrapping_add(1);
        sig.s = l_plus_one;
        assert!(!verify(&pk, &generator_g(), b"m", &sig));
    }

    #[test]
    fn zero_s_is_rejected() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let mut sig = sign(&sk, &generator_g(), b"m");
        sig.s = [0u8; 32];
        assert!(!verify(&pk, &generator_g(), b"m", &sig));
    }

    #[test]
    fn identity_r_is_rejected() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let mut sig = sign(&sk, &generator_g(), b"m");
        sig.r = RistrettoPoint::identity().compress().to_bytes();
        assert!(!verify(&pk, &generator_g(), b"m", &sig));
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        for _ in 0..128 {
            let sig = SchnorrSig {
                r: random_32(),
                s: random_32(),
            };
            let _ = verify(&pk, &generator_g(), b"m", &sig);
        }
    }

    #[test]
    fn empty_message_roundtrips() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let sig = sign(&sk, &generator_g(), b"");
        assert!(verify(&pk, &generator_g(), b"", &sig));
    }

    #[test]
    fn long_message_roundtrips() {
        let sk = random_scalar();
        let pk = generator_g() * sk;
        let msg = vec![0xa5u8; 65_536];
        let sig = sign(&sk, &generator_g(), &msg);
        assert!(verify(&pk, &generator_g(), &msg, &sig));
    }

    #[test]
    fn many_random_roundtrips() {
        for _ in 0..64 {
            let sk = random_scalar();
            let pk = generator_g() * sk;
            let msg = random_32();
            let sig = sign(&sk, &generator_g(), &msg);
            assert!(verify(&pk, &generator_g(), &msg, &sig));
        }
    }
}
