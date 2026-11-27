use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT};
use nightfall_wallet::{vault::Vault, Wallet};

// Public synthetic fixtures. Never use these keys or passwords
// for real funds. The entire chain exists only in this process.
const NET: NetworkId = NetworkId::Devnet;
const MATURITY: u64 = 10;
const START: u64 = 1_800_000_000;
const PAYMENT: u64 = DARKS_PER_NIGHT;
const FEE: u64 = DARKS_PER_NIGHT / 1_000;
const PASSWORD: &str = "public synthetic recovery fixture";

fn wallet(seed: u8) -> Wallet {
    Wallet::in_memory(NET, WalletKeys::from_seed([seed; 32]), 0)
}

fn scan(wallet: &mut Wallet, chain: &Chain) {
    wallet
        .scan_blocks(&chain.blocks_from(0, usize::MAX))
        .expect("canonical Devnet scan must succeed");
}

fn available(wallet: &Wallet, chain: &Chain) -> u64 {
    wallet
        .balances(chain.tip_height().unwrap().0, MATURITY)
        .available
}

fn assert_same_funds(expected: &Wallet, actual: &Wallet, chain: &Chain) {
    assert_eq!(expected.address_string(), actual.address_string());
    assert_eq!(expected.scanned_to(), actual.scanned_to());
    assert_eq!(expected.scan_anchor(), actual.scan_anchor());
    assert_eq!(available(expected, chain), available(actual, chain));

    let mut expected_outputs: Vec<_> = expected
        .outputs()
        .iter()
        .map(|o| (o.commit.to_hex(), o.value, o.height, o.spent))
        .collect();

    let mut actual_outputs: Vec<_> = actual
        .outputs()
        .iter()
        .map(|o| (o.commit.to_hex(), o.value, o.height, o.spent))
        .collect();

    expected_outputs.sort_unstable();
    actual_outputs.sort_unstable();

    assert_eq!(expected_outputs, actual_outputs);
}

fn funded_fixture() -> (Chain, Wallet, Wallet, u64) {
    let mut chain = Chain::new_fair(NET).unwrap();
    let mut owner = wallet(41);
    let miner = wallet(42);
    let mut timestamp = START;

    // Generate rewards for the disposable owner.
    for _ in 0..3 {
        chain
            .mine_block(&owner.address(), vec![], timestamp)
            .expect("funding block");
        timestamp += 15;
    }

    // Let the owner's rewards mature while another wallet mines.
    for _ in 0..MATURITY + 2 {
        chain
            .mine_block(&miner.address(), vec![], timestamp)
            .expect("maturity block");
        timestamp += 15;
    }

    scan(&mut owner, &chain);
    assert!(available(&owner, &chain) >= PAYMENT + FEE);
    chain.verify_supply().unwrap();

    (chain, owner, miner, timestamp)
}

fn prove_spend(chain: &mut Chain, recovered: &mut Wallet, miner: &Wallet, timestamp: u64) {
    let mut recipient = wallet(43);
    let height = chain.tip_height().unwrap().0;
    let before = available(recovered, chain);

    scan(&mut recipient, chain);
    assert_eq!(available(&recipient, chain), 0);

    let tx = recovered
        .create_payment_at(
            &recipient.address(),
            PAYMENT,
            FEE,
            "synthetic recovery assurance",
            height,
            MATURITY,
        )
        .expect("recovered wallet must create a valid payment");

    chain
        .precheck_tx(&tx)
        .expect("consensus must accept recovered spend");

    chain
        .mine_block(&miner.address(), vec![tx.clone()], timestamp)
        .expect("recovered spend must be included");

    scan(recovered, chain);
    scan(&mut recipient, chain);

    assert_eq!(available(&recipient, chain), PAYMENT);
    assert_eq!(available(recovered, chain), before - PAYMENT - FEE);

    assert!(
        chain.precheck_tx(&tx).is_err(),
        "already consumed inputs must not be accepted again"
    );

    chain
        .verify_supply()
        .expect("supply invariant after recovered spend");
}

#[test]
fn funded_phrase_restore_can_spend() {
    let (mut chain, owner, miner, timestamp) = funded_fixture();

    // Use Nightfall's own recovery path, not a replacement KDF.
    let phrase = owner.recovery_phrase();

    let keys = WalletKeys::from_mnemonic(&phrase).expect("synthetic backup must decode");

    let mut recovered = Wallet::in_memory(NET, keys, 0);

    // No scan state is carried over from the original wallet.
    assert!(recovered.outputs().is_empty());

    scan(&mut recovered, &chain);
    assert_same_funds(&owner, &recovered, &chain);

    // A different valid phrase must not recover the owner's money.
    let unrelated = WalletKeys::from_seed([44; 32]).to_mnemonic();

    let mut wrong = Wallet::in_memory(NET, WalletKeys::from_mnemonic(&unrelated).unwrap(), 0);

    scan(&mut wrong, &chain);
    assert_ne!(wrong.address_string(), owner.address_string());
    assert_eq!(available(&wrong, &chain), 0);

    drop(owner);
    prove_spend(&mut chain, &mut recovered, &miner, timestamp);
}

#[test]
fn stale_encrypted_backup_catches_up_and_can_spend() {
    let (mut chain, mut owner, miner, mut timestamp) = funded_fixture();

    let original_state = owner.export_state().unwrap();

    let vault = Vault::create(&owner, PASSWORD).unwrap();
    let encrypted = vault.sealed_bytes().to_vec();

    assert_eq!(
        original_state,
        owner.export_state().unwrap(),
        "backup creation must not modify source wallet"
    );

    drop(vault);

    // The backup predates an additional matured mining reward.
    chain
        .mine_block(&owner.address(), vec![], timestamp)
        .unwrap();

    timestamp += 15;

    for _ in 0..MATURITY + 2 {
        chain
            .mine_block(&miner.address(), vec![], timestamp)
            .unwrap();
        timestamp += 15;
    }

    scan(&mut owner, &chain);

    let mut restored = Vault::from_bytes(&encrypted).unwrap();
    assert!(restored.is_locked());

    restored.unlock(PASSWORD, NET).unwrap();

    assert!(
        restored.wallet().unwrap().scanned_to() < owner.scanned_to(),
        "fixture must actually contain an old backup"
    );

    let recovered = restored.wallet_mut().unwrap();

    scan(recovered, &chain);
    assert_same_funds(&owner, recovered, &chain);

    drop(owner);

    prove_spend(&mut chain, recovered, &miner, timestamp);
}
