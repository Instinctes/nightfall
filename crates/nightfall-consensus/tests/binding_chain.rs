//! The binding fork at chain level: crossing the activation height, the
//! mempool's first-seen rule, and template filtering.

use nightfall_consensus::*;
use nightfall_crypto::{
    build_kernel, create_output_with_features, derive_input_secret, generator_g, random_scalar,
    scan_output, sig, sign_input_key, KernelFeature, OutputFeature, WalletKeys,
};
use nightfall_ledger::{
    build_transfer, build_transfer_with, BuildRules, Input, Payment, Spendable, Transaction,
    TX_VERSION,
};
use nightfall_types::{NetworkId, TARGET_BLOCK_TIME_SECS};

const NOW: u64 = 1_800_000_000;
const FEE: u64 = 1_000_000;

fn ctx() -> &'static [u8] {
    NetworkId::Devnet.proof_context()
}

fn fork() -> u64 {
    NetworkId::Devnet.binding_fork_height()
}

fn coin_at(chain: &Chain, owner: &WalletKeys, height: usize) -> Spendable {
    let view = owner.view_key();
    let d = chain.blocks[height]
        .body
        .outputs
        .iter()
        .find_map(|o| scan_output(&view, o))
        .unwrap();
    Spendable {
        commit: d.commit,
        value: d.value,
        blind: d.blind,
        spend_secret: d.spend_secret(owner),
    }
}

fn mine_n(chain: &mut Chain, miner: &WalletKeys, n: u64) {
    for _ in 0..n {
        let i = chain.block_count();
        chain
            .mine_block(&miner.address(), vec![], NOW + i * TARGET_BLOCK_TIME_SECS)
            .unwrap();
    }
}

fn pay(from: &WalletKeys, coin: Spendable, to: &WalletKeys, rules: BuildRules) -> Transaction {
    build_transfer_with(
        from,
        &[coin],
        &[Payment {
            to: to.address(),
            amount: 1_000,
            memo: String::new(),
        }],
        FEE,
        &from.address(),
        0,
        ctx(),
        rules,
    )
    .unwrap()
}

#[test]
fn a_chain_crosses_the_fork_and_replays() {
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    mine_n(&mut chain, &alice, 12);

    // Below the fork: a bound transaction is mined, fields stripped.
    let pre = pay(
        &alice,
        coin_at(&chain, &alice, 0),
        &bob,
        BuildRules::default(),
    );
    let h = chain.block_count();
    let b = chain
        .mine_block(
            &alice.address(),
            vec![pre],
            NOW + h * TARGET_BLOCK_TIME_SECS,
        )
        .unwrap();
    assert!(b.header.height.0 < fork());
    assert!(
        !b.body.has_any_binding(),
        "pre-fork blocks keep the 1.0.x shape"
    );
    assert_eq!(b.body.kernels.len(), 2);

    let left = fork() - chain.block_count();
    mine_n(&mut chain, &alice, left);

    // At and above the fork: full binding, with a kernel offset.
    let rules = BuildRules::for_height(NetworkId::Devnet, chain.next_height().0);
    assert!(rules.kernel_offset);
    let post = pay(&alice, coin_at(&chain, &alice, 1), &bob, rules);
    let h = chain.block_count();
    let b = chain
        .mine_block(
            &alice.address(),
            vec![post],
            NOW + h * TARGET_BLOCK_TIME_SECS,
        )
        .unwrap();
    assert!(b.header.height.0 >= fork());
    assert!(b.body.has_any_binding());
    assert!(b.body.kernel_offset.is_some());
    chain.verify_supply().unwrap();

    // A fresh node replays the whole chain, both rule sets.
    let mut peer = Chain::new_fair(NetworkId::Devnet).unwrap();
    let n = peer
        .try_ingest_blocks(chain.blocks.clone(), NOW + 10_000)
        .unwrap();
    assert_eq!(n, chain.blocks.len());
    assert_eq!(peer.tip_hash(), chain.tip_hash());
    assert_eq!(peer.ledger.kernel_sum(), chain.ledger.kernel_sum());

    // And so does the trusted own-disk path.
    let mut disk = Chain::new_fair(NetworkId::Devnet).unwrap();
    for b in chain.blocks.clone() {
        disk.apply_block_from_own_disk(b).unwrap();
    }
    assert_eq!(disk.ledger.kernel_sum(), chain.ledger.kernel_sum());
    disk.verify_supply().unwrap();
}

#[test]
fn after_the_fork_an_unbound_transaction_is_refused() {
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    mine_n(&mut chain, &alice, fork());
    let mut legacy = pay(
        &alice,
        coin_at(&chain, &alice, 0),
        &bob,
        BuildRules::default(),
    );
    legacy.strip_binding();
    assert!(chain.precheck_tx(&legacy).is_err());
}

#[test]
fn first_seen_wins_in_the_mempool() {
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    mine_n(&mut chain, &alice, 12);
    let coin = coin_at(&chain, &alice, 0);
    let a = pay(&alice, coin.clone(), &bob, BuildRules::default());
    let b = build_transfer(&alice, &[coin], &[], 5 * FEE, &alice.address(), 0, ctx()).unwrap();
    chain.precheck_tx(&a).unwrap();
    chain.precheck_tx(&b).unwrap();

    let mut pool = Mempool::default();
    assert_eq!(pool.insert_checked(a.clone(), NOW), InsertOutcome::Added);
    assert_eq!(
        pool.insert_checked(b.clone(), NOW + 1),
        InsertOutcome::Conflict,
        "a higher fee does not displace it"
    );
    assert_eq!(pool.select_for_block(10), vec![a.clone()]);

    // The one replacement allowed: the same spend, regaining its binding
    // fields after an old relay dropped them.
    let mut pool = Mempool::default();
    let mut stripped = a.clone();
    stripped.strip_binding();
    assert_eq!(pool.insert_checked(stripped, NOW), InsertOutcome::Added);
    assert_eq!(
        pool.insert_checked(a.clone(), NOW + 1),
        InsertOutcome::Added
    );
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.select_for_block(10), vec![a]);
    // A different spend of the same coin, even fully bound: still refused.
    assert_eq!(pool.insert_checked(b, NOW + 2), InsertOutcome::Conflict);
}

#[test]
fn a_flagged_transaction_is_dropped_from_templates() {
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    mine_n(&mut chain, &alice, 12);

    let coin = coin_at(&chain, &alice, 0);
    let (out, s) = create_output_with_features(
        &alice.address(),
        coin.value - FEE,
        "",
        ctx(),
        OutputFeature::Coinbase,
    )
    .unwrap();
    let ko = (generator_g() * coin.spend_secret).compress().to_bytes();
    let ki = derive_input_secret(&coin.spend_secret, &coin.commit);
    let (input_key, key_sig) = sign_input_key(&ki, &coin.spend_secret, &coin.commit, &ko);
    let mut k = build_kernel(KernelFeature::Plain, FEE, 0, 0, &(s.blind - coin.blind));
    let off = random_scalar();
    k.attach_stealth(&(ki - s.ephemeral - off));
    let flagged_tx = Transaction {
        version: TX_VERSION,
        inputs: vec![Input {
            commit: coin.commit,
            sig: sig::sign(
                &coin.spend_secret,
                &generator_g(),
                &Transaction::input_message(&coin.commit),
            ),
            input_key: Some(input_key),
            key_sig: Some(key_sig),
        }],
        outputs: vec![out],
        kernels: vec![k],
        stealth_offset: Some(off.to_bytes()),
        kernel_offset: None,
    };
    assert!(
        chain.precheck_tx(&flagged_tx).is_err(),
        "admission refuses it"
    );

    let honest = pay(
        &alice,
        coin_at(&chain, &alice, 1),
        &bob,
        BuildRules::default(),
    );
    let (template, dropped) = chain
        .build_template_filtering(
            &alice.address(),
            vec![flagged_tx.clone(), honest],
            NOW + 1_000,
        )
        .unwrap();
    assert_eq!(
        template.body.kernels.len(),
        2,
        "the honest payment is mined"
    );
    assert_eq!(dropped, vec![flagged_tx.txid().to_hex()]);
}
