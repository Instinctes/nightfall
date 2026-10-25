//! Bounded public light-client HTTP framing (non-consensus).

use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 512 * 1024;
const MAX_CONCURRENT: usize = 64;

pub(super) struct Admission(Arc<AtomicUsize>);

pub(super) fn admit(n: &Arc<AtomicUsize>) -> Option<Admission> {
    let mut old = n.load(Ordering::Relaxed);
    loop {
        if old >= MAX_CONCURRENT {
            return None;
        }
        match n.compare_exchange_weak(old, old + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Some(Admission(Arc::clone(n))),
            Err(v) => old = v,
        }
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Debug)]
pub(super) enum FramingError {
    Invalid,
    TooLarge,
    Io(io::Error),
}

impl From<io::Error> for FramingError {
    fn from(x: io::Error) -> Self {
        Self::Io(x)
    }
}

pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

fn header_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_header(b: &[u8]) -> Result<(String, String, usize), FramingError> {
    let t = std::str::from_utf8(b).map_err(|_| FramingError::Invalid)?;
    let mut lines = t.split("\r\n");
    let words: Vec<_> = lines
        .next()
        .ok_or(FramingError::Invalid)?
        .split(' ')
        .collect();

    if words.len() != 3
        || words[0].is_empty()
        || !matches!(words[2], "HTTP/1.0" | "HTTP/1.1")
        || !words[1].starts_with('/')
        || words[1].bytes().any(|x| x <= 0x20 || x >= 0x7f)
    {
        return Err(FramingError::Invalid);
    }

    let mut len = None;

    for line in lines {
        if line.is_empty() || line.bytes().any(|x| x == b'\r' || x == b'\n') {
            return Err(FramingError::Invalid);
        }

        let (name, raw) = line.split_once(':').ok_or(FramingError::Invalid)?;

        if name.is_empty()
            || !name.bytes().all(|x| x.is_ascii_alphanumeric() || x == b'-')
            || raw.bytes().any(|x| (x < 0x20 && x != b'\t') || x == 0x7f)
        {
            return Err(FramingError::Invalid);
        }

        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(FramingError::Invalid);
        }

        if name.eq_ignore_ascii_case("content-length") {
            if len.is_some() {
                return Err(FramingError::Invalid);
            }

            let v = raw.trim();

            if v.is_empty() || !v.bytes().all(|x| x.is_ascii_digit()) {
                return Err(FramingError::Invalid);
            }

            let n = v.parse::<usize>().map_err(|_| FramingError::TooLarge)?;

            if n > MAX_BODY {
                return Err(FramingError::TooLarge);
            }

            len = Some(n);
        }
    }

    let n = match words[0] {
        "POST" => len.filter(|&v| v > 0).ok_or(FramingError::Invalid)?,

        "GET" | "OPTIONS" => {
            if len.unwrap_or(0) != 0 {
                return Err(FramingError::Invalid);
            }
            0
        }

        _ => len.unwrap_or(0),
    };

    Ok((words[0].into(), words[1].into(), n))
}

struct Deadline<'a> {
    socket: &'a mut TcpStream,
    until: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());

        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "deadline"));
        }

        self.socket.set_read_timeout(Some(left))?;
        self.socket.read(b)
    }
}

pub(super) fn read_timed(s: &mut TcpStream) -> Result<Request, FramingError> {
    read_request(&mut Deadline {
        socket: s,
        until: Instant::now() + Duration::from_secs(20),
    })
}

pub(super) fn read_request<R: Read>(r: &mut R) -> Result<Request, FramingError> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 1024];

    let end = loop {
        if let Some(end) = header_end(&buf) {
            if end > MAX_HEADER {
                return Err(FramingError::TooLarge);
            }
            break end;
        }

        if buf.len() > MAX_HEADER + 3 {
            return Err(FramingError::TooLarge);
        }

        let n = r.read(&mut tmp)?;

        if n == 0 {
            return Err(FramingError::Invalid);
        }

        buf.extend_from_slice(&tmp[..n]);
    };

    let (method, path, declared) = parse_header(&buf[..end])?;
    let have = buf.len() - end - 4;

    if have > declared {
        return Err(FramingError::Invalid);
    }

    let mut body = Vec::with_capacity(declared);
    body.extend_from_slice(&buf[end + 4..]);
    body.resize(declared, 0);

    r.read_exact(&mut body[have..])?;

    Ok(Request { method, path, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Byte<'a>(&'a [u8]);

    impl Read for Byte<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if out.is_empty() || self.0.is_empty() {
                return Ok(0);
            }
            out[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    fn test(b: &[u8]) -> Result<Request, FramingError> {
        read_request(&mut Byte(b))
    }

    #[test]
    fn fragmented_post() {
        let payload = br#"{"method":"status","params":{},"id":1}"#;
        let mut b = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            payload.len(),
        )
        .into_bytes();

        b.extend_from_slice(payload);
        assert_eq!(test(&b).unwrap().body, payload);
    }

    #[test]
    fn invalid_frames() {
        for b in [
            &b"POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}"[..],
            &b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n"[..],
            &b"POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\n{}"[..],
            &b"POST / HTTP/1.1\nContent-Length: 2\n\n{}"[..],
            &b"GET / HTTP/1.1\r\nContent-Length: 1\r\n\r\nx"[..],
            &b"POST / HTTP/1.1\r\n\r\n{}"[..],
        ] {
            assert!(test(b).is_err());
        }
    }

    #[test]
    fn coalesced_extra_bytes() {
        let raw = b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}X";

        assert!(matches!(
            read_request(&mut &raw[..]),
            Err(FramingError::Invalid)
        ));
    }

    #[test]
    fn body_bound_and_admission() {
        assert!(matches!(
            test(b"POST / HTTP/1.1\r\nContent-Length: 524289\r\n\r\n"),
            Err(FramingError::TooLarge)
        ));

        let n = Arc::new(AtomicUsize::new(0));

        let all: Vec<_> = (0..64).map(|_| admit(&n).unwrap()).collect();

        assert!(admit(&n).is_none());
        drop(all);

        assert_eq!(n.load(Ordering::Acquire), 0);
    }
}
