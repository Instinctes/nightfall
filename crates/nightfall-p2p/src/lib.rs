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

fn is_false(value: &bool) -> bool {
    !*value
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
        /// Supports explicit Dandelion stem-phase transaction relay.
        ///
        /// `serde(default)` is deliberate: pre-capability peers omit this
        /// field and are therefore treated as incapable rather than guessed.
        #[serde(default)]
        dandelion_stem_v1: bool,
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
        /// Supports explicit Dandelion stem-phase transaction relay.
        #[serde(default)]
        dandelion_stem_v1: bool,
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
        /// `true` while the transaction is in the Dandelion stem phase.
        ///
        /// False is omitted from the wire representation so ordinary
        /// transaction messages remain compatible with older Nightfall nodes.
        /// Older serde readers also ignore the additional `stem` field.
        #[serde(default, skip_serializing_if = "is_false")]
        stem: bool,
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

pub fn write_msg(stream: &mut TcpStream, msg: &PeerMsg) -> std::io::Result<()> {
    let line = serde_json::to_string(msg).map_err(|e| std::io::Error::other(e.to_string()))?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
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

pub fn connect_peer(addr: &str, timeout_ms: u64) -> std::io::Result<TcpStream> {
    connect_peer_via(addr, timeout_ms, None).map(|(s, _)| s)
}

/// Dial `addr`, optionally through SOCKS5.
///
/// Returns `(stream, used_tor)`. If the proxy is down, a clearnet destination
/// falls back to a direct connect. `.onion` never falls back — that would
/// leak the name to DNS.
pub fn connect_peer_via(
    addr: &str,
    timeout_ms: u64,
    proxy: Option<&SocksProxy>,
) -> std::io::Result<(TcpStream, bool)> {
    let onion = addr.contains(".onion");
    if let Some(p) = proxy {
        match p.connect(addr, timeout_ms) {
            Ok(s) => return Ok((s, true)),
            Err(e) if onion => return Err(e),
            Err(_) => {}
        }
    }
    if onion {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "onion destinations require a working SOCKS5 proxy",
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
    /// Port the remote peer advertised for inbound connections.
    pub listen_port: u16,
    /// Explicitly negotiated Dandelion stem-relay support.
    pub dandelion_stem_v1: bool,
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
            dandelion_stem_v1: true,
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
            listen_port: peer_listen_port,
            pruned: peer_pruned,
            first_height: peer_first,
            dandelion_stem_v1: peer_dandelion_stem_v1,
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
                listen_port: peer_listen_port,
                dandelion_stem_v1: peer_dandelion_stem_v1,
            })
        }
        PeerMsg::Error { message } => Err(std::io::Error::other(message)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected hello_ok",
        )),
    }
}

pub fn request_blocks(
    stream: &mut TcpStream,
    from_height: u64,
    limit: usize,
) -> std::io::Result<Vec<Block>> {
    write_msg(stream, &PeerMsg::GetBlocks { from_height, limit })?;
    let mut reader = BufReader::new(stream.try_clone()?);
    match read_msg(&mut reader)? {
        PeerMsg::Blocks { blocks } => Ok(blocks),
        PeerMsg::Error { message } => Err(std::io::Error::other(message)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected blocks",
        )),
    }
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
    write_msg(
        stream,
        &PeerMsg::Tx {
            tx: tx.clone(),
            stem: false,
        },
    )
}

/// Send a transaction in the Dandelion stem phase.
///
/// This deliberately uses the existing `tx` wire message with one optional
/// field. `stem: false` is omitted, and older peers ignore `stem: true`, so a
/// mixed-version network degrades to ordinary diffusion rather than splitting.
pub fn broadcast_stem_tx(stream: &mut TcpStream, tx: &Transaction) -> std::io::Result<()> {
    write_msg(
        stream,
        &PeerMsg::Tx {
            tx: tx.clone(),
            stem: true,
        },
    )
}

#[cfg(test)]
mod directory_tests {
    use super::is_directory_addr;

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
}

#[cfg(test)]
mod dandelion_wire_tests {
    use super::*;
    use nightfall_ledger::Transaction;

    #[derive(serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum LegacyPeerMsg {
        Tx { tx: Transaction },
    }

    fn empty_test_tx() -> Transaction {
        // Wire compatibility does not require a consensus-valid transaction.
        // The P2P codec must preserve arbitrary serialized Transaction values;
        // consensus validation happens after decoding.
        Transaction {
            version: 8,
            inputs: Vec::new(),
            outputs: Vec::new(),
            kernels: Vec::new(),
        }
    }

    #[test]
    fn ordinary_tx_keeps_the_legacy_wire_shape() {
        let encoded = serde_json::to_value(PeerMsg::Tx {
            tx: empty_test_tx(),
            stem: false,
        })
        .unwrap();

        assert_eq!(encoded.get("type").and_then(|v| v.as_str()), Some("tx"));
        assert!(
            encoded.get("stem").is_none(),
            "false stem flag must not alter ordinary transaction wire format"
        );
    }

    #[test]
    fn legacy_reader_ignores_the_new_stem_field() {
        let encoded = serde_json::to_value(PeerMsg::Tx {
            tx: empty_test_tx(),
            stem: true,
        })
        .unwrap();

        assert_eq!(encoded.get("stem").and_then(|v| v.as_bool()), Some(true));

        let legacy: LegacyPeerMsg = serde_json::from_value(encoded).unwrap();
        match legacy {
            LegacyPeerMsg::Tx { tx } => assert_eq!(tx.version, 8),
        }
    }

    #[test]
    fn new_reader_treats_legacy_tx_as_fluff() {
        let mut encoded = serde_json::to_value(PeerMsg::Tx {
            tx: empty_test_tx(),
            stem: true,
        })
        .unwrap();

        encoded.as_object_mut().unwrap().remove("stem");

        let decoded: PeerMsg = serde_json::from_value(encoded).unwrap();

        match decoded {
            PeerMsg::Tx { stem, .. } => assert!(!stem),
            other => panic!("expected tx, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod dandelion_capability_tests {
    use super::*;

    fn hello(capable: bool) -> PeerMsg {
        PeerMsg::Hello {
            wire: WIRE_VERSION,
            network: NetworkId::Devnet,
            genesis: "00".repeat(32),
            height: 7,
            tip: "11".repeat(32),
            agent: "test".into(),
            listen_port: 17891,
            pruned: false,
            first_height: 0,
            dandelion_stem_v1: capable,
        }
    }

    #[test]
    fn legacy_hello_defaults_to_no_stem_capability() {
        let mut value = serde_json::to_value(hello(true)).unwrap();

        value.as_object_mut().unwrap().remove("dandelion_stem_v1");

        let decoded: PeerMsg = serde_json::from_value(value).unwrap();

        match decoded {
            PeerMsg::Hello {
                dandelion_stem_v1, ..
            } => assert!(!dandelion_stem_v1),
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[test]
    fn new_hello_explicitly_advertises_stem_capability() {
        let value = serde_json::to_value(hello(true)).unwrap();

        assert_eq!(
            value.get("dandelion_stem_v1").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn capability_false_is_preserved() {
        let value = serde_json::to_value(hello(false)).unwrap();
        let decoded: PeerMsg = serde_json::from_value(value).unwrap();

        match decoded {
            PeerMsg::Hello {
                dandelion_stem_v1, ..
            } => assert!(!dandelion_stem_v1),
            other => panic!("expected hello, got {other:?}"),
        }
    }
}
