//! Emission, block structure, validation and chain selection.

mod difficulty;
pub use difficulty::{median_time_past, next_difficulty};

use nightfall_crypto::{
    block_work, default_threads, domain, genesis_commitment, hash_multi, meets_difficulty,
    mine_parallel, nighthash, Address, Commitment,
};
use nightfall_ledger::{build_coinbase, BlockBody, LedgerState, Transaction};
use nightfall_types::{
    Amount, GenesisConfig, Hash256, Height, NetworkId, PowParams, DARKS_PER_NIGHT,
    HALVING_INTERVAL_BLOCKS, INITIAL_BLOCK_REWARD_NIGHT, MAX_FUTURE_DRIFT_SECS, MAX_SUPPLY_DARKS,
    MAX_SUPPLY_NIGHT, PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Reorgs deeper than this are refused outright. Depth is how many of *our*
/// blocks we would abandon — the rewind to the common ancestor — not how much
/// longer the other chain is. A laptop that slept for a few hours is hundreds
/// of blocks *behind* on a one-block fork; that is catch-up, not a deep reorg.
/// Without a rewind bound, a peer can still force us to throw away the whole
/// chain and revalidate an alternate history from genesis.
pub const MAX_REORG_DEPTH: usize = 500;

/// How many leading blocks of `candidate` we already hold (same hash).
pub fn reorg_common_prefix(our_hashes: &[Hash256], candidate: &[Block]) -> usize {
    candidate
        .iter()
        .zip(our_hashes.iter())
        .take_while(|(b, h)| b.hash() == **h)
        .count()
}

/// How many of our own blocks a candidate would force us to abandon.
///
/// Walks the shared prefix. Anything after that on our side is the rewind.
pub fn reorg_rewind(our_hashes: &[Hash256], candidate: &[Block]) -> usize {
    our_hashes
        .len()
        .saturating_sub(reorg_common_prefix(our_hashes, candidate))
}

/// Cap on transactions per block.
pub const MAX_TXS_PER_BLOCK: usize = 512;

/// How many full block bodies a pruned node keeps.
///
/// Equal to [`MAX_REORG_DEPTH`]: any reorg the node is willing to adopt
/// still has every body it needs. Older bodies are dropped; headers and
/// the UTXO set stay. Seeds that serve IBD must not prune.
pub const PRUNE_KEEP_BLOCKS: usize = MAX_REORG_DEPTH;

// ---------------------------------------------------------------- emission --

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub initial_reward_darks: u64,
    pub halving_interval: u64,
    pub max_supply_darks: u64,
}

impl Default for EmissionSchedule {
    fn default() -> Self {
        Self::locked_mainnet()
    }
}

impl EmissionSchedule {
    pub fn locked_mainnet() -> Self {
        Self {
            initial_reward_darks: INITIAL_BLOCK_REWARD_NIGHT * DARKS_PER_NIGHT,
            halving_interval: HALVING_INTERVAL_BLOCKS,
            max_supply_darks: MAX_SUPPLY_DARKS,
        }
    }

    pub fn theoretical_reward_at(&self, height: Height) -> Amount {
        if self.initial_reward_darks == 0 || self.halving_interval == 0 {
            return Amount::ZERO;
        }
        let halvings = height.0 / self.halving_interval;
        if halvings >= 64 {
            return Amount::ZERO;
        }
        Amount(self.initial_reward_darks >> halvings)
    }

    /// Reward actually payable, clamped so the hard cap can never be breached
    /// even by one dark.
    pub fn reward_at(&self, height: Height, total_minted_darks: u64) -> Amount {
        if total_minted_darks >= self.max_supply_darks {
            return Amount::ZERO;
        }
        let theoretical = self.theoretical_reward_at(height).darks();
        let room = self.max_supply_darks - total_minted_darks;
        Amount(theoretical.min(room))
    }
}

// ------------------------------------------------------------------ blocks --

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub height: Height,
    pub prev_hash: Hash256,
    /// Merkle root of the UTXO set after this block.
    pub utxo_root: Hash256,
    /// Running sum of every kernel excess ever accepted, after this block.
    pub kernel_sum: Commitment,
    /// Hash over the aggregated block body.
    pub body_root: Hash256,
    pub timestamp_unix: u64,
    /// Difficulty this block was mined against.
    pub difficulty: u64,
    pub nonce: u64,
    pub reward_darks: u64,
}

impl BlockHeader {
    /// Bytes hashed for proof of work. Excludes the nonce, which the PoW
    /// function appends.
    pub fn pow_preimage(&self) -> Vec<u8> {
        hash_multi(
            domain::BLOCK,
            &[
                &self.version.to_le_bytes(),
                &self.height.0.to_le_bytes(),
                &self.prev_hash.0,
                &self.utxo_root.0,
                &self.kernel_sum.0,
                &self.body_root.0,
                &self.timestamp_unix.to_le_bytes(),
                &self.difficulty.to_le_bytes(),
                &self.reward_darks.to_le_bytes(),
            ],
        )
        .0
        .to_vec()
    }

    /// Proof-of-work hash under the given parameters.
    ///
    /// Parameters are consensus data, not a local choice: a node using
    /// different memory settings computes different hashes and forks itself
    /// off the network.
    pub fn pow_hash(&self, params: PowParams) -> Hash256 {
        nighthash(&self.pow_preimage(), self.nonce, params)
    }

    /// Canonical block identity — covers the nonce, unlike the PoW pre-image.
    pub fn hash(&self) -> Hash256 {
        hash_multi(
            domain::BLOCK,
            &[&self.pow_preimage(), &self.nonce.to_le_bytes()],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    /// One flat, canonically sorted aggregate. Transactions do not survive
    /// into a block — see `nightfall_ledger::BlockBody`.
    pub body: BlockBody,
}

impl Block {
    pub fn hash(&self) -> Hash256 {
        self.header.hash()
    }

    pub fn work(&self) -> u128 {
        block_work(self.header.difficulty)
    }

    /// Verify this block's proof of work.
    pub fn pow_is_valid(&self, params: PowParams) -> bool {
        meets_difficulty(self.header.pow_hash(params), self.header.difficulty)
    }
}

/// A block waiting for proof of work. Handed to the miner so hashing happens
/// outside the node's state lock.
#[derive(Clone, Debug)]
pub struct BlockTemplate {
    pub header: BlockHeader,
    pub body: BlockBody,
    /// Tip the template was built on. If the tip moves, the template is stale.
    pub built_on: Hash256,
}

impl BlockTemplate {
    pub fn seal(mut self, nonce: u64) -> Block {
        self.header.nonce = nonce;
        Block {
            header: self.header,
            body: self.body,
        }
    }
}

/// Header facts kept after a body is pruned. Difficulty, MTP and reorg
/// hash-walks need the whole chain; the range proofs do not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactHeader {
    pub height: u64,
    pub hash: Hash256,
    pub prev_hash: Hash256,
    pub timestamp_unix: u64,
    pub difficulty: u64,
}

impl CompactHeader {
    pub fn from_block(block: &Block) -> Self {
        Self {
            height: block.header.height.0,
            hash: block.hash(),
            prev_hash: block.header.prev_hash,
            timestamp_unix: block.header.timestamp_unix,
            difficulty: block.header.difficulty,
        }
    }

    pub fn work(&self) -> u128 {
        block_work(self.difficulty)
    }
}

/// UTXO + headers below the first kept body. Enough to rebuild a pruned
/// chain across a reorg that stays inside [`MAX_REORG_DEPTH`].
#[derive(Clone, Debug)]
pub struct PrunedBase {
    pub first_height: u64,
    pub horizon: LedgerState,
    pub horizon_work: u128,
    pub headers: Vec<CompactHeader>,
}

// ------------------------------------------------------------------- chain --

#[derive(Clone, Debug)]
pub struct Chain {
    pub network: NetworkId,
    pub genesis: GenesisConfig,
    pub genesis_hash: Hash256,
    pub emission: EmissionSchedule,
    pub ledger: LedgerState,
    /// Full bodies from [`Self::first_height`] onward. Archive: the whole
    /// chain. Pruned: the reorg window.
    pub blocks: Vec<Block>,
    /// One compact header per height from genesis, even after prune.
    pub headers: Vec<CompactHeader>,
    /// Height of `blocks[0]`. Zero on an archive node.
    pub first_height: u64,
    /// Ledger after applying headers `[0, first_height)`. `None` iff
    /// `first_height == 0`.
    pub horizon: Option<LedgerState>,
    /// Cumulative work of the horizon prefix.
    pub horizon_work: u128,
    /// Exact cumulative proof of work. This, not block count, decides forks.
    pub total_work: u128,
}

impl Chain {
    pub fn new_fair(network: NetworkId) -> Result<Self, ConsensusError> {
        let genesis = GenesisConfig::fair_launch(network);
        genesis
            .assert_fair()
            .map_err(|e| ConsensusError::UnfairGenesis(e.to_string()))?;
        let bytes =
            serde_json::to_vec(&genesis).map_err(|e| ConsensusError::Codec(e.to_string()))?;
        Ok(Self {
            network,
            genesis,
            genesis_hash: genesis_commitment(&bytes),
            emission: EmissionSchedule::locked_mainnet(),
            ledger: LedgerState::for_network(network),
            blocks: Vec::new(),
            headers: Vec::new(),
            first_height: 0,
            horizon: None,
            horizon_work: 0,
            total_work: 0,
        })
    }

    pub fn proof_ctx(&self) -> &'static [u8] {
        self.network.proof_context()
    }

    pub fn pow_params(&self) -> PowParams {
        self.network.pow_params()
    }

    pub fn tip_height(&self) -> Option<Height> {
        self.headers
            .last()
            .map(|h| Height(h.height))
            .or_else(|| self.blocks.last().map(|b| b.header.height))
    }

    pub fn next_height(&self) -> Height {
        match self.tip_height() {
            Some(h) => h.next(),
            None => Height(self.first_height),
        }
    }

    pub fn block_count(&self) -> u64 {
        if !self.headers.is_empty() {
            self.headers.len() as u64
        } else {
            self.first_height + self.blocks.len() as u64
        }
    }

    pub fn is_pruned(&self) -> bool {
        self.first_height > 0
    }

    pub fn tip_hash(&self) -> Hash256 {
        self.headers
            .last()
            .map(|h| h.hash)
            .or_else(|| self.blocks.last().map(|b| b.hash()))
            .unwrap_or(self.genesis_hash)
    }

    pub fn hash_at(&self, height: u64) -> Option<Hash256> {
        self.headers
            .get(height as usize)
            .map(|h| h.hash)
            .or_else(|| self.block_by_height(height).map(|b| b.hash()))
    }

    pub fn hash_chain(&self) -> Vec<Hash256> {
        if !self.headers.is_empty() {
            self.headers.iter().map(|h| h.hash).collect()
        } else {
            self.blocks.iter().map(|b| b.hash()).collect()
        }
    }

    /// Snapshot of everything below the first kept body. `None` on archive.
    pub fn pruned_base(&self) -> Option<PrunedBase> {
        if self.first_height == 0 {
            return None;
        }
        Some(PrunedBase {
            first_height: self.first_height,
            horizon: self.horizon.clone()?,
            horizon_work: self.horizon_work,
            headers: self
                .headers
                .iter()
                .take(self.first_height as usize)
                .cloned()
                .collect(),
        })
    }

    pub fn total_minted(&self) -> u64 {
        self.ledger.supply.total_minted_darks
    }

    pub fn max_supply_night(&self) -> u64 {
        MAX_SUPPLY_NIGHT
    }

    pub fn block_by_height(&self, height: u64) -> Option<&Block> {
        let idx = height.checked_sub(self.first_height)? as usize;
        self.blocks.get(idx).filter(|b| b.header.height.0 == height)
    }

    /// Bodies we still hold, starting at `start_height`. Heights below
    /// [`Self::first_height`] are gone — the vec is empty, not a lie.
    pub fn blocks_from(&self, start_height: u64, limit: usize) -> Vec<Block> {
        if start_height < self.first_height {
            return Vec::new();
        }
        let idx = (start_height - self.first_height) as usize;
        self.blocks.iter().skip(idx).take(limit).cloned().collect()
    }

    /// Bodies from `first_height` through `height` inclusive.
    ///
    /// `None` if that range includes discarded bodies — a reorg that
    /// deep cannot be rebuilt on a pruned node.
    pub fn bodies_through(&self, height: u64) -> Option<Vec<Block>> {
        if self.first_height == 0 {
            let end = (height as usize).saturating_add(1).min(self.blocks.len());
            return Some(self.blocks[..end].to_vec());
        }
        if height + 1 < self.first_height {
            return None;
        }
        if height + 1 == self.first_height {
            return Some(Vec::new());
        }
        let last = height.checked_sub(self.first_height)? as usize;
        if last >= self.blocks.len() {
            return None;
        }
        Some(self.blocks[..=last].to_vec())
    }

    fn difficulty_history(&self) -> Vec<(u64, u64)> {
        if !self.headers.is_empty() {
            self.headers
                .iter()
                .map(|h| (h.timestamp_unix, h.difficulty))
                .collect()
        } else {
            self.blocks
                .iter()
                .map(|b| (b.header.timestamp_unix, b.header.difficulty))
                .collect()
        }
    }

    /// Difficulty required of the next block.
    pub fn next_difficulty(&self) -> u64 {
        difficulty::next_difficulty(
            &self.difficulty_history(),
            self.network.initial_difficulty(),
            self.network.min_difficulty(),
        )
    }

    pub fn median_time_past(&self) -> u64 {
        let ts: Vec<u64> = if !self.headers.is_empty() {
            self.headers.iter().map(|h| h.timestamp_unix).collect()
        } else {
            self.blocks
                .iter()
                .map(|b| b.header.timestamp_unix)
                .collect()
        };
        median_time_past(&ts)
    }

    /// Build an unsealed block. Cheap — no hashing beyond the roots.
    /// Build a template, dropping any transaction the ledger will not take.
    ///
    /// Returns the template and the txids that were left out, so the caller
    /// can purge them from the mempool instead of offering them again.
    ///
    /// This exists because the plain `build_template` is all-or-nothing, and
    /// on 28 August 2026 that turned one unusable mempool entry into a total
    /// mining outage: the miner rebuilt the template once a second, the ledger
    /// refused the whole thing over a single transaction, the template was
    /// discarded, and the node hashed *nothing* — for as long as the entry
    /// stayed, which without a restart is six hours. The log said
    /// "duplicate output commitment" once a second and nothing said "you have
    /// stopped mining".
    ///
    /// The fast path is unchanged: one attempt, one ledger clone. Filtering
    /// only happens once that attempt has already failed, so the common case
    /// pays nothing for it.
    pub fn build_template_filtering(
        &self,
        miner: &Address,
        extra_txs: Vec<Transaction>,
        timestamp_unix: u64,
    ) -> Result<(BlockTemplate, Vec<String>), ConsensusError> {
        // The crate also compiles to wasm32 and carries no logging dependency,
        // so the failure is swallowed here and the node logs what was dropped
        // using the list this returns.
        if let Ok(t) = self.build_template(miner, extra_txs.clone(), timestamp_unix) {
            return Ok((t, Vec::new()));
        }

        // Test each candidate on its own against the current ledger. A
        // transaction that cannot be accepted now will not become acceptable
        // by being tried again in a second.
        let height = self.next_height();
        let ctx = self.proof_ctx();
        let mut keep = Vec::with_capacity(extra_txs.len());
        let mut dropped = Vec::new();
        for tx in extra_txs {
            match self.ledger.check_tx_acceptable(&tx, height, ctx) {
                Ok(()) => keep.push(tx),
                Err(_) => dropped.push(tx.txid().to_hex()),
            }
        }

        // If the remainder still will not build, mine an empty block rather
        // than mine nothing. A block with only its coinbase is a perfectly
        // good block; a miner that refuses to produce one is simply off.
        match self.build_template(miner, keep, timestamp_unix) {
            Ok(t) => Ok((t, dropped)),
            Err(_) => self
                .build_template(miner, Vec::new(), timestamp_unix)
                .map(|t| (t, dropped)),
        }
    }

    pub fn build_template(
        &self,
        miner: &Address,
        mut extra_txs: Vec<Transaction>,
        timestamp_unix: u64,
    ) -> Result<BlockTemplate, ConsensusError> {
        let height = self.next_height();
        let difficulty = self.next_difficulty();
        let subsidy = self
            .emission
            .reward_at(height, self.ledger.supply.total_minted_darks)
            .darks();
        extra_txs.truncate(MAX_TXS_PER_BLOCK - 1);
        let fees: u64 = extra_txs.iter().map(|t| t.total_fee()).sum();
        // After the last subsidy the miner is paid in fees. Those coins are
        // already circulating; they are not minted and they are not burned.
        let coinbase_darks = if subsidy > 0 { subsidy } else { fees };

        let coinbase = build_coinbase(miner, coinbase_darks, height.0, self.proof_ctx())
            .map_err(|e| ConsensusError::Tx(e.to_string()))?;

        let mut txs = vec![coinbase];
        txs.extend(extra_txs);

        // Dissolve the transactions into one sorted aggregate.
        let body = BlockBody::aggregate(&txs);

        // Apply against a scratch copy to learn the resulting roots. This is a
        // clone of the ledger, never the live one.
        let mut trial = self.ledger.clone();
        trial
            .apply_block(&body, height, subsidy, self.proof_ctx())
            .map_err(|e| ConsensusError::Ledger(e.to_string()))?;

        // Timestamp must beat median-time-past.
        let mtp = self.median_time_past();
        let timestamp_unix = timestamp_unix.max(mtp.saturating_add(1));

        Ok(BlockTemplate {
            header: BlockHeader {
                version: PROTOCOL_VERSION,
                height,
                prev_hash: self.tip_hash(),
                utxo_root: trial.utxo_root(),
                kernel_sum: trial.kernel_sum(),
                body_root: body.hash(),
                timestamp_unix,
                difficulty,
                nonce: 0,
                reward_darks: coinbase_darks,
            },
            body,
            built_on: self.tip_hash(),
        })
    }

    /// Build, mine and apply in one go. Convenience for tests and devnet.
    pub fn mine_block(
        &mut self,
        miner: &Address,
        extra_txs: Vec<Transaction>,
        timestamp_unix: u64,
    ) -> Result<Block, ConsensusError> {
        let template = self.build_template(miner, extra_txs, timestamp_unix)?;
        let difficulty = template.header.difficulty;
        // Parallel: a single Argon2id hash takes milliseconds, so a
        // single-threaded miner would leave most of a modern CPU idle.
        let (nonce, _) = mine_parallel(
            &template.header.pow_preimage(),
            difficulty,
            0,
            self.pow_params(),
            default_threads(),
            &|| false,
            None,
        )
        .ok_or(ConsensusError::MiningAborted)?;
        let block = template.seal(nonce);
        self.apply_block(block.clone(), timestamp_unix)?;
        Ok(block)
    }

    /// Validate and, only on full success, commit a block to the tip.
    ///
    /// Every check runs before the ledger is touched; the ledger itself then
    /// applies atomically. A rejected block leaves the chain byte-identical.
    pub fn apply_block(&mut self, block: Block, now_unix: u64) -> Result<(), ConsensusError> {
        self.apply_block_inner(block, now_unix, true)
    }

    /// Apply a block **without** re-verifying its proof of work.
    ///
    /// Only legitimate for replaying blocks this very node already validated
    /// and wrote to its own disk. Nighthash-v2 costs ~11 ms to verify, so
    /// re-hashing the whole chain on every restart would make startup scale
    /// painfully with height. Every other rule is still enforced.
    ///
    /// Never call this on data received from a peer.
    pub fn apply_block_locally_trusted(
        &mut self,
        block: Block,
        now_unix: u64,
    ) -> Result<(), ConsensusError> {
        self.apply_block_inner(block, now_unix, false)
    }

    /// Replay a block from this node's own `blocks.jsonl`.
    ///
    /// Linkage only (height, parent). Proofs, signatures and the supply
    /// equation already ran when we accepted the block. Used on restart so
    /// a miner is not stuck for minutes on their own file.
    pub fn apply_block_from_own_disk(&mut self, block: Block) -> Result<(), ConsensusError> {
        if block.header.version != PROTOCOL_VERSION {
            return Err(ConsensusError::BadVersion {
                got: block.header.version,
                expected: PROTOCOL_VERSION,
            });
        }
        if block.header.height != self.next_height() {
            return Err(ConsensusError::BadHeight);
        }
        if block.header.prev_hash != self.tip_hash() {
            return Err(ConsensusError::BadPrev);
        }
        // The pin is checked here too, and it has to be.
        //
        // This path skips proof of work, and since checkpoints were added it
        // is also the path that *replays a peer's blocks* below a pinned
        // height — not only our own file. A pin enforced solely in
        // `apply_block_inner` would therefore be enforced on every path except
        // the fast one it created. That is a check that only looks like a
        // check.
        if let Some(pinned) = nightfall_types::checkpoint_at(block.header.height.0) {
            if block.hash().to_hex() != pinned {
                return Err(ConsensusError::CheckpointMismatch {
                    height: block.header.height.0,
                    expected: pinned.to_string(),
                    got: block.hash().to_hex(),
                });
            }
        }
        let subsidy = self
            .emission
            .reward_at(block.header.height, self.ledger.supply.total_minted_darks)
            .darks();
        self.ledger
            .apply_block_state_only(&block.body, block.header.height, subsidy)
            .map_err(|e| ConsensusError::Ledger(e.to_string()))?;
        self.total_work = self.total_work.saturating_add(block.work());
        self.push_block(block);
        Ok(())
    }

    fn push_block(&mut self, block: Block) {
        self.headers.push(CompactHeader::from_block(&block));
        self.blocks.push(block);
    }

    fn apply_block_inner(
        &mut self,
        block: Block,
        now_unix: u64,
        verify_pow: bool,
    ) -> Result<(), ConsensusError> {
        // --- cheap structural checks first (cheapest rejection wins) ---
        if block.header.version != PROTOCOL_VERSION {
            return Err(ConsensusError::BadVersion {
                got: block.header.version,
                expected: PROTOCOL_VERSION,
            });
        }
        if block.body.outputs.is_empty() || block.body.kernels.is_empty() {
            return Err(ConsensusError::BadTxCount);
        }
        if block.header.height != self.next_height() {
            return Err(ConsensusError::BadHeight);
        }
        if block.header.prev_hash != self.tip_hash() {
            return Err(ConsensusError::BadPrev);
        }

        // --- checkpoint: this build has an opinion about this height ---
        //
        // Cheap, and checked before anything expensive. A peer offering a
        // different history at a pinned height is not a peer with a heavier
        // chain, it is a peer on another chain, and no amount of work makes
        // that one ours.
        if let Some(pinned) = nightfall_types::checkpoint_at(block.header.height.0) {
            if block.hash().to_hex() != pinned {
                return Err(ConsensusError::CheckpointMismatch {
                    height: block.header.height.0,
                    expected: pinned.to_string(),
                    got: block.hash().to_hex(),
                });
            }
        }

        // --- timestamp: bounded ahead, strictly after median-time-past ---
        if verify_pow
            && block.header.timestamp_unix > now_unix.saturating_add(MAX_FUTURE_DRIFT_SECS)
        {
            return Err(ConsensusError::TimestampTooFarAhead);
        }
        if self.block_count() > 0 && block.header.timestamp_unix <= self.median_time_past() {
            return Err(ConsensusError::TimestampBeforeMedian);
        }

        // --- difficulty must be exactly what the schedule dictates ---
        let expected_difficulty = self.next_difficulty();
        if block.header.difficulty != expected_difficulty {
            return Err(ConsensusError::BadDifficulty {
                got: block.header.difficulty,
                expected: expected_difficulty,
            });
        }

        // --- proof of work ---
        // Deliberately checked after the cheap structural rules: a memory-hard
        // hash costs milliseconds, so we never pay for it on a block that was
        // going to be rejected anyway.
        if verify_pow && !block.pow_is_valid(self.pow_params()) {
            return Err(ConsensusError::BadPow);
        }

        // --- emission ---
        let subsidy = self
            .emission
            .reward_at(block.header.height, self.ledger.supply.total_minted_darks)
            .darks();
        let fees = block.body.total_fee();
        let coinbase_due = if subsidy > 0 { subsidy } else { fees };
        if block.header.reward_darks != coinbase_due {
            return Err(ConsensusError::BadReward);
        }

        // --- body root before doing the expensive ledger work ---
        if block.body.hash() != block.header.body_root {
            return Err(ConsensusError::BadTxRoot);
        }

        // --- full ledger validation on a scratch copy ---
        let mut trial = self.ledger.clone();
        trial
            .apply_block(&block.body, block.header.height, subsidy, self.proof_ctx())
            .map_err(|e| ConsensusError::Ledger(e.to_string()))?;

        if trial.utxo_root() != block.header.utxo_root {
            return Err(ConsensusError::BadUtxoRoot);
        }
        if trial.kernel_sum() != block.header.kernel_sum {
            return Err(ConsensusError::BadKernelSum);
        }

        // --- commit ---
        self.total_work = self.total_work.saturating_add(block.work());
        self.ledger = trial;
        self.push_block(block);
        Ok(())
    }

    /// Drop bodies older than `keep`, leaving headers and the UTXO set.
    ///
    /// The live ledger stays at the tip. A second copy (`horizon`) is the
    /// UTXO just before the first kept body, which is what a reorg inside
    /// the window replays from. Returns how many bodies were discarded.
    pub fn prune_keep(&mut self, keep: usize) -> Result<usize, ConsensusError> {
        let keep = keep.max(1);
        if self.blocks.len() <= keep {
            return Ok(0);
        }
        let drop_n = self.blocks.len() - keep;
        if self.horizon.is_none() {
            self.horizon = Some(LedgerState::for_network(self.network));
            self.horizon_work = 0;
        }
        for b in &self.blocks[..drop_n] {
            let minted = self
                .horizon
                .as_ref()
                .map(|h| h.supply.total_minted_darks)
                .unwrap_or(0);
            let subsidy = self.emission.reward_at(b.header.height, minted).darks();
            self.horizon
                .as_mut()
                .unwrap()
                .apply_block_state_only(&b.body, b.header.height, subsidy)
                .map_err(|e| ConsensusError::Ledger(e.to_string()))?;
            self.horizon_work = self.horizon_work.saturating_add(b.work());
        }
        self.blocks.drain(..drop_n);
        self.first_height += drop_n as u64;
        Ok(drop_n)
    }

    /// Replay a full block list into a fresh chain.
    pub fn rebuild_from_blocks(
        network: NetworkId,
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<Self, ConsensusError> {
        Self::rebuild_from_blocks_trusted_prefix(network, blocks, 0, now_unix)
    }

    /// Replay `blocks`, treating the first `trusted_prefix` as our own file.
    ///
    /// Those leading blocks are ones this node already validated (same hash as
    /// our own history). Re-checking range proofs, signatures and a ledger
    /// clone per height on every one-block fork is what made a 13 000-block
    /// laptop sit on "1 block behind" for ten minutes: that is the untrusted
    /// path, and the prefix is not untrusted. Linkage and UTXO replay only,
    /// same as startup. Everything after the shared prefix is checked in full,
    /// including proof of work. After the prefix we require the rebuilt
    /// roots to match the last trusted header, so a drifted state cannot
    /// launder a suffix.
    pub fn rebuild_from_blocks_trusted_prefix(
        network: NetworkId,
        blocks: Vec<Block>,
        trusted_prefix: usize,
        now_unix: u64,
    ) -> Result<Self, ConsensusError> {
        // A pinned height widens the trusted prefix for everyone, not just for
        // a node replaying its own disk.
        //
        // The saving is the whole reason checkpoints exist: ~11 ms of Argon2id
        // per block, six hours for a one-year-old chain. Skipping it below a
        // pin is sound because the blocks still have to *link* — each one's
        // prev_hash must be the last one's hash — and the block at the pinned
        // height still has to equal the pin. A forged history that satisfies
        // both is the real history.
        //
        // Two conditions, and the first draft had neither. Written out because
        // getting this wrong disables proof of work rather than merely being
        // slow, which is the same shape of fault as the v4 balance proof:
        //
        //  * **Mainnet only.** The pins are mainnet heights. A devnet chain of
        //    five blocks is *shorter* than the pin, and a naive
        //    `min(blocks.len())` therefore marked every block trusted and
        //    skipped proof of work on every network. Caught by
        //    `reorg_still_validates_the_untrusted_suffix`.
        //
        //  * **The chain must actually reach the pin.** A prefix is only
        //    anchored if the pinned block is in it; a candidate that stops
        //    short of the pinned height has nothing vouching for it and gets
        //    the full check. Trusting the first 25,000 blocks of a chain that
        //    never arrives at block 25,000 trusts nothing at all.
        let pin_height = nightfall_types::highest_checkpoint_height();
        let assume_valid_to = if network == NetworkId::Mainnet
            && pin_height > 0
            && (blocks.len() as u64) > pin_height
            && std::env::var("NIGHTFALL_NO_ASSUME_VALID").is_err()
        {
            // Heights are indices here: `blocks` starts at genesis, so the
            // pinned block sits at `pin_height` and the prefix is inclusive.
            (pin_height + 1) as usize
        } else {
            0
        };
        let trusted_prefix = trusted_prefix.max(assume_valid_to);

        let mut chain = Self::new_fair(network)?;
        for (i, b) in blocks.into_iter().enumerate() {
            if i < trusted_prefix {
                chain.apply_block_from_own_disk(b)?;
                continue;
            }
            if i == trusted_prefix && trusted_prefix > 0 {
                chain.check_tip_roots()?;
            }
            chain.apply_block(b, now_unix)?;
        }
        if trusted_prefix > 0 && trusted_prefix >= chain.block_count() as usize {
            chain.check_tip_roots()?;
        }
        Ok(chain)
    }

    /// Last applied header must still describe the ledger we just rebuilt.
    fn check_tip_roots(&self) -> Result<(), ConsensusError> {
        let Some(tip) = self.blocks.last() else {
            return Ok(());
        };
        if self.ledger.utxo_root() != tip.header.utxo_root {
            return Err(ConsensusError::BadUtxoRoot);
        }
        if self.ledger.kernel_sum() != tip.header.kernel_sum {
            return Err(ConsensusError::BadKernelSum);
        }
        Ok(())
    }

    /// Extend the tip with sequential blocks, stopping at the first that does
    /// not connect. Returns how many were applied.
    pub fn try_ingest_blocks(
        &mut self,
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<usize, ConsensusError> {
        let mut applied = 0usize;
        for b in blocks {
            match self.apply_block(b, now_unix) {
                Ok(()) => applied += 1,
                Err(ConsensusError::BadPrev) | Err(ConsensusError::BadHeight) => break,
                Err(e) => {
                    if applied > 0 {
                        break;
                    }
                    return Err(e);
                }
            }
        }
        Ok(applied)
    }

    /// Adopt `blocks` if and only if it is a fully valid chain carrying **more
    /// cumulative work**.
    ///
    /// v4 compared `blocks.len()` and broke ties on tip-hash ordering. Because
    /// its difficulty was clamped to 28 bits, an attacker could mine an
    /// arbitrarily long chain of near-free blocks and take over the network for
    /// pocket change (audit finding N-02).
    pub fn maybe_reorg_to(
        &mut self,
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<bool, ConsensusError> {
        let hashes = self.hash_chain();
        let base = self.pruned_base();
        match Self::evaluate_reorg_at(
            self.network,
            self.total_work,
            &hashes,
            base,
            blocks,
            now_unix,
        )? {
            Some(candidate) => Ok(self.adopt_reorg(candidate)),
            None => Ok(false),
        }
    }

    /// Decide a reorg without touching an existing chain.
    ///
    /// This is the expensive half of [`Self::maybe_reorg_to`] — it rebuilds the
    /// candidate and verifies only the untrusted suffix — and it is
    /// deliberately a free function over borrowed facts rather than a method.
    /// A caller holding a lock around its chain can copy the work and the tip
    /// hashes, release the lock, run the rebuild, and only then come back for
    /// [`Self::adopt_reorg`].
    ///
    /// Shared history is identified by hash against `our_hashes` and replayed
    /// as our own file. The suffix still pays full validation, proof of work
    /// included. Running the untrusted path on the prefix froze a live node
    /// for about ten minutes at 13 000 blocks: range proofs and a ledger clone
    /// per height, to throw away a four-block fork.
    ///
    /// `our_hashes` is the identity of the chain being replaced, cheapest
    /// first: 32 bytes per block, no bodies. The caller copies it under the
    /// lock and drops the lock before calling this.
    pub fn evaluate_reorg(
        network: NetworkId,
        our_work: u128,
        our_hashes: &[Hash256],
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<Option<Self>, ConsensusError> {
        Self::evaluate_reorg_at(network, our_work, our_hashes, None, blocks, now_unix)
    }

    /// Same as [`Self::evaluate_reorg`], but a pruned node passes the UTXO
    /// sitting at `first_height` and a candidate that *starts there*.
    ///
    /// Archive callers leave `base` as `None` and still hand a chain from
    /// genesis. A fork that would rewind past the prune horizon is
    /// [`ConsensusError::ReorgTooDeep`] — the bodies are gone.
    pub fn evaluate_reorg_at(
        network: NetworkId,
        our_work: u128,
        our_hashes: &[Hash256],
        base: Option<PrunedBase>,
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<Option<Self>, ConsensusError> {
        if blocks.is_empty() {
            return Ok(None);
        }

        let start = base.as_ref().map(|b| b.first_height).unwrap_or(0);
        if let Some(first) = blocks.first() {
            if first.header.height.0 != start {
                return Err(ConsensusError::BadHeight);
            }
        }

        // Work first, and deliberately so.
        //
        // Fork choice is decided by cumulative work, so that is the question
        // worth asking, and summing declared difficulty is cheaper than
        // anything that follows. Checking length first meant a candidate could
        // be turned away for a reason unrelated to how it would have been
        // judged — and it reported `ReorgTooDeep` while doing it, which reads
        // as "this chain is suspicious" rather than "we never looked".
        let suffix_work: u128 = blocks.iter().map(|b| b.work()).sum();
        let claimed_work = base.as_ref().map(|b| b.horizon_work).unwrap_or(0) + suffix_work;
        if claimed_work <= our_work {
            return Ok(None);
        }

        // Depth is the rewind, not the length gap.
        //
        // The old check (`blocks.len() > our_len + MAX_REORG_DEPTH`) treated a
        // node that fell a few hours behind on a one-block fork as "too deep"
        // the moment the seed pulled 500 blocks ahead. Those two chains share
        // almost everything; the laptop would have to delete `blocks.jsonl`
        // to rejoin. A peer that actually forks at genesis, abandoning more
        // than MAX_REORG_DEPTH of our history, is still refused — that is the
        // denial-of-service limit, and it stays.
        let start_idx = start as usize;
        if our_hashes.len() < start_idx {
            return Err(ConsensusError::ReorgTooDeep);
        }
        let window_common = blocks
            .iter()
            .zip(our_hashes.iter().skip(start_idx))
            .take_while(|(b, h)| b.hash() == **h)
            .count();
        let common = start_idx + window_common;
        if our_hashes.len().saturating_sub(common) > MAX_REORG_DEPTH {
            return Err(ConsensusError::ReorgTooDeep);
        }

        // Shared history is ours. Only the suffix is untrusted work.
        let candidate = match base {
            Some(base) => {
                Self::rebuild_from_pruned_base(network, base, blocks, window_common, now_unix)?
            }
            None => Self::rebuild_from_blocks_trusted_prefix(network, blocks, common, now_unix)?,
        };
        candidate.verify_supply()?;
        if candidate.total_work > our_work {
            return Ok(Some(candidate));
        }
        Ok(None)
    }

    fn rebuild_from_pruned_base(
        network: NetworkId,
        base: PrunedBase,
        blocks: Vec<Block>,
        trusted_prefix: usize,
        now_unix: u64,
    ) -> Result<Self, ConsensusError> {
        let mut chain = Self::new_fair(network)?;
        chain.ledger = base.horizon.clone();
        chain.horizon = Some(base.horizon);
        chain.horizon_work = base.horizon_work;
        chain.first_height = base.first_height;
        chain.headers = base.headers;
        chain.total_work = base.horizon_work;
        for (i, b) in blocks.into_iter().enumerate() {
            if i < trusted_prefix {
                chain.apply_block_from_own_disk(b)?;
                continue;
            }
            if i == trusted_prefix && trusted_prefix > 0 {
                chain.check_tip_roots()?;
            }
            chain.apply_block(b, now_unix)?;
        }
        if trusted_prefix > 0 && trusted_prefix >= chain.blocks.len() {
            chain.check_tip_roots()?;
        }
        Ok(chain)
    }

    /// Take a chain produced by [`Self::evaluate_reorg`], if it still wins.
    ///
    /// Cheap, and safe to call under a lock. The work comparison is repeated
    /// rather than assumed: the rebuild ran with the lock released, so this
    /// chain may have moved on — possibly past the candidate — while it was
    /// happening. Re-checking here is what makes releasing the lock sound.
    pub fn adopt_reorg(&mut self, candidate: Self) -> bool {
        if candidate.total_work > self.total_work {
            *self = candidate;
            return true;
        }
        false
    }

    pub fn chain_work(&self) -> u128 {
        self.total_work
    }

    /// Mempool admission.
    pub fn precheck_tx(&self, tx: &Transaction) -> Result<(), ConsensusError> {
        self.ledger
            .check_tx_acceptable(tx, self.next_height(), self.proof_ctx())
            .map_err(|e| ConsensusError::Ledger(e.to_string()))
    }

    /// Re-verify the global supply invariant.
    pub fn verify_supply(&self) -> Result<(), ConsensusError> {
        self.ledger
            .verify_supply()
            .map_err(|e| ConsensusError::Ledger(e.to_string()))
    }
}

// ----------------------------------------------------------------- mempool --

#[derive(Clone, Debug, Default)]
pub struct Mempool {
    pub txs: HashMap<String, Transaction>,
    /// txid -> when this node first saw it, unix seconds. See [`Self::expire`].
    seen: HashMap<String, u64>,
}

impl Mempool {
    /// Cap on mempool entries — an unbounded map is a trivial memory exhaustion
    /// vector for any peer.
    pub const MAX_ENTRIES: usize = 10_000;

    /// How long a transaction may wait for a block before this node forgets it.
    ///
    /// Six hours is 1,440 blocks at the 15-second target — the same span as
    /// coinbase maturity. A payment that no miner has taken in that time is not
    /// going to be taken.
    pub const MAX_AGE_SECS: u64 = 6 * 3600;

    /// Take a transaction, remembering when it arrived.
    ///
    /// `now_unix` is passed in rather than read from the clock because this
    /// crate also compiles to wasm32, where `SystemTime::now()` aborts.
    pub fn insert(&mut self, tx: Transaction, now_unix: u64) -> bool {
        if self.txs.len() >= Self::MAX_ENTRIES {
            // Full is usually not "too much traffic", it is "too many corpses".
            // Sweep before refusing, so a long-running node heals itself
            // instead of quietly going deaf.
            self.expire(now_unix);
            if self.txs.len() >= Self::MAX_ENTRIES {
                return false;
            }
        }
        let id = tx.txid().to_hex();
        let fresh = self.txs.insert(id.clone(), tx).is_none();
        self.seen.entry(id).or_insert(now_unix);
        fresh
    }

    /// Forget transactions no block ever took.
    ///
    /// Returns how many were dropped.
    ///
    /// This did not exist until 0.8.2, and its absence was visible on mainnet:
    /// [`Self::remove_included`] only deletes what a block *consumed*, so a
    /// transaction that never reaches a block is never removed. On 26 Aug 2026
    /// one seed was holding 60 such entries and the other 117 — different sets
    /// of the same corpses, because each had heard different ones. Left alone
    /// the map walks to `MAX_ENTRIES` and the node stops accepting new
    /// transactions at all.
    ///
    /// Dropping one is not a loss: a transaction this node forgets is still
    /// held by whoever created it, and the wallet re-submits it. Forgetting is
    /// how the two halves fit together.
    pub fn expire(&mut self, now_unix: u64) -> usize {
        let before = self.txs.len();
        let cutoff = now_unix.saturating_sub(Self::MAX_AGE_SECS);
        let seen = &self.seen;
        self.txs
            .retain(|id, _| seen.get(id).map(|t| *t > cutoff).unwrap_or(true));
        let live: std::collections::HashSet<&String> = self.txs.keys().collect();
        let keep: Vec<String> = self
            .seen
            .keys()
            .filter(|k| live.contains(k))
            .cloned()
            .collect();
        self.seen.retain(|k, _| keep.contains(k));
        before - self.txs.len()
    }

    /// When this node first saw a transaction, if it is still held.
    pub fn first_seen(&self, txid: &str) -> Option<u64> {
        self.seen.get(txid).copied()
    }

    /// Drop everything the block consumed.
    ///
    /// Matching is by spent commitment, not by txid: aggregation destroys
    /// transaction identity, so a block cannot tell us which txids it contained.
    pub fn remove_included(&mut self, block: &Block) {
        let spent: std::collections::BTreeSet<[u8; 32]> =
            block.body.inputs.iter().map(|i| i.commit.0).collect();
        let created: std::collections::BTreeSet<[u8; 32]> =
            block.body.outputs.iter().map(|o| o.commit.0).collect();

        self.txs.retain(|_, tx| {
            let consumed = tx.inputs.iter().any(|i| spent.contains(&i.commit.0));
            let duplicated = tx.outputs.iter().any(|o| created.contains(&o.commit.0));
            !consumed && !duplicated
        });
        // The timestamp index has to shrink with the map, or it becomes the
        // unbounded thing the cap was supposed to prevent.
        let live: std::collections::HashSet<String> = self.txs.keys().cloned().collect();
        self.seen.retain(|k, _| live.contains(k));
    }

    /// Forget specific transactions by id.
    ///
    /// Used when the ledger has refused a transaction for a reason that will
    /// not change on its own — a spent input, or an output that already
    /// exists. Offering such an entry to the block builder again next second
    /// is not optimism, it is a loop.
    pub fn drop_ids(&mut self, ids: &[String]) -> usize {
        let before = self.txs.len();
        for id in ids {
            self.txs.remove(id);
            self.seen.remove(id);
        }
        before - self.txs.len()
    }

    /// Ids of everything the predicate rejects, without removing anything.
    ///
    /// Split from [`Self::drop_ids`] so the caller can hold the chain and the
    /// mempool one at a time rather than borrowing both at once.
    pub fn unacceptable_ids<F>(&self, mut acceptable: F) -> Vec<String>
    where
        F: FnMut(&Transaction) -> bool,
    {
        self.txs
            .iter()
            .filter(|(_, tx)| !acceptable(tx))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Select transactions for a block, highest fee first and skipping any that
    /// conflict with one already chosen.
    ///
    /// v4 took `HashMap::values()` in arbitrary order with no conflict check, so
    /// two conflicting transactions would both be selected, the block would fail
    /// to assemble, and mining stalled — a free denial of service.
    pub fn select_for_block(&self, max: usize) -> Vec<Transaction> {
        let mut candidates: Vec<&Transaction> = self.txs.values().collect();
        candidates.sort_by(|a, b| {
            b.total_fee()
                .cmp(&a.total_fee())
                .then_with(|| a.txid().0.cmp(&b.txid().0))
        });

        let mut chosen = Vec::new();
        let mut spent = std::collections::BTreeSet::new();
        for tx in candidates {
            if chosen.len() >= max {
                break;
            }
            if tx.inputs.iter().any(|i| spent.contains(&i.commit.0)) {
                continue;
            }
            for i in &tx.inputs {
                spent.insert(i.commit.0);
            }
            chosen.push(tx.clone());
        }
        chosen
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConsensusError {
    #[error("unfair genesis: {0}")]
    UnfairGenesis(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("tx: {0}")]
    Tx(String),
    #[error("ledger: {0}")]
    Ledger(String),
    #[error("protocol version {got}, expected {expected}")]
    BadVersion { got: u32, expected: u32 },
    #[error("invalid transaction count")]
    BadTxCount,
    #[error("insufficient proof of work")]
    BadPow,
    #[error(
        "block {height} is {got} but this build is pinned to {expected} — \
         that is a different chain, not a longer one"
    )]
    CheckpointMismatch {
        height: u64,
        expected: String,
        got: String,
    },
    #[error("difficulty {got}, expected {expected}")]
    BadDifficulty { got: u64, expected: u64 },
    #[error("previous hash does not link to our tip")]
    BadPrev,
    #[error("unexpected height")]
    BadHeight,
    #[error("block reward does not match the emission schedule")]
    BadReward,
    #[error("transaction root mismatch")]
    BadTxRoot,
    #[error("UTXO root mismatch")]
    BadUtxoRoot,
    #[error("kernel sum mismatch")]
    BadKernelSum,
    #[error("timestamp too far in the future")]
    TimestampTooFarAhead,
    #[error("timestamp not after median time past")]
    TimestampBeforeMedian,
    #[error("reorg exceeds the maximum permitted depth")]
    ReorgTooDeep,
    #[error("mining aborted")]
    MiningAborted,
}
