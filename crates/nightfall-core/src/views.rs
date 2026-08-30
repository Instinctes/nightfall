//! The seven views.

use crate::app::{parse_amount, App, Onboarding, View, DEFAULT_FEE_DARKS};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use nightfall_crypto::Address;
use nightfall_node::SyncHold;
use nightfall_storage::now_unix;
use nightfall_types::{Amount, DARKS_PER_NIGHT, MAX_SUPPLY_NIGHT, TARGET_BLOCK_TIME_SECS};
use nightfall_wallet::Direction;

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
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "Reading the chain file from this computer (last tip {}). \
                         This is not a network sync. Peers stay closed so this node \
                         does not advertise genesis. Several minutes at this height \
                         is normal — your coins are already on disk.",
                        format_int(blocks)
                    ))
                    .size(12.5)
                    .color(TEXT_DIM),
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
                        .color(Color32::from_white_alpha(215))
                        .strong(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(night(balances.available))
                            .size(40.0)
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("NIGHT")
                            .size(16.0)
                            .color(Color32::from_white_alpha(225))
                            .strong(),
                    );
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
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if primary_button(ui, "Send", balances.available > 0).clicked() {
            app.view = View::Send;
        }
        if ghost_button(ui, "  Receive  ").clicked() {
            app.view = View::Receive;
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
    ui.columns(4, |cols| {
        let cells = [
            ("BLOCKS", format_int(blocks), TEXT),
            (
                "PEERS",
                peers.to_string(),
                if peers > 0 { SUCCESS } else { WARN },
            ),
            ("MEMPOOL", mempool.to_string(), TEXT),
            ("DIFFICULTY", format_int(difficulty), TEXT),
        ];
        for (i, (label, value, color)) in cells.into_iter().enumerate() {
            card(&mut cols[i], |ui| {
                stat(ui, label, &value, color);
            });
        }
    });

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

    ui.columns(4, |cols| {
        let cells = [
            ("LAST BLOCK", last_block, last_color),
            ("NETWORK HASH", net_label, TEXT),
            ("YOUR SHARE", share_label, share_color),
            ("NEXT UNLOCK", unlock_label, unlock_color),
        ];
        for (i, (label, value, color)) in cells.into_iter().enumerate() {
            card(&mut cols[i], |ui| {
                stat(ui, label, &value, color);
            });
        }
    });

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
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(H_MARGIN, 10.0))
        .show(ui, |ui| {
            ui.set_width(inner);
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
    ui.add_space(6.0);
}

// ------------------------------------------------------------------ send ---

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

    ui.set_max_width(660.0);

    card(ui, |ui| {
        ui.set_width(ui.available_width());

        // Recipient
        ui.label(
            RichText::new("Recipient address")
                .size(12.0)
                .color(TEXT_DIM),
        );
        ui.add_space(5.0);
        ui.add(
            egui::TextEdit::multiline(&mut app.send_to)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text("nf1…"),
        );

        // Live validation — a wrong address must never reach a signature.
        let trimmed = app.send_to.trim().to_string();
        let addr_state = if trimmed.is_empty() {
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

        if !app.address_book.entries.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Address book").size(11.0).color(TEXT_DIM));
            ui.add_space(4.0);
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

        ui.add_space(18.0);

        // Amount
        ui.horizontal(|ui| {
            ui.label(RichText::new("Amount").size(12.0).color(TEXT_DIM));
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
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text("0.00000000"),
        );

        let amount_state = parse_amount(&app.send_amount);
        ui.add_space(5.0);
        match &amount_state {
            Ok(darks) => {
                let total = darks.saturating_add(app.send_fee);
                if total > balances.available {
                    ui.label(
                        RichText::new(format!(
                            "Not enough — {} needed including fee",
                            night(total)
                        ))
                        .size(11.0)
                        .color(DANGER),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("Total debit {} NIGHT", night(total)))
                            .size(11.0)
                            .color(TEXT_FAINT),
                    );
                }
            }
            Err(e) if app.send_amount.trim().is_empty() => {
                let _ = e;
                ui.label(RichText::new(" ").size(11.0));
            }
            Err(e) => {
                ui.label(RichText::new(e).size(11.0).color(DANGER));
            }
        }

        ui.add_space(18.0);

        // Memo
        ui.label(
            RichText::new("Memo (encrypted, only the recipient can read it)")
                .size(12.0)
                .color(TEXT_DIM),
        );
        ui.add_space(5.0);
        ui.add(
            egui::TextEdit::singleline(&mut app.send_memo)
                .desired_width(f32::INFINITY)
                .char_limit(64)
                .hint_text("optional"),
        );
        ui.add_space(3.0);
        ui.label(
            RichText::new(format!("{}/64 characters", app.send_memo.len()))
                .size(10.0)
                .color(TEXT_FAINT),
        );

        ui.add_space(18.0);

        // Fee
        ui.label(RichText::new("Network fee").size(12.0).color(TEXT_DIM));
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            for (label, value) in [
                ("Economy", DEFAULT_FEE_DARKS / 10),
                ("Standard", DEFAULT_FEE_DARKS),
                ("Priority", DEFAULT_FEE_DARKS * 10),
            ] {
                let selected = app.send_fee == value;
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(format!("{label}  {}", night(value)))
                            .size(11.5)
                            .color(if selected { ACCENT_HI } else { TEXT_DIM }),
                    )
                    .fill(if selected {
                        ACCENT.gamma_multiply(0.16)
                    } else {
                        SURFACE_HI
                    })
                    .stroke(Stroke::new(1.0_f32, if selected { ACCENT } else { BORDER }))
                    .rounding(Rounding::same(ROUND_SM)),
                );
                if resp.clicked() {
                    app.send_fee = value;
                }
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("The fee is destroyed, not paid to a miner. 100% burn.")
                .size(10.5)
                .color(TEXT_FAINT),
        );

        ui.add_space(22.0);

        let ready = matches!(addr_state, Some(Ok(_)))
            && amount_state
                .as_ref()
                .map(|d| d.saturating_add(app.send_fee) <= balances.available)
                .unwrap_or(false)
            && !app.send_busy;

        if primary_button(ui, "Review payment", ready).clicked() {
            app.send_confirm = true;
        }
    });

    // --- confirmation modal ---
    if app.send_confirm {
        let amount = parse_amount(&app.send_amount).unwrap_or(0);
        let mut do_send = false;
        let mut cancel = false;

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
                kv(
                    ui,
                    "To",
                    RichText::new(short_hex(app.send_to.trim())).monospace(),
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
                    if primary_button(ui, "  Send now  ", !app.send_busy).clicked() {
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

    ui.set_max_width(660.0);

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            egui::Frame::none()
                .fill(Color32::WHITE)
                .rounding(Rounding::same(10.0))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    qr_code(ui, &address, 200.0);
                });
            ui.add_space(14.0);
            ui.label(
                RichText::new("Scan or copy to receive NIGHT")
                    .size(12.5)
                    .color(TEXT_DIM),
            );
        });

        ui.add_space(18.0);
        ui.label(RichText::new("Your address").size(12.0).color(TEXT_DIM));
        ui.add_space(5.0);
        if copyable(ui, &address, true) {
            app.toasts.success(ctx, "Address copied");
        }

        ui.add_space(14.0);
        egui::Frame::none()
            .fill(SURFACE_HI)
            .rounding(Rounding::same(ROUND_SM))
            .inner_margin(egui::Margin::same(13.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("This address is safe to reuse")
                        .size(12.5)
                        .color(SUCCESS)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Every payment creates a fresh one-time key on chain. Two payments to \
                         this address share no visible field, so nobody can link them — not even \
                         the sender.",
                    )
                    .size(11.5)
                    .color(TEXT_DIM),
                );
            });
    });
}

// -------------------------------------------------------------- activity ---

pub fn activity(app: &mut App, ui: &mut egui::Ui) {
    let entries: Vec<_> = app
        .wallet
        .lock()
        .map(|w| w.history().to_vec())
        .unwrap_or_default();

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.activity_filter)
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
    let stuck: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.direction == Direction::Sent
                && e.is_pending()
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
                "Mining rewards and incoming payments appear here automatically.",
            );
        } else {
            let now = now_unix();
            // Measured once, outside the loop. Inside it, `available_width`
            // shrinks after the first row and again when the scroll bar shows,
            // so every row got a different budget and the columns drifted.
            const RECEIPT_W: f32 = 88.0;
            let row_w = ui.available_width();
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
                                ui.ctx().copy_text(json);
                                app.toasts.success(
                                    ui.ctx(),
                                    "Receipt copied — proves this one payment, not the whole wallet",
                                );
                            }
                            None => app.toasts.error(ui.ctx(), "Could not build a receipt"),
                        }
                    }
                });
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
        ui.columns(4, |cols| {
            let cells = [
                (
                    "HASHRATE",
                    format_hashrate(app.hashrate.current),
                    if mining { ACCENT_HI } else { TEXT_DIM },
                ),
                ("THIS SESSION", format_int(blocks_found), SUCCESS),
                ("LIFETIME MINED", night_compact(lifetime), TEXT),
                ("TOTAL HASHES", format_int(hashes_total), TEXT),
            ];
            for (i, (label, value, color)) in cells.into_iter().enumerate() {
                stat(&mut cols[i], label, &value, color);
            }
        });

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
                } else {
                    "No peers — mining solo".to_string()
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
    ui.set_max_width(700.0);

    titled_card(ui, "Backup", |ui| {
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

        let seed_path = app
            .wallet
            .lock()
            .map(|w| w.seed_path.display().to_string())
            .unwrap_or_default();
        kv(
            ui,
            "Seed file",
            RichText::new(seed_path).monospace().size(11.0),
        );
        kv(
            ui,
            "Permissions",
            RichText::new("0600 — owner only").color(SUCCESS),
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
                syncing.store(true, std::sync::atomic::Ordering::SeqCst);
                std::thread::spawn(move || {
                    let result = wallet
                        .lock()
                        .map_err(|_| "wallet busy".to_string())
                        .and_then(|mut w| w.rescan(&node).map_err(|e| e.to_string()));
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
            RichText::new("Use this if a balance looks wrong. It re-reads the whole chain.")
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
                    "This moves blocks.jsonl aside and downloads the live chain. Minutes to hours.",
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
                    .desired_width(140.0)
                    .hint_text("Name"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut app.book_addr)
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
    let _ = Amount::ZERO;
}

// ----------------------------------------------------------- onboarding ---

pub fn onboarding(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.set_max_width(560.0);
    ui.vertical_centered(|ui| {
        logo(ui, 56.0);
        ui.add_space(12.0);
        ui.label(RichText::new("NIGHTFALL").size(22.0).color(TEXT).strong());
        ui.label(RichText::new("Core wallet").size(13.0).color(ACCENT_HI));
    });
    ui.add_space(22.0);

    match &app.onboarding {
        Some(Onboarding::Choice) => {
            titled_card(ui, "This computer has no wallet yet", |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "Create a new wallet, or restore the same 24 words you wrote down \
                         on a phone or in the browser. The words are BIP-39; Nightfall \
                         derives keys differently, so they do not open a Bitcoin wallet.",
                    )
                    .size(13.0)
                    .color(TEXT_DIM),
                );
                ui.add_space(16.0);
                if primary_button(ui, "  Create a new wallet  ", true).clicked() {
                    app.begin_create_wallet();
                }
                ui.add_space(8.0);
                if ghost_button(ui, "  I already have 24 words  ").clicked() {
                    app.onboarding = Some(Onboarding::Restore {
                        phrase: String::new(),
                        error: None,
                    });
                }
            });
        }
        Some(Onboarding::Create { phrase, written }) => {
            let phrase = phrase.clone();
            let written = *written;
            titled_card(ui, "Write these 24 words down", |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "This is the only backup. Anyone who sees them can spend. \
                         Paper, offline. Not a screenshot.",
                    )
                    .size(13.0)
                    .color(TEXT_DIM),
                );
                ui.add_space(12.0);
                if copyable(ui, &phrase, true) {
                    app.toasts.info(ctx, "Copied — still write them on paper");
                }
                ui.add_space(12.0);
                let mut ack = written;
                if ui
                    .checkbox(&mut ack, "I wrote these 24 words down")
                    .changed()
                {
                    if let Some(Onboarding::Create { written, .. }) = &mut app.onboarding {
                        *written = ack;
                    }
                }
                ui.add_space(12.0);
                if primary_button(ui, "  Open the wallet  ", written).clicked() && written {
                    if let Err(e) = app.finish_create_wallet(&phrase) {
                        app.toasts.error(ctx, e.to_string());
                    }
                }
            });
        }
        Some(Onboarding::Restore { phrase: _, error }) => {
            let err = error.clone();
            titled_card(ui, "Restore from 24 words", |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "Paste the phrase. Whitespace and case do not matter. \
                         A bad checksum is caught before anything is written.",
                    )
                    .size(13.0)
                    .color(TEXT_DIM),
                );
                ui.add_space(10.0);
                if let Some(Onboarding::Restore { phrase, .. }) = &mut app.onboarding {
                    ui.add(
                        egui::TextEdit::multiline(phrase)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .hint_text("word1 word2 … word24"),
                    );
                }
                if let Some(e) = err {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).size(12.0).color(DANGER));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "  Restore  ", true).clicked() {
                        let phrase = match &app.onboarding {
                            Some(Onboarding::Restore { phrase, .. }) => phrase.clone(),
                            _ => String::new(),
                        };
                        if let Err(e) = app.finish_restore_wallet(&phrase) {
                            if let Some(Onboarding::Restore { error, .. }) = &mut app.onboarding {
                                *error = Some(e.to_string());
                            }
                        }
                    }
                    if ghost_button(ui, "Back").clicked() {
                        app.onboarding = Some(Onboarding::Choice);
                    }
                });
            });
        }
        None => {}
    }
}
