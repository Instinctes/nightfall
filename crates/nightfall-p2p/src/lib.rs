//! Nightfall P2P: newline-delimited JSON messages over TCP.

mod socks;
pub use socks::{looks_like_dial_target, SocksProxy};

use nightfall_consensus::Block;
use nightfall_ledger::Transaction;
use nightfall_types::{Hash256, NetworkId, MAX_MESSAGE_BYTES, WIRE_VERSION};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Never request more than this many blocks in one round trip.
pub const MAX_BLOCKS_PER_REQUEST: usize = 128;

/// Cap on addresses accepted from a single peer exchange.
pub const MAX_PEERS_PER_MSG: usize = 32;

/// Combine an observed source IP with the port a peer advertised.
///
/// Returns `None` for anything that is not a usable dial target.
pub fn dialable_addr(observed: &str, advertised_port: u16) -> Option<String> {
    if advertised_port == 0 {
        return None;
    }
    let sock: std::net::SocketAddr = observed.parse().ok()?;
    let ip = sock.ip();
    if ip.is_unspecified() || ip.is_multicast() {
        return None;
    }
    Some(match ip {
        std::net::IpAddr::V4(v4) => format!("{v4}:{advertised_port}"),
        std::net::IpAddr::V6(v6) => format!("[{v6}]:{advertised_port}"),
    })
}

/// An address we are willing to hand to a stranger as a place to dial.
///
/// Hostnames (compiled-in seeds) pass. Literal IPs must be globally routable
/// and not a documented reserved block. `.onion` stays off this list — a
/// browser or a clearnet wallet cannot use it, and publishing one leaks that
/// the node is reachable only through Tor.
pub fn is_directory_addr(addr: &str) -> bool {
    let addr = addr.trim();
    if addr.is_empty() || !looks_like_dial_target(addr) {
        return false;
    }
    if addr.contains(".onion") {
        return false;
    }
    // Hostname: seed.example:17891 — not an IP, so the directory may name it.
    if let Ok(sock) = addr.parse::<std::net::SocketAddr>() {
        return ip_is_globally_reachable(sock.ip()) && sock.port() != 0;
    }
    if let Some(rest) = addr.strip_prefix('[') {
        if let Some((host, _)) = rest.split_once("]:") {
            if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
                return ip_is_globally_reachable(std::net::IpAddr::V6(ip));
            }
        }
        return false;
    }
    let Some((host, _)) = addr.rsplit_once(':') else {
        return false;
    };
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return host
            .parse::<std::net::Ipv4Addr>()
            .map(|ip| ip_is_globally_reachable(std::net::IpAddr::V4(ip)))
            .unwrap_or(false);
    }
    // DNS name. Refuse empty labels and raw IPs that failed above.
    !host.is_empty()
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn ip_is_globally_reachable(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            !v4.is_unspecified()
                && !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_multicast()
                && !v4.is_documentation()
                && (v4.octets()[0] != 100 || v4.octets()[1] & 0xc0 != 0x40) // 100.64/10
        }
        std::net::IpAddr::V6(v6) => {
            !v6.is_unspecified()
                && !v6.is_loopback()
                && !v6.is_multicast()
                && !is_unique_local_v6(v6)
        }
    }
}

fn is_unique_local_v6(ip: std::net::Ipv6Addr) -> bool {
    // fc00::/7
    ip.octets()[0] & 0xfe == 0xfc
}

pub fn network_magic(network: NetworkId) -> [u8; 4] {
    match network {
        NetworkId::Mainnet => *b"NFL2",
        NetworkId::Testnet => *b"NFT2",
        NetworkId::Devnet => *b"NFD2",
    }
}

pub fn default_listen_addr(network: NetworkId) -> String {
    format!("0.0.0.0:{}", network.default_p2p_port())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeerMsg {
    Hello {
        wire: u32,
        network: NetworkId,
        genesis: String,
        height: u64,
        tip: String,
        agent: String,
        /// Port this node accepts connections on.
        ///
        /// Only the port: the address a peer should dial is this port combined
        /// with the source IP they observed us connecting from. Advertising a
        /// full address would mean advertising `0.0.0.0`, and recording the
        /// inbound *ephemeral* source port instead would give an address that
        /// stops working the moment the connection closes — which is why block
        /// propagation used to be one-directional.
        #[serde(default)]
        listen_port: u16,
        /// Lowest block body this node still holds, and whether it pruned.
        ///
        /// A pruned node validates every new block and proves the supply
        /// invariant exactly like an archive — but it cannot answer
        /// `GetBlocks` below `first_height`. Without saying so in the
        /// handshake the only way to find that out is to ask and receive an
        /// empty answer, which the sync loop cannot distinguish from "the
        /// chain ends here" and treats as a reason to stop. A network of
        /// pruned nodes would then be unable to bootstrap anyone new.
        ///
        /// `serde(default)` on purpose: a node built before these fields
        /// simply omits them and is read as an archive at height 0, which is
        /// what it is. No wire version bump, nothing to coordinate.
        #[serde(default)]
        pruned: bool,
        #[serde(default)]
        first_height: u64,
    },
    HelloOk {
        wire: u32,
        network: NetworkId,
        genesis: String,
        height: u64,
        tip: String,
        #[serde(default)]
        listen_port: u16,
        /// Lowest block body this node still holds, and whether it pruned.
        ///
        /// A pruned node validates every new block and proves the supply
        /// invariant exactly like an archive — but it cannot answer
        /// `GetBlocks` below `first_height`. Without saying so in the
        /// handshake the only way to find that out is to ask and receive an
        /// empty answer, which the sync loop cannot distinguish from "the
        /// chain ends here" and treats as a reason to stop. A network of
        /// pruned nodes would then be unable to bootstrap anyone new.
        ///
        /// `serde(default)` on purpose: a node built before these fields
        /// simply omits them and is read as an archive at height 0, which is
        /// what it is. No wire version bump, nothing to coordinate.
        #[serde(default)]
        pruned: bool,
        #[serde(default)]
        first_height: u64,
    },
    /// Ask a peer for the addresses it knows.
    GetPeers,
    Peers {
        addrs: Vec<String>,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    GetBlocks {
        from_height: u64,
        limit: usize,
    },
    Blocks {
        blocks: Vec<Block>,
    },
    InvBlock {
        hash: String,
        height: u64,
    },
    GetBlock {
        hash: String,
    },
    Block {
        block: Block,
    },
    Tx {
        tx: Transaction,
    },
    GetStatus,
    Status {
        height: u64,
        tip: String,
        bits: u32,
        peers: usize,
        mempool: usize,
    },
    Error {
        message: String,
    },
}

/// JSON payload budget. `read_msg` counts the trailing newline against
/// `MAX_MESSAGE_BYTES`, so the serialized JSON itself gets one byte less.
const MAX_WIRE_PAYLOAD_BYTES: usize = MAX_MESSAGE_BYTES - 1;

/// Serialization sink that refuses to grow past the wire limit.
///
/// This matters on the sending side too: serializing an oversized peer message
/// into an ordinary `String` first defeats the receive-side allocation bound.
struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(4096.min(limit)),
            limit,
        }
    }
}

impl std::io::Write for BoundedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.bytes.len().checked_add(buf.len()).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "peer message size overflow",
            )
        })?;

        if next > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("peer message exceeds {MAX_MESSAGE_BYTES} bytes"),
            ));
        }

        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encode_wire_line<T: Serialize + ?Sized>(value: &T) -> std::io::Result<Vec<u8>> {
    let mut out = BoundedBuffer::new(MAX_WIRE_PAYLOAD_BYTES);
    serde_json::to_writer(&mut out, value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    out.bytes.push(b'\n');
    Ok(out.bytes)
}

fn fits_wire<T: Serialize + ?Sized>(value: &T) -> bool {
    let mut out = BoundedBuffer::new(MAX_WIRE_PAYLOAD_BYTES);
    serde_json::to_writer(&mut out, value).is_ok()
}

/// Borrowed representation of the `Blocks` wire message, avoiding clones while
/// measuring candidate prefixes.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BorrowedPeerMsg<'a> {
    Blocks { blocks: &'a [Block] },
}

fn largest_fitting_prefix<F>(len: usize, mut fits: F) -> usize
where
    F: FnMut(usize) -> bool,
{
    let mut low = 0usize;
    let mut high = len;

    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    low
}

/// Trim a `Blocks` response to the largest prefix that can be represented as
/// one legal P2P frame.
///
/// Returning an empty batch for a non-empty input would falsely mean "chain
/// ends here", so a single unrepresentable block is an explicit error instead.
pub fn fit_blocks_response(mut blocks: Vec<Block>) -> std::io::Result<Vec<Block>> {
    if blocks.is_empty() {
        return Ok(blocks);
    }

    let keep = largest_fitting_prefix(blocks.len(), |n| {
        fits_wire(&BorrowedPeerMsg::Blocks {
            blocks: &blocks[..n],
        })
    });

    if keep == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "single block exceeds the P2P message limit",
        ));
    }

    blocks.truncate(keep);
    Ok(blocks)
}

pub fn write_msg(stream: &mut TcpStream, msg: &PeerMsg) -> std::io::Result<()> {
    // Serialize completely inside the bounded buffer before touching the
    // socket. An oversized message therefore cannot leave a partial frame.
    let line = encode_wire_line(msg)?;
    stream.write_all(&line)?;
    stream.flush()?;
    Ok(())
}

/// Read one message, refusing anything larger than [`MAX_MESSAGE_BYTES`].
///
/// v4 used an unbounded `read_line`, so any peer could stream gigabytes into a
/// `String` and kill the node with an allocation failure (audit finding N-06).
pub fn read_msg(reader: &mut BufReader<TcpStream>) -> std::io::Result<PeerMsg> {
    let mut limited = reader.take(MAX_MESSAGE_BYTES as u64 + 1);
    let mut buf = Vec::with_capacity(4096);
    let n = limited.read_until(b'\n', &mut buf)?;

    if n == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "peer closed",
        ));
    }
    if n > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("peer message exceeds {MAX_MESSAGE_BYTES} bytes — disconnecting"),
        ));
    }
    if buf.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message truncated or oversized",
        ));
    }

    let text = std::str::from_utf8(&buf)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 message"))?;
    serde_json::from_str(text.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// Local Tor SOCKS by default. Opt out with `--proxy off`.
pub const DEFAULT_TOR_PROXY: &str = "127.0.0.1:9050";

/// The one switch for fail-closed outbound privacy: `NIGHTFALL_PRIVACY_LOCKDOWN`.
///
/// Off by default. When set, every outbound P2P connection must go through
/// SOCKS5 (no direct fallback when Tor is down), the direct HTTPS peer
/// directory is skipped, and Core's "check against nightfallcoin.org" is
/// refused. Unknown values enable rather than weaken protection: a typo must
/// not silently allow clearnet.
fn privacy_flag_enabled(raw: Option<&std::ffi::OsStr>) -> bool {
    let Some(value) = raw else {
        return false;
    };

    let value = value.to_string_lossy();

    !value.is_empty()
        && !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        )
}

pub fn privacy_lockdown_enabled() -> bool {
    let value = std::env::var_os("NIGHTFALL_PRIVACY_LOCKDOWN");
    privacy_flag_enabled(value.as_deref())
}

pub fn connect_peer(addr: &str, timeout_ms: u64) -> std::io::Result<TcpStream> {
    connect_peer_via(addr, timeout_ms, None).map(|(s, _)| s)
}

/// Dial `addr`, optionally through SOCKS5.
///
/// Returns `(stream, used_tor)`. By default a clearnet destination falls back
/// to a direct connect when the proxy is down; under
/// [`privacy_lockdown_enabled`] it fails instead. `.onion` never falls back —
/// that would leak the name to DNS.
pub fn connect_peer_via(
    addr: &str,
    timeout_ms: u64,
    proxy: Option<&SocksProxy>,
) -> std::io::Result<(TcpStream, bool)> {
    connect_peer_via_policy(addr, timeout_ms, proxy, privacy_lockdown_enabled())
}

fn connect_peer_via_policy(
    addr: &str,
    timeout_ms: u64,
    proxy: Option<&SocksProxy>,
    require_proxy: bool,
) -> std::io::Result<(TcpStream, bool)> {
    if require_proxy && proxy.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "outbound P2P requires SOCKS5; no proxy configured",
        ));
    }

    let onion = addr.contains(".onion");

    if let Some(p) = proxy {
        match p.connect(addr, timeout_ms) {
            Ok(s) => return Ok((s, true)),
            Err(e) if onion || require_proxy => {
                return Err(e);
            }
            Err(_) => {}
        }
    }

    if onion {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "onion destinations require SOCKS5",
        ));
    }

    Ok((connect_direct(addr, timeout_ms)?, false))
}

pub(crate) fn connect_direct(addr: &str, timeout_ms: u64) -> std::io::Result<TcpStream> {
    use std::net::ToSocketAddrs;

    // `TcpStream::connect` has no timeout of its own: a dead address waits
    // for the OS SYN retry (often more than a minute). The sync loop joins
    // every peer thread before the next round, so one hung connect used to
    // stall the whole network — the same class of fault as walking peers
    // sequentially, just hiding behind a thread.
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let mut last_err: Option<std::io::Error> = None;
    let mut any = false;
    for sock in addr.to_socket_addrs()? {
        any = true;
        match TcpStream::connect_timeout(&sock, timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            if any {
                std::io::ErrorKind::TimedOut
            } else {
                std::io::ErrorKind::NotFound
            },
            format!("{addr} did not accept a connection"),
        )
    }))
}

/// What the other end told us about itself once the handshake succeeded.
#[derive(Clone, Debug)]
pub struct PeerIntro {
    pub height: u64,
    pub tip: String,
    /// True when the peer discarded old block bodies. Such a peer is a fine
    /// relay and a useless source for initial block download.
    pub pruned: bool,
    /// Lowest height whose body the peer can still serve. 0 for an archive.
    pub first_height: u64,
}

impl PeerIntro {
    /// Can this peer answer for the whole chain?
    pub fn is_archive(&self) -> bool {
        !self.pruned && self.first_height == 0
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handshake(
    stream: &mut TcpStream,
    network: NetworkId,
    genesis: Hash256,
    height: u64,
    tip: Hash256,
    listen_port: u16,
    pruned: bool,
    first_height: u64,
) -> std::io::Result<PeerIntro> {
    write_msg(
        stream,
        &PeerMsg::Hello {
            wire: WIRE_VERSION,
            network,
            genesis: genesis.to_hex(),
            height,
            tip: tip.to_hex(),
            agent: format!("nightfalld/{}", env!("CARGO_PKG_VERSION")),
            listen_port,
            pruned,
            first_height,
        },
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    match read_msg(&mut reader)? {
        PeerMsg::HelloOk {
            wire,
            network: net,
            genesis: g,
            height: h,
            tip: t,
            pruned: peer_pruned,
            first_height: peer_first,
            ..
        } => {
            if wire != WIRE_VERSION {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "wire version mismatch — peer speaks v{wire}, we speak \
                         v{WIRE_VERSION}. One of us needs the current release \
                         from https://nightfallcoin.org"
                    ),
                ));
            }
            if net != network {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "network mismatch",
                ));
            }
            if g != genesis.to_hex() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "genesis mismatch",
                ));
            }
            Ok(PeerIntro {
                height: h,
                tip: t,
                pruned: peer_pruned,
                first_height: peer_first,
            })
        }
        PeerMsg::Error { message } => Err(std::io::Error::other(message)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected hello_ok",
        )),
    }
}

/// Most unrelated messages tolerated while waiting for one `Blocks` reply.
const MAX_INTERLEAVED: usize = 64;

pub fn request_blocks(
    stream: &mut TcpStream,
    from_height: u64,
    limit: usize,
) -> std::io::Result<Vec<Block>> {
    let target = limit.min(MAX_BLOCKS_PER_REQUEST);
    if target == 0 {
        return Ok(Vec::new());
    }

    let mut all = Vec::with_capacity(target);
    let mut next_height = from_height;

    // Preserve buffered frames between paginated requests.
    let mut reader = BufReader::new(stream.try_clone()?);

    while all.len() < target {
        let remaining = target - all.len();

        write_msg(
            stream,
            &PeerMsg::GetBlocks {
                from_height: next_height,
                limit: remaining,
            },
        )?;

        // Preserve v1.0.6 interleaved-message recovery.
        let mut received = None;

        for _ in 0..MAX_INTERLEAVED {
            match read_msg(&mut reader) {
                Ok(PeerMsg::Blocks { blocks }) => {
                    received = Some(blocks);
                    break;
                }
                Ok(PeerMsg::Error { message }) => {
                    return Err(std::io::Error::other(message));
                }
                Ok(_) => continue,
                Err(e) if is_unparseable_line(&e) => continue,
                Err(e) => return Err(e),
            }
        }

        let batch = received.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "expected blocks")
        })?;

        if batch.is_empty() {
            break;
        }

        if batch.len() > remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "peer returned more blocks than requested",
            ));
        }

        for (offset, block) in batch.iter().enumerate() {
            let expected = next_height.checked_add(offset as u64).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "block response height overflow",
                )
            })?;

            if block.header.height.0 != expected {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "unexpected block height {}, expected {expected}",
                        block.header.height.0,
                    ),
                ));
            }
        }

        let count = batch.len();
        all.extend(batch);

        if all.len() >= target {
            break;
        }

        next_height = match next_height.checked_add(count as u64) {
            Some(v) => v,
            None => break,
        };
    }

    Ok(all)
}

/// A complete line that is not a message we understand — as opposed to a
/// closed, truncated or oversized stream, which must still fail.
fn is_unparseable_line(e: &std::io::Error) -> bool {
    if e.kind() != std::io::ErrorKind::InvalidData {
        return false;
    }
    let text = e.to_string();
    !(text.contains("exceeds") || text.contains("truncated") || text.contains("non-utf8"))
}

pub fn broadcast_block(stream: &mut TcpStream, block: &Block) -> std::io::Result<()> {
    write_msg(
        stream,
        &PeerMsg::InvBlock {
            hash: block.hash().to_hex(),
            height: block.header.height.0,
        },
    )?;
    write_msg(
        stream,
        &PeerMsg::Block {
            block: block.clone(),
        },
    )
}

pub fn broadcast_tx(stream: &mut TcpStream, tx: &Transaction) -> std::io::Result<()> {
    write_msg(stream, &PeerMsg::Tx { tx: tx.clone() })
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn seeds_and_public_ips_are_publishable() {
        assert!(is_directory_addr("seed.nightfallcoin.org:17891"));
        assert!(is_directory_addr("8.8.8.8:17891"));
        assert!(is_directory_addr("[2001:4860:4860::8888]:17891"));
    }

    #[test]
    fn private_onion_and_garbage_are_not() {
        assert!(!is_directory_addr("127.0.0.1:17891"));
        assert!(!is_directory_addr("10.0.0.1:17891"));
        assert!(!is_directory_addr("192.168.1.9:17891"));
        assert!(!is_directory_addr("100.64.1.1:17891"));
        assert!(!is_directory_addr("abcd.onion:17891"));
        assert!(!is_directory_addr("no-port"));
        assert!(!is_directory_addr(""));
    }

    #[test]
    fn outbound_wire_encoding_rejects_oversized_messages() {
        let msg = PeerMsg::Error {
            message: "x".repeat(MAX_MESSAGE_BYTES),
        };

        let err = encode_wire_line(&msg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn fitting_prefix_search_returns_largest_valid_prefix() {
        assert_eq!(largest_fitting_prefix(128, |n| n <= 37), 37);
        assert_eq!(largest_fitting_prefix(128, |_| true), 128);
        assert_eq!(largest_fitting_prefix(128, |_| false), 0);
    }

    #[test]
    fn empty_blocks_response_remains_empty() {
        assert!(fit_blocks_response(Vec::new()).unwrap().is_empty());
    }
}

#[cfg(test)]
mod interleave_tests {
    use super::*;
    use std::net::TcpListener;

    /// Issue #40: unsolicited session traffic arrives before the reply.
    #[test]
    fn a_blocks_reply_is_found_behind_unrelated_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            // Wait for the request, then answer the way a real node's session
            // can: address book, status probe, half a line, then the reply.
            let request = read_msg(&mut reader).unwrap();
            assert!(matches!(request, PeerMsg::GetBlocks { from_height: 7, .. }));
            write_msg(
                &mut sock,
                &PeerMsg::Peers {
                    addrs: vec!["1.2.3.4:17891".into()],
                },
            )
            .unwrap();
            write_msg(&mut sock, &PeerMsg::GetStatus).unwrap();
            sock.write_all(b"\"tail of a line\"}\n").unwrap();
            write_msg(&mut sock, &PeerMsg::Blocks { blocks: vec![] }).unwrap();
        });
        let mut client = TcpStream::connect(addr).unwrap();
        let blocks = request_blocks(&mut client, 7, 1).unwrap();
        assert!(blocks.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn a_closed_stream_still_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let _ = read_msg(&mut reader);
            drop(sock);
        });
        let mut client = TcpStream::connect(addr).unwrap();
        assert!(request_blocks(&mut client, 7, 1).is_err());
        server.join().unwrap();
    }
}

#[cfg(test)]
mod proxy_fail_closed_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn strict_without_proxy_never_reaches_target() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let addr = target.local_addr().unwrap().to_string();

        let err = connect_peer_via_policy(&addr, 300, None, true).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn dead_proxy_never_triggers_strict_fallback() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let addr = target.local_addr().unwrap().to_string();

        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = SocksProxy::parse(&dead.local_addr().unwrap().to_string()).unwrap();
        drop(dead);

        assert!(connect_peer_via_policy(&addr, 300, Some(&proxy), true).is_err());
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn legacy_mode_retains_explicit_fallback() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = target.local_addr().unwrap().to_string();

        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = SocksProxy::parse(&dead.local_addr().unwrap().to_string()).unwrap();
        drop(dead);

        let (_stream, used_tor) = connect_peer_via_policy(&addr, 300, Some(&proxy), false).unwrap();

        assert!(!used_tor);
        target.accept().unwrap();
    }

    #[test]
    fn strict_live_proxy_uses_socks() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = SocksProxy::parse(&listener.local_addr().unwrap().to_string()).unwrap();

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();

            let mut greeting = [0u8; 3];
            socket.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&[5, 0]).unwrap();

            let mut header = [0u8; 4];
            socket.read_exact(&mut header).unwrap();
            assert_eq!(header, [5, 1, 0, 3]);

            let mut len = [0u8; 1];
            socket.read_exact(&mut len).unwrap();

            let mut rest = vec![0u8; len[0] as usize + 2];
            socket.read_exact(&mut rest).unwrap();

            assert_eq!(&rest[..len[0] as usize], b"seed.example");

            socket.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        });

        let (stream, used_tor) =
            connect_peer_via_policy("seed.example:17891", 2_000, Some(&proxy), true).unwrap();

        assert!(used_tor);
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    fn strict_connect_rejection_never_dials_target() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let addr = target.local_addr().unwrap().to_string();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = SocksProxy::parse(&listener.local_addr().unwrap().to_string()).unwrap();

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();

            let mut greeting = [0u8; 3];
            socket.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&[5, 0]).unwrap();

            let mut header = [0u8; 4];
            socket.read_exact(&mut header).unwrap();
            assert_eq!(header, [5, 1, 0, 1]);

            let mut tail = [0u8; 6];
            socket.read_exact(&mut tail).unwrap();

            socket.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        });

        assert!(connect_peer_via_policy(&addr, 2_000, Some(&proxy), true).is_err());
        server.join().unwrap();

        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn onion_never_uses_clearnet() {
        assert!(connect_peer_via_policy("example.onion:17891", 300, None, false,).is_err());
    }
}

#[cfg(test)]
mod sovereign_privacy_lockdown_tests {
    use super::privacy_flag_enabled;
    use std::ffi::OsStr;

    #[test]
    fn opt_in_is_fail_closed_for_unknown_values() {
        assert!(!privacy_flag_enabled(None));

        for text in ["", "0", "false", "FALSE", " off "] {
            assert!(!privacy_flag_enabled(Some(OsStr::new(text))), "{text}");
        }

        for text in ["1", "true", "on", "typo", " ", "2"] {
            assert!(privacy_flag_enabled(Some(OsStr::new(text))), "{text}");
        }
    }
}
