//! K2 — a dishonest counterparty at every step. The question is always:
//! does the honest party end worse off than if they had walked away?

use nightfall_swap::packet::{Packet, PacketError};
use nightfall_swap::state::{AbortReason, Role, SwapEvent, SwapState};
use nightfall_swap::timelock::Depths;
use nightfall_swap::{Amounts, StoredSwap};
use nightfall_types::NetworkId;
use uuid::Uuid;

fn depths() -> Depths {
    Depths::testdrive()
}

/// 9.1 / never locks: honest Alice still in Setup, nothing at risk.
#[test]
fn counterparty_never_locks_alice_loses_nothing() {
    let s = SwapState::new(Role::Alice);
    assert!(matches!(s, SwapState::Setup { .. }));
}

/// Wrong lock / verify_lock failed: Bob aborts to cancel, recovers BTC.
#[test]
fn counterparty_locks_garbage_bob_cancels() {
    let d = depths();
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::VerifyLockFailed, d);
    assert!(matches!(
        s,
        SwapState::MustCancel {
            reason: AbortReason::VerifyLockFailed,
            ..
        }
    ));
}

/// Redeems too late: machine refuses ReadyToRedeem when btc is inside the margin.
#[test]
fn counterparty_redeems_late_window_is_closed() {
    let d = depths();
    let mut s = SwapState::new(Role::Alice);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::BtcConf(1), d);
    s = s.apply(SwapEvent::AlicePublishedNightLock, d);
    s = s.apply(SwapEvent::NightConf { night: 2, btc: 3 }, d);
    assert!(matches!(
        s,
        SwapState::MustCancel {
            reason: AbortReason::RedeemTooCloseToH1,
            ..
        }
    ));
}

/// Silent after cancel: Alice can punish. NIGHT may be stuck (the wart).
/// BTC is not stolen from an honest Bob who refunds; it is the punish path
/// if he does not. Honest Bob who refunds is whole.
#[test]
fn counterparty_silent_after_cancel_refund_keeps_bob_whole() {
    let d = depths();
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, d);
    s = s.apply(SwapEvent::AliceNeverLocked, d);
    s = s.apply(SwapEvent::CancelConfirmed, d);
    s = s.apply(SwapEvent::RefundConfirmed, d);
    assert!(matches!(s, SwapState::Refunded { .. }));
}

/// Stale packet: amounts changed. Honest party refuses, does not lock.
#[test]
fn stale_or_mutated_packet_is_refused() {
    let id = Uuid::nil();
    let amounts = Amounts {
        night_darks: 5,
        btc_sats: 80_000,
        btc_fee_sats: 400,
    };
    let p = Packet::new(
        NetworkId::Devnet,
        id,
        0,
        amounts.clone(),
        serde_json::json!({}),
    );
    let mut other = amounts.clone();
    other.btc_sats = 1;
    assert_eq!(
        p.verify_open(id, NetworkId::Devnet, 0, &other),
        Err(PacketError::AmountChanged)
    );
}

#[test]
fn stored_swap_carries_the_wart_after_a_crash() {
    let s = StoredSwap::new(SwapState::new(Role::Alice), 1, 2);
    assert!(s.wart.contains("no NIGHT refund") || s.wart.contains("stuck forever"));
}
