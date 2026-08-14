//! Nightfall P2P: newline-delimited JSON messages over TCP.

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

pub fn network_magic(network: NetworkId) -> [u8; 4] {
    match network {
        NetworkId::Mainnet => *b"NFL1",
        NetworkId::Testnet => *b"NFT1",
        NetworkId::Devnet => *b"NFD1",
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
    },
    HelloOk {
        wire: u32,
        network: NetworkId,
        genesis: String,
        height: u64,
        tip: String,
        #[serde(default)]
        listen_port: u16,
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

pub fn connect_peer(addr: &str, timeout_ms: u64) -> std::io::Result<TcpStream> {
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

pub fn handshake(
    stream: &mut TcpStream,
    network: NetworkId,
    genesis: Hash256,
    height: u64,
    tip: Hash256,
    listen_port: u16,
) -> std::io::Result<(u64, String)> {
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
            Ok((h, t))
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
    write_msg(stream, &PeerMsg::Tx { tx: tx.clone() })
}
