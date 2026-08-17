//! How much of a peer's chain we pull before judging it, and when mining waits.

use nightfall_consensus::MAX_REORG_DEPTH;
use nightfall_node::runtime::{
    mining_should_wait, reorg_fetch_cap, MAX_CATCHUP_WAIT_SECS, MAX_REORG_FETCH,
    PEER_HEIGHT_TTL_SECS,
};

#[test]
fn a_suffix_after_a_recent_ancestor_is_fetched_whole() {
    // The 0.6.1 bug, restated for the new meaning of the cap.
    //
    // We no longer pull from genesis. A node on 2,048 that last agreed at
    // 2,047 with a peer on 2,056 needs the nine blocks after the ancestor,
    // not a 2,000-block prefix of the peer's whole history.
    let cap = reorg_fetch_cap(2056, 2048);
    assert_eq!(cap, 9, "heights 2048..=2056 inclusive");
}

#[test]
fn a_peer_thousands_ahead_on_a_shallow_fork_is_still_fetched() {
    // The laptop-sleep case: one-block fork, seed 700 ahead. Old cap was
    // our_len + MAX_REORG_DEPTH and evaluate_reorg refused anything longer
    // than that as too deep. The suffix is 700 blocks and must come back
    // whole — 500 of a 700-block lead is the same truncated-prefix trap.
    let ancestor = 1_999;
    let peer = 1_999 + 700;
    let cap = reorg_fetch_cap(peer, ancestor + 1);
    assert_eq!(cap, 700);
    assert!(
        cap > MAX_REORG_DEPTH,
        "catch-up after a few hours is longer than the rewind bound"
    );
}

#[test]
fn a_liar_advertising_u64_max_is_capped() {
    let cap = reorg_fetch_cap(u64::MAX - 1, 0);
    assert_eq!(cap, MAX_REORG_FETCH);
}

#[test]
fn a_short_peer_is_not_padded() {
    assert_eq!(reorg_fetch_cap(9, 0), 10);
}

#[test]
fn genesis_only_peer_is_one_block() {
    assert_eq!(reorg_fetch_cap(0, 0), 1);
}

#[test]
fn nothing_to_fetch_past_their_tip() {
    assert_eq!(reorg_fetch_cap(10, 11), 0);
}

#[test]
fn empty_node_may_mine_without_peers() {
    // A network of one has to start.
    assert_eq!(
        mining_should_wait(0, 0, 0, 0, 1_800_000_000, 1_800_000_000),
        None
    );
}

#[test]
fn a_loaded_chain_does_not_mine_before_a_peer_speaks() {
    // Restart / lid-open: we have history, no fresh peer. Mining now would
    // extend a tip the seed may already have left.
    let now = 1_800_000_000;
    assert_eq!(mining_should_wait(1_900, 1_899, 0, 0, now, now), Some(1));
}

#[test]
fn isolated_node_mines_after_the_catchup_window() {
    let start = 1_800_000_000;
    let later = start + MAX_CATCHUP_WAIT_SECS + 1;
    assert_eq!(mining_should_wait(1_900, 1_899, 0, 0, start, later), None);
}

#[test]
fn stale_peer_height_is_not_trusted() {
    // Cached height still matches ours, but the last confirmation is older
    // than the TTL — the sockets died with the lid.
    let seen = 1_800_000_000;
    let now = seen + PEER_HEIGHT_TTL_SECS + 1;
    assert_eq!(
        mining_should_wait(1_900, 1_899, 1_899, seen, seen, now),
        Some(1)
    );
}

#[test]
fn a_fresh_equal_peer_lets_us_mine() {
    let now = 1_800_000_000;
    assert_eq!(mining_should_wait(1_900, 1_899, 1_899, now, now, now), None);
}

#[test]
fn a_fresh_ahead_peer_holds_mining() {
    let now = 1_800_000_000;
    assert_eq!(
        mining_should_wait(1_900, 1_899, 2_500, now, now, now),
        Some(601)
    );
}
