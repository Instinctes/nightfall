//! Node Kit legacy pruned upgrade: destructive path must require opt-in.
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn upgrade_pruned_without_force_never_mutates_wallet_or_chain() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nf-nodekit-upgrade-refuse-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(dir.join("wallet")).unwrap();
    let wallet_file = dir.join("wallet").join("sentinel");
    let meta_file = dir.join("chain-meta.json");
    fs::write(&wallet_file, b"wallet must survive").unwrap();
    fs::write(&meta_file, b"not-a-chain-fixture").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nightfall-nodekit"))
        .arg("--network")
        .arg("devnet")
        .arg("upgrade-pruned")
        .arg("--datadir")
        .arg(&dir)
        .output()
        .expect("spawn Node Kit");

    assert!(
        !output.status.success(),
        "unsafe upgrade without --force succeeded"
    );
    assert_eq!(fs::read(&wallet_file).unwrap(), b"wallet must survive");
    assert_eq!(fs::read(&meta_file).unwrap(), b"not-a-chain-fixture");
    assert!(!dir.join(".recovery-journal.json").exists());

    fs::remove_dir_all(dir).unwrap();
}
