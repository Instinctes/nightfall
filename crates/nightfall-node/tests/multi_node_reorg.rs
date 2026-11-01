//! Real multi-node fork/reorg convergence test.
//!
//! The setup is controlled in-memory only until the fork exists.
//! Fork discovery, transfer, validation and adoption then happen over
//! the real P2P sockets.

use nightfall_crypto::{Address, WalletKeys};
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_types::NetworkId;
use std::{
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).expect("create tempdir");
        Self(p)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn boot(tag: &str) -> (NodeHandle, u16, TempDir) {
    let dir = TempDir::new(tag);
    let p2p = free_port();

    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.path().to_path_buf(),
        p2p_listen: format!("127.0.0.1:{p2p}"),
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect: vec![],
        mine: false,
        miner: None,
        proxy: Some("off".into()),
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .expect("node starts");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = node.status_snapshot().expect("status");
        if !s.loading {
            break;
        }
        assert!(Instant::now() < deadline, "node did not finish loading");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Listener is spawned on its own thread.
    std::thread::sleep(Duration::from_millis(200));

    (node, p2p, dir)
}

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

        let block = chain
            .mine_block(miner, txs, timestamp)
            .expect("mine devnet block");

        let mut state = shared.lock().unwrap();
        state.chain = chain;
        state.mempool.remove_included(&block);
        state.behind_since = 0;
        state.persist().expect("persist mined chain");
    }
}

fn copy_chain(from: &NodeHandle, to: &NodeHandle) {
    let chain = from.shared().lock().unwrap().chain.clone();

    let shared = to.shared();
    let mut state = shared.lock().unwrap();
    state.chain = chain;
    state.behind_since = 0;
    state.persist().expect("persist copied chain");
}

#[test]
fn three_real_nodes_converge_across_competing_forks() {
    let (a, _a_port, _a_dir) = boot("nf-reorg-a");
    let (b, b_port, _b_dir) = boot("nf-reorg-b");
    let (c, c_port, _c_dir) = boot("nf-reorg-c");

    let miner_a = WalletKeys::from_seed([11u8; 32]).address();
    let miner_c = WalletKeys::from_seed([33u8; 32]).address();

    // All three nodes start from the exact same non-genesis ancestor.
    mine(&a, &miner_a, 2);
    copy_chain(&a, &b);
    copy_chain(&a, &c);

    let common_a = a.status_snapshot().unwrap();
    let common_b = b.status_snapshot().unwrap();
    let common_c = c.status_snapshot().unwrap();

    assert_eq!(common_a.tip, common_b.tip);
    assert_eq!(common_a.tip, common_c.tip);
    assert_eq!(common_a.total_work, common_b.total_work);
    assert_eq!(common_a.total_work, common_c.total_work);

    // A and B share the shorter fork.
    mine(&a, &miner_a, 2);
    copy_chain(&a, &b);

    // C independently builds a heavier competing fork.
    mine(&c, &miner_c, 4);

    let fork_a = a.status_snapshot().unwrap();
    let fork_b = b.status_snapshot().unwrap();
    let fork_c = c.status_snapshot().unwrap();

    assert_eq!(fork_a.tip, fork_b.tip);
    assert_ne!(fork_a.tip, fork_c.tip);

    assert!(
        fork_c.total_work > fork_a.total_work,
        "C must have the heavier branch: A={}, C={}",
        fork_a.total_work,
        fork_c.total_work
    );

    let expected_tip = fork_c.tip.clone();
    let expected_work = fork_c.total_work;
    let expected_height = fork_c.tip_height;
    let expected_utxo_root = fork_c.utxo_root.clone();
    let expected_minted = fork_c.minted;
    let expected_burned = fork_c.burned_fees;
    let expected_kernels = fork_c.kernels;

    // Topology:
    //
    //     A ---> B ---> C
    //
    // A never receives C as an explicitly configured peer.
    // Therefore the heavier fork has to propagate through B.
    a.add_peer(&format!("127.0.0.1:{b_port}"))
        .expect("A adds B");

    b.add_peer(&format!("127.0.0.1:{c_port}"))
        .expect("B adds C");

    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        let sa = a.status_snapshot().expect("A status");
        let sb = b.status_snapshot().expect("B status");
        let sc = c.status_snapshot().expect("C status");

        let converged = sa.tip == expected_tip
            && sb.tip == expected_tip
            && sc.tip == expected_tip
            && sa.total_work == expected_work
            && sb.total_work == expected_work
            && sc.total_work == expected_work;

        if converged {
            for (name, s) in [("A", &sa), ("B", &sb), ("C", &sc)] {
                assert_eq!(
                    s.tip_height, expected_height,
                    "{name}: wrong height after convergence"
                );
                assert_eq!(
                    s.utxo_root, expected_utxo_root,
                    "{name}: UTXO root diverged"
                );
                assert_eq!(s.minted, expected_minted, "{name}: minted supply diverged");
                assert_eq!(
                    s.burned_fees, expected_burned,
                    "{name}: burned fees diverged"
                );
                assert_eq!(s.kernels, expected_kernels, "{name}: kernel count diverged");
                assert!(s.supply_ok, "{name}: supply invariant failed");
                assert!(!s.reorg_in_flight, "{name}: reorg still marked in flight");
                assert!(!s.stalled_on_fork, "{name}: node remained stalled on fork");
            }

            return;
        }

        assert!(
            Instant::now() < deadline,
            "3-node convergence failed\n\
             A: tip={} h={} work={} live={} stalled={} reorg={}\n\
             B: tip={} h={} work={} live={} stalled={} reorg={}\n\
             C: tip={} h={} work={} live={} stalled={} reorg={}",
            sa.tip,
            sa.tip_height,
            sa.total_work,
            sa.live_peers,
            sa.stalled_on_fork,
            sa.reorg_in_flight,
            sb.tip,
            sb.tip_height,
            sb.total_work,
            sb.live_peers,
            sb.stalled_on_fork,
            sb.reorg_in_flight,
            sc.tip,
            sc.tip_height,
            sc.total_work,
            sc.live_peers,
            sc.stalled_on_fork,
            sc.reorg_in_flight,
        );

        std::thread::sleep(Duration::from_millis(100));
    }
}
