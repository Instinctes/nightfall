//! Adversarial response-boundary tests for Nightfall 1.0.6.
//!
//! Synthetic loopback TCP peers only.
//! No live network, wallets, payments or mining.

use nightfall_p2p::{read_msg, request_blocks, write_msg, PeerMsg};
use nightfall_types::MAX_MESSAGE_BYTES;

use std::io::{self, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

/// Execute one isolated synthetic peer interaction.
/// No real blocks are constructed or transferred.
fn mock_peer<F>(reply: F) -> io::Result<usize>
where
    F: FnOnce(&mut TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let addr = listener.local_addr().unwrap();

    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();

        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();

        stream
            .set_write_timeout(Some(Duration::from_secs(15)))
            .unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());

        match read_msg(&mut reader).unwrap() {
            PeerMsg::GetBlocks { from_height, limit } => {
                assert_eq!(from_height, 7);
                assert_eq!(limit, 2);
            }
            other => {
                panic!("Unexpected request: {other:?}");
            }
        }

        drop(reader);
        reply(&mut stream);
    });

    let mut client = TcpStream::connect(addr).unwrap();

    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();

    client
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();

    let result = request_blocks(&mut client, 7, 2).map(|blocks| blocks.len());

    peer.join().expect("Synthetic peer panicked");

    result
}

#[test]
fn allowed_interleavings_deliver_the_reply() {
    // 63 unrelated messages plus the actual response
    // occupy all 64 permitted read slots.
    for count in [0, 1, 2, 17, 31, 63] {
        let result = mock_peer(move |stream| {
            for index in 0..count {
                match index % 3 {
                    0 => {
                        write_msg(stream, &PeerMsg::GetStatus).unwrap();
                    }
                    1 => {
                        write_msg(stream, &PeerMsg::Ping { nonce: index }).unwrap();
                    }
                    _ => {
                        stream.write_all(b"this-is-not-json\n").unwrap();
                    }
                }
            }

            write_msg(stream, &PeerMsg::Blocks { blocks: vec![] }).unwrap();
        });

        assert_eq!(result.unwrap(), 0, "Interleavings: {count}");
    }
}

#[test]
fn excessive_interleaving_fails_closed() {
    let result = mock_peer(|stream| {
        for _ in 0..64 {
            write_msg(stream, &PeerMsg::GetStatus).unwrap();
        }

        write_msg(stream, &PeerMsg::Blocks { blocks: vec![] }).unwrap();
    });

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn remote_error_is_never_ignored() {
    let result = mock_peer(|stream| {
        write_msg(
            stream,
            &PeerMsg::Error {
                message: "no archived blocks".into(),
            },
        )
        .unwrap();

        write_msg(stream, &PeerMsg::Blocks { blocks: vec![] }).unwrap();
    });

    assert!(result
        .unwrap_err()
        .to_string()
        .contains("no archived blocks"));
}

#[test]
fn invalid_utf8_fails_closed() {
    let result = mock_peer(|stream| {
        stream.write_all(b"\xff\n").unwrap();
    });

    let error = result.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    assert!(error.to_string().contains("non-utf8"));
}

#[test]
fn truncated_frame_fails_closed() {
    let result = mock_peer(|stream| {
        stream.write_all(b"{\"type\":\"get_status\"").unwrap();
    });

    let error = result.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    assert!(error.to_string().contains("truncated"));
}

#[test]
fn disconnected_peer_is_not_empty_success() {
    let result = mock_peer(|_stream| {});

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn oversized_frame_rejected_before_deserialization() {
    let result = mock_peer(|stream| {
        // One byte beyond the protocol's message limit.
        let frame = vec![b'X'; MAX_MESSAGE_BYTES + 1];

        // The receiver may close the socket while
        // the oversized frame is still being written.
        let _ = stream.write_all(&frame);
    });

    let error = result.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    assert!(error.to_string().contains("exceeds"));
}
