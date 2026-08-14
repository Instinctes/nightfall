//! How much of a peer's chain we pull before judging it.

use nightfall_consensus::MAX_REORG_DEPTH;
use nightfall_node::runtime::reorg_fetch_cap;

#[test]
fn a_peer_longer_than_two_thousand_blocks_is_still_fetched_whole() {
    // The bug this exists for, with the numbers it was found at.
    //
    // The cap used to be a flat `MAX_REORG_DEPTH * 4` — 2,000 blocks. It was
    // invisible until the chain passed 2,000, and then it became a wall: a node
    // on 2,048 blocks asked a peer holding 2,057 for its chain, received the
    // first 2,000 of them, weighed that prefix against its own longer chain,
    // correctly found it lighter, and refused. Every round. The peer was not
    // offering a worse chain — we were only ever shown part of a better one, and
    // the two could never reconcile. Everything mined on the losing side was
    // lost with it.
    //
    // A cap that is any fixed number has this failure waiting in it; the only
    // question is which block reaches it.
    let cap = reorg_fetch_cap(2056, 2048);
    assert!(
        cap >= 2057,
        "must fetch the peer's whole chain, got {cap} of 2057 blocks"
    );
    assert!(
        cap > MAX_REORG_DEPTH * 4,
        "a fixed 2,000-block cap is exactly the bug"
    );
}

#[test]
fn we_never_pull_more_than_we_would_accept() {
    // The bound is not arbitrary caution, it is the acceptance rule stated
    // once: `evaluate_reorg` rejects anything longer than ours plus
    // MAX_REORG_DEPTH as too deep, so pulling further is wasted bandwidth a
    // hostile peer would be delighted to make us spend.
    let our_len = 100;
    let cap = reorg_fetch_cap(u64::MAX - 1, our_len);
    assert_eq!(cap, our_len + MAX_REORG_DEPTH);
}

#[test]
fn a_short_peer_is_not_padded() {
    // A peer with ten blocks costs ten blocks to evaluate, not a ceiling.
    assert_eq!(reorg_fetch_cap(9, 5_000), 10);
}

#[test]
fn genesis_only_peer_is_one_block() {
    assert_eq!(reorg_fetch_cap(0, 1), 1);
}
