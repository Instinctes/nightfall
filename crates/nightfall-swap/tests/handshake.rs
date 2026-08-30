//! A whole handshake, both sides, in one process.
//!
//! Until this existed the crate had every part of a swap and no swap: the
//! messages were defined and nothing built them. This drives Bob and Alice
//! through all six packets and then checks the property the entire design
//! rests on — that Alice taking the Bitcoin hands Bob the scalar that opens
//! the NIGHT.
//!
//! It is not a substitute for regtest. No transaction is broadcast, no script
//! is run by Bitcoin Core, and the timelocks are not mined. What it proves is
//! that both sides derive the same transactions and that the adaptor seam
//! carries the secret across the two curves.

use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};
use nightfall_swap::messages::Amounts;
use nightfall_swap::packet::Packet;
use nightfall_swap::session::{Accepted, Session, SessionError};
use nightfall_swap::timelock::Depths;
use nightfall_types::NetworkId;
use std::str::FromStr;

const NET: NetworkId = NetworkId::Testnet;

fn amounts() -> Amounts {
    Amounts {
        night_darks: 250_000_000,
        btc_sats: 400_000,
        btc_fee_sats: 1_000,
    }
}

/// A plausible P2WPKH script. The content does not matter to the protocol,
/// only that both sides commit to the same bytes.
fn spk(tag: u8) -> ScriptBuf {
    let mut v = vec![0x00, 0x14];
    v.extend_from_slice(&[tag; 20]);
    ScriptBuf::from_bytes(v)
}

fn funding() -> (OutPoint, Amount) {
    let txid =
        Txid::from_str("1111111111111111111111111111111111111111111111111111111111111111").unwrap();
    (OutPoint { txid, vout: 0 }, Amount::from_sat(500_000))
}

/// Run the handshake to completion and hand both sessions back.
fn handshake() -> (Session, Session) {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));

    // 0 — Bob's offer.
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();

    // 1 — Alice's reply.
    let p1 = alice.next_packet().unwrap();
    assert_eq!(bob.accept_packet(&p1).unwrap(), Accepted::Reply);

    // 2 — the Bitcoin lock, unsigned, so Alice can rebuild its children.
    let (prev, value) = funding();
    let p2 = bob.lock_packet(prev, value, Some(spk(0xdd))).unwrap();
    assert_eq!(alice.accept_packet(&p2).unwrap(), Accepted::BitcoinLock);

    // 3 — Alice signs Bob's way out before he ever broadcasts.
    let p3 = alice.next_packet().unwrap();
    assert_eq!(bob.accept_packet(&p3).unwrap(), Accepted::AbortSignatures);

    // 4 — Bob's half of the abort tree.
    let p4 = bob.next_packet().unwrap();
    assert_eq!(
        alice.accept_packet(&p4).unwrap(),
        Accepted::PunishSignatures
    );

    // 5 — the redeem adaptor. Now the swap can complete.
    let p5 = bob.next_packet().unwrap();
    assert_eq!(alice.accept_packet(&p5).unwrap(), Accepted::RedeemAdaptor);
    bob.remember_redeem_enc(&serde_json::from_value(p5.body).unwrap());

    (alice, bob)
}

#[test]
fn both_sides_agree_on_every_transaction() {
    let (alice, bob) = handshake();

    let ac = alice.tx_cancel().unwrap();
    let bc = bob.tx_cancel().unwrap();
    assert_eq!(
        ac.tx, bc.tx,
        "a cancel both sides signed must be one cancel"
    );
    assert_eq!(ac.sighash, bc.sighash);

    assert_eq!(
        alice.tx_redeem().unwrap().sighash,
        bob.tx_redeem().unwrap().sighash,
        "the redeem Bob adaptor-signs is the redeem Alice broadcasts"
    );
    assert_eq!(
        alice.tx_refund(&ac).unwrap().sighash,
        bob.tx_refund(&bc).unwrap().sighash
    );
    assert_eq!(
        alice.tx_punish(&ac).unwrap().sighash,
        bob.tx_punish(&bc).unwrap().sighash
    );
}

#[test]
fn both_sides_derive_the_same_night_address() {
    let (alice, bob) = handshake();
    assert_eq!(
        alice.shared_lock().unwrap().address().encode(),
        bob.shared_lock().unwrap().address().encode(),
        "Alice pays into the address Bob will claim from, or the NIGHT is lost"
    );
}

/// The seam. Alice decrypting the adaptor is what lets her take the Bitcoin;
/// the signature she publishes by doing so is what hands Bob `s_a`.
#[test]
fn taking_the_bitcoin_publishes_the_scalar_that_opens_the_night() {
    let (alice, bob) = handshake();

    let published = alice.decrypt_redeem().unwrap();
    let recovered = bob
        .recover_from_redeem(&published)
        .expect("Bob must be able to pull s_a out of the signature Alice published");

    assert_eq!(
        recovered,
        alice.secrets().share.secret(),
        "the recovered scalar must be Alice's actual share"
    );

    // And the recovered half, not the original, is what Bob claims with.
    //
    // The ephemeral key has to be a real point: `claim_secret` decompresses
    // it to derive the shared secret, and an arbitrary 32 bytes is almost
    // never a valid Ristretto encoding.
    let ephemeral = {
        use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
        use curve25519_dalek::scalar::Scalar;
        (RISTRETTO_BASEPOINT_POINT * Scalar::from(9_007_u64))
            .compress()
            .to_bytes()
    };
    let bobs = bob.night_claim_secret(&recovered, &ephemeral);
    let alices = alice.night_claim_secret(&bob.secrets().share.secret(), &ephemeral);
    assert_eq!(bobs, alices, "both sides must compute the same spend key");
    assert!(bobs.is_some());
}

// --- refusals -------------------------------------------------------------

#[test]
fn a_packet_from_another_swap_is_refused() {
    let (mut alice, mut bob) = (
        {
            let mut b = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
            let p0 = b.next_packet().unwrap();
            Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap()
        },
        Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb)),
    );
    let stranger = bob.next_packet().unwrap();
    let e = alice.accept_packet(&stranger).unwrap_err();
    assert!(
        matches!(e, SessionError::Packet(_)),
        "a packet for a different swap must not be applied: {e:?}"
    );
}

#[test]
fn replaying_a_message_is_refused() {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    let p1 = alice.next_packet().unwrap();
    bob.accept_packet(&p1).unwrap();
    let again = bob.accept_packet(&p1).unwrap_err();
    assert!(matches!(again, SessionError::Packet(_)), "got {again:?}");
}

#[test]
fn a_lock_paying_the_wrong_script_is_refused() {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    let p1 = alice.next_packet().unwrap();
    bob.accept_packet(&p1).unwrap();

    // Bob builds an honest lock, then Alice is handed a different one: same
    // envelope, a transaction that pays somewhere she cannot claim from.
    let (prev, value) = funding();
    let good = bob.lock_packet(prev, value, Some(spk(0xdd))).unwrap();
    let mut body: nightfall_swap::messages::Message2 =
        serde_json::from_value(good.body.clone()).unwrap();

    let mut evil = bitcoin::consensus::deserialize::<bitcoin::Transaction>(&body.tx_lock).unwrap();
    evil.output[0].script_pubkey = spk(0xee);
    body.tx_lock = bitcoin::consensus::serialize(&evil);

    let forged = Packet::new(
        NET,
        good.swap_id,
        2,
        good.amounts.clone(),
        serde_json::to_value(body).unwrap(),
    );
    let e = alice.accept_packet(&forged).unwrap_err();
    assert_eq!(
        e,
        SessionError::TermsChanged,
        "Alice must rebuild the script and refuse a lock she cannot spend"
    );
}

#[test]
fn a_lock_for_the_wrong_amount_is_refused() {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    let p1 = alice.next_packet().unwrap();
    bob.accept_packet(&p1).unwrap();

    let (prev, value) = funding();
    let good = bob.lock_packet(prev, value, Some(spk(0xdd))).unwrap();
    let mut body: nightfall_swap::messages::Message2 =
        serde_json::from_value(good.body.clone()).unwrap();
    let mut evil = bitcoin::consensus::deserialize::<bitcoin::Transaction>(&body.tx_lock).unwrap();
    evil.output[0].value = Amount::from_sat(1_000);
    body.tx_lock = bitcoin::consensus::serialize(&evil);

    let forged = Packet::new(
        NET,
        good.swap_id,
        2,
        good.amounts.clone(),
        serde_json::to_value(body).unwrap(),
    );
    assert_eq!(
        alice.accept_packet(&forged).unwrap_err(),
        SessionError::TermsChanged
    );
}

/// Roles are not interchangeable. Alice cannot produce Bob's messages even
/// if she is handed the right sequence number.
#[test]
fn a_role_cannot_speak_for_the_other() {
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    assert_eq!(alice.message0().unwrap_err(), SessionError::WrongRole);
    assert_eq!(bob.message1().unwrap_err(), SessionError::WrongRole);
}

/// Secrets must not be printable. A panic carrying a `Session` would
/// otherwise write both key halves into a log.
#[test]
fn debug_never_prints_a_key() {
    let bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    let shown = format!("{bob:?} {:?}", bob.secrets());
    assert!(shown.contains("<secret>"));
    let secret_hex = hex::encode(bob.secrets().share.secret().to_bytes());
    assert!(
        !shown.contains(&secret_hex),
        "a key reached a Debug string: {shown}"
    );
}

// --- N9 persist -----------------------------------------------------------

fn tmp() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "nf-sess-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A restart mid-handshake continues, and both sides still agree.
#[test]
fn a_restart_mid_handshake_reaches_the_same_outcome() {
    let bob_dir = tmp();
    let alice_dir = tmp();

    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.save(&bob_dir).unwrap();
    let p0 = bob.next_packet().unwrap();
    bob.save(&bob_dir).unwrap();
    assert_eq!(bob.expect, 1, "after offering, Bob waits for message 1");

    // Crash. Load must restore expect=1, not 0.
    let mut bob = Session::load(&bob_dir, bob.id).unwrap();
    assert_eq!(bob.expect, 1, "load must restore expect, not start at 0");
    assert_eq!(bob.emit, 2);

    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    alice.save(&alice_dir).unwrap();
    let p1 = alice.next_packet().unwrap();
    alice.save(&alice_dir).unwrap();

    let mut alice = Session::load(&alice_dir, alice.id).unwrap();
    assert_eq!(alice.expect, 2);
    assert_eq!(alice.emit, 3);

    bob.accept_packet(&p1).unwrap();
    bob.save(&bob_dir).unwrap();
    let mut bob = Session::load(&bob_dir, bob.id).unwrap();

    let (prev, value) = funding();
    let p2 = bob.lock_packet(prev, value, Some(spk(0xdd))).unwrap();
    bob.save(&bob_dir).unwrap();
    alice.accept_packet(&p2).unwrap();
    alice.save(&alice_dir).unwrap();

    let mut alice = Session::load(&alice_dir, alice.id).unwrap();
    let p3 = alice.next_packet().unwrap();
    alice.save(&alice_dir).unwrap();
    bob.accept_packet(&p3).unwrap();
    bob.save(&bob_dir).unwrap();

    let mut bob = Session::load(&bob_dir, bob.id).unwrap();
    let p4 = bob.next_packet().unwrap();
    bob.save(&bob_dir).unwrap();
    alice.accept_packet(&p4).unwrap();
    alice.save(&alice_dir).unwrap();

    let mut alice = Session::load(&alice_dir, alice.id).unwrap();
    let mut bob = Session::load(&bob_dir, bob.id).unwrap();
    let p5 = bob.next_packet().unwrap();
    bob.save(&bob_dir).unwrap();
    alice.accept_packet(&p5).unwrap();
    bob.remember_redeem_enc(&serde_json::from_value(p5.body.clone()).unwrap());
    bob.save(&bob_dir).unwrap();
    alice.save(&alice_dir).unwrap();

    let alice = Session::load(&alice_dir, alice.id).unwrap();
    let bob = Session::load(&bob_dir, bob.id).unwrap();

    assert_eq!(
        alice.tx_cancel().unwrap().sighash,
        bob.tx_cancel().unwrap().sighash
    );
    assert_eq!(
        alice.shared_lock().unwrap().address().encode(),
        bob.shared_lock().unwrap().address().encode()
    );
    let published = alice.decrypt_redeem().unwrap();
    let recovered = bob.recover_from_redeem(&published).unwrap();
    assert_eq!(recovered, alice.secrets().share.secret());

    let _ = std::fs::remove_dir_all(bob_dir);
    let _ = std::fs::remove_dir_all(alice_dir);
}

#[test]
fn world_readable_session_file_is_refused() {
    let dir = tmp();
    let bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.save(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = nightfall_swap::persist::secret_path(&dir, bob.id);
        let mut p = std::fs::metadata(&path).unwrap().permissions();
        p.set_mode(0o644);
        std::fs::set_permissions(&path, p).unwrap();
        assert_eq!(
            Session::load(&dir, bob.id).unwrap_err(),
            SessionError::WorldReadable
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_share_at_or_above_two_to_the_252_is_not_loaded() {
    let dir = tmp();
    let bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    bob.save(&dir).unwrap();
    let path = nightfall_swap::persist::secret_path(&dir, bob.id);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut too_big = [0u8; 32];
    too_big[31] = 0x10; // 2^252
    v["share_secret_hex"] = serde_json::Value::String(hex::encode(too_big));
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&path).unwrap().permissions();
        p.set_mode(0o600);
        std::fs::set_permissions(&path, p).unwrap();
    }
    assert_eq!(
        Session::load(&dir, bob.id).unwrap_err(),
        SessionError::BadShare
    );
    let _ = std::fs::remove_dir_all(dir);
}

// --- N10 unsigned lock ----------------------------------------------------

#[test]
fn confirmed_lock_must_be_the_one_we_built() {
    let (alice, bob) = handshake();
    let hex = bob.unsigned_lock_hex().unwrap();
    bob.verify_confirmed_lock_hex(&hex).unwrap();
    alice.verify_confirmed_lock_hex(&hex).unwrap();
    assert_eq!(bob.lock_txid().unwrap(), alice.lock_txid().unwrap());

    let psbt = bob.unsigned_lock_psbt().unwrap();
    assert!(!psbt.is_empty(), "PSBT export must produce a string");

    // A different transaction — even one that pays the same amount to a
    // different script — is not our lock.
    let mut evil: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex::decode(&hex).unwrap()).unwrap();
    evil.output[0].value = Amount::from_sat(1);
    let evil_hex = hex::encode(bitcoin::consensus::serialize(&evil));
    assert_eq!(
        bob.verify_confirmed_lock_hex(&evil_hex).unwrap_err(),
        SessionError::LockMismatch
    );
}

// --- packets over files (H7 shape, no node) -------------------------------

#[test]
fn handshake_over_files_reaches_the_same_outcome() {
    let dir = tmp();
    let mut bob = Session::open_as_bob(NET, amounts(), Depths::testdrive(), spk(0xbb));
    std::fs::write(dir.join("0.pkt"), bob.next_packet().unwrap().encode()).unwrap();

    let p0 = Packet::decode(&std::fs::read_to_string(dir.join("0.pkt")).unwrap()).unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    std::fs::write(dir.join("1.pkt"), alice.next_packet().unwrap().encode()).unwrap();

    let p1 = Packet::decode(&std::fs::read_to_string(dir.join("1.pkt")).unwrap()).unwrap();
    bob.accept_packet(&p1).unwrap();
    let (prev, value) = funding();
    std::fs::write(
        dir.join("2.pkt"),
        bob.lock_packet(prev, value, Some(spk(0xdd)))
            .unwrap()
            .encode(),
    )
    .unwrap();

    let p2 = Packet::decode(&std::fs::read_to_string(dir.join("2.pkt")).unwrap()).unwrap();
    alice.accept_packet(&p2).unwrap();
    std::fs::write(dir.join("3.pkt"), alice.next_packet().unwrap().encode()).unwrap();
    bob.accept_packet(
        &Packet::decode(&std::fs::read_to_string(dir.join("3.pkt")).unwrap()).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("4.pkt"), bob.next_packet().unwrap().encode()).unwrap();
    alice
        .accept_packet(
            &Packet::decode(&std::fs::read_to_string(dir.join("4.pkt")).unwrap()).unwrap(),
        )
        .unwrap();
    std::fs::write(dir.join("5.pkt"), bob.next_packet().unwrap().encode()).unwrap();
    let p5 = Packet::decode(&std::fs::read_to_string(dir.join("5.pkt")).unwrap()).unwrap();
    alice.accept_packet(&p5).unwrap();
    bob.remember_redeem_enc(&serde_json::from_value(p5.body).unwrap());

    assert_eq!(
        alice.tx_redeem().unwrap().sighash,
        bob.tx_redeem().unwrap().sighash
    );
    let _ = std::fs::remove_dir_all(dir);
}

// --- Opus 5.3: Bob chooses depths -----------------------------------------

#[test]
fn alice_refuses_depths_that_leave_no_redeem_window() {
    let mut bob = Session::open_as_bob(
        NET,
        amounts(),
        Depths {
            night: 2,
            bitcoin: 1,
            cancel: 2, // 1 + margin(2) is not < 2
            punish: 4,
            btc_redeem_margin: 2,
        },
        spk(0xbb),
    );
    let p0 = bob.next_packet().unwrap();
    assert_eq!(
        Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap_err(),
        SessionError::BadDepths
    );
}

// --- funding: the form and the builder must agree -------------------------

/// Two places decide whether a funding output is big enough: the form the
/// user types into (`ui::validate_funding`) and the thing that actually
/// builds the transaction (`TxLock::from_prevout`, via `lock_packet`).
///
/// If they disagree, the preview lies — either it promises a swap that then
/// fails, or it refuses one that would have worked. Pinned at the boundary,
/// where disagreements live.
#[test]
fn the_funding_form_and_the_lock_builder_draw_the_same_line() {
    use nightfall_swap::ui::{validate_funding, FundingDraft};

    let a = amounts();
    let threshold = a.btc_sats + a.btc_fee_sats;
    let txid_hex = "1111111111111111111111111111111111111111111111111111111111111111";

    let draft = |value: u64| FundingDraft {
        txid: txid_hex.into(),
        vout: "0".into(),
        value: value.to_string(),
        change_address: String::new(),
    };

    // One satoshi short: the form must refuse it.
    assert!(
        validate_funding(&draft(threshold - 1), &a).is_err(),
        "the form must refuse an output that cannot cover value plus fee"
    );

    // And so must the builder, if the form ever let it through.
    let mut bob = Session::open_as_bob(NET, a.clone(), Depths::testdrive(), spk(0xbb));
    let p0 = bob.next_packet().unwrap();
    let mut alice = Session::join_from_packet(NET, spk(0xaa), spk(0xcc), &p0).unwrap();
    let p1 = alice.next_packet().unwrap();
    bob.accept_packet(&p1).unwrap();

    let (prev, _) = funding();
    let short = bob.lock_packet(prev, Amount::from_sat(threshold - 1), None);
    assert!(
        short.is_err(),
        "the builder must refuse the same output the form refuses"
    );

    // Exactly at the threshold both must accept, or the form is stricter
    // than reality and the user is turned away for nothing.
    assert!(
        validate_funding(&draft(threshold), &a).is_ok(),
        "the form must accept exactly enough"
    );
    assert!(
        bob.lock_packet(prev, Amount::from_sat(threshold), None)
            .is_ok(),
        "and so must the builder"
    );
}

/// The exported bytes are the bytes the check accepts. A round trip through
/// hex is where an encoding mistake would show up.
#[test]
fn the_exported_lock_is_the_lock_we_verify() {
    let (_alice, bob) = handshake();
    let raw = bob.unsigned_lock_hex().unwrap();
    bob.verify_confirmed_lock_hex(&raw)
        .expect("what we hand out for signing must be what we accept back");

    // Flip one byte of the output value and it must be refused.
    let mut tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex::decode(&raw).unwrap()).unwrap();
    tx.output[0].value = Amount::from_sat(tx.output[0].value.to_sat() - 1);
    let tampered = hex::encode(bitcoin::consensus::serialize(&tx));
    assert!(
        bob.verify_confirmed_lock_hex(&tampered).is_err(),
        "a changed lock output must not pass as ours"
    );
}
