//! ECDSA adaptor signatures on secp256k1.
//!
//! The adaptor lives on Bitcoin, not on the NIGHT kernel. Completing
//! TX_redeem with `s_a` publishes `s_a`; Bob extracts it and claims NIGHT.
//! ECDSA `s` vs `n−s` malleability: `ecdsa_fun` normalises to low-s.

use curve25519_dalek::scalar::Scalar as DalekScalar;
use ecdsa_fun::adaptor::{Adaptor, EncryptedSignature};
use ecdsa_fun::fun::{g, Point, Scalar as SecpScalar, G};
use ecdsa_fun::nonce::Deterministic;
use ecdsa_fun::{Signature, ECDSA};
use rand::{CryptoRng, RngCore};
use sha2::Sha256;
use sigma_fun::HashTranscript;

type Transcript = HashTranscript<Sha256, rand_chacha::ChaCha20Rng>;
type AdaptorInst = Adaptor<Transcript, Deterministic<Sha256>>;

/// Convert a Ristretto scalar (≤ 2^252) into a secp256kfun scalar.
/// Endianness: dalek is LE, secp256kfun is BE.
pub fn dalek_to_secp(s: &DalekScalar) -> SecpScalar {
    let mut b = s.to_bytes();
    b.reverse();
    SecpScalar::from_bytes(b)
        .expect("below 2^252 fits secp order")
        .non_zero()
        .expect("non-zero")
}

pub fn secp_to_dalek(s: &SecpScalar) -> Option<DalekScalar> {
    let mut b = s.to_bytes();
    b.reverse();
    Option::<DalekScalar>::from(DalekScalar::from_canonical_bytes(b))
}

pub fn encryption_point(s: &DalekScalar) -> Point {
    let x = dalek_to_secp(s);
    g_mul(&x)
}

fn g_mul(x: &SecpScalar) -> Point {
    g!(x * G).normalize()
}

pub fn random_bitcoin_sk<R: RngCore + CryptoRng>(rng: &mut R) -> SecpScalar {
    SecpScalar::random(rng)
}

pub fn verification_key(sk: &SecpScalar) -> Point {
    ECDSA::<()>::default().verification_key_for(sk)
}

pub fn sign(sk: &SecpScalar, message32: &[u8; 32]) -> Signature {
    let ecdsa = ECDSA::<Deterministic<Sha256>>::default();
    ecdsa.sign(sk, message32)
}

pub fn verify_sig(pk: &Point, message32: &[u8; 32], sig: &Signature) -> bool {
    ECDSA::verify_only().verify(pk, message32, sig)
}

pub fn encsign(
    sk: &SecpScalar,
    encryption_key: &Point,
    message32: &[u8; 32],
) -> EncryptedSignature {
    let adaptor = AdaptorInst::default();
    adaptor.encrypted_sign(sk, encryption_key, message32)
}

pub fn verify_encsig(
    pk: &Point,
    encryption_key: &Point,
    message32: &[u8; 32],
    enc: &EncryptedSignature,
) -> bool {
    AdaptorInst::default().verify_encrypted_signature(pk, encryption_key, message32, enc)
}

pub fn decrypt(s: &DalekScalar, enc: EncryptedSignature) -> Signature {
    AdaptorInst::default().decrypt_signature(&dalek_to_secp(s), enc)
}

/// Recover `s` from a published (decrypted) signature plus the adaptor.
/// Returns None if the signature is not the decryption of `enc`.
pub fn recover(
    encryption_key: &Point,
    signature: &Signature,
    enc: &EncryptedSignature,
) -> Option<DalekScalar> {
    let adaptor = AdaptorInst::default();
    let secp = adaptor.recover_decryption_key(encryption_key, signature, enc)?;
    secp_to_dalek(&secp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::swap::SwapShare;
    use rand::rngs::OsRng;

    #[test]
    fn decrypt_publishes_the_night_secret() {
        let share = SwapShare::generate();
        let mut rng = OsRng;
        let bob_sk = random_bitcoin_sk(&mut rng);
        let bob_pk = verification_key(&bob_sk);
        let y = encryption_point(&share.secret());
        let msg = [7u8; 32];

        let enc = encsign(&bob_sk, &y, &msg);
        assert!(verify_encsig(&bob_pk, &y, &msg, &enc));

        let sig = decrypt(&share.secret(), enc.clone());
        assert!(verify_sig(&bob_pk, &msg, &sig));

        let recovered = recover(&y, &sig, &enc).expect("extract s_a");
        assert_eq!(recovered, share.secret());
    }

    #[test]
    fn wrong_signature_does_not_yield_the_secret() {
        let share = SwapShare::generate();
        let mut rng = OsRng;
        let bob_sk = random_bitcoin_sk(&mut rng);
        let y = encryption_point(&share.secret());
        let msg = [7u8; 32];
        let enc = encsign(&bob_sk, &y, &msg);
        let other = sign(&bob_sk, &[8u8; 32]);
        assert!(recover(&y, &other, &enc).is_none());
    }
}
