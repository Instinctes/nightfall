//! Shared plumbing for the live swap runs.
//!
//! Everything here talks to a regtest node through `bitcoin-cli`. Kept out of
//! the run files so that each of those reads as the story it tells, and so a
//! change to the plumbing cannot quietly alter one run and not another.
//!
//! Each run file compiles its own copy of this module, so anything only one
//! of them uses looks dead to the others. That is a property of Rust's
//! integration tests, not a sign of unused code.
#![allow(dead_code)]

use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};
use nightfall_swap::messages::Amounts;
use nightfall_swap::session::Session;
use nightfall_swap::timelock::Depths;
use nightfall_types::NetworkId;
use std::process::Command;
use std::str::FromStr;

pub const DIR: &str = "/tmp/nfregtest";
pub const PORT: &str = "18999";
pub const NET: NetworkId = NetworkId::Testnet;

pub fn cli(args: &[&str]) -> String {
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

pub fn wallet(args: &[&str]) -> String {
    let mut v = vec!["-rpcwallet=swaptest"];
    v.extend_from_slice(args);
    cli(&v)
}

/// Try to broadcast; return the node's complaint instead of panicking, so a
/// test can assert that a timelock *refused* it.
pub fn try_send(raw_hex: &str) -> Result<String, String> {
    let out = Command::new("bitcoin-cli")
        .arg(format!("-datadir={DIR}"))
        .arg(format!("-rpcport={PORT}"))
        .arg("-rpcuser=nf")
        .arg("-rpcpassword=nfpass")
        .args(["sendrawtransaction", raw_hex])
        .output()
        .expect("bitcoin-cli");
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn mine(n: u32) {
    let addr = wallet(&["getnewaddress"]);
    cli(&["generatetoaddress", &n.to_string(), &addr]);
}

pub fn node_is_up() -> bool {
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

/// A spendable output of at least `sats`, as (outpoint, value).
pub fn a_utxo(sats: u64) -> (OutPoint, Amount) {
    let addr = wallet(&["getnewaddress", "", "bech32"]);
    let btc = format!("{:.8}", (sats as f64) / 100_000_000.0);
    let txid = wallet(&["sendtoaddress", &addr, &btc]);
    mine(1);
    let raw = cli(&["getrawtransaction", &txid, "1"]);

    // Find the vout paying our address, without a JSON dependency.
    let mut vout = None;
    for (i, chunk) in raw.split("\"n\": ").enumerate().skip(1) {
        let n: u32 = chunk
            .split(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX);
        let window_end = chunk.find("\"n\": ").unwrap_or(chunk.len());
        if chunk[..window_end].contains(&addr) {
            vout = Some(n);
            break;
        }
        let _ = i;
    }
    let vout = vout.expect("our address must be among the outputs");
    (
        OutPoint {
            txid: Txid::from_str(&txid).unwrap(),
            vout,
        },
        Amount::from_sat(sats),
    )
}

pub fn spk(tag: u8) -> ScriptBuf {
    // A P2WPKH-shaped script. Content is irrelevant to the protocol; both
    // sides only have to commit to the same bytes.
    let mut v = vec![0x00, 0x14];
    v.extend_from_slice(&[tag; 20]);
    ScriptBuf::from_bytes(v)
}

pub fn amounts(fee: u64) -> Amounts {
    Amounts {
        night_darks: 250_000_000,
        btc_sats: 100_000,
        btc_fee_sats: fee,
    }
}

/// Bring both sides through the full handshake with real regtest depths.
pub fn full_handshake(depths: Depths) -> (Session, Session, Amounts) {
    let a = amounts(1_000);
    let mut bob = Session::open_as_bob(NET, a.clone(), depths, spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    let p1 = alice.next_packet().unwrap();
    bob.accept_packet(&p1).unwrap();
    (alice, bob, a)
}

/// Sign the unsigned lock with the node's wallet and put it on chain.
pub fn fund_and_broadcast_lock(bob: &mut Session, prev: OutPoint, value: Amount) -> String {
    let change = wallet(&["getnewaddress", "", "bech32"]);
    let change_spk = bitcoin::Address::from_str(&change)
        .unwrap()
        .assume_checked()
        .script_pubkey();
    bob.lock_packet(prev, value, Some(change_spk)).unwrap();

    let unsigned = bob.unsigned_lock_hex().unwrap();
    let signed_json = wallet(&["signrawtransactionwithwallet", &unsigned]);
    let signed = signed_json
        .split("\"hex\": \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("a signed hex in the response")
        .to_string();

    // Segwit: the witness does not change the txid, so the transaction we
    // hand out for signing is the transaction that confirms. If that ever
    // stopped being true, every pre-signed child would point at a ghost.
    bob.verify_confirmed_lock_hex(&signed)
        .expect("the signed transaction must still be the one we built");

    let txid = try_send(&signed).expect("the lock must broadcast");
    mine(1);
    txid
}

pub fn alice_key(alice: &Session) -> ecdsa_fun::fun::Point {
    nightfall_swap::adaptor::verification_key(&alice.secrets().btc_sk)
}

pub fn bob_key(bob: &Session) -> ecdsa_fun::fun::Point {
    nightfall_swap::adaptor::verification_key(&bob.secrets().btc_sk)
}

/// Packets 2 through 5, after Bob has put the lock on chain.
///
/// Split out because every run needs it and none of them is *about* it: the
/// interesting part of each run starts once both sides hold every signature.
pub fn exchange_rest(alice: &mut Session, bob: &mut Session) {
    let p2 = bob.last_packet().cloned().expect("the lock packet");
    alice.accept_packet(&p2).unwrap();
    let p3 = alice.next_packet().unwrap();
    bob.accept_packet(&p3).unwrap();
    let p4 = bob.next_packet().unwrap();
    alice.accept_packet(&p4).unwrap();
    let p5 = bob.next_packet().unwrap();
    alice.accept_packet(&p5).unwrap();
    bob.remember_redeem_enc(&serde_json::from_value(p5.body.clone()).unwrap());
}

/// One test binary at a time. See the note in `regtest.rs`: cargo runs these
/// binaries in parallel and they share one chain, so concurrent mining turns
/// "refused before H1" into a coin flip.
pub struct NodeLock(std::path::PathBuf);

impl Drop for NodeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn node_lock() -> NodeLock {
    let path = std::path::PathBuf::from("/tmp/nfregtest.lock");
    for _ in 0..600 {
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_ok()
        {
            return NodeLock(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(&path);
    NodeLock(path)
}
