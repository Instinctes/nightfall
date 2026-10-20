use nightfall_consensus::Chain;
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;
use std::fs;

#[test]
fn interrupted_reset_blocks_all_normal_storage_entry_points() {
    let dir = std::env::temp_dir().join(format!(
        "nf-journal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    fs::create_dir(&dir).unwrap();

    let store = ChainStore::new(&dir);
    let chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    store.save(&chain).unwrap();

    let backup = dir.join("backup");
    fs::create_dir(&backup).unwrap();

    fs::rename(dir.join("blocks.bin"), backup.join("blocks.bin")).unwrap();

    let original = fs::read(dir.join("chain-meta.json")).unwrap();

    fs::write(
        dir.join(".stalled-fork-recovery-in-progress"),
        b"incomplete",
    )
    .unwrap();

    assert!(store.load_or_new(NetworkId::Devnet).is_err());
    assert!(store
        .load_or_new_with_progress(NetworkId::Devnet, |_, _| {})
        .is_err());
    assert!(store.save(&chain).is_err());
    assert!(store.migrate_to_binary().is_err());
    assert!(store.export_snapshot(&dir.join("export")).is_err());
    assert!(store.import_snapshot(&backup, NetworkId::Devnet).is_err());

    assert!(!dir.join("blocks.bin").exists());
    assert_eq!(fs::read(dir.join("chain-meta.json")).unwrap(), original);

    fs::remove_dir_all(dir).unwrap();
}
