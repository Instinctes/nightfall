//! Bulletproof range proofs.
//!
//! Every output commitment must carry a proof that its hidden value lies in
//! `[0, 2^64)`. Without this, "amounts" are just scalars mod the group order,
//! so an attacker commits to `l - 1` (i.e. −1) and conjures value while the
//! balance equation still closes. See `docs/AUDIT-2026-08-12.md`, finding C-02.

use bulletproofs::{BulletproofGens, RangeProof};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;
use serde::{Deserialize, Serialize};

use crate::commit::{gens, Commitment};

/// Amounts are u64 darks, so 64 bits is the natural and required width.
pub const RANGE_BITS: usize = 64;

/// Generator capacity. Must be ≥ RANGE_BITS and a power of two.
const GENS_CAPACITY: usize = 64;

/// Transcript label. Both prover and verifier must use the identical label,
/// otherwise verification fails — this is Fiat–Shamir domain separation.
const TRANSCRIPT_LABEL: &[u8] = b"nightfall:rangeproof:v2";

fn bp_gens() -> &'static BulletproofGens {
    static G: std::sync::OnceLock<BulletproofGens> = std::sync::OnceLock::new();
    G.get_or_init(|| BulletproofGens::new(GENS_CAPACITY, 1))
}

fn transcript(extra: &[u8]) -> Transcript {
    let mut t = Transcript::new(TRANSCRIPT_LABEL);
    // Bind the proof to protocol context so a proof cannot be lifted from one
    // chain/network and replayed on another.
    t.append_message(b"ctx", extra);
    t
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeProofBytes(pub Vec<u8>);

impl RangeProofBytes {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Prove `value` is in range under blinding factor `blind`.
///
/// Returns the proof plus the commitment the proof is bound to. Callers must
/// use the returned commitment verbatim.
///
/// `inline(never)` keeps dalek's field ops out of the caller's frame. On
/// iOS Safari that is the difference between a send and an `unreachable` trap.
#[inline(never)]
pub fn prove(
    value: u64,
    blind: &Scalar,
    ctx: &[u8],
) -> Result<(RangeProofBytes, Commitment), RangeError> {
    let pc = gens();
    let bp = bp_gens();
    let mut t = transcript(ctx);
    let (proof, committed) = RangeProof::prove_single(bp, &pc, &mut t, value, blind, RANGE_BITS)
        .map_err(|_| RangeError::ProveFailed)?;
    Ok((
        RangeProofBytes(proof.to_bytes()),
        Commitment(committed.to_bytes()),
    ))
}

/// Verify a range proof against a commitment.
pub fn verify(proof: &RangeProofBytes, commitment: &Commitment, ctx: &[u8]) -> bool {
    // A malformed or absent proof is a hard reject, never a pass-through.
    let Ok(parsed) = RangeProof::from_bytes(&proof.0) else {
        return false;
    };
    let pc = gens();
    let bp = bp_gens();
    let mut t = transcript(ctx);
    parsed
        .verify_single(
            bp,
            &pc,
            &mut t,
            &CompressedRistretto(commitment.0),
            RANGE_BITS,
        )
        .is_ok()
}

#[derive(Debug, thiserror::Error)]
pub enum RangeError {
    #[error("range proof generation failed")]
    ProveFailed,
    #[error("range proof invalid")]
    VerifyFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    #[test]
    fn valid_proof_verifies() {
        let b = Scalar::random(&mut OsRng);
        let (proof, c) = prove(42_000_000, &b, b"test").unwrap();
        assert!(verify(&proof, &c, b"test"));
    }

    #[test]
    fn proof_is_bound_to_context() {
        let b = Scalar::random(&mut OsRng);
        let (proof, c) = prove(7, &b, b"mainnet").unwrap();
        assert!(
            !verify(&proof, &c, b"devnet"),
            "proof replayable across networks"
        );
    }

    #[test]
    fn proof_is_bound_to_commitment() {
        let b1 = Scalar::random(&mut OsRng);
        let b2 = Scalar::random(&mut OsRng);
        let (proof, _) = prove(7, &b1, b"c").unwrap();
        let other = Commitment::new(7, &b2);
        assert!(!verify(&proof, &other, b"c"));
    }

    #[test]
    fn garbage_proof_rejected() {
        let b = Scalar::random(&mut OsRng);
        let c = Commitment::new(1, &b);
        assert!(!verify(&RangeProofBytes(vec![0u8; 100]), &c, b"c"));
        assert!(!verify(&RangeProofBytes(vec![]), &c, b"c"));
    }

    #[test]
    fn negative_value_cannot_be_proven() {
        // Commit to the scalar representation of −1. There is no u64 `v` that
        // opens this commitment, so no range proof can exist for it. This is
        // exactly the attack that worked before range proofs existed.
        let b = Scalar::random(&mut OsRng);
        let minus_one = -Scalar::ONE;
        let fake = crate::commit::Commitment::from_point(
            crate::commit::generator_g() * minus_one + crate::commit::generator_h() * b,
        );
        let (proof, _) = prove(0, &b, b"c").unwrap();
        assert!(!verify(&proof, &fake, b"c"));
    }
}
