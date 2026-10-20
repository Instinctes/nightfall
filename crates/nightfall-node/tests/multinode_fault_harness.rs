//! Real-process multi-node fault harness.
//!
//! Nothing below mocks consensus, storage, RPC or P2P. Every participant is a
//! real `nightfalld` process with an independent datadir.
//!
//! Nodes are deliberately bound to different loopback IPs while all controlled
//! P2P traffic crosses a proxy on 127.0.0.1. This matters because Nightfall
//! learns a dial-back address by combining an observed source IP with the
//! peer's advertised listen port. If both nodes and the proxy shared
//! 127.0.0.1, a node could learn the other's real listener and silently bypass
//! the fault injector.

use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const START_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CONVERGE_TIMEOUT: Duration = Duration::from_secs(60);
const MINE_TIMEOUT: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_millis(200);

fn nightfalld() -> &'static str {
    env!("CARGO_BIN_EXE_nightfalld")
}

fn unique_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "nightfall-multinode-{}-{nanos}",
        std::process::id()
    ))
}

fn free_addr(ip: Ipv4Addr) -> SocketAddr {
    TcpListener::bind(SocketAddr::new(IpAddr::V4(ip), 0))
        .unwrap_or_else(|e| panic!("bind ephemeral address on {ip}: {e}"))
        .local_addr()
        .expect("ephemeral local address")
}

#[derive(Clone, Debug)]
struct NodeSpec {
    name: &'static str,
    datadir: PathBuf,
    p2p: SocketAddr,
    rpc: SocketAddr,
}

impl NodeSpec {
    fn new(root: &Path, name: &'static str, ip: Ipv4Addr) -> Self {
        Self {
            name,
            datadir: root.join(name),
            p2p: free_addr(ip),
            rpc: free_addr(ip),
        }
    }

    fn init(&self) {
        fs::create_dir_all(&self.datadir).expect("create node datadir");

        let output = Command::new(nightfalld())
            .args(["--network", "devnet", "--datadir"])
            .arg(&self.datadir)
            .arg("init")
            .output()
            .unwrap_or_else(|e| panic!("{}: launch init: {e}", self.name));

        assert!(
            output.status.success(),
            "{}: init failed\nstdout:\n{}\nstderr:\n{}",
            self.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn mine_blocks(&self, blocks: u32) {
        let output = Command::new(nightfalld())
            .args(["--network", "devnet", "--datadir"])
            .arg(&self.datadir)
            .arg("mine")
            .arg("--blocks")
            .arg(blocks.to_string())
            .output()
            .unwrap_or_else(|e| panic!("{}: launch offline miner: {e}", self.name));

        assert!(
            output.status.success(),
            "{}: offline mining failed\nstdout:\n{}\nstderr:\n{}",
            self.name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

struct Node {
    spec: NodeSpec,
    child: Child,
    log_path: PathBuf,
}

impl Node {
    fn spawn(spec: NodeSpec, mine: bool, connect: &[SocketAddr]) -> Self {
        let log_path = spec.datadir.join("harness.log");
        let log = File::create(&log_path).expect("create node log");
        let err = log.try_clone().expect("clone node log");

        let mut cmd = Command::new(nightfalld());

        cmd.args(["--network", "devnet", "--datadir"])
            .arg(&spec.datadir)
            .arg("run")
            .arg("--listen")
            .arg(spec.p2p.to_string())
            .arg("--rpc-listen")
            .arg(spec.rpc.to_string())
            // Local harness traffic must never escape through Tor and the
            // public peer directory must not inject uncontrolled peers.
            .arg("--proxy")
            .arg("off")
            .env("NIGHTFALL_PEERS_URL", "off");

        if mine {
            cmd.arg("--mine");
        }

        for peer in connect {
            cmd.arg("--connect").arg(peer.to_string());
        }

        cmd.env("RUST_LOG", "info")
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err));

        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("{}: spawn nightfalld: {e}", spec.name));

        let node = Self {
            spec,
            child,
            log_path,
        };

        node.wait_rpc(START_TIMEOUT);
        node
    }

    fn wait_rpc(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut last_error = String::new();

        while Instant::now() < deadline {
            match self.status() {
                Ok(_) => return,
                Err(e) => last_error = e,
            }

            thread::sleep(POLL);
        }

        panic!(
            "{}: RPC {} did not become ready within {:?}: {}\n{}",
            self.spec.name,
            self.spec.rpc,
            timeout,
            last_error,
            self.dump_log()
        );
    }

    fn status(&self) -> Result<NodeStatus, String> {
        let value = rpc_call(self.spec.rpc, "status", json!({}))?;
        NodeStatus::from_rpc(&value)
    }

    fn dump_log(&self) -> String {
        read_log(&self.log_path)
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Debug)]
struct NodeStatus {
    blocks: u64,
    tip: String,
    peers: u64,
    total_work: u128,
}

impl NodeStatus {
    fn from_rpc(root: &Value) -> Result<Self, String> {
        let blocks = find_u64(root, &["blocks", "height", "block_height"])
            .ok_or_else(|| format!("status response has no chain height/count: {root}"))?;

        let tip = find_string(root, &["tip", "tip_hash", "best_hash"])
            .ok_or_else(|| format!("status response has no tip hash: {root}"))?;

        let peers = find_u64(root, &["peers"])
            .ok_or_else(|| format!("status response has no peer count: {root}"))?;

        let total_work = find_u128(root, &["total_work"])
            .ok_or_else(|| format!("status response has no cumulative work: {root}"))?;

        Ok(Self {
            blocks,
            tip,
            peers,
            total_work,
        })
    }

    fn same_chain(&self, other: &Self) -> bool {
        self.blocks == other.blocks && self.tip == other.tip && self.total_work == other.total_work
    }
}

fn find_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(v) = map.get(*key) {
                    if let Some(n) = v.as_u64() {
                        return Some(n);
                    }

                    if let Some(s) = v.as_str() {
                        if let Ok(n) = s.parse::<u64>() {
                            return Some(n);
                        }
                    }
                }
            }

            map.values().find_map(|v| find_u64(v, keys))
        }
        Value::Array(items) => items.iter().find_map(|v| find_u64(v, keys)),
        _ => None,
    }
}

fn find_u128(value: &Value, keys: &[&str]) -> Option<u128> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(v) = map.get(*key) {
                    if let Some(n) = v.as_u64() {
                        return Some(n as u128);
                    }

                    if let Some(text) = v.as_str() {
                        if let Ok(n) = text.parse::<u128>() {
                            return Some(n);
                        }
                    }
                }
            }

            map.values().find_map(|v| find_u128(v, keys))
        }
        Value::Array(items) => items.iter().find_map(|v| find_u128(v, keys)),
        _ => None,
    }
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(s) = map.get(*key).and_then(Value::as_str) {
                    if !s.is_empty() {
                        return Some(s.to_owned());
                    }
                }
            }

            map.values().find_map(|v| find_string(v, keys))
        }
        Value::Array(items) => items.iter().find_map(|v| find_string(v, keys)),
        _ => None,
    }
}

fn rpc_call(addr: SocketAddr, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .map_err(|e| format!("connect {addr}: {e}"))?;

    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("set read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("set write timeout: {e}"))?;

    let request = json!({
        "method": method,
        "params": params,
        "id": 1
    });

    let mut wire =
        serde_json::to_vec(&request).map_err(|e| format!("serialize RPC request: {e}"))?;
    wire.push(b'\n');

    stream
        .write_all(&wire)
        .map_err(|e| format!("write RPC request: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("flush RPC request: {e}"))?;

    // Nightfall consumes one RPC request through EOF. Half-close only the
    // request side so the response remains readable.
    stream
        .shutdown(Shutdown::Write)
        .map_err(|e| format!("finish RPC request: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read RPC response: {e}"))?;

    serde_json::from_slice(&response).map_err(|e| {
        format!(
            "decode RPC response: {e}; raw={}",
            String::from_utf8_lossy(&response)
        )
    })
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| format!("<cannot read {}: {e}>", path.display()))
}

fn wait_until(
    description: &str,
    timeout: Duration,
    mut predicate: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;

    while Instant::now() < deadline {
        match predicate() {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => last_error = Some(e),
        }

        thread::sleep(POLL);
    }

    match last_error {
        Some(e) => Err(format!(
            "timeout waiting for {description}; last error: {e}"
        )),
        None => Err(format!("timeout waiting for {description}")),
    }
}

// ---------------------------------------------------------------- fault proxy

struct FaultProxy {
    addr: SocketAddr,
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    active: Arc<Mutex<Vec<TcpStream>>>,
    worker: Option<JoinHandle<()>>,
}

impl FaultProxy {
    fn spawn(target: SocketAddr, forbidden_ports: &[u16]) -> Self {
        let listener = loop {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fault proxy listener");
            let port = listener.local_addr().expect("proxy local addr").port();

            if !forbidden_ports.contains(&port) {
                break listener;
            }
        };

        listener
            .set_nonblocking(true)
            .expect("set fault proxy nonblocking");

        let addr = listener.local_addr().expect("fault proxy address");
        let enabled = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(Vec::new()));

        let worker_enabled = Arc::clone(&enabled);
        let worker_stop = Arc::clone(&stop);
        let worker_active = Arc::clone(&active);

        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((downstream, _)) => {
                        if !worker_enabled.load(Ordering::SeqCst) {
                            let _ = downstream.shutdown(Shutdown::Both);
                            continue;
                        }

                        let upstream =
                            match TcpStream::connect_timeout(&target, Duration::from_secs(2)) {
                                Ok(stream) => stream,
                                Err(_) => {
                                    let _ = downstream.shutdown(Shutdown::Both);
                                    continue;
                                }
                            };

                        let _ = downstream.set_nodelay(true);
                        let _ = upstream.set_nodelay(true);

                        // Partition may have raced with the upstream connect.
                        if !worker_enabled.load(Ordering::SeqCst) {
                            let _ = downstream.shutdown(Shutdown::Both);
                            let _ = upstream.shutdown(Shutdown::Both);
                            continue;
                        }

                        if let Ok(mut sockets) = worker_active.lock() {
                            if let Ok(s) = downstream.try_clone() {
                                sockets.push(s);
                            }
                            if let Ok(s) = upstream.try_clone() {
                                sockets.push(s);
                            }
                        }

                        thread::spawn(move || relay_bidirectional(downstream, upstream));
                    }

                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }

                    Err(_) => {
                        if !worker_stop.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            }
        });

        Self {
            addr,
            enabled,
            stop,
            active,
            worker: Some(worker),
        }
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn partition(&self) {
        // Refuse new connections before killing existing ones.
        self.enabled.store(false, Ordering::SeqCst);
        shutdown_all(&self.active);
    }

    fn heal(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        shutdown_all(&self.active);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn shutdown_all(active: &Arc<Mutex<Vec<TcpStream>>>) {
    if let Ok(mut sockets) = active.lock() {
        for socket in sockets.drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

fn relay_bidirectional(downstream: TcpStream, upstream: TcpStream) {
    let mut down_read = match downstream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut up_write = match upstream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };

    let forward = thread::spawn(move || {
        let _ = io::copy(&mut down_read, &mut up_write);
        let _ = up_write.shutdown(Shutdown::Write);
    });

    let mut up_read = upstream;
    let mut down_write = downstream;

    let _ = io::copy(&mut up_read, &mut down_write);
    let _ = down_write.shutdown(Shutdown::Write);

    let _ = forward.join();
}

// ---------------------------------------------------------------- scenarios

#[test]
fn partition_cuts_real_session_and_heal_resynchronizes() {
    let root = unique_root();
    fs::create_dir_all(&root).expect("create harness root");

    // Separate loopback IPs are intentional. The proxy remains 127.0.0.1,
    // preventing Nightfall's observed-IP + advertised-port peer learning from
    // discovering a usable direct path around the fault injector.
    let a_spec = NodeSpec::new(&root, "node-a", Ipv4Addr::new(127, 0, 0, 2));
    let b_spec = NodeSpec::new(&root, "node-b", Ipv4Addr::new(127, 0, 0, 3));

    a_spec.init();
    b_spec.init();

    let link = FaultProxy::spawn(
        a_spec.p2p,
        &[
            a_spec.p2p.port(),
            a_spec.rpc.port(),
            b_spec.p2p.port(),
            b_spec.rpc.port(),
        ],
    );

    let a = Node::spawn(a_spec.clone(), false, &[]);
    let b = Node::spawn(b_spec.clone(), false, &[link.addr()]);

    wait_until(
        "A and B to establish a proxied live session",
        CONNECT_TIMEOUT,
        || {
            let sa = a.status()?;
            let sb = b.status()?;
            Ok(sa.peers >= 1 && sb.peers >= 1)
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n--- node A ---\n{}\n--- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    let before_a = a.status().expect("A status before partition");
    let before_b = b.status().expect("B status before partition");

    assert!(
        before_a.same_chain(&before_b),
        "nodes were connected but did not start on the same chain: A={before_a:?} B={before_b:?}"
    );

    // The actual fault boundary: kill both directions of every live TCP
    // session and refuse all reconnects.
    link.partition();

    wait_until(
        "partition to remove all A/B sessions",
        CONNECT_TIMEOUT,
        || {
            let sa = a.status()?;
            let sb = b.status()?;
            Ok(sa.peers == 0 && sb.peers == 0)
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n--- node A ---\n{}\n--- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    // A cannot be mined offline while its daemon owns the datadir.
    drop(a);

    // Mine the first devnet block while the network is partitioned. Using the
    // one-shot miner here gives us deterministic topology: only A advances.
    a_spec.mine_blocks(1);

    let a = Node::spawn(a_spec.clone(), false, &[]);

    wait_until("A to load its isolated block", MINE_TIMEOUT, || {
        Ok(a.status()?.blocks >= 1)
    })
    .unwrap_or_else(|e| panic!("{e}\n--- node A ---\n{}", a.dump_log()));

    // Give the reconnect supervisor several opportunities. B must remain at
    // the old chain while the proxy is partitioned. If it advances here, the
    // fault boundary has leaked and this harness is invalid.
    thread::sleep(Duration::from_secs(5));

    let isolated_a = a.status().expect("A isolated status");
    let isolated_b = b.status().expect("B isolated status");

    assert!(
        isolated_a.blocks > isolated_b.blocks,
        "partition leaked: B learned A's new chain while the controlled link was down; \
         A={isolated_a:?} B={isolated_b:?}"
    );

    assert_eq!(
        isolated_b.tip, before_b.tip,
        "B changed tip while every controlled path to A was partitioned"
    );

    // Re-enable accepts. B's normal stay-connected supervisor must reconnect
    // without any test-only hook, then use Nightfall's real sync path.
    link.heal();

    wait_until("healed nodes to converge", CONVERGE_TIMEOUT, || {
        let sa = a.status()?;
        let sb = b.status()?;

        Ok(sa.same_chain(&sb) && sa.tip == isolated_a.tip)
    })
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n--- node A ---\n{}\n--- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    let final_a = a.status().expect("final A status");
    let final_b = b.status().expect("final B status");

    assert!(
        final_a.same_chain(&final_b),
        "healed nodes did not converge: A={final_a:?} B={final_b:?}"
    );
    assert_eq!(
        final_a.tip, isolated_a.tip,
        "the isolated valid block was not the chain adopted after healing"
    );

    drop(b);
    drop(a);
    drop(link);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn partitioned_competing_chains_converge_on_heavier_work() {
    let root = unique_root();
    fs::create_dir_all(&root).expect("create harness root");

    let a_spec = NodeSpec::new(&root, "fork-a", Ipv4Addr::new(127, 0, 0, 2));
    let b_spec = NodeSpec::new(&root, "fork-b", Ipv4Addr::new(127, 0, 0, 3));

    a_spec.init();
    b_spec.init();

    // Give both future branches a real common prefix. Mining it before either
    // daemon starts keeps construction deterministic.
    a_spec.mine_blocks(1);

    // Use one controlled proxy in each dialing direction. This is important
    // for the reorg scenario: after healing the lighter node must itself be
    // able to dial the heavier node and exercise the pull/reorg path.
    let a_to_b = FaultProxy::spawn(
        b_spec.p2p,
        &[
            a_spec.p2p.port(),
            a_spec.rpc.port(),
            b_spec.p2p.port(),
            b_spec.rpc.port(),
        ],
    );

    let b_to_a = FaultProxy::spawn(
        a_spec.p2p,
        &[
            a_spec.p2p.port(),
            a_spec.rpc.port(),
            b_spec.p2p.port(),
            b_spec.rpc.port(),
            a_to_b.addr().port(),
        ],
    );

    let a = Node::spawn(a_spec.clone(), false, &[a_to_b.addr()]);
    let b = Node::spawn(b_spec.clone(), false, &[b_to_a.addr()]);

    wait_until(
        "A and B to share the pre-partition chain",
        CONVERGE_TIMEOUT,
        || {
            let sa = a.status()?;
            let sb = b.status()?;
            Ok(sa.blocks >= 1 && sa.same_chain(&sb))
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n--- node A ---\n{}\n--- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    let shared = a.status().expect("shared A status");
    let shared_b = b.status().expect("shared B status");

    assert!(
        shared.same_chain(&shared_b),
        "nodes did not establish an identical common prefix: \
         A={shared:?} B={shared_b:?}"
    );

    // Cut every controlled path and wait until both real session pools observe
    // the disconnect before either branch is changed.
    a_to_b.partition();
    b_to_a.partition();

    wait_until(
        "both sides to observe the partition",
        CONNECT_TIMEOUT,
        || {
            let sa = a.status()?;
            let sb = b.status()?;
            Ok(sa.peers == 0 && sb.peers == 0)
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n--- node A ---\n{}\n--- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    // The offline miner needs exclusive ownership of each datadir. Cleanly
    // stopping the daemons here also proves that the divergent branches
    // survive persistence/reload before reconciliation.
    drop(a);
    drop(b);

    // Same common prefix, genuinely different suffixes:
    //
    // A: shared + 1 block
    // B: shared + 2 blocks
    //
    // We do not infer the winner from length. The assertion below uses the
    // consensus value exported as total_work.
    a_spec.mine_blocks(1);
    b_spec.mine_blocks(2);

    // Restart while the proxies are still partitioned. Any convergence before
    // heal would prove the network fault boundary is porous.
    let a = Node::spawn(a_spec.clone(), false, &[a_to_b.addr()]);
    let b = Node::spawn(b_spec.clone(), false, &[b_to_a.addr()]);

    let isolated_a = a.status().expect("isolated A status");
    let isolated_b = b.status().expect("isolated B status");

    assert_eq!(
        isolated_a.blocks,
        shared.blocks + 1,
        "A did not reload exactly its isolated suffix"
    );
    assert_eq!(
        isolated_b.blocks,
        shared.blocks + 2,
        "B did not reload exactly its isolated suffix"
    );

    assert_ne!(
        isolated_a.tip, isolated_b.tip,
        "partition failed to create competing chains"
    );

    assert!(
        isolated_b.total_work > isolated_a.total_work,
        "test construction did not produce a heavier B branch: \
         A={isolated_a:?} B={isolated_b:?}"
    );

    thread::sleep(Duration::from_secs(3));

    let still_a = a.status().expect("A status before heal");
    let still_b = b.status().expect("B status before heal");

    assert_eq!(
        still_a.tip, isolated_a.tip,
        "A changed chain while both controlled links were partitioned"
    );
    assert_eq!(
        still_b.tip, isolated_b.tip,
        "B changed chain while both controlled links were partitioned"
    );

    // Restore both dialing directions. The final choice must be made by the
    // production handshake/sync/common-ancestor/reorg machinery.
    a_to_b.heal();
    b_to_a.heal();

    wait_until(
        "partitioned forks to converge on the heavier chain",
        CONVERGE_TIMEOUT,
        || {
            let sa = a.status()?;
            let sb = b.status()?;

            Ok(sa.same_chain(&sb)
                && sa.tip == isolated_b.tip
                && sa.total_work == isolated_b.total_work)
        },
    )
    .unwrap_or_else(|e| {
        panic!(
            "{e}\n\
             heavier branch before heal: {isolated_b:?}\n\
             lighter branch before heal: {isolated_a:?}\n\
             --- node A ---\n{}\n\
             --- node B ---\n{}",
            a.dump_log(),
            b.dump_log()
        )
    });

    let final_a = a.status().expect("final A status");
    let final_b = b.status().expect("final B status");

    assert!(
        final_a.same_chain(&final_b),
        "nodes disagree after healed fork: A={final_a:?} B={final_b:?}"
    );
    assert_eq!(
        final_a.tip, isolated_b.tip,
        "lighter chain survived despite lower cumulative work"
    );
    assert_eq!(
        final_a.total_work, isolated_b.total_work,
        "final chain does not carry the heavier branch's cumulative work"
    );
    assert!(
        final_a.total_work > isolated_a.total_work,
        "A did not move from the lighter branch to greater cumulative work"
    );

    drop(b);
    drop(a);
    drop(b_to_a);
    drop(a_to_b);

    fs::remove_dir_all(&root).ok();
}
