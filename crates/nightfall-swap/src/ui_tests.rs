//! Tests for [`crate::ui`]. Split into its own file because the decisions in
//! there are the ones that can cost a user coins, and a long test module
//! buried under the code it tests gets skimmed.

use super::*;
use crate::state::{Role, SwapEvent, SwapState};
use crate::timelock::Depths;
use nightfall_types::NetworkId;

fn d() -> Depths {
    Depths::testdrive()
}

/// Walk the machine to a named state instead of constructing it, so these
/// tests break if the transitions change under them.
fn at(step: &str) -> SwapState {
    let dd = d();
    let mut s = SwapState::new(Role::Alice);
    if step == "setup" {
        return s;
    }
    s = s.apply(SwapEvent::BobPublishedLock, dd);
    if step == "btc" {
        return s;
    }
    s = s.apply(SwapEvent::BtcConf(1), dd);
    s = s.apply(SwapEvent::AlicePublishedNightLock, dd);
    if step == "night" {
        return s;
    }
    s = s.apply(SwapEvent::NightConf { night: 2, btc: 1 }, dd);
    if step == "ready" {
        return s;
    }
    s = s.apply(SwapEvent::AliceRedeemed, dd);
    if step == "redeeming" {
        return s;
    }
    s = s.apply(SwapEvent::BobClaimedNight, dd);
    s
}

// --- the gate -------------------------------------------------------------

/// The reason this gate exists is written in the module doc. If someone ever
/// decides to open mainnet, they have to delete this test on purpose.
#[test]
fn mainnet_cannot_start_a_swap() {
    assert!(!availability(NetworkId::Mainnet).is_enabled());
    assert!(availability(NetworkId::Testnet).is_enabled());
    assert!(availability(NetworkId::Devnet).is_enabled());
}

#[test]
fn the_locked_message_says_why_and_what_to_do_instead() {
    match availability(NetworkId::Mainnet) {
        Availability::Locked { headline, detail } => {
            assert!(headline.contains("mainnet"));
            assert!(
                detail.contains("leaf") && detail.contains("outside"),
                "the reason must be the unreviewed leaf, not a vague 'experimental'"
            );
            assert!(
                detail.contains("testnet"),
                "a refusal without an alternative is just a wall"
            );
        }
        Availability::Enabled => panic!("mainnet must be locked"),
    }
}

// --- deadlines ------------------------------------------------------------

/// The same rule the driver follows: an unanswered query is not zero.
/// Rendering "0 confirmations" as a calm, nearly-full window would tell the
/// user they have a day when they may have minutes.
#[test]
fn an_unreachable_node_is_not_a_calm_deadline() {
    let out = deadlines(&at("night"), d(), None, Some(2));
    let h1 = out.iter().find(|x| x.label.starts_with("H1")).unwrap();
    assert_eq!(h1.urgency, Urgency::Act);
    assert!(
        h1.detail.contains("unreachable"),
        "say the node is down, do not invent a number: {}",
        h1.detail
    );
    assert_eq!(h1.fraction, 0.0);
}

#[test]
fn the_h1_bar_fills_as_the_window_closes() {
    let calm = deadlines(&at("btc"), d(), Some(0), None);
    let late = deadlines(&at("btc"), d(), Some(3), None);
    let a = &calm[0];
    let b = &late[0];
    assert!(a.fraction < b.fraction);
    assert!(b.urgency > a.urgency, "closer must read as more urgent");
}

/// Inside the margin the machine refuses to redeem. The bar must say so
/// before the user wonders why nothing is happening.
#[test]
fn inside_the_redeem_margin_the_bar_demands_attention() {
    let dd = d(); // cancel 4, margin 2 → may_redeem false from 2
    assert!(!dd.may_redeem(2));
    let out = deadlines(&at("ready"), dd, Some(2), None);
    assert_eq!(out[0].urgency, Urgency::Act);
}

#[test]
fn past_h1_is_marked_passed_not_merely_urgent() {
    let out = deadlines(&at("btc"), d(), Some(9), None);
    assert_eq!(out[0].urgency, Urgency::Passed);
    assert!(out[0].detail.contains("cancelled"));
}

#[test]
fn night_depth_is_shown_against_the_reorg_bound() {
    let out = deadlines(&at("night"), d(), Some(1), Some(1));
    let n = out.iter().find(|x| x.label.contains("NIGHT")).unwrap();
    assert!(n.detail.contains("reorg bound"));
}

#[test]
fn a_finished_swap_has_no_countdown() {
    assert!(deadlines(&at("done"), d(), Some(1), Some(2)).is_empty());
}

// --- actions --------------------------------------------------------------

/// The third door into the §9.2 theft. `apply` refuses it, `resume` refuses
/// it, and the button must not offer it either: once `s_a` is public,
/// cancelling hands the Bitcoin back while the counterparty keeps the NIGHT.
#[test]
fn redeeming_offers_no_cancel_button() {
    let a = actions(&at("redeeming"));
    assert!(
        !a.contains(&Action::CancelNow),
        "cancelling after s_a is public is the theft, not a rescue: {a:?}"
    );
    assert!(
        a.contains(&Action::Recover),
        "but a human must still be able to step in"
    );
}

#[test]
fn a_swap_that_can_still_be_abandoned_offers_cancel() {
    for step in ["setup", "btc", "night", "ready"] {
        assert!(
            actions(&at(step)).contains(&Action::CancelNow),
            "{step} should still be abandonable"
        );
    }
}

#[test]
fn a_finished_swap_offers_only_forget() {
    assert_eq!(actions(&at("done")), vec![Action::Forget]);
    assert!(is_finished(&at("done")));
    assert!(!is_finished(&at("ready")));
}

/// A destructive button that does not say what it costs is a trap.
#[test]
fn every_destructive_action_states_its_cost() {
    for a in [Action::CancelNow, Action::Recover] {
        assert!(a.is_destructive());
        let c = a.consequence();
        assert!(c.len() > 40, "{a:?} needs a real sentence, got {c:?}");
    }
    assert!(
        Action::CancelNow.consequence().contains("NIGHT"),
        "cancel must name the stuck NIGHT — that is the whole wart"
    );
}

// --- the draft ------------------------------------------------------------

fn draft(night: &str, btc: &str, fee: &str) -> Draft {
    Draft {
        give_night: true,
        night: night.into(),
        btc: btc.into(),
        btc_fee: fee.into(),
    }
}

#[test]
fn a_good_draft_becomes_amounts() {
    let a = validate(&draft("1.5", "200000", "800"), 500_000_000).unwrap();
    assert_eq!(a.night_darks, 150_000_000);
    assert_eq!(a.btc_sats, 200_000);
    assert_eq!(a.btc_fee_sats, 800);
}

/// Promising the same coin twice is the failure this guards. The caller
/// passes the *unreserved* balance, not the raw one.
#[test]
fn a_draft_above_the_unreserved_balance_is_refused() {
    let e = validate(&draft("10", "200000", "800"), 500_000_000).unwrap_err();
    assert!(matches!(e, DraftError::NightAboveBalance { .. }));
    assert!(e.to_string().contains("available"));
}

/// Bob does not lock NIGHT, so his balance is not the constraint.
#[test]
fn the_balance_check_only_applies_to_the_side_that_locks_night() {
    let mut d = draft("10", "200000", "800");
    d.give_night = false;
    assert!(validate(&d, 0).is_ok());
    assert_eq!(role_of(&d), Role::Bob);
    d.give_night = true;
    assert_eq!(role_of(&d), Role::Alice);
}

#[test]
fn a_fee_that_eats_the_amount_is_refused() {
    let e = validate(&draft("1", "800", "800"), u64::MAX).unwrap_err();
    assert!(matches!(e, DraftError::FeeAboveAmount { .. }));
}

#[test]
fn zero_is_not_an_amount() {
    assert_eq!(
        validate(&draft("0", "1000", "100"), u64::MAX).unwrap_err(),
        DraftError::NightZero
    );
    assert_eq!(
        validate(&draft("1", "0", "100"), u64::MAX).unwrap_err(),
        DraftError::BtcZero
    );
}

/// Nine decimals is not a rounding problem to paper over — it means the user
/// typed something they did not mean, and the wallet should say so rather
/// than silently drop a digit.
#[test]
fn more_precision_than_night_has_is_an_error_not_a_rounding() {
    assert_eq!(
        validate(&draft("1.123456789", "1000", "100"), u64::MAX).unwrap_err(),
        DraftError::NightUnreadable
    );
    // Exactly eight is fine.
    assert!(validate(&draft("1.12345678", "1000", "100"), u64::MAX).is_ok());
}

#[test]
fn thin_spaces_from_our_own_formatter_round_trip() {
    // `night()` in the wallet groups with U+202F. Pasting its output back in
    // must work, or MAX-style buttons produce a value the form rejects.
    let a = validate(&draft("1\u{202F}234.5", "1000", "100"), u64::MAX).unwrap();
    assert_eq!(a.night_darks, 123_450_000_000);
}

#[test]
fn an_empty_fee_falls_back_to_the_ladder_not_to_zero() {
    let a = validate(&draft("1", "500000", ""), u64::MAX).unwrap();
    assert!(a.btc_fee_sats > 0, "a zero fee never confirms");
}

#[test]
fn every_draft_error_says_something_a_human_can_act_on() {
    let cases = [
        validate(&draft("", "1", "1"), 0),
        validate(&draft("x", "1", "1"), 0),
        validate(&draft("1", "", "1"), u64::MAX),
        validate(&draft("1", "x", "1"), u64::MAX),
        validate(&draft("1", "1000", "x"), u64::MAX),
    ];
    for c in cases {
        let e = c.unwrap_err();
        let s = e.to_string();
        assert!(s.len() > 15 && s.ends_with('.'), "weak message: {s:?}");
    }
}

// --- timeline -------------------------------------------------------------

/// The happy path must advance by exactly one step per transition. If it
/// jumps or stalls, the picture lies about where the swap is.
#[test]
fn the_progress_track_advances_one_step_at_a_time() {
    let steps = ["setup", "btc", "night", "ready", "redeeming", "done"];
    for (i, s) in steps.iter().enumerate() {
        let t = timeline(&at(s));
        assert_eq!(t.track, Track::Progress, "{s}");
        assert_eq!(t.current, i, "{s} should be step {i}");
        assert!(t.current < t.steps.len());
    }
}

#[test]
fn only_the_last_step_is_settled() {
    assert!(timeline(&at("done")).settled);
    for s in ["setup", "btc", "night", "ready", "redeeming"] {
        assert!(!timeline(&at(s)).settled, "{s} is not over");
    }
}

/// An abort is a different chain, not a red dot on the happy one. Drawing it
/// on the progress track would suggest the swap is still going to complete.
#[test]
fn aborting_switches_track() {
    let dd = d();
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, dd);
    s = s.apply(SwapEvent::BtcConf(dd.cancel), dd);
    assert!(matches!(s, SwapState::MustCancel { .. }));
    let t = timeline(&s);
    assert_eq!(t.track, Track::Abort);
    assert!(!t.settled, "the abort has only started");

    let c = s.apply(SwapEvent::CancelConfirmed, dd);
    assert_eq!(timeline(&c).current, 1);
    let r = c.apply(SwapEvent::RefundConfirmed, dd);
    assert!(timeline(&r).settled);
}

/// Both endings of the abort tree are endings, and they share the last step.
#[test]
fn refunded_and_punished_are_both_the_end_of_the_abort_track() {
    let dd = d();
    let mut s = SwapState::new(Role::Bob);
    s = s.apply(SwapEvent::BobPublishedLock, dd);
    s = s.apply(SwapEvent::BtcConf(dd.cancel), dd);
    let c = s.apply(SwapEvent::CancelConfirmed, dd);
    let refunded = c.clone().apply(SwapEvent::RefundConfirmed, dd);
    let punished = c.apply(SwapEvent::PunishConfirmed, dd);
    assert_eq!(timeline(&refunded).current, timeline(&punished).current);
    assert!(timeline(&refunded).settled && timeline(&punished).settled);
}

/// Adding a state to the machine without teaching the view about it would
/// otherwise show a blank timeline. Every state must land somewhere valid.
#[test]
fn every_state_maps_to_a_real_step() {
    let dd = d();
    let mut all = vec![SwapState::new(Role::Alice)];
    let mut s = SwapState::new(Role::Alice);
    for ev in [
        SwapEvent::BobPublishedLock,
        SwapEvent::BtcConf(1),
        SwapEvent::AlicePublishedNightLock,
        SwapEvent::NightConf { night: 2, btc: 1 },
        SwapEvent::AliceRedeemed,
        SwapEvent::BobClaimedNight,
    ] {
        s = s.apply(ev, dd);
        all.push(s.clone());
    }
    let mut b = SwapState::new(Role::Bob);
    b = b.apply(SwapEvent::BobPublishedLock, dd);
    b = b.apply(SwapEvent::BtcConf(dd.cancel), dd);
    all.push(b.clone());
    b = b.apply(SwapEvent::CancelConfirmed, dd);
    all.push(b.clone());
    all.push(b.clone().apply(SwapEvent::RefundConfirmed, dd));
    all.push(b.apply(SwapEvent::PunishConfirmed, dd));

    for st in all {
        let t = timeline(&st);
        assert!(
            t.current < t.steps.len(),
            "{st:?} points past the end of its track"
        );
    }
}

// --- funding --------------------------------------------------------------

fn amounts() -> Amounts {
    Amounts {
        night_darks: 250_000_000,
        btc_sats: 400_000,
        btc_fee_sats: 1_000,
    }
}

fn funding_draft(txid: &str, vout: &str, value: &str) -> FundingDraft {
    FundingDraft {
        txid: txid.into(),
        vout: vout.into(),
        value: value.into(),
        change_address: String::new(),
    }
}

const TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn a_good_funding_input_parses() {
    let f = validate_funding(&funding_draft(TXID, "0", "500000"), &amounts()).unwrap();
    assert_eq!(f.vout, 0);
    assert_eq!(f.value_sats, 500_000);
    assert_eq!(f.txid, TXID);
}

/// `TxLock::from_prevout` refuses a prevout that cannot cover value plus fee,
/// but only once the swap is already under way. Catching it in the form is
/// the difference between a sentence and a confusing failure.
#[test]
fn an_output_too_small_for_the_lock_is_refused_before_the_swap_starts() {
    let a = amounts();
    let need = a.btc_sats + a.btc_fee_sats;
    let e = validate_funding(&funding_draft(TXID, "0", &(need - 1).to_string()), &a).unwrap_err();
    assert_eq!(
        e,
        FundingError::TooSmall {
            have: need - 1,
            need
        }
    );
    assert!(e.to_string().contains(&need.to_string()));

    // Exactly enough is enough: no change, everything above the amount is fee.
    assert!(validate_funding(&funding_draft(TXID, "0", &need.to_string()), &a).is_ok());
}

/// A txid is 64 hex characters. Anything else is a typo, and a typo here
/// builds a lock spending an output that does not exist.
#[test]
fn a_txid_that_is_not_a_txid_is_refused() {
    let a = amounts();
    for bad in ["", "abc", &"1".repeat(63), &"1".repeat(65), &"z".repeat(64)] {
        assert!(
            validate_funding(&funding_draft(bad, "0", "500000"), &a).is_err(),
            "{bad:?} must not pass as a transaction id"
        );
    }
    // Upper case is fine, and is normalised so two spellings are one id.
    let upper = "A".repeat(64);
    let f = validate_funding(&funding_draft(&upper, "0", "500000"), &a).unwrap();
    assert_eq!(f.txid, upper.to_lowercase());
}

#[test]
fn every_funding_error_says_something_a_human_can_act_on() {
    let a = amounts();
    let cases = [
        validate_funding(&funding_draft("", "0", "1"), &a),
        validate_funding(&funding_draft(TXID, "", "1"), &a),
        validate_funding(&funding_draft(TXID, "x", "1"), &a),
        validate_funding(&funding_draft(TXID, "0", ""), &a),
        validate_funding(&funding_draft(TXID, "0", "x"), &a),
    ];
    for c in cases {
        let s = c.unwrap_err().to_string();
        assert!(s.len() > 15 && s.ends_with('.'), "weak message: {s:?}");
    }
}

/// Dust change is not change — it goes to the miner. Showing the user a
/// change figure they will never receive would be a lie on the screen.
#[test]
fn dust_change_is_not_reported_as_change() {
    let a = amounts();
    let spent = a.btc_sats + a.btc_fee_sats;

    let plenty =
        validate_funding(&funding_draft(TXID, "0", &(spent + 5_000).to_string()), &a).unwrap();
    assert_eq!(change_after_lock(&plenty, &a), Some(5_000));

    let dusty =
        validate_funding(&funding_draft(TXID, "0", &(spent + 100).to_string()), &a).unwrap();
    assert_eq!(change_after_lock(&dusty, &a), None);

    let exact = validate_funding(&funding_draft(TXID, "0", &spent.to_string()), &a).unwrap();
    assert_eq!(change_after_lock(&exact, &a), None);
}

// --- the abort buttons ----------------------------------------------------

/// After the cancel the two sides need different transactions, and neither
/// can complete the other's. Offering the wrong button would be offering
/// something that fails at the moment a user is already anxious.
#[test]
fn after_the_cancel_each_side_is_offered_only_what_it_can_send() {
    let dd = d();
    let mut b = SwapState::new(Role::Bob);
    b = b.apply(SwapEvent::BobPublishedLock, dd);
    b = b.apply(SwapEvent::BtcConf(dd.cancel), dd);
    let bob_cancelled = b.apply(SwapEvent::CancelConfirmed, dd);
    assert!(matches!(bob_cancelled, SwapState::Cancelled { .. }));

    let bobs = actions(&bob_cancelled);
    assert!(bobs.contains(&Action::SendRefund), "Bob refunds: {bobs:?}");
    assert!(
        !bobs.contains(&Action::SendPunish),
        "Bob cannot punish — the punish path is Alice's: {bobs:?}"
    );

    let mut a = SwapState::new(Role::Alice);
    a = a.apply(SwapEvent::BobPublishedLock, dd);
    a = a.apply(SwapEvent::BtcConf(dd.cancel), dd);
    let alice_cancelled = a.apply(SwapEvent::CancelConfirmed, dd);
    let alices = actions(&alice_cancelled);
    assert!(
        alices.contains(&Action::SendPunish),
        "Alice punishes: {alices:?}"
    );
    assert!(
        !alices.contains(&Action::SendRefund),
        "Alice cannot refund — Bob's half is an adaptor she cannot open: {alices:?}"
    );
}

/// A swap already heading for the exit still needs the cancel button, or a
/// user whose driver is not running has no way to start the abort at all.
#[test]
fn must_cancel_offers_the_cancel() {
    let dd = d();
    let mut b = SwapState::new(Role::Bob);
    b = b.apply(SwapEvent::BobPublishedLock, dd);
    b = b.apply(SwapEvent::BtcConf(dd.cancel), dd);
    assert!(matches!(b, SwapState::MustCancel { .. }));
    assert!(actions(&b).contains(&Action::CancelNow));
}

/// Punish takes someone else's coin. It has to say what it does and does not
/// fix, because the tempting misreading is "this gets my NIGHT back".
#[test]
fn punish_says_it_does_not_unstick_the_night() {
    let c = Action::SendPunish.consequence();
    assert!(Action::SendPunish.is_destructive());
    assert!(
        c.contains("NIGHT") && c.contains("stuck"),
        "punish must name what it does not fix: {c:?}"
    );
    assert!(
        Action::SendRefund.consequence().contains("s_b"),
        "refund must say that it publishes s_b"
    );
}
