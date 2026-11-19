//! A real node mining with the Apple GPU produces blocks the chain accepts.
//! Needs a real GPU, so it runs only when asked:
//! `NIGHTFALL_GPU_TESTS=1 cargo test -p nightfall-node --test gpu_mining`.
#![cfg(all(feature = "gpu", target_os = "macos"))]

use nightfall_crypto::WalletKeys;
use nightfall_node::runtime::{GPU_BATCHES, GPU_MINING};
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_types::NetworkId;
use std::net::TcpListener;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(dir: &std::path::Path, listen: String, connect: Vec<String>, seed: u8) -> NodeHandle {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.to_path_buf(),
        p2p_listen: listen,
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect,
        mine: false,
        miner: Some(WalletKeys::from_seed([seed; 32]).address()),
        proxy: Some("off".into()),
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while node.status_snapshot().unwrap().loading {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    node
}

#[test]
fn gpu_mined_blocks_are_valid_and_counted() {
    if std::env::var_os("NIGHTFALL_GPU_TESTS").is_none() {
        eprintln!("skipped: set NIGHTFALL_GPU_TESTS=1 on a machine with a GPU");
        return;
    }
    // A node with no live peer deliberately does not mine (1.0.6: no blocks on
    // a tip nobody confirms), so the GPU miner gets a plain peer to talk to.
    let base = std::env::temp_dir().join(format!("nf-gpu-mine-{}", std::process::id()));
    let port = free_port();
    let peer = start(&base.join("peer"), format!("127.0.0.1:{port}"), vec![], 6);
    let node = start(
        &base.join("gpu"),
        "127.0.0.1:0".into(),
        vec![format!("127.0.0.1:{port}")],
        5,
    );
    let dir = base.clone();

    node.set_gpu_mining(true);
    assert!(node.gpu_mining());
    node.set_mining(true);

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let s = node.status_snapshot().unwrap();
        if s.tip_height >= 25 {
            assert!(s.blocks_found >= 25, "blocks not counted as ours");
            break;
        }
        assert!(
            GPU_MINING.load(Ordering::SeqCst),
            "GPU switched itself off (hash mismatch?)"
        );
        assert!(
            Instant::now() < deadline,
            "only {} blocks in 120 s",
            s.tip_height
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    node.set_mining(false);
    let batches = GPU_BATCHES.load(Ordering::SeqCst);
    assert!(
        batches >= 25,
        "blocks came from the CPU: only {batches} GPU batches"
    );
    eprintln!("GPU batches dispatched: {batches}");

    // Every block went through apply_block, which re-verifies proof of work.
    let shared = node.shared();
    let g = shared.lock().unwrap();
    g.chain.verify_supply().unwrap();
    let blocks = g.chain.blocks_from(1, 1_000);
    assert!(blocks.len() >= 25);
    assert!(
        blocks
            .iter()
            .any(|b| b.header.height.0 >= NetworkId::Devnet.binding_fork_height()),
        "GPU test must exercise the activated binding rules"
    );
    for block in &blocks {
        assert!(
            block.pow_is_valid(g.chain.pow_params()),
            "block {} fails PoW",
            block.header.height.0
        );
    }
    drop(g);
    // The plain peer validates and adopts the GPU-mined chain.
    let deadline = Instant::now() + Duration::from_secs(60);
    let want = node.status_snapshot().unwrap().tip;
    while peer.status_snapshot().unwrap().tip != want {
        assert!(
            Instant::now() < deadline,
            "peer never adopted the GPU-mined tip"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
