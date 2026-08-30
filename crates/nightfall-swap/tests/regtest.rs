//! Task G1 — the Bitcoin side against a real node.
//!
//! Every other test in this crate checks that our own code agrees with itself.
//! This one asks the only question that matters for the Bitcoin half: **does
//! Bitcoin Core's script interpreter accept what we build?**
//!
//! Signatures verifying under `ecdsa_fun` says nothing about whether a P2WSH
//! witness satisfies the script, whether the sighash was computed over the
//! right fields, or whether a BIP68 sequence actually enforces the delay. Only
//! a node can answer that, and until it has, "the Bitcoin side is done" is a
//! guess.
//!
//! Ignored by default: it needs `bitcoind` on PATH. Run with
//!
//! ```text
//! cargo test -p nightfall-swap --test regtest -- --ignored --nocapture
//! ```
//!
//! It starts its own regtest node in a temporary directory on port 18999 and
//! never touches a mainnet datadir.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::{Amount, ScriptBuf, Txid};
use nightfall_swap::adaptor;
use nightfall_swap::bitcoin_tx::{
    alice_encsign_refund, bob_encsign_redeem, p2wsh, two_of_two, TxCancel, TxLock, TxPunish,
    TxRedeem, TxRefund,
};
use nightfall_swap::SwapShare;
use std::process::Command;
use std::str::FromStr;

const DIR: &str = "/tmp/nfregtest";
const PORT: &str = "18999";

fn cli(args: &[&str]) -> String {
    let mut c = Command::new("bitcoin-cli");
    c.arg(format!("-datadir={DIR}"))
        .arg(format!("-rpcport={PORT}"))
        .arg("-rpcuser=nf")
        .arg("-rpcpassword=nfpass")
        .args(args);
    let out = c.output().expect("bitcoin-cli must be on PATH");
    if !out.status.success() {
        panic!(
            "bitcoin-cli {:?} failed:\n{}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn wallet(args: &[&str]) -> String {
    let mut v = vec!["-rpcwallet=swaptest"];
    v.extend_from_slice(args);
    cli(&v)
}

/// One test binary at a time.
///
/// Cargo runs integration-test binaries in parallel, and they all share this
/// node. Two of them mining at once breaks every assertion of the form
/// "refused before H₁, accepted after" — the blocks arrive from somewhere
/// else. The lock is a file, held for the length of the test.
pub struct NodeLock(std::path::PathBuf);

impl Drop for NodeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn node_lock() -> NodeLock {
    let path = std::path::PathBuf::from("/tmp/nfregtest.lock");
    for _ in 0..600 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return NodeLock(path),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    // Two minutes is long past "another test is busy" and into "a crashed run
    // left the file behind". Take it rather than fail every future run.
    let _ = std::fs::remove_file(&path);
    NodeLock(path)
}

/// Bring up a regtest node in `DIR`, or use the one already running there.
///
/// The doc comment at the top of this file promised a self-starting node and
/// the code did not deliver one — running it simply failed with "data
/// directory does not exist". A test nobody can run is a test that proves
/// nothing, which is the same trap as a test that cannot fail.
///
/// Never touches a real datadir: `-regtest` plus an explicit `-datadir` under
/// `/tmp`, on a port no mainnet node uses.
fn ensure_node() {
    use std::io::Write;

    // Already up? Then reuse it; repeated runs should be cheap.
    if Command::new("bitcoin-cli")
        .arg(format!("-datadir={DIR}"))
        .arg(format!("-rpcport={PORT}"))
        .arg("-rpcuser=nf")
        .arg("-rpcpassword=nfpass")
        .arg("getblockchaininfo")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    std::fs::create_dir_all(DIR).expect("create the regtest datadir");
    let conf = format!(
        "regtest=1\n\
         server=1\n\
         rpcuser=nf\n\
         rpcpassword=nfpass\n\
         [regtest]\n\
         rpcport={PORT}\n\
         txindex=1\n\
         fallbackfee=0.0002\n\
         maxtxfee=1.0\n"
    );
    let mut f = std::fs::File::create(format!("{DIR}/bitcoin.conf")).expect("write bitcoin.conf");
    f.write_all(conf.as_bytes()).expect("write bitcoin.conf");

    let started = Command::new("bitcoind")
        .arg(format!("-datadir={DIR}"))
        .arg("-daemon")
        .output()
        .expect("bitcoind must be on PATH");
    assert!(
        started.status.success(),
        "bitcoind failed to start: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    // Wait for RPC rather than sleeping a fixed time: a slow machine would
    // otherwise fail here for no reason.
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let up = Command::new("bitcoin-cli")
            .arg(format!("-datadir={DIR}"))
            .arg(format!("-rpcport={PORT}"))
            .arg("-rpcuser=nf")
            .arg("-rpcpassword=nfpass")
            .arg("getblockchaininfo")
            .output();
        if up.map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
    }
    panic!("bitcoind did not answer RPC within 30 s");
}

/// A wallet with spendable coins. Coinbase needs 100 confirmations, so the
/// first run mines 101 blocks.
fn ensure_wallet() {
    // Three states, not two: already loaded, on disk but unloaded, or absent.
    // The first version only knew the last two, so a second run in the same
    // node lifetime tried to create a wallet that was already open and died
    // on error −4. Ask first.
    let open = cli(&["listwallets"]);
    if !open.contains("swaptest") {
        let loaded = Command::new("bitcoin-cli")
            .arg(format!("-datadir={DIR}"))
            .arg(format!("-rpcport={PORT}"))
            .arg("-rpcuser=nf")
            .arg("-rpcpassword=nfpass")
            .args(["loadwallet", "swaptest"])
            .output()
            .expect("bitcoin-cli");
        if !loaded.status.success() {
            cli(&["createwallet", "swaptest"]);
        }
    }
    let balance = wallet(&["getbalance"]);
    if balance.parse::<f64>().unwrap_or(0.0) < 5.0 {
        let addr = wallet(&["getnewaddress"]);
        cli(&["generatetoaddress", "101", &addr]);
    }
}

/// Fund `address` with `btc`; return the real funding transaction, the vout
/// paying us, and the value. The transaction itself matters: `TxLock::txid()`
/// is `tx.compute_txid()`, so the spend can only reference the right outpoint
/// if the lock carries the transaction the node actually made.
fn fund(address: &str, btc: &str) -> (bitcoin::Transaction, u32, Amount) {
    let txid = wallet(&["sendtoaddress", address, btc]);
    let raw = cli(&["getrawtransaction", &txid, "1"]);
    // Find the vout paying our address, without pulling in a JSON dependency.
    let vout = raw
        .split("\"n\": ")
        .skip(1)
        .find_map(|chunk| {
            let n: u32 = chunk.split(',').next()?.trim().parse().ok()?;
            chunk.contains(address).then_some(n)
        })
        .expect("the funding output must exist");
    let hex = cli(&["getrawtransaction", &txid]);
    let bytes = hex_bytes(&hex);
    let tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&bytes).expect("node gave us a transaction");
    assert_eq!(tx.compute_txid(), Txid::from_str(&txid).unwrap());
    (tx, vout, Amount::from_btc(btc.parse().unwrap()).unwrap())
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn mine(n: u32) {
    let addr = wallet(&["getnewaddress"]);
    cli(&["generatetoaddress", &n.to_string(), &addr]);
}

fn accepted(tx_hex: &str) -> (bool, String) {
    let out = cli(&["testmempoolaccept", &format!("[\"{tx_hex}\"]")]);
    (out.contains("\"allowed\": true"), out)
}

fn dest() -> ScriptBuf {
    let a = wallet(&["getnewaddress"]);
    let info = cli(&["getaddressinfo", &a]);
    let spk = info
        .split("\"scriptPubKey\": \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("scriptPubKey");
    ScriptBuf::from_hex(spk).expect("hex script")
}

#[test]
#[ignore = "needs a bitcoind regtest node"]
fn the_real_interpreter_accepts_our_redeem_and_enforces_the_cancel_delay() {
    let _guard = node_lock();
    ensure_node();
    ensure_wallet();
    let mut rng = rand::rngs::OsRng;

    // Bitcoin keys for the 2-of-2, and Alice's Ristretto share whose secret the
    // adaptor will publish.
    let sk_a = adaptor::random_bitcoin_sk(&mut rng);
    let sk_b = adaptor::random_bitcoin_sk(&mut rng);
    let pk_a = adaptor::verification_key(&sk_a);
    let pk_b = adaptor::verification_key(&sk_b);
    let share_a = SwapShare::generate();
    let t_a = adaptor::encryption_point(&share_a.secret());

    let script = two_of_two(&pk_a, &pk_b).unwrap();
    let spk = p2wsh(&script);
    let addr = bitcoin::Address::from_script(&spk, bitcoin::Network::Regtest).unwrap();
    println!("2-of-2 P2WSH: {addr}");

    // --- the lock: built by the node, spent by us ---------------------------
    let (funding_tx, vout, value) = fund(&addr.to_string(), "1.0");
    println!("funded vout {vout} with {value}");

    // The funding transaction pays the 2-of-2, so it *is* the lock. Carrying
    // the real transaction is what makes `outpoint()` point at the real coin.
    let lock = TxLock {
        tx: funding_tx,
        script: script.clone(),
        value,
        vout,
    };

    // --- TX_redeem: the adaptor path ---------------------------------------
    let fee = Amount::from_sat(2_000);
    let redeem = TxRedeem::new(&lock, dest(), fee).expect("redeem builds");

    let enc = bob_encsign_redeem(&sk_b, &t_a, &redeem);
    let sig_b = adaptor::decrypt(&share_a.secret(), enc.clone());
    let sig_a = adaptor::sign(&sk_a, &redeem.sighash);

    let signed = redeem
        .clone()
        .complete(&pk_a, sig_a, &pk_b, sig_b.clone(), &script)
        .expect("witness assembles");
    let hex = serialize_hex(&signed);
    let (ok, why) = accepted(&hex);
    assert!(ok, "Bitcoin Core rejected TX_redeem:\n{why}");
    println!("TX_redeem accepted by the node");

    // The secret really is recoverable from what went on chain.
    let recovered = adaptor::recover(&t_a, &sig_b, &enc).expect("recover");
    assert_eq!(
        recovered,
        share_a.secret(),
        "the published signature must hand over exactly s_a"
    );
    println!("s_a recovered from the published signature");

    // --- TX_cancel: does the node enforce the BIP68 delay? -----------------
    let h1: u32 = 10;
    let cancel = TxCancel::new(&lock, &pk_a, &pk_b, h1, fee).expect("cancel builds");
    let c_a = adaptor::sign(&sk_a, &cancel.sighash);
    let c_b = adaptor::sign(&sk_b, &cancel.sighash);
    let cancel_tx = cancel
        .clone()
        .complete(&pk_a, c_a, &pk_b, c_b, &script)
        .expect("cancel witness");
    let cancel_hex = serialize_hex(&cancel_tx);

    let (early, why_early) = accepted(&cancel_hex);
    assert!(
        !early,
        "TX_cancel must NOT be spendable before its relative timelock:\n{why_early}"
    );
    println!("TX_cancel correctly rejected before H1 ({why_early})");

    mine(h1 + 1);
    let (late, why_late) = accepted(&cancel_hex);
    assert!(
        late,
        "TX_cancel must become valid after H1 blocks:\n{why_late}"
    );
    println!("TX_cancel accepted after {h1} blocks — BIP68 enforced as designed");

    // --- TX_refund carries Alice's adaptor under T_b -----------------------
    let share_b = SwapShare::generate();
    let t_b = adaptor::encryption_point(&share_b.secret());
    let refund = TxRefund::new(&cancel, dest(), fee).expect("refund builds");
    let enc_r = alice_encsign_refund(&sk_a, &t_b, &refund);
    let sig_a_r = adaptor::decrypt(&share_b.secret(), enc_r.clone());
    let recovered_b = adaptor::recover(&t_b, &sig_a_r, &enc_r).expect("recover s_b");
    assert_eq!(
        recovered_b,
        share_b.secret(),
        "refund must publish exactly s_b"
    );
    println!("TX_refund adaptor publishes s_b");

    // --- TX_punish: the second timelock, on the cancel output ---------------
    //
    // The cancel transaction has to be on chain before anything can spend it,
    // so this also confirms the cancel path really lands.
    let cancel_txid = cli(&["sendrawtransaction", &cancel_hex]);
    println!("TX_cancel broadcast: {cancel_txid}");
    mine(1);

    let h2: u32 = 5;
    let punish = TxPunish::new(&cancel, dest(), h2, fee).expect("punish builds");
    let p_a = adaptor::sign(&sk_a, &punish.sighash);
    let p_b = adaptor::sign(&sk_b, &punish.sighash);
    let punish_tx = punish
        .complete(&pk_a, p_a, &pk_b, p_b, &cancel.script)
        .expect("punish witness");
    let punish_hex = serialize_hex(&punish_tx);

    let (early_p, why_ep) = accepted(&punish_hex);
    assert!(
        !early_p,
        "TX_punish must not be spendable before H2:\n{why_ep}"
    );
    mine(h2 + 1);
    let (late_p, why_lp) = accepted(&punish_hex);
    assert!(late_p, "TX_punish must be valid after H2:\n{why_lp}");
    println!("TX_punish rejected before H2, accepted after — the whole abort tree validates");
}
