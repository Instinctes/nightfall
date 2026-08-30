//! Run 1: the happy path, through the session layer, against a real node.
//!
//! `regtest.rs` builds the four transactions by hand and asks Core whether
//! they are valid. That answers "is our Bitcoin construction sound", which
//! matters, but it skips what a user actually goes through: the handshake,
//! the signatures it collects, and the completion code that turns those into
//! something broadcastable.
//!
//! ```text
//! cargo test -p nightfall-swap --test swap_live -- --ignored --nocapture
//! ```

mod common;
use common::*;

use nightfall_swap::timelock::Depths;

// --- run 1: the happy path -------------------------------------------------

#[test]
#[ignore = "needs a bitcoind regtest node with -txindex"]
fn run_1_happy_path_redeem_is_accepted_and_publishes_the_scalar() {
    let _guard = node_lock();
    assert!(node_is_up(), "no regtest node at {DIR}:{PORT}");
    let depths = Depths::testdrive();
    let (mut alice, mut bob, a) = full_handshake(depths);

    let (prev, value) = a_utxo(a.btc_sats + a.btc_fee_sats + 20_000);
    let lock_txid = fund_and_broadcast_lock(&mut bob, prev, value);
    println!("TX_lock on chain: {lock_txid}");

    // Alice learns the lock from the packet, exactly as she would in the app.
    exchange_rest(&mut alice, &mut bob);

    // The step that costs Alice nothing and gives Bob everything.
    let redeem_hex = alice
        .signed_redeem_hex()
        .expect("Alice can complete the redeem");
    let redeem_txid = try_send(&redeem_hex).expect("the node must accept TX_redeem");
    println!("TX_redeem accepted: {redeem_txid}");
    mine(1);

    // And the scalar really is out there: Bob reads it back off the chain.
    let raw = cli(&["getrawtransaction", &redeem_txid, "0"]);
    let tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex::decode(&raw).unwrap()).unwrap();
    // The signature that carries the secret is BOB's, not Alice's.
    //
    // Bob adaptor-signed TX_redeem under T_a; Alice decrypting it with her
    // share is what turns it into a valid signature, and that act is what
    // exposes `a`. Alice's own signature on the same transaction reveals
    // nothing. Reading the wrong slot fails silently, which is why this is
    // spelled out rather than left to the reader.
    let (ka, kb) = (alice_key(&alice), bob_key(&bob));
    let published = nightfall_swap::bitcoin_tx::signature_from_witness(&tx, &ka, &kb, &kb)
        .expect("Bob's decrypted signature must be readable from the broadcast witness");
    let recovered = bob
        .recover_from_redeem(&published)
        .expect("Bob must recover s_a from what Alice broadcast");
    assert_eq!(
        recovered,
        alice.secrets().share.secret(),
        "the scalar Bob pulls off the chain must be Alice's share"
    );
    println!("s_a recovered from the confirmed transaction — the swap completes");
}
