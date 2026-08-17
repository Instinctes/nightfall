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

// ------------------------------------------------------------------- chain --

#[derive(Clone, Debug)]
pub struct Chain {
    pub network: NetworkId,
    pub genesis: GenesisConfig,
    pub genesis_hash: Hash256,
    pub emission: EmissionSchedule,
    pub ledger: LedgerState,
    pub blocks: Vec<Block>,
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
        self.blocks.last().map(|b| b.header.height)
    }

    pub fn next_height(&self) -> Height {
        match self.tip_height() {
            None => Height(0),
            Some(h) => h.next(),
        }
    }

    pub fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn tip_hash(&self) -> Hash256 {
        self.blocks
            .last()
            .map(|b| b.hash())
            .unwrap_or(self.genesis_hash)
    }

    pub fn total_minted(&self) -> u64 {
        self.ledger.supply.total_minted_darks
    }

    pub fn max_supply_night(&self) -> u64 {
        MAX_SUPPLY_NIGHT
    }

    pub fn block_by_height(&self, height: u64) -> Option<&Block> {
        self.blocks.get(height as usize)
    }

    pub fn blocks_from(&self, start_height: u64, limit: usize) -> Vec<Block> {
        self.blocks
            .iter()
            .skip(start_height as usize)
            .take(limit)
            .cloned()
            .collect()
    }

    fn difficulty_history(&self) -> Vec<(u64, u64)> {
        self.blocks
            .iter()
            .map(|b| (b.header.timestamp_unix, b.header.difficulty))
            .collect()
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
        let ts: Vec<u64> = self
            .blocks
            .iter()
            .map(|b| b.header.timestamp_unix)
            .collect();
        median_time_past(&ts)
    }

    /// Build an unsealed block. Cheap — no hashing beyond the roots.
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

        // --- timestamp: bounded ahead, strictly after median-time-past ---
        if verify_pow
            && block.header.timestamp_unix > now_unix.saturating_add(MAX_FUTURE_DRIFT_SECS)
        {
            return Err(ConsensusError::TimestampTooFarAhead);
        }
        if !self.blocks.is_empty() && block.header.timestamp_unix <= self.median_time_past() {
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
        self.blocks.push(block);
        Ok(())
    }

    /// Replay a full block list into a fresh chain.
    pub fn rebuild_from_blocks(
        network: NetworkId,
        blocks: Vec<Block>,
        now_unix: u64,
    ) -> Result<Self, ConsensusError> {
        Self::rebuild_from_blocks_trusted_prefix(network, blocks, 0, now_unix)
    }

    /// Replay `blocks`, skipping proof-of-work on the first `trusted_prefix`.
    ///
    /// Those leading blocks are ones this node already validated (same hash as
    /// our own history). Re-hashing them on every one-block fork is what made
    /// a 6 000-block laptop appear to stop syncing: Argon2id at 11 ms each is
    /// a minute of work to throw away a single stale tip. Everything after
    /// the shared prefix is checked in full, including proof of work.
    pub fn rebuild_from_blocks_trusted_prefix(
        network: NetworkId,
        blocks: Vec<Block>,
        trusted_prefix: usize,
        now_unix: u64,
    ) -> Result<Self, ConsensusError> {
        let mut chain = Self::new_fair(network)?;
        for (i, b) in blocks.into_iter().enumerate() {
            if i < trusted_prefix {
                chain.apply_block_locally_trusted(b, now_unix)?;
            } else {
                chain.apply_block(b, now_unix)?;
            }
        }
        Ok(chain)
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
        let hashes: Vec<Hash256> = self.blocks.iter().map(|b| b.hash()).collect();
        match Self::evaluate_reorg(self.network, self.total_work, &hashes, blocks, now_unix)? {
            Some(candidate) => Ok(self.adopt_reorg(candidate)),
            None => Ok(false),
        }
    }

    /// Decide a reorg without touching an existing chain.
    ///
    /// This is the expensive half of [`Self::maybe_reorg_to`] — it rebuilds the
    /// candidate from genesis and re-verifies every block, proof of work
    /// included — and it is deliberately a free function over borrowed facts
    /// rather than a method. A caller holding a lock around its chain can copy
    /// the work and the tip hashes, release the lock, run the rebuild, and
    /// only then come back for [`Self::adopt_reorg`].
    ///
    /// That separation is not a micro-optimisation. Measured on a live 1737
    /// block chain, the rebuild takes 26.7 seconds at 15.4 ms per block, and it
    /// grows linearly with the chain. Running it inside the node's state mutex
    /// froze everything else for that entire time: no RPC answer, no interface
    /// frame, no other peer, no block submission — and every peer thread whose
    /// blocks failed to connect started its own copy of it. A node that mined
    /// while slightly behind produced exactly that condition on every round,
    /// which is what made the wallet appear to hang and the chain appear to
    /// stop.
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
        if blocks.is_empty() {
            return Ok(None);
        }

        // Work first, and deliberately so.
        //
        // Fork choice is decided by cumulative work, so that is the question
        // worth asking, and summing declared difficulty is cheaper than
        // anything that follows. Checking length first meant a candidate could
        // be turned away for a reason unrelated to how it would have been
        // judged — and it reported `ReorgTooDeep` while doing it, which reads
        // as "this chain is suspicious" rather than "we never looked".
        let claimed_work: u128 = blocks.iter().map(|b| b.work()).sum();
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
        let common = reorg_common_prefix(our_hashes, &blocks);
        if our_hashes.len().saturating_sub(common) > MAX_REORG_DEPTH {
            return Err(ConsensusError::ReorgTooDeep);
        }

        // Shared history is ours. Only the suffix is untrusted work.
        let candidate =
            Self::rebuild_from_blocks_trusted_prefix(network, blocks, common, now_unix)?;
        if candidate.total_work > our_work {
            return Ok(Some(candidate));
        }
        Ok(None)
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
}

impl Mempool {
    /// Cap on mempool entries — an unbounded map is a trivial memory exhaustion
    /// vector for any peer.
    pub const MAX_ENTRIES: usize = 10_000;

    pub fn insert(&mut self, tx: Transaction) -> bool {
        if self.txs.len() >= Self::MAX_ENTRIES {
            return false;
        }
        self.txs.insert(tx.txid().to_hex(), tx).is_none()
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
