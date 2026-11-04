//! A node far behind its peer, on a short fork of its own, must rejoin.
//!
//! Issue #40 (1.0.5): a node that had mined three blocks of its own while it
//! was still catching up sat at the same height for 19 hours, rejecting every
//! peer's chain as "unexpected height". The fork was three blocks deep; the
//! node was 4,600 blocks behind. `common_ancestor` probed back from the
//! *peer's* tip, never reached any height the lagging node had, fell through
//! to `peer_tip - 1`, and the candidate built from that skipped every height in
//! between. 1.0.6 searches back from the lower of the two tips.
//!
//! Real nodes, real sockets, loopback only, Devnet, no public peers.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_types::NetworkId;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("{tag}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(dir: &TempDir, p2p: Option<u16>, connect: Vec<String>) -> NodeHandle {
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.0.clone(),
        // `None`: a node nobody can dial back — behind NAT or WSL2, as the
        // #40 node was. It only ever *pulls*; a peer cannot push it out of a
        // fork, so the pull path alone has to find the common ancestor.
        p2p_listen: match p2p {
            Some(port) => format!("127.0.0.1:{port}"),
            None => "127.0.0.1:0".into(),
        },
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
        assert!(Instant::now() < deadline, "node never finished loading");
        std::thread::sleep(Duration::from_millis(20));
    }
    node
}

fn install(node: &NodeHandle, chain: Chain) {
    let shared = node.shared();
    let mut state = shared.lock().unwrap();
    state.chain = chain;
    state.persist().unwrap();
}

// Mines 620 real blocks and runs two nodes: about three minutes on a laptop,
// far longer on a CI runner's debug build. Run it by name, in release:
//   cargo test --release -p nightfall-node --test far_behind_fork -- --ignored
#[test]
#[ignore = "two real nodes and 620 mined blocks; too slow for CI debug builds — run it by name"]
fn a_node_far_behind_on_a_short_fork_rejoins_the_network() {
    // Timestamps 15 s apart, ending now: steady difficulty, nothing in the
    // future, so the chain is one a real node would accept.
    const NETWORK_LEN: u64 = 620;
    const SHARED: usize = 40;
    let now = nightfall_storage::now_unix();
    let t0 = now - NETWORK_LEN * 15;
    let alice = WalletKeys::from_seed([3; 32]).address();
    let mallory = WalletKeys::from_seed([4; 32]).address();

    let mut network = Chain::new_fair(NetworkId::Devnet).unwrap();
    let mut mined = Vec::new();
    for i in 0..NETWORK_LEN {
        mined.push(network.mine_block(&alice, vec![], t0 + i * 15).unwrap());
    }

    // The lagging node shares the first SHARED blocks, then mines three of
    // its own — the shape of the #40 incident, scaled down.
    let mut lagging = Chain::new_fair(NetworkId::Devnet).unwrap();
    let applied = lagging
        .try_ingest_blocks(mined[..SHARED].to_vec(), now)
        .unwrap();
    assert_eq!(applied, SHARED);
    for i in 0..3u64 {
        let ts = t0 + (SHARED as u64 + i) * 15 + 7;
        lagging.mine_block(&mallory, vec![], ts).unwrap();
    }
    let lag_tip = lagging.tip_height().unwrap().0;
    let net_tip = network.tip_height().unwrap().0;
    assert!(
        net_tip - lag_tip > nightfall_consensus::MAX_REORG_DEPTH as u64,
        "the peer must be further ahead than the reorg window, as in #40"
    );
    assert_ne!(lagging.tip_hash(), network.tip_hash());

    let dir_a = TempDir::new("nf-far-behind-a");
    let dir_b = TempDir::new("nf-far-behind-b");
    let port_a = free_port();
    let a = start(&dir_a, Some(port_a), vec![]);
    install(&a, network);
    let b = start(&dir_b, None, vec![format!("127.0.0.1:{port_a}")]);
    install(&b, lagging);

    let target = a.status_snapshot().unwrap().tip.clone();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let s = b.status_snapshot().unwrap();
        if s.tip == target {
            assert!(!s.stalled_on_fork, "rejoined, but still flagged as stalled");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "lagging node never rejoined: tip {} at {}, stalled_on_fork={}, fork_rewind={}",
            s.tip,
            s.tip_height,
            s.stalled_on_fork,
            s.fork_rewind
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    let shared = b.shared();
    let state = shared.lock().unwrap();
    assert_eq!(state.chain.tip_height().unwrap().0, net_tip);
    state.chain.verify_supply().unwrap();
}
