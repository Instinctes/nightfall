//! Real local Devnet lifecycle in subprocesses: runtime threads die BEFORE cleanup.
//! No production datadir, public seeds, directory fetch, automatic miner or peer.
use crate::wallet_state::{Custody, WalletState};
use nightfall_crypto::{Address, WalletKeys};
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_types::NetworkId;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const PASSWORD: &str = "public unfunded node vault password";
const ENV_ROOT: &str = "NF_VAULT_NODE_TEST_ROOT";

fn mine(node: &NodeHandle, miner: &Address, count: u32) {
    for _ in 0..count {
        let shared = node.shared();
        let (mut chain, txs) = {
            let state = shared.lock().unwrap();
            (
                state.chain.clone(),
                state.mempool.txs.values().cloned().collect(),
            )
        };
        let timestamp = nightfall_storage::now_unix().max(chain.median_time_past() + 1);
        let block = chain.mine_block(miner, txs, timestamp).unwrap();
        let mut state = shared.lock().unwrap();
        state.chain = chain;
        state.mempool.remove_included(&block);
        state.behind_since = 0;
        state.persist().unwrap();
    }
}

fn open_wallet(root: &Path, name: &str, seed: u8) -> WalletState {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    let lock = Arc::new(nightfall_storage::dirlock::acquire(&path).unwrap());
    let mut wallet = if WalletState::seed_exists(&path) {
        WalletState::open_vault(lock, NetworkId::Devnet).unwrap()
    } else {
        let mut wallet = WalletState::empty();
        wallet
            .initialize_vault(
                lock,
                NetworkId::Devnet,
                crate::onboarding::ProvisionSource::Words(zeroize::Zeroizing::new(
                    WalletKeys::from_seed([seed; 32]).to_mnemonic(),
                )),
                PASSWORD,
            )
            .unwrap();
        wallet
    };
    assert_eq!(wallet.custody(), Custody::Locked);
    wallet.unlock_vault(PASSWORD).unwrap();
    wallet
}

#[test]
#[ignore = "subprocess helper; invoked only by isolated_vault_node_lifecycle"]
fn vault_node_child() {
    let root =
        PathBuf::from(std::env::var_os(ENV_ROOT).expect("test parent supplies private root"));
    assert!(root.is_absolute() && root.is_dir());
    assert!(root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("nightfall-vault-node-e2e-"));
    assert!(NetworkId::Devnet.seed_nodes().is_empty());
    let phase = std::env::var("NF_VAULT_NODE_TEST_PHASE").unwrap();
    let alice_keys = WalletKeys::from_seed([0; 32]);
    let bob_keys = WalletKeys::from_seed([1; 32]);
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: root.join("node"),
        p2p_listen: "127.0.0.1:0".into(),
        rpc_listen: "127.0.0.1:0".into(),
        connect: vec![],
        mine: false,
        miner: None,
        proxy: Some("off".into()),
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while node.status_snapshot().unwrap().loading {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut alice = open_wallet(&root, "alice", 0);
    let mut bob = open_wallet(&root, "bob", 1);
    if phase == "exercise" {
        mine(&node, &alice_keys.address(), 12);
        alice.sync_from_node(&node).unwrap();
        bob.sync_from_node(&node).unwrap();
        assert!(alice.balances(12, 10).available > 100_000_100);
        alice.check_rescan_allowed().unwrap();
        let reserved = alice.pick_commit_hexes_at(100_000_100, 12, 10).unwrap();
        alice.reserve_commits(&reserved).unwrap();
        let reserved_snapshot = fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap();
        assert!(alice.rescan(&node).is_err());
        assert_eq!(
            fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap(),
            reserved_snapshot
        );
        alice.release_commits(&reserved).unwrap();
        alice.check_rescan_allowed().unwrap();
        let before_payment_chain = node.shared().lock().unwrap().chain.clone();
        let txid = alice
            .send(
                &node,
                &bob_keys.address().encode(),
                100_000_000,
                100,
                "local lifecycle test",
            )
            .unwrap();
        assert!(alice
            .history()
            .iter()
            .any(|entry| entry.txid == txid && entry.is_pending() && entry.raw.is_some()));
        assert_eq!(node.shared().lock().unwrap().mempool.txs.len(), 1);
        let pending_snapshot = fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap();
        assert!(alice.check_rescan_allowed().is_err());
        assert!(alice.rescan(&node).is_err());
        assert_eq!(
            fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap(),
            pending_snapshot
        );
        assert_eq!(node.shared().lock().unwrap().mempool.txs.len(), 1);
        // Pending is committed before submission and survives a wallet restart.
        drop(alice);
        alice = open_wallet(&root, "alice", 0);
        assert!(alice
            .history()
            .iter()
            .any(|entry| entry.txid == txid && entry.is_pending()));
        alice.lock_vault();
        assert!(alice.sync_from_node(&node).is_err());
        assert!(alice
            .send(&node, &bob_keys.address().encode(), 100, 10, "locked")
            .is_err());
        let locked_bytes = fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap();
        mine(&node, &bob_keys.address(), 1);
        assert_eq!(
            fs::read(root.join("alice/core.seed.vault/wallet.nfv")).unwrap(),
            locked_bytes
        );
        alice.unlock_vault(PASSWORD).unwrap();
        alice.sync_from_node(&node).unwrap();
        bob.sync_from_node(&node).unwrap();
        assert!(alice
            .history()
            .iter()
            .any(|entry| entry.txid == txid && !entry.is_pending()));
        assert_eq!(bob.balances(13, 10).available, 100_000_000);
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());

        // A shorter chain is not a successful empty scan. Both scanning and
        // spending must refuse without changing the authenticated snapshot.
        let canonical_chain = node.shared().lock().unwrap().chain.clone();
        let snapshot_path = root.join("alice/core.seed.vault/wallet.nfv");
        let canonical_snapshot = fs::read(&snapshot_path).unwrap();
        node.shared().lock().unwrap().chain = before_payment_chain;
        let error = alice.sync_from_node(&node).unwrap_err().to_string();
        // The wallet stands at 12; the restored shorter chain ends at 11. Both
        // numbers belong in the refusal, because "the chain got shorter" is
        // only actionable if the owner can see by how much.
        assert!(
            error.contains("already scanned height 12") && error.contains("chain ends at 11"),
            "{error}"
        );
        assert!(alice
            .send(
                &node,
                &bob_keys.address().encode(),
                100,
                10,
                "shorter chain"
            )
            .is_err());
        assert_eq!(fs::read(&snapshot_path).unwrap(), canonical_snapshot);
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());

        // Same height, another valid branch: the scan anchor must still stop
        // coin selection BEFORE an invalid pending payment can be persisted.
        mine(&node, &bob_keys.address(), 1);
        assert_ne!(
            node.shared().lock().unwrap().chain.tip_hash(),
            canonical_chain.tip_hash()
        );
        let error = alice.sync_from_node(&node).unwrap_err().to_string();
        assert!(error.contains("chain changed"), "{error}");
        assert!(alice
            .send(
                &node,
                &bob_keys.address().encode(),
                100,
                10,
                "changed anchor"
            )
            .is_err());
        assert_eq!(fs::read(&snapshot_path).unwrap(), canonical_snapshot);
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());
        alice.check_rescan_allowed().unwrap();

        // A new tip that has not been scanned cannot be used to infer that
        // old coin-selection state is still current either.
        node.shared().lock().unwrap().chain = canonical_chain.clone();
        mine(&node, &bob_keys.address(), 1);
        let error = alice
            .send(&node, &bob_keys.address().encode(), 100, 10, "scan behind")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Finish scanning"), "{error}");
        assert_eq!(fs::read(&snapshot_path).unwrap(), canonical_snapshot);
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());
        alice.check_rescan_allowed().unwrap();
        {
            let shared = node.shared();
            let mut state = shared.lock().unwrap();
            state.chain = canonical_chain;
            state.persist().unwrap();
        }
        alice.sync_from_node(&node).unwrap();

        // A failed durable save must never reach the real node's mempool.
        let path = root.join("alice/core.seed.vault/wallet.nfv");
        let preserved = fs::read(&path).unwrap();
        fs::write(&path, b"test-only simulated external damage").unwrap();
        assert!(alice
            .send(
                &node,
                &bob_keys.address().encode(),
                100,
                10,
                "must not submit"
            )
            .is_err());
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());
        // Restore only this parent-owned fixture for the second process.
        fs::write(&path, &preserved).unwrap();
        assert!(alice
            .history()
            .iter()
            .all(|entry| entry.memo != "must not submit"));
    } else {
        assert_eq!(phase, "reopen");
        alice.sync_from_node(&node).unwrap();
        bob.sync_from_node(&node).unwrap();
        assert!(alice
            .history()
            .iter()
            .any(|entry| entry.memo == "local lifecycle test" && !entry.is_pending()));
        assert_eq!(bob.balances(13, 10).available, 100_000_000);
        assert!(node.shared().lock().unwrap().mempool.txs.is_empty());
    }
    assert!(node.shared().lock().unwrap().bootstrap.is_empty());
    assert!(node.shared().lock().unwrap().peer_addrs.is_empty());
    assert!(!root.join("alice/core.seed.outputs.json").exists());
    assert!(!root.join("bob/core.seed.outputs.json").exists());
    alice.lock_vault();
    bob.lock_vault();
}

#[test]
fn isolated_vault_node_lifecycle() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nightfall-vault-node-e2e-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    for phase in ["exercise", "reopen"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "vault_node_tests::vault_node_child",
                "--ignored",
                "--nocapture",
            ])
            .env(ENV_ROOT, &root)
            .env("NF_VAULT_NODE_TEST_PHASE", phase)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(180);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "isolated node test timed out; public fixtures retained at {}",
                    root.display()
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(
            status.success(),
            "isolated node phase {phase} failed; fixtures retained at {}",
            root.display()
        );
    }
    // Neither child nor any of its forever-running node threads is alive now.
    assert!(root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("nightfall-vault-node-e2e-"));
    fs::remove_dir_all(root).unwrap();
}
