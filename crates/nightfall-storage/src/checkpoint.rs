use nightfall_consensus::{BlockHeader, Chain, CompactHeader, EmissionSchedule};
use nightfall_crypto::Commitment;
use nightfall_ledger::{
    coinbase_maturity, BlockBody, KernelAccumulator, LedgerState, SupplyState, UtxoEntry, UtxoSet,
};
use nightfall_types::{Hash256, Height, NetworkId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// State bootstrap anchored to a checkpoint compiled into the node.
///
/// `UtxoSet::root()` currently commits to commitment, output key and creation
/// height, but not `is_coinbase`. Outputs that may still be subject to
/// coinbase maturity therefore carry an authenticated creation body. Older
/// outputs are already mature forever and may safely be normalised to
/// `is_coinbase = false`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub v: u32,
    pub network: NetworkId,
    pub checkpoint_height: u64,
    pub headers: Vec<BlockHeader>,
    pub recent_bodies: Vec<CheckpointBody>,
    pub burned_darks: u64,
    pub utxos: Vec<CheckpointUtxo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointBody {
    pub height: u64,
    pub body: BlockBody,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointUtxo {
    pub commit: String,
    pub output_pk: String,
    pub height: u64,
}

/// Rebuild checkpoint state using the newest checkpoint compiled into this
/// binary.
///
/// This intentionally accepts only mainnet. Tests use the private helper with
/// a synthetic pin so they do not depend on mainnet data.
pub fn chain_from_checkpoint_snapshot(
    snapshot: CheckpointSnapshot,
    network: NetworkId,
) -> anyhow::Result<Chain> {
    if std::env::var("NIGHTFALL_NO_ASSUME_VALID").is_ok() {
        anyhow::bail!("checkpoint state bootstrap is disabled by NIGHTFALL_NO_ASSUME_VALID");
    }

    if network != NetworkId::Mainnet {
        anyhow::bail!("checkpoint state bootstrap is mainnet-only");
    }

    let height = nightfall_types::highest_checkpoint_height();
    if height == 0 {
        anyhow::bail!("this build has no checkpoint to anchor a state snapshot");
    }

    let pinned = nightfall_types::checkpoint_at(height)
        .ok_or_else(|| anyhow::anyhow!("highest checkpoint has no pinned hash"))?;

    chain_from_checkpoint_snapshot_with_pin(snapshot, network, height, pinned)
}

/// Re-bind archive bodies to the headers that authenticate them before a
/// trusted local replay is used to construct portable checkpoint state.
///
/// `apply_block_from_own_disk` deliberately skips this check because its
/// normal caller already authenticated the local file. Export is different:
/// it turns local bytes into an artifact another installation may consume.
/// Re-checking the cheap body commitment here prevents post-validation local
/// corruption from being copied into a snapshot.
fn checked_maturity_end(height: u64, maturity: u64) -> anyhow::Result<u64> {
    height
        .checked_add(maturity)
        .ok_or_else(|| anyhow::anyhow!("coinbase maturity height overflow"))
}

fn validate_archive_prefix_bodies(blocks: &[nightfall_consensus::Block]) -> anyhow::Result<()> {
    for block in blocks {
        if !block.body.is_canonical() {
            anyhow::bail!(
                "archive block {} has a non-canonical body",
                block.header.height.0
            );
        }

        if block.body.hash() != block.header.body_root {
            anyhow::bail!(
                "archive block {} body does not match its header body root",
                block.header.height.0
            );
        }
    }

    Ok(())
}

/// Build a portable state snapshot at the newest checkpoint compiled into
/// this binary.
///
/// The input chain must come from normal `ChainStore` loading. That means its
/// archive bodies were either validated during this load or already belonged
/// to this installation's authenticated validation record.
///
/// Replaying the prefix here reconstructs historical ledger state only. It is
/// not a new trust decision.
pub fn checkpoint_snapshot_from_chain(chain: &Chain) -> anyhow::Result<CheckpointSnapshot> {
    if chain.network != NetworkId::Mainnet {
        anyhow::bail!("checkpoint state snapshots are mainnet-only");
    }

    if chain.first_height != 0 {
        anyhow::bail!(
            "checkpoint state export requires an archive datadir with bodies from genesis"
        );
    }

    let checkpoint_height = nightfall_types::highest_checkpoint_height();
    if checkpoint_height == 0 {
        anyhow::bail!("this build has no checkpoint to export");
    }

    let pinned = nightfall_types::checkpoint_at(checkpoint_height)
        .ok_or_else(|| anyhow::anyhow!("highest checkpoint has no pinned hash"))?;

    let required = checkpoint_height
        .checked_add(1)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("checkpoint height does not fit this platform"))?;

    if chain.blocks.len() < required {
        anyhow::bail!(
            "archive reaches only {} blocks; checkpoint export needs {}",
            chain.blocks.len(),
            required
        );
    }

    if chain.blocks[checkpoint_height as usize].hash().to_hex() != pinned {
        anyhow::bail!(
            "local archive does not contain the compiled checkpoint hash at height {}",
            checkpoint_height
        );
    }

    let prefix = chain.blocks[..required].to_vec();
    validate_archive_prefix_bodies(&prefix)?;

    /*
     * All blocks in `prefix` came from a ChainStore load whose trust decision
     * was already established. The own-disk path avoids repeating expensive
     * cryptography while reconstructing the checkpoint state. The exact
     * checkpoint hash, ledger roots and supply equation are checked again.
     */
    let rebuilt =
        Chain::rebuild_from_blocks_trusted_prefix(chain.network, prefix.clone(), required, 0)
            .map_err(|e| anyhow::anyhow!("checkpoint replay failed: {e}"))?;

    if rebuilt.tip_hash().to_hex() != pinned {
        anyhow::bail!(
            "checkpoint replay produced {}, expected {}",
            rebuilt.tip_hash(),
            pinned
        );
    }

    rebuilt
        .verify_supply()
        .map_err(|e| anyhow::anyhow!("checkpoint replay violates supply invariant: {e}"))?;

    let next_height = checkpoint_height
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("checkpoint height overflow"))?;
    let maturity = coinbase_maturity(chain.network);

    /*
     * V8's UTXO root does not commit to `is_coinbase`. Preserve creation
     * bodies only for surviving outputs whose maturity can still matter at
     * the first post-checkpoint height. Their bodies are authenticated by the
     * corresponding full header's body_root.
     */
    let mut needed_heights = BTreeSet::new();
    for entry in rebuilt.ledger.utxos.entries.values() {
        let maturity_end = checked_maturity_end(entry.height, maturity)?;
        if next_height < maturity_end {
            needed_heights.insert(entry.height);
        }
    }

    Ok(CheckpointSnapshot {
        v: 1,
        network: chain.network,
        checkpoint_height,
        headers: prefix.iter().map(|block| block.header.clone()).collect(),
        recent_bodies: prefix
            .iter()
            .filter(|block| needed_heights.contains(&block.header.height.0))
            .map(|block| CheckpointBody {
                height: block.header.height.0,
                body: block.body.clone(),
            })
            .collect(),
        burned_darks: rebuilt.ledger.supply.total_burned_darks,
        utxos: rebuilt
            .ledger
            .utxos
            .entries
            .iter()
            .map(|(commit, entry)| CheckpointUtxo {
                commit: Hash256(*commit).to_hex(),
                output_pk: Hash256(entry.output_pk).to_hex(),
                height: entry.height,
            })
            .collect(),
    })
}

fn chain_from_checkpoint_snapshot_with_pin(
    snapshot: CheckpointSnapshot,
    network: NetworkId,
    checkpoint_height: u64,
    pinned_hash: &str,
) -> anyhow::Result<Chain> {
    if snapshot.v != 1 {
        anyhow::bail!("unsupported checkpoint snapshot version {}", snapshot.v);
    }

    if snapshot.network != network {
        anyhow::bail!(
            "checkpoint snapshot is {:?}, but {:?} was requested",
            snapshot.network,
            network
        );
    }

    if snapshot.checkpoint_height != checkpoint_height {
        anyhow::bail!(
            "snapshot is anchored at height {}, build expects {}",
            snapshot.checkpoint_height,
            checkpoint_height
        );
    }

    let expected_headers = checkpoint_height
        .checked_add(1)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("checkpoint height does not fit this platform"))?;

    if snapshot.headers.len() != expected_headers {
        anyhow::bail!(
            "checkpoint snapshot has {} headers, expected {}",
            snapshot.headers.len(),
            expected_headers
        );
    }

    /*
     * Full headers are intentional.
     *
     * A CompactHeader contains a supplied hash but not enough fields to
     * recompute that hash. Full BlockHeader values let us recompute every
     * link up to the pinned final hash. Changing timestamp, difficulty,
     * body_root, UTXO root, kernel sum or nonce therefore changes the chain
     * and cannot still arrive at the compiled checkpoint.
     */
    let mut chain = Chain::new_fair(network)?;
    let mut expected_prev = chain.genesis_hash;
    let mut compact_headers = Vec::with_capacity(snapshot.headers.len());

    for (i, header) in snapshot.headers.iter().enumerate() {
        let height = i as u64;

        if header.height.0 != height {
            anyhow::bail!(
                "checkpoint header index {i} carries height {}",
                header.height.0
            );
        }

        if header.prev_hash != expected_prev {
            anyhow::bail!("checkpoint header {height} does not link to its predecessor");
        }

        let hash = header.hash();

        compact_headers.push(CompactHeader {
            height,
            hash,
            prev_hash: header.prev_hash,
            timestamp_unix: header.timestamp_unix,
            difficulty: header.difficulty,
        });

        expected_prev = hash;
    }

    let tip = snapshot
        .headers
        .last()
        .ok_or_else(|| anyhow::anyhow!("checkpoint snapshot contains no headers"))?;

    let got_tip = tip.hash().to_hex();

    if got_tip != pinned_hash {
        anyhow::bail!(
            "checkpoint hash mismatch at height {checkpoint_height}: \
             expected {pinned_hash}, got {got_tip}"
        );
    }

    /*
     * Only bodies needed to recover still-relevant OutputFeature values are
     * retained. Their body hashes are authenticated by the checkpointed
     * header chain.
     */
    let mut bodies = BTreeMap::new();

    for item in snapshot.recent_bodies {
        if item.height > checkpoint_height {
            anyhow::bail!("snapshot body {} is past the checkpoint", item.height);
        }

        if !item.body.is_canonical() {
            anyhow::bail!("snapshot body {} is not canonical", item.height);
        }

        let header = snapshot.headers.get(item.height as usize).ok_or_else(|| {
            anyhow::anyhow!("snapshot body {} has no matching header", item.height)
        })?;

        if item.body.hash() != header.body_root {
            anyhow::bail!(
                "snapshot body {} does not match its header body root",
                item.height
            );
        }

        if bodies.insert(item.height, item.body).is_some() {
            anyhow::bail!("duplicate snapshot body at height {}", item.height);
        }
    }

    let maturity = coinbase_maturity(network);
    let next_height = checkpoint_height
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("checkpoint height overflow"))?;
    let mut utxos = UtxoSet::new();

    for item in snapshot.utxos {
        if item.height > checkpoint_height {
            anyhow::bail!(
                "UTXO created at height {} is past checkpoint {}",
                item.height,
                checkpoint_height
            );
        }

        let commit_hash = Hash256::from_hex(&item.commit)
            .map_err(|_| anyhow::anyhow!("bad commitment in checkpoint UTXO"))?;

        let output_pk = Hash256::from_hex(&item.output_pk)
            .map_err(|_| anyhow::anyhow!("bad output key in checkpoint UTXO"))?;

        let commit = Commitment(commit_hash.0);

        /*
         * At the first post-checkpoint spend height, an older coinbase is
         * already mature forever. Only younger outputs need OutputFeature
         * authenticated from their creation block.
         */
        let maturity_end = checked_maturity_end(item.height, maturity)?;
        let maturity_sensitive = next_height < maturity_end;

        let is_coinbase = if maturity_sensitive {
            let body = bodies.get(&item.height).ok_or_else(|| {
                anyhow::anyhow!(
                    "UTXO at height {} is still maturity-sensitive but its body is missing",
                    item.height
                )
            })?;

            let output = body
                .outputs
                .iter()
                .find(|o| o.commit == commit && o.output_pk == output_pk.0)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "UTXO at height {} is not present in its authenticated body",
                        item.height
                    )
                })?;

            output.features.is_coinbase()
        } else {
            false
        };

        if !utxos.insert(
            commit,
            UtxoEntry {
                output_pk: output_pk.0,
                height: item.height,
                is_coinbase,
            },
        ) {
            anyhow::bail!("duplicate commitment in checkpoint UTXO set");
        }
    }

    /*
     * The root authenticates the exact surviving commitments, ownership keys
     * and creation heights against the pinned header.
     */
    if utxos.root() != tip.utxo_root {
        anyhow::bail!("checkpoint UTXO root does not match checkpoint header");
    }

    /*
     * Minted supply is deterministic consensus state. Do not accept it from
     * the snapshot.
     */
    let emission = EmissionSchedule::locked_mainnet();
    let mut minted = 0u64;

    for h in 0..=checkpoint_height {
        let reward = emission.reward_at(Height(h), minted).darks();

        minted = minted
            .checked_add(reward)
            .ok_or_else(|| anyhow::anyhow!("minted supply overflow"))?;
    }

    if snapshot.burned_darks > minted {
        anyhow::bail!("snapshot burned supply exceeds minted supply");
    }

    /*
     * `burned_darks` is not trusted merely because the snapshot says so.
     *
     * verify_supply() below proves:
     *
     *   ΣUTXO - Σkernel_excess == (minted - burned) * G
     *
     * UTXOs are root-bound, kernel_sum is header-bound, and minted is
     * deterministic. Therefore a forged burned value cannot pass.
     */
    let ledger = LedgerState {
        height: Height(checkpoint_height),
        utxos,
        kernels: KernelAccumulator {
            sum: tip.kernel_sum,

            /*
             * Kernel count is informational in the current protocol.
             * Consensus uses the cryptographic running sum.
             */
            count: 0,
        },
        supply: SupplyState {
            total_minted_darks: minted,
            total_burned_darks: snapshot.burned_darks,
        },

        /*
         * Historical transaction count is also informational and does not
         * participate in block acceptance.
         */
        tx_count: 0,
        coinbase_maturity: maturity,
    };

    ledger
        .verify_supply()
        .map_err(|e| anyhow::anyhow!("checkpoint state violates supply invariant: {e}"))?;

    if ledger.utxo_root() != tip.utxo_root {
        anyhow::bail!("checkpoint ledger UTXO root mismatch");
    }

    if ledger.kernel_sum() != tip.kernel_sum {
        anyhow::bail!("checkpoint ledger kernel sum mismatch");
    }

    let horizon_work = compact_headers.iter().try_fold(0u128, |work, header| {
        work.checked_add(header.work())
            .ok_or_else(|| anyhow::anyhow!("checkpoint work overflow"))
    })?;

    /*
     * The checkpoint state is exactly the state before the first body we will
     * download normally. This is the same representation already used by
     * pruned chains.
     */
    chain.ledger = ledger.clone();
    chain.horizon = Some(ledger);
    chain.horizon_work = horizon_work;
    chain.total_work = horizon_work;
    chain.headers = compact_headers;
    chain.blocks.clear();
    chain.first_height = checkpoint_height
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("checkpoint height overflow"))?;

    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::now_unix;

    fn make_checkpoint_test_chain() -> Chain {
        use nightfall_crypto::WalletKeys;

        let miner = WalletKeys::generate().address();
        let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

        for i in 0..12u64 {
            chain
                .mine_block(&miner, vec![], now_unix() + i * 15)
                .unwrap();
        }

        chain
    }

    fn snapshot_for(chain: &Chain) -> CheckpointSnapshot {
        let checkpoint_height = chain.tip_height().unwrap().0;
        let next_height = checkpoint_height + 1;
        let maturity = coinbase_maturity(chain.network);

        let needed_heights: std::collections::BTreeSet<u64> = chain
            .ledger
            .utxos
            .entries
            .values()
            .filter(|entry| next_height < entry.height.saturating_add(maturity))
            .map(|entry| entry.height)
            .collect();

        CheckpointSnapshot {
            v: 1,
            network: chain.network,
            checkpoint_height,

            headers: chain.blocks.iter().map(|b| b.header.clone()).collect(),

            recent_bodies: chain
                .blocks
                .iter()
                .filter(|b| needed_heights.contains(&b.header.height.0))
                .map(|b| CheckpointBody {
                    height: b.header.height.0,
                    body: b.body.clone(),
                })
                .collect(),

            burned_darks: chain.ledger.supply.total_burned_darks,

            utxos: chain
                .ledger
                .utxos
                .entries
                .iter()
                .map(|(commit, entry)| CheckpointUtxo {
                    commit: Hash256(*commit).to_hex(),
                    output_pk: Hash256(entry.output_pk).to_hex(),
                    height: entry.height,
                })
                .collect(),
        }
    }

    #[test]
    fn checkpoint_snapshot_rebuilds_same_consensus_state() {
        let chain = make_checkpoint_test_chain();
        let height = chain.tip_height().unwrap().0;
        let pin = chain.tip_hash().to_hex();
        let snapshot = snapshot_for(&chain);

        let rebuilt =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap();

        assert_eq!(rebuilt.tip_hash(), chain.tip_hash());

        assert_eq!(rebuilt.ledger.utxo_root(), chain.ledger.utxo_root());

        assert_eq!(rebuilt.ledger.kernel_sum(), chain.ledger.kernel_sum());

        assert_eq!(
            rebuilt.ledger.supply.total_minted_darks,
            chain.ledger.supply.total_minted_darks
        );

        assert_eq!(rebuilt.next_difficulty(), chain.next_difficulty());

        assert_eq!(rebuilt.median_time_past(), chain.median_time_past());

        assert_eq!(rebuilt.first_height, height + 1);

        assert!(rebuilt.blocks.is_empty());
    }

    #[test]
    fn checkpoint_snapshot_rejects_tampered_recent_feature() {
        use nightfall_crypto::OutputFeature;

        let chain = make_checkpoint_test_chain();
        let height = chain.tip_height().unwrap().0;
        let pin = chain.tip_hash().to_hex();
        let mut snapshot = snapshot_for(&chain);

        let body = snapshot.recent_bodies.last_mut().unwrap();

        body.body.outputs[0].features = OutputFeature::Plain;

        let err =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap_err();

        assert!(err.to_string().contains("body root"), "{err}");
    }

    #[test]
    fn checkpoint_snapshot_rejects_missing_maturity_body() {
        let chain = make_checkpoint_test_chain();
        let height = chain.tip_height().unwrap().0;
        let pin = chain.tip_hash().to_hex();
        let mut snapshot = snapshot_for(&chain);

        snapshot.recent_bodies.clear();

        let err =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap_err();

        assert!(err.to_string().contains("maturity-sensitive"), "{err}");
    }

    #[test]
    fn checkpoint_snapshot_rejects_fake_burned_supply() {
        let chain = make_checkpoint_test_chain();
        let height = chain.tip_height().unwrap().0;
        let pin = chain.tip_hash().to_hex();
        let mut snapshot = snapshot_for(&chain);

        snapshot.burned_darks = snapshot.burned_darks.saturating_add(1);

        let err =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap_err();

        assert!(err.to_string().contains("supply invariant"), "{err}");
    }

    #[test]
    fn checkpoint_snapshot_rejects_header_chain_tampering() {
        let chain = make_checkpoint_test_chain();
        let height = chain.tip_height().unwrap().0;
        let pin = chain.tip_hash().to_hex();
        let mut snapshot = snapshot_for(&chain);

        snapshot.headers[1].timestamp_unix = snapshot.headers[1].timestamp_unix.saturating_add(1);

        let err =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap_err();

        assert!(err.to_string().contains("does not link"), "{err}");
    }

    #[test]
    fn checkpoint_snapshot_can_continue_with_fully_validated_blocks() {
        use nightfall_crypto::WalletKeys;

        let miner = WalletKeys::generate().address();
        let mut full = Chain::new_fair(NetworkId::Devnet).unwrap();
        let started = now_unix();

        for i in 0..15u64 {
            full.mine_block(&miner, vec![], started + i * 15).unwrap();
        }

        /*
         * Pretend height 11 is our compiled checkpoint. Heights 12..14
         * represent blocks received normally from peers after bootstrap.
         */
        let prefix_blocks = full.blocks[..12].to_vec();
        let suffix_blocks = full.blocks[12..].to_vec();

        let prefix =
            Chain::rebuild_from_blocks(NetworkId::Devnet, prefix_blocks, started + 15 * 20)
                .unwrap();

        let checkpoint_height = prefix.tip_height().unwrap().0;
        assert_eq!(checkpoint_height, 11);

        let pin = prefix.tip_hash().to_hex();
        let snapshot = snapshot_for(&prefix);

        let mut bootstrapped = chain_from_checkpoint_snapshot_with_pin(
            snapshot,
            NetworkId::Devnet,
            checkpoint_height,
            &pin,
        )
        .unwrap();

        assert_eq!(bootstrapped.next_height().0, checkpoint_height + 1);

        /*
         * These blocks use the ordinary untrusted path:
         * PoW, difficulty, timestamp, body, ledger and roots all run.
         */
        for block in suffix_blocks {
            let now = block.header.timestamp_unix;

            bootstrapped.apply_block(block, now).unwrap();
        }

        bootstrapped.verify_supply().unwrap();

        assert_eq!(bootstrapped.tip_hash(), full.tip_hash());

        assert_eq!(bootstrapped.block_count(), full.block_count());

        assert_eq!(bootstrapped.ledger.utxo_root(), full.ledger.utxo_root());

        assert_eq!(bootstrapped.ledger.kernel_sum(), full.ledger.kernel_sum());

        assert_eq!(
            bootstrapped.ledger.supply.total_minted_darks,
            full.ledger.supply.total_minted_darks
        );

        assert_eq!(
            bootstrapped.ledger.supply.total_burned_darks,
            full.ledger.supply.total_burned_darks
        );

        assert_eq!(bootstrapped.total_work, full.total_work);

        assert_eq!(bootstrapped.next_difficulty(), full.next_difficulty());

        assert_eq!(bootstrapped.median_time_past(), full.median_time_past());
    }

    #[test]
    fn checkpoint_snapshot_zero_body_state_survives_restart() {
        let source = make_checkpoint_test_chain();
        let height = source.tip_height().unwrap().0;
        let pin = source.tip_hash().to_hex();
        let snapshot = snapshot_for(&source);

        let rebuilt =
            chain_from_checkpoint_snapshot_with_pin(snapshot, NetworkId::Devnet, height, &pin)
                .unwrap();

        assert_eq!(rebuilt.first_height, height + 1);
        assert!(rebuilt.blocks.is_empty());
        assert_eq!(rebuilt.block_count(), height + 1);

        let dir = std::env::temp_dir().join(format!(
            "nf-checkpoint-restart-{}-{}",
            std::process::id(),
            now_unix()
        ));

        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::ChainStore::new(&dir);

        store.save(&rebuilt).unwrap();

        assert_eq!(
            std::fs::metadata(store.blocks_path()).unwrap().len(),
            0,
            "a checkpoint with no post-checkpoint blocks must have an empty body file"
        );

        assert!(
            store.is_own_file_trusted(),
            "a locally verified bodyless checkpoint state must survive restart"
        );

        let loaded = store.load_or_new(NetworkId::Devnet).unwrap();

        assert_eq!(loaded.tip_hash(), source.tip_hash());
        assert_eq!(loaded.tip_height(), source.tip_height());
        assert_eq!(loaded.first_height, height + 1);
        assert!(loaded.blocks.is_empty());

        assert_eq!(loaded.ledger.utxo_root(), source.ledger.utxo_root());
        assert_eq!(loaded.ledger.kernel_sum(), source.ledger.kernel_sum());
        assert_eq!(
            loaded.ledger.supply.total_minted_darks,
            source.ledger.supply.total_minted_darks
        );
        assert_eq!(
            loaded.ledger.supply.total_burned_darks,
            source.ledger.supply.total_burned_darks
        );

        assert_eq!(loaded.total_work, source.total_work);
        assert_eq!(loaded.next_difficulty(), source.next_difficulty());
        assert_eq!(loaded.median_time_past(), source.median_time_past());

        loaded.verify_supply().unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn checkpoint_export_rejects_body_not_matching_header_root() {
        let chain = make_checkpoint_test_chain();
        let mut prefix = chain.blocks.clone();

        assert!(!prefix.is_empty());
        prefix[0].header.body_root.0[0] ^= 1;

        let err = validate_archive_prefix_bodies(&prefix)
            .expect_err("mismatched archive body/header commitment must fail");

        assert!(
            err.to_string().contains("body does not match"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn maturity_height_overflow_fails_closed() {
        let err =
            checked_maturity_end(u64::MAX, 1).expect_err("maturity arithmetic must not saturate");

        assert!(
            err.to_string().contains("maturity height overflow"),
            "unexpected error: {err}"
        );
    }
}
