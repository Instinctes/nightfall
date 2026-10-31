//! The Nightfall shielded ledger.
//!
//! State transition is **atomic**: a block is validated in full against a
//! staged view before a single byte of committed state changes. v4 mutated the
//! nullifier set and the supply counters *before* checking the signature and
//! never rolled back, so one malformed transaction from any peer permanently
//! corrupted the node (audit finding N-01).

mod state_fingerprint;
pub use state_fingerprint::state_fingerprint;

mod aggregate;
mod builder;
mod tx;
mod utxo;

pub use aggregate::*;
pub use builder::*;
pub use tx::*;
pub use utxo::*;

use nightfall_crypto::{Commitment, KernelFeature};
use nightfall_types::{Hash256, Height, NetworkId};

/// Per-block aggregate limits.
pub const MAX_BLOCK_INPUTS: usize = 4_096;
pub const MAX_BLOCK_OUTPUTS: usize = 4_096;
pub const MAX_BLOCK_KERNELS: usize = 1_024;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LedgerState {
    pub height: Height,
    pub utxos: UtxoSet,
    pub kernels: KernelAccumulator,
    pub supply: SupplyState,
    pub tx_count: u64,
    /// Blocks a coinbase output must age before it can be spent.
    #[serde(default = "default_maturity")]
    pub coinbase_maturity: u64,
    /// First height validated under the binding rules. A state read from
    /// disk without the field gets the mainnet height; `Chain` re-applies
    /// its own network's value whenever it adopts a state.
    #[serde(default = "default_binding_height")]
    pub binding_height: u64,
}

fn default_maturity() -> u64 {
    COINBASE_MATURITY
}

fn default_binding_height() -> u64 {
    NetworkId::Mainnet.binding_fork_height()
}

impl LedgerState {
    /// A fresh state with the binding rules active from height 0.
    pub fn genesis() -> Self {
        Self {
            height: Height(0),
            utxos: UtxoSet::new(),
            kernels: KernelAccumulator::default(),
            supply: SupplyState::default(),
            tx_count: 0,
            coinbase_maturity: COINBASE_MATURITY,
            binding_height: 0,
        }
    }

    /// Genesis state with network-appropriate consensus parameters.
    pub fn for_network(network: NetworkId) -> Self {
        Self {
            coinbase_maturity: coinbase_maturity(network),
            binding_height: network.binding_fork_height(),
            ..Self::genesis()
        }
    }

    /// The same state with the binding rules starting at `height`.
    pub fn with_binding_height(mut self, height: u64) -> Self {
        self.binding_height = height;
        self
    }

    /// Are the binding rules in force for a block at `height`?
    pub fn binding_active(&self, height: Height) -> bool {
        height.0 >= self.binding_height
    }

    pub fn utxo_root(&self) -> Hash256 {
        self.utxos.root()
    }

    pub fn kernel_sum(&self) -> Commitment {
        self.kernels.sum
    }

    /// Cryptographic proof that the supply is exactly what it claims to be.
    pub fn verify_supply(&self) -> Result<(), SupplyError> {
        verify_supply_invariant(&self.utxos, &self.kernels, &self.supply)
    }

    /// Validate an aggregated block body and, only if everything checks out,
    /// commit it.
    ///
    /// The body is one flat set — transaction boundaries are gone by design.
    /// Validation therefore works on the aggregate: the balance equation is
    /// checked once over all inputs, outputs and kernels together.
    pub fn apply_block(
        &mut self,
        body: &BlockBody,
        height: Height,
        expected_reward: u64,
        ctx: &[u8],
    ) -> Result<(), LedgerError> {
        // Below the fork a block is judged exactly as 1.0.x judges it, which
        // means binding fields — which 1.0.x cannot even see — must not
        // change the verdict either way.
        let binding = self.binding_active(height);
        let stripped;
        let body = if !binding && body.has_any_binding() {
            let mut b = body.clone();
            b.strip_binding();
            stripped = b;
            &stripped
        } else {
            body
        };

        if body.outputs.is_empty() {
            return Err(LedgerError::EmptyBlock);
        }
        if body.kernels.is_empty() {
            return Err(LedgerError::MissingCoinbase);
        }
        // Canonical ordering is consensus: it is what makes the body hash
        // reproducible, and what stops a miner from leaking transaction
        // grouping through the order they chose.
        if !body.is_canonical() {
            return Err(LedgerError::NonCanonicalOrder);
        }
        if body.exceeds_size_limits() {
            return Err(LedgerError::BlockTooLarge);
        }

        // ---- Stage 1: validate against a staged view. ----

        // Exactly one coinbase kernel, pinned to this height so it cannot be
        // replayed at another.
        //
        // `expected_reward` is the *subsidy* (newly minted). While it is
        // positive, fees are burned. After the subsidy ends the miner is paid
        // the block's fees instead: the coinbase kernel then carries the fee
        // total, nothing is minted, nothing is burned, circulating is unchanged.
        if body.coinbase_kernels() != 1 {
            return Err(LedgerError::MissingCoinbase);
        }
        let coinbase = body
            .kernels
            .iter()
            .find(|k| k.feature == KernelFeature::Coinbase)
            .ok_or(LedgerError::MissingCoinbase)?;
        let fees = body.total_fee();
        let coinbase_due = if expected_reward > 0 {
            expected_reward
        } else {
            fees
        };
        if coinbase.reward_darks != coinbase_due {
            return Err(LedgerError::WrongReward {
                got: coinbase.reward_darks,
                expected: coinbase_due,
            });
        }
        if coinbase.lock_height != height.0 {
            return Err(LedgerError::CoinbaseHeightMismatch {
                got: coinbase.lock_height,
                expected: height.0,
            });
        }
        if self.supply.would_exceed_cap(expected_reward) {
            return Err(LedgerError::SupplyCapExceeded);
        }
        if binding && checked_fee_sum(&body.kernels).is_none() {
            return Err(LedgerError::FeeOutOfRange);
        }

        for k in &body.kernels {
            k.check_shape().map_err(|_| LedgerError::BadKernel)?;
            if !k.verify_signature() {
                return Err(LedgerError::BadKernelSignature);
            }
            if k.feature == KernelFeature::Plain && k.lock_height > height.0 {
                return Err(LedgerError::KernelLocked {
                    until: k.lock_height,
                });
            }
        }

        // Outputs: range proof, sender signature, no collisions.
        let mut created_ids: BTreeSet<[u8; 32]> = BTreeSet::new();
        for out in &body.outputs {
            if out.commit.point().is_none() || out.output_point().is_none() {
                return Err(LedgerError::MalformedOutput);
            }
            if !nightfall_crypto::rangeproofs::verify(&out.range_proof, &out.commit, ctx) {
                return Err(LedgerError::BadRangeProof);
            }
            if !out.verify_sender_sig() {
                return Err(LedgerError::BadOutputSignature);
            }
            if !created_ids.insert(out.commit.0) {
                return Err(LedgerError::DuplicateCommitment {
                    commit: out.commit.to_hex(),
                });
            }
            if self.utxos.contains(&out.commit) {
                return Err(LedgerError::DuplicateCommitment {
                    commit: out.commit.to_hex(),
                });
            }
        }

        // Inputs: must exist, be mature, be signed, and be spent only once.
        let mut spent: BTreeSet<[u8; 32]> = BTreeSet::new();
        for input in &body.inputs {
            if !spent.insert(input.commit.0) {
                return Err(LedgerError::DoubleSpendWithinBlock);
            }
            let entry = self
                .utxos
                .get(&input.commit)
                .ok_or(LedgerError::UnknownInput {
                    commit: input.commit.to_hex(),
                })?;
            if !is_mature(entry, height, self.coinbase_maturity) {
                return Err(LedgerError::ImmatureCoinbaseSpend {
                    created: entry.height,
                    now: height.0,
                });
            }
            if !Transaction::verify_input_signature(input, &entry.output_pk) {
                return Err(LedgerError::BadInputSignature);
            }
            // Input key signature (binding rules).
            if binding && !Transaction::verify_input_key(input, &entry.output_pk) {
                return Err(LedgerError::BadInputKeySignature);
            }
        }

        // The key balance, over the whole aggregate at once.
        if binding {
            key_balance(
                &body.inputs,
                &body.outputs,
                &body.kernels,
                body.stealth_offset,
            )
            .map_err(LedgerError::Binding)?;
        }

        // The balance equation, over the whole aggregate at once.
        let in_commits: Vec<Commitment> = body.inputs.iter().map(|i| i.commit).collect();
        let out_commits: Vec<Commitment> = body.outputs.iter().map(|o| o.commit).collect();
        let expected = nightfall_crypto::expected_excess(
            &in_commits,
            &out_commits,
            body.total_fee(),
            body.total_reward(),
        )
        .ok_or(LedgerError::MalformedOutput)?;
        let kernel_sum = body
            .kernel_excess_sum()
            .ok_or(LedgerError::MalformedKernelExcess)?;
        let offset = offset_point_h(body.kernel_offset).map_err(LedgerError::Binding)?;
        let kernel_sum = kernel_sum
            .point()
            .ok_or(LedgerError::MalformedKernelExcess)?
            + offset;
        if expected != kernel_sum {
            return Err(LedgerError::UnbalancedBlock);
        }

        // Exactly one output may claim the subsidy.
        let coinbase_outputs = body
            .outputs
            .iter()
            .filter(|o| o.features.is_coinbase())
            .count();
        if coinbase_outputs != 1 {
            return Err(LedgerError::BadCoinbaseOutputCount {
                got: coinbase_outputs,
            });
        }

        // ...and it must open to exactly the coinbase amount.
        if binding {
            let flagged = body
                .outputs
                .iter()
                .find(|o| o.features.is_coinbase())
                .and_then(|o| o.commit.point())
                .ok_or(LedgerError::MalformedOutput)?;
            let opened = flagged - nightfall_crypto::commit_public_value(coinbase.reward_darks);
            if Some(opened) != coinbase.excess.point() {
                return Err(LedgerError::BadCoinbaseOpening);
            }
        }

        // Everything that can fail is decided before the set changes. A
        // rejected block used to be applied to a full clone of the UTXO map
        // and thrown away; that clone is what ratcheted a seed's memory up
        // to the size of the whole history.
        let mut new_kernels = self.kernels.clone();
        for k in &body.kernels {
            new_kernels
                .add(&k.excess)
                .ok_or(LedgerError::MalformedKernelExcess)?;
        }
        new_kernels
            .add_offset(offset)
            .ok_or(LedgerError::MalformedKernelExcess)?;
        let mut new_supply = self.supply.clone();
        if expected_reward > 0 {
            new_supply.total_minted_darks = self
                .supply
                .total_minted_darks
                .checked_add(expected_reward)
                .ok_or(LedgerError::ArithmeticOverflow)?;
            new_supply.total_burned_darks = self
                .supply
                .total_burned_darks
                .checked_add(fees)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        let projected = self
            .utxos
            .sum_after(
                spent.iter().copied().map(Commitment),
                body.outputs.iter().map(|out| out.commit),
            )
            .ok_or(LedgerError::Supply(SupplyError::MalformedUtxo))?;
        supply_equation_holds(projected, &new_kernels, &new_supply).map_err(LedgerError::Supply)?;
        let undo = self.capture_undo(body)?;

        // ---- Stage 2: infallible commit. ----
        for c in &spent {
            self.utxos.remove(&Commitment(*c));
        }
        for out in &body.outputs {
            let is_coinbase = out.features.is_coinbase();
            self.utxos.insert(
                out.commit,
                UtxoEntry {
                    output_pk: out.output_pk,
                    height: height.0,
                    is_coinbase,
                },
            );
        }
        self.kernels = new_kernels;
        self.supply = new_supply;
        self.tx_count += body.kernels.len() as u64;
        self.height = height;

        // The projection above is the check. This is the same equation on
        // the set we just wrote, so a failure here means the projection and
        // the set disagree — roll back rather than publish that.
        if let Err(error) = self.verify_supply() {
            self.rewind(&undo);
            return Err(LedgerError::Supply(error));
        }
        Ok(())
    }

    /// Replay a block this node already wrote to its own disk.
    ///
    /// No range proofs, no signatures, no balance equation, no per-block
    /// supply check. Those ran when the block was accepted. Restart used
    /// to redo them for every stored block and froze wallets for minutes.
    /// The caller must still run [`verify_supply`] once the file is done.
    pub fn apply_block_state_only(
        &mut self,
        body: &BlockBody,
        height: Height,
        expected_reward: u64,
    ) -> Result<(), LedgerError> {
        if body.outputs.is_empty() || body.kernels.is_empty() {
            return Err(LedgerError::EmptyBlock);
        }
        if body.coinbase_kernels() != 1 {
            return Err(LedgerError::MissingCoinbase);
        }
        let fees = body.total_fee();
        if self.supply.would_exceed_cap(expected_reward) {
            return Err(LedgerError::SupplyCapExceeded);
        }

        let mut spent: BTreeSet<[u8; 32]> = BTreeSet::new();
        for input in &body.inputs {
            if !spent.insert(input.commit.0) {
                return Err(LedgerError::DoubleSpendWithinBlock);
            }
            if !self.utxos.contains(&input.commit) {
                return Err(LedgerError::UnknownInput {
                    commit: input.commit.to_hex(),
                });
            }
        }
        let mut new_kernels = self.kernels.clone();
        for k in &body.kernels {
            new_kernels
                .add(&k.excess)
                .ok_or(LedgerError::MalformedKernelExcess)?;
        }
        // A stored body below the fork never carries an offset; one at or
        // above it does, and the accumulator has to see it exactly as it did
        // when the block was first accepted.
        if self.binding_active(height) {
            let offset = offset_point_h(body.kernel_offset).map_err(LedgerError::Binding)?;
            new_kernels
                .add_offset(offset)
                .ok_or(LedgerError::MalformedKernelExcess)?;
        }
        let mut new_supply = self.supply.clone();
        if expected_reward > 0 {
            new_supply.total_minted_darks = self
                .supply
                .total_minted_darks
                .checked_add(expected_reward)
                .ok_or(LedgerError::ArithmeticOverflow)?;
            new_supply.total_burned_darks = self
                .supply
                .total_burned_darks
                .checked_add(fees)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        for c in &spent {
            self.utxos.remove(&Commitment(*c));
        }
        for out in &body.outputs {
            self.utxos.insert(
                out.commit,
                UtxoEntry {
                    output_pk: out.output_pk,
                    height: height.0,
                    is_coinbase: out.features.is_coinbase(),
                },
            );
        }
        self.kernels = new_kernels;
        self.supply = new_supply;
        self.tx_count += body.kernels.len() as u64;
        self.height = height;
        Ok(())
    }

    /// What [`Self::rewind`] needs to put this ledger back where it was
    /// before `body` was applied. Captured *before* the apply.
    pub fn capture_undo(&self, body: &BlockBody) -> Result<StateUndo, LedgerError> {
        let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
        let mut spent = Vec::with_capacity(body.inputs.len());
        for input in &body.inputs {
            if !seen.insert(input.commit.0) {
                return Err(LedgerError::DoubleSpendWithinBlock);
            }
            let entry = self.utxos.get(&input.commit).cloned().ok_or_else(|| {
                LedgerError::UnknownInput {
                    commit: input.commit.to_hex(),
                }
            })?;
            spent.push((input.commit, entry));
        }
        Ok(StateUndo {
            spent,
            created: body.outputs.iter().map(|out| out.commit).collect(),
            kernels: self.kernels.clone(),
            supply: self.supply.clone(),
            tx_count: self.tx_count,
            height: self.height,
        })
    }

    /// Inverse of applying the block `undo` was captured from.
    ///
    /// Removes what that block created, puts back what it spent, and restores
    /// the kernel sum, supply and height. Not a general undo of arbitrary
    /// edits: the capture and the apply have to be a pair.
    pub fn rewind(&mut self, undo: &StateUndo) {
        for commit in &undo.created {
            self.utxos.remove(commit);
        }
        for (commit, entry) in &undo.spent {
            self.utxos.insert(*commit, entry.clone());
        }
        self.kernels = undo.kernels.clone();
        self.supply = undo.supply.clone();
        self.tx_count = undo.tx_count;
        self.height = undo.height;
    }

    /// Mempool admission check: is this transaction valid against current state?
    pub fn check_tx_acceptable(
        &self,
        tx: &Transaction,
        next_height: Height,
        ctx: &[u8],
    ) -> Result<(), LedgerError> {
        if tx.is_coinbase() {
            return Err(LedgerError::CoinbaseInMempool);
        }
        let binding = self.binding_active(next_height);
        tx.verify_stateless_rules(ctx, binding)
            .map_err(|e| LedgerError::Tx {
                index: 0,
                source: e,
            })?;
        // Before the fork the binding fields are optional, but a transaction
        // that carries them must carry valid ones, or it would be relayed now
        // and refused the moment the fork activates.
        let check_keys = binding || tx.has_full_binding();

        let mut seen = BTreeSet::new();
        for input in &tx.inputs {
            if !seen.insert(input.commit.0) {
                return Err(LedgerError::DoubleSpendWithinBlock);
            }
            let entry = self
                .utxos
                .get(&input.commit)
                .ok_or(LedgerError::UnknownInput {
                    commit: input.commit.to_hex(),
                })?;
            if !is_mature(entry, next_height, self.coinbase_maturity) {
                return Err(LedgerError::ImmatureCoinbaseSpend {
                    created: entry.height,
                    now: next_height.0,
                });
            }
            if !Transaction::verify_input_signature(input, &entry.output_pk) {
                return Err(LedgerError::BadInputSignature);
            }
            if check_keys && !Transaction::verify_input_key(input, &entry.output_pk) {
                return Err(LedgerError::BadInputKeySignature);
            }
        }
        for out in &tx.outputs {
            if self.utxos.contains(&out.commit) {
                return Err(LedgerError::DuplicateCommitment {
                    commit: out.commit.to_hex(),
                });
            }
        }
        for k in &tx.kernels {
            if k.feature == KernelFeature::Coinbase {
                return Err(LedgerError::CoinbaseInMempool);
            }
        }
        Ok(())
    }
}

/// Ledger as it was before one block, plus the commitments that block created.
///
/// Small on purpose: a block spends and creates a handful of outputs. Keeping
/// the last few hundred of these is enough to leave a fork, and it is not a
/// second copy of the UTXO set.
#[derive(Clone, Debug)]
pub struct StateUndo {
    pub spent: Vec<(Commitment, UtxoEntry)>,
    pub created: Vec<Commitment>,
    pub kernels: KernelAccumulator,
    pub supply: SupplyState,
    pub tx_count: u64,
    pub height: Height,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("block contains no transactions")]
    EmptyBlock,
    #[error("first transaction must be the coinbase")]
    MissingCoinbase,
    #[error("coinbase must be the first transaction")]
    CoinbaseNotFirst,
    #[error("more than one coinbase in block")]
    MultipleCoinbase,
    #[error("coinbase transaction offered to the mempool")]
    CoinbaseInMempool,
    #[error("block reward {got} does not match consensus schedule {expected}")]
    WrongReward { got: u64, expected: u64 },
    #[error("minting this block would exceed the 90,000,000 NIGHT hard cap")]
    SupplyCapExceeded,
    #[error("input {commit} does not exist in the UTXO set")]
    UnknownInput { commit: String },
    #[error("double spend within block")]
    DoubleSpendWithinBlock,
    #[error("duplicate output commitment {commit}")]
    DuplicateCommitment { commit: String },
    #[error("immature coinbase: created at {created}, spend attempted at {now}")]
    ImmatureCoinbaseSpend { created: u64, now: u64 },
    #[error("kernel locked until height {until}")]
    KernelLocked { until: u64 },
    #[error("malformed kernel excess")]
    MalformedKernelExcess,
    #[error("block body is not in canonical order")]
    NonCanonicalOrder,
    #[error("block exceeds aggregate size limits")]
    BlockTooLarge,
    #[error("malformed output")]
    MalformedOutput,
    #[error("invalid range proof")]
    BadRangeProof,
    #[error("invalid kernel")]
    BadKernel,
    #[error("invalid kernel signature")]
    BadKernelSignature,
    #[error("invalid output sender signature")]
    BadOutputSignature,
    #[error("invalid input signature")]
    BadInputSignature,
    #[error("aggregate block does not balance")]
    UnbalancedBlock,
    #[error("coinbase kernel bound to height {got}, block is at {expected}")]
    CoinbaseHeightMismatch { got: u64, expected: u64 },
    #[error("block has {got} coinbase output(s), expected exactly 1")]
    BadCoinbaseOutputCount { got: usize },
    #[error("arithmetic overflow")]
    ArithmeticOverflow,
    #[error("transaction {index}: {source}")]
    Tx {
        index: usize,
        #[source]
        source: TxError,
    },
    #[error("supply: {0}")]
    Supply(#[source] SupplyError),
    #[error("binding: {0}")]
    Binding(#[source] TxError),
    #[error("invalid input key signature")]
    BadInputKeySignature,
    #[error("coinbase output does not open to the coinbase amount")]
    BadCoinbaseOpening,
    #[error("kernel fee out of range")]
    FeeOutOfRange,
}
