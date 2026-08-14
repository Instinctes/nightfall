//! The seven views.

use crate::app::{parse_amount, App, View, DEFAULT_FEE_DARKS};
use crate::theme::*;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use nightfall_crypto::Address;
use nightfall_storage::now_unix;
use nightfall_types::{Amount, DARKS_PER_NIGHT, MAX_SUPPLY_NIGHT};
use nightfall_wallet::Direction;

fn night(darks: u64) -> String {
    let whole = darks / DARKS_PER_NIGHT;
    let frac = darks % DARKS_PER_NIGHT;
    format!("{}.{:08}", format_int(whole), frac)
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
                s.peers,
                s.mempool,
                s.difficulty,
                s.supply_ok,
                s.minted,
                s.burned_fees,
            )
        })
        .unwrap_or((0, 0, 0, 0, false, 0, 0));

    let behind = app.status.as_ref().map(|s| s.blocks_behind).unwrap_or(0);

    // Mining is held back while the chain is behind, because a block built on a
    // tip the network has already left cannot win — it only deepens a fork.
    // Say that plainly, or the wallet looks broken: the button says "Stop
    // mining" and the hashrate reads zero.
    if app.is_mining() && behind > 0 {
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
                        RichText::new(format!(
                            "Catching up — {} block{} behind",
                            format_int(behind),
                            if behind == 1 { "" } else { "s" }
                        ))
                        .size(14.0)
                        .color(ACCENT)
                        .strong(),
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Mining starts by itself once this reaches zero. A block built on \
                         an outdated tip cannot be accepted by anyone — it would only \
                         split the chain.",
                    )
                    .size(12.5)
                    .color(TEXT_DIM),
                );
            });
        ui.add_space(14.0);
    }

    // Mining with no peers is how you end up on a private fork without
    // noticing. Say so loudly, before hours of work get discarded.
    if app.is_mining() && peers == 0 {
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

        let (colour, pulse, text) = if syncing {
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
                    RichText::new(if supply_ok {
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

fn activity_row(ui: &mut egui::Ui, e: &nightfall_wallet::HistoryEntry, now: u64) {
    // Only glyphs that exist in egui's bundled font — a missing one renders
    // as a tofu box, which is what made the first build look broken.
    let (icon, color, sign) = match e.direction {
        Direction::Received => ("+", SUCCESS, "+"),
        Direction::Mined => ("*", ACCENT_HI, "+"),
        Direction::Sent => ("-", TEXT, "-"),
    };

    egui::Frame::none()
        .fill(SURFACE_HI)
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(15.0).color(color));
                ui.add_space(6.0);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(e.direction.label()).size(13.0).strong());
                        if e.is_pending() {
                            badge(ui, "pending", WARN);
                        }
                    });
                    let sub = if e.memo.is_empty() {
                        match e.height {
                            Some(h) => format!("block {h} · {}", ago(e.timestamp, now)),
                            None => "waiting for a block".to_string(),
                        }
                    } else {
                        e.memo.clone()
                    };
                    ui.label(RichText::new(sub).size(11.0).color(TEXT_FAINT));
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{sign}{}", night(e.amount)))
                                    .size(13.5)
                                    .color(color)
                                    .monospace(),
                            );
                        });
                        if e.fee > 0 {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("fee {}", night(e.fee)))
                                            .size(10.0)
                                            .color(TEXT_FAINT),
                                    );
                                },
                            );
                        }
                    });
                });
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
            for e in filtered {
                activity_row(ui, e, now);
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
        ui.columns(3, |cols| {
            let cells = [
                (
                    "HASHRATE",
                    format_hashrate(app.hashrate.current),
                    if mining { ACCENT_HI } else { TEXT_DIM },
                ),
                ("BLOCKS FOUND", format_int(blocks_found), SUCCESS),
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

fn reward_at(height: u64) -> u64 {
    let halvings = height / nightfall_types::HALVING_INTERVAL_BLOCKS;
    if halvings >= 64 {
        return 0;
    }
    (nightfall_types::INITIAL_BLOCK_REWARD_NIGHT * DARKS_PER_NIGHT) >> halvings
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
    let s = app.status.as_ref();

    card(ui, |ui| {
        ui.set_width(ui.available_width());
        let peers = s.map(|s| s.peers).unwrap_or(0);
        ui.horizontal(|ui| {
            dot(ui, if peers > 0 { SUCCESS } else { WARN }, false);
            ui.add_space(6.0);
            ui.label(
                RichText::new(if peers > 0 {
                    format!("Connected to {peers} peer(s)")
                } else {
                    "No peers — mining solo".to_string()
                })
                .size(16.0)
                .strong(),
            );
        });
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
                "Others reach you on port {}. Forward it in your router to accept \
                 incoming connections.",
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
            "Blocks",
            RichText::new(format_int(s.map(|s| s.blocks).unwrap_or(0))).monospace(),
        );
        kv(
            ui,
            "Tip",
            RichText::new(short_hex(&s.map(|s| s.tip.clone()).unwrap_or_default())).monospace(),
        );
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
                "Your seed is the only thing that can recover this wallet. Anyone who reads it \
                 owns your coins. Write it down offline — never in a screenshot, chat or cloud note.",
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
        if app.reveal_seed {
            let seed = app.wallet.lock().map(|w| w.seed_hex()).unwrap_or_default();
            if copyable(ui, &seed, true) {
                app.toasts.info(ctx, "Seed copied — handle with care");
            }
            ui.add_space(6.0);
            if ghost_button(ui, "Hide seed").clicked() {
                app.reveal_seed = false;
            }
        } else if ghost_button(ui, "Reveal recovery seed").clicked() {
            app.reveal_seed = true;
        }
    });

    ui.add_space(14.0);

    titled_card(ui, "View key", |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(
                "A view key lets someone see every amount and memo you send or receive — an \
                 accountant or auditor, for example. It cannot spend anything.",
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
            RichText::new("not yet — kernel aggregation pending").color(WARN),
        );

        ui.add_space(12.0);
        egui::Frame::none()
            .fill(WARN.gamma_multiply(0.10))
            .rounding(Rounding::same(ROUND_SM))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Pre-launch software, not independently audited.")
                        .size(12.0)
                        .color(WARN)
                        .strong(),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new(
                        "The transaction graph is still linkable and proof of work is not yet \
                         memory-hard. Do not treat NIGHT as money you can afford to lose.",
                    )
                    .size(11.0)
                    .color(TEXT_DIM),
                );
            });
    });

    ui.add_space(20.0);
    let _ = Amount::ZERO;
}
