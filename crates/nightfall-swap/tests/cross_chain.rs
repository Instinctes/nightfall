//! The seam between the two chains, which nothing else tests.
//!
//! The Bitcoin side is validated against a real node (`regtest.rs`). The NIGHT
//! side is validated against the real ledger (`swap_claim_api.rs`). Both can be
//! entirely correct while the swap is still broken, because the thing that
//! joins them is a *scalar crossing a curve boundary*: `s_a` is created as a
//! Ristretto scalar, travels to secp256k1 as `T_a`, gets encrypted into an
//! ECDSA adaptor signature, is published on Bitcoin, and is recovered from
//! there as a secp scalar that must map back to the very same Ristretto scalar.
//!
//! Endianness, reduction, encoding — a mistake anywhere on that path leaves
//! both halves passing their own tests and the swap losing money. This test
//! walks the whole path and then does the only thing that settles it: it claims
//! the NIGHT lock with the scalar **recovered from the Bitcoin signature**,
//! never with the original, and asks the ledger to accept the block.

use bitcoin::{Amount, ScriptBuf};
use nightfall_crypto::swap::{SharedLock, SwapShare};
use nightfall_crypto::{scan_output, WalletKeys};
use nightfall_ledger::swap::build_claim;
use nightfall_ledger::*;
use nightfall_swap::adaptor;
use nightfall_swap::bitcoin_tx::{bob_encsign_redeem, two_of_two, TxLock, TxRedeem};
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

#[test]
fn the_secret_published_on_bitcoin_opens_the_night_lock() {
    let mut rng = rand::rngs::OsRng;

    // ---- phase 0: shares, with the DLEQ that binds each S to its T ---------
    let share_a = SwapShare::generate();
    let share_b = SwapShare::generate();
    let offer_a = share_a.offer();
    let offer_b = share_b.offer();
    let shared =
        SharedLock::from_verified_offers(&offer_a, &offer_b, SharedLock::fresh_scan_secret())
            .expect("two honest offers must combine");

    // `T_a` is the encryption point Bob will use on Bitcoin. It has to be the
    // same scalar the DLEQ proved, or the whole exchange is theatre.
    let t_a = adaptor::encryption_point(&share_a.secret());

    // ---- Alice locks NIGHT ------------------------------------------------
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

    let lock_value = 5 * DARKS_PER_NIGHT;
    let fee = DARKS_PER_NIGHT / 100;
    let lock_tx = build_transfer(
        &alice,
        &discover(&alice, &cb),
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

    let lock_out = lock_tx
        .outputs
        .iter()
        .find(|o| shared.verify_lock(o, lock_value).is_ok())
        .cloned()
        .expect("phase 2a: Bob verifies before releasing anything");

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
    assert!(ledger.utxos.contains(&lock_out.commit), "NIGHT is locked");

    // ---- Bitcoin: Bob's adaptor, Alice's redemption -----------------------
    let sk_a = adaptor::random_bitcoin_sk(&mut rng);
    let sk_b = adaptor::random_bitcoin_sk(&mut rng);
    let pk_a = adaptor::verification_key(&sk_a);
    let pk_b = adaptor::verification_key(&sk_b);
    let script = two_of_two(&pk_a, &pk_b).unwrap();

    let btc_lock = TxLock::from_prevout(
        bitcoin::OutPoint::null(),
        Amount::from_sat(200_000),
        &pk_a,
        &pk_b,
        Amount::from_sat(150_000),
        Amount::from_sat(2_000),
        None,
    )
    .expect("btc lock builds");

    let redeem =
        TxRedeem::new(&btc_lock, ScriptBuf::new(), Amount::from_sat(2_000)).expect("redeem builds");
    let enc = bob_encsign_redeem(&sk_b, &t_a, &redeem);

    // Alice completes it. This is the point of no return, and it is what hands
    // Bob his secret.
    let published = adaptor::decrypt(&share_a.secret(), enc.clone());
    let _ = script;

    // ---- the seam: Bob recovers s_a from the Bitcoin signature ------------
    let recovered_a = adaptor::recover(&t_a, &published, &enc)
        .expect("the published signature must yield the decryption key");
    assert_eq!(
        recovered_a,
        share_a.secret(),
        "the scalar off the Bitcoin chain must be the Ristretto scalar we started with"
    );

    // ---- and it opens the NIGHT lock --------------------------------------
    //
    // Deliberately built from `recovered_a`, not from `share_a.secret()`. If
    // the curve round trip lost or altered a single bit, this claim does not
    // verify and the swap would have taken Bob's BTC for nothing.
    let claim_fee = DARKS_PER_NIGHT / 100;
    let claim = build_claim(
        &shared,
        &lock_out,
        lock_value,
        &recovered_a,
        &share_b.secret(),
        &bob.address(),
        claim_fee,
        CTX,
    )
    .expect("the recovered scalar must build a valid claim");

    let cb3 = build_coinbase(&alice.address(), reward, h + 1, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb3, claim.clone()]),
            Height(h + 1),
            reward,
            CTX,
        )
        .expect("the ledger must accept a claim built from the Bitcoin-side secret");

    assert!(
        !ledger.utxos.contains(&lock_out.commit),
        "the NIGHT lock is spent — the swap completed across both chains"
    );
    let got: u64 = claim
        .outputs
        .iter()
        .filter_map(|o| scan_output(&bob.view_key(), o))
        .map(|d| d.value)
        .sum();
    assert_eq!(got, lock_value - claim_fee, "Bob holds the NIGHT");
    ledger.verify_supply().expect("supply invariant holds");
}

/// The counter-test: one half alone is not enough.
///
/// Without it the test above could pass on a construction where the Bitcoin
/// secret was decorative.
#[test]
fn the_bitcoin_secret_alone_does_not_open_the_lock() {
    let share_a = SwapShare::generate();
    let share_b = SwapShare::generate();
    let shared = SharedLock::from_verified_offers(
        &share_a.offer(),
        &share_b.offer(),
        SharedLock::fresh_scan_secret(),
    )
    .unwrap();

    let out = nightfall_crypto::create_output(&shared.address(), DARKS_PER_NIGHT, "", CTX)
        .unwrap()
        .0;

    let only_a = shared
        .claim_secret(
            &share_a.secret(),
            &curve25519_dalek::scalar::Scalar::ZERO,
            &out.ephemeral_pk,
        )
        .unwrap();
    assert_ne!(
        (nightfall_crypto::generator_g() * only_a)
            .compress()
            .to_bytes(),
        out.output_pk,
        "s_a alone must not be the discrete log of the lock"
    );
}
