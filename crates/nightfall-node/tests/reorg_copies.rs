//! An archive node must not copy its history for a fork it cannot adopt.
//!
//! 1.0.6 seed incident (16 Oct 2026): the fork path copied every block since
//! genesis *before* asking whether the peer's branch was too deep or heavier.
//! With peers on dead forks around, the archive seed's memory jumped by
//! gigabytes within seconds, its light API stopped answering, and the kernel
//! killed it. Here: a peer ahead on a branch that diverges at genesis, deeper
//! than MAX_REORG_DEPTH. It must be refused without a single prefix copy.
//!
//! Mines ~1,050 real blocks: run by name, in release —
//!   cargo test --release -p nightfall-node --test reorg_copies -- --ignored

use nightfall_consensus::{Chain, MAX_REORG_DEPTH};
use nightfall_crypto::WalletKeys;
use nightfall_node::runtime::REORG_PREFIX_COPIES;
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

fn start(dir: &std::path::Path, p2p: String, connect: Vec<String>) -> NodeHandle {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.to_path_buf(),
        p2p_listen: p2p,
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect,
        mine: false,
        miner: None,
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

fn chain_of(len: u64, seed: u8, t0: u64, offset: u64) -> Chain {
    let miner = WalletKeys::from_seed([seed; 32]).address();
    let mut c = Chain::new_fair(NetworkId::Devnet).unwrap();
    for i in 0..len {
        c.mine_block(&miner, vec![], t0 + i * 15 + offset).unwrap();
    }
    c
}

#[test]
#[ignore = "mines ~1,050 blocks; run by name in release"]
fn a_too_deep_fork_is_refused_without_copying_history() {
    let now = nightfall_storage::now_unix();
    let ours_len = MAX_REORG_DEPTH as u64 + 20;
    let theirs_len = ours_len + 10;
    let t0 = now - theirs_len * 15;
    let ours = chain_of(ours_len, 1, t0, 0);
    let theirs = chain_of(theirs_len, 2, t0, 3); // diverges right after genesis

    let base = std::env::temp_dir().join(format!("nf-reorg-copies-{}", std::process::id()));
    let port = free_port();
    let peer = start(&base.join("peer"), format!("127.0.0.1:{port}"), vec![]);
    {
        let s = peer.shared();
        let mut g = s.lock().unwrap();
        g.chain = theirs;
        g.persist().unwrap();
    }
    let archive = start(
        &base.join("archive"),
        "127.0.0.1:0".into(),
        vec![format!("127.0.0.1:{port}")],
    );
    {
        let s = archive.shared();
        let mut g = s.lock().unwrap();
        g.chain = ours;
        g.persist().unwrap();
    }
    let before = REORG_PREFIX_COPIES.load(Ordering::SeqCst);

    // Wait until the archive node has looked at the fork and refused it.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let s = archive.status_snapshot().unwrap();
        if s.stalled_on_fork && s.fork_rewind as usize > MAX_REORG_DEPTH {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "archive never evaluated the fork: tip {} stalled={} rewind={}",
            s.tip_height,
            s.stalled_on_fork,
            s.fork_rewind
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    // A few more sync rounds, as on the seed where it repeated every round.
    std::thread::sleep(Duration::from_secs(5));

    let s = archive.status_snapshot().unwrap();
    assert_eq!(
        s.tip_height,
        ours_len - 1,
        "a too-deep fork must not be adopted"
    );
    assert_eq!(
        REORG_PREFIX_COPIES.load(Ordering::SeqCst) - before,
        0,
        "history was copied for a fork that could never be adopted"
    );
    let _ = std::fs::remove_dir_all(&base);
}
