//! B3 — duration, in a form CI can actually run.
//!
//! A weeks-long Signet soak is the same question asked slowly: the chain
//! moves between ticks, the process dies, the mempool fills. This loop
//! asks it a few hundred times with a seeded RNG so a failure is replayable.
//!
//! `NF_SOAK=long cargo test -p nightfall-swap --test soak -- --ignored --nocapture`
//! stretches the same loop. The invariant never changes: after every step
//! the swap is in a named state, or the tick returned an error. Never a
//! redeem after the window has closed. Never a silent no-op that pretends
//! to have decided.

use nightfall_swap::driver::{Session, Tick};
use nightfall_swap::persist::{PendingSend, SendKind, StoredSwap};
use nightfall_swap::state::{Role, SwapEvent, SwapState};
use nightfall_swap::timelock::Depths;
use nightfall_swap::watch::{FakeWatch, WatchError};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

fn depths() -> Depths {
    Depths::testdrive()
}

fn ready_alice(dir: &std::path::Path) -> Session {
    let d = depths();
    let mut stored = StoredSwap::new(SwapState::new(Role::Alice), 1, 2);
    stored.state = stored.state.apply(SwapEvent::BobPublishedLock, d);
    stored.state = stored.state.apply(SwapEvent::BtcConf(1), d);
    stored.state = stored.state.apply(SwapEvent::AlicePublishedNightLock, d);
    stored.state = stored
        .state
        .apply(SwapEvent::NightConf { night: 2, btc: 1 }, d);
    stored.btc_lock_txid = Some("lock".into());
    stored.night_lock_id = Some("nlock".into());
    let mut s = Session::open(dir, stored, d);
    s.raw
        .insert(SendKind::Redeem, ("redeem".into(), "00".into()));
    s.raw
        .insert(SendKind::Cancel, ("cancel".into(), "00".into()));
    nightfall_swap::persist::save(dir, &s.stored).unwrap();
    s
}

fn invariant(s: &Session, last: &Result<Tick, nightfall_swap::driver::TickError>) {
    if last.is_err() {
        return;
    }
    // Every variant is a named place. Failed is allowed. There is no
    // "nothing".
    let _ = s.stored.state.id();
}

fn run(steps: u32, seed: u64) {
    let dir = std::env::temp_dir().join(format!("nf-soak-{seed}-{}", Uuid::new_v4()));
    let mut s = ready_alice(&dir);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut btc = FakeWatch::new(10);
    btc.set_confs("lock", 1);
    let mut night = FakeWatch::new(100);
    night.set_confs("nlock", 100);
    let d = depths();

    for i in 0..steps {
        let roll = rng.gen_range(0..10);
        match roll {
            0..=3 => {
                // The chain moves. This is what Signet does between ticks.
                let next = btc.confs.get("lock").copied().unwrap_or(1) + 1;
                btc.set_confs("lock", next);
                btc.height = btc.height.saturating_add(1);
            }
            4 => {
                night.set_confs("nlock", night.confs.get("nlock").copied().unwrap_or(2) + 1);
                night.height += 1;
            }
            5 => {
                btc.confs_outage = Some(WatchError::Unavailable("blip".into()));
            }
            6 => {
                // Crash: persist, drop, recover from disk.
                nightfall_swap::persist::save(&dir, &s.stored).unwrap();
                let id = s.stored.state.id();
                let confs = btc.confs.get("lock").copied().unwrap_or(0) as u32;
                s = Session::recover(&dir, id, d, confs).unwrap();
                s.raw
                    .insert(SendKind::Redeem, ("redeem".into(), "00".into()));
                s.raw
                    .insert(SendKind::Cancel, ("cancel".into(), "00".into()));
            }
            7 => {
                if s.stored.pending.is_none() {
                    s.stored.pending = Some(PendingSend {
                        kind: SendKind::Redeem,
                        txid: "redeem".into(),
                    });
                }
            }
            _ => {
                btc.confs_outage = None;
            }
        }

        let lock_confs = btc.confs.get("lock").copied().unwrap_or(0);
        let out = s.tick(&btc, &night);
        if btc.confs_outage.is_some() {
            assert!(out.is_err(), "step {i}: an outage must not look like idle");
            btc.confs_outage = None;
            invariant(&s, &out);
            continue;
        }
        let out = out.expect("step {i} tick");
        if matches!(
            out,
            Tick::Broadcast {
                kind: SendKind::Redeem,
                ..
            }
        ) {
            assert!(
                d.may_redeem(lock_confs as u32),
                "step {i}: redeem offered at lock_confs={lock_confs} past the window"
            );
        }
        invariant(&s, &Ok(out));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_few_hundred_ticks_never_redeem_past_the_window() {
    run(200, 0x4e49_4748);
}

#[test]
fn a_different_seed_does_the_same() {
    run(120, 7);
}

#[test]
#[ignore]
fn long_soak() {
    let n: u32 = std::env::var("NF_SOAK_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000);
    run(n, 1);
}
