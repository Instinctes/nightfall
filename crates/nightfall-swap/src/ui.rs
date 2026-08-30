//! Every decision the swap view makes, kept out of the drawing code.
//!
//! egui code cannot be unit-tested: it needs a context, a frame, and a human
//! looking at the result. So none of the decisions live there. This module is
//! plain data in, plain data out — which network may start a swap, how much
//! time is left before a deadline, which buttons a state offers, whether a
//! typed amount is usable. `views::swap` reads these and draws them.
//!
//! The split is not tidiness. The rules in here are the ones that decide
//! whether a user loses coins, and a rule nobody can test is a rule nobody
//! can trust.

use crate::messages::Amounts;
use crate::state::{Role, SwapState};
use crate::timelock::Depths;
use nightfall_types::NetworkId;

// ------------------------------------------------------------ network gate ---

/// Whether this wallet may begin a new swap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    /// New swaps may be started.
    Enabled,
    /// Existing swaps stay visible and recoverable, but nothing new begins.
    Locked {
        headline: &'static str,
        detail: &'static str,
    },
}

impl Availability {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Mainnet is closed until someone opens this gate on purpose.
///
/// The DLEQ leaf in `nightfall_crypto::dleq` is our own code inside an
/// otherwise reviewed proof system. There is no external review of that
/// leaf. A warning label does not enforce a hold — a user who is excited
/// reads past warnings. The gate does.
///
/// Testnet and devnet are open, because that is where the thing is supposed
/// to be exercised.
pub fn availability(net: NetworkId) -> Availability {
    match net {
        NetworkId::Mainnet => Availability::Locked {
            headline: "Swaps are disabled on mainnet.",
            detail: "The cross-curve proof this feature rests on is our own \
                     leaf inside a reviewed system. Nobody outside this \
                     project has signed off on that leaf. The wallet will \
                     not lock real coins into a swap until this gate is \
                     opened on purpose. Run it on testnet.",
        },
        NetworkId::Testnet | NetworkId::Devnet => Availability::Enabled,
    }
}

// ---------------------------------------------------------------- deadlines ---

/// How close a deadline is, for colour and ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    /// Nothing to do; time is ample.
    Calm,
    /// Worth watching.
    Soon,
    /// The user should be at the keyboard.
    Act,
    /// The window is gone.
    Passed,
}

/// One countdown shown as a bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Deadline {
    pub label: &'static str,
    pub detail: String,
    /// 0.0 at the start of the window, 1.0 when it closes.
    pub fraction: f32,
    pub urgency: Urgency,
}

/// The countdowns worth showing for this state.
///
/// `btc_lock_confirms` is `None` when the Bitcoin node could not be asked.
/// That is deliberately not the same as zero: an unanswered query must never
/// render as "plenty of time", which is the same rule the driver follows and
/// the same mistake a swallowed error would make.
pub fn deadlines(
    state: &SwapState,
    depths: Depths,
    btc_lock_confirms: Option<u32>,
    night_lock_confirms: Option<u64>,
) -> Vec<Deadline> {
    let mut out = Vec::new();

    match state {
        SwapState::BtcLocked { .. }
        | SwapState::NightLocked { .. }
        | SwapState::ReadyToRedeem { .. } => {
            out.push(h1(depths, btc_lock_confirms));
            if matches!(state, SwapState::NightLocked { .. }) {
                out.push(night_progress(depths, night_lock_confirms));
            }
        }
        SwapState::Redeeming { .. } => {
            out.push(Deadline {
                label: "Redeem broadcast",
                detail: "s_a is public. Waiting for the NIGHT claim.".into(),
                fraction: 1.0,
                urgency: Urgency::Calm,
            });
        }
        SwapState::MustCancel { .. } => {
            out.push(Deadline {
                label: "Cancel",
                detail: "TX_cancel must confirm before the refund can be sent.".into(),
                fraction: 0.0,
                urgency: Urgency::Act,
            });
        }
        SwapState::Cancelled { role, .. } => {
            let mine = *role == Role::Bob;
            out.push(Deadline {
                label: "H2 · punish window",
                detail: if mine {
                    format!(
                        "Refund now. After {} blocks the other side may take the Bitcoin.",
                        depths.punish
                    )
                } else {
                    format!(
                        "If no refund appears within {} blocks, punish becomes available.",
                        depths.punish
                    )
                },
                fraction: 0.0,
                urgency: if mine { Urgency::Act } else { Urgency::Soon },
            });
        }
        _ => {}
    }

    out
}

fn h1(depths: Depths, confirms: Option<u32>) -> Deadline {
    match confirms {
        None => Deadline {
            label: "H1 · cancel window",
            detail: "Bitcoin node unreachable — remaining time unknown.".into(),
            fraction: 0.0,
            urgency: Urgency::Act,
        },
        Some(c) if c >= depths.cancel => Deadline {
            label: "H1 · cancel window",
            detail: "Passed. The swap must be cancelled, not redeemed.".into(),
            fraction: 1.0,
            urgency: Urgency::Passed,
        },
        Some(c) => {
            let left = depths.cancel - c;
            Deadline {
                label: "H1 · cancel window",
                detail: format!(
                    "{left} Bitcoin blocks left ({}h). Redeem stops {} blocks early.",
                    (u64::from(left) * 10) / 60,
                    depths.btc_redeem_margin
                ),
                fraction: c as f32 / depths.cancel.max(1) as f32,
                urgency: if !depths.may_redeem(c) {
                    Urgency::Act
                } else if left <= depths.btc_redeem_margin * 3 {
                    Urgency::Soon
                } else {
                    Urgency::Calm
                },
            }
        }
    }
}

fn night_progress(depths: Depths, confirms: Option<u64>) -> Deadline {
    match confirms {
        None => Deadline {
            label: "NIGHT confirmations",
            detail: "NIGHT node unreachable — depth unknown.".into(),
            fraction: 0.0,
            urgency: Urgency::Act,
        },
        Some(c) if c >= depths.night => Deadline {
            label: "NIGHT confirmations",
            detail: format!("{c} of {} — deep enough.", depths.night),
            fraction: 1.0,
            urgency: Urgency::Calm,
        },
        Some(c) => Deadline {
            label: "NIGHT confirmations",
            detail: format!(
                "{c} of {}. This is the reorg bound, not a guess.",
                depths.night
            ),
            fraction: c as f32 / depths.night.max(1) as f32,
            urgency: Urgency::Calm,
        },
    }
}

// ----------------------------------------------------------------- timeline ---

/// Which chain of steps the swap is walking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Both sides are cooperating.
    Progress,
    /// Something went wrong and we are unwinding.
    Abort,
}

/// The state machine as a row of steps, for the view to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    pub track: Track,
    pub steps: &'static [&'static str],
    /// Index into `steps` of where the swap is now.
    pub current: usize,
    /// Nothing further will happen without a human.
    pub settled: bool,
}

/// The happy path, in the order `apply` walks it.
pub const PROGRESS_STEPS: &[&str] = &[
    "Agreed",
    "Bitcoin locked",
    "NIGHT locked",
    "Cleared",
    "Redeeming",
    "Done",
];

/// The abort tree. `Refunded` and `Punished` are alternative endings, not
/// consecutive steps, so both land on the last one.
pub const ABORT_STEPS: &[&str] = &["Aborting", "Cancel confirmed", "Settled"];

/// Map a state onto its step. Kept here rather than in the drawing code so
/// that adding a state to the machine and forgetting the view is a test
/// failure, not a blank screen.
pub fn timeline(state: &SwapState) -> Timeline {
    let (track, steps, current, settled) = match state {
        SwapState::Setup { .. } => (Track::Progress, PROGRESS_STEPS, 0, false),
        SwapState::BtcLocked { .. } => (Track::Progress, PROGRESS_STEPS, 1, false),
        SwapState::NightLocked { .. } => (Track::Progress, PROGRESS_STEPS, 2, false),
        SwapState::ReadyToRedeem { .. } => (Track::Progress, PROGRESS_STEPS, 3, false),
        SwapState::Redeeming { .. } => (Track::Progress, PROGRESS_STEPS, 4, false),
        SwapState::Done { .. } => (Track::Progress, PROGRESS_STEPS, 5, true),
        SwapState::MustCancel { .. } => (Track::Abort, ABORT_STEPS, 0, false),
        SwapState::Cancelled { .. } => (Track::Abort, ABORT_STEPS, 1, false),
        SwapState::Refunded { .. } | SwapState::Punished { .. } => {
            (Track::Abort, ABORT_STEPS, 2, true)
        }
        SwapState::Failed { .. } => (Track::Abort, ABORT_STEPS, 0, true),
    };
    Timeline {
        track,
        steps,
        current,
        settled,
    }
}

// ------------------------------------------------------------------ actions ---

/// A button the view may offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Hand the next message to the counterparty.
    ExportPacket,
    /// Take the counterparty's next message.
    ImportPacket,
    /// Give up deliberately and start the abort tree.
    CancelNow,
    /// The machine is stuck; let a human move it.
    Recover,
    /// Remove a finished swap from the list.
    Forget,
    /// Bob, after the cancel confirmed: take his Bitcoin back.
    SendRefund,
    /// Alice, after H2 with no refund in sight: take the Bitcoin.
    SendPunish,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExportPacket => "Copy packet",
            Self::ImportPacket => "Paste packet",
            Self::CancelNow => "Cancel swap",
            Self::Recover => "Recover by hand",
            Self::Forget => "Forget",
            Self::SendRefund => "Refund my Bitcoin",
            Self::SendPunish => "Take the Bitcoin",
        }
    }

    /// What it costs. Every destructive button says this before it is pressed.
    pub fn consequence(self) -> &'static str {
        match self {
            Self::ExportPacket => "Nothing moves. The other side needs this text.",
            Self::ImportPacket => "Checked before anything is believed.",
            Self::CancelNow => {
                "Starts the abort tree. Bitcoin comes back after H1 and H2. \
                 NIGHT already locked stays locked — that is the wart."
            }
            Self::Recover => {
                "For a swap the driver cannot move. Read the state first; the \
                 wrong step here can cost the coin."
            }
            Self::Forget => "Removes the file. Only offered once the swap has ended.",
            Self::SendRefund => {
                "Sends TX_refund. Your Bitcoin comes back. Publishing it also \
                 reveals s_b, which is how the other side knows you are done. \
                 Waiting instead lets them punish after H2."
            }
            Self::SendPunish => {
                "Sends TX_punish. You take the Bitcoin because the other side \
                 never refunded. It does not unstick NIGHT you locked — that \
                 stays stuck. This is compensation, not a second payout."
            }
        }
    }

    pub fn is_destructive(self) -> bool {
        matches!(self, Self::CancelNow | Self::Recover | Self::SendPunish)
    }
}

/// Which buttons this state offers.
///
/// `Cancel` is deliberately absent from `Redeeming`, `Done` and everything
/// downstream of them, for the reason `resume` and `apply` both encode: once
/// `s_a` is public, cancelling hands the Bitcoin back to the counterparty
/// while they keep the NIGHT. The button is not there to be misread.
pub fn actions(state: &SwapState) -> Vec<Action> {
    match state {
        SwapState::Setup { .. } => vec![
            Action::ExportPacket,
            Action::ImportPacket,
            Action::CancelNow,
        ],
        SwapState::BtcLocked { .. } | SwapState::NightLocked { .. } => {
            vec![
                Action::ExportPacket,
                Action::ImportPacket,
                Action::CancelNow,
            ]
        }
        SwapState::ReadyToRedeem { .. } => vec![Action::ExportPacket, Action::CancelNow],
        SwapState::Redeeming { .. } => vec![Action::ExportPacket, Action::Recover],
        SwapState::MustCancel { .. } => vec![Action::CancelNow, Action::Recover],
        // After the cancel confirms the two sides part ways: Bob takes his
        // coin back, Alice waits and then takes it if he does not. Offering
        // each side the other's button would be offering a transaction they
        // cannot complete.
        SwapState::Cancelled { role, .. } if *role == Role::Bob => {
            vec![Action::SendRefund, Action::Recover]
        }
        SwapState::Cancelled { .. } => vec![Action::SendPunish, Action::Recover],
        SwapState::Done { .. } | SwapState::Refunded { .. } | SwapState::Punished { .. } => {
            vec![Action::Forget]
        }
        SwapState::Failed { .. } => vec![Action::Recover, Action::Forget],
    }
}

/// Whether the swap has stopped moving for good.
pub fn is_finished(state: &SwapState) -> bool {
    matches!(
        state,
        SwapState::Done { .. } | SwapState::Refunded { .. } | SwapState::Punished { .. }
    )
}

// ----------------------------------------------------------------- funding ---

/// Bob's funding input, as typed. This wallet holds NIGHT, so the Bitcoin
/// that goes into the 2-of-2 comes from an output the user already controls
/// somewhere else.
#[derive(Clone, Debug, Default)]
pub struct FundingDraft {
    pub txid: String,
    pub vout: String,
    /// Value of that output, in satoshis.
    pub value: String,
    /// Optional: where the change goes. Empty means no change output.
    pub change_address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FundingError {
    TxidMissing,
    TxidNotHex,
    VoutMissing,
    VoutNotANumber,
    ValueMissing,
    ValueNotANumber,
    /// The chosen output cannot cover the swap amount plus the fee.
    TooSmall {
        have: u64,
        need: u64,
    },
}

impl std::fmt::Display for FundingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TxidMissing => write!(
                f,
                "Enter the transaction id of the output you are funding with."
            ),
            Self::TxidNotHex => write!(f, "A Bitcoin transaction id is 64 hex characters."),
            Self::VoutMissing => write!(f, "Enter which output of that transaction (0, 1, …)."),
            Self::VoutNotANumber => write!(f, "The output index must be a whole number."),
            Self::ValueMissing => write!(f, "Enter how many satoshis that output holds."),
            Self::ValueNotANumber => write!(f, "The output value must be a number of satoshis."),
            Self::TooSmall { have, need } => write!(
                f,
                "That output holds {have} sat but the lock plus its fee needs {need} sat."
            ),
        }
    }
}

/// What a checked funding draft yields. Deliberately not a `bitcoin::OutPoint`
/// so this module stays free of the Bitcoin types the drawing layer does not
/// need to know about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Funding {
    pub txid: String,
    pub vout: u32,
    pub value_sats: u64,
}

/// Check the funding input against the amounts the swap already agreed.
///
/// The size check is the one that matters: `TxLock::from_prevout` refuses a
/// prevout that cannot cover value plus fee, but it refuses it *after* the
/// user has already been told the swap is under way. Catching it in the form
/// turns a confusing failure into a sentence.
pub fn validate_funding(d: &FundingDraft, amounts: &Amounts) -> Result<Funding, FundingError> {
    let txid = d.txid.trim();
    if txid.is_empty() {
        return Err(FundingError::TxidMissing);
    }
    if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(FundingError::TxidNotHex);
    }

    let vout = d.vout.trim();
    if vout.is_empty() {
        return Err(FundingError::VoutMissing);
    }
    let vout: u32 = vout.parse().map_err(|_| FundingError::VoutNotANumber)?;

    let value = d.value.trim();
    if value.is_empty() {
        return Err(FundingError::ValueMissing);
    }
    let value_sats: u64 = value.parse().map_err(|_| FundingError::ValueNotANumber)?;

    let need = amounts.btc_sats.saturating_add(amounts.btc_fee_sats);
    if value_sats < need {
        return Err(FundingError::TooSmall {
            have: value_sats,
            need,
        });
    }

    Ok(Funding {
        txid: txid.to_ascii_lowercase(),
        vout,
        value_sats,
    })
}

/// Change that would be left over. `None` when it is below the dust limit,
/// in which case there is no change output and the remainder goes to fees.
pub fn change_after_lock(funding: &Funding, amounts: &Amounts) -> Option<u64> {
    const DUST: u64 = 546;
    let spent = amounts.btc_sats.saturating_add(amounts.btc_fee_sats);
    let change = funding.value_sats.saturating_sub(spent);
    if change > DUST {
        Some(change)
    } else {
        None
    }
}

// ------------------------------------------------------------------- draft ---

/// The start form, as typed.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub give_night: bool,
    pub night: String,
    pub btc: String,
    pub btc_fee: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftError {
    NightMissing,
    NightUnreadable,
    NightZero,
    NightAboveBalance { want: u64, have: u64 },
    BtcMissing,
    BtcUnreadable,
    BtcZero,
    FeeUnreadable,
    FeeAboveAmount { fee: u64, amount: u64 },
}

impl std::fmt::Display for DraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NightMissing => write!(f, "Enter the NIGHT amount."),
            Self::NightUnreadable => write!(f, "NIGHT amount is not a number."),
            Self::NightZero => write!(f, "NIGHT amount must be more than zero."),
            Self::NightAboveBalance { want, have } => write!(
                f,
                "You typed {} NIGHT but only {} is available and unreserved.",
                darks_to_night(*want),
                darks_to_night(*have)
            ),
            Self::BtcMissing => write!(f, "Enter the Bitcoin amount."),
            Self::BtcUnreadable => write!(f, "Bitcoin amount is not a number of satoshis."),
            Self::BtcZero => write!(f, "Bitcoin amount must be more than zero."),
            Self::FeeUnreadable => write!(f, "Bitcoin fee is not a number of satoshis."),
            Self::FeeAboveAmount { fee, amount } => write!(
                f,
                "The fee ({fee} sat) is not smaller than the amount ({amount} sat)."
            ),
        }
    }
}

/// Parse `12.5` into darks. Rejects anything a human did not mean.
fn parse_night(s: &str) -> Result<u64, DraftError> {
    const PER: u64 = 100_000_000;
    let t = s.trim().replace([' ', '\u{202F}', '_'], "");
    if t.is_empty() {
        return Err(DraftError::NightMissing);
    }
    let (whole, frac) = match t.split_once('.') {
        Some((w, f)) => (w, f),
        None => (t.as_str(), ""),
    };
    if frac.len() > 8 {
        return Err(DraftError::NightUnreadable);
    }
    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| DraftError::NightUnreadable)?
    };
    let frac_val: u64 = if frac.is_empty() {
        0
    } else {
        let padded = format!("{frac:0<8}");
        padded.parse().map_err(|_| DraftError::NightUnreadable)?
    };
    whole
        .checked_mul(PER)
        .and_then(|w| w.checked_add(frac_val))
        .ok_or(DraftError::NightUnreadable)
}

fn darks_to_night(d: u64) -> String {
    format!("{}.{:08}", d / 100_000_000, d % 100_000_000)
}

/// Turn the typed form into amounts, or say exactly what is wrong.
///
/// `available_darks` must already exclude coins reserved by another swap.
/// The wallet does that in `select_coins_at`; passing the raw balance here
/// would let a user promise the same coin to two counterparties.
pub fn validate(d: &Draft, available_darks: u64) -> Result<Amounts, DraftError> {
    let night_darks = parse_night(&d.night)?;
    if night_darks == 0 {
        return Err(DraftError::NightZero);
    }
    if d.give_night && night_darks > available_darks {
        return Err(DraftError::NightAboveBalance {
            want: night_darks,
            have: available_darks,
        });
    }

    let btc = d.btc.trim();
    if btc.is_empty() {
        return Err(DraftError::BtcMissing);
    }
    let btc_sats: u64 = btc.parse().map_err(|_| DraftError::BtcUnreadable)?;
    if btc_sats == 0 {
        return Err(DraftError::BtcZero);
    }

    let fee = d.btc_fee.trim();
    let btc_fee_sats: u64 = if fee.is_empty() {
        crate::fees::FeeLadder::mainnet().pick(1_000)
    } else {
        fee.parse().map_err(|_| DraftError::FeeUnreadable)?
    };
    if btc_fee_sats >= btc_sats {
        return Err(DraftError::FeeAboveAmount {
            fee: btc_fee_sats,
            amount: btc_sats,
        });
    }

    Ok(Amounts {
        night_darks,
        btc_sats,
        btc_fee_sats,
    })
}

/// The role a draft implies. Alice gives NIGHT and takes Bitcoin.
pub fn role_of(d: &Draft) -> Role {
    if d.give_night {
        Role::Alice
    } else {
        Role::Bob
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
