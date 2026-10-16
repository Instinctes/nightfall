//! Versioned cut-through transaction-format candidate.
//!
//! This module is deliberately NOT consensus-active.
//!
//! It defines the proposed wire objects and their canonical ordering/hashing
//! without modifying the existing `Transaction`, `Input`, `Output`, `TxKernel`
//! or `BlockBody` types.
//!
//! Consensus activation, UTXO lookup, stealth-balance validation, retention
//! horizon and actual cut-through remain later phases.

use std::collections::BTreeSet;

use curve25519_dalek::{ristretto::CompressedRistretto, scalar::Scalar};
use nightfall_crypto::{hash_multi, Commitment, Output, TxKernel};
use nightfall_types::{Hash256, Height};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::{
    is_mature, KernelBoundAuthorizationBundleError, KernelBoundAuthorizationBundleV1, LedgerState,
};

use crate::{
    KernelStealthBindingV1, SenderAuthorizationV1, SpendAuthorizationV1, StealthOffsetV1,
    MAX_INPUTS, MAX_KERNELS, MAX_OUTPUTS, TX_VERSION,
};

/// Candidate transaction version.
///
/// The active Nightfall transaction format remains `TX_VERSION == 2`.
pub const CUTTHROUGH_TX_VERSION: u32 = TX_VERSION + 1;

pub const CUTTHROUGH_BODY_DOMAIN: &[u8] = b"nightfall:cutthrough:body:v1";

pub const CUTTHROUGH_TX_DOMAIN: &[u8] = b"nightfall:cutthrough:tx:v1";

/// Versioned input candidate.
///
/// Canonical `Ko` is deliberately absent. Validation must resolve it from the
/// authoritative UTXO set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughInputV1 {
    pub commit: Commitment,
    pub authorization: SpendAuthorizationV1,
}

/// Versioned output candidate.
///
/// The existing Nightfall `Output` remains intact while the independent `Ks`
/// authorization is carried separately during this prototype phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughOutputV1 {
    pub output: Output,
    pub sender_authorization: SenderAuthorizationV1,
}

/// Versioned kernel candidate.
///
/// A kernel may carry a stealth-excess binding. The exact policy deciding when
/// that binding is mandatory belongs to the later consensus-validation phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughKernelV1 {
    pub kernel: TxKernel,
    pub stealth_binding: Option<KernelStealthBindingV1>,
}

/// Flat, aggregatable candidate body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughBodyV1 {
    pub inputs: Vec<CutThroughInputV1>,
    pub outputs: Vec<CutThroughOutputV1>,
    pub kernels: Vec<CutThroughKernelV1>,

    /// Aggregate stealth offset x'.
    pub stealth_offset: StealthOffsetV1,
}

/// Complete versioned candidate transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughTransactionV1 {
    pub version: u32,
    pub body: CutThroughBodyV1,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughV1Error {
    #[error("wrong cut-through transaction version: got {got}, expected {expected}")]
    WrongVersion { got: u32, expected: u32 },

    #[error("too many inputs")]
    TooManyInputs,

    #[error("too many outputs")]
    TooManyOutputs,

    #[error("invalid kernel count")]
    BadKernelCount,

    #[error("transaction has no outputs")]
    NoOutputs,

    #[error("kernel fee/reward arithmetic overflow")]
    KernelAmountOverflow,

    #[error("duplicate input commitment")]
    DuplicateInput,

    #[error("duplicate output commitment")]
    DuplicateOutput,

    #[error("duplicate kernel")]
    DuplicateKernel,

    #[error("malformed input commitment at index {index}")]
    MalformedInputCommitment { index: usize },

    #[error("malformed Ki at input index {index}")]
    MalformedInputKey { index: usize },

    #[error("malformed input signature at index {index}")]
    MalformedInputSignature { index: usize },

    #[error("malformed output at index {index}")]
    MalformedOutput { index: usize },

    #[error("invalid existing output signature at index {index}")]
    InvalidExistingOutputSignature { index: usize },

    #[error("invalid Ks output authorization at index {index}")]
    InvalidSenderAuthorization { index: usize },

    #[error("invalid kernel at index {index}")]
    InvalidKernel { index: usize },

    #[error("invalid kernel/stealth binding at index {index}")]
    InvalidKernelStealthBinding { index: usize },

    #[error("non-canonical stealth offset")]
    NonCanonicalStealthOffset,

    #[error("body is not canonically ordered")]
    NonCanonicalBody,
}

/// Failure while aggregating v3 cut-through candidates.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughAggregateError {
    #[error("cannot aggregate an empty transaction set")]
    EmptyAggregate,

    #[error("aggregate would exceed the input limit")]
    TooManyInputs,

    #[error("aggregate would exceed the output limit")]
    TooManyOutputs,

    #[error("aggregate would exceed the kernel limit")]
    TooManyKernels,

    #[error("child transaction {index} is invalid: {source}")]
    InvalidChild {
        index: usize,

        #[source]
        source: CutThroughV1Error,
    },

    #[error("constructed aggregate is invalid: {0}")]
    InvalidAggregate(CutThroughV1Error),
}

/// Aggregate independently valid v3 candidates into one flat,
/// canonically ordered candidate.
///
/// This intentionally performs NO cut-through/pruning.
///
/// LIP-style one-sided authorization requires inputs and associated stealth
/// authorization material to remain available until the configured
/// proof-of-work retention horizon has passed. Pruning before that point
/// would remove security-critical evidence.
///
/// Aggregation therefore only:
///
/// * concatenates inputs
/// * concatenates outputs
/// * concatenates kernels/bindings
/// * adds all stealth offsets modulo the scalar field
/// * canonicalises the resulting flat body
///
/// Transaction grouping is consequently destroyed without prematurely
/// deleting authorization material.
pub fn aggregate_cutthrough_v1(
    txs: &[CutThroughTransactionV1],
) -> Result<CutThroughTransactionV1, CutThroughAggregateError> {
    if txs.is_empty() {
        return Err(CutThroughAggregateError::EmptyAggregate);
    }

    // Validate every child independently before consuming any material.
    for (index, tx) in txs.iter().enumerate() {
        tx.check_shape()
            .map_err(|source| CutThroughAggregateError::InvalidChild { index, source })?;
    }

    // Compute all sizes before allocation. This keeps malformed/untrusted
    // aggregate requests from bypassing the existing consensus limits.
    let total_inputs = txs
        .iter()
        .try_fold(0usize, |total, tx| total.checked_add(tx.body.inputs.len()))
        .ok_or(CutThroughAggregateError::TooManyInputs)?;

    if total_inputs > MAX_INPUTS {
        return Err(CutThroughAggregateError::TooManyInputs);
    }

    let total_outputs = txs
        .iter()
        .try_fold(0usize, |total, tx| total.checked_add(tx.body.outputs.len()))
        .ok_or(CutThroughAggregateError::TooManyOutputs)?;

    if total_outputs > MAX_OUTPUTS {
        return Err(CutThroughAggregateError::TooManyOutputs);
    }

    let total_kernels = txs
        .iter()
        .try_fold(0usize, |total, tx| total.checked_add(tx.body.kernels.len()))
        .ok_or(CutThroughAggregateError::TooManyKernels)?;

    if total_kernels > MAX_KERNELS {
        return Err(CutThroughAggregateError::TooManyKernels);
    }

    let mut inputs = Vec::with_capacity(total_inputs);

    let mut outputs = Vec::with_capacity(total_outputs);

    let mut kernels = Vec::with_capacity(total_kernels);

    let mut aggregate_stealth_offset = Scalar::ZERO;

    for tx in txs {
        // check_shape() above already guarantees canonical scalar encoding.
        let offset = tx
            .body
            .stealth_offset
            .scalar()
            .expect("validated child has canonical stealth offset");

        aggregate_stealth_offset += offset;

        inputs.extend(tx.body.inputs.iter().cloned());

        outputs.extend(tx.body.outputs.iter().cloned());

        kernels.extend(tx.body.kernels.iter().cloned());
    }

    let aggregate = CutThroughTransactionV1::new(CutThroughBodyV1 {
        inputs,
        outputs,
        kernels,

        stealth_offset: StealthOffsetV1::from_scalar(&aggregate_stealth_offset),
    });

    aggregate
        .check_shape()
        .map_err(CutThroughAggregateError::InvalidAggregate)?;

    Ok(aggregate)
}

/// Failure while constructing a v3 cut-through transfer candidate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughBuildError {
    #[error("no inputs selected")]
    NoInputs,

    #[error("too many inputs")]
    TooManyInputs,

    #[error("too many outputs")]
    TooManyOutputs,

    #[error("amount arithmetic overflow")]
    AmountOverflow,

    #[error("insufficient funds: have {have} darks, need {need}")]
    InsufficientFunds { have: u64, need: u64 },

    #[error("could not construct output")]
    OutputFailed,

    #[error("spendable commitment does not match value/blind at index {index}")]
    SpendableCommitmentMismatch { index: usize },

    #[error("could not construct input authorization at index {index}")]
    InputAuthorizationFailed { index: usize },

    #[error("could not construct stealth excess")]
    StealthExcessFailed,

    #[error("could not bind stealth excess to kernel")]
    KernelBindingFailed,

    #[error("builder produced an invalid MW value equation")]
    InternalValueBalanceMismatch,

    #[error(transparent)]
    Candidate(#[from] CutThroughV1Error),

    #[error(transparent)]
    Authorization(#[from] KernelBoundAuthorizationBundleError),
}

/// Build a complete v3 cut-through transfer candidate.
///
/// This is deliberately separate from the active v2 builder.
///
/// In addition to the ordinary Nightfall MW transfer it constructs:
///
/// * fresh `Ki` for every input
/// * independent `Ks` for every output
/// * one transaction stealth excess `E'`
/// * a value-kernel/E' binding
/// * aggregate stealth offset `x'`
///
/// The resulting candidate is internally checked before being returned.
///
/// This remains prototype-only and is NOT consensus-active.
pub fn build_cutthrough_transfer_v1(
    _owner: &nightfall_crypto::WalletKeys,
    spendables: &[crate::Spendable],
    payments: &[crate::Payment],
    fee_darks: u64,
    change_to: &nightfall_crypto::Address,
    lock_height: u64,
    ctx: &[u8],
) -> Result<CutThroughTransactionV1, CutThroughBuildError> {
    if spendables.is_empty() {
        return Err(CutThroughBuildError::NoInputs);
    }

    if spendables.len() > MAX_INPUTS {
        return Err(CutThroughBuildError::TooManyInputs);
    }

    let output_count = payments
        .len()
        .checked_add(1)
        .ok_or(CutThroughBuildError::AmountOverflow)?;

    if output_count > MAX_OUTPUTS {
        return Err(CutThroughBuildError::TooManyOutputs);
    }

    let input_value = spendables
        .iter()
        .try_fold(0u64, |sum, spendable| sum.checked_add(spendable.value))
        .ok_or(CutThroughBuildError::AmountOverflow)?;

    let payment_value = payments
        .iter()
        .try_fold(0u64, |sum, payment| sum.checked_add(payment.amount))
        .ok_or(CutThroughBuildError::AmountOverflow)?;

    let required = payment_value
        .checked_add(fee_darks)
        .ok_or(CutThroughBuildError::AmountOverflow)?;

    if input_value < required {
        return Err(CutThroughBuildError::InsufficientFunds {
            have: input_value,
            need: required,
        });
    }

    let change = input_value - required;

    // Validate wallet-supplied commitment openings before using their
    // blinding factors in the MW excess calculation.
    for (index, spendable) in spendables.iter().enumerate() {
        if Commitment::new(spendable.value, &spendable.blind) != spendable.commit {
            return Err(CutThroughBuildError::SpendableCommitmentMismatch { index });
        }
    }

    // -------------------------------------------------
    // Outputs + independent Ks.
    // -------------------------------------------------

    let mut outputs = Vec::with_capacity(output_count);

    let mut sender_secrets = Vec::with_capacity(output_count);

    let mut output_blind_sum = Scalar::ZERO;

    for payment in payments {
        let (output, secrets) =
            nightfall_crypto::create_output(&payment.to, payment.amount, &payment.memo, ctx)
                .map_err(|_| CutThroughBuildError::OutputFailed)?;

        output_blind_sum += secrets.blind;

        let ks = Scalar::random(&mut OsRng);

        let sender_authorization = SenderAuthorizationV1::sign(&output, &ks);

        sender_secrets.push(ks);

        outputs.push(CutThroughOutputV1 {
            output,
            sender_authorization,
        });
    }

    // Match the existing builder's privacy behaviour:
    // always produce change, including zero-valued change.
    let (change_output, change_secrets) =
        nightfall_crypto::create_output(change_to, change, "", ctx)
            .map_err(|_| CutThroughBuildError::OutputFailed)?;

    output_blind_sum += change_secrets.blind;

    let change_ks = Scalar::random(&mut OsRng);

    let change_sender_authorization = SenderAuthorizationV1::sign(&change_output, &change_ks);

    sender_secrets.push(change_ks);

    outputs.push(CutThroughOutputV1 {
        output: change_output,

        sender_authorization: change_sender_authorization,
    });

    // -------------------------------------------------
    // Inputs + fresh Ki.
    // -------------------------------------------------

    let mut inputs = Vec::with_capacity(spendables.len());

    let mut input_ephemeral_secrets = Vec::with_capacity(spendables.len());

    let mut spent_output_secrets = Vec::with_capacity(spendables.len());

    // canonicalise() can reorder the input vector.
    // Retain Ko keyed by commitment so the self-check can reconstruct
    // authoritative ordering afterwards.
    let mut ko_by_commit = std::collections::BTreeMap::<[u8; 32], [u8; 32]>::new();

    for (index, spendable) in spendables.iter().enumerate() {
        let ko_secret = spendable.spend_secret;

        let canonical_ko = (nightfall_crypto::generator_g() * ko_secret)
            .compress()
            .to_bytes();

        let ki_secret = Scalar::random(&mut OsRng);

        let authorization =
            SpendAuthorizationV1::sign(&spendable.commit, &ki_secret, &ko_secret, &canonical_ko)
                .ok_or(CutThroughBuildError::InputAuthorizationFailed { index })?;

        ko_by_commit.insert(spendable.commit.0, canonical_ko);

        input_ephemeral_secrets.push(ki_secret);

        spent_output_secrets.push(ko_secret);

        inputs.push(CutThroughInputV1 {
            commit: spendable.commit,

            authorization,
        });
    }

    // -------------------------------------------------
    // Ordinary Nightfall MW value kernel.
    // -------------------------------------------------

    let input_blind_sum = spendables
        .iter()
        .fold(Scalar::ZERO, |sum, spendable| sum + spendable.blind);

    // Σout − Σin + fee·G = excess·H
    let kernel_secret = output_blind_sum - input_blind_sum;

    let kernel = nightfall_crypto::build_kernel(
        nightfall_crypto::KernelFeature::Plain,
        fee_darks,
        0,
        lock_height,
        &kernel_secret,
    );

    // -------------------------------------------------
    // E' + explicit kernel binding.
    // -------------------------------------------------

    let stealth_excess_secret = Scalar::random(&mut OsRng);

    let stealth_excess = crate::StealthExcessV1::new(Commitment::from_point(
        nightfall_crypto::generator_g() * stealth_excess_secret,
    ))
    .ok_or(CutThroughBuildError::StealthExcessFailed)?;

    let kernel_binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
        .ok_or(CutThroughBuildError::KernelBindingFailed)?;

    // -------------------------------------------------
    // Aggregate stealth offset.
    // -------------------------------------------------

    let stealth_offset = crate::stealth_offset_secret(
        &sender_secrets,
        &input_ephemeral_secrets,
        &spent_output_secrets,
        &[stealth_excess_secret],
    );

    let tx = CutThroughTransactionV1::new(CutThroughBodyV1 {
        inputs,
        outputs,

        kernels: vec![CutThroughKernelV1 {
            kernel,

            stealth_binding: Some(kernel_binding),
        }],

        stealth_offset: StealthOffsetV1::from_scalar(&stealth_offset),
    });

    // -------------------------------------------------
    // Self-check 1: structural candidate validity.
    // -------------------------------------------------

    tx.check_shape()?;

    // -------------------------------------------------
    // Self-check 2: ordinary MW value equation.
    // -------------------------------------------------

    let input_commits: Vec<Commitment> = tx.body.inputs.iter().map(|input| input.commit).collect();

    let output_commits: Vec<Commitment> = tx
        .body
        .outputs
        .iter()
        .map(|output| output.output.commit)
        .collect();

    let expected = nightfall_crypto::expected_excess(
        &input_commits,
        &output_commits,
        tx.body.total_fee(),
        tx.body.total_reward(),
    )
    .ok_or(CutThroughBuildError::InternalValueBalanceMismatch)?;

    let kernel_excesses: Vec<Commitment> = tx
        .body
        .kernels
        .iter()
        .map(|kernel| kernel.kernel.excess)
        .collect();

    let actual = Commitment::sum(&kernel_excesses)
        .ok_or(CutThroughBuildError::InternalValueBalanceMismatch)?;

    if expected != actual {
        return Err(CutThroughBuildError::InternalValueBalanceMismatch);
    }

    // -------------------------------------------------
    // Self-check 3: complete cut-through authorization.
    // -------------------------------------------------

    let canonical_kos: Vec<[u8; 32]> = tx
        .body
        .inputs
        .iter()
        .map(|input| {
            *ko_by_commit
                .get(&input.commit.0)
                .expect("builder retained Ko for every input")
        })
        .collect();

    let sender_authorizations = tx
        .body
        .outputs
        .iter()
        .map(|output| output.sender_authorization)
        .collect();

    let input_authorizations = tx
        .body
        .inputs
        .iter()
        .map(|input| input.authorization)
        .collect();

    let kernel_bindings = tx
        .body
        .kernels
        .iter()
        .filter_map(|kernel| kernel.stealth_binding)
        .collect();

    let bound_kernels = tx
        .body
        .kernels
        .iter()
        .filter(|kernel| kernel.stealth_binding.is_some())
        .map(|kernel| kernel.kernel.clone())
        .collect::<Vec<_>>();

    let raw_outputs = tx
        .body
        .outputs
        .iter()
        .map(|output| output.output.clone())
        .collect::<Vec<_>>();

    let bundle = KernelBoundAuthorizationBundleV1 {
        sender_authorizations,
        input_authorizations,
        kernel_bindings,

        stealth_offset: tx.body.stealth_offset,
    };

    bundle.validate(&raw_outputs, &input_commits, &canonical_kos, &bound_kernels)?;

    Ok(tx)
}

/// Failure during stateful validation of a v3 cut-through candidate.
///
/// This validator is intentionally separate from the active v2 mempool and
/// block-validation paths.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughStateError {
    #[error(transparent)]
    Shape(#[from] CutThroughV1Error),

    #[error("coinbase material is not accepted by the transfer validator")]
    CoinbaseInTransfer,

    #[error("input {commit} does not exist in authoritative UTXO state")]
    UnknownInput { commit: String },

    #[error("immature coinbase: created at {created}, spend attempted at {now}")]
    ImmatureCoinbaseSpend { created: u64, now: u64 },

    #[error("output commitment already exists in authoritative UTXO state")]
    OutputAlreadyExists,

    #[error("invalid range proof at output index {index}")]
    BadRangeProof { index: usize },

    #[error("kernel locked until height {until}")]
    KernelLocked { until: u64 },

    #[error("malformed value-balance material")]
    MalformedValueBalance,

    #[error("MW value equation does not balance")]
    UnbalancedTransaction,

    #[error("stealth authorization validation failed: {0}")]
    StealthAuthorization(KernelBoundAuthorizationBundleError),
}

impl CutThroughBodyV1 {
    pub fn total_fee(&self) -> u64 {
        self.kernels
            .iter()
            .map(|kernel| kernel.kernel.fee_darks)
            .sum()
    }

    pub fn total_reward(&self) -> u64 {
        self.kernels
            .iter()
            .map(|kernel| kernel.kernel.reward_darks)
            .sum()
    }
}

impl LedgerState {
    /// Validate a v3 cut-through transfer candidate against authoritative
    /// ledger state without mutating the state.
    ///
    /// Checks performed here:
    ///
    /// 1. canonical candidate shape/encoding
    /// 2. no coinbase material in the transfer path
    /// 3. kernel lock heights
    /// 4. output range proofs and UTXO collisions
    /// 5. authoritative input lookup and coinbase maturity
    /// 6. input authorization against state-derived canonical Ko
    /// 7. ordinary Mimblewimble value balance
    /// 8. Ks/Ki/Ko/E'/x' stealth balance
    /// 9. every retained E' is bound to its concrete kernel
    ///
    /// This method does NOT mutate the UTXO set, kernel accumulator, supply,
    /// height or transaction counter.
    pub fn check_cutthrough_v1_acceptable(
        &self,
        tx: &CutThroughTransactionV1,
        next_height: Height,
        ctx: &[u8],
    ) -> Result<(), CutThroughStateError> {
        tx.check_shape()?;

        // This method models the mempool/ordinary-transfer path.
        //
        // Coinbase construction will receive its own explicit v3 path rather
        // than weakening the transfer rules.
        if tx
            .body
            .kernels
            .iter()
            .any(|kernel| kernel.kernel.feature == nightfall_crypto::KernelFeature::Coinbase)
            || tx
                .body
                .outputs
                .iter()
                .any(|output| output.output.features.is_coinbase())
        {
            return Err(CutThroughStateError::CoinbaseInTransfer);
        }

        for kernel in &tx.body.kernels {
            if kernel.kernel.lock_height > next_height.0 {
                return Err(CutThroughStateError::KernelLocked {
                    until: kernel.kernel.lock_height,
                });
            }
        }

        // Full output crypto checks.
        //
        // check_shape() already verifies both output signatures and point
        // encodings; range proofs need the network proof context and therefore
        // live here.
        for (index, output) in tx.body.outputs.iter().enumerate() {
            if !nightfall_crypto::rangeproofs::verify(
                &output.output.range_proof,
                &output.output.commit,
                ctx,
            ) {
                return Err(CutThroughStateError::BadRangeProof { index });
            }

            if self.utxos.contains(&output.output.commit) {
                return Err(CutThroughStateError::OutputAlreadyExists);
            }
        }

        // Resolve Ko ONLY from authoritative UTXO state.
        let mut canonical_kos = Vec::with_capacity(tx.body.inputs.len());

        for input in &tx.body.inputs {
            let entry = self.utxos.get(&input.commit).ok_or_else(|| {
                CutThroughStateError::UnknownInput {
                    commit: input.commit.to_hex(),
                }
            })?;

            if !is_mature(entry, next_height, self.coinbase_maturity) {
                return Err(CutThroughStateError::ImmatureCoinbaseSpend {
                    created: entry.height,
                    now: next_height.0,
                });
            }

            canonical_kos.push(entry.output_pk);
        }

        // Preserve Nightfall's existing inflation-resistance equation.
        let input_commits: Vec<Commitment> =
            tx.body.inputs.iter().map(|input| input.commit).collect();

        let output_commits: Vec<Commitment> = tx
            .body
            .outputs
            .iter()
            .map(|output| output.output.commit)
            .collect();

        let expected = nightfall_crypto::expected_excess(
            &input_commits,
            &output_commits,
            tx.body.total_fee(),
            tx.body.total_reward(),
        )
        .ok_or(CutThroughStateError::MalformedValueBalance)?;

        let kernel_excesses: Vec<Commitment> = tx
            .body
            .kernels
            .iter()
            .map(|kernel| kernel.kernel.excess)
            .collect();

        let kernel_sum =
            Commitment::sum(&kernel_excesses).ok_or(CutThroughStateError::MalformedValueBalance)?;

        if expected != kernel_sum {
            return Err(CutThroughStateError::UnbalancedTransaction);
        }

        // Only kernels actually carrying E' participate in the kernel-bound
        // stealth bundle. Kernels without E' remain ordinary MW kernels.
        let mut bindings = Vec::new();

        let mut bound_kernels = Vec::new();

        for kernel in &tx.body.kernels {
            if let Some(binding) = kernel.stealth_binding {
                bindings.push(binding);
                bound_kernels.push(kernel.kernel.clone());
            }
        }

        let sender_authorizations = tx
            .body
            .outputs
            .iter()
            .map(|output| output.sender_authorization)
            .collect();

        let input_authorizations = tx
            .body
            .inputs
            .iter()
            .map(|input| input.authorization)
            .collect();

        let outputs: Vec<nightfall_crypto::Output> = tx
            .body
            .outputs
            .iter()
            .map(|output| output.output.clone())
            .collect();

        let bundle = KernelBoundAuthorizationBundleV1 {
            sender_authorizations,
            input_authorizations,
            kernel_bindings: bindings,
            stealth_offset: tx.body.stealth_offset,
        };

        bundle
            .validate(&outputs, &input_commits, &canonical_kos, &bound_kernels)
            .map_err(CutThroughStateError::StealthAuthorization)?;

        Ok(())
    }
}

impl CutThroughTransactionV1 {
    /// Construct a canonically ordered v3 candidate.
    pub fn new(mut body: CutThroughBodyV1) -> Self {
        body.canonicalise();

        Self {
            version: CUTTHROUGH_TX_VERSION,
            body,
        }
    }

    pub fn check_shape(&self) -> Result<(), CutThroughV1Error> {
        if self.version != CUTTHROUGH_TX_VERSION {
            return Err(CutThroughV1Error::WrongVersion {
                got: self.version,
                expected: CUTTHROUGH_TX_VERSION,
            });
        }

        self.body.check_shape()
    }

    /// Candidate txid covering the explicit version and complete candidate
    /// body hash.
    pub fn txid(&self) -> Hash256 {
        let body_hash = self.body.hash();

        hash_multi(
            CUTTHROUGH_TX_DOMAIN,
            &[&self.version.to_le_bytes(), &body_hash.0],
        )
    }
}

impl CutThroughBodyV1 {
    /// Canonical ordering mirrors Nightfall's current aggregate philosophy:
    ///
    /// * inputs by spent commitment
    /// * outputs by output commitment
    /// * kernels by stable kernel id
    pub fn canonicalise(&mut self) {
        self.inputs.sort_by_key(|input| input.commit.0);

        self.outputs.sort_by_key(|output| output.output.commit.0);

        self.kernels.sort_by_key(|kernel| kernel.kernel.id().0);
    }

    pub fn is_canonical(&self) -> bool {
        let mut copy = self.clone();
        copy.canonicalise();
        copy == *self
    }

    /// Structural/encoding checks only.
    ///
    /// This intentionally does NOT perform authoritative Ko lookup, stealth
    /// balance, MW value balance or retention-horizon rules.
    pub fn check_shape(&self) -> Result<(), CutThroughV1Error> {
        if self.inputs.len() > MAX_INPUTS {
            return Err(CutThroughV1Error::TooManyInputs);
        }

        if self.outputs.len() > MAX_OUTPUTS {
            return Err(CutThroughV1Error::TooManyOutputs);
        }

        if self.kernels.is_empty() || self.kernels.len() > MAX_KERNELS {
            return Err(CutThroughV1Error::BadKernelCount);
        }

        if self.outputs.is_empty() {
            return Err(CutThroughV1Error::NoOutputs);
        }

        if self
            .kernels
            .iter()
            .try_fold(0u64, |sum, kernel| sum.checked_add(kernel.kernel.fee_darks))
            .is_none()
            || self
                .kernels
                .iter()
                .try_fold(0u64, |sum, kernel| {
                    sum.checked_add(kernel.kernel.reward_darks)
                })
                .is_none()
        {
            return Err(CutThroughV1Error::KernelAmountOverflow);
        }

        if !self.stealth_offset.is_canonical() {
            return Err(CutThroughV1Error::NonCanonicalStealthOffset);
        }

        let mut seen_inputs = BTreeSet::new();

        for (index, input) in self.inputs.iter().enumerate() {
            if input.commit.point().is_none() {
                return Err(CutThroughV1Error::MalformedInputCommitment { index });
            }

            if !seen_inputs.insert(input.commit.0) {
                return Err(CutThroughV1Error::DuplicateInput);
            }

            if CompressedRistretto(input.authorization.ki)
                .decompress()
                .is_none()
            {
                return Err(CutThroughV1Error::MalformedInputKey { index });
            }

            if CompressedRistretto(input.authorization.signature.r)
                .decompress()
                .is_none()
                || Option::<Scalar>::from(Scalar::from_canonical_bytes(
                    input.authorization.signature.s,
                ))
                .is_none()
            {
                return Err(CutThroughV1Error::MalformedInputSignature { index });
            }
        }

        let mut seen_outputs = BTreeSet::new();

        for (index, output) in self.outputs.iter().enumerate() {
            if output.output.commit.point().is_none() || output.output.output_point().is_none() {
                return Err(CutThroughV1Error::MalformedOutput { index });
            }

            if !seen_outputs.insert(output.output.commit.0) {
                return Err(CutThroughV1Error::DuplicateOutput);
            }

            if !output.output.verify_sender_sig() {
                return Err(CutThroughV1Error::InvalidExistingOutputSignature { index });
            }

            if !output.sender_authorization.verify(&output.output) {
                return Err(CutThroughV1Error::InvalidSenderAuthorization { index });
            }
        }

        let mut seen_kernels = BTreeSet::new();

        for (index, kernel) in self.kernels.iter().enumerate() {
            if kernel.kernel.check_shape().is_err() || !kernel.kernel.verify_signature() {
                return Err(CutThroughV1Error::InvalidKernel { index });
            }

            if !seen_kernels.insert(kernel.kernel.id().0) {
                return Err(CutThroughV1Error::DuplicateKernel);
            }

            if let Some(binding) = &kernel.stealth_binding {
                if !binding.verify(&kernel.kernel) {
                    return Err(CutThroughV1Error::InvalidKernelStealthBinding { index });
                }
            }
        }

        if !self.is_canonical() {
            return Err(CutThroughV1Error::NonCanonicalBody);
        }

        Ok(())
    }

    /// Hash every consensus-candidate byte explicitly.
    ///
    /// This uses its own domain and does not modify the active BlockBody hash.
    pub fn hash(&self) -> Hash256 {
        let mut parts: Vec<Vec<u8>> = Vec::new();

        for input in &self.inputs {
            let mut bytes = b"input".to_vec();

            bytes.extend_from_slice(&input.commit.0);

            bytes.extend_from_slice(&input.authorization.canonical_bytes());

            parts.push(bytes);
        }

        for output in &self.outputs {
            let mut bytes = b"output".to_vec();

            bytes.extend_from_slice(&output.output.commitment_bytes());

            bytes.extend_from_slice(&output.output.sender_sig.r);

            bytes.extend_from_slice(&output.output.sender_sig.s);

            bytes.extend_from_slice(&output.sender_authorization.canonical_bytes());

            parts.push(bytes);
        }

        for kernel in &self.kernels {
            let mut bytes = b"kernel".to_vec();

            bytes.extend_from_slice(&kernel.kernel.signing_message());

            bytes.extend_from_slice(&kernel.kernel.excess_sig.r);

            bytes.extend_from_slice(&kernel.kernel.excess_sig.s);

            match &kernel.stealth_binding {
                None => {
                    bytes.push(0);
                }

                Some(binding) => {
                    bytes.push(1);

                    bytes.extend_from_slice(&binding.canonical_bytes());
                }
            }

            parts.push(bytes);
        }

        let mut offset = b"stealth_offset".to_vec();

        offset.extend_from_slice(&self.stealth_offset.canonical_bytes());

        parts.push(offset);

        let refs: Vec<&[u8]> = parts.iter().map(|part| part.as_slice()).collect();

        hash_multi(CUTTHROUGH_BODY_DOMAIN, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use curve25519_dalek::scalar::Scalar;
    use nightfall_crypto::{
        build_kernel, create_output, generator_g, scan_output, KernelFeature, WalletKeys,
    };
    use nightfall_types::NetworkId;
    use rand::rngs::OsRng;

    use crate::{
        KernelStealthBindingV1, SenderAuthorizationV1, SpendAuthorizationV1, StealthExcessV1,
        StealthOffsetV1,
    };

    fn candidate_parts() -> (CutThroughInputV1, CutThroughOutputV1, CutThroughKernelV1) {
        let owner = WalletKeys::generate();

        let (spent_output, _) = create_output(
            &owner.address(),
            10_000,
            "spent",
            NetworkId::Devnet.proof_context(),
        )
        .expect("spent output");

        let discovered =
            scan_output(&owner.view_key(), &spent_output).expect("discover spent output");

        let ko_secret = discovered.spend_secret(&owner);

        assert_eq!(
            (generator_g() * ko_secret).compress().to_bytes(),
            spent_output.output_pk,
        );

        let ki_secret = Scalar::random(&mut OsRng);

        let input_auth = SpendAuthorizationV1::sign(
            &spent_output.commit,
            &ki_secret,
            &ko_secret,
            &spent_output.output_pk,
        )
        .expect("input auth");

        let input = CutThroughInputV1 {
            commit: spent_output.commit,
            authorization: input_auth,
        };

        let receiver = WalletKeys::generate();

        let (new_output, _) = create_output(
            &receiver.address(),
            9_000,
            "new",
            NetworkId::Devnet.proof_context(),
        )
        .expect("new output");

        let ks = Scalar::random(&mut OsRng);

        let sender_authorization = SenderAuthorizationV1::sign(&new_output, &ks);

        let output = CutThroughOutputV1 {
            output: new_output,
            sender_authorization,
        };

        let kernel_secret = Scalar::random(&mut OsRng);

        let kernel = build_kernel(KernelFeature::Plain, 1_000, 0, 0, &kernel_secret);

        let stealth_secret = Scalar::random(&mut OsRng);

        let stealth_excess =
            StealthExcessV1::new(Commitment::from_point(generator_g() * stealth_secret))
                .expect("stealth excess");

        let stealth_binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("kernel stealth binding");

        let kernel = CutThroughKernelV1 {
            kernel,
            stealth_binding: Some(stealth_binding),
        };

        (input, output, kernel)
    }

    fn candidate_transaction() -> CutThroughTransactionV1 {
        let (input, output, kernel) = candidate_parts();

        CutThroughTransactionV1::new(CutThroughBodyV1 {
            inputs: vec![input],
            outputs: vec![output],
            kernels: vec![kernel],

            // Phase 3A checks encoding/canonicality only.
            stealth_offset: StealthOffsetV1::from_scalar(&Scalar::random(&mut OsRng)),
        })
    }

    #[test]
    fn cutthrough_version_is_separate_from_active_v2() {
        assert_eq!(CUTTHROUGH_TX_VERSION, TX_VERSION + 1,);

        assert_ne!(CUTTHROUGH_TX_VERSION, TX_VERSION,);
    }

    #[test]
    fn candidate_transaction_has_valid_shape() {
        let tx = candidate_transaction();

        assert_eq!(tx.check_shape(), Ok(()),);

        assert!(tx.body.is_canonical());
    }

    #[test]
    fn candidate_transaction_roundtrips_exactly() {
        let tx = candidate_transaction();

        let before_txid = tx.txid();

        let encoded = serde_json::to_vec(&tx).expect("serialize v3 candidate");

        let decoded: CutThroughTransactionV1 =
            serde_json::from_slice(&encoded).expect("deserialize v3 candidate");

        assert_eq!(decoded, tx,);

        assert_eq!(decoded.txid(), before_txid,);

        assert_eq!(decoded.check_shape(), Ok(()),);
    }

    #[test]
    fn canonical_order_does_not_leak_grouping() {
        let (input_a, output_a, kernel_a) = candidate_parts();

        let (input_b, output_b, kernel_b) = candidate_parts();

        let offset = StealthOffsetV1::from_scalar(&Scalar::from(7u64));

        let mut a = CutThroughBodyV1 {
            inputs: vec![input_a.clone(), input_b.clone()],

            outputs: vec![output_a.clone(), output_b.clone()],

            kernels: vec![kernel_a.clone(), kernel_b.clone()],

            stealth_offset: offset,
        };

        let mut b = CutThroughBodyV1 {
            inputs: vec![input_b, input_a],

            outputs: vec![output_b, output_a],

            kernels: vec![kernel_b, kernel_a],

            stealth_offset: offset,
        };

        a.canonicalise();
        b.canonicalise();

        assert_eq!(a, b);
        assert_eq!(a.hash(), b.hash());

        assert_eq!(a.check_shape(), Ok(()),);
    }

    #[test]
    fn candidate_hash_binds_stealth_offset() {
        let tx = candidate_transaction();

        let mut changed = tx.clone();

        let old = changed
            .body
            .stealth_offset
            .scalar()
            .expect("canonical offset");

        changed.body.stealth_offset = StealthOffsetV1::from_scalar(&(old + Scalar::ONE));

        assert_ne!(tx.body.hash(), changed.body.hash(),);

        assert_ne!(tx.txid(), changed.txid(),);
    }

    #[test]
    fn candidate_hash_binds_kernel_stealth_proof() {
        let tx = candidate_transaction();

        let mut changed = tx.clone();

        changed.body.kernels[0]
            .stealth_binding
            .as_mut()
            .expect("binding")
            .signature
            .s[0] ^= 1;

        assert_ne!(tx.body.hash(), changed.body.hash(),);

        assert!(matches!(
            changed.check_shape(),
            Err(CutThroughV1Error::InvalidKernelStealthBinding { index: 0 })
        ));
    }

    #[test]
    fn malformed_input_authorization_fails_closed() {
        let mut tx = candidate_transaction();

        tx.body.inputs[0].authorization.ki = [0xff; 32];

        assert_eq!(
            tx.check_shape(),
            Err(CutThroughV1Error::MalformedInputKey { index: 0 }),
        );
    }

    #[test]
    fn noncanonical_offset_fails_closed() {
        let mut tx = candidate_transaction();

        tx.body.stealth_offset = StealthOffsetV1 { bytes: [0xff; 32] };

        assert_eq!(
            tx.check_shape(),
            Err(CutThroughV1Error::NonCanonicalStealthOffset),
        );
    }

    #[test]
    fn wrong_candidate_version_is_rejected() {
        let mut tx = candidate_transaction();

        tx.version = TX_VERSION;

        assert_eq!(
            tx.check_shape(),
            Err(CutThroughV1Error::WrongVersion {
                got: TX_VERSION,
                expected: CUTTHROUGH_TX_VERSION,
            }),
        );
    }

    #[test]
    fn sender_authorization_tampering_is_rejected() {
        let mut tx = candidate_transaction();

        tx.body.outputs[0].sender_authorization.key = (generator_g() * Scalar::random(&mut OsRng))
            .compress()
            .to_bytes();

        assert_eq!(
            tx.check_shape(),
            Err(CutThroughV1Error::InvalidSenderAuthorization { index: 0 }),
        );
    }

    fn stateful_candidate_fixture() -> (LedgerState, CutThroughTransactionV1, Height) {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let (spent_output, spent_sender_secrets) =
            create_output(&owner.address(), 10_000, "stateful-input", ctx).expect("spent output");

        let discovered =
            scan_output(&owner.view_key(), &spent_output).expect("discover spent output");

        let ko_secret = discovered.spend_secret(&owner);

        assert_eq!(
            (generator_g() * ko_secret).compress().to_bytes(),
            spent_output.output_pk,
        );

        let mut state = LedgerState::for_network(NetworkId::Devnet);

        assert!(state.utxos.insert(
            spent_output.commit,
            crate::UtxoEntry {
                output_pk: spent_output.output_pk,
                height: 0,
                is_coinbase: false,
            },
        ));

        let receiver = WalletKeys::generate();

        let (new_output, new_sender_secrets) =
            create_output(&receiver.address(), 9_000, "stateful-output", ctx).expect("new output");

        let fee = 1_000u64;

        // Same MW relation used by the active builder:
        //
        // Σout - Σin + fee*G = excess*H
        let kernel_secret = new_sender_secrets.blind - spent_sender_secrets.blind;

        let kernel = build_kernel(KernelFeature::Plain, fee, 0, 0, &kernel_secret);

        assert!(kernel.verify_signature());

        let ki_secret = Scalar::random(&mut OsRng);

        let input_authorization = SpendAuthorizationV1::sign(
            &spent_output.commit,
            &ki_secret,
            &ko_secret,
            &spent_output.output_pk,
        )
        .expect("input authorization");

        let ks_secret = Scalar::random(&mut OsRng);

        let sender_authorization = SenderAuthorizationV1::sign(&new_output, &ks_secret);

        let stealth_excess_secret = Scalar::random(&mut OsRng);

        let stealth_excess = StealthExcessV1::new(Commitment::from_point(
            generator_g() * stealth_excess_secret,
        ))
        .expect("stealth excess");

        let stealth_binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("kernel/E' binding");

        let stealth_offset = crate::stealth_offset_secret(
            &[ks_secret],
            &[ki_secret],
            &[ko_secret],
            &[stealth_excess_secret],
        );

        let tx = CutThroughTransactionV1::new(CutThroughBodyV1 {
            inputs: vec![CutThroughInputV1 {
                commit: spent_output.commit,

                authorization: input_authorization,
            }],

            outputs: vec![CutThroughOutputV1 {
                output: new_output,

                sender_authorization,
            }],

            kernels: vec![CutThroughKernelV1 {
                kernel,

                stealth_binding: Some(stealth_binding),
            }],

            stealth_offset: StealthOffsetV1::from_scalar(&stealth_offset),
        });

        (state, tx, Height(1))
    }

    #[test]
    fn stateful_candidate_validates_against_real_utxo_state() {
        let (state, tx, next_height) = stateful_candidate_fixture();

        assert_eq!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Ok(()),
        );
    }

    #[test]
    fn stateful_candidate_uses_authoritative_ko() {
        let (mut state, tx, next_height) = stateful_candidate_fixture();

        let input_commit = tx.body.inputs[0].commit;

        let replacement_secret = Scalar::random(&mut OsRng);

        state
            .utxos
            .entries
            .get_mut(&input_commit.0)
            .expect("UTXO")
            .output_pk = (generator_g() * replacement_secret).compress().to_bytes();

        assert!(matches!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::StealthAuthorization(
                crate::KernelBoundAuthorizationBundleError::Authorization(
                    crate::AuthorizationBundleError::InvalidInputAuthorization { index: 0 }
                )
            ))
        ));
    }

    #[test]
    fn stateful_candidate_rejects_unknown_input() {
        let (mut state, tx, next_height) = stateful_candidate_fixture();

        let commit = tx.body.inputs[0].commit;

        state.utxos.remove(&commit);

        assert!(matches!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::UnknownInput { .. })
        ));
    }

    #[test]
    fn stateful_candidate_preserves_coinbase_maturity() {
        let (mut state, tx, _next_height) = stateful_candidate_fixture();

        let commit = tx.body.inputs[0].commit;

        let entry = state.utxos.entries.get_mut(&commit.0).expect("UTXO");

        entry.is_coinbase = true;
        entry.height = 5;

        assert_eq!(
            state
                .check_cutthrough_v1_acceptable(&tx, Height(6), NetworkId::Devnet.proof_context(),),
            Err(CutThroughStateError::ImmatureCoinbaseSpend { created: 5, now: 6 }),
        );
    }

    #[test]
    fn stateful_candidate_verifies_rangeproofs() {
        let (state, mut tx, next_height) = stateful_candidate_fixture();

        assert!(!tx.body.outputs[0].output.range_proof.0.is_empty());

        tx.body.outputs[0].output.range_proof.0[0] ^= 1;

        // Existing output signature covers the range proof as well, so shape
        // validation may reject first. Either outcome is fail-closed.
        assert!(state
            .check_cutthrough_v1_acceptable(&tx, next_height, NetworkId::Devnet.proof_context(),)
            .is_err());
    }

    #[test]
    fn stateful_candidate_rejects_mw_value_imbalance() {
        let (state, mut tx, next_height) = stateful_candidate_fixture();

        let old_binding = tx.body.kernels[0].stealth_binding.expect("binding");

        let wrong_kernel_secret = Scalar::random(&mut OsRng);

        let wrong_kernel = build_kernel(KernelFeature::Plain, 1_000, 0, 0, &wrong_kernel_secret);

        let rebound = KernelStealthBindingV1::sign(
            &wrong_kernel,
            &wrong_kernel_secret,
            old_binding.stealth_excess,
        )
        .expect("rebound E'");

        tx.body.kernels[0] = CutThroughKernelV1 {
            kernel: wrong_kernel,
            stealth_binding: Some(rebound),
        };

        tx.body.canonicalise();

        assert_eq!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::UnbalancedTransaction),
        );
    }

    #[test]
    fn stateful_candidate_rejects_stealth_imbalance() {
        let (state, mut tx, next_height) = stateful_candidate_fixture();

        let old = tx.body.stealth_offset.scalar().expect("canonical");

        tx.body.stealth_offset = StealthOffsetV1::from_scalar(&(old + Scalar::ONE));

        assert!(matches!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::StealthAuthorization(
                crate::KernelBoundAuthorizationBundleError::Authorization(
                    crate::AuthorizationBundleError::StealthBalanceMismatch
                )
            ))
        ));
    }

    #[test]
    fn stateful_candidate_rejects_output_collision() {
        let (mut state, tx, next_height) = stateful_candidate_fixture();

        let commit = tx.body.outputs[0].output.commit;

        state.utxos.insert(
            commit,
            crate::UtxoEntry {
                output_pk: tx.body.outputs[0].output.output_pk,

                height: 0,
                is_coinbase: false,
            },
        );

        assert_eq!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::OutputAlreadyExists),
        );
    }

    #[test]
    fn stateful_candidate_rejects_future_kernel_lock() {
        let (state, mut tx, next_height) = stateful_candidate_fixture();

        let old_kernel = tx.body.kernels[0].kernel.clone();

        let old_binding = tx.body.kernels[0].stealth_binding.expect("binding");

        // We cannot mutate a signed kernel field and retain validity.
        // Rebuild one with the same E' but a future lock height.
        //
        // Its value balance secret is deliberately unrelated here because
        // lock-height rejection must happen first.
        let secret = Scalar::random(&mut OsRng);

        let locked_kernel = build_kernel(
            KernelFeature::Plain,
            old_kernel.fee_darks,
            0,
            next_height.0 + 10,
            &secret,
        );

        let binding =
            KernelStealthBindingV1::sign(&locked_kernel, &secret, old_binding.stealth_excess)
                .expect("locked binding");

        tx.body.kernels[0] = CutThroughKernelV1 {
            kernel: locked_kernel,
            stealth_binding: Some(binding),
        };

        tx.body.canonicalise();

        assert_eq!(
            state.check_cutthrough_v1_acceptable(
                &tx,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Err(CutThroughStateError::KernelLocked {
                until: next_height.0 + 10,
            }),
        );
    }

    #[test]
    fn stateful_validation_never_mutates_ledger_state() {
        let (state, mut tx, next_height) = stateful_candidate_fixture();

        let root_before = state.utxo_root();

        let kernel_before = state.kernel_sum();

        let height_before = state.height;

        let tx_count_before = state.tx_count;

        let minted_before = state.supply.total_minted_darks;

        let burned_before = state.supply.total_burned_darks;

        let old = tx.body.stealth_offset.scalar().expect("canonical");

        tx.body.stealth_offset = StealthOffsetV1::from_scalar(&(old + Scalar::ONE));

        assert!(state
            .check_cutthrough_v1_acceptable(&tx, next_height, NetworkId::Devnet.proof_context(),)
            .is_err());

        assert_eq!(state.utxo_root(), root_before,);

        assert_eq!(state.kernel_sum(), kernel_before,);

        assert_eq!(state.height, height_before,);

        assert_eq!(state.tx_count, tx_count_before,);

        assert_eq!(state.supply.total_minted_darks, minted_before,);

        assert_eq!(state.supply.total_burned_darks, burned_before,);
    }

    fn builder_spendable(
        owner: &WalletKeys,
        value: u64,
        memo: &str,
        ctx: &[u8],
    ) -> (nightfall_crypto::Output, crate::Spendable) {
        let (output, secrets) =
            create_output(&owner.address(), value, memo, ctx).expect("source output");

        let discovered = scan_output(&owner.view_key(), &output).expect("discover source output");

        let spendable = crate::Spendable {
            commit: output.commit,

            value,

            blind: secrets.blind,

            spend_secret: discovered.spend_secret(owner),
        };

        (output, spendable)
    }

    #[test]
    fn builder_constructs_state_valid_candidate() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver = WalletKeys::generate();

        let (source, spendable) = builder_spendable(&owner, 20_000, "source", ctx);

        let mut state = LedgerState::for_network(NetworkId::Devnet);

        assert!(state.utxos.insert(
            source.commit,
            crate::UtxoEntry {
                output_pk: source.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        let payments = vec![crate::Payment {
            to: receiver.address(),

            amount: 7_000,

            memo: "payment".into(),
        }];

        let tx = build_cutthrough_transfer_v1(
            &owner,
            &[spendable],
            &payments,
            1_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("build v3 transfer");

        assert_eq!(tx.version, CUTTHROUGH_TX_VERSION,);

        assert_eq!(tx.body.inputs.len(), 1,);

        assert_eq!(tx.body.outputs.len(), 2,);

        assert_eq!(tx.body.kernels.len(), 1,);

        assert!(tx.body.kernels[0].stealth_binding.is_some());

        assert_eq!(
            state.check_cutthrough_v1_acceptable(&tx, Height(1), ctx,),
            Ok(()),
        );

        let received = tx
            .body
            .outputs
            .iter()
            .find_map(|output| scan_output(&receiver.view_key(), &output.output))
            .expect("receiver payment");

        assert_eq!(received.value, 7_000,);

        let change = tx
            .body
            .outputs
            .iter()
            .filter_map(|output| scan_output(&owner.view_key(), &output.output))
            .find(|output| output.value == 12_000)
            .expect("change output");

        assert_eq!(change.value, 12_000,);
    }

    #[test]
    fn builder_handles_multiple_inputs_and_outputs() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver_a = WalletKeys::generate();

        let receiver_b = WalletKeys::generate();

        let (source_a, spendable_a) = builder_spendable(&owner, 11_000, "a", ctx);

        let (source_b, spendable_b) = builder_spendable(&owner, 14_000, "b", ctx);

        let mut state = LedgerState::for_network(NetworkId::Devnet);

        for source in [&source_a, &source_b] {
            assert!(state.utxos.insert(
                source.commit,
                crate::UtxoEntry {
                    output_pk: source.output_pk,

                    height: 0,

                    is_coinbase: false,
                },
            ));
        }

        let payments = vec![
            crate::Payment {
                to: receiver_a.address(),

                amount: 5_000,

                memo: "A".into(),
            },
            crate::Payment {
                to: receiver_b.address(),

                amount: 8_000,

                memo: "B".into(),
            },
        ];

        let tx = build_cutthrough_transfer_v1(
            &owner,
            &[spendable_a, spendable_b],
            &payments,
            2_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("multi-input v3 transfer");

        assert_eq!(tx.body.inputs.len(), 2,);

        // Two payments + mandatory change.
        assert_eq!(tx.body.outputs.len(), 3,);

        assert_eq!(
            state.check_cutthrough_v1_acceptable(&tx, Height(1), ctx,),
            Ok(()),
        );

        let a = tx
            .body
            .outputs
            .iter()
            .find_map(|output| scan_output(&receiver_a.view_key(), &output.output))
            .expect("receiver A");

        let b = tx
            .body
            .outputs
            .iter()
            .find_map(|output| scan_output(&receiver_b.view_key(), &output.output))
            .expect("receiver B");

        assert_eq!(a.value, 5_000);

        assert_eq!(b.value, 8_000);

        // 25,000 - 13,000 - 2,000 = 10,000.
        let change = tx
            .body
            .outputs
            .iter()
            .filter_map(|output| scan_output(&owner.view_key(), &output.output))
            .find(|output| output.value == 10_000)
            .expect("multi-input change");

        assert_eq!(change.value, 10_000,);
    }

    #[test]
    fn builder_rejects_no_inputs() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        assert_eq!(
            build_cutthrough_transfer_v1(&owner, &[], &[], 0, &owner.address(), 0, ctx,),
            Err(CutThroughBuildError::NoInputs),
        );
    }

    #[test]
    fn builder_rejects_insufficient_funds() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver = WalletKeys::generate();

        let (_source, spendable) = builder_spendable(&owner, 5_000, "", ctx);

        let payments = vec![crate::Payment {
            to: receiver.address(),

            amount: 5_000,

            memo: String::new(),
        }];

        assert_eq!(
            build_cutthrough_transfer_v1(
                &owner,
                &[spendable],
                &payments,
                1,
                &owner.address(),
                0,
                ctx,
            ),
            Err(CutThroughBuildError::InsufficientFunds {
                have: 5_000,
                need: 5_001,
            }),
        );
    }

    #[test]
    fn builder_rejects_wrong_spendable_blind() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let (_source, mut spendable) = builder_spendable(&owner, 10_000, "", ctx);

        spendable.blind += Scalar::ONE;

        assert_eq!(
            build_cutthrough_transfer_v1(
                &owner,
                &[spendable],
                &[],
                1_000,
                &owner.address(),
                0,
                ctx,
            ),
            Err(CutThroughBuildError::SpendableCommitmentMismatch { index: 0 }),
        );
    }

    #[test]
    fn builder_produces_fresh_authorization_material() {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver = WalletKeys::generate();

        let (_source, spendable) = builder_spendable(&owner, 10_000, "", ctx);

        let payments = vec![crate::Payment {
            to: receiver.address(),

            amount: 5_000,

            memo: String::new(),
        }];

        let first = build_cutthrough_transfer_v1(
            &owner,
            std::slice::from_ref(&spendable),
            &payments,
            1_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("first build");

        let second = build_cutthrough_transfer_v1(
            &owner,
            &[spendable],
            &payments,
            1_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("second build");

        assert_ne!(first.txid(), second.txid(),);

        assert_ne!(
            first.body.inputs[0].authorization.ki,
            second.body.inputs[0].authorization.ki,
        );
    }

    fn aggregate_candidate_fixture() -> (LedgerState, Vec<CutThroughTransactionV1>, Height) {
        let ctx = NetworkId::Devnet.proof_context();

        let owner_a = WalletKeys::generate();

        let owner_b = WalletKeys::generate();

        let receiver_a = WalletKeys::generate();

        let receiver_b = WalletKeys::generate();

        let (source_a, spendable_a) =
            builder_spendable(&owner_a, 15_000, "aggregate-source-a", ctx);

        let (source_b, spendable_b) =
            builder_spendable(&owner_b, 18_000, "aggregate-source-b", ctx);

        let mut state = LedgerState::for_network(NetworkId::Devnet);

        assert!(state.utxos.insert(
            source_a.commit,
            crate::UtxoEntry {
                output_pk: source_a.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        assert!(state.utxos.insert(
            source_b.commit,
            crate::UtxoEntry {
                output_pk: source_b.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        let tx_a = build_cutthrough_transfer_v1(
            &owner_a,
            &[spendable_a],
            &[crate::Payment {
                to: receiver_a.address(),

                amount: 4_000,

                memo: "aggregate-a".into(),
            }],
            1_000,
            &owner_a.address(),
            0,
            ctx,
        )
        .expect("build aggregate child A");

        let tx_b = build_cutthrough_transfer_v1(
            &owner_b,
            &[spendable_b],
            &[crate::Payment {
                to: receiver_b.address(),

                amount: 5_000,

                memo: "aggregate-b".into(),
            }],
            2_000,
            &owner_b.address(),
            0,
            ctx,
        )
        .expect("build aggregate child B");

        (state, vec![tx_a, tx_b], Height(1))
    }

    #[test]
    fn aggregate_combines_candidates_and_validates() {
        let (state, txs, next_height) = aggregate_candidate_fixture();

        let aggregate = aggregate_cutthrough_v1(&txs).expect("aggregate candidates");

        assert_eq!(aggregate.body.inputs.len(), 2,);

        // Two payments + two change outputs.
        assert_eq!(aggregate.body.outputs.len(), 4,);

        assert_eq!(aggregate.body.kernels.len(), 2,);

        assert_eq!(aggregate.body.total_fee(), 3_000,);

        assert!(aggregate.body.is_canonical());

        assert_eq!(
            state.check_cutthrough_v1_acceptable(
                &aggregate,
                next_height,
                NetworkId::Devnet.proof_context(),
            ),
            Ok(()),
        );
    }

    #[test]
    fn aggregate_order_does_not_reveal_grouping() {
        let (_state, txs, _next_height) = aggregate_candidate_fixture();

        let forward =
            aggregate_cutthrough_v1(&[txs[0].clone(), txs[1].clone()]).expect("forward aggregate");

        let reverse =
            aggregate_cutthrough_v1(&[txs[1].clone(), txs[0].clone()]).expect("reverse aggregate");

        assert_eq!(forward, reverse,);

        assert_eq!(forward.txid(), reverse.txid(),);

        assert_eq!(forward.body.hash(), reverse.body.hash(),);
    }

    #[test]
    fn aggregate_stealth_offset_is_exact_sum() {
        let (_state, txs, _next_height) = aggregate_candidate_fixture();

        let left = txs[0]
            .body
            .stealth_offset
            .scalar()
            .expect("left canonical offset");

        let right = txs[1]
            .body
            .stealth_offset
            .scalar()
            .expect("right canonical offset");

        let aggregate = aggregate_cutthrough_v1(&txs).expect("aggregate");

        assert_eq!(aggregate.body.stealth_offset.scalar(), Some(left + right),);
    }

    #[test]
    fn aggregate_rejects_empty_set() {
        assert_eq!(
            aggregate_cutthrough_v1(&[]),
            Err(CutThroughAggregateError::EmptyAggregate),
        );
    }

    #[test]
    fn aggregate_rejects_invalid_child() {
        let (_state, mut txs, _next_height) = aggregate_candidate_fixture();

        txs[1].version = TX_VERSION;

        assert_eq!(
            aggregate_cutthrough_v1(&txs),
            Err(CutThroughAggregateError::InvalidChild {
                index: 1,

                source: CutThroughV1Error::WrongVersion {
                    got: TX_VERSION,

                    expected: CUTTHROUGH_TX_VERSION,
                },
            }),
        );
    }

    #[test]
    fn aggregate_rejects_duplicate_transaction_material() {
        let (_state, txs, _next_height) = aggregate_candidate_fixture();

        let duplicate = txs[0].clone();

        assert_eq!(
            aggregate_cutthrough_v1(&[duplicate.clone(), duplicate,],),
            Err(CutThroughAggregateError::InvalidAggregate(
                CutThroughV1Error::DuplicateInput
            )),
        );
    }

    #[test]
    fn aggregation_does_not_mutate_sources() {
        let (_state, txs, _next_height) = aggregate_candidate_fixture();

        let before = txs.clone();

        let _aggregate = aggregate_cutthrough_v1(&txs).expect("aggregate");

        assert_eq!(txs, before,);
    }

    #[test]
    fn kernel_amount_overflow_fails_closed() {
        let mut tx = candidate_transaction();

        let first_secret = Scalar::random(&mut OsRng);

        let second_secret = Scalar::random(&mut OsRng);

        let first = build_kernel(KernelFeature::Plain, u64::MAX, 0, 0, &first_secret);

        let second = build_kernel(KernelFeature::Plain, 1, 0, 0, &second_secret);

        tx.body.kernels = vec![
            CutThroughKernelV1 {
                kernel: first,
                stealth_binding: None,
            },
            CutThroughKernelV1 {
                kernel: second,
                stealth_binding: None,
            },
        ];

        tx.body.canonicalise();

        assert_eq!(
            tx.check_shape(),
            Err(CutThroughV1Error::KernelAmountOverflow),
        );
    }
}
