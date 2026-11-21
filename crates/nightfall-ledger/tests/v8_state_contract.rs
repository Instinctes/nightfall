use nightfall_crypto::{domain, hash_multi, Commitment};
use nightfall_ledger::{UtxoEntry, UtxoSet};

fn entry(output_byte: u8, height: u64, is_coinbase: bool) -> UtxoEntry {
    UtxoEntry {
        output_pk: [output_byte; 32],
        height,
        is_coinbase,
    }
}

#[test]
fn v8_utxo_merkle_domains_are_frozen() {
    assert_eq!(domain::MERKLE, b"nightfall:merkle:v2");
    assert_eq!(domain::MERKLE_LEAF, b"nightfall:merkle:leaf:v2");
}

#[test]
fn v8_single_utxo_leaf_encoding_is_frozen() {
    let commit = Commitment([0x21; 32]);
    let e = entry(0x31, 1234, false);

    let mut set = UtxoSet::new();
    assert!(set.insert(commit, e.clone()));

    let expected = hash_multi(
        domain::MERKLE_LEAF,
        &[&commit.0, &e.output_pk, &e.height.to_le_bytes()],
    );

    assert_eq!(set.root(), expected, "V8 UTXO leaf encoding changed");
}

#[test]
fn v8_utxo_root_commits_to_output_key_and_creation_height() {
    let commit = Commitment([0x41; 32]);

    let mut base = UtxoSet::new();
    base.insert(commit, entry(0x51, 100, false));

    let mut different_key = UtxoSet::new();
    different_key.insert(commit, entry(0x52, 100, false));

    let mut different_height = UtxoSet::new();
    different_height.insert(commit, entry(0x51, 101, false));

    assert_ne!(base.root(), different_key.root());
    assert_ne!(base.root(), different_height.root());
}

#[test]
fn v8_utxo_root_does_not_authenticate_coinbase_metadata() {
    let commit = Commitment([0x61; 32]);

    let mut regular = UtxoSet::new();
    regular.insert(commit, entry(0x71, 500, false));

    let mut coinbase = UtxoSet::new();
    coinbase.insert(commit, entry(0x71, 500, true));

    assert_eq!(
        regular.root(),
        coinbase.root(),
        "V8 root semantics changed; snapshot authentication must solve this outside consensus"
    );
}

#[test]
fn v8_utxo_root_is_independent_of_insertion_order() {
    let c1 = Commitment([0x81; 32]);
    let c2 = Commitment([0x91; 32]);

    let mut a = UtxoSet::new();
    a.insert(c1, entry(0xa1, 10, false));
    a.insert(c2, entry(0xb1, 11, false));

    let mut b = UtxoSet::new();
    b.insert(c2, entry(0xb1, 11, false));
    b.insert(c1, entry(0xa1, 10, false));

    assert_eq!(a.root(), b.root(), "V8 UTXO root must remain canonical");
}

#[test]
fn local_state_fingerprint_authenticates_coinbase_metadata_without_changing_v8() {
    use nightfall_ledger::{state_fingerprint, LedgerState};
    use nightfall_types::NetworkId;

    let commit = Commitment([0x61; 32]);

    let mut regular = LedgerState::for_network(NetworkId::Devnet);
    regular.utxos.insert(commit, entry(0x71, 500, false));

    let mut coinbase = LedgerState::for_network(NetworkId::Devnet);
    coinbase.utxos.insert(commit, entry(0x71, 500, true));

    // Consensus behaviour is frozen.
    assert_eq!(regular.utxo_root(), coinbase.utxo_root());

    // Snapshot/storage authentication is deliberately stronger.
    assert_ne!(
        state_fingerprint(&regular),
        state_fingerprint(&coinbase),
        "local chainstate authentication lost consensus-relevant metadata"
    );
}
