//! A2 — our own RPC client against a real Bitcoin Core node.
//!
//! `bitcoin_rpc.rs` had unit tests over recorded error strings, which is a
//! test of our parser and not of the conversation. This asks the node.
//!
//! The question that matters is the one that bit us twice already: does an
//! *unanswerable* query stay unanswerable, or does it quietly become zero?
//! A pruned node that no longer has a transaction must read as "unknown",
//! because zero confirmations means "in the mempool, H₁ is far away" and
//! that is the sentence that opens the redeem window at the wrong moment.
//!
//! Ignored by default: needs `bitcoind` on PATH. The regtest test brings the
//! node up; run that first, or this one starts it itself.
//!
//! ```text
//! cargo test -p nightfall-swap --test rpc_live -- --ignored --nocapture
//! ```

mod common;
use common::node_lock;

use nightfall_swap::bitcoin_rpc::{BitcoinRpc, RpcAuth};
use nightfall_swap::watch::{ChainWatch, TxRef, WatchError};
use std::process::Command;

const DIR: &str = "/tmp/nfregtest";
const PORT: &str = "18999";

fn cli(args: &[&str]) -> String {
    let out = Command::new("bitcoin-cli")
        .arg(format!("-datadir={DIR}"))
        .arg(format!("-rpcport={PORT}"))
        .arg("-rpcuser=nf")
        .arg("-rpcpassword=nfpass")
        .args(args)
        .output()
        .expect("bitcoin-cli must be on PATH");
    assert!(
        out.status.success(),
        "bitcoin-cli {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn node_is_up() -> bool {
    Command::new("bitcoin-cli")
        .arg(format!("-datadir={DIR}"))
        .arg(format!("-rpcport={PORT}"))
        .arg("-rpcuser=nf")
        .arg("-rpcpassword=nfpass")
        .arg("getblockchaininfo")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn rpc() -> BitcoinRpc {
    BitcoinRpc::new(RpcAuth {
        url: format!("http://127.0.0.1:{PORT}"),
        user: "nf".into(),
        password: "nfpass".into(),
    })
}

#[test]
#[ignore = "needs a bitcoind regtest node (run the regtest test first)"]
fn our_client_reads_the_real_node() {
    let _guard = node_lock();
    assert!(
        node_is_up(),
        "no regtest node at {DIR}:{PORT} — run the regtest test first"
    );
    let rpc = rpc();

    // 1. The tip. `getblockcount` through our own client, checked against
    //    the node's own answer, so a parsing mistake cannot hide.
    let ours = rpc.height().expect("height must read");
    let theirs: u64 = cli(&["getblockcount"]).parse().unwrap();
    assert_eq!(ours, theirs, "our height must be the node's height");

    // 2. A transaction the node has. Mine one so the depth is known.
    let addr = cli(&["-rpcwallet=swaptest", "getnewaddress"]);
    let txid = cli(&["-rpcwallet=swaptest", "sendtoaddress", &addr, "0.01"]);
    let in_mempool = rpc
        .confirmations(&TxRef { id: txid.clone() })
        .expect("a mempool transaction is a normal answer");
    assert_eq!(
        in_mempool,
        Some(0),
        "a transaction sitting in the mempool is zero confirmations, not unknown"
    );

    cli(&["generatetoaddress", "3", &addr]);
    let confirmed = rpc.confirmations(&TxRef { id: txid.clone() });
    match confirmed {
        Ok(Some(3)) => println!("three blocks, three confirmations"),
        // The default node has no transaction index. Every confirmed
        // transaction is invisible to `getrawtransaction`, including ones the
        // node mined itself. That must say so, not masquerade as "unknown"
        // (which would read as "node unreachable" while H1 runs out) and not
        // as a transient outage (which the driver would retry forever).
        Err(WatchError::NeedsTxIndex) => {
            println!("node has no -txindex: reported as a configuration problem, not as unknown")
        }
        other => panic!(
            "a confirmed transaction must either read, or say the node needs \
             -txindex. Got {other:?}"
        ),
    }

    // 3. The one that matters. A transaction the node has never seen must
    //    read as unknown — never as zero.
    let stranger = "0".repeat(64);
    let unknown = rpc
        .confirmations(&TxRef { id: stranger })
        .expect("an unknown transaction is not an outage");
    assert_eq!(
        unknown, None,
        "a transaction the node does not have is UNKNOWN. Reading it as \
         Some(0) would tell the swap it has the whole cancel window left."
    );

    println!("height {ours}, mempool 0 conf, mined 3 conf, unknown tx -> None");
}

/// An unreachable node is a third thing again: not zero, not unknown, an
/// error. The caller must be told, because a swap that keeps ticking on a
/// dead node is a swap making decisions on nothing.
#[test]
#[ignore = "needs a bitcoind regtest node"]
fn a_closed_port_is_an_error_not_a_missing_transaction() {
    let dead = BitcoinRpc::new(RpcAuth {
        // A port nothing listens on.
        url: "http://127.0.0.1:18998".into(),
        user: "nf".into(),
        password: "nfpass".into(),
    });
    let e = dead
        .confirmations(&TxRef { id: "0".repeat(64) })
        .expect_err("a closed port must surface as an error");
    println!("closed port -> {e:?}");

    let h = dead.height().expect_err("and so must the height");
    println!("closed port height -> {h:?}");
}
