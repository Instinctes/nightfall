//! Transaction kernels — the object that actually enforces "no value created".
//!
//! # The balance equation
//!
//! For a spend:
//!
//! ```text
//!   Σ outputs − Σ inputs − fee·G  =  excess·H
//! ```
//!
//! For a coinbase:
//!
//! ```text
//!   Σ outputs − reward·G  =  excess·H
//! ```
//!
//! The kernel publishes `excess·H` as a point and a Schnorr signature over
//! generator `H`. Verifying that signature proves the signer knows the discrete
//! log of the excess point **with respect to H**, which is only possible if the
//! point has no `G` component — i.e. the amounts cancel exactly.
//!
//! # Why the old code was unsound
//!
//! The previous `BalanceProof` stored `excess` and a `value_delta_ok_tag`, and
//! `verify_balance_proof` recomputed both from public data and compared them to
//! themselves. It could not fail. There was no signature and therefore no
//! statement being proven. Any attacker could pick any input and output amounts
//! at all. See `docs/AUDIT-2026-08-12.md`, finding C-01.

use crate::commit::{commit_public_value, generator_h, Commitment};
use crate::hash_multi;
use crate::schnorr::{self, SchnorrSig};
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use nightfall_types::Hash256;
use serde::{Deserialize, Serialize};

pub const KERNEL_DOMAIN: &[u8] = b"nightfall:kernel:v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelFeature {
    /// Ordinary transfer. `fee` is burned, `reward` must be zero.
    Plain,
    /// Block subsidy. `reward` is minted, `fee` must be zero.
    Coinbase,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxKernel {
    pub feature: KernelFeature,
    /// Burned in full (100% fee burn). Public by design — it is the one number
    /// that must be auditable to prove the burn actually happened.
    pub fee_darks: u64,
    /// Block subsidy for coinbase kernels, else 0.
    pub reward_darks: u64,
    /// Earliest height at which this kernel may be included.
    pub lock_height: u64,
    /// The excess point `excess·H`.
    pub excess: Commitment,
    /// Schnorr signature over generator `H` proving knowledge of `excess`.
    pub excess_sig: SchnorrSig,
}

impl TxKernel {
    /// Bytes the excess signature commits to.
    ///
    /// **Only the kernel's own fields.** This is what makes kernels
    /// aggregatable: a block merges every transaction's inputs, outputs and
    /// kernels into one flat set, and a kernel that had signed its originating
    /// transaction's body could no longer be verified once that body ceased to
    /// exist.
    ///
    /// Output integrity — the griefing hole from audit finding C-05, where a
    /// relay corrupted a ciphertext and destroyed the recipient's funds — is
    /// instead guarded by each output's own sender signature. See
    /// [`crate::stealth::Output::sender_sig`].
    pub fn signing_message(&self) -> Vec<u8> {
        let feature = match self.feature {
            KernelFeature::Plain => 0u8,
            KernelFeature::Coinbase => 1u8,
        };
        hash_multi(
            KERNEL_DOMAIN,
            &[
                &[feature],
                &self.fee_darks.to_le_bytes(),
                &self.reward_darks.to_le_bytes(),
                &self.lock_height.to_le_bytes(),
                &self.excess.0,
            ],
        )
        .0
        .to_vec()
    }

    /// Verify the excess signature. Does **not** check the balance equation —
    /// that is the ledger's job via [`expected_excess`].
    pub fn verify_signature(&self) -> bool {
        let Some(excess_point) = self.excess.point() else {
            return false;
        };
        // Reject the identity excess: it would let a kernel with a trivially
        // known DL (zero) authorise anything.
        if excess_point == RistrettoPoint::default() {
            return false;
        }
        let msg = self.signing_message();
        schnorr::verify(&excess_point, &generator_h(), &msg, &self.excess_sig)
    }

    /// Stable identifier, used to canonicalise kernel ordering in a block.
    pub fn id(&self) -> Hash256 {
        hash_multi(KERNEL_DOMAIN, &[&self.excess.0, &self.excess_sig.r])
    }

    /// Structural sanity independent of the rest of the block.
    pub fn check_shape(&self) -> Result<(), KernelError> {
        match self.feature {
            KernelFeature::Plain => {
                if self.reward_darks != 0 {
                    return Err(KernelError::RewardOnPlainKernel);
                }
            }
            KernelFeature::Coinbase => {
                if self.fee_darks != 0 {
                    return Err(KernelError::FeeOnCoinbaseKernel);
                }
            }
        }
        if self.excess.point().is_none() {
            return Err(KernelError::MalformedExcess);
        }
        Ok(())
    }
}

/// The excess point the ledger expects, computed purely from public data:
/// `Σ outputs − Σ inputs + fee·G − reward·G`.
///
/// Sanity check on the signs. For a spend of 100 into 90 + 10 fee:
/// `(90−100)·G + (b_out−b_in)·H + 10·G = 0·G + (b_out−b_in)·H` ✓
/// For a coinbase of `reward`:
/// `reward·G + b·H − reward·G = b·H` ✓
///
/// The kernel's stored excess must equal this **and** be signed. Equality alone
/// proves nothing (that was the old bug); the signature is what carries the
/// soundness.
pub fn expected_excess(
    inputs: &[Commitment],
    outputs: &[Commitment],
    fee_darks: u64,
    reward_darks: u64,
) -> Option<RistrettoPoint> {
    let mut acc = Commitment::sum(outputs)?;
    acc -= Commitment::sum(inputs)?;
    acc += commit_public_value(fee_darks);
    acc -= commit_public_value(reward_darks);
    Some(acc)
}

/// Build and sign a kernel. `excess_secret` must be
/// `Σ output_blinds − Σ input_blinds`.
pub fn build_kernel(
    feature: KernelFeature,
    fee_darks: u64,
    reward_darks: u64,
    lock_height: u64,
    excess_secret: &Scalar,
) -> TxKernel {
    let excess_point = generator_h() * excess_secret;
    let mut kernel = TxKernel {
        feature,
        fee_darks,
        reward_darks,
        lock_height,
        excess: Commitment::from_point(excess_point),
        excess_sig: SchnorrSig {
            r: [0u8; 32],
            s: [0u8; 32],
        },
    };
    let msg = kernel.signing_message();
    kernel.excess_sig = schnorr::sign(excess_secret, &generator_h(), &msg);
    kernel
}

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("reward set on a non-coinbase kernel")]
    RewardOnPlainKernel,
    #[error("fee set on a coinbase kernel")]
    FeeOnCoinbaseKernel,
    #[error("malformed excess point")]
    MalformedExcess,
    #[error("excess does not match the balance equation")]
    ExcessMismatch,
    #[error("invalid excess signature")]
    BadSignature,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::generator_g;
    use rand::rngs::OsRng;

    #[test]
    fn balanced_transaction_verifies() {
        // in: 100 -> out: 90 + fee 10
        let bin = Scalar::random(&mut OsRng);
        let bout = Scalar::random(&mut OsRng);
        let input = Commitment::new(100, &bin);
        let output = Commitment::new(90, &bout);

        let excess_secret = bout - bin;
        let k = build_kernel(KernelFeature::Plain, 10, 0, 0, &excess_secret);

        assert!(k.verify_signature());
        let expected = expected_excess(&[input], &[output], 10, 0).unwrap();
        assert_eq!(Commitment::from_point(expected), k.excess);
    }

    #[test]
    fn inflation_attempt_fails() {
        // Attacker claims 100 in, wants 1_000_000 out. The excess point then
        // carries a −999_910·G term. To sign it under H the attacker would have
        // to know the DL between G and H, which is the hardness assumption.
        let bin = Scalar::random(&mut OsRng);
        let bout = Scalar::random(&mut OsRng);
        let input = Commitment::new(100, &bin);
        let output = Commitment::new(1_000_000, &bout);

        // The honest excess secret the attacker can compute:
        let excess_secret = bout - bin;
        let k = build_kernel(KernelFeature::Plain, 0, 0, 0, &excess_secret);

        // Signature over H is valid for the point excess_secret·H ...
        assert!(k.verify_signature());
        // ... but that point is NOT the excess the balance equation demands.
        let expected = expected_excess(&[input], &[output], 0, 0).unwrap();
        assert_ne!(
            Commitment::from_point(expected),
            k.excess,
            "balance equation must expose the inflation"
        );
    }

    #[test]
    fn cannot_sign_a_point_with_g_component() {
        // Take a valid excess and add value·G to it. There is no way to produce
        // a signature over H for the result.
        let secret = Scalar::random(&mut OsRng);
        let tainted = generator_h() * secret + generator_g() * Scalar::from(500u64);
        let mut k = build_kernel(KernelFeature::Plain, 0, 0, 0, &secret);
        k.excess = Commitment::from_point(tainted);
        assert!(
            !k.verify_signature(),
            "a point with a G component must not verify under H"
        );
    }

    #[test]
    fn signature_binds_the_fee() {
        let secret = Scalar::random(&mut OsRng);
        let mut k = build_kernel(KernelFeature::Plain, 5, 0, 0, &secret);
        assert!(k.verify_signature());
        k.fee_darks = 0; // steal the fee
        assert!(!k.verify_signature());
    }

    #[test]
    fn signature_binds_the_reward_and_lock_height() {
        let secret = Scalar::random(&mut OsRng);
        let mut k = build_kernel(KernelFeature::Coinbase, 0, 100, 7, &secret);
        assert!(k.verify_signature());
        k.reward_darks = 1_000_000;
        assert!(!k.verify_signature(), "reward must be signed");

        let mut k2 = build_kernel(KernelFeature::Coinbase, 0, 100, 7, &secret);
        k2.lock_height = 8;
        assert!(!k2.verify_signature(), "lock height must be signed");
    }

    #[test]
    fn kernels_survive_aggregation() {
        // Two independent kernels dropped into one flat set must both still
        // verify. This is the property that lets a block dissolve transaction
        // boundaries — and the reason the kernel no longer signs a tx body.
        let a_secret = Scalar::random(&mut OsRng);
        let b_secret = Scalar::random(&mut OsRng);
        let a = build_kernel(KernelFeature::Plain, 3, 0, 0, &a_secret);
        let b = build_kernel(KernelFeature::Plain, 7, 0, 0, &b_secret);

        for k in [&b, &a] {
            assert!(k.verify_signature());
        }

        // And the excesses sum exactly the way the balance equation needs.
        let sum = Commitment::sum(&[a.excess, b.excess]).unwrap();
        assert_eq!(
            Commitment::from_point(sum),
            Commitment::from_point(generator_h() * (a_secret + b_secret)),
            "aggregate excess must equal the sum of the parts"
        );
    }

    #[test]
    fn coinbase_kernel_balances() {
        let b = Scalar::random(&mut OsRng);
        let reward = 20_00000000u64;
        let out = Commitment::new(reward, &b);
        let k = build_kernel(KernelFeature::Coinbase, 0, reward, 0, &b);
        assert!(k.verify_signature());
        // Σout − reward·G == b·H
        let expected = expected_excess(&[], &[out], 0, reward).unwrap();
        assert_eq!(Commitment::from_point(expected), k.excess);
    }

    #[test]
    fn shape_rules_enforced() {
        let secret = Scalar::random(&mut OsRng);
        let mut k = build_kernel(KernelFeature::Coinbase, 0, 100, 0, &secret);
        assert!(k.check_shape().is_ok());
        k.fee_darks = 1;
        assert!(k.check_shape().is_err());
    }
}
