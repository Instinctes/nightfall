//! A real node downloads short block pages over TCP.
//! All network traffic is restricted to loopback Devnet.

use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_p2p::{read_msg, write_msg, PeerMsg};
use nightfall_storage::now_unix;
use nightfall_types::{NetworkId, WIRE_VERSION};

use std::io::{BufReader, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn real_node_recovers_every_short_wire_page() {
    let miner = WalletKeys::from_seed([17u8; 32]).address();

    let mut chain = Chain::new_fair(NetworkId::Devnet).expect("devnet chain");

    let start = now_unix().saturating_sub(200);

    // Nine genuine mined Devnet blocks.
    for i in 0..9u64 {
        chain
            .mine_block(&miner, vec![], start + i * 15)
            .expect("mine valid devnet block");
    }

    let expected_tip = chain.tip_hash().to_hex();
    let genesis = chain.genesis_hash.to_hex();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let peer_port = listener.local_addr().unwrap().port();

    // Share observed TCP page requests for CI diagnostics.
    let seen_requests = Arc::new(Mutex::new(Vec::<u64>::new()));
    let seen_by_peer = Arc::clone(&seen_requests);
    let sync_complete = Arc::new(AtomicBool::new(false));
    let completed_by_peer = Arc::clone(&sync_complete);

    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept real node");

        // Poll for test completion while keeping the peer alive during
        // potentially slow Windows proof verification.
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());

        let mut writer: TcpStream = stream;

        match read_msg(&mut reader).expect("peer hello") {
            PeerMsg::Hello {
                wire,
                network,
                genesis: remote_genesis,
                ..
            } => {
                assert_eq!(wire, WIRE_VERSION);
                assert_eq!(network, NetworkId::Devnet);
                assert_eq!(remote_genesis, genesis);
            }
            other => panic!("unexpected hello: {other:?}"),
        }

        write_msg(
            &mut writer,
            &PeerMsg::HelloOk {
                wire: WIRE_VERSION,
                network: NetworkId::Devnet,
                genesis,
                height: 8,
                tip: chain.tip_hash().to_hex(),
                listen_port: 0,
                pruned: false,
                first_height: 0,
            },
        )
        .expect("send hello_ok");

        let mut requested = Vec::new();

        loop {
            if completed_by_peer.load(Ordering::SeqCst) {
                break;
            }
            let message = match read_msg(&mut reader) {
                Ok(message) => message,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    continue;
                }
                Err(e) => panic!("read peer request: {e}"),
            };

            match message {
                PeerMsg::GetBlocks { from_height, limit } => {
                    requested.push(from_height);
                    seen_by_peer.lock().expect("request log").push(from_height);

                    // Deliberately return a maximum of
                    // three blocks, never the requested 128.
                    let blocks = chain.blocks_from(from_height, limit.min(3));

                    write_msg(&mut writer, &PeerMsg::Blocks { blocks })
                        .expect("send short block page");

                    // Do not close the socket immediately after page 6.
                    // A slow receiver must still be able to read it.
                }

                PeerMsg::GetStatus => {
                    write_msg(
                        &mut writer,
                        &PeerMsg::Status {
                            height: 8,
                            tip: chain.tip_hash().to_hex(),
                            bits: 1,
                            peers: 1,
                            mempool: 0,
                        },
                    )
                    .unwrap();
                }

                PeerMsg::Ping { nonce } => {
                    write_msg(&mut writer, &PeerMsg::Pong { nonce }).unwrap();
                }

                PeerMsg::GetPeers => {
                    write_msg(&mut writer, &PeerMsg::Peers { addrs: Vec::new() }).unwrap();
                }

                _ => {}
            }

            assert!(requested.len() <= 24, "too many requests: {requested:?}");
        }

        requested
    });

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let datadir =
        std::env::temp_dir().join(format!("nightfall-ibd-{}-{nonce}", std::process::id()));

    std::fs::create_dir_all(&datadir).unwrap();

    let node = NodeHandle::start(NodeConfig {
        network: NetworkId::Devnet,
        datadir,
        p2p_listen: "127.0.0.1:0".into(),
        rpc_listen: format!("127.0.0.1:{}", free_port()),
        connect: vec![format!("127.0.0.1:{peer_port}")],
        mine: false,
        miner: None,
        proxy: Some("off".into()),
        mobile_listen: None,
        peers_url: Some("off".into()),
        introducer: false,
        prune: false,
    })
    .expect("start real node");

    let started = Instant::now();
    // An unchanged tip means a stall; advancing tips reset the quiet deadline.
    // The separate total cap still prevents an unbounded CI run.
    let quiet_budget = Duration::from_secs(if cfg!(windows) { 240 } else { 50 });
    let total_budget = Duration::from_secs(if cfg!(windows) { 540 } else { 150 });
    let mut last_progress = started;
    let mut last_height = None;

    loop {
        let status = node.status_snapshot().expect("node status");

        if status.tip == expected_tip {
            break;
        }

        if last_height != Some(status.tip_height) {
            last_height = Some(status.tip_height);
            last_progress = Instant::now();
            eprintln!("IBD progress: height {}", status.tip_height);
        }

        assert!(
            last_progress.elapsed() < quiet_budget && started.elapsed() < total_budget,
            "IBD stalled at height {} after {}s ({}s without progress), requested pages {:?}",
            status.tip_height,
            started.elapsed().as_secs(),
            last_progress.elapsed().as_secs(),
            seen_requests.lock().expect("request log").clone()
        );

        std::thread::sleep(Duration::from_millis(100));
    }

    sync_complete.store(true, Ordering::SeqCst);
    let requests = peer.join().expect("peer thread");

    assert!(requests.contains(&0), "genesis not requested: {requests:?}");

    assert!(
        requests.contains(&3),
        "missing short-page successor 3: {requests:?}"
    );

    assert!(
        requests.contains(&6),
        "missing short-page successor 6: {requests:?}"
    );

    assert!(
        !requests.contains(&128),
        "invalid height jump: {requests:?}"
    );

    {
        let state = node.shared();
        let inner = state.lock().unwrap();

        assert_eq!(inner.chain.block_count(), 9);
        assert_eq!(inner.chain.tip_hash().to_hex(), expected_tip);

        inner.chain.verify_supply().expect("supply invariant");
    }

    eprintln!(
        "P0 REAL TCP IBD PASS: 9 blocks, {} ms, requests {:?}",
        started.elapsed().as_millis(),
        requests
    );
}
