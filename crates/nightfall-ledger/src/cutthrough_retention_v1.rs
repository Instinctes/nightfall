//! Retention/horizon state for the v3 cut-through candidate.
//!
//! This module does NOT prune chain history.
//!
//! It defines the security boundary that must exist before pruning can be
//! implemented:
//!
//! * recent spend authorizations remain available;
//! * canonical `Ko` is captured from authoritative UTXO state;
//! * enough UTXO metadata is retained for later reorg handling;
//! * stealth-excess bindings remain associated with the retained block;
//! * pruning eligibility is deterministic and explicitly height-based.
//!
//! Actual historical-body deletion, horizon snapshots and reorg integration
//! belong to later phases.

use std::collections::{BTreeMap, BTreeSet};

use nightfall_crypto::{hash_multi, Commitment};

use nightfall_types::{Hash256, Height};

use serde::{Deserialize, Serialize};

use crate::{
    CutThroughTransactionV1, CutThroughV1Error, KernelStealthBindingV1, LedgerState,
    SpendAuthorizationV1, UtxoEntry,
};

/// Domain for deterministic retention-state hashing.
pub const CUTTHROUGH_RETENTION_DOMAIN: &[u8] = b"nightfall:cutthrough:retention:v1";

/// Consensus-candidate retention policy.
///
/// `horizon_blocks` is deliberately explicit. No mainnet value is selected
/// here yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughRetentionPolicyV1 {
    pub horizon_blocks: u64,
}

impl CutThroughRetentionPolicyV1 {
    pub fn new(horizon_blocks: u64) -> Result<Self, CutThroughRetentionError> {
        if horizon_blocks == 0 {
            return Err(CutThroughRetentionError::ZeroHorizon);
        }

        Ok(Self { horizon_blocks })
    }

    pub fn validate(&self) -> Result<(), CutThroughRetentionError> {
        if self.horizon_blocks == 0 {
            return Err(CutThroughRetentionError::ZeroHorizon);
        }

        Ok(())
    }

    /// Highest confirmation height that is beyond the retention horizon.
    ///
    /// Example with h=10:
    ///
    /// * tip=109 -> prune through 99
    /// * tip=110 -> prune through 100
    pub fn prune_through_height(&self, tip: Height) -> Option<u64> {
        if tip.0 < self.horizon_blocks {
            None
        } else {
            Some(tip.0 - self.horizon_blocks)
        }
    }

    /// True only after at least `horizon_blocks` blocks have accumulated
    /// after the confirmation height.
    pub fn is_beyond_horizon(&self, confirmed_height: Height, tip: Height) -> bool {
        if tip.0 < confirmed_height.0 {
            return false;
        }

        tip.0 - confirmed_height.0 >= self.horizon_blocks
    }
}

/// Input-side evidence retained while its spend remains inside the horizon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetainedInputAuthorizationV1 {
    pub commit: Commitment,

    /// MUST originate from authoritative UTXO state.
    pub canonical_ko: [u8; 32],

    pub authorization: SpendAuthorizationV1,

    /// Metadata needed to reconstruct the spent UTXO during later reorg work.
    pub created_height: u64,

    pub was_coinbase: bool,
}

impl RetainedInputAuthorizationV1 {
    pub fn restored_utxo_entry(&self) -> UtxoEntry {
        UtxoEntry {
            output_pk: self.canonical_ko,

            height: self.created_height,

            is_coinbase: self.was_coinbase,
        }
    }

    fn validate(
        &self,
        index: usize,
        confirmed_height: u64,
    ) -> Result<(), CutThroughRetentionError> {
        if !self.authorization.verify(&self.commit, &self.canonical_ko) {
            return Err(
                CutThroughRetentionError::InvalidRetainedInputAuthorization {
                    height: confirmed_height,

                    index,
                },
            );
        }

        Ok(())
    }
}

/// Authorization evidence retained for one confirmed v3 aggregate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetainedCutThroughBlockV1 {
    pub confirmed_height: u64,

    /// Hash of the complete candidate body from which this record was made.
    pub body_hash: Hash256,

    pub inputs: Vec<RetainedInputAuthorizationV1>,

    /// E' material must remain available while inside the horizon.
    pub kernel_bindings: Vec<KernelStealthBindingV1>,
}

impl RetainedCutThroughBlockV1 {
    /// Capture retention evidence BEFORE the corresponding inputs disappear
    /// from authoritative UTXO state.
    pub fn capture(
        state: &LedgerState,
        tx: &CutThroughTransactionV1,
        confirmed_height: Height,
    ) -> Result<Self, CutThroughRetentionError> {
        tx.check_shape()
            .map_err(CutThroughRetentionError::Candidate)?;

        let mut retained_inputs = Vec::with_capacity(tx.body.inputs.len());

        for (index, input) in tx.body.inputs.iter().enumerate() {
            let entry = state.utxos.get(&input.commit).ok_or_else(|| {
                CutThroughRetentionError::UnknownInput {
                    commit: input.commit.to_hex(),
                }
            })?;

            if !input.authorization.verify(&input.commit, &entry.output_pk) {
                return Err(
                    CutThroughRetentionError::InvalidRetainedInputAuthorization {
                        height: confirmed_height.0,

                        index,
                    },
                );
            }

            retained_inputs.push(RetainedInputAuthorizationV1 {
                commit: input.commit,

                canonical_ko: entry.output_pk,

                authorization: input.authorization,

                created_height: entry.height,

                was_coinbase: entry.is_coinbase,
            });
        }

        let kernel_bindings = tx
            .body
            .kernels
            .iter()
            .filter_map(|kernel| kernel.stealth_binding)
            .collect();

        let record = Self {
            confirmed_height: confirmed_height.0,

            body_hash: tx.body.hash(),

            inputs: retained_inputs,

            kernel_bindings,
        };

        record.validate()?;

        Ok(record)
    }

    pub fn validate(&self) -> Result<(), CutThroughRetentionError> {
        let mut seen_inputs = BTreeSet::new();

        for (index, input) in self.inputs.iter().enumerate() {
            if !seen_inputs.insert(input.commit.0) {
                return Err(CutThroughRetentionError::DuplicateRetainedSpend {
                    commit: input.commit.to_hex(),
                });
            }

            input.validate(index, self.confirmed_height)?;
        }

        for (index, binding) in self.kernel_bindings.iter().enumerate() {
            if !binding.stealth_excess.is_well_formed() {
                return Err(CutThroughRetentionError::MalformedRetainedStealthExcess {
                    height: self.confirmed_height,

                    index,
                });
            }
        }

        Ok(())
    }
}

/// Deterministic in-memory representation of the authorization horizon.
///
/// This is not yet persisted by `nightfall-storage`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughRetentionWindowV1 {
    policy: CutThroughRetentionPolicyV1,

    blocks: BTreeMap<u64, RetainedCutThroughBlockV1>,
}

impl CutThroughRetentionWindowV1 {
    pub fn new(policy: CutThroughRetentionPolicyV1) -> Result<Self, CutThroughRetentionError> {
        policy.validate()?;

        Ok(Self {
            policy,
            blocks: BTreeMap::new(),
        })
    }

    pub fn policy(&self) -> CutThroughRetentionPolicyV1 {
        self.policy
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn blocks(&self) -> &BTreeMap<u64, RetainedCutThroughBlockV1> {
        &self.blocks
    }

    /// Insert atomically. Existing state is untouched on any error.
    pub fn insert(
        &mut self,
        record: RetainedCutThroughBlockV1,
    ) -> Result<(), CutThroughRetentionError> {
        self.policy.validate()?;
        record.validate()?;

        if self.blocks.contains_key(&record.confirmed_height) {
            return Err(CutThroughRetentionError::DuplicateHeight {
                height: record.confirmed_height,
            });
        }

        let mut existing_spends = BTreeSet::new();

        for block in self.blocks.values() {
            for input in &block.inputs {
                existing_spends.insert(input.commit.0);
            }
        }

        for input in &record.inputs {
            if existing_spends.contains(&input.commit.0) {
                return Err(CutThroughRetentionError::DuplicateRetainedSpend {
                    commit: input.commit.to_hex(),
                });
            }
        }

        self.blocks.insert(record.confirmed_height, record);

        Ok(())
    }

    /// Validate deserialised/reloaded retention state.
    pub fn validate(&self) -> Result<(), CutThroughRetentionError> {
        self.policy.validate()?;

        let mut all_spends = BTreeSet::new();

        for (height, record) in &self.blocks {
            if *height != record.confirmed_height {
                return Err(CutThroughRetentionError::HeightKeyMismatch {
                    key: *height,

                    record: record.confirmed_height,
                });
            }

            record.validate()?;

            for input in &record.inputs {
                if !all_spends.insert(input.commit.0) {
                    return Err(CutThroughRetentionError::DuplicateRetainedSpend {
                        commit: input.commit.to_hex(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Records which have crossed the retention boundary.
    ///
    /// Nothing is deleted here.
    pub fn prunable_heights(&self, tip: Height) -> Vec<u64> {
        self.blocks
            .keys()
            .copied()
            .filter(|height| self.policy.is_beyond_horizon(Height(*height), tip))
            .collect()
    }

    /// Records which MUST still remain available.
    pub fn retained_heights(&self, tip: Height) -> Vec<u64> {
        self.blocks
            .keys()
            .copied()
            .filter(|height| !self.policy.is_beyond_horizon(Height(*height), tip))
            .collect()
    }

    /// Deterministic integrity hash for future storage/restart integration.
    ///
    /// This does not depend on JSON formatting or map insertion order.
    pub fn hash(&self) -> Result<Hash256, CutThroughRetentionError> {
        self.validate()?;

        let mut parts: Vec<Vec<u8>> = Vec::new();

        let mut policy = b"policy".to_vec();

        policy.extend_from_slice(&self.policy.horizon_blocks.to_le_bytes());

        parts.push(policy);

        for (height, record) in &self.blocks {
            let mut bytes = b"block".to_vec();

            bytes.extend_from_slice(&height.to_le_bytes());

            bytes.extend_from_slice(&record.body_hash.0);

            bytes.extend_from_slice(&(record.inputs.len() as u64).to_le_bytes());

            for input in &record.inputs {
                bytes.extend_from_slice(&input.commit.0);

                bytes.extend_from_slice(&input.canonical_ko);

                bytes.extend_from_slice(&input.authorization.canonical_bytes());

                bytes.extend_from_slice(&input.created_height.to_le_bytes());

                bytes.push(u8::from(input.was_coinbase));
            }

            bytes.extend_from_slice(&(record.kernel_bindings.len() as u64).to_le_bytes());

            for binding in &record.kernel_bindings {
                bytes.extend_from_slice(&binding.canonical_bytes());
            }

            parts.push(bytes);
        }

        let refs: Vec<&[u8]> = parts.iter().map(|part| part.as_slice()).collect();

        Ok(hash_multi(CUTTHROUGH_RETENTION_DOMAIN, &refs))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughRetentionError {
    #[error("cut-through retention horizon must be non-zero")]
    ZeroHorizon,

    #[error("candidate transaction is invalid: {0}")]
    Candidate(CutThroughV1Error),

    #[error("input {commit} is absent from authoritative UTXO state")]
    UnknownInput { commit: String },

    #[error("retained input authorization is invalid at height {height}, index {index}")]
    InvalidRetainedInputAuthorization { height: u64, index: usize },

    #[error("malformed retained stealth excess at height {height}, index {index}")]
    MalformedRetainedStealthExcess { height: u64, index: usize },

    #[error("duplicate retained block height {height}")]
    DuplicateHeight { height: u64 },

    #[error("duplicate retained spend {commit}")]
    DuplicateRetainedSpend { commit: String },

    #[error("retention map key {key} does not match record height {record}")]
    HeightKeyMismatch { key: u64, record: u64 },
}

/// One UTXO mutation retained for deterministic rollback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughUndoUtxoV1 {
    pub commit: Commitment,
    pub entry: UtxoEntry,
}

/// Complete undo material for one confirmed v3 transfer aggregate.
///
/// The record stores both the authoritative pre-state and the expected
/// post-state. Rollback therefore fails closed if the live state no longer
/// matches the state that this undo record was created for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutThroughUndoRecordV1 {
    pub height: u64,
    pub previous_height: u64,

    pub body_hash: Hash256,

    pub previous_utxo_root: Hash256,
    pub post_utxo_root: Hash256,

    pub previous_kernel_sum: Commitment,
    pub previous_kernel_count: u64,

    pub post_kernel_sum: Commitment,
    pub post_kernel_count: u64,

    pub previous_minted_darks: u64,
    pub previous_burned_darks: u64,

    pub post_minted_darks: u64,
    pub post_burned_darks: u64,

    pub previous_tx_count: u64,
    pub post_tx_count: u64,

    pub spent_inputs: Vec<CutThroughUndoUtxoV1>,

    pub created_outputs: Vec<CutThroughUndoUtxoV1>,
}

impl CutThroughUndoRecordV1 {
    pub fn validate(&self) -> Result<(), CutThroughApplyError> {
        if self.previous_height.checked_add(1) != Some(self.height) {
            return Err(CutThroughApplyError::InvalidUndoRecord {
                height: self.height,
            });
        }

        let kernel_delta = self
            .post_kernel_count
            .checked_sub(self.previous_kernel_count)
            .ok_or(CutThroughApplyError::InvalidUndoRecord {
                height: self.height,
            })?;

        let tx_delta = self
            .post_tx_count
            .checked_sub(self.previous_tx_count)
            .ok_or(CutThroughApplyError::InvalidUndoRecord {
                height: self.height,
            })?;

        if kernel_delta == 0 || kernel_delta != tx_delta {
            return Err(CutThroughApplyError::InvalidUndoRecord {
                height: self.height,
            });
        }

        // Transfer-only Phase 4B never mints.
        if self.previous_minted_darks != self.post_minted_darks
            || self.post_burned_darks < self.previous_burned_darks
        {
            return Err(CutThroughApplyError::InvalidUndoRecord {
                height: self.height,
            });
        }

        let mut seen = BTreeSet::new();

        for spent in &self.spent_inputs {
            if !seen.insert(spent.commit.0) {
                return Err(CutThroughApplyError::InvalidUndoRecord {
                    height: self.height,
                });
            }
        }

        for created in &self.created_outputs {
            if !seen.insert(created.commit.0) {
                return Err(CutThroughApplyError::InvalidUndoRecord {
                    height: self.height,
                });
            }

            if created.entry.height != self.height || created.entry.is_coinbase {
                return Err(CutThroughApplyError::InvalidUndoRecord {
                    height: self.height,
                });
            }
        }

        Ok(())
    }
}

/// Prototype state coupling the ordinary Nightfall ledger with
/// the v3 authorization-retention window and its reorg undo material.
///
/// This remains isolated from the active v2 block path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutThroughStateV1 {
    pub ledger: LedgerState,

    pub retention: CutThroughRetentionWindowV1,

    /// Reorg material. No entry may be pruned while its block remains inside
    /// the cut-through authorization horizon.
    #[serde(default)]
    pub undo: BTreeMap<u64, CutThroughUndoRecordV1>,
}

/// Audit record for one retention/undo pair removed after the
/// configured cut-through authorization horizon.
///
/// `body_hash` identifies the exact historical v3 candidate whose
/// local authorization and inverse-transition evidence was discarded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunedCutThroughHistoryV1 {
    pub height: u64,
    pub body_hash: Hash256,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CutThroughApplyError {
    #[error(transparent)]
    Validation(#[from] crate::CutThroughStateError),

    #[error(transparent)]
    Retention(#[from] CutThroughRetentionError),

    #[error(transparent)]
    Supply(#[from] crate::SupplyError),

    #[error("confirmed height {got} is not the next ledger height {expected}")]
    NonSequentialHeight { got: u64, expected: u64 },

    #[error("retention height {retained} is ahead of ledger height {ledger}")]
    RetentionAheadOfLedger { retained: u64, ledger: u64 },

    #[error("undo map key {key} does not match record height {record}")]
    UndoHeightKeyMismatch { key: u64, record: u64 },

    #[error("undo height {undo} is ahead of ledger height {ledger}")]
    UndoAheadOfLedger { undo: u64, ledger: u64 },

    #[error("undo tip {undo} does not match ledger tip {ledger}")]
    UndoTipMismatch { undo: u64, ledger: u64 },

    #[error("invalid undo record at height {height}")]
    InvalidUndoRecord { height: u64 },

    #[error("missing retention record for undo height {height}")]
    MissingRetentionForUndo { height: u64 },

    #[error("undo and retention body hash differ at height {height}")]
    UndoRetentionMismatch { height: u64 },

    #[error("live state does not match undo post-state at height {height}")]
    CurrentStateDoesNotMatchUndo { height: u64 },

    #[error("duplicate undo height {height}")]
    DuplicateUndoHeight { height: u64 },

    #[error("no rollback material exists for ledger tip {height}")]
    NoUndoAtTip { height: u64 },

    #[error("validated input {commit} disappeared during staged commit")]
    MissingInputDuringCommit { commit: String },

    #[error("validated output {commit} collided during staged commit")]
    OutputCollisionDuringCommit { commit: String },

    #[error("created output {commit} is missing during rollback")]
    MissingCreatedOutputDuringRollback { commit: String },

    #[error("created output {commit} metadata changed before rollback")]
    CreatedOutputMetadataMismatch { commit: String },

    #[error("spent input {commit} already exists during rollback")]
    RestoredInputCollision { commit: String },

    #[error("retention record disappeared during rollback at height {height}")]
    RetentionRecordMissingDuringRollback { height: u64 },

    #[error("rollback did not reconstruct the exact pre-state at height {height}")]
    RollbackStateMismatch { height: u64 },

    #[error("malformed kernel excess during staged commit")]
    MalformedKernelExcess,
    #[error("prunable height {height} has no matching retention record")]
    MissingRetentionForPrune { height: u64 },

    #[error("retention height {height} crossed the horizon but has no matching undo record")]
    MissingUndoForPrune { height: u64 },

    #[error("retention/undo body hash mismatch while pruning height {height}")]
    PruneBodyHashMismatch { height: u64 },

    #[error("attempted to prune current ledger tip {height}")]
    PruneWouldRemoveTip { height: u64 },

    #[error("state arithmetic overflow")]
    ArithmeticOverflow,
}

impl CutThroughStateV1 {
    pub fn new(
        ledger: LedgerState,
        retention: CutThroughRetentionWindowV1,
    ) -> Result<Self, CutThroughApplyError> {
        let state = Self {
            ledger,
            retention,

            undo: BTreeMap::new(),
        };

        state.validate()?;

        Ok(state)
    }

    /// Validate coupled state after construction, reload or staging.
    pub fn validate(&self) -> Result<(), CutThroughApplyError> {
        self.retention.validate()?;

        self.ledger.verify_supply()?;

        if let Some(retained_height) = self.retention.blocks().keys().next_back().copied() {
            if retained_height > self.ledger.height.0 {
                return Err(CutThroughApplyError::RetentionAheadOfLedger {
                    retained: retained_height,

                    ledger: self.ledger.height.0,
                });
            }
        }

        for (height, undo) in &self.undo {
            if *height != undo.height {
                return Err(CutThroughApplyError::UndoHeightKeyMismatch {
                    key: *height,

                    record: undo.height,
                });
            }

            undo.validate()?;

            if *height > self.ledger.height.0 {
                return Err(CutThroughApplyError::UndoAheadOfLedger {
                    undo: *height,

                    ledger: self.ledger.height.0,
                });
            }

            let retained = self
                .retention
                .blocks()
                .get(height)
                .ok_or(CutThroughApplyError::MissingRetentionForUndo { height: *height })?;

            if retained.body_hash != undo.body_hash {
                return Err(CutThroughApplyError::UndoRetentionMismatch { height: *height });
            }
        }

        if let Some((undo_height, undo)) = self.undo.iter().next_back() {
            if *undo_height != self.ledger.height.0 {
                return Err(CutThroughApplyError::UndoTipMismatch {
                    undo: *undo_height,

                    ledger: self.ledger.height.0,
                });
            }

            if self.ledger.utxo_root() != undo.post_utxo_root
                || self.ledger.kernels.sum != undo.post_kernel_sum
                || self.ledger.kernels.count != undo.post_kernel_count
                || self.ledger.supply.total_minted_darks != undo.post_minted_darks
                || self.ledger.supply.total_burned_darks != undo.post_burned_darks
                || self.ledger.tx_count != undo.post_tx_count
            {
                return Err(CutThroughApplyError::CurrentStateDoesNotMatchUndo {
                    height: undo.height,
                });
            }
        }

        Ok(())
    }

    /// Atomically apply one ordinary v3 transfer aggregate.
    ///
    /// `self` is never partially mutated. The exact inverse transition is
    /// retained together with the authorization horizon record.
    pub fn apply_transfer(
        &mut self,
        tx: &CutThroughTransactionV1,
        confirmed_height: Height,
        ctx: &[u8],
    ) -> Result<(), CutThroughApplyError> {
        self.validate()?;

        let expected_height = self
            .ledger
            .height
            .0
            .checked_add(1)
            .ok_or(CutThroughApplyError::ArithmeticOverflow)?;

        if confirmed_height.0 != expected_height {
            return Err(CutThroughApplyError::NonSequentialHeight {
                got: confirmed_height.0,

                expected: expected_height,
            });
        }

        if self.undo.contains_key(&confirmed_height.0) {
            return Err(CutThroughApplyError::DuplicateUndoHeight {
                height: confirmed_height.0,
            });
        }

        self.ledger
            .check_cutthrough_v1_acceptable(tx, confirmed_height, ctx)?;

        let fees = tx.body.total_fee();

        let kernel_count = u64::try_from(tx.body.kernels.len())
            .map_err(|_| CutThroughApplyError::ArithmeticOverflow)?;

        let next_kernel_count = self
            .ledger
            .kernels
            .count
            .checked_add(kernel_count)
            .ok_or(CutThroughApplyError::ArithmeticOverflow)?;

        let next_tx_count = self
            .ledger
            .tx_count
            .checked_add(kernel_count)
            .ok_or(CutThroughApplyError::ArithmeticOverflow)?;

        let next_burned = self
            .ledger
            .supply
            .total_burned_darks
            .checked_add(fees)
            .ok_or(CutThroughApplyError::ArithmeticOverflow)?;

        // Capture canonical Ko and original UTXO metadata BEFORE spending.
        let retained = RetainedCutThroughBlockV1::capture(&self.ledger, tx, confirmed_height)?;

        let spent_inputs = retained
            .inputs
            .iter()
            .map(|input| CutThroughUndoUtxoV1 {
                commit: input.commit,

                entry: input.restored_utxo_entry(),
            })
            .collect::<Vec<_>>();

        let previous_height = self.ledger.height.0;

        let previous_utxo_root = self.ledger.utxo_root();

        let previous_kernel_sum = self.ledger.kernels.sum;

        let previous_kernel_count = self.ledger.kernels.count;

        let previous_minted_darks = self.ledger.supply.total_minted_darks;

        let previous_burned_darks = self.ledger.supply.total_burned_darks;

        let previous_tx_count = self.ledger.tx_count;

        let body_hash = retained.body_hash;

        // No mutation of `self` before this point.
        let mut staged_ledger = self.ledger.clone();

        let mut staged_retention = self.retention.clone();

        let mut staged_undo = self.undo.clone();

        for input in &tx.body.inputs {
            if staged_ledger.utxos.remove(&input.commit).is_none() {
                return Err(CutThroughApplyError::MissingInputDuringCommit {
                    commit: input.commit.to_hex(),
                });
            }
        }

        for output in &tx.body.outputs {
            let inserted = staged_ledger.utxos.insert(
                output.output.commit,
                UtxoEntry {
                    output_pk: output.output.output_pk,

                    height: confirmed_height.0,

                    is_coinbase: output.output.features.is_coinbase(),
                },
            );

            if !inserted {
                return Err(CutThroughApplyError::OutputCollisionDuringCommit {
                    commit: output.output.commit.to_hex(),
                });
            }
        }

        for kernel in &tx.body.kernels {
            staged_ledger
                .kernels
                .add(&kernel.kernel.excess)
                .ok_or(CutThroughApplyError::MalformedKernelExcess)?;
        }

        if staged_ledger.kernels.count != next_kernel_count {
            return Err(CutThroughApplyError::ArithmeticOverflow);
        }

        staged_ledger.supply.total_burned_darks = next_burned;

        staged_ledger.tx_count = next_tx_count;

        staged_ledger.height = confirmed_height;

        staged_ledger.verify_supply()?;

        let created_outputs = tx
            .body
            .outputs
            .iter()
            .map(|output| CutThroughUndoUtxoV1 {
                commit: output.output.commit,

                entry: UtxoEntry {
                    output_pk: output.output.output_pk,

                    height: confirmed_height.0,

                    is_coinbase: false,
                },
            })
            .collect::<Vec<_>>();

        staged_retention.insert(retained)?;

        let undo = CutThroughUndoRecordV1 {
            height: confirmed_height.0,

            previous_height,

            body_hash,

            previous_utxo_root,

            post_utxo_root: staged_ledger.utxo_root(),

            previous_kernel_sum,

            previous_kernel_count,

            post_kernel_sum: staged_ledger.kernels.sum,

            post_kernel_count: staged_ledger.kernels.count,

            previous_minted_darks,

            previous_burned_darks,

            post_minted_darks: staged_ledger.supply.total_minted_darks,

            post_burned_darks: staged_ledger.supply.total_burned_darks,

            previous_tx_count,

            post_tx_count: staged_ledger.tx_count,

            spent_inputs,

            created_outputs,
        };

        undo.validate()?;

        if staged_undo.insert(confirmed_height.0, undo).is_some() {
            return Err(CutThroughApplyError::DuplicateUndoHeight {
                height: confirmed_height.0,
            });
        }

        let staged = Self {
            ledger: staged_ledger,

            retention: staged_retention,

            undo: staged_undo,
        };

        staged.validate()?;

        *self = staged;

        Ok(())
    }

    /// Roll back exactly the current v3 ledger tip.
    ///
    /// Rollback is intentionally tip-only. Reorgs deeper than one block call
    /// this repeatedly, newest block first. Nothing is mutated unless the
    /// current state exactly matches the retained post-state.
    pub fn rollback_tip(&mut self) -> Result<Hash256, CutThroughApplyError> {
        self.validate()?;

        let tip = self.ledger.height.0;

        let undo = self
            .undo
            .get(&tip)
            .cloned()
            .ok_or(CutThroughApplyError::NoUndoAtTip { height: tip })?;

        undo.validate()?;

        let retained = self
            .retention
            .blocks()
            .get(&tip)
            .ok_or(CutThroughApplyError::MissingRetentionForUndo { height: tip })?;

        if retained.body_hash != undo.body_hash {
            return Err(CutThroughApplyError::UndoRetentionMismatch { height: tip });
        }

        let mut staged_ledger = self.ledger.clone();

        let mut staged_retention = self.retention.clone();

        let mut staged_undo = self.undo.clone();

        // Remove outputs created by the orphaned tip.
        for created in &undo.created_outputs {
            let current = staged_ledger
                .utxos
                .get(&created.commit)
                .cloned()
                .ok_or_else(
                    || CutThroughApplyError::MissingCreatedOutputDuringRollback {
                        commit: created.commit.to_hex(),
                    },
                )?;

            if current != created.entry {
                return Err(CutThroughApplyError::CreatedOutputMetadataMismatch {
                    commit: created.commit.to_hex(),
                });
            }

            staged_ledger.utxos.remove(&created.commit).ok_or_else(|| {
                CutThroughApplyError::MissingCreatedOutputDuringRollback {
                    commit: created.commit.to_hex(),
                }
            })?;
        }

        // Restore the exact UTXO metadata consumed by the orphaned tip.
        for spent in &undo.spent_inputs {
            if !staged_ledger
                .utxos
                .insert(spent.commit, spent.entry.clone())
            {
                return Err(CutThroughApplyError::RestoredInputCollision {
                    commit: spent.commit.to_hex(),
                });
            }
        }

        // KernelAccumulator has an append-only active API. For the isolated
        // v3 prototype we restore the exact trusted pre-state snapshot rather
        // than changing the active v2 accumulator interface.
        staged_ledger.kernels.sum = undo.previous_kernel_sum;

        staged_ledger.kernels.count = undo.previous_kernel_count;

        staged_ledger.supply.total_minted_darks = undo.previous_minted_darks;

        staged_ledger.supply.total_burned_darks = undo.previous_burned_darks;

        staged_ledger.tx_count = undo.previous_tx_count;

        staged_ledger.height = Height(undo.previous_height);

        let removed_retention = staged_retention
            .blocks
            .remove(&tip)
            .ok_or(CutThroughApplyError::RetentionRecordMissingDuringRollback { height: tip })?;

        if removed_retention.body_hash != undo.body_hash {
            return Err(CutThroughApplyError::UndoRetentionMismatch { height: tip });
        }

        if staged_undo.remove(&tip).is_none() {
            return Err(CutThroughApplyError::NoUndoAtTip { height: tip });
        }

        if staged_ledger.utxo_root() != undo.previous_utxo_root
            || staged_ledger.kernels.sum != undo.previous_kernel_sum
            || staged_ledger.kernels.count != undo.previous_kernel_count
            || staged_ledger.supply.total_minted_darks != undo.previous_minted_darks
            || staged_ledger.supply.total_burned_darks != undo.previous_burned_darks
            || staged_ledger.tx_count != undo.previous_tx_count
            || staged_ledger.height.0 != undo.previous_height
        {
            return Err(CutThroughApplyError::RollbackStateMismatch { height: tip });
        }

        staged_ledger.verify_supply()?;

        let staged = Self {
            ledger: staged_ledger,

            retention: staged_retention,

            undo: staged_undo,
        };

        staged.validate()?;

        *self = staged;

        Ok(undo.body_hash)
    }

    /// Return retention/undo pairs which have crossed the configured
    /// authorization horizon.
    ///
    /// This is a pure query. Nothing is removed.
    ///
    /// A retained authorization record is considered locally prunable only
    /// when its matching undo record still exists and both identify the same
    /// v3 body.
    pub fn prunable_history(&self) -> Result<Vec<PrunedCutThroughHistoryV1>, CutThroughApplyError> {
        self.validate()?;

        let tip = self.ledger.height;

        let heights = self.retention.prunable_heights(tip);

        let mut result = Vec::with_capacity(heights.len());

        for height in heights {
            // Defensive check. The retention policy should already prevent
            // the current tip from becoming prunable.
            if height == tip.0 {
                return Err(CutThroughApplyError::PruneWouldRemoveTip { height });
            }

            let retained = self
                .retention
                .blocks()
                .get(&height)
                .ok_or(CutThroughApplyError::MissingRetentionForPrune { height })?;

            let undo = self
                .undo
                .get(&height)
                .ok_or(CutThroughApplyError::MissingUndoForPrune { height })?;

            if retained.body_hash != undo.body_hash {
                return Err(CutThroughApplyError::PruneBodyHashMismatch { height });
            }

            result.push(PrunedCutThroughHistoryV1 {
                height,

                body_hash: retained.body_hash,
            });
        }

        Ok(result)
    }

    /// Atomically remove authorization-retention and undo material after
    /// the configured horizon has been crossed.
    ///
    /// Retention and undo form one logical historical object:
    ///
    /// * neither side is removed early;
    /// * neither side is removed independently;
    /// * current-tip rollback data is never removed;
    /// * inconsistencies fail before externally visible mutation.
    ///
    /// Once history has been pruned, a reorg deeper than the retained
    /// horizon must be recovered by replay/resync from older trusted chain
    /// data rather than reconstructed from missing local undo evidence.
    pub fn prune_expired_history(
        &mut self,
    ) -> Result<Vec<PrunedCutThroughHistoryV1>, CutThroughApplyError> {
        let prunable = self.prunable_history()?;

        if prunable.is_empty() {
            return Ok(Vec::new());
        }

        let mut staged_retention = self.retention.clone();

        let mut staged_undo = self.undo.clone();

        for item in &prunable {
            if item.height == self.ledger.height.0 {
                return Err(CutThroughApplyError::PruneWouldRemoveTip {
                    height: item.height,
                });
            }

            let retained = staged_retention.blocks.remove(&item.height).ok_or(
                CutThroughApplyError::MissingRetentionForPrune {
                    height: item.height,
                },
            )?;

            let undo = staged_undo.remove(&item.height).ok_or(
                CutThroughApplyError::MissingUndoForPrune {
                    height: item.height,
                },
            )?;

            if retained.body_hash != item.body_hash || undo.body_hash != item.body_hash {
                return Err(CutThroughApplyError::PruneBodyHashMismatch {
                    height: item.height,
                });
            }
        }

        let staged = Self {
            ledger: self.ledger.clone(),

            retention: staged_retention,

            undo: staged_undo,
        };

        staged.validate()?;

        // Only externally visible mutation.
        *self = staged;

        Ok(prunable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nightfall_crypto::{create_output, scan_output, WalletKeys};

    use nightfall_types::NetworkId;

    use crate::{build_cutthrough_transfer_v1, Payment, Spendable};

    fn retention_fixture() -> (
        LedgerState,
        CutThroughTransactionV1,
        Height,
        Commitment,
        [u8; 32],
    ) {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver = WalletKeys::generate();

        let (source, source_secrets) =
            create_output(&owner.address(), 20_000, "retention-source", ctx)
                .expect("source output");

        let discovered = scan_output(&owner.view_key(), &source).expect("discover source");

        let spendable = Spendable {
            commit: source.commit,

            value: 20_000,

            blind: source_secrets.blind,

            spend_secret: discovered.spend_secret(&owner),
        };

        let mut state = LedgerState::for_network(NetworkId::Devnet);

        assert!(state.utxos.insert(
            source.commit,
            UtxoEntry {
                output_pk: source.output_pk,

                height: 7,

                is_coinbase: false,
            },
        ));

        let tx = build_cutthrough_transfer_v1(
            &owner,
            &[spendable],
            &[Payment {
                to: receiver.address(),

                amount: 8_000,

                memo: "retention-payment".into(),
            }],
            1_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("v3 transaction");

        (state, tx, Height(20), source.commit, source.output_pk)
    }

    #[test]
    fn retention_policy_rejects_zero_horizon() {
        assert_eq!(
            CutThroughRetentionPolicyV1::new(0),
            Err(CutThroughRetentionError::ZeroHorizon),
        );
    }

    #[test]
    fn retention_boundary_is_exact_and_never_early() {
        let policy = CutThroughRetentionPolicyV1::new(10).expect("valid policy");

        assert!(!policy.is_beyond_horizon(Height(100), Height(109),));

        assert!(policy.is_beyond_horizon(Height(100), Height(110),));

        assert_eq!(policy.prune_through_height(Height(109)), Some(99),);

        assert_eq!(policy.prune_through_height(Height(110)), Some(100),);

        assert!(!policy.is_beyond_horizon(Height(100), Height(99),));
    }

    #[test]
    fn retention_capture_uses_authoritative_ko() {
        let (state, tx, height, source_commit, source_ko) = retention_fixture();

        let record = RetainedCutThroughBlockV1::capture(&state, &tx, height)
            .expect("capture retention record");

        assert_eq!(record.confirmed_height, 20,);

        assert_eq!(record.body_hash, tx.body.hash(),);

        assert_eq!(record.inputs.len(), 1,);

        assert_eq!(record.inputs[0].commit, source_commit,);

        assert_eq!(record.inputs[0].canonical_ko, source_ko,);

        assert_eq!(record.inputs[0].created_height, 7,);

        assert!(!record.inputs[0].was_coinbase);

        assert_eq!(record.kernel_bindings.len(), 1,);
    }

    #[test]
    fn retained_input_reconstructs_utxo_metadata() {
        let (state, tx, height, source_commit, _source_ko) = retention_fixture();

        let original = state
            .utxos
            .get(&source_commit)
            .expect("source UTXO")
            .clone();

        let record = RetainedCutThroughBlockV1::capture(&state, &tx, height).expect("capture");

        assert_eq!(record.inputs[0].restored_utxo_entry(), original,);
    }

    #[test]
    fn retention_capture_rejects_unknown_input() {
        let (mut state, tx, height, source_commit, _source_ko) = retention_fixture();

        state.utxos.remove(&source_commit);

        assert!(matches!(
            RetainedCutThroughBlockV1::capture(&state, &tx, height,),
            Err(CutThroughRetentionError::UnknownInput { .. })
        ));
    }

    #[test]
    fn retention_window_marks_only_old_enough_records() {
        let (state_a, tx_a, _height_a, _commit_a, _ko_a) = retention_fixture();

        let (state_b, tx_b, _height_b, _commit_b, _ko_b) = retention_fixture();

        let record_a =
            RetainedCutThroughBlockV1::capture(&state_a, &tx_a, Height(100)).expect("record A");

        let record_b =
            RetainedCutThroughBlockV1::capture(&state_b, &tx_b, Height(105)).expect("record B");

        let policy = CutThroughRetentionPolicyV1::new(10).expect("policy");

        let mut window = CutThroughRetentionWindowV1::new(policy).expect("window");

        window.insert(record_a).expect("insert A");

        window.insert(record_b).expect("insert B");

        assert_eq!(window.prunable_heights(Height(109)), Vec::<u64>::new(),);

        assert_eq!(window.retained_heights(Height(109)), vec![100, 105],);

        assert_eq!(window.prunable_heights(Height(110)), vec![100],);

        assert_eq!(window.retained_heights(Height(110)), vec![105],);

        assert_eq!(window.prunable_heights(Height(115)), vec![100, 105],);
    }

    #[test]
    fn retention_window_rejects_duplicate_height_atomically() {
        let (state_a, tx_a, _, _, _) = retention_fixture();

        let (state_b, tx_b, _, _, _) = retention_fixture();

        let record_a =
            RetainedCutThroughBlockV1::capture(&state_a, &tx_a, Height(42)).expect("record A");

        let record_b =
            RetainedCutThroughBlockV1::capture(&state_b, &tx_b, Height(42)).expect("record B");

        let policy = CutThroughRetentionPolicyV1::new(10).expect("policy");

        let mut window = CutThroughRetentionWindowV1::new(policy).expect("window");

        window.insert(record_a).expect("first insert");

        let before = window.clone();

        assert_eq!(
            window.insert(record_b),
            Err(CutThroughRetentionError::DuplicateHeight { height: 42 }),
        );

        assert_eq!(window, before,);
    }

    #[test]
    fn retention_window_roundtrips_with_stable_hash() {
        let (state, tx, _, _, _) = retention_fixture();

        let record = RetainedCutThroughBlockV1::capture(&state, &tx, Height(20)).expect("record");

        let policy = CutThroughRetentionPolicyV1::new(10).expect("policy");

        let mut window = CutThroughRetentionWindowV1::new(policy).expect("window");

        window.insert(record).expect("insert");

        let before_hash = window.hash().expect("hash");

        let encoded = serde_json::to_vec(&window).expect("serialize retention window");

        let decoded: CutThroughRetentionWindowV1 =
            serde_json::from_slice(&encoded).expect("deserialize retention window");

        decoded.validate().expect("reloaded state valid");

        assert_eq!(decoded, window,);

        assert_eq!(decoded.hash().expect("reloaded hash"), before_hash,);
    }

    #[test]
    fn retention_hash_is_insertion_order_independent() {
        let (state_a, tx_a, _, _, _) = retention_fixture();

        let (state_b, tx_b, _, _, _) = retention_fixture();

        let record_a = RetainedCutThroughBlockV1::capture(&state_a, &tx_a, Height(20)).expect("A");

        let record_b = RetainedCutThroughBlockV1::capture(&state_b, &tx_b, Height(21)).expect("B");

        let policy = CutThroughRetentionPolicyV1::new(10).expect("policy");

        let mut first = CutThroughRetentionWindowV1::new(policy).expect("first");

        first.insert(record_a.clone()).expect("first A");

        first.insert(record_b.clone()).expect("first B");

        let mut second = CutThroughRetentionWindowV1::new(policy).expect("second");

        second.insert(record_b).expect("second B");

        second.insert(record_a).expect("second A");

        assert_eq!(
            first.hash().expect("first hash"),
            second.hash().expect("second hash"),
        );
    }

    fn atomic_apply_fixture() -> (
        CutThroughStateV1,
        CutThroughTransactionV1,
        Commitment,
        [u8; 32],
    ) {
        let ctx = NetworkId::Devnet.proof_context();

        let owner = WalletKeys::generate();

        let receiver = WalletKeys::generate();

        let (source, source_secrets) =
            create_output(&owner.address(), 20_000, "atomic-source", ctx).expect("source");

        let discovered = scan_output(&owner.view_key(), &source).expect("discover source");

        let spendable = Spendable {
            commit: source.commit,

            value: 20_000,

            blind: source_secrets.blind,

            spend_secret: discovered.spend_secret(&owner),
        };

        let mut ledger = LedgerState::for_network(NetworkId::Devnet);

        assert!(ledger.utxos.insert(
            source.commit,
            UtxoEntry {
                output_pk: source.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        // Establish a cryptographically consistent pre-transfer
        // supply state:
        //
        // UTXO = 20_000*G + blind*H
        // kernel = blind*H
        // UTXO - kernel = 20_000*G.
        let mint_kernel = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Coinbase,
            0,
            20_000,
            0,
            &source_secrets.blind,
        );

        ledger
            .kernels
            .add(&mint_kernel.excess)
            .expect("initial kernel excess");

        ledger.supply.total_minted_darks = 20_000;

        ledger.height = Height(0);

        ledger.verify_supply().expect("initial supply invariant");

        let tx = build_cutthrough_transfer_v1(
            &owner,
            &[spendable],
            &[Payment {
                to: receiver.address(),

                amount: 8_000,

                memo: "atomic-payment".into(),
            }],
            1_000,
            &owner.address(),
            0,
            ctx,
        )
        .expect("v3 transfer");

        let retention =
            CutThroughRetentionWindowV1::new(CutThroughRetentionPolicyV1::new(10).expect("policy"))
                .expect("retention");

        let state = CutThroughStateV1::new(ledger, retention).expect("coupled state");

        (state, tx, source.commit, source.output_pk)
    }

    #[test]
    fn atomic_apply_updates_ledger_and_retention_together() {
        let (mut state, tx, source_commit, source_ko) = atomic_apply_fixture();

        let old_kernel_count = state.ledger.kernels.count;

        state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context())
            .expect("atomic apply");

        assert_eq!(state.ledger.height, Height(1),);

        assert!(!state.ledger.utxos.contains(&source_commit));

        for output in &tx.body.outputs {
            let entry = state
                .ledger
                .utxos
                .get(&output.output.commit)
                .expect("new UTXO");

            assert_eq!(entry.output_pk, output.output.output_pk,);

            assert_eq!(entry.height, 1,);

            assert!(!entry.is_coinbase);
        }

        assert_eq!(state.ledger.supply.total_minted_darks, 20_000,);

        assert_eq!(state.ledger.supply.total_burned_darks, 1_000,);

        assert_eq!(state.ledger.supply.circulating(), 19_000,);

        assert_eq!(state.ledger.tx_count, tx.body.kernels.len() as u64,);

        assert_eq!(
            state.ledger.kernels.count,
            old_kernel_count + tx.body.kernels.len() as u64,
        );

        state
            .ledger
            .verify_supply()
            .expect("post-apply supply invariant");

        assert_eq!(state.retention.len(), 1,);

        let retained = state
            .retention
            .blocks()
            .get(&1)
            .expect("height-1 retention");

        assert_eq!(retained.body_hash, tx.body.hash(),);

        assert_eq!(retained.inputs.len(), 1,);

        assert_eq!(retained.inputs[0].commit, source_commit,);

        assert_eq!(retained.inputs[0].canonical_ko, source_ko,);

        assert_eq!(retained.kernel_bindings.len(), 1,);

        state.validate().expect("coupled state valid");
    }

    #[test]
    fn atomic_apply_rejects_nonsequential_height_without_mutation() {
        let (mut state, tx, _, _) = atomic_apply_fixture();

        let root_before = state.ledger.utxo_root();

        let kernel_before = state.ledger.kernel_sum();

        let retention_before = state.retention.hash().expect("retention hash");

        let tx_count_before = state.ledger.tx_count;

        let burned_before = state.ledger.supply.total_burned_darks;

        assert_eq!(
            state.apply_transfer(&tx, Height(2), NetworkId::Devnet.proof_context(),),
            Err(CutThroughApplyError::NonSequentialHeight {
                got: 2,
                expected: 1,
            }),
        );

        assert_eq!(state.ledger.utxo_root(), root_before,);

        assert_eq!(state.ledger.kernel_sum(), kernel_before,);

        assert_eq!(state.ledger.tx_count, tx_count_before,);

        assert_eq!(state.ledger.supply.total_burned_darks, burned_before,);

        assert_eq!(
            state.retention.hash().expect("unchanged retention"),
            retention_before,
        );

        assert_eq!(state.ledger.height, Height(0),);
    }

    #[test]
    fn failed_validation_mutates_neither_ledger_nor_retention() {
        let (mut state, mut tx, source_commit, _) = atomic_apply_fixture();

        let root_before = state.ledger.utxo_root();

        let kernel_before = state.ledger.kernel_sum();

        let tx_count_before = state.ledger.tx_count;

        let burned_before = state.ledger.supply.total_burned_darks;

        let retention_before = state.retention.hash().expect("retention hash");

        let old = tx.body.stealth_offset.scalar().expect("canonical offset");

        tx.body.stealth_offset =
            crate::StealthOffsetV1::from_scalar(&(old + curve25519_dalek::scalar::Scalar::ONE));

        assert!(state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context(),)
            .is_err());

        assert!(state.ledger.utxos.contains(&source_commit));

        assert_eq!(state.ledger.utxo_root(), root_before,);

        assert_eq!(state.ledger.kernel_sum(), kernel_before,);

        assert_eq!(state.ledger.tx_count, tx_count_before,);

        assert_eq!(state.ledger.supply.total_burned_darks, burned_before,);

        assert_eq!(
            state.retention.hash().expect("unchanged retention"),
            retention_before,
        );

        assert_eq!(state.ledger.height, Height(0),);
    }

    #[test]
    fn coupled_state_rejects_retention_ahead_of_ledger() {
        let (mut state, tx, _, _) = atomic_apply_fixture();

        let record = RetainedCutThroughBlockV1::capture(&state.ledger, &tx, Height(5))
            .expect("future retention record");

        state
            .retention
            .insert(record)
            .expect("structurally valid retention");

        assert_eq!(
            state.validate(),
            Err(CutThroughApplyError::RetentionAheadOfLedger {
                retained: 5,
                ledger: 0,
            }),
        );
    }

    #[test]
    fn apply_preserves_global_supply_equation_exactly() {
        let (mut state, tx, _, _) = atomic_apply_fixture();

        state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply");

        assert_eq!(state.ledger.supply.total_minted_darks, 20_000,);

        assert_eq!(state.ledger.supply.total_burned_darks, tx.body.total_fee(),);

        assert_eq!(
            state.ledger.supply.circulating(),
            20_000 - tx.body.total_fee(),
        );

        assert_eq!(state.ledger.verify_supply(), Ok(()),);
    }

    #[test]
    fn rollback_restores_exact_preapply_state() {
        let (mut state, tx, source_commit, source_ko) = atomic_apply_fixture();

        let root_before = state.ledger.utxo_root();

        let kernel_sum_before = state.ledger.kernel_sum();

        let kernel_count_before = state.ledger.kernels.count;

        let minted_before = state.ledger.supply.total_minted_darks;

        let burned_before = state.ledger.supply.total_burned_darks;

        let tx_count_before = state.ledger.tx_count;

        let height_before = state.ledger.height;

        let retention_before = state.retention.hash().expect("retention hash");

        let source_before = state
            .ledger
            .utxos
            .get(&source_commit)
            .expect("source")
            .clone();

        state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply");

        assert_eq!(state.undo.len(), 1,);

        assert!(!state.ledger.utxos.contains(&source_commit));

        let rolled_back = state.rollback_tip().expect("rollback");

        assert_eq!(rolled_back, tx.body.hash(),);

        assert_eq!(state.ledger.utxo_root(), root_before,);

        assert_eq!(state.ledger.kernel_sum(), kernel_sum_before,);

        assert_eq!(state.ledger.kernels.count, kernel_count_before,);

        assert_eq!(state.ledger.supply.total_minted_darks, minted_before,);

        assert_eq!(state.ledger.supply.total_burned_darks, burned_before,);

        assert_eq!(state.ledger.tx_count, tx_count_before,);

        assert_eq!(state.ledger.height, height_before,);

        assert_eq!(
            state.retention.hash().expect("restored retention"),
            retention_before,
        );

        assert!(state.undo.is_empty());

        let restored = state
            .ledger
            .utxos
            .get(&source_commit)
            .expect("restored source");

        assert_eq!(restored, &source_before,);

        assert_eq!(restored.output_pk, source_ko,);

        for output in &tx.body.outputs {
            assert!(!state.ledger.utxos.contains(&output.output.commit));
        }

        assert_eq!(state.ledger.verify_supply(), Ok(()),);

        state.validate().expect("restored state valid");
    }

    #[test]
    fn rollback_without_undo_is_atomic() {
        let (mut state, _tx, _, _) = atomic_apply_fixture();

        let root_before = state.ledger.utxo_root();

        let kernel_before = state.ledger.kernel_sum();

        let retention_before = state.retention.hash().expect("retention");

        assert_eq!(
            state.rollback_tip(),
            Err(CutThroughApplyError::NoUndoAtTip { height: 0 }),
        );

        assert_eq!(state.ledger.utxo_root(), root_before,);

        assert_eq!(state.ledger.kernel_sum(), kernel_before,);

        assert_eq!(state.retention.hash().expect("unchanged"), retention_before,);

        assert_eq!(state.ledger.height, Height(0),);
    }

    #[test]
    fn rollback_detects_post_state_tampering() {
        let (mut state, tx, _, _) = atomic_apply_fixture();

        state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply");

        state.ledger.tx_count += 1;

        let tampered_count = state.ledger.tx_count;

        assert_eq!(
            state.rollback_tip(),
            Err(CutThroughApplyError::CurrentStateDoesNotMatchUndo { height: 1 }),
        );

        assert_eq!(state.ledger.tx_count, tampered_count,);

        assert_eq!(state.ledger.height, Height(1),);

        assert_eq!(state.undo.len(), 1,);

        assert_eq!(state.retention.len(), 1,);
    }

    #[test]
    fn undo_and_retention_body_hash_must_match() {
        let (mut state, tx, _, _) = atomic_apply_fixture();

        state
            .apply_transfer(&tx, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply");

        state.undo.get_mut(&1).expect("undo").body_hash = Hash256::ZERO;

        assert_eq!(
            state.validate(),
            Err(CutThroughApplyError::UndoRetentionMismatch { height: 1 }),
        );
    }

    fn two_block_rollback_fixture() -> (
        CutThroughStateV1,
        CutThroughTransactionV1,
        CutThroughTransactionV1,
        Hash256,
        Commitment,
        Commitment,
    ) {
        let ctx = NetworkId::Devnet.proof_context();

        let owner_a = WalletKeys::generate();

        let owner_b = WalletKeys::generate();

        let receiver_a = WalletKeys::generate();

        let receiver_b = WalletKeys::generate();

        let (source_a, secrets_a) =
            create_output(&owner_a.address(), 18_000, "reorg-source-a", ctx).expect("source A");

        let (source_b, secrets_b) =
            create_output(&owner_b.address(), 22_000, "reorg-source-b", ctx).expect("source B");

        let discovered_a = scan_output(&owner_a.view_key(), &source_a).expect("discover A");

        let discovered_b = scan_output(&owner_b.view_key(), &source_b).expect("discover B");

        let spendable_a = Spendable {
            commit: source_a.commit,

            value: 18_000,

            blind: secrets_a.blind,

            spend_secret: discovered_a.spend_secret(&owner_a),
        };

        let spendable_b = Spendable {
            commit: source_b.commit,

            value: 22_000,

            blind: secrets_b.blind,

            spend_secret: discovered_b.spend_secret(&owner_b),
        };

        let mut ledger = LedgerState::for_network(NetworkId::Devnet);

        assert!(ledger.utxos.insert(
            source_a.commit,
            UtxoEntry {
                output_pk: source_a.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        assert!(ledger.utxos.insert(
            source_b.commit,
            UtxoEntry {
                output_pk: source_b.output_pk,

                height: 0,

                is_coinbase: false,
            },
        ));

        let mint_a = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Coinbase,
            0,
            18_000,
            0,
            &secrets_a.blind,
        );

        let mint_b = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Coinbase,
            0,
            22_000,
            0,
            &secrets_b.blind,
        );

        ledger.kernels.add(&mint_a.excess).expect("mint A");

        ledger.kernels.add(&mint_b.excess).expect("mint B");

        ledger.supply.total_minted_darks = 40_000;

        ledger.height = Height(0);

        ledger.verify_supply().expect("initial supply");

        let initial_root = ledger.utxo_root();

        let tx_a = build_cutthrough_transfer_v1(
            &owner_a,
            &[spendable_a],
            &[Payment {
                to: receiver_a.address(),

                amount: 6_000,

                memo: "reorg-A".into(),
            }],
            1_000,
            &owner_a.address(),
            0,
            ctx,
        )
        .expect("tx A");

        let tx_b = build_cutthrough_transfer_v1(
            &owner_b,
            &[spendable_b],
            &[Payment {
                to: receiver_b.address(),

                amount: 7_000,

                memo: "reorg-B".into(),
            }],
            2_000,
            &owner_b.address(),
            0,
            ctx,
        )
        .expect("tx B");

        let retention =
            CutThroughRetentionWindowV1::new(CutThroughRetentionPolicyV1::new(10).expect("policy"))
                .expect("retention");

        let state = CutThroughStateV1::new(ledger, retention).expect("state");

        (
            state,
            tx_a,
            tx_b,
            initial_root,
            source_a.commit,
            source_b.commit,
        )
    }

    #[test]
    fn multiple_apply_and_rollback_restores_each_tip_in_order() {
        let (mut state, tx_a, tx_b, initial_root, source_a, source_b) =
            two_block_rollback_fixture();

        let initial_kernel_sum = state.ledger.kernel_sum();

        let initial_kernel_count = state.ledger.kernels.count;

        state
            .apply_transfer(&tx_a, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply A");

        let height1_root = state.ledger.utxo_root();

        let height1_kernel_sum = state.ledger.kernel_sum();

        let height1_kernel_count = state.ledger.kernels.count;

        assert_eq!(state.ledger.supply.total_burned_darks, 1_000,);

        state
            .apply_transfer(&tx_b, Height(2), NetworkId::Devnet.proof_context())
            .expect("apply B");

        assert_eq!(state.ledger.height, Height(2),);

        assert_eq!(state.undo.len(), 2,);

        assert_eq!(state.retention.len(), 2,);

        assert_eq!(state.ledger.supply.total_burned_darks, 3_000,);

        assert_eq!(state.rollback_tip().expect("rollback B"), tx_b.body.hash(),);

        assert_eq!(state.ledger.height, Height(1),);

        assert_eq!(state.ledger.utxo_root(), height1_root,);

        assert_eq!(state.ledger.kernel_sum(), height1_kernel_sum,);

        assert_eq!(state.ledger.kernels.count, height1_kernel_count,);

        assert_eq!(state.ledger.supply.total_burned_darks, 1_000,);

        assert_eq!(state.undo.len(), 1,);

        assert_eq!(state.retention.len(), 1,);

        assert!(state.ledger.utxos.contains(&source_b));

        assert_eq!(state.rollback_tip().expect("rollback A"), tx_a.body.hash(),);

        assert_eq!(state.ledger.height, Height(0),);

        assert_eq!(state.ledger.utxo_root(), initial_root,);

        assert_eq!(state.ledger.kernel_sum(), initial_kernel_sum,);

        assert_eq!(state.ledger.kernels.count, initial_kernel_count,);

        assert_eq!(state.ledger.supply.total_burned_darks, 0,);

        assert_eq!(state.ledger.supply.total_minted_darks, 40_000,);

        assert!(state.ledger.utxos.contains(&source_a));

        assert!(state.ledger.utxos.contains(&source_b));

        assert!(state.undo.is_empty());

        assert!(state.retention.is_empty());

        assert_eq!(state.ledger.verify_supply(), Ok(()),);

        state.validate().expect("fully restored state");
    }

    #[test]
    fn pruning_never_happens_before_horizon() {
        let (mut state, tx_a, tx_b, _, _, _) = two_block_rollback_fixture();

        state
            .apply_transfer(&tx_a, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply A");

        state
            .apply_transfer(&tx_b, Height(2), NetworkId::Devnet.proof_context())
            .expect("apply B");

        // Fixture horizon is 10 blocks.
        assert!(state.prunable_history().expect("prunable query").is_empty());

        assert!(state.prune_expired_history().expect("prune").is_empty());

        assert_eq!(state.retention.len(), 2,);

        assert_eq!(state.undo.len(), 2,);

        assert!(state.retention.blocks().contains_key(&1));

        assert!(state.retention.blocks().contains_key(&2));

        assert!(state.undo.contains_key(&1));

        assert!(state.undo.contains_key(&2));
    }

    #[test]
    fn pruning_removes_only_horizon_expired_retention_undo_pairs() {
        let (mut state, tx_a, tx_b, _, _, _) = two_block_rollback_fixture();

        // h = 1:
        //
        // at tip 2, height 1 has crossed the horizon,
        // while the current tip at height 2 must remain.
        state.retention.policy = CutThroughRetentionPolicyV1::new(1).expect("policy");

        state
            .apply_transfer(&tx_a, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply A");

        state
            .apply_transfer(&tx_b, Height(2), NetworkId::Devnet.proof_context())
            .expect("apply B");

        let expected = vec![PrunedCutThroughHistoryV1 {
            height: 1,

            body_hash: tx_a.body.hash(),
        }];

        assert_eq!(state.prunable_history().expect("query"), expected,);

        let pruned = state.prune_expired_history().expect("prune");

        assert_eq!(pruned, expected,);

        assert!(!state.retention.blocks().contains_key(&1));

        assert!(!state.undo.contains_key(&1));

        assert!(state.retention.blocks().contains_key(&2));

        assert!(state.undo.contains_key(&2));

        assert_eq!(state.retention.len(), 1,);

        assert_eq!(state.undo.len(), 1,);

        state.validate().expect("post-prune state");

        // Idempotent at an unchanged tip.
        assert!(state
            .prune_expired_history()
            .expect("second prune")
            .is_empty());
    }

    #[test]
    fn pruning_preserves_current_tip_rollback() {
        let (mut state, tx_a, tx_b, _, _, _) = two_block_rollback_fixture();

        state.retention.policy = CutThroughRetentionPolicyV1::new(1).expect("policy");

        state
            .apply_transfer(&tx_a, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply A");

        let height1_root = state.ledger.utxo_root();

        let height1_kernel = state.ledger.kernel_sum();

        let height1_burned = state.ledger.supply.total_burned_darks;

        state
            .apply_transfer(&tx_b, Height(2), NetworkId::Devnet.proof_context())
            .expect("apply B");

        state
            .prune_expired_history()
            .expect("prune horizon-expired A");

        assert!(!state.undo.contains_key(&1));

        assert!(state.undo.contains_key(&2));

        assert_eq!(
            state.rollback_tip().expect("rollback current tip"),
            tx_b.body.hash(),
        );

        assert_eq!(state.ledger.height, Height(1),);

        assert_eq!(state.ledger.utxo_root(), height1_root,);

        assert_eq!(state.ledger.kernel_sum(), height1_kernel,);

        assert_eq!(state.ledger.supply.total_burned_darks, height1_burned,);

        assert!(state.undo.is_empty());

        assert!(state.retention.is_empty());

        // Height 1 itself has already lost local undo evidence.
        // A deeper rollback must therefore fail closed.
        assert_eq!(
            state.rollback_tip(),
            Err(CutThroughApplyError::NoUndoAtTip { height: 1 }),
        );

        assert_eq!(state.ledger.verify_supply(), Ok(()),);
    }

    #[test]
    fn pruning_fails_closed_when_retention_has_no_matching_undo() {
        let (mut state, tx_a, tx_b, _, _, _) = two_block_rollback_fixture();

        state.retention.policy = CutThroughRetentionPolicyV1::new(1).expect("policy");

        state
            .apply_transfer(&tx_a, Height(1), NetworkId::Devnet.proof_context())
            .expect("apply A");

        state
            .apply_transfer(&tx_b, Height(2), NetworkId::Devnet.proof_context())
            .expect("apply B");

        // Simulate incomplete/corrupt local historical state.
        state.undo.remove(&1).expect("remove old undo");

        let root_before = state.ledger.utxo_root();

        let kernel_before = state.ledger.kernel_sum();

        let retention_before = state.retention.hash().expect("retention hash");

        let undo_before = state.undo.clone();

        assert_eq!(
            state.prune_expired_history(),
            Err(CutThroughApplyError::MissingUndoForPrune { height: 1 }),
        );

        assert_eq!(state.ledger.utxo_root(), root_before,);

        assert_eq!(state.ledger.kernel_sum(), kernel_before,);

        assert_eq!(
            state.retention.hash().expect("retention unchanged"),
            retention_before,
        );

        assert_eq!(state.undo, undo_before,);

        assert!(state.retention.blocks().contains_key(&1));

        assert!(state.retention.blocks().contains_key(&2));
    }

    #[test]
    fn pruning_missing_retention_error_is_typed() {
        let error = CutThroughApplyError::MissingRetentionForPrune { height: 42 };

        assert_eq!(
            error.to_string(),
            "prunable height 42 has no matching retention record",
        );
    }
}
