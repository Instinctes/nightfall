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
use std::io::{self, BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
pub(crate) struct RpcReq {
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: serde_json::Value,
    #[serde(default)]
    pub(crate) id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct RpcRes {
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


 // PARADIGM CREATOR / RESOURCE SOVEREIGNTY
 // Memory, thread and wall-clock bounds belong at the accept/read boundary.
 // This is process protection, not consensus validation or RPC authentication.
 const MAX_RPC_CONNECTIONS: usize = 32;
 const MAX_RPC_STREAMS: usize = 8;
 const RPC_FRAME_BUDGET: Duration = Duration::from_secs(10);
 const RPC_STREAM_LIFETIME: Duration = Duration::from_secs(600);

 struct RpcSlot(Arc<AtomicUsize>);
 impl Drop for RpcSlot {
     fn drop(&mut self) {
         self.0.fetch_sub(1, Ordering::AcqRel);
     }
 }

 fn reserve(counter: &Arc<AtomicUsize>, limit: usize) -> Option<RpcSlot> {
     counter
         .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
             (n < limit).then_some(n + 1)
         })
         .ok()?;
     Some(RpcSlot(Arc::clone(counter)))
 }

 // A read_timeout alone is insufficient: an attacker could drip one byte
 // before each timeout. This function enforces a deadline across the ENTIRE
 // line while preserving already-buffered pipelined frames.
 fn read_rpc_frame(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Vec<u8>>> {
     let deadline = Instant::now() + RPC_FRAME_BUDGET;
     let mut frame = Vec::with_capacity(1024);
     loop {
         let remaining = deadline.saturating_duration_since(Instant::now());
         if remaining.is_zero() {
             return Err(io::Error::new(io::ErrorKind::TimedOut, "RPC frame deadline"));
         }
         reader.get_mut().set_read_timeout(Some(remaining))?;
         let (used, end) = {
             let available = reader.fill_buf()?;
             if available.is_empty() {
                 return if frame.is_empty() {
                     Ok(None)
                 } else {
                     Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated RPC frame"))
                 };
             }
             let used = available
                 .iter()
                 .position(|byte| *byte == b'\n')
                 .map_or(available.len(), |i| i + 1);
             if frame.len().saturating_add(used) > MAX_MESSAGE_BYTES {
                 return Err(io::Error::new(io::ErrorKind::InvalidData, "request too large"));
             }
             frame.extend_from_slice(&available[..used]);
             (used, frame.last() == Some(&b'\n'))
         };
         reader.consume(used);
         if end {
             return Ok(Some(frame));
         }
     }
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

    let active = Arc::new(AtomicUsize::new(0));
    let subscriptions = Arc::new(AtomicUsize::new(0));
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
                    // Reject BEFORE spawning: slow clients cannot create
                    // unbounded blocked threads on a small node.
                    let Some(slot) = reserve(&active, MAX_RPC_CONNECTIONS) else {
                        drop(stream);
                        continue;
                    };
                    let st = Arc::clone(&state);
                    let stream_budget = Arc::clone(&subscriptions);
                    thread::spawn(move || {
                        let _slot = slot;
                        if let Err(e) = handle_client(stream, st, stream_budget) {
                            tracing::debug!("rpc client: {e}");
                        }
                    });
                }
                Err(e) => tracing::warn!("rpc accept: {e}"),
            }
        }
    });
}

fn handle_client(
    stream: TcpStream,
    state: SharedState,
    subscriptions: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(RPC_FRAME_BUDGET))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    loop {
        let buf = match read_rpc_frame(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let res = err("request too large", json!(null));
                writeln!(writer, "{}", serde_json::to_string(&res)?)?;
                break;
            }
            Err(e) => return Err(e.into()),
        };

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
            // Streams may be long-lived, so they consume a SEPARATE,
            // smaller budget rather than starving ordinary wallet requests.
            let Some(_stream_slot) = reserve(&subscriptions, MAX_RPC_STREAMS) else {
                let res = err("too many RPC subscriptions", req.id.clone());
                writeln!(writer, "{}", serde_json::to_string(&res)?)?;
                break;
            };
            handle_scan_subscribe(&req, &state, &mut writer)?;
            break;
        }
        let res = dispatch(&req, &state);
        writeln!(writer, "{}", serde_json::to_string(&res)?)?;
    }
    Ok(())
}

pub(crate) fn dispatch(req: &RpcReq, state: &SharedState) -> RpcRes {
    let id = req.id.clone();

    match req.method.as_str() {
        "status" => {
            let g = state.lock().unwrap();
            let loading = g.loading;
            let chain = &g.chain;
            let supply_ok = !loading && chain.verify_supply().is_ok();
            let blocks = if loading {
                g.preview_blocks
            } else {
                chain.block_count()
            };
            let tip = if loading && !g.preview_tip.is_empty() {
                g.preview_tip.clone()
            } else {
                chain.tip_hash().to_hex()
            };
            let tip_height = if loading {
                g.preview_blocks.saturating_sub(1)
            } else {
                chain.tip_height().map(|h| h.0).unwrap_or(0)
            };
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
                    "blocks": blocks,
                    "tip_height": tip_height,
                    "tip": tip,
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
                    "tor_proxy": g.proxy.is_some()
                        && g.last_tor_ok.load(std::sync::atomic::Ordering::Relaxed),
                    "dandelion": true,
                    "loading": loading,
                    "last_dial_error": g.last_dial_error,
                    // Headless miners poll this. The GUI already had the
                    // same three numbers via StatusSnap; leaving them off
                    // the RPC meant a nightfalld box had no official way
                    // to see whether it was hashing.
                    "mining": g.mining_enabled.load(Ordering::SeqCst),
                    "hashes_total": g.hashes_total.load(Ordering::Relaxed),
                    "blocks_found": g.blocks_found.load(Ordering::Relaxed),
                    "tip_time": if loading {
                        0
                    } else {
                        chain
                            .headers
                            .last()
                            .map(|h| h.timestamp_unix)
                            .or_else(|| {
                                chain.blocks.last().map(|b| b.header.timestamp_unix)
                            })
                            .unwrap_or(0)
                    },
                    "stalled_on_fork": g.stalled_on_fork.load(Ordering::SeqCst),
                    "reorg_in_flight": g.reorg_in_flight.load(Ordering::SeqCst),
                    "best_peer_height": g.best_peer_height,
                    "fork_rewind": g.fork_rewind.load(Ordering::Relaxed),
                    "hashrate": if g.mining_enabled.load(Ordering::SeqCst) {
                        g.hashrate_hps.load(Ordering::Relaxed)
                    } else {
                        0
                    },
                    "mining_threads": g.mining_threads.load(Ordering::Relaxed),
                    // Why a switched-on miner is producing nothing. Until
                    // 0.8.4 "mining: true, hashrate: 0" had three different
                    // causes and looked the same from outside.
                    "mining_idle": match g.mining_idle_reason.load(Ordering::Relaxed) {
                        1 => "behind a peer",
                        2 => "no template",
                        _ => "",
                    },
                    "pruned": chain.is_pruned(),
                    "prune_height": chain.first_height,
                }),
                id,
            )
        }

        "peers" => {
            let g = state.lock().unwrap();
            ok(
                json!({
                    "peers": g.publishable_peers(),
                    "genesis": g.chain.genesis_hash.to_hex(),
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
            if state.lock().map(|g| g.loading).unwrap_or(false) {
                return err("chain is still loading from disk", id);
            }
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
            if state.lock().map(|g| g.loading).unwrap_or(false) {
                return err("chain is still loading from disk", id);
            }
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
            if from < g.chain.first_height {
                return err(
                    format!(
                        "this node is pruned; bodies start at height {}",
                        g.chain.first_height
                    ),
                    id,
                );
            }
            match serde_json::to_value(g.chain.blocks_from(from, limit)) {
                Ok(v) => ok(v, id),
                Err(e) => err(e.to_string(), id),
            }
        }

        // Block headers plus the three counts an observer can legitimately
        // see. Everything here is already public in the block: the amounts
        // stay in their commitments, the outputs stay unlinkable, and no
        // address exists on the chain to reveal in the first place.
        //
        // Separate from `get_blocks` on purpose. `get_blocks` ships full
        // bodies with range proofs — megabytes, and useless to a browser.
        // This is the shape a chain view actually wants, and it is small
        // enough to be safe on the public light API.
        "get_headers" => {
            const MAX_HEADERS: usize = 512;
            let g = state.lock().unwrap();
            let tip = g.chain.tip_height().map(|h| h.0).unwrap_or(0);
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .clamp(1, MAX_HEADERS as u64) as usize;
            // No `from` means "the newest ones", which is what a viewer opens
            // with and what saves it a round trip to learn the tip first.
            let from = match req.params.get("from").and_then(|v| v.as_u64()) {
                Some(f) => f,
                None => tip.saturating_sub(limit as u64 - 1),
            };
            let from = from.max(g.chain.first_height);

            let mut out = Vec::with_capacity(limit);
            for b in g.chain.blocks_from(from, limit) {
                out.push(json!({
                    "height": b.header.height.0,
                    "hash": b.hash().to_hex(),
                    "prev_hash": b.header.prev_hash.to_hex(),
                    "time": b.header.timestamp_unix,
                    "difficulty": b.header.difficulty,
                    "reward": b.header.reward_darks,
                    "utxo_root": b.header.utxo_root.to_hex(),
                    "inputs": b.body.inputs.len(),
                    "outputs": b.body.outputs.len(),
                    "kernels": b.body.kernels.len(),
                }));
            }
            // A pruned node has thrown the bodies away but kept every header.
            // Serve those rather than an empty list, with the counts left out
            // so nobody mistakes "not stored" for "zero".
            if out.is_empty() && !g.chain.headers.is_empty() {
                for h in g
                    .chain
                    .headers
                    .iter()
                    .filter(|h| h.height >= from)
                    .take(limit)
                {
                    out.push(json!({
                        "height": h.height,
                        "hash": h.hash.to_hex(),
                        "prev_hash": h.prev_hash.to_hex(),
                        "time": h.timestamp_unix,
                        "difficulty": h.difficulty,
                    }));
                }
            }
            ok(
                json!({
                    "tip_height": tip,
                    "first_height": g.chain.first_height,
                    "pruned": g.chain.is_pruned(),
                    "headers": out,
                }),
                id,
            )
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
                "timestamp": block.header.timestamp_unix,
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
        "pruned": g.chain.is_pruned(),
        "available_from": g.chain.first_height,
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
    // Bound live subscriptions too; clients reconnect after this limit.
    // A stale or malicious subscriber must not reserve a slot indefinitely.
    let deadline = Instant::now() + RPC_STREAM_LIFETIME;

    loop {
        if Instant::now() >= deadline {
            break;
        }
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
                .wait_timeout(
                    guard,
                    Duration::from_secs(30)
                        .min(deadline.saturating_duration_since(Instant::now())),
                )
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
    #[test]
    fn resource_slots_are_atomic_and_reclaimed() {
        let active = Arc::new(AtomicUsize::new(0));
        let a = reserve(&active, 2).expect("slot a");
        let b = reserve(&active, 2).expect("slot b");
        assert!(reserve(&active, 2).is_none());
        assert_eq!(active.load(Ordering::Acquire), 2);
        drop(a);
        let c = reserve(&active, 2).expect("reclaimed slot");
        drop(b);
        drop(c);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn framed_rpc_preserves_pipelining_and_rejects_truncated_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(port).unwrap();
            stream.write_all(b"{\"id\":1}\n{\"id\":2}\n").unwrap();
        });
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        assert_eq!(read_rpc_frame(&mut reader).unwrap(), Some(b"{\"id\":1}\n".to_vec()));
        assert_eq!(read_rpc_frame(&mut reader).unwrap(), Some(b"{\"id\":2}\n".to_vec()));
        assert_eq!(read_rpc_frame(&mut reader).unwrap(), None);
        sender.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(port).unwrap();
            stream.write_all(b"{\"id\":3}").unwrap();
        });
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        assert_eq!(read_rpc_frame(&mut reader).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        sender.join().unwrap();
    }

    #[test]
    fn overlong_frames_fail_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(port).unwrap();
            let chunk = vec![b'x'; 8_192];
            let mut remaining = MAX_MESSAGE_BYTES + 1;
            while remaining > 0 {
                let n = remaining.min(chunk.len());
                stream.write_all(&chunk[..n]).unwrap();
                remaining -= n;
            }
        });
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        assert_eq!(read_rpc_frame(&mut reader).unwrap_err().kind(), io::ErrorKind::InvalidData);
        sender.join().unwrap();
    }

}
