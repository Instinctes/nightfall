//! Exercise the SAME worker the GUI runs, against Bitcoin Core and a real NIGHT node.
use crate::{swap_worker::SwapWorker, wallet_state::WalletState};
use bitcoin::{Amount, OutPoint};
use nightfall_node::{NodeConfig, NodeHandle};
use nightfall_swap::{persist, session::Session, Amounts, Depths, Role, StoredSwap, SwapState};
use nightfall_types::NetworkId;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct BitcoinTest {
    process: Child,
    dir: PathBuf,
    port: String,
}
impl Drop for BitcoinTest {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
impl BitcoinTest {
    fn start(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
            .to_string();
        let process = Command::new("bitcoind")
            .args([
                "-regtest",
                "-server",
                "-txindex=1",
                "-listen=0",
                "-connect=0",
                "-dnsseed=0",
                "-discover=0",
                "-fallbackfee=0.0002",
                "-rpcuser=nf-test",
                "-rpcpassword=local-test-only",
            ])
            .arg(format!("-datadir={}", dir.display()))
            .arg(format!("-rpcport={port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("bitcoind installed");
        let mut node = Self { process, dir, port };
        for _ in 0..100 {
            if node.try_cli(&["getblockchaininfo"]).is_ok() {
                node.cli(&["createwallet", "test"]);
                node.mine(101);
                return node;
            }
            assert!(
                node.process.try_wait().unwrap().is_none(),
                "bitcoind exited"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("bitcoind failed to start");
    }
    fn try_cli(&self, args: &[&str]) -> Result<serde_json::Value, String> {
        let o = Command::new("bitcoin-cli")
            .args([
                "-regtest",
                "-rpcuser=nf-test",
                "-rpcpassword=local-test-only",
            ])
            .arg(format!("-datadir={}", self.dir.display()))
            .arg(format!("-rpcport={}", self.port))
            .args(args)
            .output()
            .unwrap();
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).into());
        }
        let text = String::from_utf8(o.stdout).unwrap();
        Ok(serde_json::from_str(&text)
            .unwrap_or_else(|_| serde_json::Value::String(text.trim().into())))
    }
    fn cli(&self, args: &[&str]) -> serde_json::Value {
        self.try_cli(args).unwrap()
    }
    fn address(&self) -> String {
        self.cli(&["-rpcwallet=test", "getnewaddress", "", "bech32"])
            .as_str()
            .unwrap()
            .into()
    }
    fn mine(&self, n: u32) {
        self.cli(&["generatetoaddress", &n.to_string(), &self.address()]);
    }
    fn config(&self, dir: &std::path::Path) {
        nightfall_storage::write_secret_file(
            &dir.join("bitcoin-rpc.conf"),
            &format!(
                "url=http://127.0.0.1:{}\nuser=nf-test\npassword=local-test-only\n",
                self.port
            ),
        )
        .unwrap();
    }
}

fn tick(worker: &mut SwapWorker, id: &str) {
    // Drop cached handshakes before EVERY step: equivalent to repeated app restarts.
    worker.swap_sessions.clear();
    let s = persist::load(&worker.datadir, id.parse().unwrap()).unwrap();
    if !s.is_finished() {
        worker.tick_one_swap(s).unwrap();
    }
}

fn mine_night(node: &NodeHandle, miner: &nightfall_crypto::Address, n: u32) {
    for _ in 0..n {
        let shared = node.shared();
        let (mut chain, txs) = {
            let g = shared.lock().unwrap();
            (g.chain.clone(), g.mempool.txs.values().cloned().collect())
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .max(chain.median_time_past() + 1);
        let block = chain.mine_block(miner, txs, timestamp).unwrap();
        let mut g = shared.lock().unwrap();
        g.chain = chain;
        g.mempool.remove_included(&block);
        // This deliberately isolated fixture has no remote peer. Expire the
        // startup peer-discovery hold, just as a standalone dev node does.
        g.behind_since = 0;
    }
}

#[test]
#[ignore = "starts isolated bitcoind and NIGHT nodes; run with --ignored --nocapture"]
fn core_worker_completes_both_chains_and_recovers_night_after_refund() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nf-core-swap-{stamp}"));
    let btc = BitcoinTest::start(root.join("bitcoin"));
    let net = NetworkId::Devnet;
    let alice_dir = root.join("alice");
    let bob_dir = root.join("bob");
    let alice_wallet = Arc::new(Mutex::new(
        WalletState::load_or_create(&alice_dir, net).unwrap(),
    ));
    let bob_wallet = Arc::new(Mutex::new(
        WalletState::load_or_create(&bob_dir, net).unwrap(),
    ));
    btc.config(&alice_dir);
    btc.config(&bob_dir);
    let miner = alice_wallet.lock().unwrap().address().unwrap();
    let node = Arc::new(
        NodeHandle::start(NodeConfig {
            network: net,
            datadir: root.join("node"),
            p2p_listen: "127.0.0.1:0".into(),
            rpc_listen: "127.0.0.1:0".into(),
            connect: vec![],
            mine: false,
            miner: Some(miner),
            proxy: None,
            mobile_listen: None,
            peers_url: Some("off".into()),
            introducer: false,
            prune: false,
        })
        .unwrap(),
    );
    mine_night(&node, &miner, 12);
    let mut alice = SwapWorker {
        network: net,
        datadir: alice_dir,
        node: Some(node.clone()),
        wallet: alice_wallet,
        swap_sessions: HashMap::new(),
    };
    let mut bob = SwapWorker {
        network: net,
        datadir: bob_dir,
        node: Some(node.clone()),
        wallet: bob_wallet,
        swap_sessions: HashMap::new(),
    };
    for abort in [false, true] {
        let script = |address: String| {
            address
                .parse::<bitcoin::Address<_>>()
                .unwrap()
                .require_network(bitcoin::Network::Regtest)
                .unwrap()
                .script_pubkey()
        };
        let amounts = Amounts {
            night_darks: 100_000_000,
            btc_sats: 100_000,
            btc_fee_sats: 1_000,
        };
        let mut bs = Session::open_as_bob(net, amounts, Depths::devnet(), script(btc.address()));
        bs.persist_to(&bob.datadir);
        let mut asess = Session::join_from_packet(
            net,
            script(btc.address()),
            script(btc.address()),
            &bs.next_packet().unwrap(),
        )
        .unwrap();
        asess.persist_to(&alice.datadir);
        bs.accept_packet(&asess.next_packet().unwrap()).unwrap();
        let address = btc.address();
        let funding = btc.cli(&["-rpcwallet=test", "sendtoaddress", &address, "0.002"]);
        btc.mine(1);
        let funding_id = funding.as_str().unwrap();
        let transaction = btc.cli(&["getrawtransaction", funding_id, "true"]);
        let out = transaction["vout"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["scriptPubKey"]["address"].as_str() == Some(&address))
            .unwrap();
        let prev = OutPoint {
            txid: funding_id.parse().unwrap(),
            vout: out["n"].as_u64().unwrap() as u32,
        };
        asess
            .accept_packet(
                &bs.lock_packet(prev, Amount::from_sat(200_000), Some(script(btc.address())))
                    .unwrap(),
            )
            .unwrap();
        bs.accept_packet(&asess.next_packet().unwrap()).unwrap();
        asess.accept_packet(&bs.next_packet().unwrap()).unwrap();
        asess.accept_packet(&bs.next_packet().unwrap()).unwrap();
        let id = bs.id.to_string();
        let mut a = StoredSwap::from_session(&asess);
        alice.wallet.lock().unwrap().sync_from_node(&node).unwrap();
        a.reserved_commits = alice
            .wallet
            .lock()
            .unwrap()
            .pick_commit_hexes_at(
                100_000_000 + crate::app::DEFAULT_FEE_DARKS,
                node.status_snapshot().unwrap().tip_height,
                10,
            )
            .unwrap();
        alice
            .wallet
            .lock()
            .unwrap()
            .reserve_commits(&a.reserved_commits)
            .unwrap();
        persist::save(&alice.datadir, &a).unwrap();
        let b = StoredSwap::from_session(&bs);
        persist::save(&bob.datadir, &b).unwrap();
        let signed = btc.cli(&[
            "-rpcwallet=test",
            "signrawtransactionwithwallet",
            &bs.unsigned_lock_hex().unwrap(),
        ]);
        bob.broadcast_lock(&b, signed["hex"].as_str().unwrap())
            .unwrap();
        btc.mine(1);
        tick(&mut alice, &id);
        tick(&mut alice, &id);
        let a = persist::load(&alice.datadir, asess.id).unwrap();
        alice.lock_night(&a).unwrap();
        assert!(
            alice
                .lock_night(&persist::load(&alice.datadir, asess.id).unwrap())
                .is_err(),
            "cannot pay twice"
        );
        mine_night(&node, &miner, 2);
        if abort {
            btc.mine(4);
            for _ in 0..3 {
                tick(&mut bob, &id);
            }
            btc.mine(1); // cancel
            tick(&mut bob, &id);
            btc.mine(1); // refund
            for _ in 0..3 {
                tick(&mut alice, &id);
            }
            let a = persist::load(&alice.datadir, asess.id).unwrap();
            assert!(matches!(
                a.state,
                SwapState::Refunded {
                    role: Role::Alice,
                    ..
                }
            ));
            assert!(
                !a.is_finished(),
                "NIGHT recovery cannot finish in the mempool"
            );
            mine_night(&node, &miner, 2);
            tick(&mut alice, &id);
            tick(&mut bob, &id);
            assert!(
                persist::load(&alice.datadir, asess.id)
                    .unwrap()
                    .night_recovered
            );
            assert!(persist::load(&bob.datadir, bs.id).unwrap().is_finished());
            println!(
                "PASS: Bitcoin cancel/refund and Alice NIGHT recovery, with restart at every tick"
            );
        } else {
            for _ in 0..3 {
                tick(&mut alice, &id);
            }
            btc.mine(1); // redeem leaves mempool BEFORE Bob sees it
            for _ in 0..3 {
                tick(&mut bob, &id);
            }
            assert!(!persist::load(&bob.datadir, bs.id).unwrap().is_finished());
            mine_night(&node, &miner, 2);
            tick(&mut bob, &id);
            tick(&mut alice, &id);
            assert!(matches!(
                persist::load(&bob.datadir, bs.id).unwrap().state,
                SwapState::Done { .. }
            ));
            assert!(matches!(
                persist::load(&alice.datadir, asess.id).unwrap().state,
                SwapState::Done { .. }
            ));
            bob.wallet.lock().unwrap().sync_from_node(&node).unwrap();
            assert!(
                bob.wallet
                    .lock()
                    .unwrap()
                    .balances(node.status_snapshot().unwrap().tip_height, 10)
                    .available
                    > 0
            );
            println!("PASS: Core worker NIGHT lock → confirmed Bitcoin redeem → Bob NIGHT claim, with restart at every tick");
        }
        node.shared().lock().unwrap().chain.verify_supply().unwrap();
    }
    println!("Isolated test data: {}", root.display());
}
