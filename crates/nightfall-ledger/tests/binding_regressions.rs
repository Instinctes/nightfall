//! Protocol 1.1.0 binding rules: input keys, the key balance, kernel offsets,
//! the coinbase opening and fee bounds, on both sides of the activation height.

use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::{
    build_kernel, create_output_with_features, derive_input_secret, generator_g, random_scalar,
    scan_output, sig, sign_input_key, KernelFeature, Output, OutputFeature, SenderSecrets,
    WalletKeys,
};
use nightfall_ledger::*;
use nightfall_types::{Height, DARKS_PER_NIGHT};

const CTX: &[u8] = b"nightfall:test:binding";
const REWARD: u64 = 20 * DARKS_PER_NIGHT;
const FEE: u64 = DARKS_PER_NIGHT / 100;

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

fn genesis_coin(ledger: &mut LedgerState, owner: &WalletKeys) -> (Transaction, Spendable) {
    let cb = build_coinbase(&owner.address(), REWARD, 0, CTX).unwrap();
    ledger
        .apply_block(
            &BlockBody::aggregate(std::slice::from_ref(&cb)),
            Height(0),
            REWARD,
            CTX,
        )
        .unwrap();
    let coin = discover(owner, &cb).remove(0);
    (cb, coin)
}

fn mine(ledger: &mut LedgerState, h: u64, txs: Vec<Transaction>) -> Result<(), LedgerError> {
    let mut all = vec![build_coinbase(&WalletKeys::generate().address(), REWARD, h, CTX).unwrap()];
    all.extend(txs);
    ledger.apply_block(&BlockBody::aggregate(&all), Height(h), REWARD, CTX)
}

/// A correctly bound transaction with hand-picked outputs, so a test can
/// build shapes `build_transfer` never would (flagged outputs, say).
fn bound_tx(coins: &[Spendable], outs: Vec<(Output, SenderSecrets)>, fee: u64) -> Transaction {
    let mut inputs = Vec::new();
    let (mut ki_sum, mut in_blind) = (Scalar::ZERO, Scalar::ZERO);
    for c in coins {
        let ko_pub = (generator_g() * c.spend_secret).compress().to_bytes();
        let ki = derive_input_secret(&c.spend_secret, &c.commit);
        let (input_key, key_sig) = sign_input_key(&ki, &c.spend_secret, &c.commit, &ko_pub);
        ki_sum += ki;
        in_blind += c.blind;
        inputs.push(Input {
            commit: c.commit,
            sig: sig::sign(
                &c.spend_secret,
                &generator_g(),
                &Transaction::input_message(&c.commit),
            ),
            input_key: Some(input_key),
            key_sig: Some(key_sig),
        });
    }
    let out_blind: Scalar = outs.iter().map(|(_, s)| s.blind).sum();
    let r_sum: Scalar = outs.iter().map(|(_, s)| s.ephemeral).sum();
    let mut kernel = build_kernel(KernelFeature::Plain, fee, 0, 0, &(out_blind - in_blind));
    let s = random_scalar();
    kernel.attach_stealth(&(ki_sum - r_sum - s));
    Transaction {
        version: TX_VERSION,
        inputs,
        outputs: outs.into_iter().map(|(o, _)| o).collect(),
        kernels: vec![kernel],
        stealth_offset: Some(s.to_bytes()),
        kernel_offset: None,
    }
}

#[test]
fn honest_bound_transfer_is_accepted_and_supply_holds() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let tx = build_transfer(
        &alice,
        &[coin],
        &[Payment {
            to: bob.address(),
            amount: 3 * DARKS_PER_NIGHT,
            memo: "hi".into(),
        }],
        FEE,
        &alice.address(),
        0,
        CTX,
    )
    .unwrap();
    assert!(tx.has_full_binding());
    ledger
        .check_tx_acceptable(&tx, Height(COINBASE_MATURITY), CTX)
        .unwrap();
    mine(&mut ledger, COINBASE_MATURITY, vec![tx.clone()]).unwrap();
    ledger.verify_supply().unwrap();
    assert_eq!(discover(&bob, &tx)[0].value, 3 * DARKS_PER_NIGHT);
}

#[test]
fn the_key_balance_is_enforced() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let tx = build_transfer(&alice, &[coin], &[], FEE, &alice.address(), 0, CTX).unwrap();

    // A stealth excess that does not balance the transaction's keys.
    let mut unbalanced = tx.clone();
    unbalanced.kernels[0].attach_stealth(&random_scalar());
    assert!(matches!(
        ledger.check_tx_acceptable(&unbalanced, Height(COINBASE_MATURITY), CTX),
        Err(LedgerError::Tx {
            source: TxError::KeyImbalance,
            ..
        })
    ));
    let r = mine(&mut ledger.clone(), COINBASE_MATURITY, vec![unbalanced]);
    assert!(
        matches!(r, Err(LedgerError::Binding(TxError::KeyImbalance))),
        "{r:?}"
    );

    // An input key signed by the wrong key.
    let mut wrong_sig = tx.clone();
    let other = derive_input_secret(&random_scalar(), &wrong_sig.inputs[0].commit);
    let (_, sig) = sign_input_key(
        &other,
        &random_scalar(),
        &wrong_sig.inputs[0].commit,
        &[0; 32],
    );
    wrong_sig.inputs[0].key_sig = Some(sig);
    let r = mine(&mut ledger.clone(), COINBASE_MATURITY, vec![wrong_sig]);
    assert!(matches!(r, Err(LedgerError::BadInputKeySignature)), "{r:?}");

    // No binding fields at all.
    let mut unbound = tx.clone();
    unbound.strip_binding();
    assert!(ledger
        .check_tx_acceptable(&unbound, Height(COINBASE_MATURITY), CTX)
        .is_err());
    assert!(mine(&mut ledger.clone(), COINBASE_MATURITY, vec![unbound]).is_err());

    mine(&mut ledger, COINBASE_MATURITY, vec![tx]).unwrap();
}

#[test]
fn coinbase_flag_is_refused_outside_a_coinbase() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let flagged = create_output_with_features(
        &alice.address(),
        coin.value - FEE,
        "",
        CTX,
        OutputFeature::Coinbase,
    )
    .unwrap();
    let poison = bound_tx(&[coin], vec![flagged], FEE);
    let r = ledger.check_tx_acceptable(&poison, Height(COINBASE_MATURITY), CTX);
    assert!(
        matches!(
            r,
            Err(LedgerError::Tx {
                source: TxError::CoinbaseOutputInTransaction,
                ..
            })
        ),
        "{r:?}"
    );
}

#[test]
fn the_subsidy_must_sit_in_the_flagged_output() {
    let mut ledger = LedgerState::genesis();
    let miner = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &miner);
    let h = COINBASE_MATURITY;

    // Subsidy in a Plain output...
    let (subsidy, s) =
        create_output_with_features(&miner.address(), REWARD, "", CTX, OutputFeature::Plain)
            .unwrap();
    let mut k = build_kernel(KernelFeature::Coinbase, 0, REWARD, h, &s.blind);
    let off = random_scalar();
    k.attach_stealth(&(-s.ephemeral - off));
    let cb = Transaction {
        version: TX_VERSION,
        inputs: vec![],
        outputs: vec![subsidy],
        kernels: vec![k],
        stealth_offset: Some(off.to_bytes()),
        kernel_offset: None,
    };
    // ...and the flag on a self-payment.
    let flagged = create_output_with_features(
        &miner.address(),
        coin.value - FEE,
        "",
        CTX,
        OutputFeature::Coinbase,
    )
    .unwrap();
    let decoy = bound_tx(&[coin], vec![flagged], FEE);

    let r = ledger.apply_block(&BlockBody::aggregate(&[cb, decoy]), Height(h), REWARD, CTX);
    assert!(matches!(r, Err(LedgerError::BadCoinbaseOpening)), "{r:?}");
}

#[test]
fn kernel_fees_are_bounded() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let mut tx = build_transfer(&alice, &[coin], &[], FEE, &alice.address(), 0, CTX).unwrap();
    let mut huge = build_kernel(KernelFeature::Plain, u64::MAX, 0, 0, &Scalar::from(7u64));
    huge.attach_stealth(&Scalar::ZERO);
    tx.kernels.push(huge);
    assert!(matches!(
        ledger.check_tx_acceptable(&tx, Height(COINBASE_MATURITY), CTX),
        Err(LedgerError::Tx {
            source: TxError::FeeOutOfRange,
            ..
        })
    ));
    let r = mine(&mut ledger, COINBASE_MATURITY, vec![tx]);
    assert!(matches!(r, Err(LedgerError::FeeOutOfRange)), "{r:?}");
}

#[test]
fn kernel_offset_hides_the_transaction_boundary() {
    let mut ledger = LedgerState::genesis();
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let tx = build_transfer_with(
        &alice,
        &[coin],
        &[Payment {
            to: bob.address(),
            amount: DARKS_PER_NIGHT,
            memo: String::new(),
        }],
        FEE,
        &alice.address(),
        0,
        CTX,
        BuildRules {
            kernel_offset: true,
        },
    )
    .unwrap();
    assert!(tx.kernel_offset.is_some());

    // The kernel's excess no longer equals its own transaction's commitments.
    let ins: Vec<_> = tx.inputs.iter().map(|i| i.commit).collect();
    let outs: Vec<_> = tx.outputs.iter().map(|o| o.commit).collect();
    let own = nightfall_crypto::expected_excess(&ins, &outs, FEE, 0).unwrap();
    assert_ne!(Some(own), tx.kernels[0].excess.point());

    mine(&mut ledger, COINBASE_MATURITY, vec![tx]).unwrap();
    ledger.verify_supply().unwrap();
}

#[test]
fn below_the_fork_blocks_are_judged_as_before() {
    let mut ledger = LedgerState::genesis().with_binding_height(u64::MAX);
    let alice = WalletKeys::generate();
    let (_, coin) = genesis_coin(&mut ledger, &alice);
    let bound = build_transfer(
        &alice,
        std::slice::from_ref(&coin),
        &[],
        FEE,
        &alice.address(),
        0,
        CTX,
    )
    .unwrap();

    // A kernel offset is not valid yet.
    let with_offset = build_transfer_with(
        &alice,
        std::slice::from_ref(&coin),
        &[],
        FEE,
        &alice.address(),
        0,
        CTX,
        BuildRules {
            kernel_offset: true,
        },
    )
    .unwrap();
    assert!(matches!(
        ledger.check_tx_acceptable(&with_offset, Height(COINBASE_MATURITY), CTX),
        Err(LedgerError::Tx {
            source: TxError::KernelOffsetBeforeFork,
            ..
        })
    ));

    // A bound transaction is accepted, and so is its stripped 1.0.x shape.
    ledger
        .check_tx_acceptable(&bound, Height(COINBASE_MATURITY), CTX)
        .unwrap();
    let mut legacy = bound.clone();
    legacy.strip_binding();
    ledger
        .check_tx_acceptable(&legacy, Height(COINBASE_MATURITY), CTX)
        .unwrap();
    assert_eq!(bound.legacy_txid(), legacy.legacy_txid());

    // Binding fields leave no trace below the fork: the same block with and
    // without them hashes identically once stripped, and both apply.
    let cb = build_coinbase(&alice.address(), REWARD, COINBASE_MATURITY, CTX).unwrap();
    let full = BlockBody::aggregate(&[cb.clone(), bound]);
    let mut stripped = full.clone();
    stripped.strip_binding();
    let mut legacy_cb = cb;
    legacy_cb.strip_binding();
    assert_eq!(
        stripped.hash(),
        BlockBody::aggregate(&[legacy_cb, legacy]).hash()
    );
    ledger
        .clone()
        .apply_block(&stripped, Height(COINBASE_MATURITY), REWARD, CTX)
        .unwrap();
    ledger
        .apply_block(&full, Height(COINBASE_MATURITY), REWARD, CTX)
        .unwrap();
    ledger.verify_supply().unwrap();
}
