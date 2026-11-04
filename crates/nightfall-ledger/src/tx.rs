//! Transactions: inputs, outputs, kernels.
//!
//! # Signature binding
//!
//! * **Input signature** under the output's one-time key `Ko`, over the
//!   commitment alone, proving the spender is the party the funds were sent
//!   to.
//! * **Kernel excess signature** under generator `H`, proving no value was
//!   created.
//!
//! From protocol 1.1.0 every input also carries a fresh input key with a
//! signature under `Ki + h·Ko`, every kernel a stealth excess, and one
//! block-wide key balance ties them to the outputs' ephemeral keys. See
//! [`nightfall_crypto::binding`](nightfall_crypto).
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

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
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
    /// Fresh input key `Ki`. Required from the binding fork on, stripped
    /// below it. See [`nightfall_crypto::binding`](nightfall_crypto).
    #[serde(default)]
    pub input_key: Option<[u8; 32]>,
    /// Signature under `Ki + h·Ko` over `H(commit ‖ Ki)`.
    #[serde(default)]
    pub key_sig: Option<SchnorrSig>,
}

impl Input {
    /// The 1.0.x shape: commitment and signature only.
    pub fn legacy(commit: Commitment, sig: SchnorrSig) -> Self {
        Self {
            commit,
            sig,
            input_key: None,
            key_sig: None,
        }
    }

    pub fn strip_binding(&mut self) {
        self.input_key = None;
        self.key_sig = None;
    }

    pub fn has_binding(&self) -> bool {
        self.input_key.is_some() || self.key_sig.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub version: u32,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub kernels: Vec<TxKernel>,
    /// Public scalar `s` of the key balance. Summed when aggregated.
    #[serde(default)]
    pub stealth_offset: Option<[u8; 32]>,
    /// Public scalar `k` with `Σout − Σin + fee·G − reward·G = Σ E + k·H`.
    /// Only valid from the binding fork on.
    #[serde(default)]
    pub kernel_offset: Option<[u8; 32]>,
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

    /// Wrapping sum, exactly as 1.0.x release builds computed it. Consensus
    /// below the binding fork depends on this; use [`Self::checked_total_fee`]
    /// anywhere else.
    pub fn total_fee(&self) -> u64 {
        self.kernels
            .iter()
            .fold(0u64, |acc, k| acc.wrapping_add(k.fee_darks))
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
    /// The input key signature ([`Transaction::verify_input_key`]) binds the
    /// input to its transaction.
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
        if !self.has_any_binding() {
            return self.legacy_txid();
        }
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut ins: Vec<&Input> = self.inputs.iter().collect();
        ins.sort_unstable_by_key(|i| i.commit.0);
        for i in &ins {
            let mut v = i.commit.0.to_vec();
            v.extend_from_slice(&i.input_key.unwrap_or_default());
            parts.push(v);
        }
        for o in &self.outputs {
            parts.push(o.commitment_bytes());
        }
        for k in &self.kernels {
            let mut v = k.signing_message();
            v.extend_from_slice(&k.stealth_excess.unwrap_or_default());
            parts.push(v);
        }
        parts.push(self.stealth_offset.unwrap_or_default().to_vec());
        parts.push(self.kernel_offset.unwrap_or_default().to_vec());
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        hash_multi(domain::TX_BOUND, &refs)
    }

    /// The 1.0.x transaction id, which ignores every binding field.
    ///
    /// An old relay re-serialises a transaction without the fields it does
    /// not know, so the same spend can arrive in two shapes; this id is what
    /// they have in common.
    pub fn legacy_txid(&self) -> Hash256 {
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

    /// Does any field of the binding rules appear?
    pub fn has_any_binding(&self) -> bool {
        self.stealth_offset.is_some()
            || self.kernel_offset.is_some()
            || self.inputs.iter().any(Input::has_binding)
            || self
                .kernels
                .iter()
                .any(|k| k.stealth_excess.is_some() || k.stealth_sig.is_some())
    }

    /// Does every object carry its binding fields?
    pub fn has_full_binding(&self) -> bool {
        self.inputs
            .iter()
            .all(|i| i.input_key.is_some() && i.key_sig.is_some())
            && self
                .kernels
                .iter()
                .all(|k| k.stealth_excess.is_some() && k.stealth_sig.is_some())
    }

    /// Restore the 1.0.x shape.
    pub fn strip_binding(&mut self) {
        for i in &mut self.inputs {
            i.strip_binding();
        }
        for k in &mut self.kernels {
            k.strip_binding();
        }
        self.stealth_offset = None;
        self.kernel_offset = None;
    }

    /// Total fee, or `None` if any kernel fee is out of range or the sum
    /// overflows.
    pub fn checked_total_fee(&self) -> Option<u64> {
        checked_fee_sum(&self.kernels)
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
        // The Coinbase flag is reserved for the coinbase output.
        if coinbase_kernels == 0 && self.outputs.iter().any(|o| o.features.is_coinbase()) {
            return Err(TxError::CoinbaseOutputInTransaction);
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
        self.verify_stateless_rules(ctx, false)
    }

    /// [`Self::verify_stateless`] under the rules of a given height.
    ///
    /// `binding` is whether the binding fork is active for the block this
    /// transaction would go into. Below it the binding fields are optional
    /// and a kernel offset is refused; at or above it every field is required
    /// and both balance equations must hold.
    pub fn verify_stateless_rules(&self, ctx: &[u8], binding: bool) -> Result<(), TxError> {
        self.check_shape()?;
        if self.checked_total_fee().is_none() {
            return Err(TxError::FeeOutOfRange);
        }
        if !binding && self.kernel_offset.is_some() {
            return Err(TxError::KernelOffsetBeforeFork);
        }
        if binding && !self.has_full_binding() {
            return Err(TxError::MissingBinding);
        }
        // Present binding fields must be valid even before the fork, so a node
        // never relays a transaction that will turn invalid at activation.
        if binding || self.has_full_binding() {
            for k in &self.kernels {
                if k.verified_stealth_excess().is_none() {
                    return Err(TxError::BadStealthExcess);
                }
            }
            key_balance(
                &self.inputs,
                &self.outputs,
                &self.kernels,
                self.stealth_offset,
            )?;
        }
        self.verify_value_balance(ctx)
    }

    fn verify_value_balance(&self, ctx: &[u8]) -> Result<(), TxError> {
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
                .ok_or(TxError::MalformedCommitment)?
                + offset_point_h(self.kernel_offset)?;

        if expected != kernel_sum {
            return Err(TxError::UnbalancedTransaction);
        }
        Ok(())
    }

    /// Verify one input's binding signature against the `Ko` in the UTXO set.
    pub fn verify_input_key(input: &Input, output_pk: &[u8; 32]) -> bool {
        match (input.input_key, input.key_sig) {
            (Some(ki), Some(sig)) => {
                nightfall_crypto::verify_input_key(&input.commit, &ki, output_pk, &sig)
            }
            _ => false,
        }
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

/// Largest fee a single kernel may declare: the whole supply.
pub const MAX_KERNEL_FEE_DARKS: u64 = nightfall_types::MAX_SUPPLY_DARKS;

/// Σ fees with every fee bounded and the sum checked.
pub fn checked_fee_sum(kernels: &[TxKernel]) -> Option<u64> {
    kernels.iter().try_fold(0u64, |acc, k| {
        if k.fee_darks > MAX_KERNEL_FEE_DARKS {
            return None;
        }
        acc.checked_add(k.fee_darks)
    })
}

fn decode_offset(offset: Option<[u8; 32]>) -> Result<Scalar, TxError> {
    match offset {
        None => Ok(Scalar::ZERO),
        Some(bytes) => nightfall_crypto::scalar_from_bytes(&bytes).ok_or(TxError::BadOffset),
    }
}

/// `k·H` for a kernel offset (identity when absent).
pub fn offset_point_h(offset: Option<[u8; 32]>) -> Result<RistrettoPoint, TxError> {
    Ok(nightfall_crypto::generator_h() * decode_offset(offset)?)
}

/// Add two optional offsets. `None` + `None` stays `None`.
pub fn add_offsets(a: Option<[u8; 32]>, b: Option<[u8; 32]>) -> Option<[u8; 32]> {
    match (a, b) {
        (None, None) => None,
        _ => {
            let sa = a
                .and_then(|x| nightfall_crypto::scalar_from_bytes(&x))
                .unwrap_or(Scalar::ZERO);
            let sb = b
                .and_then(|x| nightfall_crypto::scalar_from_bytes(&x))
                .unwrap_or(Scalar::ZERO);
            Some((sa + sb).to_bytes())
        }
    }
}

/// The key balance: `Σ Ki − Σ Ke = Σ X + s·G`.
///
/// Every input must carry an input key and every kernel a verified stealth
/// excess; input key *signatures* need the UTXO set and are checked by the
/// ledger.
pub fn key_balance(
    inputs: &[Input],
    outputs: &[Output],
    kernels: &[TxKernel],
    stealth_offset: Option<[u8; 32]>,
) -> Result<(), TxError> {
    let mut lhs = RistrettoPoint::identity();
    for i in inputs {
        let ki = i.input_key.ok_or(TxError::MissingBinding)?;
        lhs += CompressedRistretto(ki)
            .decompress()
            .ok_or(TxError::BadInputKey)?;
    }
    for o in outputs {
        lhs -= CompressedRistretto(o.ephemeral_pk)
            .decompress()
            .ok_or(TxError::MalformedOutputKey)?;
    }
    let mut rhs = nightfall_crypto::generator_g() * decode_offset(stealth_offset)?;
    for k in kernels {
        rhs += k
            .verified_stealth_excess()
            .ok_or(TxError::BadStealthExcess)?;
    }
    if lhs != rhs {
        return Err(TxError::KeyImbalance);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TxError {
    #[error("binding fields missing")]
    MissingBinding,
    #[error("malformed input key")]
    BadInputKey,
    #[error("invalid input key signature")]
    BadInputKeySignature,
    #[error("missing or invalid stealth excess")]
    BadStealthExcess,
    #[error("key balance does not hold")]
    KeyImbalance,
    #[error("malformed offset")]
    BadOffset,
    #[error("kernel fee out of range")]
    FeeOutOfRange,
    #[error("kernel offset before the binding fork")]
    KernelOffsetBeforeFork,
    #[error("coinbase-flagged output outside a coinbase")]
    CoinbaseOutputInTransaction,
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
