//! End-to-end ledger behaviour: mine, discover, spend, verify supply.

use nightfall_crypto::{scan_output, WalletKeys};
use nightfall_ledger::*;
use nightfall_types::{Height, DARKS_PER_NIGHT};

const CTX: &[u8] = b"nightfall:mainnet:v5";

/// Mine `reward` to `miner` at `height` and return the block's transactions.
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

/// Recover everything `keys` owns from a transaction.
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
fn coinbase_is_credited_and_discoverable() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    let cb = mine_to(&mut ledger, &miner, reward, 0);

    assert_eq!(ledger.supply.total_minted_darks, reward);
    assert_eq!(ledger.utxos.len(), 1);
    ledger.verify_supply().expect("supply invariant");

    let mine = discover(&miner, &cb);
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].value, reward);
}

#[test]
fn full_transfer_flow_with_fee_burn() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let alice = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    // Mine one block, then enough blocks for the coinbase to mature.
    let cb = mine_to(&mut ledger, &miner, reward, 0);
    let spend_height = COINBASE_MATURITY;

    let inputs = discover(&miner, &cb);
    let fee = DARKS_PER_NIGHT / 100; // 0.01 NIGHT
    let amount = 5 * DARKS_PER_NIGHT;

    let tx = build_transfer(
        &miner,
        &inputs,
        &[Payment {
            to: alice.address(),
            amount,
            memo: "hello alice".into(),
        }],
        fee,
        &miner.address(),
        0,
        CTX,
    )
    .unwrap();

    // Mine the block containing the transfer.
    let cb2 = build_coinbase(&miner.address(), reward, spend_height, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(&[cb2, tx.clone()]),
            Height(spend_height),
            reward,
            CTX,
        )
        .unwrap();

    // Fee was burned, not paid to anyone.
    assert_eq!(ledger.supply.total_burned_darks, fee);
    assert_eq!(ledger.supply.total_minted_darks, 2 * reward);
    assert_eq!(ledger.supply.circulating(), 2 * reward - fee);

    // The invariant is the real proof that nothing was created or lost.
    ledger
        .verify_supply()
        .expect("supply invariant after transfer");

    // Alice can find her payment, including the memo.
    let alice_view = alice.view_key();
    let found: Vec<_> = tx
        .outputs
        .iter()
        .filter_map(|o| scan_output(&alice_view, o))
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, amount);
    assert_eq!(found[0].memo, "hello alice");

    // Miner keeps the change.
    let change = discover(&miner, &tx);
    assert_eq!(change.len(), 1);
    assert_eq!(change[0].value, reward - amount - fee);

    // The spent coinbase output is gone from the UTXO set.
    assert!(!ledger.utxos.contains(&inputs[0].commit));
}

#[test]
fn double_spend_is_rejected() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    let cb = mine_to(&mut ledger, &miner, reward, 0);
    let inputs = discover(&miner, &cb);
    let h = COINBASE_MATURITY;

    let mk = |amount: u64| {
        build_transfer(
            &miner,
            &inputs,
            &[Payment {
                to: bob.address(),
                amount,
                memo: String::new(),
            }],
            1000,
            &miner.address(),
            0,
            CTX,
        )
        .unwrap()
    };

    let tx1 = mk(DARKS_PER_NIGHT);
    let tx2 = mk(2 * DARKS_PER_NIGHT);

    let cb2 = build_coinbase(&miner.address(), reward, h, CTX).unwrap();
    let err = ledger
        .apply_block(
            &BlockBody::aggregate(&[cb2, tx1, tx2]),
            Height(h),
            reward,
            CTX,
        )
        .unwrap_err();
    assert!(
        matches!(err, LedgerError::DoubleSpendWithinBlock),
        "got {err:?}"
    );

    // And the failed block must not have changed a thing.
    assert_eq!(ledger.utxos.len(), 1);
    assert_eq!(ledger.supply.total_minted_darks, reward);
    ledger.verify_supply().unwrap();
}

#[test]
fn immature_coinbase_cannot_be_spent() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    let cb = mine_to(&mut ledger, &miner, reward, 0);
    let inputs = discover(&miner, &cb);

    let tx = build_transfer(
        &miner,
        &inputs,
        &[Payment {
            to: bob.address(),
            amount: DARKS_PER_NIGHT,
            memo: String::new(),
        }],
        1000,
        &miner.address(),
        0,
        CTX,
    )
    .unwrap();

    let cb2 = build_coinbase(&miner.address(), reward, 1, CTX).unwrap();
    let err = ledger
        .apply_block(&BlockBody::aggregate(&[cb2, tx]), Height(1), reward, CTX)
        .unwrap_err();
    assert!(
        matches!(err, LedgerError::ImmatureCoinbaseSpend { .. }),
        "got {err:?}"
    );
}

#[test]
fn wrong_reward_is_rejected() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let greedy = 1_000 * DARKS_PER_NIGHT;
    let allowed = 20 * DARKS_PER_NIGHT;

    let cb = build_coinbase(&miner.address(), greedy, 0, CTX).unwrap();
    let err = ledger
        .apply_block(&BlockBody::aggregate(&[cb]), Height(0), allowed, CTX)
        .unwrap_err();
    assert!(
        matches!(err, LedgerError::WrongReward { .. }),
        "got {err:?}"
    );
    assert_eq!(ledger.supply.total_minted_darks, 0);
}

#[test]
fn failed_block_leaves_state_untouched() {
    // v4 mutated state before validating and never rolled back. This test locks
    // that behaviour out permanently.
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;
    mine_to(&mut ledger, &miner, reward, 0);

    let before_root = ledger.utxo_root();
    let before_supply = ledger.supply.clone();
    let before_kernels = ledger.kernels.count;
    let before_utxos = ledger.utxos.len();

    // A block whose second tx references a non-existent input.
    let cb = build_coinbase(&miner.address(), reward, 1, CTX).unwrap();
    let fake = Spendable {
        commit: nightfall_crypto::Commitment::new(999, &curve25519_dalek::scalar::Scalar::ONE),
        value: 999,
        blind: curve25519_dalek::scalar::Scalar::ONE,
        spend_secret: curve25519_dalek::scalar::Scalar::ONE,
    };
    let bad = build_transfer(
        &miner,
        &[fake],
        &[Payment {
            to: miner.address(),
            amount: 99,
            memo: String::new(),
        }],
        0,
        &miner.address(),
        0,
        CTX,
    )
    .unwrap();

    assert!(ledger
        .apply_block(&BlockBody::aggregate(&[cb, bad]), Height(1), reward, CTX)
        .is_err());

    assert_eq!(ledger.utxo_root(), before_root, "UTXO root changed");
    assert_eq!(ledger.utxos.len(), before_utxos, "UTXO count changed");
    assert_eq!(ledger.kernels.count, before_kernels, "kernels changed");
    assert_eq!(
        ledger.supply.total_minted_darks,
        before_supply.total_minted_darks
    );
    assert_eq!(
        ledger.supply.total_burned_darks,
        before_supply.total_burned_darks
    );
    ledger.verify_supply().unwrap();
}

#[test]
fn supply_cap_is_hard() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();

    // Pretend we are one dark below the cap.
    ledger.supply.total_minted_darks = nightfall_types::MAX_SUPPLY_DARKS;

    let cb = build_coinbase(&miner.address(), 1, 0, CTX).unwrap();
    let err = ledger
        .apply_block(&BlockBody::aggregate(&[cb]), Height(0), 1, CTX)
        .unwrap_err();
    assert!(matches!(err, LedgerError::SupplyCapExceeded), "got {err:?}");
}

#[test]
fn many_blocks_keep_the_invariant() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let reward = 20 * DARKS_PER_NIGHT;

    for h in 0..25u64 {
        mine_to(&mut ledger, &miner, reward, h);
    }
    assert_eq!(ledger.supply.total_minted_darks, 25 * reward);
    assert_eq!(ledger.utxos.len(), 25);
    ledger
        .verify_supply()
        .expect("invariant across many blocks");
}
// Does a payment reach a stranger's wallet, end to end, through the same
// scanning path a real recipient uses?
#[test]
fn a_stranger_can_find_a_payment_sent_to_them() {
    use nightfall_crypto::{scan_output, WalletKeys};
    use nightfall_ledger::{build_coinbase, build_transfer, BlockBody, Payment, Spendable};
    use nightfall_types::{NetworkId, DARKS_PER_NIGHT};

    let ctx = NetworkId::Devnet.proof_context();
    let sender = WalletKeys::generate();
    let stranger = WalletKeys::generate();

    // Fund the sender with two coinbases, exactly as on the real chain.
    let mut spendables = Vec::new();
    for _ in 0..2 {
        let cb = build_coinbase(&sender.address(), 20 * DARKS_PER_NIGHT, 0, ctx).unwrap();
        let view = sender.view_key();
        for o in &cb.outputs {
            if let Some(d) = scan_output(&view, o) {
                spendables.push(Spendable {
                    commit: d.commit,
                    value: d.value,
                    blind: d.blind,
                    spend_secret: d.spend_secret(&sender),
                });
            }
        }
    }
    assert_eq!(
        spendables.len(),
        2,
        "sender should hold two coinbase outputs"
    );

    // 30 NIGHT to the stranger, fee 0.001, change back to the sender.
    let tx = build_transfer(
        &sender,
        &spendables,
        &[Payment {
            to: stranger.address(),
            amount: 30 * DARKS_PER_NIGHT,
            memo: "Nightfall For Life".into(),
        }],
        100_000,
        &sender.address(),
        0,
        ctx,
    )
    .expect("transfer builds");

    let body = BlockBody::aggregate(&[tx]);

    // What the stranger's wallet does: scan every output in the block.
    let their_view = stranger.view_key();
    let found: Vec<_> = body
        .outputs
        .iter()
        .filter_map(|o| scan_output(&their_view, o))
        .collect();

    assert_eq!(found.len(), 1, "the stranger must find exactly one output");
    assert_eq!(found[0].value, 30 * DARKS_PER_NIGHT, "for the amount sent");
    assert_eq!(found[0].memo, "Nightfall For Life", "with the memo");

    // And the sender finds only their change, never the payment.
    let mine: Vec<_> = body
        .outputs
        .iter()
        .filter_map(|o| scan_output(&sender.view_key(), o))
        .collect();
    assert_eq!(mine.len(), 1, "sender sees only change");
    assert_eq!(
        mine[0].value,
        40 * DARKS_PER_NIGHT - 30 * DARKS_PER_NIGHT - 100_000
    );
}
