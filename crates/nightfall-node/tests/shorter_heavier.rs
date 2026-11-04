//! Real TCP acceptance: a shorter branch with more cumulative work wins.
//! Uses fully mined Devnet blocks, isolated datadirs and two live NodeHandles.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_storage::ChainStore;
use nightfall_types::{NetworkId, TARGET_BLOCK_TIME_SECS};
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn ephemeral_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until<F: FnMut() -> bool>(name: &str, duration: Duration, mut ok: F) {
    let deadline = Instant::now() + duration;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out: {name}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn boot(dir: &Path) -> (NodeHandle, u16) {
    let p2p = ephemeral_port();
    let rpc = ephemeral_port();
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.to_path_buf(),
        p2p_listen: format!("127.0.0.1:{p2p}"),
        rpc_listen: format!("127.0.0.1:{rpc}"),
        connect: Vec::new(),
        mine: false,
        miner: None,
        proxy: None,
        mobile_listen: None,
        peers_url: Some("off".to_owned()),
        introducer: false,
        prune: false,
    })
    .expect("start isolated node");

    wait_until("node TCP listener ready", Duration::from_secs(25), || {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], p2p));
        TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
            && node.status_snapshot().is_ok()
    });
    (node, p2p)
}

fn test_directory() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "nf-shorter-heavier-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn real_two_nodes_adopt_shorter_heavier_branch_only_one_way() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let base = now.saturating_sub(9_000);
    let shared_miner = WalletKeys::from_seed([0x41; 32]).address();
    let light_miner = WalletKeys::from_seed([0x42; 32]).address();
    let heavy_miner = WalletKeys::from_seed([0x43; 32]).address();

    // Difficulty adjusts over the 90-block LWMA window. Fast branch B
    // eventually has strictly more work despite having fewer blocks.
    let mut prefix = Chain::new_fair(NetworkId::Devnet).unwrap();
    for i in 0..2 {
        prefix
            .mine_block(
                &shared_miner,
                vec![],
                base + 10 + i * TARGET_BLOCK_TIME_SECS,
            )
            .unwrap();
    }

    // Clone the fully validated shared ancestor, not the genesis body twice.
    let mut a_long_light = prefix.clone();
    let mut b_short_heavy = prefix.clone();

    for i in 0..125u64 {
        a_long_light
            .mine_block(&light_miner, vec![], base + 50 + (i + 1) * 60)
            .unwrap();
    }
    for i in 0..115u64 {
        b_short_heavy
            .mine_block(&heavy_miner, vec![], base + 50 + (i + 1) * 2)
            .unwrap();
    }

    a_long_light.verify_supply().unwrap();
    b_short_heavy.verify_supply().unwrap();
    assert!(
        a_long_light.tip_height().unwrap().0 > b_short_heavy.tip_height().unwrap().0,
        "A must be strictly longer"
    );
    assert!(
        b_short_heavy.total_work > a_long_light.total_work,
        "B must be strictly heavier despite being shorter: B={}, A={}",
        b_short_heavy.total_work,
        a_long_light.total_work
    );

    let a_tip = a_long_light.tip_hash().to_hex();
    let b_tip = b_short_heavy.tip_hash().to_hex();
    let b_work = b_short_heavy.total_work;
    let b_root = b_short_heavy.ledger.utxo_root().to_hex();

    let root = test_directory();
    let adir = root.join("a");
    let bdir = root.join("b");
    ChainStore::new(&adir).save(&a_long_light).unwrap();
    ChainStore::new(&bdir).save(&b_short_heavy).unwrap();

    let (a, _a_port) = boot(&adir);
    let (b, b_port) = boot(&bdir);

    wait_until("both branches loaded", Duration::from_secs(45), || {
        let sa = a.status_snapshot().unwrap();
        let sb = b.status_snapshot().unwrap();
        !sa.loading && !sb.loading && sa.tip == a_tip && sb.tip == b_tip
    });

    // No peer is configured until both branches are verified on disk.
    a.add_peer(&format!("127.0.0.1:{b_port}")).unwrap();

    wait_until(
        "A adopts B shorter/heavier chain",
        Duration::from_secs(120),
        || {
            let sa = a.status_snapshot().unwrap();
            let sb = b.status_snapshot().unwrap();
            !sa.loading
                && !sb.loading
                && sa.tip == b_tip
                && sb.tip == b_tip
                && sa.total_work == b_work
                && sb.total_work == b_work
                && sa.utxo_root == b_root
                && sb.utxo_root == b_root
                && sa.supply_ok
                && sb.supply_ok
                && !sa.reorg_in_flight
                && !sb.reorg_in_flight
        },
    );

    // B must not switch onto A's longer but lighter chain.
    let sa = a.status_snapshot().unwrap();
    let sb = b.status_snapshot().unwrap();
    assert_ne!(a_tip, sa.tip, "A failed to leave the lighter chain");
    assert_eq!(sa.tip, b_tip);
    assert_eq!(sb.tip, b_tip);
    assert_eq!(sa.total_work, sb.total_work);
    assert_eq!(sa.utxo_root, sb.utxo_root);
    assert_eq!(sa.minted, sb.minted);
    assert_eq!(sa.burned_fees, sb.burned_fees);

    // Existing NodeHandle on this PR has no worker join lifecycle yet.
    // The ephemeral test-runner workspace is deliberately not removed
    // while either live node may have background writers.
    let _ = root;
}
