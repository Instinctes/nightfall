//! The NIGHT half of a swap, through the real ledger.
//!
//! `nightfall-crypto::swap` can prove that `s_a + s_b + offset` is the
//! discrete log of `Ko` and that `lock_blind()` equals the sender's output
//! blind. That is key arithmetic. It does not prove that a kernel built from
//! that blind balances, that the input signature verifies against the UTXO
//! set, or that `apply_block` will accept the claim.
//!
//! This file is that missing test: Alice pays a shared address, the lock
//! confirms, Bob derives `b_in` from the scan key alone (he never saw `r`),
//! and the claim is applied. If `lock_blind` were lying, the block would
//! come back `UnbalancedBlock`.

use nightfall_crypto::swap::{SharedLock, SwapShare};
use nightfall_crypto::{scan_output, WalletKeys};
use nightfall_ledger::*;
use nightfall_types::{Height, DARKS_PER_NIGHT};

const CTX: &[u8] = b"nightfall:mainnet:v5";

fn mine_to(ledger: &mut LedgerState, miner: &WalletKeys, reward: u64, height: u64) -> Transaction {
    let cb = build_coinbase(&miner.address(), reward, height, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(std::slice::from_ref(&cb)),
            Height(height),
            reward,
            CTX,
        )
        .unwrap();
    cb
}

fn discover(keys: &WalletKeys, tx: &Transaction) -> Vec<Spendable> {
    let view = keys.view_key();
    tx.outputs
        .iter()
        .filter_map(|o| {
            scan_output(&view, o).map(|d| Spendable {
                commit: d.commit,
                value: d.value,
                blind: d.blind,
                spend_secret: d.spend_secret(keys),
            })
        })
        .collect()
}

#[test]
fn bob_claims_a_confirmed_lock_using_only_the_shared_scan_key() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    let cb = mine_to(&mut ledger, &alice, reward, 0);
    let spend_height = COINBASE_MATURITY;
    let alice_inputs = discover(&alice, &cb);

    let a = SwapShare::generate();
    let b = SwapShare::generate();
    let shared = SharedLock::new(a.public(), b.public(), SharedLock::fresh_scan_secret());

    let lock_value = 5 * DARKS_PER_NIGHT;
    let fee = DARKS_PER_NIGHT / 100;

    let lock_tx = build_transfer(
        &alice,
        &alice_inputs,
        &[Payment {
            to: shared.address(),
            amount: lock_value,
            memo: "swap-lock".into(),
        }],
        fee,
        &alice.address(),
        0,
        CTX,
    )
    .unwrap();

    let lock_out = lock_tx
        .outputs
        .iter()
        .find(|o| shared.verify_lock(o, lock_value).is_ok())
        .cloned()
        .expect("the lock output must pass verify_lock");

    // Change to Alice must not look like the lock.
    let change_hits = lock_tx
        .outputs
        .iter()
        .filter(|o| shared.verify_lock(o, lock_value).is_ok())
        .count();
    assert_eq!(change_hits, 1, "exactly one output is the lock");

    let cb2 = build_coinbase(&alice.address(), reward, spend_height, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb2, lock_tx.clone()]),
            Height(spend_height),
            reward,
            CTX,
        )
        .unwrap();
    assert!(
        ledger.utxos.contains(&lock_out.commit),
        "lock must confirm before Bob claims"
    );

    // Bob never saw r. He has a_shared, Ke, both key halves.
    let bin = shared
        .lock_blind(&lock_out.ephemeral_pk)
        .expect("lock_blind from scan key");
    let spend_secret = shared
        .claim_secret(&a.secret(), &b.secret(), &lock_out.ephemeral_pk)
        .expect("claim secret");

    let claim_fee = DARKS_PER_NIGHT / 100;
    let claim_amount = lock_value - claim_fee;
    let claim_tx = build_transfer(
        &bob,
        &[Spendable {
            commit: lock_out.commit,
            value: lock_value,
            blind: bin,
            spend_secret,
        }],
        &[Payment {
            to: bob.address(),
            amount: claim_amount,
            memo: "swap-claim".into(),
        }],
        claim_fee,
        &bob.address(),
        0,
        CTX,
    )
    .expect("claim must build — if lock_blind is wrong, excess_secret is wrong");

    let claim_height = spend_height + 1;
    let cb3 = build_coinbase(&alice.address(), reward, claim_height, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb3, claim_tx.clone()]),
            Height(claim_height),
            reward,
            CTX,
        )
        .expect("claim must confirm — UnbalancedBlock here means lock_blind lied");

    assert!(
        !ledger.utxos.contains(&lock_out.commit),
        "the lock must be spent"
    );
    ledger
        .verify_supply()
        .expect("supply invariant after claim");

    let bob_got: Vec<_> = claim_tx
        .outputs
        .iter()
        .filter_map(|o| scan_output(&bob.view_key(), o))
        .collect();
    let bob_total: u64 = bob_got.iter().map(|d| d.value).sum();
    assert_eq!(
        bob_total,
        lock_value - claim_fee,
        "Bob must receive the lock minus the claim fee"
    );
}

#[test]
fn verify_lock_rejects_alices_change() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;
    let cb = mine_to(&mut ledger, &alice, reward, 0);
    let inputs = discover(&alice, &cb);

    let a = SwapShare::generate();
    let b = SwapShare::generate();
    let shared = SharedLock::new(a.public(), b.public(), SharedLock::fresh_scan_secret());

    let lock_value = 5 * DARKS_PER_NIGHT;
    let fee = DARKS_PER_NIGHT / 100;
    let tx = build_transfer(
        &alice,
        &inputs,
        &[Payment {
            to: shared.address(),
            amount: lock_value,
            memo: String::new(),
        }],
        fee,
        &alice.address(),
        0,
        CTX,
    )
    .unwrap();

    let results: Vec<_> = tx
        .outputs
        .iter()
        .map(|o| shared.verify_lock(o, lock_value))
        .collect();
    let oks = results.iter().filter(|r| r.is_ok()).count();
    let not_ours = results
        .iter()
        .filter(|r| *r == &Err(nightfall_crypto::swap::LockError::NotOurOutput))
        .count();
    assert_eq!(oks, 1);
    assert_eq!(not_ours, 1);
}
