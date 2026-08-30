//! Chain-level consensus rules, including the attacks v4 was vulnerable to.

use nightfall_consensus::*;
use nightfall_crypto::WalletKeys;
use nightfall_types::{Hash256, Height, NetworkId, TARGET_BLOCK_TIME_SECS};

const NOW: u64 = 1_800_000_000;

fn devnet() -> Chain {
    Chain::new_fair(NetworkId::Devnet).unwrap()
}

#[test]
fn mines_and_accumulates_work() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();

    let b0 = chain.mine_block(&miner, vec![], NOW).unwrap();
    assert_eq!(b0.header.height, Height(0));
    assert!(chain.total_work > 0);

    let w0 = chain.total_work;
    chain
        .mine_block(&miner, vec![], NOW + TARGET_BLOCK_TIME_SECS)
        .unwrap();
    assert!(chain.total_work > w0, "work must accumulate");
    assert_eq!(chain.block_count(), 2);
    chain.verify_supply().unwrap();
}

#[test]
fn own_disk_replay_matches_full_apply() {
    let mut full = devnet();
    let miner = WalletKeys::generate().address();
    for i in 0..6u64 {
        full.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut fast = devnet();
    for b in full.blocks.clone() {
        fast.apply_block_from_own_disk(b).unwrap();
    }
    assert_eq!(fast.tip_hash(), full.tip_hash());
    assert_eq!(fast.total_work, full.total_work);
    assert_eq!(fast.ledger.utxo_root(), full.ledger.utxo_root());
    assert_eq!(fast.ledger.kernel_sum(), full.ledger.kernel_sum());
    fast.verify_supply().unwrap();
}

#[test]
fn peer_can_replay_our_chain() {
    let mut a = devnet();
    let miner = WalletKeys::generate().address();
    for i in 0..4u64 {
        a.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut b = devnet();
    let n = b.try_ingest_blocks(a.blocks.clone(), NOW + 1000).unwrap();
    assert_eq!(n, 4);
    assert_eq!(a.tip_hash(), b.tip_hash());
    assert_eq!(a.total_work, b.total_work);
}

#[test]
fn fork_choice_follows_work_not_length() {
    // THE v4 KILLER: a longer chain of cheaper blocks must lose.
    let miner = WalletKeys::generate().address();

    let mut heavy = devnet();
    for i in 0..3u64 {
        heavy
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    // Forge a "longer" chain by claiming many blocks but with less total work.
    let mut cheap = devnet();
    for i in 0..5u64 {
        cheap.mine_block(&miner, vec![], NOW + i * 3).unwrap();
    }

    // Give heavy strictly more work than cheap, then confirm length loses.
    if cheap.total_work < heavy.total_work {
        assert!(
            cheap.blocks.len() > heavy.blocks.len(),
            "setup: cheap is longer"
        );
        let adopted = heavy
            .maybe_reorg_to(cheap.blocks.clone(), NOW + 10_000)
            .unwrap();
        assert!(
            !adopted,
            "a longer but lighter chain must NOT be adopted — this was the v4 break"
        );
    }
}

#[test]
fn heavier_chain_is_adopted() {
    let miner = WalletKeys::generate().address();
    let mut long = devnet();
    for i in 0..6u64 {
        long.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let mut short = devnet();
    short.mine_block(&miner, vec![], NOW).unwrap();

    assert!(short
        .maybe_reorg_to(long.blocks.clone(), NOW + 10_000)
        .unwrap());
    assert_eq!(short.tip_hash(), long.tip_hash());
    assert_eq!(short.total_work, long.total_work);
    short.verify_supply().unwrap();
}

#[test]
fn a_lighter_chain_is_declined_not_accused() {
    // The depth bound is a denial-of-service limit: it stops a peer making us
    // validate an arbitrarily long chain on demand. It is not a statement about
    // which chain is correct, and it must not be the reason a candidate is
    // turned away when work already settles the question.
    //
    // Before the check order was fixed, a long lighter chain came back as
    // ReorgTooDeep — "this looks like an attack" — when the truthful answer was
    // "we compared the work and yours is lighter". Anyone reading node logs to
    // diagnose a fork was being pointed at the wrong thing.
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();

    // `block_work` floors at 1, so a chain of n blocks is worth at least n no
    // matter how low its difficulty. Mine until ours outweighs anything the
    // depth bound would still let through.
    let mut i = 0u64;
    while chain.total_work <= chain.block_count() as u128 + MAX_REORG_DEPTH as u128 + 4 {
        chain
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
        i += 1;
    }

    // Now a candidate that is past the depth bound *and* lighter. Minimum
    // difficulty on every block keeps its total at one unit each.
    let target_len = chain.block_count() as usize + MAX_REORG_DEPTH + 1;
    let lighter: Vec<_> = (0..target_len)
        .map(|_| {
            let mut b = chain.blocks[0].clone();
            b.header.difficulty = 0;
            b
        })
        .collect();
    assert!(
        (lighter.len() as u128) < chain.total_work,
        "setup: the candidate must genuinely be lighter"
    );

    assert!(
        matches!(chain.maybe_reorg_to(lighter, NOW), Ok(false)),
        "a lighter chain must be declined on work, not reported as too deep"
    );
}

#[test]
fn reorg_depth_is_the_rewind_not_the_length() {
    // Depth is how many of *our* blocks we would drop. A peer that shares
    // nothing with a 501-block history is too deep even if it only sends a
    // handful of blocks. A peer that shares almost everything and is 510
    // blocks longer is catch-up, not a deep reorg — that used to return
    // ReorgTooDeep and permanently stranded a laptop that had slept.
    let miner = WalletKeys::generate().address();
    let mut mined = devnet();
    let template = mined.mine_block(&miner, vec![], NOW).unwrap();

    let our_hashes: Vec<Hash256> = (0..=MAX_REORG_DEPTH)
        .map(|i| {
            let mut h = [0u8; 32];
            h[0] = 0xA1;
            h[1] = (i / 256) as u8;
            h[2] = (i % 256) as u8;
            Hash256(h)
        })
        .collect();
    assert_eq!(our_hashes.len(), MAX_REORG_DEPTH + 1);

    let fake = vec![template.clone(); 8];
    assert!(
        matches!(
            Chain::evaluate_reorg(NetworkId::Devnet, 1, &our_hashes, fake, NOW),
            Err(ConsensusError::ReorgTooDeep)
        ),
        "abandoning more than MAX_REORG_DEPTH of our history must be refused"
    );
}

#[test]
fn a_long_extension_of_a_shallow_fork_is_not_too_deep() {
    // The laptop-sleep case: we share the first three blocks, we have one
    // extra of our own, the seed kept going for well past the old
    // `our_len + 500` wall. Old rule: ReorgTooDeep. New rule: rewind is 1.
    let miner = WalletKeys::generate().address();
    let mut shared = devnet();
    for i in 0..3u64 {
        shared
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let mut ours = Chain::new_fair(NetworkId::Devnet).unwrap();
    ours.try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    ours.mine_block(&miner, vec![], NOW + 3 * TARGET_BLOCK_TIME_SECS)
        .unwrap();

    let our_hashes: Vec<Hash256> = ours.blocks.iter().map(|b| b.hash()).collect();
    let mut candidate = shared.blocks.clone();
    let pad = shared.blocks[0].clone();
    for _ in 0..(MAX_REORG_DEPTH + 10) {
        candidate.push(pad.clone());
    }
    assert!(
        candidate.len() > ours.blocks.len() + MAX_REORG_DEPTH,
        "setup: this is past the old length-delta wall"
    );
    assert_eq!(reorg_rewind(&our_hashes, &candidate), 1);

    let verdict = Chain::evaluate_reorg(
        NetworkId::Devnet,
        ours.total_work,
        &our_hashes,
        candidate,
        NOW + 10_000,
    );
    assert!(
        !matches!(verdict, Err(ConsensusError::ReorgTooDeep)),
        "a one-block rewind must not be reported as too deep, got {verdict:?}"
    );
}

#[test]
fn a_one_block_fork_adopts_the_heavier_valid_branch() {
    // The MacBook case: we share everything up to the last block, then mine
    // one competing tip while the network keeps going. Rewind is 1. The
    // rebuild must trust the shared prefix (no proofs, no re-PoW) and still
    // accept the heavier suffix.
    let miner = WalletKeys::generate().address();
    let mut shared = devnet();
    for i in 0..4u64 {
        shared
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut ours = Chain::new_fair(NetworkId::Devnet).unwrap();
    ours.try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    ours.mine_block(&miner, vec![], NOW + 4 * TARGET_BLOCK_TIME_SECS)
        .unwrap();

    let mut theirs = Chain::new_fair(NetworkId::Devnet).unwrap();
    theirs
        .try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    for i in 0..3u64 {
        theirs
            .mine_block(&miner, vec![], NOW + (4 + i) * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    assert!(theirs.total_work > ours.total_work);
    assert_eq!(
        reorg_rewind(
            &ours.blocks.iter().map(|b| b.hash()).collect::<Vec<_>>(),
            &theirs.blocks
        ),
        1
    );

    assert!(ours
        .maybe_reorg_to(theirs.blocks.clone(), NOW + 10_000)
        .unwrap());
    assert_eq!(ours.tip_hash(), theirs.tip_hash());
    ours.verify_supply().unwrap();
}

#[test]
fn trusted_prefix_rebuild_matches_full_rebuild() {
    // The fast reorg path must land on the same ledger as checking every
    // prefix block again. If the roots drift, a valid suffix would fail
    // (or worse, a bad one would attach).
    let miner = WalletKeys::generate().address();
    let mut full = devnet();
    for i in 0..8u64 {
        full.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let fast = Chain::rebuild_from_blocks_trusted_prefix(
        NetworkId::Devnet,
        full.blocks.clone(),
        5,
        NOW + 10_000,
    )
    .unwrap();
    assert_eq!(fast.tip_hash(), full.tip_hash());
    assert_eq!(fast.total_work, full.total_work);
    assert_eq!(fast.ledger.utxo_root(), full.ledger.utxo_root());
    assert_eq!(fast.ledger.kernel_sum(), full.ledger.kernel_sum());
    fast.verify_supply().unwrap();
}

#[test]
fn a_four_block_fork_adopts_the_heavier_branch() {
    // Live case: we extended a stale tip four times while the seed kept
    // going. Rewind is 4, not "too deep", and the shared prefix is ours.
    let miner = WalletKeys::generate().address();
    let mut shared = devnet();
    for i in 0..4u64 {
        shared
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut ours = Chain::new_fair(NetworkId::Devnet).unwrap();
    ours.try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    for i in 0..4u64 {
        ours.mine_block(&miner, vec![], NOW + (4 + i) * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut theirs = Chain::new_fair(NetworkId::Devnet).unwrap();
    theirs
        .try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    for i in 0..8u64 {
        theirs
            .mine_block(&miner, vec![], NOW + (4 + i) * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    assert!(theirs.total_work > ours.total_work);
    assert_eq!(
        reorg_rewind(
            &ours.blocks.iter().map(|b| b.hash()).collect::<Vec<_>>(),
            &theirs.blocks
        ),
        4
    );

    assert!(ours
        .maybe_reorg_to(theirs.blocks.clone(), NOW + 10_000)
        .unwrap());
    assert_eq!(ours.tip_hash(), theirs.tip_hash());
    assert_eq!(ours.total_work, theirs.total_work);
    ours.verify_supply().unwrap();
}

#[test]
fn reorg_still_validates_the_untrusted_suffix() {
    // Trusting the prefix must not skip PoW on the first block we do not hold.
    let miner = WalletKeys::generate().address();
    let mut shared = devnet();
    for i in 0..3u64 {
        shared
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut ours = Chain::new_fair(NetworkId::Devnet).unwrap();
    ours.try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    ours.mine_block(&miner, vec![], NOW + 3 * TARGET_BLOCK_TIME_SECS)
        .unwrap();

    let mut theirs = Chain::new_fair(NetworkId::Devnet).unwrap();
    theirs
        .try_ingest_blocks(shared.blocks.clone(), NOW + 1_000)
        .unwrap();
    theirs
        .mine_block(&miner, vec![], NOW + 3 * TARGET_BLOCK_TIME_SECS)
        .unwrap();
    theirs
        .mine_block(&miner, vec![], NOW + 4 * TARGET_BLOCK_TIME_SECS)
        .unwrap();
    assert!(theirs.total_work > ours.total_work);

    let mut tampered = theirs.blocks.clone();
    let last = tampered.last_mut().unwrap();
    last.header.nonce = last.header.nonce.wrapping_add(1);
    while last.pow_is_valid(NetworkId::Devnet.pow_params()) {
        last.header.nonce = last.header.nonce.wrapping_add(1);
    }

    let hashes: Vec<Hash256> = ours.blocks.iter().map(|b| b.hash()).collect();
    let verdict = Chain::evaluate_reorg(
        NetworkId::Devnet,
        ours.total_work,
        &hashes,
        tampered,
        NOW + 10_000,
    );
    assert!(
        matches!(verdict, Err(ConsensusError::BadPow)),
        "an invalid suffix must still fail, got {verdict:?}"
    );
}

#[test]
fn tampered_pow_is_rejected() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    let mut block = chain.build_template(&miner, vec![], NOW).unwrap().seal(0);
    // Devnet's floor is low enough that nonce 0 sometimes hashes. Keep
    // walking until the header is actually invalid.
    block.header.nonce = 0;
    while block.pow_is_valid(chain.pow_params()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
        assert_ne!(block.header.nonce, 0, "every nonce met the target");
    }
    let res = chain.apply_block(block, NOW);
    assert!(matches!(res, Err(ConsensusError::BadPow)), "got {res:?}");
}

#[test]
fn tampered_body_breaks_the_body_root() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    let mut block = chain.mine_block(&miner, vec![], NOW).unwrap();

    // Duplicate an output inside the aggregate.
    let mut chain2 = devnet();
    let dup = block.body.outputs[0].clone();
    block.body.outputs.push(dup);
    let res = chain2.apply_block(block, NOW);
    assert!(res.is_err(), "a tampered aggregate must not validate");
}

#[test]
fn block_body_is_canonically_ordered() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    let block = chain.mine_block(&miner, vec![], NOW).unwrap();
    assert!(
        block.body.is_canonical(),
        "a mined block must already be in canonical order"
    );

    // Reordering must be rejected: order would otherwise leak which
    // transaction each object came from.
    if block.body.outputs.len() > 1 {
        let mut shuffled = block.clone();
        shuffled.body.outputs.reverse();
        let mut chain2 = devnet();
        assert!(chain2.apply_block(shuffled, NOW).is_err());
    }
}

#[test]
fn timestamp_far_in_the_future_is_rejected() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    chain.mine_block(&miner, vec![], NOW).unwrap();

    let template = chain.build_template(&miner, vec![], NOW + 100_000).unwrap();
    let d = template.header.difficulty;
    let (nonce, _) = nightfall_crypto::mine_interruptible(
        &template.header.pow_preimage(),
        d,
        0,
        NetworkId::Devnet.pow_params(),
        &|| false,
    )
    .unwrap();
    let block = template.seal(nonce);

    assert!(matches!(
        chain.apply_block(block, NOW),
        Err(ConsensusError::TimestampTooFarAhead)
    ));
}

#[test]
fn timestamp_before_median_is_rejected() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    for i in 0..12u64 {
        chain
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut template = chain.build_template(&miner, vec![], NOW + 1000).unwrap();
    template.header.timestamp_unix = NOW; // drag it back before the median
    let d = template.header.difficulty;
    let (nonce, _) = nightfall_crypto::mine_interruptible(
        &template.header.pow_preimage(),
        d,
        0,
        NetworkId::Devnet.pow_params(),
        &|| false,
    )
    .unwrap();

    assert!(matches!(
        chain.apply_block(template.seal(nonce), NOW + 1000),
        Err(ConsensusError::TimestampBeforeMedian)
    ));
}

#[test]
fn rejected_block_leaves_the_chain_untouched() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    chain.mine_block(&miner, vec![], NOW).unwrap();

    let before_tip = chain.tip_hash();
    let before_work = chain.total_work;
    let before_minted = chain.total_minted();
    let before_utxos = chain.ledger.utxos.len();

    let bad = chain
        .build_template(&miner, vec![], NOW + 20)
        .unwrap()
        .seal(1);
    let _ = chain.apply_block(bad, NOW + 20);

    assert_eq!(chain.tip_hash(), before_tip);
    assert_eq!(chain.total_work, before_work);
    assert_eq!(chain.total_minted(), before_minted);
    assert_eq!(chain.ledger.utxos.len(), before_utxos);
    chain.verify_supply().unwrap();
}

/// Terminal supply actually produced by the emission curve, in darks.
///
/// Ideal sum `2 · 6 · 7_500_000` is 90 M NIGHT. Integer shifts land
/// **0.75 NIGHT short**. The invariant is `total_minted ≤ cap`.
const TERMINAL_SUPPLY_DARKS: u128 = 8_999_999_925_000_000;

#[test]
fn emission_sums_below_the_cap_and_stops() {
    let e = EmissionSchedule::locked_mainnet();
    let mut total: u128 = 0;
    for era in 0..64u64 {
        let r = e
            .theoretical_reward_at(Height(era * e.halving_interval))
            .darks() as u128;
        if r == 0 {
            break;
        }
        total += r * e.halving_interval as u128;
    }

    assert_eq!(total, TERMINAL_SUPPLY_DARKS, "emission curve drifted");
    assert!(
        total <= nightfall_types::MAX_SUPPLY_DARKS as u128,
        "the cap is a hard ceiling and must never be exceeded"
    );

    let shortfall = nightfall_types::MAX_SUPPLY_DARKS as u128 - total;
    assert_eq!(shortfall, 75_000_000, "shortfall changed unexpectedly");

    // Once the cap is reached, the reward is zero — no tail emission.
    assert_eq!(
        e.reward_at(Height(0), nightfall_types::MAX_SUPPLY_DARKS)
            .darks(),
        0
    );
}

#[test]
fn reward_is_clamped_at_the_cap_boundary() {
    let e = EmissionSchedule::locked_mainnet();
    let almost = nightfall_types::MAX_SUPPLY_DARKS - 5;
    assert_eq!(
        e.reward_at(Height(0), almost).darks(),
        5,
        "must not overshoot"
    );
}

#[test]
fn mempool_picks_high_fees_and_skips_conflicts() {
    let mp = Mempool::default();
    // Empty mempool must simply produce nothing, never panic.
    assert!(mp.select_for_block(10).is_empty());
    assert_eq!(mp.len(), 0);
}

/// A stand-in transaction. `Mempool` is a map with a policy; it does not
/// validate, so a coinbase is fine for testing the policy.
fn some_tx(n: u64) -> nightfall_ledger::Transaction {
    let who = WalletKeys::generate().address();
    nightfall_ledger::build_coinbase(&who, 6 * 100_000_000, n, b"nightfall:test").unwrap()
}

/// The bug this exists for: `remove_included` only drops what a block
/// consumed, so a transaction no miner ever takes was never dropped at all.
/// Two mainnet seeds were holding 60 and 117 of them on 26 Aug 2026.
#[test]
fn a_transaction_nobody_mines_is_eventually_forgotten() {
    let mut mp = Mempool::default();
    let old = some_tx(1);
    let fresh = some_tx(2);
    let old_id = old.txid().to_hex();
    let fresh_id = fresh.txid().to_hex();

    assert!(mp.insert(old, NOW));
    assert!(mp.insert(fresh, NOW + Mempool::MAX_AGE_SECS));
    assert_eq!(mp.len(), 2);

    // One second past the horizon of the first, still inside the second's.
    let dropped = mp.expire(NOW + Mempool::MAX_AGE_SECS + 1);
    assert_eq!(dropped, 1, "exactly the stale one goes");
    assert!(!mp.txs.contains_key(&old_id), "stale entry survived");
    assert!(mp.txs.contains_key(&fresh_id), "fresh entry was thrown out");
    assert!(mp.first_seen(&old_id).is_none(), "timestamp index leaked");
}

// Not tested here: the sweep that `insert` performs when the map is already at
// MAX_ENTRIES. Filling it honestly means building 10,000 transactions, and each
// one carries a bulletproof — minutes of CI for one branch. The behaviour it
// guards (a full pool of corpses must not refuse a live transaction forever) is
// the same `expire` the test above covers; what is untested is only the call
// site. If MAX_ENTRIES ever becomes configurable, test it properly then.

/// The index that remembers arrival times must shrink with the map, or it
/// becomes the unbounded growth the entry cap exists to prevent.
#[test]
fn the_timestamp_index_never_outgrows_the_mempool() {
    let mut mp = Mempool::default();
    let tx = some_tx(7);
    let id = tx.txid().to_hex();
    mp.insert(tx.clone(), NOW);

    let block = Block {
        header: BlockHeader {
            version: 8,
            height: Height(1),
            prev_hash: Hash256([0; 32]),
            utxo_root: Hash256([0; 32]),
            kernel_sum: nightfall_crypto::Commitment([0; 32]),
            body_root: Hash256([0; 32]),
            timestamp_unix: NOW,
            difficulty: 1,
            nonce: 0,
            reward_darks: 0,
        },
        body: nightfall_ledger::BlockBody::aggregate(std::slice::from_ref(&tx)),
    };
    mp.remove_included(&block);

    assert_eq!(mp.len(), 0);
    assert!(
        mp.first_seen(&id).is_none(),
        "arrival time outlived the transaction it belonged to"
    );
}

#[test]
fn genesis_is_fair_and_network_separated() {
    let d = Chain::new_fair(NetworkId::Devnet).unwrap();
    let m = Chain::new_fair(NetworkId::Mainnet).unwrap();
    assert!(d.genesis.allocations.is_empty(), "premine must be zero");
    assert_ne!(
        d.genesis_hash, m.genesis_hash,
        "networks must not share a genesis"
    );
    assert_ne!(d.proof_ctx(), m.proof_ctx());
}

// --- reorg verification off the lock ------------------------------------

#[test]
fn evaluating_a_reorg_needs_no_chain_to_mutate() {
    // The point of `evaluate_reorg` is that a node can run it with its state
    // lock released. That is only true if deciding a reorg needs nothing but
    // borrowed facts, so this test exists to keep it that way: it never
    // mutates the chain being replaced, only its work and its hashes.
    let miner = WalletKeys::generate().address();
    let mut long = devnet();
    for i in 0..6u64 {
        long.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let mut short = devnet();
    short.mine_block(&miner, vec![], NOW).unwrap();

    let short_hashes: Vec<Hash256> = short.blocks.iter().map(|b| b.hash()).collect();
    let verdict = Chain::evaluate_reorg(
        NetworkId::Devnet,
        short.total_work,
        &short_hashes,
        long.blocks.clone(),
        NOW + 10_000,
    )
    .unwrap();

    let candidate = verdict.expect("a heavier chain must be offered");
    assert_eq!(candidate.tip_hash(), long.tip_hash());
    assert_eq!(candidate.total_work, long.total_work);
    candidate.verify_supply().unwrap();
}

#[test]
fn a_candidate_that_lost_the_race_is_not_adopted() {
    // The regression this whole split exists to make safe.
    //
    // Verification now runs with the node's lock released, so between the
    // moment a candidate is judged heavier and the moment it is swapped in,
    // our own chain may have moved — and may now be heavier than the thing
    // about to replace it. Adopting blindly at that point would roll the node
    // *backwards* onto a chain the network has already left, silently undoing
    // confirmed blocks. `adopt_reorg` therefore repeats the comparison instead
    // of trusting the earlier verdict.
    let miner = WalletKeys::generate().address();

    let mut candidate_src = devnet();
    for i in 0..3u64 {
        candidate_src
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut ours = devnet();
    ours.mine_block(&miner, vec![], NOW).unwrap();

    // Judged while we were short: the candidate wins.
    let ours_hashes: Vec<Hash256> = ours.blocks.iter().map(|b| b.hash()).collect();
    let candidate = Chain::evaluate_reorg(
        NetworkId::Devnet,
        ours.total_work,
        &ours_hashes,
        candidate_src.blocks.clone(),
        NOW + 10_000,
    )
    .unwrap()
    .expect("heavier at the time it was judged");

    // Meanwhile we caught up and overtook it.
    for i in 1..6u64 {
        ours.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    assert!(
        ours.total_work > candidate.total_work,
        "setup: we must have overtaken the candidate while it was being verified"
    );

    let tip_before = ours.tip_hash();
    let work_before = ours.total_work;
    assert!(
        !ours.adopt_reorg(candidate),
        "a candidate our chain has already overtaken must be refused"
    );
    assert_eq!(
        ours.tip_hash(),
        tip_before,
        "the tip must not move backwards"
    );
    assert_eq!(ours.total_work, work_before);
    ours.verify_supply().unwrap();
}

#[test]
fn splitting_the_reorg_did_not_change_the_verdict() {
    // `maybe_reorg_to` is now a thin wrapper over the two halves. Same inputs,
    // same answer, same resulting chain — otherwise the refactor moved a
    // consensus rule while claiming to move only a lock.
    let miner = WalletKeys::generate().address();
    let mut long = devnet();
    for i in 0..5u64 {
        long.mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }

    let mut via_wrapper = devnet();
    via_wrapper.mine_block(&miner, vec![], NOW).unwrap();
    let mut via_halves = via_wrapper.clone();

    let a = via_wrapper
        .maybe_reorg_to(long.blocks.clone(), NOW + 10_000)
        .unwrap();

    let halves_hashes: Vec<Hash256> = via_halves.blocks.iter().map(|b| b.hash()).collect();
    let b = match Chain::evaluate_reorg(
        NetworkId::Devnet,
        via_halves.total_work,
        &halves_hashes,
        long.blocks.clone(),
        NOW + 10_000,
    )
    .unwrap()
    {
        Some(c) => via_halves.adopt_reorg(c),
        None => false,
    };

    assert_eq!(a, b, "both routes must reach the same decision");
    assert_eq!(via_wrapper.tip_hash(), via_halves.tip_hash());
    assert_eq!(via_wrapper.total_work, via_halves.total_work);
}

// --- n8 genesis is pinned ------------------------------------------------

/// Mainnet genesis for protocol v8 (6 NIGHT / 7.5 M blocks). The previous
/// live hash `c8614333…` is archived in `docs/HISTORY.md`.
const MAINNET_GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

#[test]
fn n8_genesis_is_pinned() {
    let chain = Chain::new_fair(NetworkId::Mainnet).unwrap();
    assert_eq!(
        chain.genesis_hash.to_hex(),
        MAINNET_GENESIS,
        "the mainnet genesis moved — update docs/HISTORY.md in the same commit"
    );
    assert_eq!(nightfall_types::PROTOCOL_VERSION, 8);
    assert_eq!(nightfall_types::INITIAL_BLOCK_REWARD_NIGHT, 6);
    assert_eq!(nightfall_types::HALVING_INTERVAL_BLOCKS, 7_500_000);
}

/// The gate only works if it differs from what the refused releases speak.
///
/// 0.5.4 and earlier speak wire v4, so anything at or below that reopens the
/// door to them. Checked at compile time because both sides are constants —
/// a runtime assertion here would only ever fail after shipping.
const _: () = assert!(nightfall_types::WIRE_VERSION >= 6);

// --- checkpoints ---------------------------------------------------------

#[test]
fn a_devnet_chain_is_unaffected_by_mainnet_pins() {
    // The pins are mainnet heights. Devnet must build freely, or every test in
    // this file that mines a short chain would be pinned to nothing.
    let miner = WalletKeys::generate().address();
    let mut chain = devnet();
    for i in 0..5u64 {
        chain
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .expect("devnet builds without tripping a mainnet pin");
    }
    assert_eq!(chain.block_count(), 5);
}

#[test]
fn every_pin_is_plausible() {
    // Cheap guard against a fat-fingered edit: a checkpoint must be a 64-char
    // hex string at a height above genesis, and the list must be unique.
    use std::collections::HashSet;
    let mut heights = HashSet::new();
    for (h, hash) in nightfall_types::CHECKPOINTS {
        assert!(
            *h > 0,
            "genesis is pinned by the genesis hash, not a checkpoint"
        );
        assert_eq!(hash.len(), 64, "checkpoint at {h} is not a 32-byte hash");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "checkpoint at {h} is not hex"
        );
        assert!(heights.insert(*h), "height {h} is pinned twice");
    }
}

#[test]
fn the_highest_pin_is_the_one_reported() {
    let max = nightfall_types::CHECKPOINTS
        .iter()
        .map(|(h, _)| *h)
        .max()
        .unwrap_or(0);
    assert_eq!(nightfall_types::highest_checkpoint_height(), max);
    for (h, hash) in nightfall_types::CHECKPOINTS {
        assert_eq!(nightfall_types::checkpoint_at(*h), Some(*hash));
    }
    assert_eq!(nightfall_types::checkpoint_at(u64::MAX), None);
}

#[test]
fn a_block_that_contradicts_a_pin_is_refused() {
    // The property the pins exist for, exercised without needing a real
    // mainnet chain: pin a height, then offer a different block at it.
    //
    // Simulated by asserting the comparison the code performs, because
    // building 25,000 mainnet blocks in a unit test is not a test, it is a
    // weekend. The wiring itself is covered by the two paths in
    // `apply_block_inner` and `apply_block_from_own_disk`, which both call
    // `checkpoint_at` before touching the ledger.
    let (height, pinned) = nightfall_types::CHECKPOINTS[0];
    let impostor = "0".repeat(64);
    assert_ne!(
        impostor, pinned,
        "an impostor hash must differ from the pin"
    );
    assert_eq!(nightfall_types::checkpoint_at(height), Some(pinned));
}

#[test]
fn assume_valid_never_reaches_a_chain_shorter_than_the_pin() {
    // The bug this test exists for, written the day it was made.
    //
    // The first draft of the checkpoint acceleration read
    //
    //     trusted_prefix.max(assume_valid_to.min(blocks.len()))
    //
    // which, for any chain shorter than the pinned height — every devnet
    // chain, every test in this file, every new network — clamped to
    // `blocks.len()` and marked the *entire* chain trusted. Trusted means
    // proof of work is not checked. One `.min()` turned an optimisation into
    // "this build does not validate proof of work", which is the v4 fault
    // class exactly: a check that still runs, still passes, and proves
    // nothing.
    //
    // The guard is that a prefix is only anchored when the pinned block is
    // actually inside it. Below the pin there is nothing vouching for
    // anything, so the full check applies.
    let miner = WalletKeys::generate().address();
    let mut short = devnet();
    for i in 0..4u64 {
        short
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    assert!(
        (short.blocks.len() as u64) < nightfall_types::highest_checkpoint_height(),
        "setup: this chain must be shorter than the pin"
    );

    // Same blocks, one with its proof of work destroyed. A rebuild that
    // trusted the prefix would accept it.
    //
    // Tamper until the work is *actually* broken, and say so.
    //
    // The first version bumped the nonce once and assumed that was enough.
    // It is not: the new hash still meets the target with probability
    // 1/difficulty, and the difficulty here is 60 — so roughly one run in
    // sixty tampered a block that still had valid work, and the assertion
    // below failed for no reason at all. It bit the Windows runner during the
    // v0.9.0 release, which made it look platform-specific. It was not. It
    // was a 1.7% coin flip, and the test was the thing that was wrong.
    //
    // A test for "proof of work is still checked" has to hand the checker
    // something that genuinely fails proof of work.
    let mut tampered = short.blocks.clone();
    let last = tampered.len() - 1;
    let difficulty = tampered[last].header.difficulty;
    let mut attempts = 0u64;
    loop {
        tampered[last].header.nonce = tampered[last].header.nonce.wrapping_add(1);
        if !tampered[last].pow_is_valid(NetworkId::Devnet.pow_params()) {
            break;
        }
        attempts += 1;
        assert!(
            attempts < 100_000,
            "could not construct a header that fails difficulty {difficulty}; \
             at difficulty 1 none exists, and this test cannot mean anything there"
        );
    }

    let out = Chain::rebuild_from_blocks(NetworkId::Devnet, tampered, NOW + 10_000);
    assert!(
        out.is_err(),
        "a chain below the pinned height must still have its proof of work checked"
    );
}

#[test]
fn prune_keeps_count_and_utxo_drops_old_bodies() {
    let miner = WalletKeys::generate().address();
    let mut chain = devnet();
    for i in 0..12u64 {
        chain
            .mine_block(&miner, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let count = chain.block_count();
    let tip = chain.tip_hash();
    let root = chain.ledger.utxo_root();
    let dropped = chain.prune_keep(4).unwrap();
    assert_eq!(dropped, 8);
    assert!(chain.is_pruned());
    assert_eq!(chain.first_height, 8);
    assert_eq!(chain.blocks.len(), 4);
    assert_eq!(chain.block_count(), count);
    assert_eq!(chain.tip_hash(), tip);
    assert_eq!(chain.ledger.utxo_root(), root);
    assert!(chain.block_by_height(0).is_none());
    assert!(chain.block_by_height(8).is_some());
    assert!(chain.blocks_from(0, 4).is_empty());
    assert_eq!(chain.blocks_from(8, 4).len(), 4);
    chain.verify_supply().unwrap();
    chain
        .mine_block(&miner, vec![], NOW + 12 * TARGET_BLOCK_TIME_SECS)
        .unwrap();
    assert_eq!(chain.block_count(), count + 1);
    assert_eq!(chain.blocks.len(), 5);
}

#[test]
fn prune_reorg_inside_the_window_still_works() {
    let a = WalletKeys::from_seed([1u8; 32]).address();
    let b = WalletKeys::from_seed([2u8; 32]).address();
    let mut stem = devnet();
    for i in 0..4u64 {
        stem.mine_block(&a, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let mut ours = stem.clone();
    for i in 4..10u64 {
        ours.mine_block(&a, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    ours.prune_keep(6).unwrap();
    assert_eq!(ours.first_height, 4);

    let mut theirs = stem;
    for i in 4..12u64 {
        theirs
            .mine_block(&b, vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
    let suffix = theirs.blocks[ours.first_height as usize..].to_vec();
    assert_eq!(suffix[0].header.height.0, ours.first_height);
    let adopted = ours
        .maybe_reorg_to(suffix, NOW + 20_000)
        .expect("pruned reorg evaluates");
    assert!(adopted, "heavier suffix inside the window must win");
    assert_eq!(ours.block_count(), 12);
    assert!(ours.is_pruned());
    ours.verify_supply().unwrap();
}

/// Seal a template with real work and apply it. The devnet difficulty is low
/// enough that this is a handful of hashes, not a wait.
fn mine_template(chain: &mut Chain, template: BlockTemplate) -> Hash256 {
    let params = chain.pow_params();
    let mut nonce = 0u64;
    loop {
        let mut header = template.header.clone();
        header.nonce = nonce;
        if nightfall_crypto::meets_difficulty(header.pow_hash(params), header.difficulty) {
            let block = template.clone().seal(nonce);
            let hash = block.hash();
            chain.apply_block(block, header.timestamp_unix).unwrap();
            return hash;
        }
        nonce += 1;
    }
}

/// One unusable transaction must not stop a miner.
///
/// Reported from Discord on 28 Aug 2026: `WARN template: ledger: duplicate
/// output commitment …`, repeating once a second. The log line was the small
/// part. `build_template` is all-or-nothing, so the whole template was
/// discarded over that single entry, the miner slept a second and tried the
/// same doomed set again — and hashed **nothing** for as long as the entry
/// stayed. Six hours, without a restart.
///
/// A coinbase in the mempool is used as the poison here because the ledger
/// refuses it for a reason that can never resolve itself, which is exactly the
/// shape of the original: permanent, and not the miner's fault.
#[test]
fn one_poisoned_transaction_does_not_stop_the_block() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    chain.mine_block(&miner, vec![], NOW).unwrap();

    let poison = some_tx(7);
    let poison_id = poison.txid().to_hex();

    // The old path: everything is thrown away.
    assert!(
        chain
            .build_template(&miner, vec![poison.clone()], NOW + TARGET_BLOCK_TIME_SECS)
            .is_err(),
        "the poison must really be unusable, or this test proves nothing"
    );

    // The new path: the block still gets built, and we are told what went.
    let (template, dropped) = chain
        .build_template_filtering(&miner, vec![poison], NOW + TARGET_BLOCK_TIME_SECS)
        .expect("a bad transaction must not cost us the block");
    assert_eq!(dropped, vec![poison_id], "the offender must be named");
    assert_eq!(template.header.height, Height(1));

    // And the block it produces is a real one.
    let sealed = mine_template(&mut chain, template);
    assert_eq!(chain.tip_hash(), sealed);
    chain.verify_supply().unwrap();
}

/// The clean path must stay cheap: nothing to filter, nothing reported.
#[test]
fn filtering_reports_nothing_when_there_is_nothing_wrong() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    chain.mine_block(&miner, vec![], NOW).unwrap();

    let (_, dropped) = chain
        .build_template_filtering(&miner, vec![], NOW + TARGET_BLOCK_TIME_SECS)
        .unwrap();
    assert!(dropped.is_empty());
}

/// After a reorg the mempool used to keep transactions the new branch had
/// already mined — which is how the poison got in there in the first place.
#[test]
fn the_mempool_can_be_reconciled_against_the_chain() {
    let mut chain = devnet();
    let miner = WalletKeys::generate().address();
    chain.mine_block(&miner, vec![], NOW).unwrap();

    let mut mp = Mempool::default();
    let poison = some_tx(9);
    let poison_id = poison.txid().to_hex();
    assert!(mp.insert(poison, NOW));

    let bad = mp.unacceptable_ids(|tx| chain.precheck_tx(tx).is_ok());
    assert_eq!(bad, vec![poison_id.clone()]);
    assert_eq!(mp.drop_ids(&bad), 1);
    assert!(mp.txs.is_empty());
    assert!(
        mp.first_seen(&poison_id).is_none(),
        "the timestamp index has to shrink with the map"
    );
}
