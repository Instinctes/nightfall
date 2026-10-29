//! Public light-client HTTP surface for phones.
//!
//! The full RPC is unauthenticated and includes `mine_one`. A phone must never
//! reach that. This listener answers only `status`, `scan_feed`, `submit_tx`,
//! `get_utxo_root`, `banner`, `peers` and `get_headers`.
//!
//! Plain HTTP. TLS belongs on a reverse proxy (Caddy / nginx) in front.

use crate::rpc::{self, RpcReq};
use crate::runtime::{SharedState, Shutdown, Workers};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
#[cfg(test)]
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
const WORKERS: usize = 8;
const QUEUED_CONNECTIONS: usize = 16;
const MAX_RATE_KEYS: usize = 4096;
const REQUEST_BUDGET: Duration = Duration::from_secs(20);

/// Fixed workers and a bounded queue prevent per-connection stack/allocator growth.
fn connection_pool(
    workers: usize,
    queued: usize,
    registry: Arc<Workers>,
    shutdown: Arc<Shutdown>,
    handle: impl Fn(TcpStream, Instant) + Send + Sync + 'static,
) -> std::io::Result<mpsc::SyncSender<(TcpStream, Instant)>> {
    let (tx, rx) = mpsc::sync_channel::<(TcpStream, Instant)>(queued);

    let rx = Arc::new(Mutex::new(rx));
    let handle = Arc::new(handle);

    for _index in 0..workers {
        let rx = Arc::clone(&rx);
        let handle = Arc::clone(&handle);
        let stop = Arc::clone(&shutdown);

        registry.spawn(move || loop {
            if stop.is_requested() {
                break;
            }

            let item = match rx.lock() {
                Ok(receiver) => receiver.recv_timeout(Duration::from_millis(100)),
                Err(_) => break,
            };

            match item {
                Ok((stream, accepted)) if accepted.elapsed() < REQUEST_BUDGET => {
                    if !stop.is_requested() {
                        handle(stream, accepted);
                    }
                }

                Ok(_) => {}

                Err(mpsc::RecvTimeoutError::Timeout) => {}

                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        });
    }

    Ok(tx)
}

struct Rate {
    window: Instant,
    count: u32,
}

pub(crate) fn spawn_mobile(
    addr: String,
    state: SharedState,
    shutdown: Arc<Shutdown>,
    workers: Arc<Workers>,
) {
    let client_workers = Arc::clone(&workers);

    workers.spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("mobile bind {addr}: {e}");
                return;
            }
        };

        if let Err(e) = listener.set_nonblocking(true) {
            tracing::error!("mobile nonblocking listener {addr}: {e}");
            return;
        }

        tracing::info!("mobile API listening on {addr}");

        let trust_proxy = std::env::var("NIGHTFALL_TRUST_LOOPBACK_PROXY").as_deref() == Ok("1");

        let rates: Arc<Mutex<HashMap<String, Rate>>> = Arc::new(Mutex::new(HashMap::new()));

        let handler_stop = Arc::clone(&shutdown);

        let queue = match connection_pool(
            WORKERS,
            QUEUED_CONNECTIONS,
            Arc::clone(&client_workers),
            Arc::clone(&shutdown),
            move |stream, accepted| {
                if handler_stop.is_requested() {
                    return;
                }

                if let Err(e) = handle(
                    stream,
                    Arc::clone(&state),
                    Arc::clone(&rates),
                    accepted,
                    trust_proxy,
                    Arc::clone(&handler_stop),
                ) {
                    tracing::debug!("mobile client: {e}");
                }
            },
        ) {
            Ok(q) => q,
            Err(e) => {
                tracing::error!("mobile workers: {e}");
                return;
            }
        };

        while !shutdown.is_requested() {
            match listener.accept() {
                Ok((stream, _)) => {
                    if shutdown.is_requested() {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        break;
                    }

                    // Normalize macOS accepted sockets.
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }

                    let _ = queue.try_send((stream, Instant::now()));
                }

                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    shutdown.wait_timeout(Duration::from_millis(100));
                }

                Err(e) => {
                    tracing::warn!("mobile accept: {e}");

                    shutdown.wait_timeout(Duration::from_millis(100));
                }
            }
        }

        drop(queue);
    });
}

fn handle(
    mut stream: TcpStream,
    state: SharedState,
    rates: Arc<Mutex<HashMap<String, Rate>>>,
    accepted: Instant,
    trust_proxy: bool,
    shutdown: Arc<Shutdown>,
) -> anyhow::Result<()> {
    if shutdown.is_requested() {
        return Ok(());
    }

    stream.set_nonblocking(false)?;

    let _pending_socket = shutdown.track_socket(&stream)?;

    stream.set_write_timeout(Some(REQUEST_BUDGET))?;
    let peer = stream.peer_addr().map(|a| a.ip()).ok();
    let deadline = accepted + REQUEST_BUDGET;
    let (headers, mut body) = read_headers(&mut stream, deadline)?;
    if !allow(&rates, &rate_key(peer, &headers, trust_proxy)) {
        return write_http(&mut stream, 429, "{\"error\":\"rate limited\"}");
    }
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

    let content_len = match body_length(&headers) {
        Ok(n) if n <= MAX_BODY => n,
        Ok(_) => return write_http(&mut stream, 413, "{\"error\":\"body too large\"}"),
        Err(_) => return write_http(&mut stream, 400, "{\"error\":\"invalid HTTP framing\"}"),
    };
    body.truncate(content_len);
    while body.len() < content_len {
        let mut more = [0u8; 8192];
        let wanted = (content_len - body.len()).min(more.len());
        let k = read_before(&mut stream, &mut more[..wanted], deadline)?;
        anyhow::ensure!(k > 0, "truncated HTTP body");
        body.extend_from_slice(&more[..k]);
    }
    let text = std::str::from_utf8(&body)?;
    let req: RpcReq = match serde_json::from_str(text.trim()) {
        Ok(r) => r,
        Err(e) => {
            return write_http(
                &mut stream,
                400,
                &json!({"error": format!("bad json: {e}")}).to_string(),
            );
        }
    };
    if !ALLOWED.contains(&req.method.as_str()) {
        return write_http(
            &mut stream,
            403,
            &json!({"error": format!("method '{}' is not available on the mobile API", req.method)}).to_string(),
        );
    }
    let res = rpc::dispatch(&req, &state);
    write_http(&mut stream, 200, &serde_json::to_string(&res)?)
}

fn read_before(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> anyhow::Result<usize> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| anyhow::anyhow!("request deadline expired"))?;
    stream.set_read_timeout(Some(remaining))?;
    Ok(stream.read(buf)?)
}

fn read_headers(stream: &mut TcpStream, deadline: Instant) -> anyhow::Result<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    loop {
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            anyhow::ensure!(end + 4 <= 16384, "headers too large");
            let headers = std::str::from_utf8(&bytes[..end])?.to_owned();
            return Ok((headers, bytes[end + 4..].to_vec()));
        }
        anyhow::ensure!(bytes.len() < 16384, "headers too large");
        let mut buf = [0u8; 4096];
        let n = read_before(stream, &mut buf, deadline)?;
        anyhow::ensure!(n > 0, "truncated HTTP headers");
        bytes.extend_from_slice(&buf[..n]);
    }
}

fn body_length(headers: &str) -> anyhow::Result<usize> {
    let mut length = None;
    for line in headers.lines().skip(1) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid header"))?;
        anyhow::ensure!(
            !name.eq_ignore_ascii_case("transfer-encoding"),
            "chunked bodies unsupported"
        );
        if name.eq_ignore_ascii_case("content-length") {
            anyhow::ensure!(length.is_none(), "duplicate content length");
            let value = value.trim();
            anyhow::ensure!(
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                "invalid content length"
            );
            length = Some(value.parse()?);
        }
    }
    length.ok_or_else(|| anyhow::anyhow!("content length required"))
}

/// Whose budget a request is charged to.
///
/// Normally the socket's address. The one exception is a reverse proxy on the
/// same machine (someone self-hosting the web wallet in front of their own
/// node): every request then arrives from loopback, and without this all of
/// that proxy's users would share one 120-per-minute bucket — a single fresh
/// wallet scan would lock everyone else out. So when, and only when, the
/// socket peer is loopback AND the operator opted in using
/// NIGHTFALL_TRUST_LOOPBACK_PROXY=1, the last `X-Forwarded-For` entry (the one the
/// local proxy itself appended) names the client. A remote peer cannot use
/// the header to pick someone else's bucket or to get a fresh one: from
/// anywhere but loopback it is ignored.
fn rate_key(peer: Option<std::net::IpAddr>, headers: &str, trust_proxy: bool) -> String {
    let Some(peer) = peer else {
        return "unknown".into();
    };
    if trust_proxy && peer.is_loopback() {
        let forwarded = headers.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-forwarded-for")
                .then_some(v)
        });
        if let Some(ip) = forwarded
            .and_then(|v| v.rsplit(',').next())
            .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok())
        {
            return format!("fwd:{ip}");
        }
    }
    peer.to_string()
}

fn allow(rates: &Mutex<HashMap<String, Rate>>, ip: &str) -> bool {
    allow_at(rates, ip, Instant::now())
}

fn allow_at(rates: &Mutex<HashMap<String, Rate>>, ip: &str, now: Instant) -> bool {
    let Ok(mut g) = rates.lock() else {
        return false;
    };
    // Bounded table; expiry runs before admission, including for new addresses.
    g.retain(|_, rate| now.saturating_duration_since(rate.window) < Duration::from_secs(60));
    if !g.contains_key(ip) && g.len() >= MAX_RATE_KEYS {
        return false;
    }
    let e = g.entry(ip.to_string()).or_insert(Rate {
        window: now,
        count: 0,
    });
    if now.duration_since(e.window) > Duration::from_secs(60) {
        e.window = now;
        e.count = 0;
    }
    e.count = e.count.saturating_add(1);
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
    use super::{rate_key, ALLOWED};
    use std::net::IpAddr;

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn rate_table_is_bounded_expires_and_never_overflows() {
        use super::*;
        let rates = Mutex::new(HashMap::new());
        let now = Instant::now();
        for n in 0..MAX_RATE_KEYS {
            assert!(allow_at(&rates, &n.to_string(), now));
        }
        assert!(!allow_at(&rates, "extra", now));
        for _ in 1..PER_IP_PER_MIN {
            assert!(allow_at(&rates, "0", now));
        }
        assert!(!allow_at(&rates, "0", now));
        rates.lock().unwrap().get_mut("0").unwrap().count = u32::MAX;
        assert!(!allow_at(&rates, "0", now));
        assert!(allow_at(&rates, "extra", now + Duration::from_secs(60)));
        assert_eq!(rates.lock().unwrap().len(), 1);
    }

    #[test]
    fn http_framing_refuses_ambiguous_lengths_and_chunking() {
        use super::body_length;
        assert_eq!(
            body_length("POST / HTTP/1.1\r\nContent-Length: 12").unwrap(),
            12
        );
        for headers in [
            "",
            "Content-Length: 1\r\nContent-Length: 1",
            "Transfer-Encoding: chunked\r\nContent-Length: 1",
            "Content-Length: -1",
            "Content-Length: +1",
            "Content-Length: abc",
        ] {
            assert!(body_length(&format!("POST / HTTP/1.1\r\n{headers}")).is_err());
        }
    }

    #[test]
    fn split_http_headers_and_expired_deadlines_are_handled() {
        use super::*;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let writer = thread::spawn(move || {
            client.write_all(b"POST / HTTP/1.1\r\nContent-Len").unwrap();
            thread::sleep(Duration::from_millis(20));
            client.write_all(b"gth: 2\r\n\r\n{}").unwrap();
        });
        let (header, _) =
            read_headers(&mut server, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(body_length(&header).unwrap(), 2);
        writer.join().unwrap();
        assert!(read_before(&mut server, &mut [0u8; 1], Instant::now()).is_err());
    }

    #[test]
    fn worker_pool_reuses_workers_and_rejects_excess_sockets() {
        use super::*;
        let (entered, entries) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let releases = Mutex::new(releases);
        let pool = connection_pool(
            1,
            1,
            Arc::new(Workers::new()),
            Arc::new(Shutdown::default()),
            move |_, _| {
                entered.send(thread::current().id()).unwrap();
                releases.lock().unwrap().recv().unwrap();
            },
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = || {
            let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (server, _) = listener.accept().unwrap();
            drop(client);
            server
        };
        pool.try_send((socket(), Instant::now())).unwrap();
        let first = entries.recv_timeout(Duration::from_secs(2)).unwrap();
        pool.try_send((socket(), Instant::now())).unwrap();
        assert!(matches!(
            pool.try_send((socket(), Instant::now())),
            Err(mpsc::TrySendError::Full(_))
        ));
        release.send(()).unwrap();
        assert_eq!(entries.recv_timeout(Duration::from_secs(2)).unwrap(), first);
        release.send(()).unwrap();
        drop(pool);
    }

    /// Behind a local reverse proxy each user gets their own budget; from
    /// anywhere else the header is worthless, so nobody can dodge the limit
    /// or spend someone else's by writing it.
    #[test]
    fn only_a_local_proxy_may_say_who_the_client_is() {
        let h = "POST / HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: 203.0.113.9, 198.51.100.4";
        assert_eq!(rate_key(ip("127.0.0.1"), h, false), "127.0.0.1");
        assert_eq!(rate_key(ip("127.0.0.1"), h, true), "fwd:198.51.100.4");
        assert_eq!(rate_key(ip("::1"), h, true), "fwd:198.51.100.4");
        assert_eq!(rate_key(ip("192.0.2.7"), h, true), "192.0.2.7");
        let lower = "POST / HTTP/1.1\r\nx-forwarded-for:  2001:db8::5 ";
        assert_eq!(rate_key(ip("127.0.0.1"), lower, true), "fwd:2001:db8::5");
        // Garbage in the header falls back to the socket, never to a free bucket.
        let junk = "POST / HTTP/1.1\r\nX-Forwarded-For: not-an-ip";
        assert_eq!(rate_key(ip("127.0.0.1"), junk, true), "127.0.0.1");
        assert_eq!(
            rate_key(ip("127.0.0.1"), "POST / HTTP/1.1", true),
            "127.0.0.1"
        );
        // The request line itself is never read as a header.
        assert_eq!(
            rate_key(ip("127.0.0.1"), "X-Forwarded-For: 1.2.3.4", true),
            "127.0.0.1"
        );
        assert_eq!(rate_key(None, h, true), "unknown");
    }

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
