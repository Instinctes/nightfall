//! Local JSON-RPC over TCP (newline-delimited JSON) for wallets.
//!
//! The RPC has **no authentication**, so it must never listen on a public
//! interface. v4 documented "run on localhost only" but did not enforce it —
//! a single mistyped `--rpc-listen` exposed full wallet control to the
//! internet. Binding a non-loopback address now requires an explicit opt-in.

use crate::runtime::SharedState;
use nightfall_ledger::Transaction;
use nightfall_storage::now_unix;
use nightfall_types::{Amount, MAX_MESSAGE_BYTES, MAX_SUPPLY_NIGHT, TICKER};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct RpcReq {
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    #[serde(default)]
    id: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcRes {
    result: Option<serde_json::Value>,
    error: Option<String>,
    id: serde_json::Value,
}

fn ok(result: serde_json::Value, id: serde_json::Value) -> RpcRes {
    RpcRes {
        result: Some(result),
        error: None,
        id,
    }
}

fn err(message: impl Into<String>, id: serde_json::Value) -> RpcRes {
    RpcRes {
        result: None,
        error: Some(message.into()),
        id,
    }
}

/// Is this address safe to expose an unauthenticated RPC on?
pub fn is_loopback_addr(addr: &str) -> bool {
    addr.parse::<SocketAddr>()
        .map(|s| match s.ip() {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback(),
        })
        .unwrap_or(false)
}

pub fn spawn_rpc(addr: String, state: SharedState) {
    if !is_loopback_addr(&addr) && std::env::var("NF_ALLOW_PUBLIC_RPC").is_err() {
        tracing::error!(
            "refusing to bind RPC to non-loopback address {addr}. \
             The RPC is unauthenticated and grants full wallet control. \
             Use 127.0.0.1, or set NF_ALLOW_PUBLIC_RPC=1 if it sits behind an \
             authenticating reverse proxy."
        );
        return;
    }

    thread::spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("rpc bind {addr}: {e}");
                return;
            }
        };
        tracing::info!("rpc listening on {addr}");
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    let st = Arc::clone(&state);
                    thread::spawn(move || {
                        if let Err(e) = handle_client(stream, st) {
                            tracing::debug!("rpc client: {e}");
                        }
                    });
                }
                Err(e) => tracing::warn!("rpc accept: {e}"),
            }
        }
    });
}

fn handle_client(stream: TcpStream, state: SharedState) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    loop {
        // Bounded read: an unbounded one is a trivial memory exhaustion vector.
        let mut buf = Vec::with_capacity(1024);
        let n = (&mut reader)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if n > MAX_MESSAGE_BYTES {
            let res = err("request too large", json!(null));
            writeln!(writer, "{}", serde_json::to_string(&res)?)?;
            break;
        }

        let text = String::from_utf8_lossy(&buf);
        let req: RpcReq = match serde_json::from_str(text.trim()) {
            Ok(r) => r,
            Err(e) => {
                let res = err(format!("bad json: {e}"), json!(null));
                writeln!(writer, "{}", serde_json::to_string(&res)?)?;
                continue;
            }
        };
        if req.method == "scan_subscribe" {
            handle_scan_subscribe(&req, &state, &mut writer)?;
            break;
        }
        let res = dispatch(&req, &state);
        writeln!(writer, "{}", serde_json::to_string(&res)?)?;
    }
    Ok(())
}

fn dispatch(req: &RpcReq, state: &SharedState) -> RpcRes {
    let id = req.id.clone();

    match req.method.as_str() {
        "status" => {
            let g = state.lock().unwrap();
            let chain = &g.chain;
            let supply_ok = chain.verify_supply().is_ok();
            // What the rest of the network is running, counted by version.
            // During an incident this is the first thing worth knowing, and it
            // used to be unanswerable from anywhere: the handshake carried an
            // agent string that the node received and discarded.
            let mut peer_versions: std::collections::BTreeMap<&str, usize> =
                std::collections::BTreeMap::new();
            for agent in g.peer_agents.values() {
                *peer_versions.entry(agent.as_str()).or_insert(0) += 1;
            }
            ok(
                json!({
                    "network": chain.network.as_str(),
                    "protocol_version": nightfall_types::PROTOCOL_VERSION,
                    "blocks": chain.block_count(),
                    "tip_height": chain.tip_height().map(|h| h.0),
                    "tip": chain.tip_hash().to_hex(),
                    "genesis": chain.genesis_hash.to_hex(),
                    "difficulty": chain.next_difficulty(),
                    "total_work": chain.total_work.to_string(),
                    "minted": chain.ledger.supply.total_minted_darks,
                    "burned_fees": chain.ledger.supply.total_burned_darks,
                    "circulating": chain.ledger.supply.circulating(),
                    "utxos": chain.ledger.utxos.len(),
                    "kernels": chain.ledger.kernels.count,
                    "utxo_root": chain.ledger.utxo_root().to_hex(),
                    "supply_invariant_ok": supply_ok,
                    "mempool": g.mempool.len(),
                    "peers": g.sessions.len(),
                    "known_peers": g.peer_addrs.len(),
                    "live_peers": g.sessions.len(),
                    "wire_version": nightfall_types::WIRE_VERSION,
                    "peer_versions": peer_versions,
                    "max_supply": MAX_SUPPLY_NIGHT,
                    "ticker": TICKER,
                }),
                id,
            )
        }

        "verify_supply" => {
            let g = state.lock().unwrap();
            match g.chain.verify_supply() {
                Ok(()) => ok(
                    json!({
                        "ok": true,
                        "circulating_darks": g.chain.ledger.supply.circulating(),
                        "circulating": Amount(g.chain.ledger.supply.circulating()).to_string(),
                    }),
                    id,
                ),
                Err(e) => err(e.to_string(), id),
            }
        }

        "get_utxo_root" => {
            let g = state.lock().unwrap();
            ok(
                json!({
                    "utxo_root": g.chain.ledger.utxo_root().to_hex(),
                    "kernel_sum": g.chain.ledger.kernel_sum().to_hex(),
                    "blocks": g.chain.block_count(),
                }),
                id,
            )
        }

        "submit_tx" => {
            let raw = req.params.get("tx").cloned().unwrap_or(json!(null));
            let tx: Transaction = match serde_json::from_value(raw) {
                Ok(t) => t,
                Err(e) => return err(format!("tx decode: {e}"), id),
            };
            match state.lock().unwrap().submit_tx(tx) {
                Ok(txid) => ok(json!({ "txid": txid, "accepted": true }), id),
                Err(e) => err(e, id),
            }
        }

        // Everything a wallet needs to find its own coins, and nothing else.
        //
        // A block is dominated by Bulletproofs: ~672 bytes per output, and the
        // scanner never looks at them — it computes an ECDH against the
        // ephemeral key and compares the result to the one-time key. Stripping
        // the proofs and the kernels cuts the wire cost by roughly 5x, which
        // on a phone is the difference between a sync people tolerate and one
        // they do not.
        //
        // The client asks for height ranges, never for a named commitment.
        // Asking "do you have this output" would tell the node exactly which
        // output is yours and throw away the privacy that scanning locally
        // buys in the first place. There is deliberately no such method.
        //
        // Trust: this returns what the node believes. A wallet on a phone
        // cannot check the proof of work — Argon2id at 32 MiB per hash is not
        // a thing a battery does — so a hostile node can show a payment that
        // does not exist. It cannot spend anything, because the seed never
        // leaves the device. Point the wallet at your own node.
        "scan_feed" => {
            let from = req.params.get("from").and_then(|v| v.as_u64()).unwrap_or(0);
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(256)
                .clamp(1, 1_024) as usize;
            ok(scan_feed_snapshot(state, from, limit), id)
        }

        "get_blocks" => {
            let from = req.params.get("from").and_then(|v| v.as_u64()).unwrap_or(0);
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(128)
                .min(nightfall_p2p::MAX_BLOCKS_PER_REQUEST as u64) as usize;
            let g = state.lock().unwrap();
            match serde_json::to_value(g.chain.blocks_from(from, limit)) {
                Ok(v) => ok(v, id),
                Err(e) => err(e.to_string(), id),
            }
        }

        "mine_one" => {
            // Build the template and mine it without holding the lock.
            let (template, miner_present) = {
                let g = state.lock().unwrap();
                match &g.miner {
                    None => (None, false),
                    Some(m) => {
                        let txs = g
                            .mempool
                            .select_for_block(nightfall_consensus::MAX_TXS_PER_BLOCK - 1);
                        (g.chain.build_template(m, txs, now_unix()).ok(), true)
                    }
                }
            };
            if !miner_present {
                return err("mining not enabled on this node", id);
            }
            let Some(template) = template else {
                return err("could not build a block template", id);
            };

            let difficulty = template.header.difficulty;
            let pow_params = {
                let g = state.lock().unwrap();
                g.chain.pow_params()
            };
            let Some((nonce, _)) = nightfall_crypto::mine_interruptible(
                &template.header.pow_preimage(),
                difficulty,
                rand::random(),
                pow_params,
                &|| false,
            ) else {
                return err("mining aborted", id);
            };
            let block = template.seal(nonce);

            let mut g = state.lock().unwrap();
            match g.chain.apply_block(block.clone(), now_unix()) {
                Ok(()) => {
                    g.mempool.remove_included(&block);
                    let _ = g.persist();
                    let response = json!({
                        "height": block.header.height.0,
                        "hash": block.hash().to_hex(),
                        "difficulty": difficulty,
                        "reward": Amount(block.header.reward_darks).to_string(),
                    });
                    g.announce_block(block);
                    ok(response, id)
                }
                Err(e) => err(e.to_string(), id),
            }
        }

        "banner" => ok(
            json!({
                "coin": "NIGHTFALLCOIN",
                "tagline": "Money that refuses to snitch.",
                "max_supply": format!("{MAX_SUPPLY_NIGHT} {TICKER}"),
                "protocol": nightfall_types::PROTOCOL_VERSION,
            }),
            id,
        ),

        other => err(format!("unknown method {other}"), id),
    }
}

/// One page of the light-client feed. Shared by `scan_feed` and the
/// long-lived `scan_subscribe` stream so a phone and a one-shot CLI see
/// the same shape.
fn scan_feed_snapshot(state: &SharedState, from: u64, limit: usize) -> serde_json::Value {
    let g = state.lock().unwrap();
    let blocks = g.chain.blocks_from(from, limit);

    let mut outputs = Vec::new();
    let mut spent = Vec::new();
    let mut scanned_to = from;

    for block in &blocks {
        scanned_to = scanned_to.max(block.header.height.0);
        for input in &block.body.inputs {
            spent.push(hex::encode(input.commit.0));
        }
        for out in &block.body.outputs {
            outputs.push(json!({
                "height": block.header.height.0,
                "commit": hex::encode(out.commit.0),
                "ephemeral_pk": hex::encode(out.ephemeral_pk),
                "output_pk": hex::encode(out.output_pk),
                "view_tag": out.view_tag,
                "payload": hex::encode(&out.payload),
                "coinbase": out.features.is_coinbase(),
            }));
        }
    }

    json!({
        "from": from,
        "scanned_to": scanned_to,
        "blocks": blocks.len(),
        "tip_height": g.chain.tip_height().map(|h| h.0),
        "genesis": g.chain.genesis_hash.to_hex(),
        "outputs": outputs,
        "spent": spent,
        "heartbeat": false,
    })
}

/// Push a `scan_feed` page every time the tip moves. The client keeps the
/// TCP connection open; a disconnect is how it unsubscribes.
///
/// A 30-second idle tick sends an empty page (`heartbeat: true`) so a
/// phone can tell a silent node from a dead one without polling.
fn handle_scan_subscribe(
    req: &RpcReq,
    state: &SharedState,
    writer: &mut TcpStream,
) -> anyhow::Result<()> {
    let mut from = req.params.get("from").and_then(|v| v.as_u64()).unwrap_or(0);
    let limit = req
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(256)
        .clamp(1, 1_024) as usize;
    let id = req.id.clone();

    loop {
        let snap = scan_feed_snapshot(state, from, limit);
        let scanned_to = snap
            .get("scanned_to")
            .and_then(|v| v.as_u64())
            .unwrap_or(from);
        let blocks = snap.get("blocks").and_then(|v| v.as_u64()).unwrap_or(0);
        let res = ok(snap, id.clone());
        writeln!(writer, "{}", serde_json::to_string(&res)?)?;
        writer.flush()?;

        if blocks > 0 && scanned_to >= from {
            from = scanned_to.saturating_add(1);
        }

        let notify = {
            let g = state.lock().unwrap();
            Arc::clone(&g.tip_notify)
        };
        let (lock, cv) = &*notify;
        let seen = lock.lock().map(|g| *g).unwrap_or(0);
        let Ok(guard) = lock.lock() else {
            break;
        };
        if *guard == seen {
            let (g, timeout) = cv
                .wait_timeout(guard, Duration::from_secs(30))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if timeout.timed_out() {
                let heartbeat = ok(
                    json!({
                        "from": from,
                        "scanned_to": from,
                        "blocks": 0,
                        "tip_height": state.lock().ok().and_then(|st| st.chain.tip_height().map(|h| h.0)),
                        "genesis": state.lock().ok().map(|st| st.chain.genesis_hash.to_hex()),
                        "outputs": [],
                        "spent": [],
                        "heartbeat": true,
                    }),
                    id.clone(),
                );
                writeln!(writer, "{}", serde_json::to_string(&heartbeat)?)?;
                writer.flush()?;
                drop(g);
                continue;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_addr("127.0.0.1:17881"));
        assert!(is_loopback_addr("[::1]:17881"));
        assert!(!is_loopback_addr("0.0.0.0:17881"));
        assert!(!is_loopback_addr("192.168.1.5:17881"));
        assert!(!is_loopback_addr("nonsense"));
    }
}
