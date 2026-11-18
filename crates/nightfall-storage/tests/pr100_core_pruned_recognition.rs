use nightfall_consensus::Chain;
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;

#[test]
fn only_unsealed_matching_legacy_pruned_state_is_offered_recovery() {
    let root = std::env::temp_dir().join(format!(
        "nightfall-pr100-core-{}-{}",
        std::process::id(),
        nightfall_storage::now_unix()
    ));

    std::fs::create_dir(&root).unwrap();

    let chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    let genesis = chain.genesis_hash.to_hex();
    let store = ChainStore::new(&root);

    assert!(!store.legacy_pruned_recovery_required(NetworkId::Devnet));
    assert!(!store.checkpoint_counts_unknown());

    std::fs::write(root.join("checkpoint-wallet-v3.bin"), b"artifact").unwrap();

    assert!(store.checkpoint_counts_unknown());

    std::fs::remove_file(root.join("checkpoint-wallet-v3.bin")).unwrap();

    assert!(!store.checkpoint_counts_unknown());

    std::fs::write(root.join("blocks.bin"), b"untrusted legacy bodies").unwrap();

    std::fs::write(root.join("headers.jsonl"), b"legacy headers").unwrap();

    std::fs::write(root.join("utxo-horizon.json"), b"legacy horizon").unwrap();

    let mut meta = serde_json::json!({
        "network": NetworkId::Devnet,
        "genesis_hash": genesis,
        "block_count": 11,
        "protocol_version": nightfall_types::PROTOCOL_VERSION,
        "validated_by": "legacy",
        "validated_tip": "00",
        "validated_bytes": 23,
        "first_height": 10,
        "pruned": true,
        "horizon_work": "0",
        "validated_horizon_bytes": 10,
        "validated_headers_bytes": 10
    });

    std::fs::write(root.join("chain-meta.json"), meta.to_string()).unwrap();

    assert!(store.legacy_pruned_recovery_required(NetworkId::Devnet));

    assert!(!store.legacy_pruned_recovery_required(NetworkId::Mainnet));

    meta["validated_horizon_fingerprint"] = "aa".into();
    meta["validated_headers_fingerprint"] = "bb".into();

    std::fs::write(root.join("chain-meta.json"), meta.to_string()).unwrap();

    assert!(!store.legacy_pruned_recovery_required(NetworkId::Devnet));

    std::fs::remove_dir_all(&root).unwrap();
}
