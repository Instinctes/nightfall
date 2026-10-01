//! Real multi-node Dandelion++ transport integration.
//!
//! Unlike the unit harness, this test uses three complete NodeHandle instances,
//! real loopback TCP connections, the real handshake/capability negotiation,
//! the real PeerMsg wire codec and the real runtime receive path.
//!
//! The chain fixture is prepared directly through the public NodeHandle test
//! surface. That keeps consensus setup deterministic; transaction transport
//! itself receives no shortcut.

use nightfall_crypto::{scan_output, WalletKeys};
use nightfall_ledger::{build_transfer, coinbase_maturity, Payment, Spendable};
use nightfall_node::dandelion::RelayMode;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_storage::now_unix;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT};

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
        ));

        std::fs::create_dir_all(&path).expect("create temporary node directory");

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

struct TestNode {
    node: NodeHandle,
    p2p: u16,
    _dir: TempDir,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("local address")
        .port()
}

fn addr(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

fn wait_until<F>(label: &str, timeout: Duration, mut condition: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if condition() {
            return;
        }

        std::thread::sleep(Duration::from_millis(25));
    }

    panic!("timed out waiting for {label}");
}

fn boot(tag: &str, p2p: u16, connect: Vec<String>) -> TestNode {
    let dir = TempDir::new(tag);

    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.path().to_path_buf(),
        p2p_listen: addr(p2p),
        rpc_listen: "127.0.0.1:0".into(),
        connect,
        mine: false,
        miner: None,
        proxy: Some("off".into()),
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .expect("node starts");

    wait_until("node loading", Duration::from_secs(10), || {
        node.status_snapshot()
            .map(|status| !status.loading)
            .unwrap_or(false)
    });

    TestNode {
        node,
        p2p,
        _dir: dir,
    }
}

fn boot_with_proxy(tag: &str, p2p: u16, connect: Vec<String>, proxy: Option<String>) -> TestNode {
    let dir = TempDir::new(tag);

    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir: dir.path().to_path_buf(),
        p2p_listen: addr(p2p),
        rpc_listen: "127.0.0.1:0".into(),
        connect,
        mine: false,
        miner: None,
        proxy,
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .expect("node starts");

    wait_until("node loading", Duration::from_secs(10), || {
        node.status_snapshot()
            .map(|status| !status.loading)
            .unwrap_or(false)
    });

    TestNode {
        node,
        p2p,
        _dir: dir,
    }
}

fn sessions_with_peer(node: &NodeHandle, peer_id: &str, outbound: bool) -> usize {
    let shared = node.shared();
    let state = shared.lock().unwrap();

    state
        .sessions
        .all()
        .iter()
        .filter(|session| session.peer_id == peer_id && session.outbound == outbound)
        .count()
}

/// Advance the existing production Dandelion router through epoch boundaries
/// until the requested relay mode is selected.
///
/// This is integration-test orchestration only. No production flag, RPC hook,
/// environment override or alternate routing implementation is introduced.
fn condition_router_mode(node: &NodeHandle, excluded_source_peer: &str, desired: RelayMode) {
    let shared = node.shared();
    let mut state = shared.lock().unwrap();

    let outbound = state
        .sessions
        .dandelion_outbound_keys_except_peer(Some(excluded_source_peer));

    assert!(
        !outbound.is_empty(),
        "test topology has no safe Dandelion destination"
    );

    let mut synthetic_now = now_unix();

    for _ in 0..4096 {
        state.dandelion_router.refresh(synthetic_now, &outbound);

        if state.dandelion_router.mode() == desired {
            return;
        }

        synthetic_now = state.dandelion_router.epoch_ends_at();
    }

    panic!("could not condition router to {desired:?} in 4096 epochs");
}

fn prepare_shared_chain(nodes: [&NodeHandle; 3], miner: &WalletKeys) -> Spendable {
    let mut chain = {
        let shared = nodes[0].shared();
        let state = shared.lock().unwrap();
        chain_clone(&state.chain)
    };

    let miner_address = miner.address();
    let maturity = coinbase_maturity(NetworkId::Devnet);

    // Two blocks beyond maturity so the fixture does not sit on a boundary.
    for _ in 0..maturity.saturating_add(2) {
        let timestamp = now_unix().max(chain.median_time_past() + 1);

        chain
            .mine_block(&miner_address, Vec::new(), timestamp)
            .expect("mine deterministic Devnet fixture");
    }

    let tip = chain.tip_height().expect("fixture has a tip").0;

    let next_height = tip.saturating_add(1);
    let view = miner.view_key();

    let mut spendable = None;

    for block in chain.blocks_from(0, 128) {
        let created = block.header.height.0;

        if next_height < created.saturating_add(maturity) {
            continue;
        }

        for output in &block.body.outputs {
            if let Some(decoded) = scan_output(&view, output) {
                spendable = Some(Spendable {
                    commit: decoded.commit,
                    value: decoded.value,
                    blind: decoded.blind,
                    spend_secret: decoded.spend_secret(miner),
                });

                break;
            }
        }

        if spendable.is_some() {
            break;
        }
    }

    let spendable = spendable.expect("mature miner output");

    // Install exactly the same valid consensus state in all three nodes.
    // The purpose of this integration test is transport/relay, not IBD.
    let a = nodes[0].shared();
    let b = nodes[1].shared();
    let c = nodes[2].shared();

    let mut ga = a.lock().unwrap();
    let mut gb = b.lock().unwrap();
    let mut gc = c.lock().unwrap();

    assert!(ga.mempool.is_empty());
    assert!(gb.mempool.is_empty());
    assert!(gc.mempool.is_empty());

    assert!(ga.stempool.is_empty());
    assert!(gb.stempool.is_empty());
    assert!(gc.stempool.is_empty());

    ga.chain = chain.clone();
    gb.chain = chain.clone();
    gc.chain = chain;

    ga.best_peer_height = tip;
    gb.best_peer_height = tip;
    gc.best_peer_height = tip;

    ga.behind_since = 0;
    gb.behind_since = 0;
    gc.behind_since = 0;

    spendable
}

// Kept separate so the fixture makes the intentional Chain clone obvious.
fn chain_clone(chain: &nightfall_consensus::Chain) -> nightfall_consensus::Chain {
    chain.clone()
}

fn run_real_three_node_case(desired_mode: RelayMode) {
    assert!(
        NetworkId::Devnet.seed_nodes().is_empty(),
        "Devnet integration must never reach public seeds"
    );

    // Reserve all addresses before any node starts. This lets each side keep
    // explicit reciprocal connections and therefore reproduces the exact
    // inbound/outbound-sibling condition Stage 2F2B is meant to protect.
    let a_port = free_port();
    let b_port = free_port();
    let c_port = free_port();

    let a_addr = addr(a_port);
    let b_addr = addr(b_port);
    let c_addr = addr(c_port);

    // Start C first. B explicitly dials both A and C; A explicitly dials B.
    // B therefore ends up with:
    //
    //   inbound from A
    //   outbound to A
    //   outbound to C
    //
    // The first two sockets are different sessions but one logical peer.
    let c = boot("nf-dpp-c", c_port, vec![b_addr.clone()]);

    let b = boot("nf-dpp-b", b_port, vec![a_addr.clone(), c_addr.clone()]);

    let a = boot("nf-dpp-a", a_port, vec![b_addr.clone()]);

    wait_until(
        "reciprocal three-node topology",
        Duration::from_secs(20),
        || {
            sessions_with_peer(&a.node, &b_addr, true) >= 1
                && sessions_with_peer(&b.node, &a_addr, false) >= 1
                && sessions_with_peer(&b.node, &a_addr, true) >= 1
                && sessions_with_peer(&b.node, &c_addr, true) >= 1
                && sessions_with_peer(&c.node, &b_addr, false) >= 1
        },
    );

    // Prove the real handshakes produced the identity collision we care about:
    // B has two independent sockets carrying the same logical A identity.
    {
        let shared = b.node.shared();
        let state = shared.lock().unwrap();
        let sessions = state.sessions.all();

        let a_sessions = sessions
            .iter()
            .filter(|session| session.peer_id == a_addr)
            .collect::<Vec<_>>();

        assert!(
            a_sessions.iter().any(|s| s.outbound),
            "B needs a real outbound sibling to A"
        );

        assert!(
            a_sessions.iter().any(|s| !s.outbound),
            "B needs the real inbound source socket from A"
        );

        assert!(
            sessions
                .iter()
                .any(|s| { s.outbound && s.peer_id == c_addr && s.dandelion_stem_v1 }),
            "B needs a negotiated D++ route to C"
        );
    }

    // A and C must have no direct relationship. Otherwise a legitimate fluff
    // from C could return to A and make the anti-echo assertion ambiguous.
    {
        let shared = a.node.shared();
        let state = shared.lock().unwrap();

        assert!(
            state.sessions.all().iter().all(|s| s.peer_id != c_addr),
            "A unexpectedly learned a direct C session"
        );
    }

    {
        let shared = c.node.shared();
        let state = shared.lock().unwrap();

        assert!(
            state.sessions.all().iter().all(|s| s.peer_id != a_addr),
            "C unexpectedly learned a direct A session"
        );
    }

    let miner = WalletKeys::from_seed([0x31; 32]);

    let recipient = WalletKeys::from_seed([0x72; 32]);

    let spendable = prepare_shared_chain([&a.node, &b.node, &c.node], &miner);

    // Freeze B onto the requested production relay branch before the
    // transaction reaches it. The synthetic epoch end remains ahead of the
    // real wall clock, so the normal receive path does not redraw the mode.
    condition_router_mode(&b.node, &a_addr, desired_mode);

    let tx = build_transfer(
        &miner,
        &[spendable],
        &[Payment {
            to: recipient.address(),
            amount: DARKS_PER_NIGHT,
            memo: "three-node Dandelion TCP test".into(),
        }],
        100,
        &miner.address(),
        0,
        NetworkId::Devnet.proof_context(),
    )
    .expect("build valid transfer");

    let expected_txid = tx.txid().to_hex();

    // The real local submission path must place the payment in A's stempool,
    // not its mining mempool, and send it through A's negotiated TCP session.
    let submitted_txid = {
        let shared = a.node.shared();
        let mut state = shared.lock().unwrap();

        state.submit_tx(tx).expect("origin accepts transfer")
    };

    assert_eq!(submitted_txid, expected_txid);

    {
        let shared = a.node.shared();
        let state = shared.lock().unwrap();

        assert!(
            state.stempool.contains(&expected_txid),
            "origin did not enter stem phase"
        );

        assert!(
            state.mempool.first_seen(&expected_txid).is_none(),
            "origin transaction became mineable before diffusion"
        );
    }

    // B must receive the actual P2P transaction. Its epoch may legitimately be
    // either Stem or Fluff; both branches have a deterministic safe outcome.
    wait_until(
        "B receives transaction over TCP",
        Duration::from_secs(5),
        || {
            let shared = b.node.shared();
            let state = shared.lock().unwrap();

            state.stempool.contains(&expected_txid)
                || state.mempool.first_seen(&expected_txid).is_some()
        },
    );

    {
        let shared = b.node.shared();
        let state = shared.lock().unwrap();

        assert_eq!(
            state.dandelion_router.mode(),
            desired_mode,
            "B changed Dandelion mode before transaction processing",
        );

        match desired_mode {
            RelayMode::Stem => {
                assert!(
                    state.stempool.contains(&expected_txid),
                    "explicit STEM case did not remain in B stempool",
                );

                assert!(
                    state.mempool.first_seen(&expected_txid).is_none(),
                    "explicit STEM case became mineable at B",
                );
            }

            RelayMode::Fluff => {
                assert!(
                    !state.stempool.contains(&expected_txid),
                    "explicit FLUFF case remained in B stempool",
                );

                assert!(
                    state.mempool.first_seen(&expected_txid).is_some(),
                    "explicit FLUFF case did not enter B mempool",
                );
            }
        }

        if let Some(entry) = state.stempool.get(&expected_txid) {
            assert_eq!(
                entry.source.as_deref(),
                Some(a_addr.as_str()),
                "B did not preserve A's logical peer identity"
            );

            let route = state
                .sessions
                .get(&entry.route)
                .expect("stem route remains a live session");

            assert_ne!(
                route.peer_id, a_addr,
                "B routed the stem back to A through A's sibling socket"
            );

            assert_eq!(
                route.peer_id, c_addr,
                "with A excluded, B's only valid next peer is C"
            );

            eprintln!(
                "B exercised STEM path: {} -> {}",
                entry.source.as_deref().unwrap(),
                route.peer_id
            );
        } else {
            assert!(
                state.mempool.first_seen(&expected_txid).is_some(),
                "B holds transaction in neither relay phase"
            );

            eprintln!("B exercised FLUFF path for this epoch");
        }
    }

    // C has no logical peer other than B. Therefore an incoming stem has no
    // safe next edge and must enter diffusion; incoming fluff does the same.
    wait_until(
        "C receives downstream transaction",
        Duration::from_secs(5),
        || {
            let shared = c.node.shared();
            let state = shared.lock().unwrap();

            state.mempool.first_seen(&expected_txid).is_some()
        },
    );

    // We are still comfortably before Nightfall's 12-second minimum embargo.
    // If A has already moved to its public mempool now, the transaction came
    // back over the network — precisely the same-peer echo Stage 2F2B forbids.
    {
        let shared = a.node.shared();
        let state = shared.lock().unwrap();

        assert!(
            state.stempool.contains(&expected_txid),
            "origin stem disappeared before its embargo"
        );

        assert!(
            state.mempool.first_seen(&expected_txid).is_none(),
            "downstream diffusion echoed back to A through a sibling session"
        );
    }

    eprintln!(
        "real TCP path verified: A({}) -> B({}) -> C({})",
        a.p2p, b.p2p, c.p2p,
    );
}

#[test]
#[ignore = "real three-node TCP Dandelion STEM topology"]
fn real_three_node_stem_never_echoes_to_same_logical_peer() {
    run_real_three_node_case(RelayMode::Stem);
}

#[test]
#[ignore = "real three-node TCP Dandelion FLUFF topology"]
fn real_three_node_fluff_never_echoes_to_same_logical_peer() {
    run_real_three_node_case(RelayMode::Fluff);
}

#[test]
#[ignore = "real TCP alias identity and Dandelion deduplication"]
fn real_outbound_aliases_collapse_to_one_logical_dandelion_candidate() {
    assert!(
        NetworkId::Devnet.seed_nodes().is_empty(),
        "Devnet integration must never reach public seeds"
    );

    let a_port = free_port();
    let b_port = free_port();
    let c_port = free_port();

    let a_addr = addr(a_port);
    let b_addr = addr(b_port);
    let c_addr = addr(c_port);

    // Two distinct dial strings intentionally address the exact same
    // Nightfall listener.
    //
    // Session identity must remain transport-specific:
    //
    //   out:127.0.0.1:A
    //   out:localhost:A
    //
    // Logical Dandelion identity must instead collapse both to the address
    // derived from the real TCP peer plus A's advertised listening port:
    //
    //   127.0.0.1:A
    let a_alias = format!("localhost:{a_port}");

    let _a = boot("nf-dpp-alias-a", a_port, Vec::new());

    let _c = boot("nf-dpp-alias-c", c_port, Vec::new());

    let b = boot(
        "nf-dpp-alias-b",
        b_port,
        vec![a_addr.clone(), a_alias.clone(), c_addr.clone()],
    );

    wait_until(
        "two real outbound aliases to logical A and one route to C",
        Duration::from_secs(20),
        || {
            sessions_with_peer(&b.node, &a_addr, true) >= 2
                && sessions_with_peer(&b.node, &c_addr, true) >= 1
        },
    );

    let shared = b.node.shared();
    let state = shared.lock().unwrap();

    let sessions = state.sessions.all();

    let a_sessions = sessions
        .iter()
        .filter(|session| {
            session.outbound && session.dandelion_stem_v1 && session.peer_id == a_addr
        })
        .collect::<Vec<_>>();

    assert_eq!(
        a_sessions.len(),
        2,
        "real handshake did not retain two independent transport sessions to A"
    );

    let a_transport_keys = a_sessions
        .iter()
        .map(|session| session.key.clone())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(
        a_transport_keys.len(),
        2,
        "two A aliases unexpectedly collapsed at the session-key layer"
    );

    let literal_key = format!("out:{a_addr}");

    let alias_key = format!("out:{a_alias}");

    assert!(
        a_transport_keys.contains(&literal_key),
        "literal 127.0.0.1 transport session missing"
    );

    assert!(
        a_transport_keys.contains(&alias_key),
        "localhost transport session missing"
    );

    // Both independent TCP sessions must resolve to one logical privacy
    // identity through the real handshake.
    for session in &a_sessions {
        assert_eq!(
            session.peer_id, a_addr,
            "real alias connection produced a different logical peer identity"
        );
    }

    let routes = state.sessions.dandelion_outbound_keys();

    // Three capable outbound sockets exist:
    //
    //   A literal
    //   A localhost alias
    //   C
    //
    // but only two logical peers exist.
    assert_eq!(
        routes.len(),
        2,
        "multiple sockets to logical A consumed multiple Dandelion slots"
    );

    let routed_sessions = routes
        .iter()
        .map(|route| {
            state
                .sessions
                .get(route)
                .expect("selected Dandelion route must remain live")
        })
        .collect::<Vec<_>>();

    let routed_peer_ids = routed_sessions
        .iter()
        .map(|session| session.peer_id.clone())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(
        routed_peer_ids.len(),
        routes.len(),
        "real Dandelion route set contains duplicate logical peers"
    );

    assert!(
        routed_peer_ids.contains(&a_addr),
        "logical A disappeared entirely from Dandelion candidates"
    );

    assert!(
        routed_peer_ids.contains(&c_addr),
        "logical C disappeared from Dandelion candidates"
    );

    let routed_to_a = routed_sessions
        .iter()
        .filter(|session| session.peer_id == a_addr)
        .count();

    assert_eq!(
        routed_to_a, 1,
        "logical A still occupies more than one Dandelion destination"
    );

    // Stage 2G1 deliberately chooses the lexicographically smallest live
    // session key as representative.
    let expected_a_route = if literal_key < alias_key {
        literal_key.clone()
    } else {
        alias_key.clone()
    };

    assert!(
        routes.contains(&expected_a_route),
        "logical A did not use its deterministic representative route"
    );

    let without_a = state
        .sessions
        .dandelion_outbound_keys_except_peer(Some(a_addr.as_str()));

    assert_eq!(
        without_a.len(),
        1,
        "excluding logical A did not remove both real alias sockets"
    );

    let only_remaining = state
        .sessions
        .get(&without_a[0])
        .expect("remaining logical-C route");

    assert_eq!(
        only_remaining.peer_id, c_addr,
        "logical-A exclusion left an alias route behind"
    );

    eprintln!(
        "real alias dedup verified: B({}) has {} TCP sockets to A({}) \
         but exactly one Dandelion candidate for logical A",
        b_addr,
        a_sessions.len(),
        a_addr,
    );
}

fn spawn_mapping_socks_proxy(
    routes: std::collections::HashMap<String, String>,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test SOCKS5 proxy");

    let proxy_addr = listener
        .local_addr()
        .expect("test SOCKS5 proxy address")
        .to_string();

    let routes = std::sync::Arc::new(routes);

    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    let observed_accept = std::sync::Arc::clone(&observed);

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut client) = incoming else {
                break;
            };

            let routes = std::sync::Arc::clone(&routes);

            let observed = std::sync::Arc::clone(&observed_accept);

            std::thread::spawn(move || {
                use std::io::{Read, Write};

                let mut greeting = [0u8; 3];

                if client.read_exact(&mut greeting).is_err() {
                    return;
                }

                if greeting != [0x05, 0x01, 0x00] {
                    return;
                }

                if client.write_all(&[0x05, 0x00]).is_err() {
                    return;
                }

                let mut head = [0u8; 4];

                if client.read_exact(&mut head).is_err() {
                    return;
                }

                if head[0] != 0x05 || head[1] != 0x01 {
                    return;
                }

                let host = match head[3] {
                    0x01 => {
                        let mut raw = [0u8; 4];

                        if client.read_exact(&mut raw).is_err() {
                            return;
                        }

                        std::net::Ipv4Addr::from(raw).to_string()
                    }

                    0x03 => {
                        let mut length = [0u8; 1];

                        if client.read_exact(&mut length).is_err() {
                            return;
                        }

                        let mut raw = vec![0u8; length[0] as usize];

                        if client.read_exact(&mut raw).is_err() {
                            return;
                        }

                        match String::from_utf8(raw) {
                            Ok(host) => host,
                            Err(_) => return,
                        }
                    }

                    0x04 => {
                        let mut raw = [0u8; 16];

                        if client.read_exact(&mut raw).is_err() {
                            return;
                        }

                        std::net::Ipv6Addr::from(raw).to_string()
                    }

                    _ => return,
                };

                let mut raw_port = [0u8; 2];

                if client.read_exact(&mut raw_port).is_err() {
                    return;
                }

                let port = u16::from_be_bytes(raw_port);

                let requested = format!("{host}:{port}");

                if let Ok(mut list) = observed.lock() {
                    list.push(requested.clone());
                }

                let Some(destination) = routes.get(&requested).cloned() else {
                    let _ = client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);

                    return;
                };

                let Ok(mut upstream) = std::net::TcpStream::connect(destination) else {
                    let _ = client.write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);

                    return;
                };

                if client
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .is_err()
                {
                    return;
                }

                let Ok(mut client_to_upstream) = client.try_clone() else {
                    return;
                };

                let Ok(mut upstream_writer) = upstream.try_clone() else {
                    return;
                };

                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut client_to_upstream, &mut upstream_writer);
                });

                let _ = std::io::copy(&mut upstream, &mut client);
            });
        }
    });

    (proxy_addr, observed)
}

#[test]
#[ignore = "real SOCKS5 Dandelion logical identity isolation"]
fn real_socks_onion_targets_keep_distinct_logical_identities() {
    assert!(
        NetworkId::Devnet.seed_nodes().is_empty(),
        "Devnet integration must never reach public seeds"
    );

    let a_port = free_port();
    let b_port = free_port();
    let c_port = free_port();

    let a_addr = addr(a_port);
    let c_addr = addr(c_port);

    let _a = boot("nf-dpp-socks-a", a_port, Vec::new());

    let _c = boot("nf-dpp-socks-c", c_port, Vec::new());

    // Synthetic v3-style Onion hostnames.
    //
    // No live Tor daemon is required: the local test
    // SOCKS proxy maps them onto the two real Devnet
    // Nightfall listeners.
    let onion_a_host = format!("{}.onion", "a".repeat(56));

    let onion_c_host = format!("{}.onion", "b".repeat(56));

    let onion_a = format!("{onion_a_host}:{a_port}");

    let onion_c = format!("{onion_c_host}:{c_port}");

    let mut mappings = std::collections::HashMap::new();

    mappings.insert(onion_a.clone(), a_addr.clone());

    mappings.insert(onion_c.clone(), c_addr.clone());

    let (proxy_addr, observed) = spawn_mapping_socks_proxy(mappings);

    let b = boot_with_proxy(
        "nf-dpp-socks-b",
        b_port,
        vec![onion_a.clone(), onion_c.clone()],
        Some(proxy_addr.clone()),
    );

    wait_until(
        "both Onion destinations reached SOCKS proxy",
        Duration::from_secs(20),
        || {
            let Ok(list) = observed.lock() else {
                return false;
            };

            list.iter().any(|target| target == &onion_a)
                && list.iter().any(|target| target == &onion_c)
        },
    );

    wait_until(
        "distinct Onion logical sessions",
        Duration::from_secs(20),
        || {
            sessions_with_peer(&b.node, &onion_a, true) >= 1
                && sessions_with_peer(&b.node, &onion_c, true) >= 1
        },
    );

    let shared = b.node.shared();
    let state = shared.lock().unwrap();

    let sessions = state.sessions.all();

    let a_sessions = sessions
        .iter()
        .filter(|session| {
            session.outbound && session.dandelion_stem_v1 && session.peer_id == onion_a
        })
        .collect::<Vec<_>>();

    let c_sessions = sessions
        .iter()
        .filter(|session| {
            session.outbound && session.dandelion_stem_v1 && session.peer_id == onion_c
        })
        .collect::<Vec<_>>();

    assert!(
        !a_sessions.is_empty(),
        "Onion A did not retain its dial target as peer_id"
    );

    assert!(
        !c_sessions.is_empty(),
        "Onion C did not retain its dial target as peer_id"
    );

    assert_ne!(onion_a, onion_c);

    for session in &a_sessions {
        assert_eq!(session.peer_id, onion_a);

        assert_ne!(
            session.peer_id, proxy_addr,
            "SOCKS proxy became logical identity"
        );
    }

    for session in &c_sessions {
        assert_eq!(session.peer_id, onion_c);

        assert_ne!(
            session.peer_id, proxy_addr,
            "SOCKS proxy became logical identity"
        );
    }

    let routes = state.sessions.dandelion_outbound_keys();

    let routed_peer_ids = routes
        .iter()
        .map(|route| {
            state
                .sessions
                .get(route)
                .expect("selected route remains live")
                .peer_id
        })
        .collect::<std::collections::HashSet<_>>();

    assert!(
        routed_peer_ids.contains(&onion_a),
        "Onion A absent from Dandelion candidates"
    );

    assert!(
        routed_peer_ids.contains(&onion_c),
        "Onion C absent from Dandelion candidates"
    );

    assert_eq!(
        routed_peer_ids.len(),
        routes.len(),
        "shared proxy collapsed distinct logical peers"
    );

    assert!(
        !routed_peer_ids.contains(&proxy_addr),
        "proxy endpoint leaked into Dandelion identity"
    );

    let without_a = state
        .sessions
        .dandelion_outbound_keys_except_peer(Some(onion_a.as_str()));

    let without_a_peer_ids = without_a
        .iter()
        .map(|route| {
            state
                .sessions
                .get(route)
                .expect("remaining route remains live")
                .peer_id
        })
        .collect::<std::collections::HashSet<_>>();

    assert!(
        !without_a_peer_ids.contains(&onion_a),
        "Onion A survived logical source exclusion"
    );

    assert!(
        without_a_peer_ids.contains(&onion_c),
        "excluding Onion A also removed Onion C"
    );

    let list = observed.lock().unwrap();

    assert!(
        list.iter().any(|target| target == &onion_a),
        "SOCKS did not receive Onion A hostname"
    );

    assert!(
        list.iter().any(|target| target == &onion_c),
        "SOCKS did not receive Onion C hostname"
    );

    eprintln!(
        "real SOCKS identity isolation verified: \
         shared proxy {} preserved logical peers {} and {}",
        proxy_addr, onion_a, onion_c,
    );
}
