//! A running node continues from a chain archive: validated like any foreign
//! chain file, adopted only when it is ahead, and its own afterwards.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "nf-archive-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A devnet chain of `n` blocks, written to `dir` the way a node writes it.
fn chain_on_disk(dir: &Path, n: u64) -> Chain {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    let miner = WalletKeys::generate().address();
    let start = now() - 3_600;
    for i in 0..n {
        chain.mine_block(&miner, vec![], start + i * 15).unwrap();
    }
    ChainStore::new(dir).save(&chain).unwrap();
    chain
}

/// An import directory holding only the chain file of `source`, which is all
/// a published archive contains.
fn import_dir_from(source: &Path, tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    std::fs::copy(
        ChainStore::new(source).blocks_path(),
        dir.join(nightfall_node::bootstrap::ARCHIVE_FILE),
    )
    .unwrap();
    dir
}

fn start(datadir: &Path) -> NodeHandle {
    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: datadir.to_path_buf(),
        p2p_listen: format!("127.0.0.1:{}", free_port()),
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect: vec![],
        mine: false,
        miner: None,
        proxy: None,
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .expect("node starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while node.status_snapshot().map(|s| s.loading).unwrap_or(true) {
        assert!(Instant::now() < deadline, "node never finished loading");
        std::thread::sleep(Duration::from_millis(50));
    }
    node
}

fn tip(node: &NodeHandle) -> (u64, String) {
    let g = node.shared();
    let g = g.lock().unwrap();
    (g.chain.block_count(), g.chain.tip_hash().to_hex())
}

#[test]
fn a_fresh_node_continues_from_an_archive_and_owns_it_afterwards() {
    let source = temp_dir("source");
    let archive = chain_on_disk(&source, 14);
    let import = import_dir_from(&source, "import");

    let datadir = temp_dir("node");
    let node = start(&datadir);
    assert_eq!(tip(&node).0, 0);

    let mut seen = (0, 0);
    let blocks = node
        .adopt_chain_archive(&import, |done, total| seen = (done, total))
        .unwrap();
    assert_eq!(blocks, 14);
    assert_eq!(seen, (14, 14), "progress counts the archive's own blocks");
    assert_eq!(tip(&node), (14, archive.tip_hash().to_hex()));
    node.shutdown().unwrap();
    drop(node);

    // The node wrote the chain to its own datadir with its own validation
    // record: the next start trusts it and does not validate it again.
    assert!(ChainStore::new(&datadir).is_own_file_trusted());
    let again = start(&datadir);
    assert_eq!(tip(&again), (14, archive.tip_hash().to_hex()));
    again.shutdown().unwrap();

    for dir in [source, import, datadir] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn an_archive_that_is_not_ahead_changes_nothing() {
    let long = temp_dir("long");
    let ours = chain_on_disk(&long, 9);
    let short = temp_dir("short");
    chain_on_disk(&short, 4);

    // The node starts on the longer chain, as its own file.
    let node = start(&long);
    assert_eq!(tip(&node), (9, ours.tip_hash().to_hex()));

    let import = import_dir_from(&short, "import-short");
    let err = node.adopt_chain_archive(&import, |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("not ahead"), "{err}");
    assert_eq!(tip(&node), (9, ours.tip_hash().to_hex()));

    // The same chain again is not ahead either.
    let same = import_dir_from(&long, "import-same");
    let err = node.adopt_chain_archive(&same, |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("not ahead"), "{err}");
    assert_eq!(tip(&node), (9, ours.tip_hash().to_hex()));
    node.shutdown().unwrap();

    for dir in [long, short, import, same] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn an_archive_with_one_invalid_block_is_refused_whole() {
    // Build a chain, then rewrite its file with one block's nonce changed so
    // that its proof of work no longer holds. Everything else about the file
    // is intact, including that block's place in the chain file.
    let source = temp_dir("bad-source");
    let mut chain = chain_on_disk(&source, 8);
    let params = NetworkId::Devnet.pow_params();
    let victim = &mut chain.blocks[6];
    let original = victim.header.nonce;
    victim.header.nonce = (0..100_000u64)
        .find(|n| {
            let mut probe = victim.clone();
            probe.header.nonce = *n;
            *n != original && !probe.pow_is_valid(params)
        })
        .expect("an invalid nonce exists");
    let import = temp_dir("bad-import");
    let mut file =
        std::fs::File::create(import.join(nightfall_node::bootstrap::ARCHIVE_FILE)).unwrap();
    for block in &chain.blocks {
        nightfall_storage::codec::write_block(
            &mut file,
            block,
            nightfall_storage::codec::Format::Binary,
        )
        .unwrap();
    }
    drop(file);

    let datadir = temp_dir("bad-node");
    let node = start(&datadir);
    let err = node.adopt_chain_archive(&import, |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("stored block 6"), "{err}");
    assert_eq!(tip(&node).0, 0, "nothing from a refused archive is kept");
    node.shutdown().unwrap();

    for dir in [source, import, datadir] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
