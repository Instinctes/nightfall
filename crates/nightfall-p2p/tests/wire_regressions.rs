use nightfall_consensus::{Block, BlockHeader};
use nightfall_p2p::{read_msg, request_blocks, write_msg, PeerMsg};
use nightfall_types::{Hash256, Height, MAX_MESSAGE_BYTES, PROTOCOL_VERSION};
use std::{
    io::{BufReader, Read},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};
#[test]
fn paginated_blocks_survive_real_tcp_interleaved_frames() {
    // Deliberately unvalidated transport fixtures: this tests framing, not consensus.
    let blocks: Vec<Block> = (0..3)
        .map(|h| Block {
            header: BlockHeader {
                version: PROTOCOL_VERSION,
                height: Height(h),
                prev_hash: Hash256::ZERO,
                utxo_root: Hash256::ZERO,
                kernel_sum: serde_json::from_value(serde_json::json!(vec![0u8; 32])).unwrap(),
                body_root: Hash256::ZERO,
                timestamp_unix: 1_800_000_000 + h,
                difficulty: 1,
                nonce: 0,
                reward_darks: 0,
            },
            body: Default::default(),
        })
        .collect();
    let listen = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listen.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut socket, _) = listen.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        assert!(matches!(
            read_msg(&mut reader).unwrap(),
            PeerMsg::GetBlocks {
                from_height: 0,
                limit: 3
            }
        ));
        write_msg(
            &mut socket,
            &PeerMsg::Blocks {
                blocks: blocks[..1].to_vec(),
            },
        )
        .unwrap();
        write_msg(&mut socket, &PeerMsg::GetStatus).unwrap();
        assert!(matches!(
            read_msg(&mut reader).unwrap(),
            PeerMsg::GetBlocks {
                from_height: 1,
                limit: 2
            }
        ));
        write_msg(
            &mut socket,
            &PeerMsg::Blocks {
                blocks: blocks[1..].to_vec(),
            },
        )
        .unwrap();
    });
    let mut socket = TcpStream::connect(addr).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let found = request_blocks(&mut socket, 0, 3).unwrap();
    assert_eq!(found.len(), 3);
    assert_eq!(
        found.iter().map(|b| b.header.height.0).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    worker.join().unwrap();
}
#[test]
fn oversized_write_leaves_no_partial_tcp_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    assert!(write_msg(
        &mut client,
        &PeerMsg::Error {
            message: "x".repeat(MAX_MESSAGE_BYTES)
        }
    )
    .is_err());
    drop(client);
    let mut got = Vec::new();
    server.read_to_end(&mut got).unwrap();
    assert!(got.is_empty());
}
