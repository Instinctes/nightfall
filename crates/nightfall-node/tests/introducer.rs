//! The introducer answers, hands over the address book, and hangs up.
//!
//! This is the first test in the project that starts a real node and talks to
//! it over a socket. Every networking fault this network has had — a freeze, a
//! fork, a 2,000-block wall, a handshake checked in only one direction — was
//! found by real peers disagreeing while the suite passed, because nothing in
//! the suite ever opened a connection. This file is the beginning of the other
//! kind of test.

use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_types::{NetworkId, WIRE_VERSION};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// A throwaway data directory that removes itself. No dev-dependency for a
/// dozen lines; the workspace has none today and a test should not be the
/// reason to add one.
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

/// A port nobody else is on. Bind, read the number back, drop the listener.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn boot(introducer: bool) -> (NodeHandle, u16, TempDir) {
    let dir = TempDir::new("nf-introducer");
    let p2p = free_port();
    let cfg = NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.path().to_path_buf(),
        p2p_listen: format!("127.0.0.1:{p2p}"),
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect: vec![],
        mine: false,
        miner: None,
        proxy: None,
        mobile_listen: None,
        // Never let a test reach for the real network directory.
        peers_url: Some("off".into()),
        introducer,
        prune: false,
    };
    let node = NodeHandle::start(cfg).expect("node starts");
    // The listener spawns on its own thread; give it a moment to bind.
    std::thread::sleep(Duration::from_millis(400));
    (node, p2p, dir)
}

/// Say hello the way a real peer does and return every line the node sends
/// before it closes, or until we stop waiting.
fn handshake(port: u16, genesis: &str) -> (Vec<String>, bool) {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut w = stream.try_clone().expect("clone");
    let hello = serde_json::json!({
        "type": "hello",
        "wire": WIRE_VERSION,
        "network": "devnet",
        "genesis": genesis,
        "height": 0,
        "tip": "00".repeat(32),
        "agent": "introducer-test/1.0",
        "listen_port": 0,
    });
    writeln!(w, "{hello}").expect("write hello");
    w.flush().ok();

    let mut lines = Vec::new();
    let mut closed = false;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(l) if !l.trim().is_empty() => lines.push(l),
            Ok(_) => {}
            Err(_) => break, // read timeout — the node kept the socket open
        }
        if lines.len() >= 4 {
            break;
        }
    }
    // `lines()` ending without error means the peer closed on us.
    if lines.len() < 4 {
        closed = true;
    }
    (lines, closed)
}

fn kinds(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .collect()
}

#[test]
fn an_introducer_answers_hands_over_and_hangs_up() {
    let (node, port, _dir) = boot(true);
    let genesis = node.genesis_hex();

    let (lines, closed) = handshake(port, &genesis);
    let seen = kinds(&lines);

    assert!(
        seen.first().map(String::as_str) == Some("hello_ok"),
        "first message must be hello_ok, got {seen:?}"
    );
    assert!(
        seen.iter().any(|k| k == "peers"),
        "an introducer's whole job is the address book, got {seen:?}"
    );
    assert!(
        closed,
        "the introducer must hang up — it kept the socket open instead"
    );
}

#[test]
fn an_introduction_costs_no_session_seat() {
    // The reason this mode exists. A seed that keeps every introduction as a
    // live session is capped at MAX_PEERS; two of them are 256 slots for the
    // whole network. Introductions must leave nothing behind.
    let (node, port, _dir) = boot(true);
    let genesis = node.genesis_hex();

    for _ in 0..12 {
        let _ = handshake(port, &genesis);
    }
    std::thread::sleep(Duration::from_millis(300));

    let live = node.status_snapshot().expect("status").live_peers;
    assert_eq!(
        live, 0,
        "twelve introductions left {live} session(s) behind; they must leave none"
    );
}

#[test]
fn a_normal_node_keeps_the_session() {
    // The mirror image, so the flag is proven to do something rather than
    // nothing: without it the same handshake produces a live peer.
    let (node, port, _dir) = boot(false);
    let genesis = node.genesis_hex();

    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut w = stream.try_clone().expect("clone");
    let hello = serde_json::json!({
        "type": "hello", "wire": WIRE_VERSION, "network": "devnet",
        "genesis": genesis, "height": 0, "tip": "00".repeat(32),
        "agent": "introducer-test/1.0", "listen_port": 0,
    });
    writeln!(w, "{hello}").expect("write");
    w.flush().ok();
    std::thread::sleep(Duration::from_millis(600));

    assert_eq!(
        node.status_snapshot().expect("status").live_peers,
        1,
        "a normal node should hold the session open"
    );
    drop(stream);
}
