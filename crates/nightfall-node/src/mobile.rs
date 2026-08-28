//! Public light-client HTTP surface for phones.
//!
//! The full RPC is unauthenticated and includes `mine_one`. A phone must never
//! reach that. This listener answers only `status`, `scan_feed`, `submit_tx`,
//! `get_utxo_root`, `banner`, `peers` and `get_headers`.
//!
//! Plain HTTP. TLS belongs on a reverse proxy (Caddy / nginx) in front.

use crate::rpc::{self, RpcReq};
use crate::runtime::SharedState;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const ALLOWED: &[&str] = &[
    "status",
    "scan_feed",
    "submit_tx",
    "get_utxo_root",
    "banner",
    "peers",
    // Read-only, and small: headers plus input/output/kernel counts. Nothing
    // here is hidden by the protocol in the first place. `get_blocks` stays
    // off this list — full bodies with range proofs do not belong on a public
    // endpoint that anyone can hammer.
    "get_headers",
];
const MAX_BODY: usize = 512 * 1024;
const PER_IP_PER_MIN: u32 = 120;

struct Rate {
    window: Instant,
    count: u32,
}

pub fn spawn_mobile(addr: String, state: SharedState) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("mobile bind {addr}: {e}");
                return;
            }
        };
        tracing::info!("mobile API listening on {addr} (status/scan_feed/submit_tx only)");
        let rates: Arc<Mutex<HashMap<String, Rate>>> = Arc::new(Mutex::new(HashMap::new()));
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    let st = Arc::clone(&state);
                    let rates = Arc::clone(&rates);
                    thread::spawn(move || {
                        if let Err(e) = handle(stream, st, rates) {
                            tracing::debug!("mobile client: {e}");
                        }
                    });
                }
                Err(e) => tracing::warn!("mobile accept: {e}"),
            }
        }
    });
}

fn handle(
    mut stream: TcpStream,
    state: SharedState,
    rates: Arc<Mutex<HashMap<String, Rate>>>,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(Duration::from_secs(20)))?;
    let peer = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".into());
    if !allow(&rates, &peer) {
        return write_http(&mut stream, 429, "{\"error\":\"rate limited\"}");
    }

    let mut buf = vec![0u8; 16_384];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(());
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let (headers, rest) = head.split_once("\r\n\r\n").unwrap_or((&head, ""));
    let first = headers.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    if method == "OPTIONS" {
        return write_http(&mut stream, 204, "");
    }
    if method == "GET" && (path == "/peers" || path == "/peers/") {
        let req = RpcReq {
            method: "peers".into(),
            params: json!({}),
            id: json!(1),
        };
        let res = rpc::dispatch(&req, &state);
        return write_http(&mut stream, 200, &serde_json::to_string(&res)?);
    }
    if method == "GET" && (path == "/status" || path == "/") {
        let req = RpcReq {
            method: "status".into(),
            params: json!({}),
            id: json!(1),
        };
        let res = rpc::dispatch(&req, &state);
        return write_http(&mut stream, 200, &serde_json::to_string(&res)?);
    }
    if method != "POST" {
        return write_http(&mut stream, 405, "{\"error\":\"POST a JSON-RPC body\"}");
    }

    let content_len = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.eq_ignore_ascii_case("content-length")).then(|| v.trim().parse::<usize>().ok())
        })
        .flatten()
        .unwrap_or(rest.len());
    if content_len > MAX_BODY {
        return write_http(&mut stream, 413, "{\"error\":\"body too large\"}");
    }
    let mut body = rest.as_bytes().to_vec();
    while body.len() < content_len {
        let mut more = vec![0u8; (content_len - body.len()).min(8192)];
        let k = stream.read(&mut more)?;
        if k == 0 {
            break;
        }
        body.extend_from_slice(&more[..k]);
    }
    body.truncate(content_len);
    let text = String::from_utf8_lossy(&body);
    let req: RpcReq = match serde_json::from_str(text.trim()) {
        Ok(r) => r,
        Err(e) => {
            return write_http(
                &mut stream,
                400,
                &format!("{{\"error\":\"bad json: {e}\"}}"),
            );
        }
    };
    if !ALLOWED.contains(&req.method.as_str()) {
        return write_http(
            &mut stream,
            403,
            &format!(
                "{{\"error\":\"method '{}' is not available on the mobile API\"}}",
                req.method
            ),
        );
    }
    let res = rpc::dispatch(&req, &state);
    write_http(&mut stream, 200, &serde_json::to_string(&res)?)
}

fn allow(rates: &Mutex<HashMap<String, Rate>>, ip: &str) -> bool {
    let Ok(mut g) = rates.lock() else {
        return true;
    };
    let now = Instant::now();
    let e = g.entry(ip.to_string()).or_insert(Rate {
        window: now,
        count: 0,
    });
    if now.duration_since(e.window) > Duration::from_secs(60) {
        e.window = now;
        e.count = 0;
    }
    e.count += 1;
    e.count <= PER_IP_PER_MIN
}

fn write_http(stream: &mut TcpStream, code: u16, body: &str) -> anyhow::Result<()> {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let hdr = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(hdr.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ALLOWED;

    /// The public endpoint is an allow-list, and this test exists so that
    /// widening it stays a deliberate act. `get_blocks` in particular must
    /// never appear here: it serves full bodies with range proofs, which is
    /// megabytes per call and a free amplifier for anyone who asks twice.
    #[test]
    fn the_public_endpoint_serves_only_what_it_is_meant_to() {
        for m in [
            "status",
            "scan_feed",
            "submit_tx",
            "get_utxo_root",
            "banner",
            "peers",
            "get_headers",
        ] {
            assert!(ALLOWED.contains(&m), "{m} should be reachable");
        }
        for m in ["get_blocks", "mine_one", "verify_supply"] {
            assert!(!ALLOWED.contains(&m), "{m} must not be public");
        }
        assert_eq!(ALLOWED.len(), 7, "adding a method here needs a reason");
    }
}
