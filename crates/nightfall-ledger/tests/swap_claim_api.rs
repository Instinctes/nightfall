//! The claim API, through the real ledger.
//!
//! `swap_claim.rs` proves that a hand-built claim confirms. This proves the
//! *supported* path — `nightfall_ledger::swap::build_claim` — does the same,
//! and refuses the locks a counterparty must never claim.

use nightfall_crypto::swap::{LockError, SharedLock, SwapShare};
use nightfall_crypto::{create_output, scan_output, WalletKeys};
use nightfall_ledger::swap::{build_claim, claim_spendable, ClaimError};
use nightfall_ledger::*;
use nightfall_types::{Height, DARKS_PER_NIGHT};

const CTX: &[u8] = b"nightfall:mainnet:v5";

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

fn shared() -> (SwapShare, SwapShare, SharedLock) {
    let a = SwapShare::generate();
    let b = SwapShare::generate();
    let lock = SharedLock::new(a.public(), b.public(), SharedLock::fresh_scan_secret());
    (a, b, lock)
}

#[test]
fn the_supported_path_claims_a_lock_the_ledger_accepts() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    let cb = build_coinbase(&alice.address(), reward, 0, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(std::slice::from_ref(&cb)),
            Height(0),
            reward,
            CTX,
        )
        .unwrap();
    let funds = discover(&alice, &cb);

    let (a, b, shared_lock) = shared();
    let lock_value = 5 * DARKS_PER_NIGHT;
    let fee = DARKS_PER_NIGHT / 100;

    let lock_tx = build_transfer(
        &alice,
        &funds,
        &[Payment {
            to: shared_lock.address(),
            amount: lock_value,
            memo: String::new(),
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
        .find(|o| shared_lock.verify_lock(o, lock_value).is_ok())
        .cloned()
        .expect("one output is the lock");

    let h = COINBASE_MATURITY;
    let cb2 = build_coinbase(&alice.address(), reward, h, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb2, lock_tx]),
            Height(h),
            reward,
            CTX,
        )
        .unwrap();
    assert!(ledger.utxos.contains(&lock_out.commit), "lock confirmed");

    // Bob claims through the API, with a_shared and both halves — never r.
    let claim_fee = DARKS_PER_NIGHT / 100;
    let claim = build_claim(
        &shared_lock,
        &lock_out,
        lock_value,
        &a.secret(),
        &b.secret(),
        &bob.address(),
        claim_fee,
        CTX,
    )
    .expect("the claim must build from lock_blind, not from the payload");

    let cb3 = build_coinbase(&alice.address(), reward, h + 1, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb3, claim.clone()]),
            Height(h + 1),
            reward,
            CTX,
        )
        .expect("UnbalancedBlock here would mean the API took the wrong blind");

    assert!(
        !ledger.utxos.contains(&lock_out.commit),
        "the lock must be spent"
    );
    let got: u64 = claim
        .outputs
        .iter()
        .filter_map(|o| scan_output(&bob.view_key(), o))
        .map(|d| d.value)
        .sum();
    assert_eq!(got, lock_value - claim_fee);
    ledger.verify_supply().expect("supply invariant holds");
}

#[test]
fn a_lock_for_the_wrong_amount_is_not_claimable() {
    let (a, b, shared_lock) = shared();
    let out = create_output(&shared_lock.address(), 5 * DARKS_PER_NIGHT, "", CTX)
        .unwrap()
        .0;
    assert_eq!(
        claim_spendable(
            &shared_lock,
            &out,
            6 * DARKS_PER_NIGHT,
            &a.secret(),
            &b.secret()
        )
        .unwrap_err(),
        ClaimError::Lock(LockError::WrongAmount)
    );
}

#[test]
fn an_output_that_is_not_ours_is_not_claimable() {
    let (a, b, shared_lock) = shared();
    let stranger = WalletKeys::generate().address();
    let out = create_output(&stranger, DARKS_PER_NIGHT, "", CTX)
        .unwrap()
        .0;
    assert_eq!(
        claim_spendable(
            &shared_lock,
            &out,
            DARKS_PER_NIGHT,
            &a.secret(),
            &b.secret()
        )
        .unwrap_err(),
        ClaimError::Lock(LockError::NotOurOutput)
    );
}

/// Verification is what protects the claim, so prove it actually runs.
///
/// A flipped payload byte breaks the sender signature. If `claim_spendable`
/// ever stopped calling `verify_lock`, this would build a `Spendable` for an
/// output consensus will refuse — and the counterparty would release a Bitcoin
/// secret for a lock that never confirms.
#[test]
fn a_tampered_lock_is_refused_because_verification_runs_first() {
    let (a, b, shared_lock) = shared();
    let mut out = create_output(&shared_lock.address(), DARKS_PER_NIGHT, "", CTX)
        .unwrap()
        .0;
    assert!(
        claim_spendable(
            &shared_lock,
            &out,
            DARKS_PER_NIGHT,
            &a.secret(),
            &b.secret()
        )
        .is_ok(),
        "the honest control must pass, or this test proves nothing"
    );

    out.payload[0] ^= 0xFF;
    assert_eq!(
        claim_spendable(
            &shared_lock,
            &out,
            DARKS_PER_NIGHT,
            &a.secret(),
            &b.secret()
        )
        .unwrap_err(),
        ClaimError::Lock(LockError::BadSenderSig),
        "a tampered output must never become a Spendable"
    );
}
