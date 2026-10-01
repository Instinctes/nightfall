//! Mixed-version handshake adversarial tests.

use nightfall_p2p::PeerMsg;
use nightfall_types::NetworkId;

fn hello_ok(capable: bool) -> PeerMsg {
    PeerMsg::HelloOk {
        wire: 1234,
        network: NetworkId::Devnet,
        genesis: "00".repeat(32),
        height: 9,
        tip: "11".repeat(32),
        listen_port: 17891,
        pruned: false,
        first_height: 0,
        dandelion_stem_v1: capable,
    }
}

#[test]
fn legacy_hello_ok_defaults_to_no_stem_capability() {
    let mut value = serde_json::to_value(hello_ok(true)).unwrap();

    value.as_object_mut().unwrap().remove("dandelion_stem_v1");

    let decoded: PeerMsg = serde_json::from_value(value).unwrap();

    match decoded {
        PeerMsg::HelloOk {
            dandelion_stem_v1, ..
        } => {
            assert!(!dandelion_stem_v1);
        }
        other => panic!("expected hello_ok, got {other:?}"),
    }
}

#[test]
fn new_hello_ok_preserves_explicit_stem_capability() {
    let value = serde_json::to_value(hello_ok(true)).unwrap();

    assert_eq!(
        value.get("dandelion_stem_v1").and_then(|v| v.as_bool()),
        Some(true)
    );

    let decoded: PeerMsg = serde_json::from_value(value).unwrap();

    match decoded {
        PeerMsg::HelloOk {
            dandelion_stem_v1, ..
        } => {
            assert!(dandelion_stem_v1);
        }
        other => panic!("expected hello_ok, got {other:?}"),
    }
}
