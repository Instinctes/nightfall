//! The Nightfall shielded ledger.
//!
//! State transition is **atomic**: a block is validated in full against a
//! staged view before a single byte of committed state changes. v4 mutated the
//! nullifier set and the supply counters *before* checking the signature and
//! never rolled back, so one malformed transaction from any peer permanently
//! corrupted the node (audit finding N-01).

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
}

fn default_maturity() -> u64 {
    COINBASE_MATURITY
}

impl LedgerState {
    pub fn genesis() -> Self {
        Self {
            height: Height(0),
            utxos: UtxoSet::new(),
            kernels: KernelAccumulator::default(),
            supply: SupplyState::default(),
            tx_count: 0,
            coinbase_maturity: COINBASE_MATURITY,
        }
    }

    /// Genesis state with network-appropriate consensus parameters.
    pub fn for_network(network: NetworkId) -> Self {
        Self {
            coinbase_maturity: coinbase_maturity(network),
            ..Self::genesis()
        }
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
        if body.inputs.len() > MAX_BLOCK_INPUTS
            || body.outputs.len() > MAX_BLOCK_OUTPUTS
            || body.kernels.len() > MAX_BLOCK_KERNELS
        {
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
        if Commitment::from_point(expected) != kernel_sum {
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

        // ---- Stage 2: everything validated. Commit. ----
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
        for k in &body.kernels {
            self.kernels
                .add(&k.excess)
                .ok_or(LedgerError::MalformedKernelExcess)?;
        }
        if expected_reward > 0 {
            self.supply.total_minted_darks = self
                .supply
                .total_minted_darks
                .checked_add(expected_reward)
                .ok_or(LedgerError::ArithmeticOverflow)?;
            self.supply.total_burned_darks = self
                .supply
                .total_burned_darks
                .checked_add(fees)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        self.tx_count += body.kernels.len() as u64;
        self.height = height;

        // ---- Stage 3: the invariant must still hold. ----
        self.verify_supply().map_err(LedgerError::Supply)?;
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
        for k in &body.kernels {
            self.kernels
                .add(&k.excess)
                .ok_or(LedgerError::MalformedKernelExcess)?;
        }
        if expected_reward > 0 {
            self.supply.total_minted_darks = self
                .supply
                .total_minted_darks
                .checked_add(expected_reward)
                .ok_or(LedgerError::ArithmeticOverflow)?;
            self.supply.total_burned_darks = self
                .supply
                .total_burned_darks
                .checked_add(fees)
                .ok_or(LedgerError::ArithmeticOverflow)?;
        }
        self.tx_count += body.kernels.len() as u64;
        self.height = height;
        Ok(())
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
        tx.verify_stateless(ctx).map_err(|e| LedgerError::Tx {
            index: 0,
            source: e,
        })?;

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
}
