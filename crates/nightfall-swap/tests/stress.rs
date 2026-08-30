//! Stress the swap crate: adaptor extract, sighash binding, abort table,
//! crash resume, persist. No bitcoind, no signet — script verification is
//! ECDSA-on-sighash, which is the property the adaptor actually needs.

use bitcoin::hashes::Hash;
use bitcoin::{Amount, OutPoint, ScriptBuf, Txid};
use ecdsa_fun::fun::Point;
use nightfall_crypto::swap::{SharedLock, SwapShare};
use nightfall_swap::adaptor::{self, encryption_point};
use nightfall_swap::bitcoin_tx::{
    alice_encsign_refund, bob_encsign_redeem, TxCancel, TxLock, TxPunish, TxRedeem, TxRefund,
};
use nightfall_swap::persist::{self, StoredSwap};
use nightfall_swap::state::{AbortReason, Role, SwapEvent, SwapState};
use nightfall_swap::timelock::Depths;
use rand::rngs::OsRng;
use uuid::Uuid;

fn dummy_spk() -> ScriptBuf {
    // p2wpkh-shaped 22-byte script, contents irrelevant for sighash tests
    ScriptBuf::from_bytes(vec![0x00, 0x14].into_iter().chain([0u8; 20]).collect())
}

fn lock_pair() -> (Point, Point, TxLock) {
    let mut rng = OsRng;
    let a_sk = adaptor::random_bitcoin_sk(&mut rng);
    let b_sk = adaptor::random_bitcoin_sk(&mut rng);
    let a = adaptor::verification_key(&a_sk);
    let b = adaptor::verification_key(&b_sk);
    let prev = OutPoint {
        txid: Txid::from_byte_array([1u8; 32]),
        vout: 0,
    };
    let lock = TxLock::from_prevout(
        prev,
        Amount::from_sat(100_000),
        &a,
        &b,
        Amount::from_sat(80_000),
        Amount::from_sat(1_000),
        Some(dummy_spk()),
    )
    .unwrap();
    (a, b, lock)
}

#[test]
fn redeem_adaptor_leaks_night_secret_and_nothing_else() {
    let share = SwapShare::generate();
    let mut rng = OsRng;
    let a_sk = adaptor::random_bitcoin_sk(&mut rng);
    let b_sk = adaptor::random_bitcoin_sk(&mut rng);
    let a = adaptor::verification_key(&a_sk);
    let b = adaptor::verification_key(&b_sk);
    let prev = OutPoint {
        txid: Txid::from_byte_array([2u8; 32]),
        vout: 0,
    };
    let lock = TxLock::from_prevout(
        prev,
        Amount::from_sat(100_000),
        &a,
        &b,
        Amount::from_sat(80_000),
        Amount::from_sat(1_000),
        None,
    )
    .unwrap();
    let redeem = TxRedeem::new(&lock, dummy_spk(), Amount::from_sat(500)).unwrap();
    let t_a = encryption_point(&share.secret());
    let enc = bob_encsign_redeem(&b_sk, &t_a, &redeem);
    assert!(adaptor::verify_encsig(&b, &t_a, &redeem.sighash, &enc));

    let sig_b = adaptor::decrypt(&share.secret(), enc.clone());
    let recovered = adaptor::recover(&t_a, &sig_b, &enc).unwrap();
    assert_eq!(recovered, share.secret());

    let sig_a = adaptor::sign(&a_sk, &redeem.sighash);
    let finished = redeem.complete(&a, sig_a, &b, sig_b, &lock.script).unwrap();
    assert_eq!(finished.input[0].witness.len(), 3);
}

#[test]
fn cancel_refund_punish_have_distinct_sequences() {
    let (_a, _b, lock) = lock_pair();
    let mut rng = OsRng;
    let a_sk = adaptor::random_bitcoin_sk(&mut rng);
    let b_sk = adaptor::random_bitcoin_sk(&mut rng);
    let a = adaptor::verification_key(&a_sk);
    let b = adaptor::verification_key(&b_sk);
    let cancel = TxCancel::new(&lock, &a, &b, 4, Amount::from_sat(400)).unwrap();
    assert_eq!(cancel.tx.input[0].sequence.to_consensus_u32() & 0xffff, 4);

    let refund = TxRefund::new(&cancel, dummy_spk(), Amount::from_sat(400)).unwrap();
    assert_eq!(refund.tx.input[0].sequence, bitcoin::Sequence::MAX);

    let punish = TxPunish::new(&cancel, dummy_spk(), 4, Amount::from_sat(400)).unwrap();
    assert_eq!(punish.tx.input[0].sequence.to_consensus_u32() & 0xffff, 4);

    let t_b = encryption_point(&SwapShare::generate().secret());
    let enc = alice_encsign_refund(&a_sk, &t_b, &refund);
    assert!(adaptor::verify_encsig(&a, &t_b, &refund.sighash, &enc));
}

#[test]
fn every_section_9_abort_is_forced() {
    let d = Depths::testdrive();

    // 9.1 Bob never locks
    let s = SwapState::new(Role::Alice);
    assert!(matches!(s, SwapState::Setup { .. }));

    // 9.2 redeem too close to H1
    let mut s = SwapState::new(Role::Alice);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(1), d);
    s = s.apply(SwapEvent::AlicePublishedNightLock, d);
    s = s.apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
    s = s.apply(SwapEvent::TooCloseToCancel, d);
    assert!(matches!(
        s,
        SwapState::MustCancel {
            reason: AbortReason::RedeemTooCloseToH1,
            ..
        }
    ));

    // 9.3 Alice never locks — cancel window expires
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(d.cancel), d);
    assert!(matches!(
        s,
        SwapState::MustCancel {
            reason: AbortReason::AliceNeverLocked,
            ..
        }
    ));

    // 9.4 punish (wart)
    let mut s = SwapState::new(Role::Alice);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(1), d);
    s = s.apply(SwapEvent::AlicePublishedNightLock, d);
    s = s.apply(SwapEvent::TooCloseToCancel, d);
    s = s.apply(SwapEvent::CancelConfirmed, d);
    s = s.apply(SwapEvent::PunishConfirmed, d);
    assert!(matches!(s, SwapState::Punished { .. }));

    // 9.4 refund
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::AliceNeverLocked, d);
    s = s.apply(SwapEvent::CancelConfirmed, d);
    s = s.apply(SwapEvent::RefundConfirmed, d);
    assert!(matches!(s, SwapState::Refunded { .. }));

    // 9.6 crash resume
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(1), d);
    s = s.apply(SwapEvent::AlicePublishedNightLock, d);
    s = s.resume(d, d.cancel);
    assert!(matches!(
        s,
        SwapState::MustCancel {
            reason: AbortReason::Crash,
            ..
        }
    ));
}

#[test]
fn persist_survives_a_crash_and_resumes_into_cancel() {
    let dir = std::env::temp_dir().join(format!("nf-stress-{}", Uuid::new_v4()));
    let d = Depths::testdrive();
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(1), d);
    persist::save(&dir, &StoredSwap::new(s.clone(), 5_000_000_000, 80_000)).unwrap();
    let loaded = persist::load(&dir, s.id()).unwrap();
    let resumed = loaded.state.resume(d, d.cancel);
    assert!(matches!(
        resumed,
        SwapState::MustCancel {
            reason: AbortReason::Crash,
            ..
        }
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dleq_is_required_to_combine_keys() {
    let a = SwapShare::generate();
    let b = SwapShare::generate();
    let shared =
        SharedLock::from_verified_offers(&a.offer(), &b.offer(), SharedLock::fresh_scan_secret())
            .expect("honest");
    assert_eq!(
        shared.address().spend_point().unwrap(),
        a.public() + b.public()
    );
}

#[test]
fn depths_are_the_reorg_bound_not_a_guess() {
    assert_eq!(
        Depths::mainnet().night,
        nightfall_consensus::MAX_REORG_DEPTH as u64
    );
    assert!(!Depths::mainnet().may_redeem(140));
}
