//! Event loop. Query both chains, persist, then act. A failed query does
//! nothing. A send is recorded on disk before it hits the wire.

use crate::persist::{save, PendingSend, SendKind, StoredSwap};
use crate::state::{Role, SwapEvent, SwapState};
use crate::timelock::Depths;
use crate::watch::{BroadcastResult, Broadcaster, ChainWatch, MempoolAccept, TxRef, WatchError};
use std::path::Path;
use thiserror::Error;

pub const DEFAULT_TICK_SECS: u64 = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tick {
    Idle,
    /// Disk already records `pending`. The caller broadcasts, then reports
    /// [`Session::note_broadcast`].
    Broadcast {
        kind: SendKind,
        txid: String,
        raw_hex: String,
    },
    NeedsAttention {
        why: String,
    },
    Advanced,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TickError {
    #[error("{0}")]
    Watch(#[from] WatchError),
    #[error("persist: {0}")]
    Persist(String),
}

pub struct Session {
    pub stored: StoredSwap,
    pub depths: Depths,
    pub datadir: std::path::PathBuf,
    /// Raw hex the actor would send for each kind. Tests inject; a live
    /// wallet fills these from pre-signed transactions.
    pub raw: std::collections::HashMap<SendKind, (String, String)>, // kind -> (txid, hex)
}

impl Session {
    pub fn open(datadir: &Path, stored: StoredSwap, depths: Depths) -> Self {
        Self {
            stored,
            depths,
            datadir: datadir.to_path_buf(),
            raw: std::collections::HashMap::new(),
        }
    }

    fn persist(&self) -> Result<(), TickError> {
        save(&self.datadir, &self.stored).map_err(|e| TickError::Persist(e.to_string()))
    }

    /// R1: both chains in one pass. R3: either error → no mutation.
    pub fn snapshot(
        btc: &dyn ChainWatch,
        night: &dyn ChainWatch,
    ) -> Result<(u64, u64), WatchError> {
        let btc_h = btc.height();
        let night_h = night.height();
        match (btc_h, night_h) {
            (Ok(b), Ok(n)) => Ok((b, n)),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }

    pub fn tick(
        &mut self,
        btc: &dyn ChainWatch,
        night: &dyn ChainWatch,
    ) -> Result<Tick, TickError> {
        let before = self.stored.state.clone();
        let (btc_h, night_h) = match Self::snapshot(btc, night) {
            Ok(v) => v,
            Err(e) => {
                // Must not apply. Caller sees the error; state is untouched.
                debug_assert_eq!(self.stored.state, before);
                return Err(e.into());
            }
        };
        let _ = (btc_h, night_h);

        // Lock depths are read at most once per tick. A pending *cancel*
        // must still be re-offered if the lock lookup is down — so this is
        // filled lazily, not before the pending branch.
        let mut btc_lock_confs = None;
        let mut night_lock_confs = None;
        let mut locks_loaded = false;

        if let Some(p) = self.stored.pending.clone() {
            match btc.confirmations(&TxRef { id: p.txid.clone() }) {
                Ok(Some(_)) => {
                    self.stored.pending = None;
                    self.apply_send_confirmed(p.kind);
                    self.persist()?;
                    return Ok(Tick::Advanced);
                }
                Ok(None) => {
                    // Still unknown. Re-offering it is right for the abort
                    // transactions — we want those to land — but not for the
                    // redeem.
                    //
                    // A redeem that has not confirmed is a decision made
                    // against an older chain. Blocks arrive between ticks, and
                    // once the H1 margin is gone, re-broadcasting is how Alice
                    // publishes s_a into a race she can lose: Bob's cancel
                    // confirms first, he claims the NIGHT with the leaked
                    // scalar, and refunds his Bitcoin. She loses both sides.
                    //
                    // So the redeem is withdrawn and the deadline re-read.
                    // Found by `a_block_arriving_between_ticks_withdraws_the_redeem`,
                    // which is the whole reason regtest is not enough: there
                    // the chain never moves unless a test moves it.
                    if p.kind == SendKind::Redeem {
                        self.load_lock_confs(
                            btc,
                            night,
                            &mut btc_lock_confs,
                            &mut night_lock_confs,
                        )?;
                        locks_loaded = true;
                    }
                    if p.kind == SendKind::Redeem && !self.redeem_still_allowed(btc_lock_confs) {
                        self.stored.pending = None;
                        self.persist()?;
                        // Fall through: the fresh depth below turns this into
                        // MustCancel rather than leaving the swap idle.
                    } else if let Some((txid, hex)) = self.raw.get(&p.kind).cloned() {
                        return Ok(Tick::Broadcast {
                            kind: p.kind,
                            txid,
                            raw_hex: hex,
                        });
                    } else {
                        return Ok(Tick::Idle);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        if !locks_loaded {
            self.load_lock_confs(btc, night, &mut btc_lock_confs, &mut night_lock_confs)?;
        }

        let ev = self.event_from_chain(btc_lock_confs, night_lock_confs);
        if let Some(ev) = ev {
            let next = self.stored.state.apply(ev, self.depths);
            if next != self.stored.state {
                self.stored.state = next;
                self.persist()?;
            }
        }

        if let Some(tick) = self.maybe_intend_send(btc_lock_confs.map(|c| c as u32))? {
            return Ok(tick);
        }
        Ok(Tick::Idle)
    }

    fn load_lock_confs(
        &self,
        btc: &dyn ChainWatch,
        night: &dyn ChainWatch,
        btc_lock: &mut Option<u64>,
        night_lock: &mut Option<u64>,
    ) -> Result<(), TickError> {
        *btc_lock = match &self.stored.btc_lock_txid {
            Some(id) => btc.confirmations(&TxRef { id: id.clone() })?,
            None => None,
        };
        *night_lock = match &self.stored.night_lock_id {
            Some(id) => night.confirmations(&TxRef { id: id.clone() })?,
            None => None,
        };
        Ok(())
    }

    /// Is the redeem window still open, measured from this tick's reading?
    ///
    /// An unanswerable query is not a yes. The same rule the rest of the
    /// driver follows: if we cannot ask, we do not act. A missing lock id
    /// is also not a yes — without a lock there is nothing to redeem, and
    /// the old `Ok(true)` here would have re-broadcast a pending redeem
    /// after the id was lost from disk.
    fn redeem_still_allowed(&self, lock_confs: Option<u64>) -> bool {
        match lock_confs {
            Some(c) => self.depths.may_redeem(c as u32),
            None => false,
        }
    }

    fn event_from_chain(
        &self,
        btc_lock: Option<u64>,
        night_lock: Option<u64>,
    ) -> Option<SwapEvent> {
        match &self.stored.state {
            SwapState::BtcLocked { .. } => btc_lock.map(|n| SwapEvent::BtcConf(n as u32)),
            SwapState::NightLocked { .. } => match (night_lock, btc_lock) {
                (Some(n), Some(b)) => Some(SwapEvent::NightConf {
                    night: n,
                    btc: b as u32,
                }),
                (_, Some(b)) => Some(SwapEvent::BtcConf(b as u32)),
                _ => None,
            },
            SwapState::ReadyToRedeem { .. } => btc_lock.map(|n| SwapEvent::BtcConf(n as u32)),
            SwapState::MustCancel { .. } => None,
            _ => None,
        }
    }

    fn apply_send_confirmed(&mut self, kind: SendKind) {
        let ev = match kind {
            SendKind::Redeem => SwapEvent::AliceRedeemed,
            SendKind::Cancel => SwapEvent::CancelConfirmed,
            SendKind::Refund => SwapEvent::RefundConfirmed,
            SendKind::Punish => SwapEvent::PunishConfirmed,
            SendKind::NightClaim => SwapEvent::BobClaimedNight,
        };
        self.stored.state = self.stored.state.apply(ev, self.depths);
    }

    /// R2: write pending to disk, then tell the caller to broadcast.
    ///
    /// `btc_lock_confs` is the depth read *this tick*, not a remembered one,
    /// and `None` means the node could not be asked. The redeem is the only
    /// irreversible step here, so it is the only one gated on that figure:
    /// publishing `s_a` when we cannot see how much of the cancel window is
    /// left is a bet with no way back.
    fn maybe_intend_send(
        &mut self,
        btc_lock_confs: Option<u32>,
    ) -> Result<Option<Tick>, TickError> {
        let kind = match (&self.stored.state, self.stored.state.role()) {
            (SwapState::ReadyToRedeem { .. }, Role::Alice) => {
                // Being in `ReadyToRedeem` is not a licence. The state was
                // entered against an older chain; the window is re-checked
                // against the freshest reading every single tick.
                if !btc_lock_confs.is_some_and(|c| self.depths.may_redeem(c)) {
                    return Ok(None);
                }
                SendKind::Redeem
            }
            // Cancel, refund and punish are not gated on a depth we re-read.
            // Cancel: the later the better. Refund: send as soon as Cancelled.
            // Punish: BIP68 on TX_punish *is* the H₂ gate. The node refuses
            // `non-BIP68-final`; we name that in `explain_broadcast_reject`.
            // We do not pre-filter punish here — we do not store the cancel's
            // confirmation count, and a guessed wait would delay a real one.
            (SwapState::MustCancel { .. }, _) => SendKind::Cancel,
            (SwapState::Cancelled { .. }, Role::Bob) => SendKind::Refund,
            (SwapState::Cancelled { .. }, Role::Alice) => SendKind::Punish,
            (SwapState::Redeeming { .. }, Role::Bob) => SendKind::NightClaim,
            _ => return Ok(None),
        };
        let Some((txid, hex)) = self.raw.get(&kind).cloned() else {
            return Ok(None);
        };
        self.stored.pending = Some(PendingSend {
            kind,
            txid: txid.clone(),
        });
        self.persist()?;
        Ok(Some(Tick::Broadcast {
            kind,
            txid,
            raw_hex: hex,
        }))
    }

    pub fn note_broadcast(&mut self, result: BroadcastResult) {
        match result {
            BroadcastResult::Accepted { .. } | BroadcastResult::AlreadyKnown { .. } => {
                // pending stays until confirmations() sees it
            }
        }
    }

    pub fn note_rejected(&mut self, why: String) -> Tick {
        Tick::NeedsAttention { why }
    }

    /// Crash/restart: load from disk, apply resume() against live confirms.
    pub fn recover(
        datadir: &Path,
        id: uuid::Uuid,
        depths: Depths,
        btc_lock_confirms: u32,
    ) -> Result<Self, TickError> {
        let stored =
            crate::persist::load(datadir, id).map_err(|e| TickError::Persist(e.to_string()))?;
        let mut s = Self::open(datadir, stored, depths);
        s.stored.state = s.stored.state.clone().resume(depths, btc_lock_confirms);
        s.persist()?;
        Ok(s)
    }
}

pub fn preflight(
    broadcaster: &dyn Broadcaster,
    raw_hex: &str,
) -> Result<MempoolAccept, WatchError> {
    broadcaster.test_accept(raw_hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Role;
    use crate::watch::FakeWatch;
    use uuid::Uuid;

    fn session(role: Role) -> (Session, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("nf-drv-{}", Uuid::new_v4()));
        let stored = StoredSwap::new(SwapState::new(role), 1, 2);
        let s = Session::open(&dir, stored, Depths::testdrive());
        (s, dir)
    }

    /// The dangerous half of the outage story, which the test below does not
    /// reach.
    ///
    /// `a_bitcoin_outage_does_not_move_state` sets `FakeWatch::outage`, which
    /// breaks *every* query — so `tick` fails at the opening `snapshot` call
    /// and never reaches the line that reads the lock's confirmations. That
    /// line was therefore untested. Replacing its `?` with
    /// `.unwrap_or(Some(0))` left all forty tests green.
    ///
    /// The consequence of swallowing it is not a stall, it is theft. In
    /// `NightLocked` the Bitcoin depth becomes the `btc` field of `NightConf`,
    /// and `may_redeem(0)` is **true**. So a fabricated zero reports "plenty
    /// of time before H₁" at the exact moment H₁ may already have passed —
    /// the stale-data hole that carrying the Bitcoin height was introduced to
    /// close.
    ///
    /// The partial outage this test uses is the realistic one: the tip still
    /// reads, the transaction lookup does not.
    #[test]
    fn an_outage_in_night_locked_cannot_fabricate_a_redeem_window() {
        let (mut s, dir) = session(Role::Alice);
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, s.depths);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(1), s.depths);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::AlicePublishedNightLock, s.depths);
        s.stored.btc_lock_txid = Some("lock".into());
        s.stored.night_lock_id = Some("nlock".into());
        s.persist().unwrap();
        assert!(
            matches!(s.stored.state, SwapState::NightLocked { .. }),
            "setup: expected NightLocked, got {:?}",
            s.stored.state
        );
        let before = s.stored.state.clone();

        // The tip answers. The transaction lookup does not. NIGHT is deep
        // enough that a fabricated "0 Bitcoin confirmations" would open the
        // redeem window.
        let mut btc = FakeWatch::new(10);
        btc.set_confs("lock", 9);
        btc.confs_outage = Some(WatchError::Unavailable("index rebuilding".into()));
        let mut night = FakeWatch::new(100);
        night.set_confs("nlock", 100);

        let outcome = s.tick(&btc, &night);

        // The substantive property first, so a regression names the theft
        // rather than the mechanism.
        assert!(
            !matches!(s.stored.state, SwapState::ReadyToRedeem { .. }),
            "an unreachable Bitcoin node opened the redeem window: a swallowed \
             lookup error became `btc: 0`, which reads as 'plenty of time \
             before H1'"
        );
        assert_eq!(
            s.stored.state, before,
            "R3: an unanswerable query must not advance the swap"
        );
        // And the mechanism: the caller is told, not quietly handed a guess.
        let err = outcome.expect_err("a failed lookup must surface as an error");
        assert!(
            matches!(err, TickError::Watch(WatchError::Unavailable(_))),
            "expected the lookup failure to propagate; got {err:?}"
        );

        // With the truth available — 9 of 10 blocks gone before H₁ — the swap
        // must abort rather than redeem.
        btc.confs_outage = None;
        s.tick(&btc, &night).unwrap();
        assert!(
            matches!(
                s.stored.state,
                SwapState::MustCancel {
                    reason: crate::AbortReason::RedeemTooCloseToH1,
                    ..
                }
            ),
            "too close to H1 must abort; got {:?}",
            s.stored.state
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bitcoin_outage_does_not_move_state() {
        let (mut s, dir) = session(Role::Bob);
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, s.depths);
        s.stored.btc_lock_txid = Some("lock".into());
        s.persist().unwrap();
        let before = s.stored.state.clone();

        let mut btc = FakeWatch::new(10);
        btc.outage = Some(WatchError::Unavailable("down".into()));
        btc.set_confs("lock", 3);
        let night = FakeWatch::new(100);

        let err = s.tick(&btc, &night).unwrap_err();
        assert!(matches!(err, TickError::Watch(WatchError::Unavailable(_))));
        assert_eq!(s.stored.state, before, "R3: outage must not apply BtcConf");

        // And the next healthy tick still works.
        btc.outage = None;
        s.tick(&btc, &night).unwrap();
        match s.stored.state {
            SwapState::BtcLocked { lock_confirms, .. } => assert_eq!(lock_confirms, 3),
            ref other => panic!("expected BtcLocked, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn night_loading_blocks_the_tick() {
        let (mut s, dir) = session(Role::Alice);
        let btc = FakeWatch::new(10);
        let mut night = FakeWatch::new(5);
        night.loading = true;
        assert_eq!(
            s.tick(&btc, &night),
            Err(TickError::Watch(WatchError::Loading))
        );
        assert!(matches!(s.stored.state, SwapState::Setup { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn intend_send_hits_disk_before_the_wire() {
        let (mut s, dir) = session(Role::Alice);
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, s.depths);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(1), s.depths);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::AlicePublishedNightLock, s.depths);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::NightConf { night: 2, btc: 1 }, s.depths);
        s.raw
            .insert(SendKind::Redeem, ("txid-r".into(), "deadbeef".into()));

        // A swap cannot reach `ReadyToRedeem` without a Bitcoin lock, so the
        // watcher has to know about one. The original setup left it out, and
        // once the redeem became gated on a freshly read depth that gap made
        // this test fail for a reason that has nothing to do with what it
        // checks — R2, persist before send.
        s.stored.btc_lock_txid = Some("lock".into());
        let mut btc = FakeWatch::new(10);
        btc.set_confs("lock", 1);
        assert!(
            s.depths.may_redeem(1),
            "precondition: one confirmation is inside the redeem window"
        );

        let tick = s.tick(&btc, &FakeWatch::new(10)).unwrap();
        match tick {
            Tick::Broadcast { kind, txid, .. } => {
                assert_eq!(kind, SendKind::Redeem);
                assert_eq!(txid, "txid-r");
            }
            other => panic!("{other:?}"),
        }
        let loaded = crate::persist::load(&dir, s.stored.state.id()).unwrap();
        assert_eq!(
            loaded.pending.as_ref().map(|p| p.txid.as_str()),
            Some("txid-r"),
            "R2: pending must be on disk before the caller sends"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn broadcasting_twice_is_already_known() {
        let mut btc = FakeWatch::new(10);
        let first = btc.broadcast_mut("x", "aa").unwrap();
        let second = btc.broadcast_mut("x", "aa").unwrap();
        assert!(matches!(first, BroadcastResult::Accepted { .. }));
        assert!(matches!(second, BroadcastResult::AlreadyKnown { .. }));
        assert_eq!(btc.sent.len(), 1, "the chain saw the tx once");
    }

    /// The chain moving under our feet — the class of problem regtest hides.
    ///
    /// On regtest the chain only grows when a test says so, so every decision
    /// is made against a chain that politely waits. On a real network blocks
    /// arrive between one tick and the next, and the dangerous moment is
    /// exactly here: a swap that was safe to redeem a minute ago may not be
    /// now.
    ///
    /// The driver must re-read and re-decide on every tick. Entering
    /// `ReadyToRedeem` once must not license a redeem forever.
    #[test]
    fn a_block_arriving_between_ticks_withdraws_the_redeem() {
        let (mut s, dir) = session(Role::Alice);
        let d = s.depths;
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, d);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(1), d);
        s.stored.state = s.stored.state.apply(SwapEvent::AlicePublishedNightLock, d);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        assert!(matches!(s.stored.state, SwapState::ReadyToRedeem { .. }));
        s.stored.btc_lock_txid = Some("lock".into());
        s.stored.night_lock_id = Some("nlock".into());
        // A redeem is ready to go out.
        s.raw
            .insert(SendKind::Redeem, ("redeem".into(), "00".into()));
        s.persist().unwrap();

        // Tick one: still early. The driver offers the redeem.
        let mut btc = FakeWatch::new(10);
        btc.set_confs("lock", 1);
        let mut night = FakeWatch::new(100);
        night.set_confs("nlock", 100);
        assert!(
            d.may_redeem(1),
            "precondition: one confirmation is still comfortably early"
        );
        match s.tick(&btc, &night).unwrap() {
            Tick::Broadcast {
                kind: SendKind::Redeem,
                ..
            } => {}
            other => panic!("expected the redeem to be offered, got {other:?}"),
        }

        // Blocks arrive. Nobody asked them to; that is the point.
        let inside = d.cancel - d.btc_redeem_margin;
        assert!(!d.may_redeem(inside), "precondition: now it is too late");
        btc.set_confs("lock", u64::from(inside));

        // Tick two must withdraw the offer, not repeat it.
        let after = s.tick(&btc, &night).unwrap();
        assert!(
            !matches!(
                after,
                Tick::Broadcast {
                    kind: SendKind::Redeem,
                    ..
                }
            ),
            "the driver offered a redeem after the window closed: {after:?}"
        );
        assert!(
            matches!(
                s.stored.state,
                SwapState::MustCancel {
                    reason: crate::AbortReason::RedeemTooCloseToH1,
                    ..
                }
            ),
            "and it must abort rather than sit still; got {:?}",
            s.stored.state
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The other half of the same rule: an abort transaction must keep being
    /// offered.
    ///
    /// Withdrawing the redeem when time runs out is right; doing the same to
    /// a cancel would be a disaster. The cancel is what a stuck swap needs in
    /// order to end, and the later it gets the more it is needed. This test
    /// exists so that fixing the redeem cannot quietly break the exit.
    #[test]
    fn a_pending_cancel_keeps_being_offered_however_late_it_gets() {
        let (mut s, dir) = session(Role::Bob);
        let d = s.depths;
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, d);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(d.cancel), d);
        assert!(matches!(s.stored.state, SwapState::MustCancel { .. }));
        s.stored.btc_lock_txid = Some("lock".into());
        s.raw
            .insert(SendKind::Cancel, ("cancel".into(), "00".into()));
        s.stored.pending = Some(PendingSend {
            kind: SendKind::Cancel,
            txid: "cancel".into(),
        });
        s.persist().unwrap();

        // Deep past H1 — exactly where a redeem would be withdrawn.
        let mut btc = FakeWatch::new(500);
        btc.set_confs("lock", u64::from(d.cancel) + 50);
        let night = FakeWatch::new(100);

        match s.tick(&btc, &night).unwrap() {
            Tick::Broadcast {
                kind: SendKind::Cancel,
                ..
            } => {}
            other => {
                panic!("a cancel that has not confirmed must keep being offered; got {other:?}")
            }
        }

        // And it must still be *pending*, not merely re-offered.
        //
        // Checking the returned tick alone was too weak: clearing the pending
        // record leaves `maybe_intend_send` to offer the cancel again anyway,
        // so the tick looks identical while the bookkeeping has forgotten that
        // we already handed it to the caller. Found by mutation.
        assert_eq!(
            s.stored.pending.as_ref().map(|p| p.kind),
            Some(SendKind::Cancel),
            "the cancel must stay on the pending list"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pending redeem while the lock's depth cannot be read.
    ///
    /// The sibling of `an_outage_in_night_locked_cannot_fabricate_a_redeem_window`,
    /// one branch further in. If we cannot ask how deep the lock is, we
    /// cannot know whether the window is still open — and "do not know" must
    /// not resolve to "go ahead". Publishing `s_a` on a guess is the one
    /// mistake with no way back.
    #[test]
    fn a_pending_redeem_is_withheld_while_the_depth_is_unknown() {
        let (mut s, dir) = session(Role::Alice);
        let d = s.depths;
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, d);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(1), d);
        s.stored.state = s.stored.state.apply(SwapEvent::AlicePublishedNightLock, d);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        s.stored.btc_lock_txid = Some("lock".into());
        s.raw
            .insert(SendKind::Redeem, ("redeem".into(), "00".into()));
        s.stored.pending = Some(PendingSend {
            kind: SendKind::Redeem,
            txid: "redeem".into(),
        });
        s.persist().unwrap();

        // The node answers about the tip but has never heard of either
        // transaction: neither the redeem nor the lock. Depth is unknown.
        let btc = FakeWatch::new(10);
        let night = FakeWatch::new(100);

        let out = s.tick(&btc, &night).unwrap();
        assert!(
            !matches!(
                out,
                Tick::Broadcast {
                    kind: SendKind::Redeem,
                    ..
                }
            ),
            "a redeem must not go out while the deadline cannot be read: {out:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// First tick, no pending yet: `maybe_intend_send` must still refuse
    /// when the lock's depth is unknown. The sibling of the test above,
    /// one branch earlier.
    #[test]
    fn a_redeem_is_not_intended_when_the_lock_depth_is_unknown() {
        let (mut s, dir) = session(Role::Alice);
        let d = s.depths;
        s.stored.state = s.stored.state.apply(SwapEvent::BobPublishedLock, d);
        s.stored.state = s.stored.state.apply(SwapEvent::BtcConf(1), d);
        s.stored.state = s.stored.state.apply(SwapEvent::AlicePublishedNightLock, d);
        s.stored.state = s
            .stored
            .state
            .apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
        s.stored.btc_lock_txid = Some("lock".into());
        s.raw
            .insert(SendKind::Redeem, ("redeem".into(), "00".into()));
        let btc = FakeWatch::new(10);
        let night = FakeWatch::new(100);
        let out = s.tick(&btc, &night).unwrap();
        assert!(
            !matches!(
                out,
                Tick::Broadcast {
                    kind: SendKind::Redeem,
                    ..
                }
            ),
            "maybe_intend_send must not offer a redeem on an unread lock: {out:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One row of the crash table: the state to kill in, the Bitcoin depth
    /// the node reports on restart, and what `recover` must produce.
    type CrashRow = (SwapState, u32, fn(&SwapState) -> bool);

    #[test]
    fn crash_table_every_state() {
        let d = Depths::testdrive();
        let dir = std::env::temp_dir().join(format!("nf-crash-{}", Uuid::new_v4()));
        let rows: [CrashRow; 8] = {
            let setup = SwapState::new(Role::Bob);
            let mut btc_locked = setup.clone().apply(SwapEvent::BobPublishedLock, d);
            btc_locked = btc_locked.apply(SwapEvent::BtcConf(1), d);
            let night = btc_locked
                .clone()
                .apply(SwapEvent::AlicePublishedNightLock, d);
            let ready = night
                .clone()
                .apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
            let redeeming = ready.clone().apply(SwapEvent::AliceRedeemed, d);
            let done = redeeming.clone().apply(SwapEvent::BobClaimedNight, d);
            let must = btc_locked.clone().apply(SwapEvent::BtcConf(4), d);
            let cancelled = must.clone().apply(SwapEvent::CancelConfirmed, d);
            [
                (setup, 0, |s| matches!(s, SwapState::Setup { .. })),
                (btc_locked, 1, |s| matches!(s, SwapState::BtcLocked { .. })),
                (night, 1, |s| matches!(s, SwapState::NightLocked { .. })),
                (ready, 1, |s| matches!(s, SwapState::ReadyToRedeem { .. })),
                (redeeming, 1, |s| matches!(s, SwapState::Redeeming { .. })),
                (done, 1, |s| matches!(s, SwapState::Done { .. })),
                (must, 4, |s| matches!(s, SwapState::MustCancel { .. })),
                (cancelled, 4, |s| matches!(s, SwapState::Cancelled { .. })),
            ]
        };

        for (state, confs, pred) in rows {
            let id = state.id();
            let stored = StoredSwap::new(state, 1, 2);
            save(&dir, &stored).unwrap();
            // hard "kill": drop everything, load from disk
            let recovered = Session::recover(&dir, id, d, confs).unwrap();
            assert!(
                pred(&recovered.stored.state),
                "resume from {:?} with confs={confs} landed in {:?}",
                recovered.stored.state,
                recovered.stored.state
            );
        }

        // And when Bitcoin has passed H₁ during the crash:
        let mut s = SwapState::new(Role::Bob);
        s = s.apply(SwapEvent::BobPublishedLock, d);
        s = s.apply(SwapEvent::BtcConf(1), d);
        let id = s.id();
        save(&dir, &StoredSwap::new(s, 1, 2)).unwrap();
        let recovered = Session::recover(&dir, id, d, 4).unwrap();
        assert!(
            matches!(recovered.stored.state, SwapState::MustCancel { .. }),
            "H₁ passed while we were down → cancel, not wait"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
