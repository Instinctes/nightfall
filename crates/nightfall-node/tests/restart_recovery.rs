//! Real multi-node integration harness.
//!
//! Goals:
//! - real TCP sockets
//! - isolated datadirs
//! - no public peer directory
//! - deterministic state polling instead of fixed sleeps
//! - useful diagnostics on timeout
//!
//! This deliberately uses the public NodeHandle API: if this test breaks
//! after an update, the node's externally useful integration surface changed.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle, SyncHold};
use nightfall_storage::ChainStore;
use nightfall_types::{NetworkId, TARGET_BLOCK_TIME_SECS};

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const START_TIMEOUT: Duration = Duration::from_secs(10);
const PEER_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(100);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));

        std::fs::create_dir_all(&path).expect("create test datadir");
        Self(path)
    }

    fn path(&self) -> &Path {
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
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}

struct TestNode {
    name: &'static str,
    node: NodeHandle,
    p2p_port: u16,
    _dir: TempDir,
}

impl TestNode {
    fn start_with<F>(name: &'static str, connect: Vec<String>, prepare: F) -> Self
    where
        F: FnOnce(&Path),
    {
        let dir = TempDir::new(&format!("nf-multinode-{name}"));
        prepare(dir.path());
        let p2p_port = free_port();
        let rpc_port = free_port();

        let cfg = NodeConfig {
            network: NetworkId::Devnet,
            datadir: dir.path().to_path_buf(),
            p2p_listen: format!("127.0.0.1:{p2p_port}"),
            rpc_listen: format!("127.0.0.1:{rpc_port}"),
            connect,
            mine: false,
            miner: None,
            proxy: None,
            mobile_listen: None,

            // Integration tests must never depend on Internet/network state.
            peers_url: Some("off".into()),

            introducer: false,
            prune: false,
        };

        let node = NodeHandle::start(cfg)
            .unwrap_or_else(|e| panic!("{name}: NodeHandle::start failed: {e:#}"));

        let test_node = Self {
            name,
            node,
            p2p_port,
            _dir: dir,
        };

        test_node.wait_ready();
        test_node
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.p2p_port)
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + START_TIMEOUT;

        loop {
            if TcpListener::bind(("127.0.0.1", self.p2p_port)).is_err() {
                return;
            }

            if Instant::now() >= deadline {
                panic!(
                    "{}: P2P listener did not bind within {:?}",
                    self.name, START_TIMEOUT
                );
            }

            std::thread::sleep(POLL);
        }
    }

    fn snapshot(&self) -> nightfall_node::StatusSnap {
        self.node
            .status_snapshot()
            .unwrap_or_else(|e| panic!("{}: status_snapshot failed: {e:#}", self.name))
    }

    fn dump(&self) -> String {
        let s = self.snapshot();

        format!(
            concat!(
                "{}: ",
                "height={} ",
                "tip={} ",
                "live_peers={} ",
                "known_peers={} ",
                "best_peer_height={} ",
                "behind={} ",
                "loading={} ",
                "stalled_on_fork={} ",
                "reorg_in_flight={} ",
                "last_dial_error={:?}"
            ),
            self.name,
            s.tip_height,
            s.tip,
            s.live_peers,
            s.peers,
            s.best_peer_height,
            s.blocks_behind,
            s.loading,
            s.stalled_on_fork,
            s.reorg_in_flight,
            s.last_dial_error,
        )
    }
}

fn wait_until<F>(description: &str, timeout: Duration, nodes: &[&TestNode], mut ok: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;

    loop {
        if ok() {
            return;
        }

        if Instant::now() >= deadline {
            let diagnostics = nodes
                .iter()
                .map(|n| n.dump())
                .collect::<Vec<_>>()
                .join("\n");

            panic!(
                "timeout waiting for: {description}\n\
                 timeout: {timeout:?}\n\
                 node diagnostics:\n{diagnostics}"
            );
        }

        std::thread::sleep(POLL);
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

fn build_competing_devnet_chains() -> (Chain, Chain) {
    let now = unix_now();

    // Keep every synthetic timestamp safely in the past.
    let base = now.saturating_sub(5 * 60);

    // Deterministic test-only keys. These are not real wallet secrets.
    let shared_miner = WalletKeys::from_seed([0x11; 32]).address();
    let short_miner = WalletKeys::from_seed([0x22; 32]).address();
    let heavy_miner = WalletKeys::from_seed([0x33; 32]).address();

    // Start with a genuine shared prefix.
    let mut shared = Chain::new_fair(NetworkId::Devnet).expect("create shared devnet chain");

    for i in 0..2u64 {
        shared
            .mine_block(&shared_miner, vec![], base + i * TARGET_BLOCK_TIME_SECS)
            .expect("mine shared-prefix block");
    }

    // A: shared prefix + one competing block.
    let mut short = Chain::new_fair(NetworkId::Devnet).expect("create short branch");

    short
        .try_ingest_blocks(shared.blocks.clone(), now)
        .expect("replay shared prefix into short branch");

    short
        .mine_block(&short_miner, vec![], base + 2 * TARGET_BLOCK_TIME_SECS)
        .expect("mine short-branch block");

    // B: same prefix + three competing blocks.
    let mut heavy = Chain::new_fair(NetworkId::Devnet).expect("create heavy branch");

    heavy
        .try_ingest_blocks(shared.blocks.clone(), now)
        .expect("replay shared prefix into heavy branch");

    for i in 0..3u64 {
        heavy
            .mine_block(
                &heavy_miner,
                vec![],
                base + (2 + i) * TARGET_BLOCK_TIME_SECS,
            )
            .expect("mine heavy-branch block");
    }

    assert_ne!(
        short.tip_hash(),
        heavy.tip_hash(),
        "test setup must create competing tips"
    );

    assert!(
        heavy.total_work > short.total_work,
        "test setup requires heavy branch work {} > short branch work {}",
        heavy.total_work,
        short.total_work
    );

    short
        .verify_supply()
        .expect("short branch must satisfy supply invariant");

    heavy
        .verify_supply()
        .expect("heavy branch must satisfy supply invariant");

    (short, heavy)
}

#[test]
fn real_nodes_reorg_to_heavier_devnet_branch() {
    let (short, heavy) = build_competing_devnet_chains();

    let short_tip = short.tip_hash().to_hex();
    let expected_tip = heavy.tip_hash().to_hex();
    let expected_work = heavy.total_work;
    let expected_root = heavy.ledger.utxo_root().to_hex();
    let expected_height = heavy.tip_height().map(|h| h.0).unwrap_or(0);

    // Persist both branches exactly as normal node storage.
    // Neither node has any configured peer at startup.
    let a = TestNode::start_with("A", vec![], |dir| {
        ChainStore::new(dir)
            .save(&short)
            .expect("persist A short branch");
    });

    let b = TestNode::start_with("B", vec![], |dir| {
        ChainStore::new(dir)
            .save(&heavy)
            .expect("persist B heavy branch");
    });

    // Prove the fork exists before any networking can resolve it.
    wait_until(
        "prepared competing branches load",
        START_TIMEOUT,
        &[&a, &b],
        || {
            let sa = a.snapshot();
            let sb = b.snapshot();

            !sa.loading
                && !sb.loading
                && sa.tip == short_tip
                && sb.tip == expected_tip
                && sa.tip != sb.tip
                && sb.total_work > sa.total_work
        },
    );

    let before = a.snapshot();

    assert_eq!(before.tip, short_tip, "A did not start on short branch");
    assert_ne!(
        before.tip, expected_tip,
        "A already had B's tip before connecting"
    );

    // This is the only connection introduced by the test.
    // Devnet has no compiled seeds and the harness disables peers_url.
    a.node.add_peer(&b.addr()).expect("add local B peer to A");

    wait_until("real A<->B P2P session", PEER_TIMEOUT, &[&a, &b], || {
        let sa = a.snapshot();
        let sb = b.snapshot();
        sa.live_peers >= 1 && sb.live_peers >= 1
    });

    // The important assertion: A must detect the competing branch,
    // fetch it through P2P, validate it, and adopt it by total work.
    wait_until(
        "A reorgs onto B heavier branch",
        Duration::from_secs(60),
        &[&a, &b],
        || {
            let sa = a.snapshot();
            let sb = b.snapshot();

            !sa.loading
                && !sb.loading
                && sa.tip == expected_tip
                && sb.tip == expected_tip
                && sa.tip_height == expected_height
                && sb.tip_height == expected_height
                && sa.total_work == expected_work
                && sb.total_work == expected_work
                && sa.utxo_root == expected_root
                && sb.utxo_root == expected_root
                && sa.supply_ok
                && sb.supply_ok
                && !sa.stalled_on_fork
                && !sb.stalled_on_fork
                && !sa.reorg_in_flight
                && !sb.reorg_in_flight
                && matches!(sa.sync_hold, SyncHold::Synced)
                && matches!(sb.sync_hold, SyncHold::Synced)
        },
    );

    let sa = a.snapshot();
    let sb = b.snapshot();

    // A really changed chain; this is not merely a connectivity test.
    assert_ne!(
        before.tip, sa.tip,
        "A never left its original competing branch"
    );

    // Consensus identity.
    assert_eq!(sa.tip, sb.tip, "nodes disagree on final tip");
    assert_eq!(
        sa.tip_height, sb.tip_height,
        "nodes disagree on final height"
    );
    assert_eq!(
        sa.total_work, sb.total_work,
        "nodes disagree on cumulative work"
    );

    // Ledger identity.
    assert_eq!(
        sa.utxo_root, sb.utxo_root,
        "nodes disagree on final UTXO root"
    );
    assert_eq!(sa.utxos, sb.utxos, "nodes disagree on UTXO count");
    assert_eq!(sa.kernels, sb.kernels, "nodes disagree on kernel count");
    assert_eq!(sa.minted, sb.minted, "nodes disagree on minted supply");
    assert_eq!(
        sa.burned_fees, sb.burned_fees,
        "nodes disagree on burned fees"
    );

    for n in [&a, &b] {
        let s = n.snapshot();

        assert!(
            s.supply_ok,
            "{} failed supply invariant:\n{}",
            n.name,
            n.dump()
        );

        assert!(
            !s.stalled_on_fork,
            "{} remained stalled on fork:\n{}",
            n.name,
            n.dump()
        );

        assert!(
            !s.reorg_in_flight,
            "{} still reports an active reorg:\n{}",
            n.name,
            n.dump()
        );

        assert!(
            matches!(s.sync_hold, SyncHold::Synced),
            "{} did not return to SyncHold::Synced:\n{}",
            n.name,
            n.dump()
        );
    }

    // Re-open A's store independently after the runtime reorg.
    // This verifies that the adopted canonical branch was written to the
    // persisted store, rather than existing only in the node's in-memory state.
    let persisted_a = ChainStore::new(a._dir.path())
        .load_or_new(NetworkId::Devnet)
        .expect("reload A chain from disk after P2P reorg");

    assert_eq!(
        persisted_a.tip_hash().to_hex(),
        expected_tip,
        "A persisted the wrong canonical tip after reorg"
    );
    assert_eq!(
        persisted_a.tip_height().map(|h| h.0).unwrap_or(0),
        expected_height,
        "A persisted the wrong canonical height after reorg"
    );
    assert_eq!(
        persisted_a.total_work, expected_work,
        "A persisted the wrong total work after reorg"
    );
    assert_eq!(
        persisted_a.ledger.utxo_root().to_hex(),
        expected_root,
        "A persisted the wrong UTXO root after reorg"
    );

    persisted_a
        .verify_supply()
        .expect("A persisted chain violates supply invariant after reorg");

    eprintln!("REAL P2P REORG OK");
    eprintln!("before A tip={}", before.tip);
    eprintln!("final  A {}", a.dump());
    eprintln!("final  B {}", b.dump());
}

#[test]
fn reorg_survives_clean_shutdown_restart_and_continues_syncing() {
    let (short, heavy) = build_competing_devnet_chains();

    let short_tip = short.tip_hash().to_hex();
    let heavy_tip = heavy.tip_hash().to_hex();

    let a = TestNode::start_with("restart-A", vec![], |dir| {
        ChainStore::new(dir)
            .save(&short)
            .expect("persist A short branch");
    });

    let b = TestNode::start_with("restart-B", vec![], |dir| {
        ChainStore::new(dir)
            .save(&heavy)
            .expect("persist B heavy branch");
    });

    wait_until(
        "prepared restart test branches load",
        START_TIMEOUT,
        &[&a, &b],
        || {
            let sa = a.snapshot();
            let sb = b.snapshot();

            !sa.loading
                && !sb.loading
                && sa.tip == short_tip
                && sb.tip == heavy_tip
                && sa.tip != sb.tip
                && sb.total_work > sa.total_work
        },
    );

    a.node.add_peer(&b.addr()).expect("connect A to local B");

    wait_until(
        "restart-A adopts restart-B heavier branch",
        Duration::from_secs(60),
        &[&a, &b],
        || {
            let sa = a.snapshot();
            let sb = b.snapshot();

            !sa.loading
                && !sb.loading
                && sa.tip == sb.tip
                && sa.tip == heavy_tip
                && sa.tip_height == sb.tip_height
                && sa.total_work == sb.total_work
                && sa.utxo_root == sb.utxo_root
                && sa.utxos == sb.utxos
                && sa.kernels == sb.kernels
                && sa.minted == sb.minted
                && sa.burned_fees == sb.burned_fees
                && sa.supply_ok
                && sb.supply_ok
                && !sa.reorg_in_flight
                && matches!(sa.sync_hold, SyncHold::Synced)
        },
    );

    let before_restart = a.snapshot();

    assert!(
        before_restart.supply_ok,
        "A supply invariant failed before restart:\n{}",
        a.dump()
    );

    let datadir = a._dir.path().to_path_buf();

    /*
     * Consume the old runtime completely.  The TempDir must remain alive,
     * therefore move only NodeHandle out of TestNode by replacing it with a
     * temporary handle is undesirable.  Destructure the wrapper instead.
     */
    let TestNode {
        name: _,
        node: old_node,
        p2p_port: _,
        _dir: dir_guard,
    } = a;

    let shutdown_started = Instant::now();

    old_node.shutdown().expect("clean shutdown after reorg");

    let shutdown_elapsed = shutdown_started.elapsed();

    assert!(
        shutdown_elapsed < Duration::from_secs(20),
        "clean shutdown took unexpectedly long: {shutdown_elapsed:?}"
    );

    /*
     * Fresh runtime instance, same persisted datadir.
     * New loopback ports avoid depending on immediate port reuse.
     */
    let restart_p2p = free_port();
    let restart_rpc = free_port();

    let restarted_cfg = NodeConfig {
        network: NetworkId::Devnet,
        datadir: datadir.clone(),
        p2p_listen: format!("127.0.0.1:{restart_p2p}"),
        rpc_listen: format!("127.0.0.1:{restart_rpc}"),
        connect: vec![],
        mine: false,
        miner: None,
        proxy: None,
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    };

    let restarted_node =
        NodeHandle::start(restarted_cfg).expect("restart NodeHandle on same datadir");

    let restarted = TestNode {
        name: "restart-A2",
        node: restarted_node,
        p2p_port: restart_p2p,
        _dir: dir_guard,
    };

    restarted.wait_ready();

    wait_until(
        "restarted A finishes loading persisted reorg state",
        START_TIMEOUT,
        &[&restarted],
        || !restarted.snapshot().loading,
    );

    let after_restart = restarted.snapshot();

    assert_eq!(
        after_restart.tip, before_restart.tip,
        "canonical tip changed across clean restart"
    );
    assert_eq!(
        after_restart.tip_height, before_restart.tip_height,
        "height changed across clean restart"
    );
    assert_eq!(
        after_restart.blocks, before_restart.blocks,
        "block count changed across clean restart"
    );
    assert_eq!(
        after_restart.total_work, before_restart.total_work,
        "total work changed across clean restart"
    );
    assert_eq!(
        after_restart.utxo_root, before_restart.utxo_root,
        "UTXO root changed across clean restart"
    );
    assert_eq!(
        after_restart.utxos, before_restart.utxos,
        "UTXO count changed across clean restart"
    );
    assert_eq!(
        after_restart.kernels, before_restart.kernels,
        "kernel count changed across clean restart"
    );
    assert_eq!(
        after_restart.minted, before_restart.minted,
        "minted supply changed across clean restart"
    );
    assert_eq!(
        after_restart.burned_fees, before_restart.burned_fees,
        "burned fees changed across clean restart"
    );
    assert!(
        after_restart.supply_ok,
        "supply invariant failed after clean restart:\n{}",
        restarted.dump()
    );

    /*
     * Continue through the real P2P path after restart.
     * Mine exactly one additional valid Devnet block into B's persisted/live
     * chain via its shared state, persist it, then announce it.
     */
    let continuation_miner = WalletKeys::from_seed([0x44; 32]).address();

    let next_block = {
        let shared = b.node.shared();
        let mut g = shared.lock().expect("lock B state for continuation block");

        let timestamp = unix_now();

        let block = g
            .chain
            .mine_block(&continuation_miner, vec![], timestamp)
            .expect("mine continuation block on B");

        g.persist().expect("persist B continuation block");

        g.announce_block(block.clone());

        block
    };

    let continuation_tip = next_block.hash().to_hex();
    let continuation_height = next_block.header.height.0;

    restarted
        .node
        .add_peer(&b.addr())
        .expect("reconnect restarted A to B");

    wait_until(
        "restarted A syncs one additional valid block",
        Duration::from_secs(60),
        &[&restarted, &b],
        || {
            let sa = restarted.snapshot();
            let sb = b.snapshot();

            !sa.loading
                && sa.tip == continuation_tip
                && sb.tip == continuation_tip
                && sa.tip_height == continuation_height
                && sb.tip_height == continuation_height
                && sa.total_work == sb.total_work
                && sa.utxo_root == sb.utxo_root
                && sa.utxos == sb.utxos
                && sa.kernels == sb.kernels
                && sa.minted == sb.minted
                && sa.burned_fees == sb.burned_fees
                && sa.supply_ok
                && sb.supply_ok
        },
    );

    restarted.node.shutdown().expect("shutdown restarted A");

    b.node.shutdown().expect("shutdown B");
}
