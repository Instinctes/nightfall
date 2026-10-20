use nightfall_consensus::BlockHeader;
use nightfall_crypto::{domain, hash_multi, Commitment};
use nightfall_types::{Hash256, Height, PROTOCOL_VERSION};

fn header() -> BlockHeader {
    BlockHeader {
        version: PROTOCOL_VERSION,
        height: Height(42),
        prev_hash: Hash256([0x11; 32]),
        utxo_root: Hash256([0x22; 32]),
        kernel_sum: Commitment([0x33; 32]),
        body_root: Hash256([0x44; 32]),
        timestamp_unix: 1_700_000_000,
        difficulty: 5_000,
        nonce: 0x0102_0304_0506_0708,
        reward_darks: 123_456_789,
    }
}

#[test]
fn protocol_execution_is_permanently_v8() {
    assert_eq!(
        PROTOCOL_VERSION, 8,
        "mainnet consensus is V8; changing this requires a new chain and is forbidden"
    );
}

#[test]
fn v8_block_hash_domain_is_frozen() {
    assert_eq!(domain::BLOCK, b"nightfall:block:v2");
}

#[test]
fn v8_pow_preimage_field_set_and_order_are_frozen() {
    let h = header();

    let expected = hash_multi(
        domain::BLOCK,
        &[
            &h.version.to_le_bytes(),
            &h.height.0.to_le_bytes(),
            &h.prev_hash.0,
            &h.utxo_root.0,
            &h.kernel_sum.0,
            &h.body_root.0,
            &h.timestamp_unix.to_le_bytes(),
            &h.difficulty.to_le_bytes(),
            &h.reward_darks.to_le_bytes(),
        ],
    )
    .0
    .to_vec();

    assert_eq!(
        h.pow_preimage(),
        expected,
        "V8 PoW preimage encoding changed"
    );
}

#[test]
fn v8_nonce_is_outside_preimage_but_inside_block_identity() {
    let a = header();
    let mut b = a.clone();
    b.nonce ^= 1;

    assert_eq!(
        a.pow_preimage(),
        b.pow_preimage(),
        "V8 nonce must remain outside pow_preimage"
    );

    assert_ne!(
        a.hash(),
        b.hash(),
        "V8 canonical block identity must include nonce"
    );

    let expected = hash_multi(domain::BLOCK, &[&a.pow_preimage(), &a.nonce.to_le_bytes()]);

    assert_eq!(
        a.hash(),
        expected,
        "V8 canonical block hash encoding changed"
    );
}

#[test]
fn every_v8_consensus_preimage_field_is_committed() {
    let base = header();
    let want = base.pow_preimage();

    let mut h = base.clone();
    h.version ^= 1;
    assert_ne!(h.pow_preimage(), want, "version dropped from preimage");

    let mut h = base.clone();
    h.height = Height(base.height.0 + 1);
    assert_ne!(h.pow_preimage(), want, "height dropped from preimage");

    let mut h = base.clone();
    h.prev_hash = Hash256([0x12; 32]);
    assert_ne!(h.pow_preimage(), want, "prev_hash dropped from preimage");

    let mut h = base.clone();
    h.utxo_root = Hash256([0x23; 32]);
    assert_ne!(h.pow_preimage(), want, "utxo_root dropped from preimage");

    let mut h = base.clone();
    h.kernel_sum = Commitment([0x34; 32]);
    assert_ne!(h.pow_preimage(), want, "kernel_sum dropped from preimage");

    let mut h = base.clone();
    h.body_root = Hash256([0x45; 32]);
    assert_ne!(h.pow_preimage(), want, "body_root dropped from preimage");

    let mut h = base.clone();
    h.timestamp_unix += 1;
    assert_ne!(h.pow_preimage(), want, "timestamp dropped from preimage");

    let mut h = base.clone();
    h.difficulty += 1;
    assert_ne!(h.pow_preimage(), want, "difficulty dropped from preimage");

    let mut h = base;
    h.reward_darks += 1;
    assert_ne!(h.pow_preimage(), want, "reward dropped from preimage");
}
