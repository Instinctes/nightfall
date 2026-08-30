//! The three runs where something goes wrong, against a real node.
//!
//! Run 1 (the happy path) lives in `swap_live.rs`. These are the ones that
//! decide whether a user gets their money back when the other side walks
//! away, and the one that decides whether our own machine stops us doing
//! something irreversible too late.
//!
//! ```text
//! cargo test -p nightfall-swap --test swap_live_aborts -- --ignored --nocapture
//! ```

mod common;
use common::*;

use nightfall_swap::timelock::Depths;

// --- run 2: Alice vanishes, Bob gets his Bitcoin back ----------------------

/// Bob locked Bitcoin. Alice never locked NIGHT, or locked it and stopped
/// answering. His way out is TX_cancel followed by TX_refund, and the whole
/// point is that he can walk it alone.
#[test]
#[ignore = "needs a bitcoind regtest node with -txindex"]
fn run_2_alice_vanishes_and_bob_recovers_his_bitcoin() {
    let _guard = node_lock();
    assert!(node_is_up(), "no regtest node");
    let depths = Depths::testdrive();
    let (mut alice, mut bob, a) = full_handshake(depths);

    let (prev, value) = a_utxo(a.btc_sats + a.btc_fee_sats + 20_000);
    let lock_txid = fund_and_broadcast_lock(&mut bob, prev, value);
    exchange_rest(&mut alice, &mut bob);
    println!("TX_lock on chain: {lock_txid}");

    // Alice stops here. Bob has everything he needs without her.
    let cancel_hex = bob
        .signed_cancel_hex()
        .expect("Bob can complete TX_cancel alone");

    // Before H1 the chain itself refuses it. That is the guarantee Alice has.
    let early = try_send(&cancel_hex);
    assert!(
        early.is_err(),
        "TX_cancel must not confirm before H1 — Alice's redeem window depends on it"
    );
    let msg = early.unwrap_err();
    assert!(
        msg.contains("non-BIP68-final"),
        "expected the timelock to be the reason, got: {msg}"
    );
    println!("TX_cancel refused before H1: non-BIP68-final");

    mine(depths.cancel);
    let cancel_txid = try_send(&cancel_hex).expect("after H1 the cancel must go through");
    mine(1);
    println!("TX_cancel confirmed: {cancel_txid}");

    // And the refund, which is the part that actually returns his coin.
    let refund_hex = bob.signed_refund_hex().expect("Bob can complete TX_refund");
    let refund_txid = try_send(&refund_hex).expect("the refund must broadcast");
    mine(1);
    println!("TX_refund confirmed: {refund_txid} — Bob is whole again");

    // Alice cannot complete the refund: her half was an adaptor under T_b,
    // and she does not hold b.
    //
    // Checked by *which* error, not merely that there is one. Without the
    // role guard she fails anyway — she never received a refund adaptor, she
    // sent one — so `is_err()` alone was true for the wrong reason and left
    // the guard untested. Found by mutation.
    assert_eq!(
        alice.signed_refund_hex().unwrap_err(),
        nightfall_swap::session::SessionError::WrongRole,
        "the refund must be refused because Alice is the wrong role, not by \
         accident of what she happens to be missing"
    );
}

// --- run 3: Bob vanishes after the cancel, Alice punishes ------------------

/// The cancel confirmed and Bob never refunded. After H2 Alice may take the
/// Bitcoin. This is compensation for NIGHT that is stuck, not a second
/// payout — the NIGHT stays stuck either way, which is the wart.
#[test]
#[ignore = "needs a bitcoind regtest node with -txindex"]
fn run_3_bob_stalls_after_the_cancel_and_alice_punishes() {
    let _guard = node_lock();
    assert!(node_is_up(), "no regtest node");
    let depths = Depths::testdrive();
    let (mut alice, mut bob, a) = full_handshake(depths);

    let (prev, value) = a_utxo(a.btc_sats + a.btc_fee_sats + 20_000);
    fund_and_broadcast_lock(&mut bob, prev, value);
    exchange_rest(&mut alice, &mut bob);

    mine(depths.cancel);
    let cancel_hex = alice.signed_cancel_hex().expect("Alice can cancel too");
    let cancel_txid = try_send(&cancel_hex).expect("the cancel must confirm");
    mine(1);
    println!("TX_cancel confirmed: {cancel_txid}");

    // Bob does nothing. Before H2 the chain holds Alice back as well.
    let punish_hex = alice
        .signed_punish_hex()
        .expect("Alice can complete TX_punish");
    let early = try_send(&punish_hex);
    assert!(
        early.is_err(),
        "punish must wait for H2 — otherwise Bob has no chance to refund"
    );
    println!("TX_punish refused before H2");

    mine(depths.punish);
    let punish_txid = try_send(&punish_hex).expect("after H2 the punish must go through");
    mine(1);
    println!("TX_punish confirmed: {punish_txid}");

    // And Bob cannot punish himself out of it — again by role, not by luck.
    assert_eq!(
        bob.signed_punish_hex().unwrap_err(),
        nightfall_swap::session::SessionError::WrongRole,
        "the punish path belongs to Alice"
    );
}

// --- run 4: the refusal that protects Alice from herself -------------------

/// The most important one.
///
/// Late in the window, redeeming is a trap: Alice publishes `s_a`, Bob takes
/// the NIGHT, and TX_cancel confirms before her redeem does — so he ends up
/// with both sides. The chain will happily accept the redeem, because
/// nothing on Bitcoin knows about H1 from the redeem's point of view. The
/// refusal has to be ours.
///
/// This proves both halves: the node *would* take it, and our rule says no.
#[test]
#[ignore = "needs a bitcoind regtest node with -txindex"]
fn run_4_the_machine_refuses_a_redeem_too_close_to_h1() {
    let _guard = node_lock();
    assert!(node_is_up(), "no regtest node");
    let depths = Depths::testdrive();
    let (mut alice, mut bob, a) = full_handshake(depths);

    let (prev, value) = a_utxo(a.btc_sats + a.btc_fee_sats + 20_000);
    fund_and_broadcast_lock(&mut bob, prev, value);
    exchange_rest(&mut alice, &mut bob);

    // Walk the lock to a depth inside the margin.
    let inside = depths.cancel - depths.btc_redeem_margin;
    assert!(
        !depths.may_redeem(inside),
        "precondition: {inside} confirmations must already be too late"
    );
    mine(inside);

    // Our rule says no.
    assert!(
        !depths.may_redeem(inside),
        "the swap must refuse to redeem this late"
    );

    // The chain, meanwhile, has no objection at all — which is exactly why
    // the rule cannot live on Bitcoin.
    let redeem_hex = alice
        .signed_redeem_hex()
        .expect("the transaction is well formed");
    let accepted = try_send(&redeem_hex);
    assert!(
        accepted.is_ok(),
        "the node should still accept it; the danger is a race, not invalidity. Got {accepted:?}"
    );
    mine(1);
    println!(
        "the node accepted a redeem our own rule forbids — the H1 margin is \
         policy, and it is the only thing standing between Alice and a race \
         she can lose"
    );

    let _ = bob;
}
