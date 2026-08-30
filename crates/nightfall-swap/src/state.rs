//! Swap state machine. Persist after every transition. Resume must land
//! in cancel/refund, never in "wait for the peer" after H₁.

use crate::timelock::Depths;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::messages::Role;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SwapState {
    /// Phase 0. Nothing locked.
    Setup { id: Uuid, role: Role },
    /// Bob has published TX_lock. Waiting for bitcoin depth.
    BtcLocked {
        id: Uuid,
        role: Role,
        lock_confirms: u32,
    },
    /// Alice has published TX_night_lock. Waiting for NIGHT depth.
    NightLocked {
        id: Uuid,
        role: Role,
        lock_confirms: u32,
        night_confirms: u64,
    },
    /// Alice is allowed to redeem. Still before H₁ minus margin.
    ReadyToRedeem { id: Uuid, role: Role },
    /// Alice broadcast TX_redeem. s_a is public.
    Redeeming { id: Uuid, role: Role },
    /// Happy path done.
    Done { id: Uuid, role: Role },
    /// Too close to H₁, or Alice never locked. Wait for cancel.
    MustCancel {
        id: Uuid,
        role: Role,
        reason: AbortReason,
    },
    /// TX_cancel confirmed. Bob should refund immediately.
    Cancelled { id: Uuid, role: Role },
    /// TX_refund confirmed. s_b public. Alice can claim NIGHT if she locked.
    Refunded { id: Uuid, role: Role },
    /// TX_punish confirmed. NIGHT is stuck if Alice locked. The wart.
    Punished { id: Uuid, role: Role },
    Failed {
        id: Uuid,
        role: Role,
        reason: AbortReason,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AbortReason {
    BobNeverLocked,
    AliceNeverLocked,
    RedeemTooCloseToH1,
    CounterpartyGone,
    Crash,
    VerifyLockFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapEvent {
    BobPublishedLock,
    BtcConf(u32),
    AlicePublishedNightLock,
    /// NIGHT confirmations of the lock — **and the current Bitcoin height**.
    ///
    /// The Bitcoin figure is not decoration. Entering `ReadyToRedeem` used to
    /// be decided against `lock_confirms`, the *last seen* Bitcoin depth. A
    /// driver that stopped sending `BtcConf` therefore let a stale value keep
    /// the redeem window open past H₁ — which is spec §9.2, the theft where
    /// Alice redeems too late, Bob cancels, and takes both sides.
    ///
    /// Grok's fix made `NightLocked` react to `BtcConf`, which closes the hole
    /// *when the event arrives*. Carrying the height here closes it when the
    /// event does not: a caller cannot advance the swap without saying how
    /// deep Bitcoin is, so the contract is enforced by the type rather than by
    /// discipline. There is currently no driver at all in the wallet, which is
    /// exactly why this must not depend on one behaving well.
    NightConf {
        night: u64,
        btc: u32,
    },
    AliceRedeemed,
    BobClaimedNight,
    /// Remaining window to H₁ is inside the margin.
    TooCloseToCancel,
    CancelConfirmed,
    RefundConfirmed,
    PunishConfirmed,
    AliceNeverLocked,
    VerifyLockFailed,
    Crash,
}

impl SwapState {
    pub fn new(role: Role) -> Self {
        Self::Setup {
            id: Uuid::new_v4(),
            role,
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            Self::Setup { id, .. }
            | Self::BtcLocked { id, .. }
            | Self::NightLocked { id, .. }
            | Self::ReadyToRedeem { id, .. }
            | Self::Redeeming { id, .. }
            | Self::Done { id, .. }
            | Self::MustCancel { id, .. }
            | Self::Cancelled { id, .. }
            | Self::Refunded { id, .. }
            | Self::Punished { id, .. }
            | Self::Failed { id, .. } => *id,
        }
    }

    pub fn role(&self) -> Role {
        match self {
            Self::Setup { role, .. }
            | Self::BtcLocked { role, .. }
            | Self::NightLocked { role, .. }
            | Self::ReadyToRedeem { role, .. }
            | Self::Redeeming { role, .. }
            | Self::Done { role, .. }
            | Self::MustCancel { role, .. }
            | Self::Cancelled { role, .. }
            | Self::Refunded { role, .. }
            | Self::Punished { role, .. }
            | Self::Failed { role, .. } => *role,
        }
    }

    /// After a crash, if bitcoin lock confirms ≥ H₁, go to MustCancel.
    /// Restart. A crash is not information about the counterparty, so the only
    /// question is how much of the H₁ window we slept through.
    ///
    /// `Redeeming` is deliberately **not** in the list below, for the same
    /// reason `TooCloseToCancel` does not reach it in `apply`: entering
    /// `Redeeming` means `AliceRedeemed` fired, which means TX_redeem was
    /// published, which means `s_a` is already public. Bob can claim the NIGHT
    /// whatever we do next. Sending Alice to `MustCancel` from there makes her
    /// broadcast TX_cancel — the transaction whose child TX_refund pays Bob's
    /// Bitcoin back to Bob. She would hand back the coin she is owed while he
    /// keeps the NIGHT: a loss produced by the recovery path, not by the
    /// counterparty. With `s_a` out, the only thing left to salvage is the
    /// Bitcoin, and the only way to salvage it is to keep pressing the redeem.
    ///
    /// The same hazard therefore has two doors. `apply` closed one; this is
    /// the other. Pinned by `a_crash_while_redeeming_does_not_hand_the_coin_back`.
    pub fn resume(self, depths: Depths, btc_lock_confirms: u32) -> Self {
        match self {
            Self::BtcLocked { id, role, .. }
            | Self::NightLocked { id, role, .. }
            | Self::ReadyToRedeem { id, role }
                if btc_lock_confirms >= depths.cancel =>
            {
                Self::MustCancel {
                    id,
                    role,
                    reason: AbortReason::Crash,
                }
            }
            other => other,
        }
    }

    pub fn apply(&self, ev: SwapEvent, depths: Depths) -> Self {
        let id = self.id();
        let role = self.role();
        match (self, ev) {
            (Self::Setup { .. }, SwapEvent::BobPublishedLock) => Self::BtcLocked {
                id,
                role,
                lock_confirms: 0,
            },
            (Self::Setup { .. }, SwapEvent::AliceNeverLocked) => Self::MustCancel {
                id,
                role,
                reason: AbortReason::AliceNeverLocked,
            },
            (Self::BtcLocked { .. }, SwapEvent::BtcConf(n)) => {
                if n >= depths.cancel {
                    Self::MustCancel {
                        id,
                        role,
                        reason: AbortReason::AliceNeverLocked,
                    }
                } else {
                    Self::BtcLocked {
                        id,
                        role,
                        lock_confirms: n,
                    }
                }
            }
            (Self::BtcLocked { lock_confirms, .. }, SwapEvent::AlicePublishedNightLock)
                if *lock_confirms >= depths.bitcoin =>
            {
                if !depths.may_redeem(*lock_confirms) {
                    Self::MustCancel {
                        id,
                        role,
                        reason: AbortReason::RedeemTooCloseToH1,
                    }
                } else {
                    Self::NightLocked {
                        id,
                        role,
                        lock_confirms: *lock_confirms,
                        night_confirms: 0,
                    }
                }
            }
            (Self::BtcLocked { .. }, SwapEvent::VerifyLockFailed) => Self::MustCancel {
                id,
                role,
                reason: AbortReason::VerifyLockFailed,
            },
            (Self::BtcLocked { .. }, SwapEvent::AliceNeverLocked) => Self::MustCancel {
                id,
                role,
                reason: AbortReason::AliceNeverLocked,
            },
            (Self::NightLocked { night_confirms, .. }, SwapEvent::BtcConf(n)) => {
                if n >= depths.cancel || !depths.may_redeem(n) {
                    Self::MustCancel {
                        id,
                        role,
                        reason: AbortReason::RedeemTooCloseToH1,
                    }
                } else {
                    Self::NightLocked {
                        id,
                        role,
                        lock_confirms: n,
                        night_confirms: *night_confirms,
                    }
                }
            }
            // Decided against the Bitcoin height in *this* event, never against
            // the remembered one.
            (Self::NightLocked { .. }, SwapEvent::NightConf { night, btc })
                if night >= depths.night && depths.may_redeem(btc) =>
            {
                Self::ReadyToRedeem { id, role }
            }
            // Deep enough on NIGHT but too close to H₁ on Bitcoin: abort rather
            // than offer a redeem that cannot confirm in time.
            (Self::NightLocked { .. }, SwapEvent::NightConf { night, btc })
                if night >= depths.night =>
            {
                let _ = btc;
                Self::MustCancel {
                    id,
                    role,
                    reason: AbortReason::RedeemTooCloseToH1,
                }
            }
            (
                Self::NightLocked {
                    lock_confirms,
                    night_confirms,
                    ..
                },
                SwapEvent::NightConf { night, btc },
            ) => Self::NightLocked {
                id,
                role,
                lock_confirms: btc.max(*lock_confirms),
                night_confirms: night.max(*night_confirms),
            },
            (Self::ReadyToRedeem { .. }, SwapEvent::BtcConf(n))
                if n >= depths.cancel || !depths.may_redeem(n) =>
            {
                Self::MustCancel {
                    id,
                    role,
                    reason: AbortReason::RedeemTooCloseToH1,
                }
            }
            (
                Self::NightLocked { .. } | Self::ReadyToRedeem { .. },
                SwapEvent::TooCloseToCancel,
            ) => Self::MustCancel {
                id,
                role,
                reason: AbortReason::RedeemTooCloseToH1,
            },
            (Self::ReadyToRedeem { .. }, SwapEvent::AliceRedeemed) => Self::Redeeming { id, role },
            (Self::Redeeming { .. }, SwapEvent::BobClaimedNight) => Self::Done { id, role },
            (Self::MustCancel { .. }, SwapEvent::CancelConfirmed) => Self::Cancelled { id, role },
            (Self::Cancelled { .. }, SwapEvent::RefundConfirmed) => Self::Refunded { id, role },
            (Self::Cancelled { .. }, SwapEvent::PunishConfirmed) => Self::Punished { id, role },
            (s, _) => s.clone(),
        }
    }
}

#[cfg(test)]
mod driver_contract_tests {
    use super::*;

    fn locked(d: Depths) -> SwapState {
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s
    }

    /// The hole Grok's report named but could not close from inside the state
    /// machine: a driver that simply stops sending `BtcConf`.
    ///
    /// Before `NightConf` carried the Bitcoin height, the decision to open the
    /// redeem window used `lock_confirms` — the last value the driver happened
    /// to supply. A driver that went quiet left a stale figure saying "plenty
    /// of time" while H₁ came and went, and Alice redeemed into spec §9.2.
    ///
    /// Now the caller has to state the Bitcoin depth to advance at all, so
    /// there is no version of "forgot to send BtcConf" that still reaches
    /// `ReadyToRedeem`.
    #[test]
    fn a_driver_that_stops_reporting_bitcoin_cannot_open_the_redeem_window() {
        let d = Depths::testdrive();
        // Bitcoin has moved to 3 — inside the margin — but the swap was last
        // told 1. The event carries the truth, so the stale figure cannot win.
        let s = locked(d).apply(SwapEvent::NightConf { night: 2, btc: 3 }, d);

        assert!(
            matches!(
                s,
                SwapState::MustCancel {
                    reason: AbortReason::RedeemTooCloseToH1,
                    ..
                }
            ),
            "a fresh Bitcoin height inside the margin must abort, not redeem; got {s:?}"
        );
    }

    /// And the honest case still works, or the test above proves only that the
    /// machine refuses everything.
    #[test]
    fn a_driver_reporting_honestly_still_reaches_ready() {
        let d = Depths::testdrive();
        let s = locked(d).apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        assert!(
            matches!(s, SwapState::ReadyToRedeem { .. }),
            "with room to spare the swap must proceed; got {s:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depths() -> Depths {
        Depths::testdrive()
    }

    #[test]
    fn happy_path() {
        let d = depths();
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s = s.apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        assert!(matches!(s, SwapState::ReadyToRedeem { .. }));
        s = s.apply(SwapEvent::AliceRedeemed, d);
        s = s.apply(SwapEvent::BobClaimedNight, d);
        assert!(matches!(s, SwapState::Done { .. }));
    }

    #[test]
    fn section_9_1_bob_never_locks() {
        let s = SwapState::new(Role::Alice);
        assert!(matches!(s, SwapState::Setup { .. }));
    }

    #[test]
    fn section_9_2_redeem_too_close_to_h1() {
        let d = depths();
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
    }

    #[test]
    fn section_9_3_alice_never_locks() {
        let d = depths();
        let mut s = SwapState::new(Role::Bob);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(4), d);
        assert!(matches!(
            s,
            SwapState::MustCancel {
                reason: AbortReason::AliceNeverLocked,
                ..
            }
        ));
    }

    #[test]
    fn section_9_4_punish_is_the_wart() {
        let d = depths();
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s = s.apply(SwapEvent::TooCloseToCancel, d);
        s = s.apply(SwapEvent::CancelConfirmed, d);
        s = s.apply(SwapEvent::PunishConfirmed, d);
        assert!(matches!(s, SwapState::Punished { .. }));
    }

    #[test]
    fn section_9_4_refund_after_cancel() {
        let d = depths();
        let mut s = SwapState::new(Role::Bob);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::AliceNeverLocked, d);
        s = s.apply(SwapEvent::CancelConfirmed, d);
        s = s.apply(SwapEvent::RefundConfirmed, d);
        assert!(matches!(s, SwapState::Refunded { .. }));
    }

    #[test]
    fn section_9_6_crash_resumes_into_cancel() {
        let d = depths();
        let mut s = SwapState::new(Role::Bob);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s = s.resume(d, 4);
        assert!(matches!(
            s,
            SwapState::MustCancel {
                reason: AbortReason::Crash,
                ..
            }
        ));
    }

    #[test]
    fn verify_lock_failure_aborts_to_cancel() {
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

    /// H₁ can arrive while Alice is still waiting for NIGHT confirms.
    /// If BtcConf is ignored in NightLocked, NightConf later opens
    /// ReadyToRedeem after cancel is already valid — Alice redeems into
    /// Bob's cancel, he takes both (§9.2).
    #[test]
    fn btc_conf_during_night_wait_closes_the_redeem_window() {
        let d = depths();
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        assert!(matches!(s, SwapState::NightLocked { .. }));
        s = s.apply(SwapEvent::BtcConf(2), d);
        assert!(
            matches!(
                s,
                SwapState::MustCancel {
                    reason: AbortReason::RedeemTooCloseToH1,
                    ..
                }
            ),
            "may_redeem(2) is false on testdrive; the machine must abort, not wait"
        );
    }

    /// A completed swap must not be dragged back into cancel by a stray event.
    #[test]
    fn too_close_does_not_unwind_done_or_redeeming() {
        let d = depths();
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s = s.apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        s = s.apply(SwapEvent::AliceRedeemed, d);
        let redeeming = s.clone();
        s = s.apply(SwapEvent::TooCloseToCancel, d);
        assert!(
            matches!(s, SwapState::Redeeming { .. }),
            "Alice already published s_a; cancel is the §9.2 theft, not a recovery"
        );
        s = redeeming.apply(SwapEvent::BobClaimedNight, d);
        s = s.apply(SwapEvent::TooCloseToCancel, d);
        assert!(matches!(s, SwapState::Done { .. }));
    }

    /// The other door into the §9.2 theft.
    ///
    /// `too_close_does_not_unwind_done_or_redeeming` closes it in `apply`.
    /// `resume` is a second way into the same room, and until this test
    /// existed nothing walked it: the crash table exercised `Redeeming` only
    /// at one confirmation, never past H₁.
    ///
    /// Found by mutation — dropping `Redeeming` from `resume`'s arm changed
    /// nothing, so the arm was untested. Reading it then showed the arm was
    /// not merely untested but wrong.
    #[test]
    fn a_crash_while_redeeming_does_not_hand_the_coin_back() {
        let d = depths();
        let mut s = SwapState::new(Role::Alice);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::AlicePublishedNightLock, d);
        s = s.apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        s = s.apply(SwapEvent::AliceRedeemed, d);
        assert!(matches!(s, SwapState::Redeeming { .. }), "setup");

        // We were down long enough for H₁ to pass. s_a is public either way.
        let after = s.resume(d, d.cancel + 3);
        assert!(
            matches!(after, SwapState::Redeeming { .. }),
            "a restart past H1 must keep pressing the redeem, not cancel: \
             cancelling feeds TX_refund, which pays Bob's Bitcoin back to Bob \
             while he claims the NIGHT with the s_a we already published. \
             Got {after:?}"
        );

        // The states where nothing is published yet must still abort.
        let mut b = SwapState::new(Role::Bob);
        b = b.apply(SwapEvent::BobPublishedLock, d);
        b = b.apply(SwapEvent::BtcConf(1), d);
        assert!(
            matches!(
                b.clone().resume(d, d.cancel),
                SwapState::MustCancel {
                    reason: AbortReason::Crash,
                    ..
                }
            ),
            "BtcLocked past H1 must still abort"
        );
        let n = b.apply(SwapEvent::AlicePublishedNightLock, d);
        assert!(
            matches!(n.resume(d, d.cancel), SwapState::MustCancel { .. }),
            "NightLocked past H1 must still abort"
        );
    }

    /// Crash before H₁ must not abort. resume() is the path that looks at
    /// confirms; SwapEvent::Crash without a height is not a reason to cancel.
    #[test]
    fn crash_before_h1_does_not_abort() {
        let d = depths();
        let mut s = SwapState::new(Role::Bob);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        s = s.apply(SwapEvent::Crash, d);
        assert!(
            matches!(s, SwapState::BtcLocked { .. }),
            "a restart at conf=1 of 4 is not an abort"
        );
        s = s.resume(d, 1);
        assert!(matches!(s, SwapState::BtcLocked { .. }));
    }
}
