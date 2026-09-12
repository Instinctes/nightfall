//! The eight Core views, sharing the web wallet's visual hierarchy.

use crate::app::{parse_amount, App, View, DEFAULT_FEE_DARKS};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use nightfall_crypto::Address;
use nightfall_node::SyncHold;
use nightfall_storage::now_unix;
use nightfall_types::{
    Amount, NetworkId, DARKS_PER_NIGHT, MAX_SUPPLY_NIGHT, TARGET_BLOCK_TIME_SECS,
};
// Not to be confused with `vault_ui::PaymentRequest`, which is a job handed to
// the payment worker — a payment this wallet is about to make. This one is what
// somebody else asked for, before anyone has agreed to it.
use nightfall_wallet::payment_request::{self, PaymentRequest};
use nightfall_wallet::Direction;

/// One line saying what a page is for. Rendered as the topbar's subtitle.
pub fn page_description(view: View) -> &'static str {
    match view {
        View::Dashboard => "Your balance, recent activity and network health at a glance.",
        View::Send => "Choose a recipient, enter an amount and review the total before sending.",
        View::Receive => "Your address, and the invoices you are waiting to be paid.",
        View::Activity => "Payments, mining rewards, confirmation status and receipts.",
        View::Mining => "Manage CPU usage and follow your mining performance.",
        View::Network => "Check connectivity, synchronization and network privacy.",
        View::Swap => {
            "Exchange NIGHT and Bitcoin. Follow each step and keep Core running until completion."
        }
        View::Settings => "Manage backups, privacy and wallet maintenance.",
    }
}

/// Kept for the layout test, which measures the description at each width.
pub fn page_intro(view: View, ui: &mut egui::Ui) {
    let _ = (page_description(view), ui);
}

fn night(darks: u64) -> String {
    let whole = darks / DARKS_PER_NIGHT;
    let frac = darks % DARKS_PER_NIGHT;
    format!("{}.{:08}", format_int(whole), frac)
}

// `manual_is_multiple_of` does not exist on every toolchain we build with;
// `unknown_lints` keeps the older one from failing on the allow itself.
#[allow(unknown_lints)]
#[allow(clippy::manual_is_multiple_of)]
fn night_compact(darks: u64) -> String {
    if darks % DARKS_PER_NIGHT == 0 {
        return format_int(darks / DARKS_PER_NIGHT);
    }
    let whole = darks / DARKS_PER_NIGHT;
    let frac = format!("{:08}", darks % DARKS_PER_NIGHT);
    format!("{}.{}", format_int(whole), frac.trim_end_matches('0'))
}

// ------------------------------------------------------------- dashboard ---

pub fn dashboard(app: &mut App, ui: &mut egui::Ui) {
    let tip = app.tip_height();
    let maturity = app.maturity();
    let balances = app
        .wallet
        .lock()
        .map(|w| w.balances(tip, maturity))
        .unwrap_or_default();

    let (blocks, peers, mempool, difficulty, supply_ok, minted, burned) = app
        .status
        .as_ref()
        .map(|s| {
            (
                s.blocks,
                s.live_peers,
                s.mempool,
                s.difficulty,
                s.supply_ok,
                s.minted,
                s.burned_fees,
            )
        })
        .unwrap_or((0, 0, 0, 0, false, 0, 0));

    let loading = app.status.as_ref().map(|s| s.loading).unwrap_or(false);
    let sync_hold = app
        .status
        .as_ref()
        .map(|s| s.sync_hold)
        .unwrap_or(SyncHold::Synced);

    if loading {
        egui::Frame::none()
            .fill(ACCENT.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.55)))
            .rounding(Rounding::same(ROUND))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 32.0);
                ui.horizontal(|ui| {
                    dot(ui, ACCENT, true);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Loading the chain from disk")
                            .size(14.0)
                            .color(ACCENT)
                            .strong(),
                    );
                });
                ui.add_space(10.0);

                // A count with no denominator is not progress, it is a number
                // that keeps changing. This load takes many minutes on a full
                // chain, and for that whole time the old panel said only how
                // far it had got — no total, no fraction, no idea whether that
                // meant nearly done or barely started. People reasonably
                // concluded the wallet had hung.
                let total = app.status.as_ref().map(|s| s.loading_total).unwrap_or(0);

                if total > 0 {
                    let frac = (blocks as f32 / total as f32).clamp(0.0, 1.0);
                    progress(
                        ui,
                        frac,
                        &format!(
                            "{} of {} blocks · {:.0}%",
                            format_int(blocks),
                            format_int(total),
                            frac * 100.0
                        ),
                    );
                    ui.add_space(8.0);
                    if let Some(left) = app.load_eta(blocks, total) {
                        ui.label(
                            RichText::new(format!("about {left} left"))
                                .size(12.0)
                                .color(TEXT_DIM),
                        );
                        ui.add_space(6.0);
                    }
                } else {
                    ui.label(
                        RichText::new(format!("{} blocks read", format_int(blocks)))
                            .size(12.5)
                            .color(TEXT_DIM),
                    );
                    ui.add_space(6.0);
                }

                ui.label(
                    RichText::new(
                        "Every proof of work in the file is being re-derived. This is not \
                         a network sync and nothing is being downloaded — peers stay \
                         closed so this node does not advertise genesis. Your coins are \
                         already on disk.",
                    )
                    .size(12.0)
                    .color(TEXT_FAINT),
                );
            });
        ui.add_space(14.0);
    }

    // Mining is held back while the chain is behind or on a fork. The old
    // label always said "1 block behind" on a fork, which is why a node
    // stranded for days looked one block late.
    let hold_banner = match sync_hold {
        SyncHold::CatchingUp(n) if n > 0 => Some((
            ACCENT,
            format!(
                "Catching up — {} block{} behind",
                format_int(n),
                if n == 1 { "" } else { "s" }
            ),
            "Mining starts by itself once this reaches zero. A block built on an outdated tip cannot be accepted by anyone — it would only split the chain.".to_string(),
            false,
        )),
        SyncHold::CompetingTip { reorging } => Some((
            WARN,
            if reorging {
                "On a competing tip — reorg in progress".to_string()
            } else {
                "On a competing tip — waiting to reorg".to_string()
            },
            "BLOCKS is frozen because the next network block does not connect here. The \"1 behind\" figure was a hold, not a distance.".to_string(),
            false,
        )),
        SyncHold::DeadBranch { gap } => Some((
            DANGER,
            "Stuck on a dead branch".to_string(),
            format!(
                "This tip diverged {} blocks back — past the 500-block reorg limit. Resync the chain file in Settings. The seed and wallet stay. Coinbase mined on this branch is gone.",
                format_int(gap)
            ),
            true,
        )),
        _ => None,
    };
    if let Some((color, title, body, offer_resync)) = hold_banner {
        egui::Frame::none()
            .fill(color.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.55)))
            .rounding(Rounding::same(ROUND))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 32.0);
                ui.horizontal(|ui| {
                    dot(ui, color, true);
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).size(14.0).color(color).strong());
                });
                ui.add_space(6.0);
                ui.label(RichText::new(body).size(12.5).color(TEXT_DIM));
                if offer_resync {
                    ui.add_space(10.0);
                    if ghost_button(ui, "  Open Settings to resync  ").clicked() {
                        app.view = View::Settings;
                        app.resync_confirm = true;
                    }
                }
            });
        ui.add_space(14.0);
    }

    if !app.backup_acked {
        egui::Frame::none()
            .fill(WARN.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, WARN.gamma_multiply(0.55)))
            .rounding(Rounding::same(ROUND))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 32.0);
                ui.horizontal(|ui| {
                    dot(ui, WARN, false);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Write down the 24 words")
                            .size(14.0)
                            .color(WARN)
                            .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "The hex seed is no longer the backup. Settings → Backup shows the same 24 words the phone and browser wallets use. Anyone who sees them can spend.",
                    )
                    .size(12.5)
                    .color(TEXT_DIM),
                );
                ui.add_space(10.0);
                if ghost_button(ui, "  Open Backup  ").clicked() {
                    app.view = View::Settings;
                    app.reveal_mnemonic = true;
                }
            });
        ui.add_space(14.0);
    }

    // Mining with no peers is how you end up on a private fork without
    // noticing. Say so loudly, before hours of work get discarded.
    if app.is_mining() && peers == 0 && !loading {
        egui::Frame::none()
            .fill(WARN.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, WARN.gamma_multiply(0.55)))
            .rounding(Rounding::same(ROUND))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 32.0);
                ui.horizontal(|ui| {
                    dot(ui, WARN, true);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Mining alone — not connected to anyone")
                            .size(14.0)
                            .color(WARN)
                            .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "You are building your own chain. If another miner is running on the \
                         same genesis, one of the two chains will be discarded when you finally \
                         connect — and everything mined on the lighter one is lost.",
                    )
                    .size(11.5)
                    .color(TEXT_DIM),
                );
                ui.add_space(10.0);
                if ghost_button(ui, "  Connect to a peer  ").clicked() {
                    app.view = View::Network;
                }
            });
        ui.add_space(14.0);
    }

    // --- balance hero: the one gradient surface in the app ---
    gradient_card(ui, 172.0, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("TOTAL BALANCE")
                        .size(11.0)
                        .color(INK)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(night(balances.available))
                            .size(40.0)
                            .color(INK)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(RichText::new("NIGHT").size(16.0).color(INK).strong());
                });

                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    if balances.immature > 0 {
                        on_gradient_chip(ui, &format!("{} unlocking", night(balances.immature)));
                    }
                    if balances.pending_out > 0 {
                        on_gradient_chip(ui, &format!("{} pending", night(balances.pending_out)));
                    }
                    let count = app.wallet.lock().map(|w| w.output_count()).unwrap_or(0);
                    on_gradient_chip(ui, &format!("{count} outputs"));
                });
            });
        });
    });

    // Actions live below the gradient, not on it: a white pill on the light end
    // of the gradient is nearly invisible.
    ui.add_space(GAP_MD);
    ui.horizontal(|ui| {
        // One width for both. "Send" at 75 points beside "Receive" at 92 was
        // two sizes for one pair of equally important choices — and the
        // padding hack that produced it (`"  Receive  "`) is exactly the kind
        // of thing that stops working the moment the label changes.
        if let Some(index) = button_row(
            ui,
            &["Send", "Receive"],
            true,
        ) {
            app.view = if index == 0 { View::Send } else { View::Receive };
        }

        // The wallet syncs itself every few seconds; this is a status readout,
        // not a button. Nothing here should ever need clicking to stay current.
        //
        // "In sync" with no peers is a lie, and an expensive one: it is exactly
        // what lets someone keep mining a chain nobody will ever accept. With
        // nobody to be in sync *with*, the only honest thing to report is that
        // there is nobody.
        ui.add_space(6.0);
        let syncing = app.syncing.load(std::sync::atomic::Ordering::SeqCst);
        let connected = peers > 0;
        let scanned = app.wallet.lock().map(|w| w.scanned_to()).unwrap_or(0);

        let loading = app.status.as_ref().map(|s| s.loading).unwrap_or(false);
        let (colour, pulse, text) = if loading {
            (
                WARN,
                true,
                format!("Loading chain · local block {}", format_int(blocks)),
            )
        } else if syncing {
            (WARN, true, "Scanning…".to_string())
        } else if connected {
            (
                SUCCESS,
                false,
                format!("In sync · block {}", format_int(scanned)),
            )
        } else if crate::app::IS_DEV_BUILD {
            (
                TEXT_DIM,
                false,
                format!("Local Devnet · block {}", format_int(scanned)),
            )
        } else {
            (
                WARN,
                false,
                format!("No peers · local block {}", format_int(scanned)),
            )
        };

        dot(ui, colour, pulse);
        ui.add_space(2.0);
        ui.label(RichText::new(text).size(11.5).color(TEXT_FAINT));
    });

    if let Some(reason) = peers_zero_reason(app, peers, blocks) {
        ui.add_space(8.0);
        ui.label(RichText::new(reason).size(11.5).color(TEXT_DIM));
    }

    let scanned = app.wallet.lock().map(|w| w.scanned_to()).unwrap_or(0);
    if !loading && tip > scanned {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "Wallet scan {} / {} — coins in the last {} block{} are still being looked for.",
                format_int(scanned),
                format_int(tip),
                format_int(tip.saturating_sub(scanned)),
                if tip.saturating_sub(scanned) == 1 {
                    ""
                } else {
                    "s"
                }
            ))
            .size(11.5)
            .color(ACCENT_HI),
        );
    }

    if app.is_mining() && app.hashrate.current > 0.0 && difficulty > 0 {
        let secs = difficulty as f64 / app.hashrate.current.max(1.0);
        let eta = if secs < 90_000.0 {
            format!(
                "Your hashrate × this difficulty ≈ one block every {}",
                human_duration(secs)
            )
        } else {
            "A block at this hashrate is more than a day away.".to_string()
        };
        ui.add_space(6.0);
        ui.label(RichText::new(eta).size(11.5).color(ACCENT_HI));
    }

    // Mining is on and nothing is happening. Before 0.8.4 this was silent:
    // the switch said mining, the rate said 0, and the reason was in a log
    // line at the moment it scrolled past. One person on Discord ran for
    // hours like this without knowing.
    let idle = app.status.as_ref().map(|s| s.mining_idle).unwrap_or("");
    if app.is_mining() && !idle.is_empty() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Not mining right now — {idle}"))
                .size(11.5)
                .color(WARN),
        );
    }

    if balances.immature > 0 {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "Mining rewards unlock after {maturity} blocks. They are yours already — just not spendable yet."
            ))
            .size(11.5)
            .color(TEXT_DIM),
        );
    }

    ui.add_space(14.0);

    // `ui.columns` gives each cell its own top-down layout. Using
    // `allocate_ui` inside a horizontal layout makes the children horizontal
    // too, which is what staggered the cards diagonally.
    metric_grid(
        ui,
        &[
            ("BLOCKS", format_int(blocks), TEXT),
            (
                "PEERS",
                peers.to_string(),
                if peers > 0 { SUCCESS } else { WARN },
            ),
            ("MEMPOOL", mempool.to_string(), TEXT),
            ("DIFFICULTY", format_int(difficulty), TEXT),
        ],
        true,
    );

    ui.add_space(14.0);

    let now = now_unix();
    let tip_time = app.status.as_ref().map(|s| s.tip_time).unwrap_or(0);
    let last_age = now.saturating_sub(tip_time);
    let last_block = if loading || tip_time == 0 {
        "—".to_string()
    } else {
        ago(tip_time, now)
    };
    let last_color = if loading || tip_time == 0 {
        TEXT_DIM
    } else if last_age > 60 {
        WARN
    } else {
        TEXT
    };

    let net_hs = if loading || difficulty == 0 {
        0.0
    } else {
        difficulty as f64 / TARGET_BLOCK_TIME_SECS as f64
    };
    let net_label = if net_hs <= 0.0 {
        "—".to_string()
    } else {
        format_hashrate(net_hs)
    };

    let local_hs = app.hashrate.current;
    let (share_label, share_color) = if app.is_mining() && local_hs > 0.0 && net_hs > 0.0 {
        (format!("{:.2}%", 100.0 * local_hs / net_hs), ACCENT_HI)
    } else {
        ("—".to_string(), TEXT_DIM)
    };

    let next = next_unlock(app);
    let (unlock_label, unlock_color) = match next {
        Some((value, left)) => {
            let secs = left as f64 * TARGET_BLOCK_TIME_SECS as f64;
            (
                format!("{} · {}", night_compact(value), human_duration(secs)),
                WARN,
            )
        }
        None => ("—".to_string(), TEXT_DIM),
    };

    metric_grid(
        ui,
        &[
            ("LAST BLOCK", last_block, last_color),
            ("NETWORK HASH", net_label, TEXT),
            ("YOUR SHARE", share_label, share_color),
            ("NEXT UNLOCK", unlock_label, unlock_color),
        ],
        true,
    );

    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Network hash is difficulty ÷ 15 s — an estimate, not a miner count. \
             Your share is this machine against that estimate.",
        )
        .size(11.0)
        .color(TEXT_FAINT),
    );

    ui.add_space(14.0);

    ui.columns(2, |cols| {
        // --- recent activity ---
        titled_card(&mut cols[0], "Recent activity", |ui| {
            let entries: Vec<_> = app
                .wallet
                .lock()
                .map(|w| w.history().iter().take(6).cloned().collect())
                .unwrap_or_default();

            if entries.is_empty() {
                empty_state(
                    ui,
                    "Nothing yet",
                    "Turn on mining or share your address to receive NIGHT.",
                );
            } else {
                let now = now_unix();
                for e in &entries {
                    activity_row(ui, e, now);
                }
                ui.add_space(4.0);
                if ghost_button(ui, "View all activity").clicked() {
                    app.view = View::Activity;
                }
            }
        });

        // --- supply panel ---
        titled_card(&mut cols[1], "Network supply", |ui| {
            if loading {
                ui.label(RichText::new("—").size(20.0).monospace().strong());
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Hidden until the chain file is in memory. Not zero.")
                        .size(11.5)
                        .color(TEXT_DIM),
                );
                return;
            }
            let circulating = minted.saturating_sub(burned);
            let pct = circulating as f64 / (MAX_SUPPLY_NIGHT as f64 * DARKS_PER_NIGHT as f64);

            ui.label(
                RichText::new(format!("{} NIGHT", night(circulating)))
                    .size(20.0)
                    .monospace()
                    .strong(),
            );
            ui.add_space(3.0);
            ui.label(
                RichText::new(format!(
                    "of {} max · {:.4}% issued",
                    format_int(MAX_SUPPLY_NIGHT),
                    pct * 100.0
                ))
                .size(11.5)
                .color(TEXT_DIM),
            );
            ui.add_space(10.0);
            progress(ui, pct as f32, "");

            ui.add_space(14.0);
            kv(
                ui,
                "Mined",
                RichText::new(night(minted)).monospace().color(TEXT),
            );
            kv(
                ui,
                "Burned in fees",
                RichText::new(night(burned)).monospace().color(WARN),
            );

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                dot(ui, status_color(supply_ok), false);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if loading {
                        "Supply proof waits until the file is loaded"
                    } else if supply_ok {
                        "Supply proof verified"
                    } else {
                        "Supply proof FAILED"
                    })
                    .size(12.0)
                    .color(status_color(supply_ok)),
                );
            });
            ui.label(
                RichText::new("Every node re-checks that no coin exists which was never mined.")
                    .size(10.5)
                    .color(TEXT_FAINT),
            );
        });
    });
}

fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(14.0).color(TEXT_DIM));
        ui.add_space(4.0);
        ui.label(RichText::new(hint).size(11.5).color(TEXT_FAINT));
    });
    ui.add_space(18.0);
}

/// One row of the Activity list.
///
/// The columns are allocated at fixed widths on purpose. The first version
/// asked `ui.available_width()` inside the row and subtracted a constant. That
/// number changes the moment the scroll bar appears, so every row measured a
/// slightly different width and the amounts walked sideways as you scrolled —
/// which is exactly what it looked like: a broken table. A right-aligned column
/// has to be pinned to a width the row cannot renegotiate.
fn activity_row(ui: &mut egui::Ui, e: &nightfall_wallet::HistoryEntry, now: u64) {
    // Only glyphs that exist in egui's bundled font — a missing one renders
    // as a tofu box, which is what made the first build look broken.
    let (icon, color, sign) = match e.direction {
        Direction::Received => ("+", SUCCESS, "+"),
        Direction::Mined => ("*", ACCENT_HI, "+"),
        Direction::Sent => ("-", TEXT, "-"),
    };

    const ICON_W: f32 = 18.0;
    const AMOUNT_W: f32 = 150.0;
    const H_MARGIN: f32 = 12.0;

    let full = ui.available_width();
    let inner = (full - H_MARGIN * 2.0).max(200.0);
    let mid = (inner - ICON_W - AMOUNT_W - 14.0).max(90.0);

    egui::Frame::none()
        .fill(SURFACE_HI)
        .rounding(Rounding::same(ROUND_FIELD))
        .inner_margin(egui::Margin::symmetric(H_MARGIN, 9.0))
        .show(ui, |ui| {
            ui.set_width(inner);
            // Two lines of text, not two paragraphs. The default vertical
            // spacing put ten points between the direction and its memo and
            // another ten under it, so a list of five payments was mostly
            // padding and only three rows fitted on screen at once.
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ICON_W, 0.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(icon).size(15.0).color(color));
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(mid, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_width(mid);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(e.direction.label()).size(13.0).strong());
                            if e.is_pending() {
                                badge(ui, "pending", WARN);
                            }
                        });
                        let sub = if e.memo.is_empty() {
                            match e.height {
                                Some(h) => format!("block {h} · {}", ago(e.timestamp, now)),
                                None => format!("not in a block yet · {}", ago(e.timestamp, now)),
                            }
                        } else {
                            e.memo.clone()
                        };
                        // Truncate rather than wrap: a long memo used to push
                        // the row taller and shove everything after it around.
                        ui.add(
                            egui::Label::new(RichText::new(sub).size(11.0).color(TEXT_FAINT))
                                .truncate(),
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    Vec2::new(AMOUNT_W, 0.0),
                    egui::Layout::top_down(egui::Align::RIGHT),
                    |ui| {
                        ui.set_width(AMOUNT_W);
                        ui.label(
                            RichText::new(format!("{sign}{}", night(e.amount)))
                                .size(13.5)
                                .color(color)
                                .monospace(),
                        );
                        if e.fee > 0 {
                            ui.label(
                                RichText::new(format!("fee {}", night(e.fee)))
                                    .size(10.0)
                                    .color(TEXT_FAINT),
                            );
                        }
                    },
                );
            });
        });
    ui.add_space(2.0);
}

// ------------------------------------------------------------------ send ---

/// Show a pasted payment request, and return it only if the owner applies it.
///
/// Returns `None` when the field holds anything other than a request — an
/// ordinary address goes straight to the validator above and never reaches
/// here. A request that cannot be paid is still *shown*, with the reason: a
/// wallet that merely refuses to understand a testnet request leaves its owner
/// guessing, and the guess they are most likely to make is that the address is
/// wrong.
/// A Proof Card: what a receipt establishes, and what it leaves open.
///
/// The three questions are kept visually apart because they are three
/// different strengths of claim, and the commonest way to be misled by a
/// receipt is to read a signed sentence as a settled payment. What is proven
/// comes first, what is not comes immediately after, and neither is a footnote.
///
/// The wording is [`nightfall_wallet::ReceiptProof`]'s, not this screen's, so
/// the desktop wallet and the command line cannot describe one document in two
/// ways.
pub fn proof_card(ui: &mut egui::Ui, proof: &nightfall_wallet::ReceiptProof) {
    use nightfall_wallet::ReceiptKind;

    let (label, tone) = match proof.kind {
        ReceiptKind::Received => ("Received", SUCCESS),
        ReceiptKind::Mined => ("Mined", SUCCESS),
        // A sent receipt is the signer's own account of a payment. It is not
        // worthless, and it is not proof of an amount; the colour says so
        // before any of the text is read.
        ReceiptKind::Sent => ("Sent — the signer's own account", WARN),
        ReceiptKind::Other => ("Unrecognised kind", DANGER),
    };
    ui.horizontal(|ui| {
        badge(ui, label, tone);
        if proof.version < nightfall_wallet::RECEIPT_VERSION {
            badge(ui, &format!("older format (v{})", proof.version), WARN);
        }
    });
    ui.add_space(10.0);

    // Monospace for the address, for the same reason as everywhere else: the
    // only defence against a swapped address is a person comparing characters.
    kv(
        ui,
        "Signed by",
        RichText::new(&proof.address).monospace().size(11.5),
    );
    kv(
        ui,
        "Amount",
        RichText::new(format!("{}", Amount(proof.amount_darks)))
            .color(if proof.amount_proven { TEXT } else { WARN }),
    );
    if !proof.memo.is_empty() {
        kv(ui, "Memo", RichText::new(&proof.memo));
    }
    kv(ui, "Height", RichText::new(proof.height.to_string()));
    kv(
        ui,
        if proof.amount_proven {
            "Commitment"
        } else {
            "Transaction"
        },
        RichText::new(&proof.reference).monospace().size(11.0),
    );

    // The two halves as two blocks, not as one column of bullets. What a
    // receipt proves and what it leaves open are different kinds of statement,
    // and a reader who has to work out which paragraph is which will read the
    // reassuring one and stop.
    ui.add_space(GAP_MD);
    let tone = if proof.amount_proven { SUCCESS } else { WARN };
    // A framed block's own margins come out of the parent's width, so asking
    // for the full available width *inside* it adds them back and pushes the
    // block past its container. Take them off first.
    let inner = (ui.available_width() - 28.0).max(80.0);
    egui::Frame::none()
        .fill(tone.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0_f32, tone.gamma_multiply(0.35)))
        .rounding(Rounding::same(ROUND_FIELD))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(inner);
            kicker(ui, "WHAT THIS PROVES");
            ui.add_space(GAP_XS);
            ui.colored_label(tone, proof.summary());
        });

    ui.add_space(GAP_SM);
    egui::Frame::none()
        .fill(SURFACE_LOW)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(ROUND_FIELD))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(inner);
            kicker(ui, "WHAT IT DOES NOT");
            ui.add_space(GAP_XS);
            // One label, not a dash beside a label: a horizontal layout does
            // not wrap its children, so the long v1 warning ran straight off
            // the right edge of the card. The dash goes in the string.
            for line in proof.not_established() {
                ui.label(
                    RichText::new(format!("—  {line}"))
                        .color(TEXT_DIM)
                        .size(12.5),
                );
                ui.add_space(GAP_XS);
            }
        });
}

/// Verify a pasted receipt and remember the answer.
///
/// Kept out of the rendering so the card can be shown for a receipt this
/// wallet just produced as well as one that arrived from somebody else.
pub fn check_receipt(app: &mut App) {
    let text = app.proof_input.trim();
    if text.is_empty() {
        app.proof_result = None;
        return;
    }
    app.proof_result = Some(
        serde_json::from_str::<nightfall_wallet::PaymentReceipt>(text)
            .map_err(|e| format!("This is not a receipt this wallet can read: {e}"))
            .and_then(|receipt| {
                nightfall_wallet::verify_receipt(&receipt).map_err(|e| e.to_string())
            }),
    );
}

pub fn payment_request_card(
    ui: &mut egui::Ui,
    field: &str,
    network: NetworkId,
) -> Option<PaymentRequest> {
    let text = field.trim();
    // Cheap gate: only text that claims to be a request gets parsed at all.
    if !text
        .split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(payment_request::SCHEME))
    {
        return None;
    }

    let mut applied = None;
    divider(ui);
    ui.label(RichText::new("Payment request").size(11.0).color(TEXT_DIM));
    ui.add_space(6.0);

    match PaymentRequest::parse(text) {
        Err(error) => {
            ui.colored_label(DANGER, error.to_string());
        }
        Ok(request) => {
            let payable = request.require_network(network);
            // Monospace, because the only defence against a swapped address is a
            // person comparing it character by character.
            kv(
                ui,
                "Pays to",
                RichText::new(request.address.encode())
                    .monospace()
                    .size(11.5),
            );
            kv(ui, "Amount", RichText::new(request.amount_text()));
            if !request.memo.is_empty() {
                kv(ui, "Memo", RichText::new(&request.memo));
            }
            if !request.invoice.is_empty() {
                kv(ui, "Their reference", RichText::new(&request.invoice));
            }
            kv(ui, "Network", RichText::new(request.network.as_str()));

            if let Err(error) = &payable {
                ui.add_space(6.0);
                ui.colored_label(DANGER, error.to_string());
            } else {
                // Shown, never enforced. A payment made after a payee's own
                // deadline is perfectly valid on the chain; whether they still
                // honour it is between the two of them, and saying otherwise
                // would be inventing a consensus rule that does not exist.
                if request.is_expired(now_unix()) {
                    ui.add_space(6.0);
                    ui.colored_label(
                        WARN,
                        "The payee's own deadline for this request has passed. The \
                         chain does not care and the payment would still go through — \
                         but they may no longer treat it as settling this invoice. \
                         Ask before paying.",
                    );
                }
                ui.add_space(8.0);
                if primary_button(ui, "Use this request", true).clicked() {
                    applied = Some(request);
                }
                ui.label(
                    RichText::new("Fills in the address and amount below. Nothing is sent yet.")
                        .size(11.0)
                        .color(TEXT_FAINT),
                );
            }
        }
    }
    applied
}

pub fn send(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let tip = app.tip_height();
    let maturity = app.maturity();
    let balances = app
        .wallet
        .lock()
        .map(|w| w.balances(tip, maturity))
        .unwrap_or_default();
    let own_address = app
        .wallet
        .lock()
        .map(|w| w.address_string())
        .unwrap_or_default();

    // Three cards and a summary, rather than one long column.
    //
    // Everything used to live in a single card: address, book, amount, memo,
    // fee, button, stacked with blank space between them. Nothing said which
    // parts belonged together, so the form had to be read top to bottom every
    // time. The grouping here is the same one the user already has in their
    // head — who, how much, what it costs — and the summary at the end states
    // the consequence before the irreversible button.
    narrow_column(ui, 660.0, |ui| {
        ui.add_enabled_ui(!app.send_confirm, |ui| {
            let mut addr_state = None;

            titled_card(ui, "Recipient", |ui| {
                ui.set_width(ui.available_width());

                field_label(ui, "Address", None);
                ui.add(
                    egui::TextEdit::multiline(&mut app.send_to)
                        .margin(FIELD_MARGIN)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("nf1…"),
                );

                // Live validation — a wrong address must never reach a signature.
                // Untouched by the redesign: this is the check that stands between a
                // typo and a payment that cannot come back.
                let trimmed = app.send_to.trim().to_string();
                addr_state = if trimmed.is_empty() {
                    None
                } else if trimmed == own_address {
                    Some(Err("This is your own address".to_string()))
                } else {
                    Some(Address::decode(&trimmed).map_err(|e| e.to_string()))
                };

                ui.add_space(5.0);
                match &addr_state {
                    None => {
                        ui.label(
                            RichText::new("Paste the nf1 address the recipient shared with you.")
                                .size(11.0)
                                .color(TEXT_FAINT),
                        );
                    }
                    Some(Ok(a)) => {
                        ui.horizontal(|ui| {
                            dot(ui, SUCCESS, false);
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new(format!("Valid address · {}", a.short()))
                                    .size(11.0)
                                    .color(SUCCESS),
                            );
                        });
                    }
                    Some(Err(e)) => {
                        ui.horizontal(|ui| {
                            dot(ui, DANGER, false);
                            ui.add_space(3.0);
                            ui.label(RichText::new(e).size(11.0).color(DANGER));
                        });
                    }
                }

                // A pasted payment request is read here and applied only when
                // the owner presses the button.
                //
                // Nothing fills itself in. A request is someone else's
                // statement of what they want, and the gap between reading it
                // and acting on it is where a person gets to notice that the
                // address is not the one they expected.
                if let Some(applied) = payment_request_card(ui, &app.send_to, app.network) {
                    app.send_to = applied.address.encode();
                    if let Some(darks) = applied.amount_darks {
                        app.send_amount = Amount(darks).decimal_string();
                    }
                    if !applied.memo.is_empty() {
                        app.send_memo = applied.memo.clone();
                    }
                }

                if !app.address_book.entries.is_empty() {
                    divider(ui);
                    ui.label(RichText::new("Address book").size(11.0).color(TEXT_DIM));
                    ui.add_space(6.0);
                    let picks: Vec<(String, String)> = app
                        .address_book
                        .entries
                        .iter()
                        .map(|e| (e.name.clone(), e.address.clone()))
                        .collect();
                    ui.horizontal_wrapped(|ui| {
                        for (name, addr) in picks {
                            if ghost_button(ui, &format!("  {name}  ")).clicked() {
                                app.send_to = addr;
                            }
                        }
                    });
                }
            });

            ui.add_space(14.0);

            let mut amount_state = parse_amount(&app.send_amount);

            titled_card(ui, "Amount", |ui| {
                ui.set_width(ui.available_width());

                // The MAX button and the available balance belong to the amount
                // field, so they sit on its label row instead of floating above it.
                ui.horizontal(|ui| {
                    ui.label(RichText::new("NIGHT to send").size(12.0).color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("MAX").size(10.5).color(ACCENT_HI))
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::new(1.0_f32, ACCENT_DIM))
                                    .rounding(Rounding::same(999.0)),
                            )
                            .on_hover_text("Send everything, minus the fee")
                            .clicked()
                        {
                            let max = balances.available.saturating_sub(app.send_fee);
                            app.send_amount = night(max).replace('\u{202F}', "");
                        }
                        ui.label(
                            RichText::new(format!("Available {}", night(balances.available)))
                                .size(11.0)
                                .color(TEXT_FAINT),
                        );
                    });
                });
                ui.add_space(5.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.send_amount)
                        .margin(FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("0.00000000"),
                );

                amount_state = parse_amount(&app.send_amount);
                ui.add_space(4.0);
                // Only errors live here now. "Total debit" used to sit under the
                // field, competing with the memo and the fee for the same glance;
                // it belongs in the summary, where the reader is deciding.
                let amount_error = match &amount_state {
                    Ok(darks) => {
                        let total = darks.saturating_add(app.send_fee);
                        (total > balances.available)
                            .then(|| format!("Not enough — {} needed including fee", night(total)))
                    }
                    Err(_) if app.send_amount.trim().is_empty() => None,
                    Err(e) => Some(e.clone()),
                };
                message_slot(ui, |ui| {
                    if let Some(msg) = &amount_error {
                        dot(ui, DANGER, false);
                        ui.add_space(3.0);
                        ui.label(RichText::new(msg).size(11.0).color(DANGER));
                    }
                });

                divider(ui);

                // Memo
                field_label(
                    ui,
                    "Memo",
                    Some(
                        RichText::new(format!("{}/64", app.send_memo.len()))
                            .size(10.5)
                            .color(TEXT_FAINT),
                    ),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.send_memo)
                        .margin(FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .char_limit(64)
                        .hint_text("optional"),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Encrypted. Only the recipient can read it.")
                        .size(10.5)
                        .color(TEXT_FAINT),
                );
            });

            ui.add_space(14.0);

            // There is one fee, and it was never a choice.
            //
            // This card offered Economy, Standard and Priority. Nothing in the
            // protocol tells them apart: the fee is burned rather than paid to a
            // miner, and no part of the node orders the mempool by it. So
            // "Priority" promised faster confirmation that nothing delivers, and
            // "Economy" invited paying a tenth for exactly the same service —
            // three buttons where the phone wallet and the command line each show
            // one number. A choice that changes nothing is worse than no choice:
            // it makes people think they got it wrong when a payment is slow.
            app.send_fee = DEFAULT_FEE_DARKS;

            // --- what is about to happen -------------------------------------------
            //
            // The numbers that decide the payment, gathered in one recessed block
            // immediately above the button that commits it. Previously the total sat
            // three fields further up, next to the amount, and the button sat alone
            // under the fee chips — so the last thing read before pressing was the
            // burn note, not the sum leaving the wallet.
            let ready = matches!(addr_state, Some(Ok(_)))
                && amount_state
                    .as_ref()
                    .map(|d| d.saturating_add(app.send_fee) <= balances.available)
                    .unwrap_or(false)
                && !app.send_busy
                && app.wallet_sync_error.is_none();

            card(ui, |ui| {
                ui.set_width(ui.available_width());
                let amount = amount_state.as_ref().copied().unwrap_or(0);
                let total = amount.saturating_add(app.send_fee);

                well(ui, |ui| {
                    summary_row(ui, "Amount", RichText::new(night(amount)), false);
                    ui.add_space(6.0);
                    summary_row(
                        ui,
                        "Fee (burned)",
                        RichText::new(night(app.send_fee)).color(WARN),
                        false,
                    );
                    ui.add_space(8.0);
                    let w = ui.available_width();
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), egui::Sense::hover());
                    ui.painter().hline(
                        r.x_range(),
                        r.center().y,
                        Stroke::new(1.0_f32, BORDER.gamma_multiply(0.9)),
                    );
                    ui.add_space(8.0);
                    summary_row(
                        ui,
                        "Leaves your wallet",
                        RichText::new(night(total)).strong().size(15.0),
                        true,
                    );
                    if let Some(Ok(a)) = &addr_state {
                        ui.add_space(6.0);
                        summary_row(
                            ui,
                            "To",
                            RichText::new(a.short()).color(TEXT_DIM).size(12.0),
                            false,
                        );
                    }
                });

                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "The fee is fixed and destroyed, not paid to a miner. 100% burn.",
                    )
                    .size(10.5)
                    .color(TEXT_FAINT),
                );

                ui.add_space(14.0);
                if primary_button(ui, "Review payment", ready).clicked() {
                    app.send_confirm = true;
                }
            });
        });
    });

    // --- confirmation modal ---
    if app.send_confirm {
        let amount = parse_amount(&app.send_amount).unwrap_or(0);
        let mut do_send = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        egui::Window::new("Confirm payment")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::none()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0_f32, BORDER_HI))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(egui::Margin::same(22.0))
                    .shadow(egui::epaint::Shadow {
                        offset: egui::vec2(0.0, 8.0),
                        blur: 30.0,
                        spread: 0.0,
                        color: Color32::from_black_alpha(160),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("{} NIGHT", night(amount)))
                        .size(28.0)
                        .strong(),
                );
                ui.add_space(14.0);
                field_label(ui, "Recipient — verify the full address", None);
                ui.add(
                    egui::Label::new(RichText::new(app.send_to.trim()).monospace().size(12.0))
                        .wrap(),
                );
                kv(
                    ui,
                    "Fee (burned)",
                    RichText::new(night(app.send_fee)).monospace().color(WARN),
                );
                kv(
                    ui,
                    "Total",
                    RichText::new(night(amount.saturating_add(app.send_fee)))
                        .monospace()
                        .strong(),
                );
                if !app.send_memo.trim().is_empty() {
                    kv(ui, "Memo", RichText::new(app.send_memo.trim()));
                }

                ui.add_space(14.0);
                egui::Frame::none()
                    .fill(WARN.gamma_multiply(0.10))
                    .rounding(Rounding::same(ROUND_SM))
                    .inner_margin(egui::Margin::same(11.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(
                                "Payments cannot be reversed. Check the address once more.",
                            )
                            .size(11.5)
                            .color(WARN),
                        );
                    });

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if primary_button(
                        ui,
                        "  Send now  ",
                        !app.send_busy && app.wallet_sync_error.is_none(),
                    )
                    .clicked()
                    {
                        do_send = true;
                    }
                    if ghost_button(ui, "  Cancel  ").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            app.send_confirm = false;
        }
        if do_send {
            app.do_send(ctx);
        }
    }
}

// --------------------------------------------------------------- receive ---

pub fn receive(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let address = app
        .wallet
        .lock()
        .map(|w| w.address_string())
        .unwrap_or_default();

    let (scanned, outputs) = app
        .wallet
        .lock()
        .map(|w| (w.scanned_to(), w.outputs().len()))
        .unwrap_or_default();
    let tip = app.tip_height();

    // Two columns, because one narrow card against the left edge left two
    // thirds of the window empty. The right column is not filler: everything
    // in it is a question someone actually asks while waiting for a payment —
    // has the wallet caught up, and can I hand this address out twice.
    two_columns(
        ui,
        380.0,
        |ui| {
            card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    qr_code(ui, &address, 224.0);
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("Scan or copy to receive NIGHT")
                            .size(12.5)
                            .color(TEXT_DIM),
                    );
                });

                divider(ui);
                field_label(ui, "Your address", None);
                if copyable(ui, &address, true) {
                    app.toasts.success(ctx, "Address copied");
                }
            });
        },
        |ui| {
            titled_card(ui, "Safe to reuse", |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dot(ui, SUCCESS, false);
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Hand this address out as often as you like")
                            .size(12.5)
                            .color(SUCCESS)
                            .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Every payment creates a fresh one-time key on chain. Two payments to \
                         this address do not publish the shared receiving address. Keep in mind \
                         that a sender still knows the payments they made to you.",
                    )
                    .size(11.5)
                    .color(TEXT_DIM),
                );
            });

            ui.add_space(14.0);

            titled_card(ui, "Receiving status", |ui| {
                ui.set_width(ui.available_width());
                data_row(
                    ui,
                    "Chain tip",
                    RichText::new(format_int(tip)).monospace(),
                    false,
                );
                data_row(
                    ui,
                    "Wallet scanned to",
                    RichText::new(format_int(scanned))
                        .monospace()
                        .color(if scanned + 2 >= tip { SUCCESS } else { WARN }),
                    false,
                );
                data_row(
                    ui,
                    "Outputs found",
                    RichText::new(format_int(outputs as u64)).monospace(),
                    true,
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(if scanned + 2 >= tip {
                        "The wallet is level with the chain. A payment shows up as soon as \
                         it is in a block."
                    } else {
                        "The wallet is still reading older blocks. A payment that has already \
                         arrived will appear once the scan catches up — nothing is lost."
                    })
                    .size(11.0)
                    .color(TEXT_FAINT),
                );
            });
        },
    );

    ui.add_space(14.0);
    counter_card(app, ui, ctx, &address);
}

/// One invoice as the till shows it.
///
/// Separate from the card so it can be measured at every width with content
/// that is actually long — the candidate warning is the longest thing this
/// screen can say, and it is the sentence that must never be pushed off.
pub fn invoice_row(
    ui: &mut egui::Ui,
    invoice: &nightfall_wallet::counter::Invoice,
    state: &nightfall_wallet::counter::InvoiceStatus,
) {
    use nightfall_wallet::counter::InvoiceState;

    ui.horizontal_wrapped(|ui| {
        let tone = match state.state {
            InvoiceState::Paid { late: false } => SUCCESS,
            InvoiceState::Paid { late: true } | InvoiceState::Overpaid { .. } => WARN,
            InvoiceState::Closed => TEXT_DIM,
            InvoiceState::Open => TEXT_FAINT,
            InvoiceState::Underpaid { .. } | InvoiceState::Expired => DANGER,
        };
        badge(ui, &state.headline(), tone);
        ui.label(RichText::new(&invoice.reference).monospace().size(12.0));
        ui.label(
            RichText::new(match invoice.amount_darks {
                Some(darks) => format!("{}", Amount(darks)),
                None => "any amount".to_owned(),
            })
            .size(12.0),
        );
        if !invoice.description.is_empty() {
            ui.label(
                RichText::new(&invoice.description)
                    .size(12.0)
                    .color(TEXT_DIM),
            );
        }
    });
    if let Some(note) = &invoice.closed_note {
        ui.label(
            RichText::new(format!("Closed by you: {note}"))
                .size(11.0)
                .color(TEXT_FAINT),
        );
    }
    // Candidates are the honest part of this screen: money that arrived for
    // the right figure without saying what it was for. Named, counted, and
    // applied to nothing.
    if !state.candidates.is_empty() {
        let n = state.candidates.len();
        ui.label(
            RichText::new(if n == 1 {
                "One payment for this amount arrived without quoting the reference. \
                 It has not been applied to anything — check it in Activity before \
                 treating this as paid."
                    .to_owned()
            } else {
                format!(
                    "{n} payments for this amount arrived without quoting the \
                     reference. They have not been applied to anything — check them \
                     in Activity before treating this as paid."
                )
            })
            .size(11.0)
            .color(WARN),
        );
    }
}

/// The till: invoices this wallet asked to be paid, and where each one stands.
///
/// Matching lives in `nightfall_wallet::counter` and the rule it enforces is
/// worth repeating here, because this screen is where it would be tempting to
/// relax it: an invoice is never settled by an amount. A payment settles an
/// invoice only when it quotes that invoice's reference. Anything else is
/// shown as a candidate and left for the merchant to decide, because a till
/// that guesses will one day ask a customer to pay twice.
fn counter_card(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, address: &str) {
    use nightfall_wallet::counter::Invoice;

    let now = now_unix();
    let till = app
        .wallet
        .lock()
        .map(|w| w.till(now))
        .unwrap_or_default();

    titled_card(ui, "Counter", |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(
                "Ask to be paid for something specific, then watch for it. A payment \
                 settles an invoice when it quotes that invoice's reference — never \
                 by amount alone, because two customers owing the same figure would \
                 otherwise settle each other's bills.",
            )
            .size(12.0)
            .color(TEXT_DIM),
        );

        divider(ui);
        // Three fields on one line, sharing the width and sitting on the same
        // baseline. They were three fixed-width columns inside a wrapping row,
        // which gave each one a different top edge — the labels stepped
        // downwards across the form and nothing lined up with anything.
        let columns: [(&str, &str, &mut String); 3] = [
            ("Reference", "A-17", &mut app.till_reference),
            ("Amount in NIGHT", "1.50", &mut app.till_amount),
            ("What for", "Two coffees", &mut app.till_description),
        ];
        let gap = GAP_SM;
        let each = ((ui.available_width() - gap * 2.0) / 3.0).max(120.0);
        ui.horizontal_top(|ui| {
            // egui inserts `item_spacing.x` between items on top of anything
            // added by hand. Adding a gap as well made the row exactly two
            // spacings wider than the space it was given — which the layout
            // test caught as a 20-point overflow at 620 points.
            ui.spacing_mut().item_spacing.x = 0.0;
            for (index, (label, hint, value)) in columns.into_iter().enumerate() {
                if index > 0 {
                    ui.add_space(gap);
                }
                ui.allocate_ui_with_layout(
                    Vec2::new(each, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_width(each);
                        ui.label(RichText::new(label).size(11.5).color(TEXT_FAINT));
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::singleline(value)
                                .desired_width(f32::INFINITY)
                                .margin(egui::Margin::symmetric(12.0, 9.0))
                                .hint_text(RichText::new(hint).color(TEXT_FAINT)),
                        );
                    },
                );
            }
        });
        ui.add_space(GAP_MD);
        if primary_button(ui, "Add invoice", true).clicked() {
            let amount = app.till_amount.trim();
            let parsed = if amount.is_empty() {
                Ok(None)
            } else {
                parse_amount(amount).map(Some)
            };
            match parsed {
                Err(problem) => app.toasts.error(ctx, problem),
                Ok(amount_darks) => {
                    let invoice = Invoice {
                        reference: app.till_reference.clone(),
                        amount_darks,
                        description: app.till_description.clone(),
                        created_unix: now,
                        expires_unix: None,
                        closed_note: None,
                    };
                    let added = app
                        .wallet
                        .lock()
                        .map(|mut w| w.add_invoice(invoice))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("wallet is busy")));
                    match added {
                        Ok(()) => {
                            app.till_reference.clear();
                            app.till_amount.clear();
                            app.till_description.clear();
                            app.toasts.success(ctx, "Invoice added");
                        }
                        Err(problem) => app.toasts.error(ctx, problem.to_string()),
                    }
                }
            }
        }

        if till.is_empty() {
            return;
        }

        divider(ui);
        let takings: u64 = till
            .iter()
            .map(|(_, s)| {
                if s.is_settled() {
                    s.received_darks
                } else {
                    0
                }
            })
            .fold(0, u64::saturating_add);
        kv(
            ui,
            "Taken in (settled invoices only)",
            RichText::new(format!("{}", Amount(takings))).color(SUCCESS),
        );
        ui.add_space(8.0);

        let mut close: Option<String> = None;
        let mut remove: Option<String> = None;
        for (invoice, state) in &till {
            hairline(ui);
            ui.add_space(6.0);
            invoice_row(ui, invoice, state);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ghost_button(ui, "Copy request").clicked() {
                    let request = nightfall_wallet::counter::request_for(
                        invoice,
                        match Address::decode(address) {
                            Ok(a) => a,
                            Err(_) => return,
                        },
                        app.network,
                    );
                    ctx.copy_text(request.to_uri());
                    app.toasts
                        .success(ctx, "Request copied — it carries this invoice's reference");
                }
                if invoice.closed_note.is_none() && ghost_button(ui, "Close by hand").clicked() {
                    close = Some(invoice.reference.clone());
                }
                if ghost_button(ui, "Remove").clicked() {
                    remove = Some(invoice.reference.clone());
                }
            });
            ui.add_space(4.0);
        }

        if let Some(reference) = close {
            let done = app
                .wallet
                .lock()
                .map(|mut w| w.close_invoice(&reference, "closed at the counter"))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("wallet is busy")));
            match done {
                Ok(()) => app.toasts.success(ctx, "Invoice closed — no payment was invented"),
                Err(problem) => app.toasts.error(ctx, problem.to_string()),
            }
        }
        if let Some(reference) = remove {
            let done = app
                .wallet
                .lock()
                .map(|mut w| w.remove_invoice(&reference))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("wallet is busy")));
            if let Err(problem) = done {
                app.toasts.error(ctx, problem.to_string());
            }
        }
    });
}

// -------------------------------------------------------------- activity ---

/// Payments a restored backup carried in, which this wallet is withholding.
///
/// They are deliberately kept out of the stuck-payment notice further down.
/// A withheld payment is pending and almost always older than half an hour,
/// so that notice would describe it as a payment this wallet made and lost —
/// which is exactly what it is not. This wallet never sent it; it read it out
/// of a file and is refusing to put it back on the wire.
///
/// Returns true when the owner asked for the rescan screen.
pub fn withheld_notice(
    ui: &mut egui::Ui,
    entries: &[nightfall_wallet::HistoryEntry],
    now: u64,
) -> bool {
    let withheld: Vec<_> = entries
        .iter()
        .filter(|e| e.direction == Direction::Sent && e.needs_owner_decision())
        .collect();
    let Some(oldest) = withheld.iter().min_by_key(|e| e.timestamp) else {
        return false;
    };
    let n = withheld.len();
    let mut rescan = false;
    egui::Frame::none()
        .fill(DANGER.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0_f32, DANGER.gamma_multiply(0.5)))
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                dot(ui, DANGER, true);
                ui.add_space(6.0);
                ui.label(
                    RichText::new(if n == 1 {
                        "One payment from your backup is unresolved".to_string()
                    } else {
                        format!("{n} payments from your backup are unresolved")
                    })
                    .size(13.0)
                    .strong(),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Recorded {} in the backup you restored, and held back since. This \
                     wallet will not broadcast them on its own. A backup is a photograph \
                     of one moment: by now each of these may have confirmed, expired, or \
                     had its coins spent another way, and the file cannot tell you which. \
                     Check each against the chain before you spend.",
                    ago(oldest.timestamp, now)
                ))
                .size(11.5)
                .color(TEXT_DIM),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                rescan = ghost_button(ui, "Open Settings → Rescan").clicked();
                ui.label(
                    RichText::new("rebuilds this wallet from the chain the node has")
                        .size(11.0)
                        .color(TEXT_FAINT),
                );
            });
        });
    ui.add_space(12.0);
    rescan
}

pub fn activity(app: &mut App, ui: &mut egui::Ui) {
    let entries: Vec<_> = app
        .wallet
        .lock()
        .map(|w| w.history().to_vec())
        .unwrap_or_default();

    // A running total across the whole history, above the list.
    //
    // With no transactions this page was a filter box and one empty card on an
    // otherwise blank screen. With transactions it was a list that never
    // answered the first question anyone has about a list of payments: how
    // much, in total, in each direction. Both are fixed by the same strip, and
    // it is computed from the same entries the list draws, so it cannot
    // disagree with them.
    let (mut got, mut paid, mut mined, mut burned) = (0u64, 0u64, 0u64, 0u64);
    for e in &entries {
        match e.direction {
            Direction::Received => got = got.saturating_add(e.amount),
            Direction::Mined => mined = mined.saturating_add(e.amount),
            Direction::Sent => {
                paid = paid.saturating_add(e.amount);
                burned = burned.saturating_add(e.fee);
            }
        }
    }

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        metric_grid(
            ui,
            &[
                ("RECEIVED · NIGHT", night_compact(got), SUCCESS),
                ("MINED · NIGHT", night_compact(mined), ACCENT_HI),
                ("SENT · NIGHT", night_compact(paid), TEXT),
                ("FEES · NIGHT", night_compact(burned), WARN),
            ],
            false,
        );
    });
    ui.add_space(14.0);

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.activity_filter)
                .margin(FIELD_MARGIN)
                .desired_width(260.0)
                .hint_text("Filter by memo, direction or txid"),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{} entries", entries.len()))
                    .size(11.5)
                    .color(TEXT_FAINT),
            );
        });
    });
    ui.add_space(12.0);

    // A send that is not in a block will not get there by itself: Nightfall
    // hands a new transaction to exactly one peer and nothing rebroadcasts it.
    // Before this notice existed, the row just said "pending" forever and the
    // only way to find out what that meant was to read the source.
    let now_for_stuck = now_unix();
    const STUCK_AFTER_SECS: u64 = 30 * 60;

    if withheld_notice(ui, &entries, now_for_stuck) {
        app.view = View::Settings;
    }

    let stuck: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.direction == Direction::Sent
                && e.is_pending()
                && !e.quarantined
                && now_for_stuck.saturating_sub(e.timestamp) > STUCK_AFTER_SECS
        })
        .collect();
    if let Some(oldest) = stuck.iter().min_by_key(|e| e.timestamp) {
        let n = stuck.len();
        egui::Frame::none()
            .fill(WARN.gamma_multiply(0.10))
            .stroke(Stroke::new(1.0_f32, WARN.gamma_multiply(0.5)))
            .rounding(Rounding::same(ROUND_SM))
            .inner_margin(egui::Margin::symmetric(14.0, 12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dot(ui, WARN, true);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if n == 1 {
                            "One payment never made it into a block".to_string()
                        } else {
                            format!("{n} payments never made it into a block")
                        })
                        .size(13.0)
                        .strong(),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Sent {} and still in no block. A transaction is handed to one \
                         peer and nothing re-sends it, so this will not confirm on its \
                         own. The coins were never spent — they are still yours, but this \
                         wallet is holding them reserved for a payment that died.",
                        ago(oldest.timestamp, now_for_stuck)
                    ))
                    .size(11.5)
                    .color(TEXT_DIM),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ghost_button(ui, "Open Settings → Rescan").clicked() {
                        app.view = View::Settings;
                    }
                    ui.label(
                        RichText::new("releases the reserved coins, then send again")
                            .size(11.0)
                            .color(TEXT_FAINT),
                    );
                });
            });
        ui.add_space(12.0);
    }

    let needle = app.activity_filter.to_lowercase();
    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| {
            needle.is_empty()
                || e.memo.to_lowercase().contains(&needle)
                || e.direction.label().to_lowercase().contains(&needle)
                || e.txid.contains(&needle)
        })
        .collect();

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        if filtered.is_empty() {
            empty_state(
                ui,
                if entries.is_empty() {
                    "No transactions yet"
                } else {
                    "Nothing matches that filter"
                },
                if entries.is_empty() {
                    "Mining rewards and incoming payments appear here automatically."
                } else {
                    "Try a shorter memo, a transaction ID or another direction."
                },
            );
            if !entries.is_empty() && ghost_button(ui, "Clear filter").clicked() {
                app.activity_filter.clear();
            }
        } else {
            let now = now_unix();
            // Measured once, outside the loop. Inside it, `available_width`
            // shrinks after the first row and again when the scroll bar shows,
            // so every row got a different budget and the columns drifted.
            const RECEIPT_W: f32 = 88.0;
            let row_w = ui.available_width();
            // A list, not a stack of cards. The default spacing put ten points
            // between every row on top of each row's own margins, which is
            // what made five payments fill a window.
            ui.spacing_mut().item_spacing.y = 4.0;
            for e in filtered {
                let can_receipt = matches!(e.direction, Direction::Received | Direction::Mined);
                let body_w = (row_w - if can_receipt { RECEIPT_W } else { 0.0 }).max(240.0);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(body_w, 0.0),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.set_width(body_w);
                            activity_row(ui, e, now);
                        },
                    );
                    if can_receipt && ghost_button(ui, "Receipt").clicked() {
                        match app
                            .wallet
                            .lock()
                            .ok()
                            .and_then(|w| w.receipt_json(&e.txid).ok())
                        {
                            Some(json) => {
                                ui.ctx().copy_text(json.clone());
                                // Put it through the same check the recipient
                                // will run, and show the same card. Handing
                                // over a disclosure without seeing what it
                                // discloses is how people reveal more than
                                // they meant to.
                                app.proof_input = json;
                                check_receipt(app);
                                // …and take the owner to it. The card is below
                                // the whole history list, so on a wallet with
                                // any history at all "see the card below" was
                                // an instruction to go looking.
                                app.proof_scroll = true;
                                app.toasts.success(
                                    ui.ctx(),
                                    "Receipt copied — the Proof Card below shows what it proves",
                                );
                            }
                            None => app.toasts.error(ui.ctx(), "Could not build a receipt"),
                        }
                    }
                });
            }
        }
    });

    ui.add_space(14.0);

    titled_card(ui, "Proof Card", |ui| {
        ui.set_width(ui.available_width());
        if std::mem::take(&mut app.proof_scroll) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        ui.label(
            RichText::new(
                "Check a receipt — one you were given, or one you are about to give. A \
                 receipt discloses a single payment without handing over the view key \
                 that would show every other one.",
            )
            .size(12.0)
            .color(TEXT_DIM),
        );
        ui.add_space(10.0);
        field_label(ui, "Receipt", None);
        let changed = ui
            .add(
                egui::TextEdit::multiline(&mut app.proof_input)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Paste the receipt JSON"),
            )
            .changed();
        // Re-check as it is typed, and — the part that matters — throw the old
        // answer away the moment the text changes. A verdict left standing
        // beside a different receipt is a verdict about the wrong document.
        if changed {
            check_receipt(app);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ghost_button(ui, "Check").clicked() {
                check_receipt(app);
            }
            if ghost_button(ui, "Clear").clicked() {
                app.proof_input.clear();
                app.proof_result = None;
            }
        });
        match &app.proof_result {
            None => {}
            Some(Err(problem)) => {
                ui.add_space(10.0);
                ui.colored_label(DANGER, problem);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Nothing about this document has been established. Do not treat \
                         it as evidence of anything.",
                    )
                    .size(12.0)
                    .color(TEXT_DIM),
                );
            }
            Some(Ok(proof)) => {
                ui.add_space(12.0);
                divider(ui);
                ui.add_space(10.0);
                proof_card(ui, proof);
            }
        }
    });
}

// ---------------------------------------------------------------- mining ---

pub fn mining(app: &mut App, ui: &mut egui::Ui) {
    let mining = app.is_mining();
    let (difficulty, blocks_found, hashes_total, blocks) = app
        .status
        .as_ref()
        .map(|s| (s.difficulty, s.blocks_found, s.hashes_total, s.blocks))
        .unwrap_or((0, 0, 0, 0));

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            dot(ui, if mining { SUCCESS } else { TEXT_FAINT }, mining);
            ui.add_space(6.0);
            ui.label(
                RichText::new(if mining {
                    "Mining active"
                } else {
                    "Mining stopped"
                })
                .size(16.0)
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if mining { "Stop" } else { "Start mining" };
                if primary_button(ui, label, true).clicked() {
                    app.set_mining(!mining);
                }
            });
        });

        ui.add_space(16.0);
        let lifetime = lifetime_mined(app);
        metric_grid(
            ui,
            &[
                (
                    "HASHRATE",
                    format_hashrate(app.hashrate.current),
                    if mining { ACCENT_HI } else { TEXT_DIM },
                ),
                ("THIS SESSION", format_int(blocks_found), SUCCESS),
                ("LIFETIME MINED", night_compact(lifetime), TEXT),
                ("TOTAL HASHES", format_int(hashes_total), TEXT),
            ],
            false,
        );

        // Hashrate sparkline
        if app.hashrate.history.len() > 2 {
            ui.add_space(16.0);
            sparkline(ui, &app.hashrate.history, 54.0);
        }

        ui.add_space(16.0);
        let max_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .clamp(1, 64) as i32;
        ui.label(RichText::new("CPU threads").size(12.0).color(TEXT_DIM));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut n = app.mining_threads as i32;
            let resp = ui.add(egui::Slider::new(&mut n, 1..=max_threads).integer());
            if resp.changed() {
                app.set_mining_threads(n as usize);
            }
            ui.label(
                RichText::new(format!("{} × 32 MiB RAM", app.mining_threads))
                    .size(11.0)
                    .color(TEXT_FAINT),
            );
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Each thread holds its own Argon2 workspace. Leave one core for the wallet. Change lands on the next template — no restart.",
            )
            .size(11.0)
            .color(TEXT_FAINT),
        );
    });

    ui.add_space(14.0);

    ui.columns(2, |cols| {
        titled_card(&mut cols[0], "Difficulty", |ui| {
            ui.label(
                RichText::new(format_int(difficulty))
                    .size(22.0)
                    .monospace()
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Retargeted every block using a linearly-weighted moving average over \
                     the last 90 blocks, aiming at 15 second intervals.",
                )
                .size(11.5)
                .color(TEXT_DIM),
            );
            ui.add_space(10.0);
            let expected = if app.hashrate.current > 0.0 {
                let secs = difficulty as f64 / app.hashrate.current.max(1.0);
                if secs < 90_000.0 {
                    format!("≈ one block every {}", human_duration(secs))
                } else {
                    "Block time beyond a day at this hashrate".to_string()
                }
            } else {
                "Start mining to estimate your block time".to_string()
            };
            ui.label(RichText::new(expected).size(12.0).color(ACCENT_HI));
        });

        titled_card(&mut cols[1], "Rewards", |ui| {
            let maturity = app.maturity();
            let tip = app.tip_height();
            let balances = app
                .wallet
                .lock()
                .map(|w| w.balances(tip, maturity))
                .unwrap_or_default();

            kv(
                ui,
                "Block subsidy",
                RichText::new(format!("{} NIGHT", night(reward_at(blocks)))).monospace(),
            );
            kv(
                ui,
                "Immature",
                RichText::new(night(balances.immature))
                    .monospace()
                    .color(WARN),
            );
            kv(
                ui,
                "Maturity",
                RichText::new(format!("{maturity} blocks")).monospace(),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Mining pays only the block subsidy. Transaction fees are burned, so \
                     nobody profits from congestion.",
                )
                .size(11.5)
                .color(TEXT_DIM),
            );
        });
    });

    // --- rewards still ripening ---
    let ripening: Vec<(u64, u64)> = app
        .wallet
        .lock()
        .map(|w| {
            let mut v: Vec<(u64, u64)> = w
                .outputs()
                .iter()
                .filter(|o| !o.spent)
                .filter_map(|o| {
                    w.blocks_until_mature(o, app.tip_height(), app.maturity())
                        .map(|left| (o.value, left))
                })
                .collect();
            v.sort_by_key(|(_, left)| *left);
            v.truncate(8);
            v
        })
        .unwrap_or_default();

    if !ripening.is_empty() {
        ui.add_space(14.0);
        titled_card(ui, "Rewards unlocking soon", |ui| {
            ui.set_width(ui.available_width());
            for (value, blocks_left) in ripening {
                let progress_pct =
                    1.0 - (blocks_left as f32 / app.maturity().max(1) as f32).clamp(0.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(night(value))
                            .monospace()
                            .size(12.5)
                            .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} blocks left", format_int(blocks_left)))
                                .size(11.5)
                                .color(WARN),
                        );
                    });
                });
                ui.add_space(3.0);
                progress(ui, progress_pct, "");
                ui.add_space(9.0);
            }
        });
    }
}

fn next_unlock(app: &App) -> Option<(u64, u64)> {
    let tip = app.tip_height();
    let maturity = app.maturity();
    app.wallet.lock().ok().and_then(|w| {
        let mut soonest: Option<(u64, u64)> = None;
        for o in w.outputs().iter().filter(|o| !o.spent) {
            if let Some(left) = w.blocks_until_mature(o, tip, maturity) {
                match soonest {
                    Some((_, best)) if left >= best => {}
                    _ => soonest = Some((o.value, left)),
                }
            }
        }
        soonest
    })
}

fn lifetime_mined(app: &App) -> u64 {
    app.wallet
        .lock()
        .ok()
        .map(|w| {
            w.history()
                .iter()
                .filter(|e| e.direction == Direction::Mined && e.height.is_some())
                .map(|e| e.amount)
                .sum()
        })
        .unwrap_or(0)
}

fn format_peer_versions(map: &std::collections::BTreeMap<String, usize>) -> String {
    if map.is_empty() {
        return "No handshake versions yet".into();
    }
    let mut merged: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (agent, n) in map {
        let label = agent.rsplit('/').next().unwrap_or(agent).to_string();
        *merged.entry(label).or_insert(0) += *n;
    }
    merged
        .into_iter()
        .map(|(v, n)| format!("{n}× {v}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn reward_at(height: u64) -> u64 {
    let halvings = height / nightfall_types::HALVING_INTERVAL_BLOCKS;
    if halvings >= 64 {
        return 0;
    }
    (nightfall_types::INITIAL_BLOCK_REWARD_NIGHT * DARKS_PER_NIGHT) >> halvings
}

fn peers_zero_reason(app: &App, peers: usize, blocks: u64) -> Option<String> {
    if peers > 0 {
        return None;
    }
    if crate::app::IS_DEV_BUILD {
        return Some("Local test build: zero peers is expected. Start mining to create Devnet test coins; this app does not sync Mainnet and test coins have no value.".into());
    }
    let s = app.status.as_ref()?;
    if s.loading {
        return Some(format!(
            "P2P is closed until the chain file is loaded. Last tip on disk: block {}.",
            format_int(s.blocks)
        ));
    }
    let port = app.network.default_p2p_port();
    let mut msg = String::new();
    if let Some(err) = &s.last_dial_error {
        msg.push_str(&format!("Last dial failed: {err}. "));
    }
    if blocks <= 1 {
        msg.push_str(&format!(
            "This node is still at genesis. Allow outbound TCP {port} \
             (Windows Defender often blocks it). 0.7.3+ also fetches listeners \
             from nightfallcoin.org/peers."
        ));
    } else {
        msg.push_str(&format!(
            "No live socket. Outbound to a seed on port {port} is enough — \
             you do not have to be reachable. If this stays at zero, {port} is \
             blocked or the compiled seed is full; 0.7.3+ asks \
             https://nightfallcoin.org/peers for other listeners."
        ));
    }
    Some(msg)
}

fn human_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.0} s")
    } else if secs < 3600.0 {
        format!("{:.1} min", secs / 60.0)
    } else {
        format!("{:.1} h", secs / 3600.0)
    }
}

fn sparkline(ui: &mut egui::Ui, data: &[f64], height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let max = data.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
    let n = data.len();
    if n < 2 {
        return;
    }

    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), SURFACE_HI);

    let points: Vec<egui::Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.min.x + 6.0 + (rect.width() - 12.0) * (i as f32 / (n - 1) as f32);
            let y = rect.max.y - 6.0 - (rect.height() - 12.0) * (*v / max) as f32;
            egui::pos2(x, y)
        })
        .collect();

    painter.add(egui::Shape::line(
        points.clone(),
        Stroke::new(1.8_f32, ACCENT_HI),
    ));

    if let Some(last) = points.last() {
        painter.circle_filled(*last, 3.0, ACCENT_HI);
    }
}

// --------------------------------------------------------------- network ---

pub fn network(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let status = app.status.clone();
    let s = status.as_ref();

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        let peers = s.map(|s| s.live_peers).unwrap_or(0);
        ui.horizontal(|ui| {
            dot(ui, if peers > 0 { SUCCESS } else { WARN }, false);
            ui.add_space(6.0);
            ui.label(
                RichText::new(if peers > 0 {
                    format!("Connected to {peers} peer(s)")
                } else if s.map(|s| s.loading).unwrap_or(false) {
                    "No peers — chain still loading".to_string()
                } else if crate::app::IS_DEV_BUILD {
                    "Local Devnet — isolated test node".to_string()
                } else {
                    "No peers — disconnected".to_string()
                })
                .size(16.0)
                .strong(),
            );
        });
        if let Some(reason) = peers_zero_reason(app, peers, s.map(|s| s.blocks).unwrap_or(0)) {
            ui.add_space(8.0);
            ui.label(RichText::new(reason).size(12.0).color(TEXT_DIM));
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Add a peer").size(12.0).color(TEXT_DIM));
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut app.peer_input)
                    .margin(FIELD_MARGIN)
                    .desired_width(280.0)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("host:port"),
            );
            let submit = ui.add_enabled(
                !app.peer_input.trim().is_empty(),
                egui::Button::new("Connect")
                    .fill(SURFACE_HI)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .rounding(Rounding::same(ROUND_PILL))
                    .min_size(Vec2::new(0.0, 34.0)),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if submit.clicked() || enter {
                if let Some(node) = &app.node {
                    match node.add_peer(&app.peer_input) {
                        Ok(()) => {
                            app.toasts
                                .success(ctx, format!("Peer added: {}", app.peer_input.trim()));
                            app.peer_input.clear();
                        }
                        Err(e) => app.toasts.error(ctx, e.to_string()),
                    }
                }
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "Live sockets: {peers}. Others reach you on port {}. Forward it to \
                 accept incoming connections. Outbound to a seed is enough to \
                 stay on the tip — you do not have to be dialable. Transactions \
                 leave this node as a Dandelion stem (one random hop), not a \
                 broadcast.",
                app.network.default_p2p_port()
            ))
            .size(11.0)
            .color(TEXT_FAINT),
        );

        let known = app.node.as_ref().map(|n| n.peers()).unwrap_or_default();
        if !known.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Known peers").size(12.0).color(TEXT_DIM));
            ui.add_space(4.0);
            for p in known.iter().take(12) {
                ui.label(RichText::new(p).monospace().size(11.5).color(TEXT_FAINT));
            }
        }

        ui.add_space(12.0);
        ui.label(RichText::new("Peer versions").size(12.0).color(TEXT_DIM));
        ui.add_space(4.0);
        let empty_versions = std::collections::BTreeMap::new();
        ui.label(
            RichText::new(format_peer_versions(
                s.map(|st| &st.peer_versions).unwrap_or(&empty_versions),
            ))
            .size(12.0)
            .color(if peers > 0 { TEXT } else { TEXT_FAINT }),
        );
    });

    ui.add_space(14.0);

    let tor_on = app.status.as_ref().map(|st| st.tor_proxy).unwrap_or(false);
    titled_card(ui, "Network privacy", |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(
                "Outbound connections can go through Tor. Your ISP then sees only \
                 a SOCKS handshake, not which seed you dial. Destination hostnames \
                 are not resolved locally.",
            )
            .size(12.0)
            .color(TEXT_DIM),
        );
        ui.add_space(10.0);
        kv(
            ui,
            "SOCKS5 / Tor",
            RichText::new(if tor_on { "on" } else { "off" })
                .monospace()
                .color(if tor_on { SUCCESS } else { TEXT_FAINT }),
        );
        kv(
            ui,
            "Tx relay",
            RichText::new("Dandelion stem / fluff").monospace(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.proxy_input)
                    .margin(FIELD_MARGIN)
                    .desired_width(280.0)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("127.0.0.1:9050"),
            );
            if ghost_button(ui, "Apply").clicked() {
                match app.apply_proxy() {
                    Ok(()) => app.toasts.success(
                        ctx,
                        if matches!(app.proxy_input.trim(), "" | "off" | "none" | "clearnet") {
                            "Tor off — new dials go clearnet"
                        } else {
                            "SOCKS5 saved — new outbound dials try Tor first"
                        },
                    ),
                    Err(e) => app.toasts.error(ctx, e.to_string()),
                }
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Tor is the default (127.0.0.1:9050). If Tor is down, dials \
                 fall back to clearnet and this meter goes off. Type off to \
                 disable. .onion seeds never fall back.",
            )
            .size(11.0)
            .color(TEXT_FAINT),
        );
    });

    ui.add_space(14.0);

    titled_card(ui, "Chain", |ui| {
        ui.set_width(ui.available_width());
        kv(
            ui,
            "Network",
            RichText::new(app.network.as_str()).monospace(),
        );
        kv(
            ui,
            "Protocol version",
            RichText::new(nightfall_types::PROTOCOL_VERSION.to_string()).monospace(),
        );
        kv(
            ui,
            "Wallet version",
            RichText::new(crate::app::WALLET_VERSION).monospace(),
        );
        kv(
            ui,
            "Blocks",
            RichText::new(format_int(s.map(|s| s.blocks).unwrap_or(0))).monospace(),
        );
        kv(
            ui,
            "Tip",
            RichText::new(short_hex(&s.map(|s| s.tip.clone()).unwrap_or_default())).monospace(),
        );
        let tip_full = s.map(|st| st.tip.clone()).unwrap_or_default();
        if !tip_full.is_empty() && copyable(ui, &tip_full, true) {
            app.toasts.success(ctx, "Tip hash copied");
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let busy = app
                .chain_check_busy
                .load(std::sync::atomic::Ordering::SeqCst);
            if ghost_button(
                ui,
                if busy {
                    "  Checking…  "
                } else {
                    "  Same chain as the seed?  "
                },
            )
            .clicked()
                && !busy
            {
                app.start_chain_check();
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Asks nightfallcoin.org/network.json. The site sees that you checked — not a private probe.",
            )
            .size(10.5)
            .color(TEXT_FAINT),
        );
        if let Some(check) = &app.chain_check {
            ui.add_space(8.0);
            if let Some(err) = &check.error {
                ui.label(RichText::new(err).size(12.0).color(DANGER));
            } else if check.same {
                ui.label(
                    RichText::new(format!(
                        "Same tip as the public seed (height {}). Genesis {}.",
                        format_int(check.public_height),
                        short_hex(&check.genesis)
                    ))
                    .size(12.0)
                    .color(SUCCESS),
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "Different tip. Yours {} @ {}. Seed {} @ {}.",
                        short_hex(&check.our_tip),
                        format_int(check.our_height),
                        short_hex(&check.public_tip),
                        format_int(check.public_height)
                    ))
                    .size(12.0)
                    .color(WARN),
                );
            }
        }
        kv(
            ui,
            "Total work",
            RichText::new(format_int(
                s.map(|s| s.total_work).unwrap_or(0).min(u64::MAX as u128) as u64,
            ))
            .monospace(),
        );
        kv(
            ui,
            "P2P port",
            RichText::new(app.network.default_p2p_port().to_string()).monospace(),
        );
        kv(
            ui,
            "Mempool",
            RichText::new(s.map(|s| s.mempool).unwrap_or(0).to_string()).monospace(),
        );
    });

    ui.add_space(14.0);

    titled_card(ui, "Ledger state", |ui| {
        ui.set_width(ui.available_width());
        kv(
            ui,
            "UTXOs",
            RichText::new(format_int(s.map(|s| s.utxos).unwrap_or(0) as u64)).monospace(),
        );
        kv(
            ui,
            "Kernels",
            RichText::new(format_int(s.map(|s| s.kernels).unwrap_or(0))).monospace(),
        );
        kv(
            ui,
            "UTXO root",
            RichText::new(short_hex(
                &s.map(|s| s.utxo_root.clone()).unwrap_or_default(),
            ))
            .monospace(),
        );
        let ok = s.map(|s| s.supply_ok).unwrap_or(false);
        kv(
            ui,
            "Supply invariant",
            RichText::new(if ok { "verified" } else { "FAILED" }).color(status_color(ok)),
        );
    });
}

// -------------------------------------------------------------- settings ---

pub fn settings(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    // Centred rather than pinned left. Seven cards in a 700px column against
    // the left edge of a wide window is most of a screen of nothing.
    narrow_column(ui, 760.0, |ui| {
        app.show_vault_settings(ui, ctx);
        if app.vault_ui.busy() {
            return;
        }
        ui.add_space(14.0);
        let address = app.wallet.lock().ok().and_then(|wallet| wallet.address());
        app.recovery_studio.show(ui, address.as_ref());
        ui.add_space(14.0);
        titled_card(ui, "Recovery words", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(
                    "The 24 words are the wallet — the same phrase the phone and browser use. \
                 Anyone who reads them owns the coins. Write them on paper. Never a screenshot, \
                 chat or cloud note. These words are not a Bitcoin seed.",
                )
                .size(12.0)
                .color(TEXT_DIM),
            );
            ui.add_space(12.0);

            let (storage_path, encrypted) = app
                .wallet
                .lock()
                .map(|w| {
                    if w.is_vault() {
                        (
                            w.seed_path
                                .with_file_name("core.seed.vault")
                                .join("wallet.nfv")
                                .display()
                                .to_string(),
                            true,
                        )
                    } else {
                        (w.seed_path.display().to_string(), false)
                    }
                })
                .unwrap_or_default();
            kv(
                ui,
                "Wallet storage",
                RichText::new(storage_path).monospace().size(11.0),
            );
            kv(
                ui,
                "Protection",
                RichText::new(if encrypted {
                    "Encrypted Vault"
                } else {
                    "Legacy seed — not encrypted"
                })
                .color(if encrypted { SUCCESS } else { WARN }),
            );

            ui.add_space(12.0);
            if app.reveal_mnemonic {
                let phrase = app
                    .wallet
                    .lock()
                    .map(|w| w.recovery_phrase())
                    .unwrap_or_default();
                if copyable(ui, &phrase, true) {
                    app.toasts.info(ctx, "Phrase copied — handle with care");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ghost_button(ui, "Hide words").clicked() {
                        app.reveal_mnemonic = false;
                    }
                    if !app.backup_acked && ghost_button(ui, "I wrote these words down").clicked() {
                        app.ack_backup();
                        app.toasts.success(ctx, "Backup acknowledged");
                    }
                });
            } else if ghost_button(ui, "Reveal 24 words").clicked() {
                app.reveal_mnemonic = true;
            }

            ui.add_space(10.0);
            if app.reveal_seed {
                let seed = app.wallet.lock().map(|w| w.seed_hex()).unwrap_or_default();
                ui.label(
                    RichText::new("Hex seed (same 32 bytes as the words)")
                        .size(11.0)
                        .color(TEXT_FAINT),
                );
                ui.add_space(4.0);
                if copyable(ui, &seed, true) {
                    app.toasts.info(ctx, "Hex seed copied — handle with care");
                }
                ui.add_space(6.0);
                if ghost_button(ui, "Hide hex").clicked() {
                    app.reveal_seed = false;
                }
            } else if ghost_button(ui, "Show hex seed").clicked() {
                app.reveal_seed = true;
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "View key", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(
                    "A view key lets someone see every amount and memo you send or receive — an \
                 accountant or auditor, for example. It cannot spend anything. To prove a \
                 single payment instead, copy a Receipt from the Activity list.",
                )
                .size(12.0)
                .color(TEXT_DIM),
            );
            ui.add_space(12.0);
            if app.reveal_view_key {
                let vk = app
                    .wallet
                    .lock()
                    .map(|w| w.view_key_string())
                    .unwrap_or_default();
                if copyable(ui, &vk, true) {
                    app.toasts.success(ctx, "View key copied");
                }
                ui.add_space(6.0);
                if ghost_button(ui, "Hide view key").clicked() {
                    app.reveal_view_key = false;
                }
            } else if ghost_button(ui, "Show view key").clicked() {
                app.reveal_view_key = true;
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "Wallet", |ui| {
            ui.set_width(ui.available_width());
            let (scanned, outputs) = app
                .wallet
                .lock()
                .map(|w| (w.scanned_to(), w.outputs().len()))
                .unwrap_or((0, 0));
            kv(
                ui,
                "Scanned to block",
                RichText::new(format_int(scanned)).monospace(),
            );
            kv(
                ui,
                "Known outputs",
                RichText::new(outputs.to_string()).monospace(),
            );
            kv(
                ui,
                "Data folder",
                RichText::new(app.datadir.display().to_string())
                    .monospace()
                    .size(11.0),
            );

            ui.add_space(12.0);
            let pruned = app.status.as_ref().map(|s| s.pruned).unwrap_or(false);
            let prune_height = app.status.as_ref().map(|s| s.prune_height).unwrap_or(0);
            if pruned {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "This node is pruned. Bodies start at height {}. \
                     Rescan from genesis needs an archive node or the phone/web light API.",
                        format_int(prune_height)
                    ))
                    .size(11.5)
                    .color(WARN),
                );
            }
            ui.add_space(12.0);
            if ghost_button(ui, "Rescan from genesis").clicked() {
                if let Some(node) = app.node.clone() {
                    let wallet = std::sync::Arc::clone(&app.wallet);
                    let signal = std::sync::Arc::clone(&app.sync_signal);
                    let syncing = std::sync::Arc::clone(&app.syncing);
                    let access = std::sync::Arc::clone(&app.wallet_scan_access);
                    let paused = std::sync::Arc::clone(&app.wallet_paused);
                    syncing.store(true, std::sync::atomic::Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let Ok(_access) = access.lock() else {
                            syncing.store(false, std::sync::atomic::Ordering::SeqCst);
                            return;
                        };
                        let result = wallet
                            .lock()
                            .map_err(|_| "wallet busy".to_string())
                            .and_then(|mut w| {
                                if paused.load(std::sync::atomic::Ordering::SeqCst) || !w.can_scan()
                                {
                                    return Err("Rescan cancelled: wallet is being secured.".into());
                                }
                                w.rescan(&node).map_err(|e| e.to_string())
                            });
                        syncing.store(false, std::sync::atomic::Ordering::SeqCst);
                        if let Ok(mut slot) = signal.lock() {
                            *slot = Some(result);
                        }
                    });
                    app.toasts.info(ctx, "Rescanning from genesis…");
                }
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Re-reads the chain and replaces local history; notes/receipts may be lost. Export a backup first. Blocked while payments or reservations are pending.")
                    .size(10.5)
                    .color(TEXT_FAINT),
            );

            ui.add_space(16.0);
            ui.label(
                RichText::new(
                    "Resync the chain file if BLOCKS is frozen on a dead branch. \
                 Wallet keys stay. Coinbase mined on the abandoned tip does not come back.",
                )
                .size(12.0)
                .color(TEXT_DIM),
            );
            ui.add_space(8.0);
            if app.resync_confirm {
                ui.label(
                RichText::new(
                    "This backs up the chain file and downloads it again. Wallet keys stay. This can take minutes to hours.",
                )
                .size(11.5)
                .color(WARN),
            );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "  Resync now  ", true).clicked() {
                        app.resync_chain(ctx);
                    }
                    if ghost_button(ui, "Cancel").clicked() {
                        app.resync_confirm = false;
                    }
                });
            } else if ghost_button(ui, "Resync chain, keep wallet").clicked() {
                app.resync_confirm = true;
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "Bitcoin node (atomic swaps)", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(
                    "bitcoind JSON-RPC. Mode 0600, three lines: url=, user=, password=. \
                     Needs -txindex=1. This wallet never holds Bitcoin keys.",
                )
                .size(12.0)
                .color(TEXT_DIM),
            );
            ui.add_space(10.0);
            kv(
                ui,
                "Config",
                RichText::new(app.btc_rpc_path().display().to_string())
                    .monospace()
                    .size(11.0),
            );
            ui.add_space(8.0);
            if app.bitcoin_rpc_configured() {
                ui.label(
                    RichText::new("Configuration saved. Connection and network checks are shown on the Swap page.")
                        .size(11.5)
                        .color(TEXT_DIM),
                );
                if ghost_button(ui, "Check swap connection").clicked() {
                    app.view = View::Swap;
                    app.reveal_seed = false;
                    app.reveal_mnemonic = false;
                    app.reveal_view_key = false;
                }
            } else if ghost_button(ui, "Write credential template").clicked() {
                match app.write_bitcoin_rpc_template() {
                    Ok(msg) => app.toasts.success(ctx, msg),
                    Err(e) => app.toasts.error(ctx, e),
                }
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "Address book", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Local labels for nf1 addresses you send to. Never published.")
                    .size(12.0)
                    .color(TEXT_DIM),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.book_name)
                        .margin(FIELD_MARGIN)
                        .desired_width(140.0)
                        .hint_text("Name"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.book_addr)
                        .margin(FIELD_MARGIN)
                        .desired_width(280.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("nf1…"),
                );
                if ghost_button(ui, "Add").clicked() {
                    match app
                        .address_book
                        .add(app.book_name.clone(), app.book_addr.clone())
                    {
                        Ok(()) => {
                            let _ = app.address_book.save(&app.datadir);
                            app.book_name.clear();
                            app.book_addr.clear();
                            app.toasts.success(ctx, "Contact saved");
                        }
                        Err(e) => app.toasts.error(ctx, e),
                    }
                }
            });
            ui.add_space(8.0);
            let entries = app.address_book.entries.clone();
            for e in entries {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&e.name).size(12.5));
                    ui.label(
                        RichText::new(short_hex(&e.address))
                            .monospace()
                            .size(11.0)
                            .color(TEXT_FAINT),
                    );
                    if ghost_button(ui, "Remove").clicked() {
                        app.address_book.remove(&e.address);
                        let _ = app.address_book.save(&app.datadir);
                    }
                });
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "Storage", |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut prune = app.prune;
                if ui
                    .checkbox(&mut prune, "Prune old blocks — keep UTXO + last 500 bodies")
                    .changed()
                {
                    app.set_prune(prune, ctx);
                }
            });
            ui.add_space(4.0);
            ui.label(
            RichText::new(
                "Laptop default for a chain that no longer fits in RAM. Seeds stay full archives. \
                 After prune you cannot rescan stealth outputs from genesis on this machine — \
                 use the phone/web wallet (light API) or Resync chain from a seed.",
            )
            .size(11.0)
            .color(TEXT_FAINT),
        );
            if let Some(s) = &app.status {
                if s.pruned {
                    ui.add_space(6.0);
                    kv(
                        ui,
                        "Bodies from",
                        RichText::new(format_int(s.prune_height)).monospace(),
                    );
                }
            }
        });

        ui.add_space(14.0);

        titled_card(ui, "Window", |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut tray = app.close_to_tray;
                if ui
                    .checkbox(&mut tray, "Close to tray — mining keeps running")
                    .changed()
                {
                    app.set_close_to_tray(tray);
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "On Windows the process looks gone if the window closes. Tray Show / Quit. \
                 macOS keeps the dock icon either way.",
                )
                .size(11.0)
                .color(TEXT_FAINT),
            );
        });

        ui.add_space(14.0);

        titled_card(ui, "About", |ui| {
            ui.set_width(ui.available_width());
            kv(
                ui,
                "Max supply",
                RichText::new(format!("{} NIGHT", format_int(MAX_SUPPLY_NIGHT))).monospace(),
            );
            kv(ui, "Premine", RichText::new("0").monospace().color(SUCCESS));
            kv(
                ui,
                "Fee model",
                RichText::new("100% burned").monospace().color(WARN),
            );
            kv(
                ui,
                "Amount privacy",
                RichText::new("Pedersen + Bulletproofs").color(SUCCESS),
            );
            kv(
                ui,
                "Recipient privacy",
                RichText::new("one-time keys, unlinkable").color(SUCCESS),
            );
            kv(
                ui,
                "Graph privacy",
                RichText::new("obscured by aggregation, not erased").color(WARN),
            );
            kv(
                ui,
                "Proof of work",
                RichText::new("Nighthash-v2 · Argon2id 32 MiB").color(SUCCESS),
            );

            ui.add_space(12.0);
            egui::Frame::none()
                .fill(WARN.gamma_multiply(0.10))
                .rounding(Rounding::same(ROUND_SM))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Not independently audited.")
                            .size(12.0)
                            .color(WARN)
                            .strong(),
                    );
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(
                            "Amounts and addresses are hidden. The transaction graph is still \
                         linkable. Do not treat NIGHT as money you can afford to lose.",
                        )
                        .size(11.0)
                        .color(TEXT_DIM),
                    );
                });
        });

        ui.add_space(20.0);
    });
    let _ = Amount::ZERO;
}

// ----------------------------------------------------------- onboarding ---

pub fn onboarding(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    // The mark and the network now live in `screen_header`, which the caller
    // draws. A second copy of them here was the same fact stated twice, one
    // line apart, in two different sizes.
    app.show_onboarding(ui, ctx);
}
