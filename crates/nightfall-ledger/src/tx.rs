//! Transactions: inputs, outputs, kernels.
//!
//! # Signature binding
//!
//! Two signatures guard every spend, and both cover the whole transaction:
//!
//! * **Input signature** under the output's one-time key `Ko`, proving the
//!   spender is the party the funds were sent to. Without it, whoever created
//!   the output (the *sender*) could spend it back — the classic hole in naive
//!   non-interactive Mimblewimble.
//! * **Kernel excess signature** under generator `H`, proving no value was
//!   created.
//!
//! A third signature sits on each output, made by the sender's ephemeral key,
//! sealing the commitment, range proof, one-time key and encrypted payload.
//!
//! None of the three binds a *transaction body*, and that is deliberate: a
//! block aggregates all transactions into one flat set of inputs, outputs and
//! kernels, so any signature over a per-transaction body would stop verifying
//! the moment that body ceased to exist. Integrity is instead attached to the
//! individual objects.
//!
//! In v4, `auth_transcript()` omitted `input_commits`, the ciphertexts, the
//! proof and even `auth_pk` — so signatures were swappable, txids were
//! malleable, and a single flipped ciphertext byte destroyed the recipient's
//! money (audit findings C-03 to C-05).

use nightfall_crypto::{
    domain, expected_excess, hash_multi, sig, Commitment, KernelFeature, Output, SchnorrSig,
    TxKernel,
};
use nightfall_types::Hash256;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Spending reference to an existing UTXO.
///
/// The one-time key is *not* carried here: it is looked up from the UTXO set,
/// so a spender cannot substitute a key of their choosing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub commit: Commitment,
    /// Schnorr signature under the referenced output's `Ko`, over `input_message`.
    pub sig: SchnorrSig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub version: u32,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub kernels: Vec<TxKernel>,
}

/// Consensus limits. Unbounded transactions are a remote OOM vector: the v4 P2P
/// layer read newline-delimited JSON with no size cap at all.
pub const MAX_INPUTS: usize = 512;
pub const MAX_OUTPUTS: usize = 512;
pub const MAX_KERNELS: usize = 64;

impl Transaction {
    pub fn is_coinbase(&self) -> bool {
        self.kernels
            .iter()
            .any(|k| k.feature == KernelFeature::Coinbase)
    }

    pub fn total_fee(&self) -> u64 {
        self.kernels.iter().map(|k| k.fee_darks).sum()
    }

    pub fn total_reward(&self) -> u64 {
        self.kernels.iter().map(|k| k.reward_darks).sum()
    }

    /// Message an input signature covers.
    ///
    /// Only the commitment being spent. Under kernel aggregation a block has
    /// no per-transaction body to bind to, and binding to one would make
    /// inputs unaggregatable.
    ///
    /// Replaying a captured input signature is useless: the output is removed
    /// from the UTXO set the moment it is spent. Re-routing the value is
    /// prevented by the kernel, which requires knowledge of the input's
    /// blinding factor that no third party has.
    pub fn input_message(commit: &Commitment) -> Vec<u8> {
        hash_multi(domain::INPUT, &[&commit.0]).0.to_vec()
    }

    /// Canonical transaction id, covering the entire body including proofs
    /// and payloads, so it is not malleable.
    ///
    /// Only meaningful before the transaction is mined. A block aggregates
    /// every transaction into one flat set, at which point individual
    /// transactions no longer exist on chain — that is the point of
    /// aggregation. The wallet tracks a pending send by this id and marks it
    /// confirmed when its inputs are observed spent.
    pub fn txid(&self) -> Hash256 {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut ins: Vec<[u8; 32]> = self.inputs.iter().map(|i| i.commit.0).collect();
        ins.sort_unstable();
        for c in &ins {
            parts.push(c.to_vec());
        }
        for o in &self.outputs {
            parts.push(o.commitment_bytes());
        }
        for k in &self.kernels {
            parts.push(k.signing_message());
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        hash_multi(domain::TX, &refs)
    }

    /// Checks that need no chain context.
    pub fn check_shape(&self) -> Result<(), TxError> {
        if self.inputs.len() > MAX_INPUTS {
            return Err(TxError::TooManyInputs);
        }
        if self.outputs.len() > MAX_OUTPUTS {
            return Err(TxError::TooManyOutputs);
        }
        if self.kernels.is_empty() || self.kernels.len() > MAX_KERNELS {
            return Err(TxError::BadKernelCount);
        }
        if self.outputs.is_empty() {
            return Err(TxError::NoOutputs);
        }

        // No duplicate inputs within a single transaction.
        let mut seen = BTreeSet::new();
        for i in &self.inputs {
            if !seen.insert(i.commit.0) {
                return Err(TxError::DuplicateInput);
            }
        }
        // No duplicate output commitments either — they would collide in the
        // UTXO set and make one of them unspendable.
        let mut seen_out = BTreeSet::new();
        for o in &self.outputs {
            if !seen_out.insert(o.commit.0) {
                return Err(TxError::DuplicateOutput);
            }
            if o.commit.point().is_none() {
                return Err(TxError::MalformedCommitment);
            }
            if o.output_point().is_none() {
                return Err(TxError::MalformedOutputKey);
            }
        }

        let coinbase_kernels = self
            .kernels
            .iter()
            .filter(|k| k.feature == KernelFeature::Coinbase)
            .count();
        if coinbase_kernels > 1 {
            return Err(TxError::MultipleCoinbaseKernels);
        }
        // A coinbase mints; it must not also consume existing coins.
        if coinbase_kernels == 1 && !self.inputs.is_empty() {
            return Err(TxError::CoinbaseWithInputs);
        }

        for k in &self.kernels {
            k.check_shape().map_err(|_| TxError::BadKernel)?;
        }
        Ok(())
    }

    /// Verify range proofs, kernel signatures and the balance equation.
    ///
    /// Input signatures cannot be checked here because they need the UTXO set
    /// to resolve each input's one-time key — the ledger does that.
    pub fn verify_stateless(&self, ctx: &[u8]) -> Result<(), TxError> {
        self.check_shape()?;

        // Every output must carry a valid range proof. Skipping this is what
        // allowed a commitment to −1 to pass as a legitimate amount.
        for o in &self.outputs {
            if !nightfall_crypto::rangeproofs::verify(&o.range_proof, &o.commit, ctx) {
                return Err(TxError::BadRangeProof);
            }
            // And a signature from whoever created it, so a relay cannot
            // corrupt the encrypted payload and destroy the funds.
            if !o.verify_sender_sig() {
                return Err(TxError::BadOutputSignature);
            }
        }

        for k in &self.kernels {
            if !k.verify_signature() {
                return Err(TxError::BadKernelSignature);
            }
        }

        // Balance: Σout − Σin + fee·G − reward·G must equal Σ kernel excesses.
        let in_commits: Vec<Commitment> = self.inputs.iter().map(|i| i.commit).collect();
        let out_commits: Vec<Commitment> = self.outputs.iter().map(|o| o.commit).collect();
        let expected = expected_excess(
            &in_commits,
            &out_commits,
            self.total_fee(),
            self.total_reward(),
        )
        .ok_or(TxError::MalformedCommitment)?;

        let kernel_sum =
            Commitment::sum(&self.kernels.iter().map(|k| k.excess).collect::<Vec<_>>())
                .ok_or(TxError::MalformedCommitment)?;

        if expected != kernel_sum {
            return Err(TxError::UnbalancedTransaction);
        }
        Ok(())
    }

    /// Verify one input's signature against the one-time key recorded in the
    /// UTXO set.
    ///
    /// Free function on the type rather than a method: after aggregation the
    /// input no longer belongs to any transaction.
    pub fn verify_input_signature(input: &Input, output_pk: &[u8; 32]) -> bool {
        use curve25519_dalek::ristretto::CompressedRistretto;
        let Some(ko) = CompressedRistretto(*output_pk).decompress() else {
            return false;
        };
        let msg = Self::input_message(&input.commit);
        sig::verify(&ko, &nightfall_crypto::generator_g(), &msg, &input.sig)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TxError {
    #[error("too many inputs")]
    TooManyInputs,
    #[error("too many outputs")]
    TooManyOutputs,
    #[error("transaction has no outputs")]
    NoOutputs,
    #[error("invalid kernel count")]
    BadKernelCount,
    #[error("duplicate input within transaction")]
    DuplicateInput,
    #[error("duplicate output commitment")]
    DuplicateOutput,
    #[error("malformed commitment")]
    MalformedCommitment,
    #[error("malformed one-time output key")]
    MalformedOutputKey,
    #[error("more than one coinbase kernel")]
    MultipleCoinbaseKernels,
    #[error("coinbase must not spend inputs")]
    CoinbaseWithInputs,
    #[error("malformed kernel")]
    BadKernel,
    #[error("invalid range proof")]
    BadRangeProof,
    #[error("invalid kernel excess signature")]
    BadKernelSignature,
    #[error("transaction does not balance")]
    UnbalancedTransaction,
    #[error("invalid input signature")]
    BadInputSignature,
    #[error("invalid output sender signature")]
    BadOutputSignature,
}
