//! Wallet key hierarchy: scan key, spend key, addresses, view keys.
//!
//! Everything lives on Ristretto now. The old design used ed25519 for spending
//! and x25519 for encryption, which meant two curves, two encodings, and a
//! nullifier derived from `hash(spend_secret_bytes)` — so handing anyone a
//! "view key" would have handed them the spend key. See audit finding P-03.
//!
//! ```text
//!   seed ─┬─> scan_sk  (a)   A = a·G   detect + decrypt outputs
//!         └─> spend_sk (b)   B = b·G   authorise spending
//! ```
//!
//! An address is `(A, B)`. A **view key** is `(a, B)`: it finds and opens every
//! output belonging to the wallet and reads amounts and memos, but cannot sign,
//! because signing an output needs `b`. That is the user-controlled disclosure
//! promised in `docs/ATTRIBUTES.md`, now actually implemented.

use crate::hash_multi;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::commit::generator_g;

fn scalar_from_seed(domain: &[u8], seed: &[u8; 32]) -> Scalar {
    let a = hash_multi(domain, &[seed, b"0"]);
    let b = hash_multi(domain, &[seed, b"1"]);
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a.0);
    wide[32..].copy_from_slice(&b.0);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// Public payment address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Scan public key `A`.
    pub scan_pk: [u8; 32],
    /// Spend public key `B`.
    pub spend_pk: [u8; 32],
}

impl Address {
    pub fn scan_point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.scan_pk).decompress()
    }

    pub fn spend_point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.spend_pk).decompress()
    }

    /// Wire form: `nf1` + 64 bytes hex. Includes a 4-byte checksum so a
    /// mistyped address is rejected instead of burning funds.
    pub fn encode(&self) -> String {
        let mut payload = Vec::with_capacity(64);
        payload.extend_from_slice(&self.scan_pk);
        payload.extend_from_slice(&self.spend_pk);
        let check = hash_multi(b"nightfall:addr:checksum:v2", &[&payload]).0;
        format!("nf1{}{}", hex::encode(&payload), hex::encode(&check[..4]))
    }

    pub fn decode(s: &str) -> Result<Self, CryptoError> {
        let s = s.trim();
        let body = s.strip_prefix("nf1").ok_or(CryptoError::BadAddress)?;
        let raw = hex::decode(body).map_err(|_| CryptoError::BadAddress)?;
        if raw.len() != 68 {
            return Err(CryptoError::BadAddress);
        }
        let (payload, check) = raw.split_at(64);
        let expect = hash_multi(b"nightfall:addr:checksum:v2", &[payload]).0;
        if check != &expect[..4] {
            return Err(CryptoError::BadAddressChecksum);
        }
        let mut scan_pk = [0u8; 32];
        let mut spend_pk = [0u8; 32];
        scan_pk.copy_from_slice(&payload[..32]);
        spend_pk.copy_from_slice(&payload[32..]);
        let addr = Self { scan_pk, spend_pk };
        // Reject addresses whose keys are not valid group elements.
        if addr.scan_point().is_none() || addr.spend_point().is_none() {
            return Err(CryptoError::BadAddress);
        }
        Ok(addr)
    }

    pub fn short(&self) -> String {
        let s = self.encode();
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    }
}

/// Full wallet secret material.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct WalletKeys {
    pub seed: [u8; 32],
    scan_sk: Scalar,
    spend_sk: Scalar,
}

impl Clone for WalletKeys {
    fn clone(&self) -> Self {
        Self::from_seed(self.seed)
    }
}

impl std::fmt::Debug for WalletKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print secrets, not even accidentally via a derived Debug on an
        // enclosing struct.
        f.debug_struct("WalletKeys")
            .field("address", &self.address().short())
            .finish_non_exhaustive()
    }
}

impl WalletKeys {
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        Self::from_seed(seed)
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            seed,
            scan_sk: scalar_from_seed(b"nightfall:derive:scan:v2", &seed),
            spend_sk: scalar_from_seed(b"nightfall:derive:spend:v2", &seed),
        }
    }

    pub fn scan_secret(&self) -> Scalar {
        self.scan_sk
    }

    pub fn spend_secret(&self) -> Scalar {
        self.spend_sk
    }

    pub fn scan_public(&self) -> RistrettoPoint {
        generator_g() * self.scan_sk
    }

    pub fn spend_public(&self) -> RistrettoPoint {
        generator_g() * self.spend_sk
    }

    pub fn address(&self) -> Address {
        Address {
            scan_pk: self.scan_public().compress().to_bytes(),
            spend_pk: self.spend_public().compress().to_bytes(),
        }
    }

    /// Watch-only credential: sees everything, spends nothing.
    pub fn view_key(&self) -> ViewKey {
        ViewKey {
            scan_sk: self.scan_sk,
            spend_pk: self.spend_public().compress().to_bytes(),
        }
    }
}

/// Watch-only key. Detects and decrypts outputs; cannot authorise a spend.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ViewKey {
    pub(crate) scan_sk: Scalar,
    #[zeroize(skip)]
    pub(crate) spend_pk: [u8; 32],
}

impl ViewKey {
    pub fn spend_point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.spend_pk).decompress()
    }

    pub fn encode(&self) -> String {
        format!(
            "nfview1{}{}",
            hex::encode(self.scan_sk.to_bytes()),
            hex::encode(self.spend_pk)
        )
    }

    pub fn decode(s: &str) -> Result<Self, CryptoError> {
        let body = s
            .trim()
            .strip_prefix("nfview1")
            .ok_or(CryptoError::BadViewKey)?;
        let raw = hex::decode(body).map_err(|_| CryptoError::BadViewKey)?;
        if raw.len() != 64 {
            return Err(CryptoError::BadViewKey);
        }
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&raw[..32]);
        let mut spend_pk = [0u8; 32];
        spend_pk.copy_from_slice(&raw[32..]);
        let scan_sk = Option::<Scalar>::from(Scalar::from_canonical_bytes(sk))
            .ok_or(CryptoError::BadViewKey)?;
        Ok(Self { scan_sk, spend_pk })
    }
}

impl From<&WalletKeys> for ViewKey {
    fn from(k: &WalletKeys) -> Self {
        k.view_key()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed")]
    Decrypt,
    #[error("malformed address")]
    BadAddress,
    #[error("address checksum mismatch — check for a typo")]
    BadAddressChecksum,
    #[error("malformed view key")]
    BadViewKey,
    #[error("commitment does not open to the stated value")]
    CommitmentMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_roundtrip() {
        let k = WalletKeys::generate();
        let a = k.address();
        assert_eq!(Address::decode(&a.encode()).unwrap(), a);
    }

    #[test]
    fn address_checksum_catches_typos() {
        let k = WalletKeys::generate();
        let mut s = k.address().encode();
        // flip one hex character in the payload
        let bytes = unsafe { s.as_bytes_mut() };
        bytes[10] = if bytes[10] == b'a' { b'b' } else { b'a' };
        assert!(matches!(
            Address::decode(&s),
            Err(CryptoError::BadAddressChecksum) | Err(CryptoError::BadAddress)
        ));
    }

    #[test]
    fn seed_is_deterministic() {
        let seed = [7u8; 32];
        assert_eq!(
            WalletKeys::from_seed(seed).address(),
            WalletKeys::from_seed(seed).address()
        );
    }

    #[test]
    fn scan_and_spend_keys_are_independent() {
        let k = WalletKeys::generate();
        assert_ne!(k.scan_secret(), k.spend_secret());
    }

    #[test]
    fn view_key_roundtrip_and_cannot_spend() {
        let k = WalletKeys::generate();
        let v = k.view_key();
        let decoded = ViewKey::decode(&v.encode()).unwrap();
        assert_eq!(decoded.spend_pk, v.spend_pk);
        // The view key type simply has no method returning the spend secret.
        // This is enforced by the type system, not by convention.
    }

    #[test]
    fn debug_does_not_leak_secrets() {
        let k = WalletKeys::generate();
        let s = format!("{k:?}");
        assert!(!s.contains(&hex::encode(k.seed)));
        assert!(!s.contains(&hex::encode(k.spend_secret().to_bytes())));
    }
}
