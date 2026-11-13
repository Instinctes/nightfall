//! Hashing proof of work on several cores must change how long validation
//! takes and nothing else: same accepted chain, same rejected block, same
//! reason.

use nightfall_consensus::*;
use nightfall_crypto::WalletKeys;
use nightfall_types::{NetworkId, TARGET_BLOCK_TIME_SECS};

const NOW: u64 = 1_800_000_000;

fn devnet() -> Chain {
    Chain::new_fair(NetworkId::Devnet).unwrap()
}

fn mined(n: u64) -> Chain {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    for i in 0..n {
        chain
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    chain
}

/// The same block with a nonce that does not meet its difficulty. Nothing
/// else about it changes: the nonce is not part of the body or the pre-image.
fn with_bad_pow(block: &Block) -> Block {
    let params = NetworkId::Devnet.pow_params();
    let mut bad = block.clone();
    for nonce in 0..100_000u64 {
        bad.header.nonce = nonce;
        if !bad.pow_is_valid(params) {
            return bad;
        }
    }
    panic!("devnet difficulty is too low to produce an invalid nonce");
}

fn later() -> u64 {
    NOW + 10_000
}

#[test]
fn parallel_verdicts_agree_with_the_sequential_check() {
    let chain = mined(9);
    let params = NetworkId::Devnet.pow_params();
    for threads in [1, 2, 4, 16] {
        let verdicts = verify_pow_parallel(&chain.blocks, params, threads);
        assert_eq!(verdicts.len(), chain.blocks.len());
        assert!(verdicts.iter().all(|v| v.is_some()), "threads={threads}");

        let mut replay = devnet();
        let applied = replay
            .try_ingest_blocks_prechecked(chain.blocks.clone(), &verdicts, later())
            .unwrap();
        assert_eq!(applied, 9, "threads={threads}");
        assert_eq!(replay.tip_hash(), chain.tip_hash());
        assert_eq!(replay.total_work, chain.total_work);
    }
}

#[test]
fn windowed_replay_matches_block_by_block_replay() {
    let chain = mined(12);

    let mut one_by_one = devnet();
    for b in chain.blocks.clone() {
        one_by_one.apply_block(b, later()).unwrap();
    }

    let mut windowed = devnet();
    let mut reports = 0;
    windowed
        .apply_blocks_parallel_pow(chain.blocks.clone(), later(), 4, |_| reports += 1)
        .unwrap();

    assert!(reports >= 1);
    assert_eq!(windowed.tip_hash(), one_by_one.tip_hash());
    assert_eq!(windowed.total_work, one_by_one.total_work);
    assert_eq!(windowed.ledger.utxo_root(), one_by_one.ledger.utxo_root());
    assert_eq!(windowed.ledger.kernel_sum(), one_by_one.ledger.kernel_sum());
    windowed.verify_supply().unwrap();
}

#[test]
fn a_block_with_invalid_work_is_rejected_at_its_own_index() {
    let chain = mined(8);
    let mut blocks = chain.blocks.clone();
    blocks[5] = with_bad_pow(&blocks[5]);

    let mut sequential = devnet();
    let mut sequential_err = None;
    for (i, b) in blocks.clone().into_iter().enumerate() {
        if let Err(e) = sequential.apply_block(b, later()) {
            sequential_err = Some((i, e.to_string()));
            break;
        }
    }

    let mut windowed = devnet();
    let (index, err) = windowed
        .apply_blocks_parallel_pow(blocks, later(), 4, |_| {})
        .unwrap_err();

    assert!(matches!(err, ConsensusError::BadPow), "{err}");
    assert_eq!(Some((index, err.to_string())), sequential_err);
    assert_eq!(index, 5);
    // Everything before the bad block stays applied, nothing after it.
    assert_eq!(windowed.block_count(), 5);
    assert_eq!(windowed.tip_hash(), sequential.tip_hash());
}

#[test]
fn hashing_stops_at_the_first_invalid_block() {
    let chain = mined(8);
    let mut blocks = chain.blocks.clone();
    blocks[2] = with_bad_pow(&blocks[2]);
    let verdicts = verify_pow_parallel(&blocks, NetworkId::Devnet.pow_params(), 1);
    // One worker: 0, 1 and 2 are hashed, then it stops.
    assert!(verdicts[..3].iter().all(|v| v.is_some()));
    assert!(verdicts[3..].iter().all(|v| v.is_none()));
}

#[test]
fn a_verdict_for_another_block_buys_nothing() {
    let chain = mined(4);
    let params = NetworkId::Devnet.pow_params();
    let good = PowVerdict::compute(&chain.blocks[3], params);
    let bad_block = with_bad_pow(&chain.blocks[3]);

    let mut replay = devnet();
    for b in chain.blocks[..3].iter().cloned() {
        replay.apply_block(b, later()).unwrap();
    }
    // The verdict says "valid", but it is about the honest block. Offered
    // with the tampered one it is ignored and the block is hashed again.
    let err = replay
        .apply_block_prechecked(bad_block, later(), Some(&good))
        .unwrap_err();
    assert!(matches!(err, ConsensusError::BadPow), "{err}");
    assert_eq!(replay.block_count(), 3);

    // A verdict computed for another network's parameters is ignored too.
    let foreign = PowVerdict::compute(&chain.blocks[3], NetworkId::Testnet.pow_params());
    replay
        .apply_block_prechecked(chain.blocks[3].clone(), later(), Some(&foreign))
        .unwrap();
    assert_eq!(replay.tip_hash(), chain.tip_hash());
}

#[test]
fn a_batch_that_breaks_its_own_chain_is_only_hashed_up_to_the_break() {
    let chain = mined(6);
    let mut blocks = chain.blocks.clone();
    blocks.swap(3, 4);
    let verdicts = precheck_pow(&blocks, NetworkId::Devnet.pow_params(), 4);
    assert_eq!(verdicts.len(), 3);

    let mut replay = devnet();
    let applied = replay
        .try_ingest_blocks_prechecked(blocks, &verdicts, later())
        .unwrap();
    assert_eq!(applied, 3);
}

#[test]
fn a_window_that_does_not_extend_the_tip_is_rejected_without_hashing() {
    let chain = mined(5);
    let mut replay = devnet();
    // Skips block 0: the first block does not connect.
    let (index, err) = replay
        .apply_blocks_parallel_pow(chain.blocks[1..].to_vec(), later(), 4, |_| {})
        .unwrap_err();
    assert_eq!(index, 0);
    assert!(
        matches!(err, ConsensusError::BadHeight | ConsensusError::BadPrev),
        "{err}"
    );
    assert_eq!(replay.block_count(), 0);
}

#[test]
fn a_foreign_file_without_a_checkpoint_is_validated_in_full() {
    // Devnet carries no checkpoints, so nothing may be skipped: a file with
    // one block of invalid work is refused at that block.
    let chain = mined(7);

    let mut ok = devnet();
    let mut last_seen = 0;
    ok.replay_foreign_blocks(chain.blocks.clone(), later(), 4, |c| {
        last_seen = c.block_count()
    })
    .unwrap();
    assert_eq!(ok.tip_hash(), chain.tip_hash());
    assert_eq!(last_seen, 7);

    let mut blocks = chain.blocks.clone();
    blocks[1] = with_bad_pow(&blocks[1]);
    let mut refused = devnet();
    let (index, err) = refused
        .replay_foreign_blocks(blocks, later(), 4, |_| {})
        .unwrap_err();
    assert_eq!(index, 1);
    assert!(matches!(err, ConsensusError::BadPow), "{err}");
}
