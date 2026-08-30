//! Persist-before-send, one layer above payments.
//!
//! Grok described this hole precisely and did not close it, because the brief
//! I wrote specified `Session::load(datadir, id)` as a free call and said
//! nothing about the session knowing where it lives. That constraint was
//! mine and it was wrong.
//!
//! The hole: `next_packet` advances `emit` and stores `last_packet` in
//! memory. Saving was the caller's duty. A crash between producing a packet
//! and saving leaves disk one step behind — on restart `next_packet` refuses
//! that sequence number (emit does not match) and `last_packet` is the
//! previous one. Before the Bitcoin lock that costs time. After it, the coin.
//!
//! These tests pin the fix: with a datadir set, the session writes itself
//! down before the caller ever sees the packet, and a write that fails does
//! not hand out a packet we cannot reproduce.

use bitcoin::ScriptBuf;
use nightfall_swap::messages::Amounts;
use nightfall_swap::session::{Session, SessionError};
use nightfall_swap::timelock::Depths;
use nightfall_types::NetworkId;
use uuid::Uuid;

const NET: NetworkId = NetworkId::Testnet;

fn amounts() -> Amounts {
    Amounts {
        night_darks: 250_000_000,
        btc_sats: 400_000,
        btc_fee_sats: 1_000,
    }
}

fn spk(tag: u8) -> ScriptBuf {
    let mut v = vec![0x00, 0x14];
    v.extend_from_slice(&[tag; 20]);
    ScriptBuf::from_bytes(v)
}

fn tempdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nf-pbs-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The crash this is all about: produce a packet, lose the process before
/// the caller could have saved anything, and still be able to hand the same
/// packet to the counterparty.
#[test]
fn a_packet_survives_a_crash_the_caller_never_saved() {
    let dir = tempdir();
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.persist_to(&dir);
    bob.save(&dir).unwrap();
    let id = bob.id;

    let p0 = bob.next_packet().unwrap();
    // No explicit save here — that is the whole point. Simulate the crash.
    drop(bob);

    let restored = Session::load(&dir, id).expect("the session must be on disk");
    let again = restored
        .last_packet()
        .expect("the packet we produced must have been written down");
    assert_eq!(
        again.encode(),
        p0.encode(),
        "after a crash the same packet must be reproducible byte for byte"
    );

    let _ = std::fs::remove_dir_all(dir);
}

/// `emit` must be on disk too, or the restored session would offer to
/// produce message 0 a second time.
#[test]
fn progress_is_on_disk_without_an_explicit_save() {
    let dir = tempdir();
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.persist_to(&dir);
    bob.save(&dir).unwrap();
    let id = bob.id;

    let _ = bob.next_packet().unwrap();
    let emit_in_memory = bob.emit;
    drop(bob);

    let restored = Session::load(&dir, id).unwrap();
    assert_eq!(
        restored.emit, emit_in_memory,
        "disk must not lag behind the packet we already handed out"
    );
    assert_ne!(restored.emit, 0, "message 0 must not be offered twice");

    let _ = std::fs::remove_dir_all(dir);
}

/// An accepted packet moves `expect` and stores the counterparty's keys.
/// Losing that is as bad as losing an outbound packet — on restart we would
/// ask them for a message they already sent.
#[test]
fn accepting_a_packet_is_written_down_too() {
    let bob_dir = tempdir();
    let alice_dir = tempdir();

    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.persist_to(&bob_dir);
    bob.save(&bob_dir).unwrap();
    let p0 = bob.next_packet().unwrap();

    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    alice.persist_to(&alice_dir);
    alice.save(&alice_dir).unwrap();
    let alice_id = alice.id;
    let p1 = alice.next_packet().unwrap();

    let bob_id = bob.id;
    bob.accept_packet(&p1).unwrap();
    let expect_in_memory = bob.expect;
    drop(bob);

    let restored = Session::load(&bob_dir, bob_id).unwrap();
    assert_eq!(
        restored.expect, expect_in_memory,
        "an applied packet must be on disk before the caller moves on"
    );

    let _ = std::fs::remove_dir_all(bob_dir);
    let _ = std::fs::remove_dir_all(alice_dir);
    let _ = alice_id;
}

/// If the write fails, the caller must not receive the packet. A packet on
/// the wire that we have no record of is the state we cannot recover from,
/// so the advance is rolled back and a retry is possible.
#[test]
fn a_packet_that_cannot_be_written_is_not_handed_out() {
    let dir = tempdir();
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.save(&dir).unwrap();

    // Point the session at a path that cannot be written: a *file* where the
    // datadir should be. No permission games, so this behaves the same for
    // root and for a normal user.
    let blocked = std::env::temp_dir().join(format!("nf-pbs-file-{}", Uuid::new_v4()));
    std::fs::write(&blocked, b"not a directory").unwrap();
    bob.persist_to(&blocked);

    let emit_before = bob.emit;
    let err = bob
        .next_packet()
        .expect_err("an unwritable session must not produce a packet");
    assert!(
        matches!(err, SessionError::Corrupt | SessionError::WorldReadable),
        "expected a persistence failure, got {err:?}"
    );
    assert_eq!(
        bob.emit, emit_before,
        "a failed write must not leave the session one step ahead of its disk"
    );

    // And once the disk works again, the same packet can still be made.
    bob.persist_to(&dir);
    let p0 = bob
        .next_packet()
        .expect("retry must work after the rollback");
    assert_eq!(p0.seq, 0);

    let _ = std::fs::remove_file(blocked);
    let _ = std::fs::remove_dir_all(dir);
}

/// Without a datadir the session still works — tests and the handshake
/// exercise it that way — but then saving really is the caller's duty.
#[test]
fn a_session_without_a_datadir_still_runs() {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    assert!(bob.datadir().is_none());
    let p0 = bob.next_packet().unwrap();
    assert_eq!(p0.seq, 0);
}

/// A session read from disk must keep writing to that disk without being
/// told again.
///
/// Found by mutation: setting `datadir: None` in `load` changed nothing,
/// because every existing test saves explicitly after each step. That made
/// the auto-datadir a promise no test held — and the promise matters most
/// exactly here, after a restart, when a caller is least likely to remember
/// to switch persistence back on.
#[test]
fn a_restored_session_keeps_persisting_without_being_told() {
    let dir = tempdir();
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.persist_to(&dir);
    bob.save(&dir).unwrap();
    let id = bob.id;
    drop(bob);

    // Restart. Nobody calls persist_to again.
    let mut restored = Session::load(&dir, id).unwrap();
    assert!(
        restored.datadir().is_some(),
        "a loaded session must already know where it lives"
    );

    let p0 = restored.next_packet().unwrap();
    // Second crash, again with no explicit save.
    drop(restored);

    let twice = Session::load(&dir, id).unwrap();
    assert_eq!(
        twice
            .last_packet()
            .expect("the packet produced after the restart must be on disk")
            .encode(),
        p0.encode(),
        "a restored session that stopped persisting would lose the next packet"
    );
    assert_ne!(twice.emit, 0, "and its progress");

    let _ = std::fs::remove_dir_all(dir);
}
