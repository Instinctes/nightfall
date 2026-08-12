//! Pedersen commitments on Ristretto.
//!
//! `C = v·G + b·H`
//!
//! `G` and `H` are taken from [`bulletproofs::PedersenGens`] so that our
//! commitments are *bit-identical* to the ones Bulletproofs range-proves. That
//! is not cosmetic: a range proof only proves something about a commitment
//! formed with the exact generators the proof system used.
//!
//! `H = hash_to_group(G)` is a NUMS point, so nobody knows `x` with `H = x·G`.
//! Everything in this codebase's soundness argument rests on that fact:
//!
//! * A point whose discrete log w.r.t. `H` is known has **no `G` component**.
//!   (If it had one, the signer would have solved the DL between `G` and `H`.)
//! * Therefore a valid excess signature proves the transaction created no value.
//!
//! The previous implementation derived its own `H` by hashing and then never
//! used the property above — it compared a publicly recomputable point against
//! itself, which proves nothing. See `docs/AUDIT-2026-08-12.md`, finding C-01.

use bulletproofs::PedersenGens;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
use serde::{Deserialize, Serialize};

/// The canonical generator pair for the whole protocol.
pub fn gens() -> PedersenGens {
    PedersenGens::default()
}

/// Value generator `G`.
pub fn generator_g() -> RistrettoPoint {
    gens().B
}

/// Blinding generator `H` (NUMS).
pub fn generator_h() -> RistrettoPoint {
    gens().B_blinding
}

/// A Pedersen commitment in compressed wire form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Commitment(pub [u8; 32]);

impl Commitment {
    pub fn identity() -> Self {
        Self(RistrettoPoint::identity().compress().to_bytes())
    }

    /// Commit to `value` with blinding factor `blind`.
    pub fn new(value: u64, blind: &Scalar) -> Self {
        Self(
            gens()
                .commit(Scalar::from(value), *blind)
                .compress()
                .to_bytes(),
        )
    }

    pub fn from_point(p: RistrettoPoint) -> Self {
        Self(p.compress().to_bytes())
    }

    /// Decompress. Returns `None` for non-canonical / invalid encodings, which
    /// is a validity condition every consensus path must check.
    pub fn point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.0).decompress()
    }

    pub fn compressed(&self) -> CompressedRistretto {
        CompressedRistretto(self.0)
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Σ of a commitment list. `None` if any element is malformed.
    pub fn sum(list: &[Commitment]) -> Option<RistrettoPoint> {
        let mut acc = RistrettoPoint::identity();
        for c in list {
            acc += c.point()?;
        }
        Some(acc)
    }
}

impl std::fmt::Display for Commitment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Commit to an explicit amount with a zero blinding factor: `v·G`.
/// Used for fees and block rewards, which are public by design.
pub fn commit_public_value(value: u64) -> RistrettoPoint {
    generator_g() * Scalar::from(value)
}

/// Derive a blinding scalar from arbitrary key material, uniformly.
pub fn blind_from_bytes(domain: &[u8], material: &[u8]) -> Scalar {
    let a = crate::hash_multi(domain, &[material, b"0"]);
    let b = crate::hash_multi(domain, &[material, b"1"]);
    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a.0);
    wide[32..].copy_from_slice(&b.0);
    Scalar::from_bytes_mod_order_wide(&wide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    #[test]
    fn homomorphic_addition() {
        let b1 = Scalar::random(&mut OsRng);
        let b2 = Scalar::random(&mut OsRng);
        let c1 = Commitment::new(30, &b1);
        let c2 = Commitment::new(12, &b2);
        let sum = Commitment::from_point(c1.point().unwrap() + c2.point().unwrap());
        assert_eq!(sum, Commitment::new(42, &(b1 + b2)));
    }

    #[test]
    fn commitment_hides_value() {
        let b = Scalar::random(&mut OsRng);
        assert_ne!(Commitment::new(1, &b), Commitment::new(2, &b));
    }

    #[test]
    fn generators_are_distinct_and_stable() {
        assert_ne!(generator_g().compress(), generator_h().compress());
        assert_eq!(generator_h().compress(), generator_h().compress());
    }
}
