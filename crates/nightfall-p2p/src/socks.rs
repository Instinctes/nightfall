//! SOCKS5 CONNECT for outbound P2P.
//!
//! Destinations are sent to the proxy as a hostname whenever they are not
//! already an IP. That is what keeps DNS off the local resolver — a wallet
//! pointing at Tor would otherwise leak every seed lookup to the ISP.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
use std::time::Duration;

use crate::connect_direct;

/// A SOCKS5 proxy that can open outbound Nightfall sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocksProxy {
    pub addr: String,
}

impl SocksProxy {
    /// Accepts `127.0.0.1:9050`, `socks5://127.0.0.1:9050`, or `socks://…`.
    pub fn parse(s: &str) -> std::io::Result<Self> {
        let s = s.trim();
        let s = s
            .strip_prefix("socks5h://")
            .or_else(|| s.strip_prefix("socks5://"))
            .or_else(|| s.strip_prefix("socks://"))
            .unwrap_or(s)
            .trim();
        if !looks_like_dial_target(s) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "proxy must be host:port (Tor default is 127.0.0.1:9050)",
            ));
        }
        Ok(Self {
            addr: s.to_string(),
        })
    }

    pub fn connect(&self, dest: &str, timeout_ms: u64) -> std::io::Result<TcpStream> {
        let mut stream = connect_direct(&self.addr, timeout_ms)?;
        let timeout = Duration::from_millis(timeout_ms.max(1));
        socks5_connect(&mut stream, dest, timeout)?;
        Ok(stream)
    }
}

/// `host:port` or `[ipv6]:port`, including `.onion` names. Does not resolve.
pub fn looks_like_dial_target(addr: &str) -> bool {
    let addr = addr.trim();
    if addr.is_empty() || addr.starts_with(':') {
        return false;
    }
    if let Some(rest) = addr.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return false;
        };
        return !host.is_empty() && port.parse::<u16>().map(|p| p > 0).unwrap_or(false);
    }
    match addr.rsplit_once(':') {
        Some((host, port)) => !host.is_empty() && port.parse::<u16>().map(|p| p > 0).unwrap_or(false),
        None => false,
    }
}

fn split_host_port(addr: &str) -> std::io::Result<(String, u16)> {
    let addr = addr.trim();
    if let Some(rest) = addr.strip_prefix('[') {
        let (host, port) = rest.split_once("]:").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad IPv6 dial target")
        })?;
        let port: u16 = port.parse().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad port")
        })?;
        if port == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "port must be non-zero",
            ));
        }
        return Ok((host.to_string(), port));
    }
    let (host, port) = addr.split_once(':').filter(|(h, _)| !h.is_empty()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "dial target must be host:port")
    })?;
    let port: u16 = port.parse().map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad port")
    })?;
    if port == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "port must be non-zero",
        ));
    }
    Ok((host.to_string(), port))
}

fn socks5_connect(stream: &mut TcpStream, dest: &str, timeout: Duration) -> std::io::Result<()> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    // Greeting: SOCKS5, one method, no authentication.
    stream.write_all(&[0x05, 0x01, 0x00])?;
    let mut greet = [0u8; 2];
    stream.read_exact(&mut greet)?;
    if greet[0] != 0x05 || greet[1] != 0x00 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SOCKS5 proxy refused unauthenticated CONNECT",
        ));
    }

    let (host, port) = split_host_port(dest)?;
    let mut req = Vec::with_capacity(7 + host.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00]);
    if let Ok(v4) = host.parse::<Ipv4Addr>() {
        req.push(0x01);
        req.extend_from_slice(&v4.octets());
    } else if let Ok(v6) = host.parse::<Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&v6.octets());
    } else {
        if host.len() > 255 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "hostname too long for SOCKS5",
            ));
        }
        req.push(0x03);
        req.push(host.len() as u8);
        req.extend_from_slice(host.as_bytes());
    }
    req.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&req)?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head)?;
    if head[0] != 0x05 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a SOCKS5 reply",
        ));
    }
    if head[1] != 0x00 {
        return Err(std::io::Error::other(format!(
            "SOCKS5 CONNECT failed (code {})",
            head[1]
        )));
    }
    consume_bind_addr(stream, head[3])?;
    Ok(())
}

fn consume_bind_addr(stream: &mut TcpStream, atyp: u8) -> std::io::Result<()> {
    match atyp {
        0x01 => {
            let mut rest = [0u8; 6];
            stream.read_exact(&mut rest)
        }
        0x04 => {
            let mut rest = [0u8; 18];
            stream.read_exact(&mut rest)
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len)?;
            let mut rest = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut rest)
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unknown SOCKS5 address type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parse_strips_scheme() {
        let p = SocksProxy::parse("socks5://127.0.0.1:9050").unwrap();
        assert_eq!(p.addr, "127.0.0.1:9050");
        assert!(SocksProxy::parse("nope").is_err());
    }

    #[test]
    fn dial_target_accepts_onion_without_resolving() {
        assert!(looks_like_dial_target(
            "abcdefghijklmnopqrstuvwxyz234567.onion:17891"
        ));
        assert!(looks_like_dial_target("[2001:db8::1]:17891"));
        assert!(!looks_like_dial_target("no-port"));
        assert!(!looks_like_dial_target(":17891"));
    }

    #[test]
    fn socks5_handshake_against_a_toy_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut greet = [0u8; 3];
            c.read_exact(&mut greet).unwrap();
            assert_eq!(greet, [0x05, 0x01, 0x00]);
            c.write_all(&[0x05, 0x00]).unwrap();
            let mut head = [0u8; 4];
            c.read_exact(&mut head).unwrap();
            assert_eq!(head[0], 0x05);
            assert_eq!(head[1], 0x01);
            // domain name
            assert_eq!(head[3], 0x03);
            let mut ln = [0u8; 1];
            c.read_exact(&mut ln).unwrap();
            let mut rest = vec![0u8; ln[0] as usize + 2];
            c.read_exact(&mut rest).unwrap();
            // success, IPv4 bind 0.0.0.0:0
            c.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .unwrap();
        });
        let p = SocksProxy {
            addr: proxy.to_string(),
        };
        let s = p.connect("seed.example:17891", 2_000).unwrap();
        drop(s);
    }
}
