use super::*;
use nightfall_crypto::WalletKeys;
use std::net::TcpListener;
use std::thread;
use std::time::Instant;

// Public deterministic test fixture; no real wallet or configured datadir.
fn fixture() -> (Wallet, Vec<Block>) {
    let keys = WalletKeys::from_seed([0x69; 32]);
    let mut chain = nightfall_consensus::Chain::new_fair(NetworkId::Devnet).unwrap();
    let now = nightfall_storage::now_unix();
    for i in 0..3 {
        chain
            .mine_block(&keys.address(), vec![], now - 300 + i * 15)
            .unwrap();
    }
    (Wallet::in_memory(NetworkId::Devnet, keys, 0), chain.blocks)
}

fn mock_rpc(
    responses: Vec<serde_json::Value>,
) -> (String, thread::JoinHandle<Vec<serde_json::Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for result in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing expected RPC request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("mock RPC accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            requests.push(serde_json::from_str(&line).unwrap());
            writeln!(stream, "{}", json!({"result": result, "id": 1})).unwrap();
        }
        requests
    });
    (addr, worker)
}

#[test]
fn real_rpc_single_block_pages_continue_to_the_actual_end() {
    let (mut wallet, blocks) = fixture();
    let (addr, worker) = mock_rpc(vec![
        json!({"pruned": false}),
        json!([blocks[0]]),
        json!([blocks[1]]),
        json!([blocks[2]]),
        json!([]),
    ]);
    let count = sync(&mut wallet, &addr, 0).unwrap();
    let requests = worker.join().unwrap();
    assert_eq!(count, 3);
    assert_eq!(wallet.scanned_to(), 2);
    assert_eq!(
        requests
            .iter()
            .skip(1)
            .map(|r| r["params"]["from"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
}

#[test]
fn real_rpc_gap_after_a_good_page_preserves_wallet_state() {
    let (mut wallet, blocks) = fixture();
    let before = wallet.export_state().unwrap();
    let (addr, worker) = mock_rpc(vec![
        json!({"pruned": false}),
        json!([blocks[0]]),
        json!([blocks[2]]),
    ]);
    let result = sync(&mut wallet, &addr, 0);
    worker.join().unwrap();
    assert!(result.is_err());
    assert_eq!(wallet.export_state().unwrap(), before);
}
