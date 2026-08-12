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
                    "peers": g.peer_addrs.len(),
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
