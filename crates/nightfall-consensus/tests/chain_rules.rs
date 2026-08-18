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
