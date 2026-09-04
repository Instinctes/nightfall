//! The swap tab.
//!
//! Drawing only. Every decision this view appears to make — whether the
//! network may start a swap, how much of the cancel window is left, which
//! buttons a state offers, whether a typed amount is usable — is made in
//! `nightfall_swap::ui` and tested there. If you are looking for a rule,
//! it is not in this file.

use crate::app::App;
use crate::theme::*;
use crate::widgets::*;
use crate::widgets_swap::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke};
use nightfall_swap::ui as logic;

fn night(darks: u64) -> String {
    format!(
        "{}.{:08}",
        format_int(darks / 100_000_000),
        darks % 100_000_000
    )
}

fn urgency_colour(u: logic::Urgency) -> Color32 {
    match u {
        logic::Urgency::Calm => SUCCESS,
        logic::Urgency::Soon => ACCENT_HI,
        logic::Urgency::Act => WARN,
        logic::Urgency::Passed => DANGER,
    }
}

pub fn swap(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    // Centred instead of pinned to the left edge. On a wide window this page
    // was a column of cards hugging one side with the rest of the screen
    // empty, which read as something failing to load.
    crate::widgets::narrow_column(ui, 860.0, |ui| {
        header(app, ui);
        ui.add_space(14.0);

        let gate = logic::availability(app.network);
        if let logic::Availability::Locked { headline, detail } = &gate {
            locked_notice(ui, headline, detail);
            ui.add_space(14.0);
        }

        warnings(ui);
        ui.add_space(14.0);

        if gate.is_enabled() {
            start_form(app, ui, ctx);
            ui.add_space(14.0);
        }

        packets(app, ui, ctx);
        ui.add_space(14.0);

        swap_list(app, ui, ctx);
    });
}

// ------------------------------------------------------------------ header ---

fn header(app: &App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("NIGHT ↔ BTC").size(24.0).color(TEXT).strong());
        ui.add_space(10.0);
        pill(ui, "Atomic swap", ACCENT_HI);
        ui.add_space(6.0);
        pill(ui, app.network.as_str(), TEXT_DIM);
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Trade Bitcoin for NIGHT directly with another person. No exchange, \
             no custodian, no account.",
        )
        .size(12.5)
        .color(TEXT_DIM),
    );
}

fn locked_notice(ui: &mut egui::Ui, headline: &str, detail: &str) {
    egui::Frame::none()
        .fill(DANGER.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0_f32, DANGER.gamma_multiply(0.45)))
        .rounding(Rounding::same(ROUND))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 32.0);
            ui.horizontal(|ui| {
                dot(ui, DANGER, false);
                ui.add_space(4.0);
                ui.label(RichText::new(headline).size(13.5).color(DANGER).strong());
            });
            ui.add_space(6.0);
            ui.label(RichText::new(detail).size(12.0).color(TEXT));
        });
}

fn warnings(ui: &mut egui::Ui) {
    egui::Frame::none()
        .fill(WARN.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0_f32, WARN.gamma_multiply(0.45)))
        .rounding(Rounding::same(ROUND))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 32.0);
            kicker(ui, "Read this before you lock anything");
            ui.add_space(8.0);
            ui.label(
                RichText::new(nightfall_swap::warnings::NOT_PRIVATE)
                    .size(12.5)
                    .color(WARN)
                    .strong(),
            );
            ui.add_space(7.0);
            ui.label(
                RichText::new(nightfall_swap::warnings::NO_NIGHT_REFUND)
                    .size(12.0)
                    .color(TEXT),
            );
        });
}

// -------------------------------------------------------------- start form ---

fn start_form(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let available = app.unreserved_darks();

    titled_card(ui, "Start a swap", |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            let give = app.swap_draft.give_night;
            if ui
                .selectable_label(give, RichText::new("  I give NIGHT  ").size(12.5))
                .clicked()
            {
                app.swap_draft.give_night = true;
            }
            if ui
                .selectable_label(!give, RichText::new("  I give Bitcoin  ").size(12.5))
                .clicked()
            {
                app.swap_draft.give_night = false;
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(if app.swap_draft.give_night {
                "You lock NIGHT and receive Bitcoin. If the other side vanishes \
                 after the cancel, your NIGHT is stuck — there is no NIGHT refund."
            } else {
                "You lock Bitcoin and receive NIGHT. Your Bitcoin can always be \
                 cancelled and refunded."
            })
            .size(11.5)
            .color(TEXT_FAINT),
        );

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("NIGHT").size(11.5).color(TEXT_DIM));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_draft.night)
                        .margin(FIELD_MARGIN)
                        .desired_width(180.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("0.00000000"),
                );
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Bitcoin (sat)").size(11.5).color(TEXT_DIM));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_draft.btc)
                        .margin(FIELD_MARGIN)
                        .desired_width(150.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("200000"),
                );
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("BTC fee (sat)").size(11.5).color(TEXT_DIM));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_draft.btc_fee)
                        .margin(FIELD_MARGIN)
                        .desired_width(110.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("auto"),
                );
            });
        });

        if app.swap_draft.give_night {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("Available and unreserved: {}", night(available)))
                    .size(11.0)
                    .color(TEXT_FAINT),
            );
        }

        ui.add_space(16.0);
        kicker(ui, "Your Bitcoin addresses");
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "This wallet holds NIGHT, not Bitcoin. These come from your own \
                 Bitcoin wallet — an address you do not hold the key to is a burn.",
            )
            .size(11.0)
            .color(TEXT_FAINT),
        );
        ui.add_space(8.0);
        for (label, field, hint) in [
            (
                "Refund to",
                &mut app.swap_btc_refund,
                "where your Bitcoin returns if the swap is cancelled",
            ),
            (
                "Redeem to",
                &mut app.swap_btc_redeem,
                "where the Bitcoin you bought arrives",
            ),
            (
                "Punish to",
                &mut app.swap_btc_punish,
                "where the Bitcoin goes if the other side stalls after a cancel",
            ),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [90.0, 20.0],
                    egui::Label::new(RichText::new(label).size(11.5).color(TEXT_DIM))
                        .selectable(false),
                );
                ui.add(
                    egui::TextEdit::singleline(field)
                        .margin(FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint),
                );
            });
            ui.add_space(5.0);
        }

        ui.add_space(12.0);
        let checked = logic::validate(&app.swap_draft, available);
        match &checked {
            Ok(a) => {
                trade_card(
                    ui,
                    &night(a.night_darks),
                    if app.swap_draft.give_night {
                        "NIGHT you lock"
                    } else {
                        "NIGHT you receive"
                    },
                    &format_int(a.btc_sats),
                    if app.swap_draft.give_night {
                        "sat you receive"
                    } else {
                        "sat you lock"
                    },
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!(
                        "Bitcoin fee {} sat, taken from the locked amount.",
                        format_int(a.btc_fee_sats)
                    ))
                    .size(11.0)
                    .color(TEXT_FAINT),
                );
            }
            Err(e) => {
                ui.horizontal(|ui| {
                    dot(ui, DANGER, false);
                    ui.add_space(4.0);
                    ui.label(RichText::new(e.to_string()).size(11.5).color(DANGER));
                });
            }
        }

        ui.add_space(14.0);
        if primary_button(ui, "Create swap", checked.is_ok()).clicked() {
            if let Ok(amounts) = checked {
                match app.create_swap(logic::role_of(&app.swap_draft.clone()), amounts) {
                    Ok(id) => {
                        app.swap_start_error = None;
                        app.toasts
                            .success(ctx, format!("Swap {id} created. Nothing is locked yet."));
                        app.swap_draft.night.clear();
                        app.swap_draft.btc.clear();
                    }
                    Err(e) => app.swap_start_error = Some(e),
                }
            }
        }
        if let Some(e) = &app.swap_start_error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).size(11.5).color(DANGER));
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Creating a swap reserves the coins so you cannot spend them twice. \
                 Nothing is broadcast until both sides have exchanged packets.",
            )
            .size(11.0)
            .color(TEXT_FAINT),
        );
    });
}

// ----------------------------------------------------------------- packets ---

fn packets(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    titled_card(ui, "Packets", |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(
                "There is no server. You and the other side pass these blocks of \
                 text by whatever channel you already trust.",
            )
            .size(11.5)
            .color(TEXT_FAINT),
        );

        ui.add_space(12.0);
        kicker(ui, "Paste what they sent you");
        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::multiline(&mut app.swap_packet_in)
                .margin(FIELD_MARGIN)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .hint_text("{\"version\":1,…}"),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ghost_button(ui, "  Check and import  ").clicked() {
                match app.import_packet() {
                    Ok(msg) => {
                        app.swap_import_error = None;
                        app.swap_packet_in.clear();
                        app.toasts.success(ctx, msg);
                    }
                    Err(e) => app.swap_import_error = Some(e),
                }
            }
            if ghost_button(ui, "  Clear  ").clicked() {
                app.swap_packet_in.clear();
                app.swap_import_error = None;
            }
        });
        if let Some(e) = &app.swap_import_error {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                dot(ui, DANGER, false);
                ui.add_space(4.0);
                ui.label(RichText::new(e).size(11.5).color(DANGER));
            });
        }

        if !app.swap_packet_out.is_empty() {
            ui.add_space(16.0);
            kicker(ui, "Send them this");
            ui.add_space(6.0);
            let text = app.swap_packet_out.clone();
            if copyable(ui, &text, true) {
                ctx.copy_text(text);
                app.toasts.success(ctx, "Packet copied.");
            }
        }
    });
}

// -------------------------------------------------------------- swap cards ---

fn swap_list(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let list = match nightfall_swap::persist::list(&app.datadir) {
        Ok(l) => l,
        Err(e) => {
            titled_card(ui, "Your swaps", |ui| {
                ui.label(RichText::new(e.to_string()).color(DANGER));
            });
            return;
        }
    };

    if list.is_empty() {
        titled_card(ui, "Your swaps", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("None yet. This wallet has not locked any coins for a swap.")
                    .size(12.0)
                    .color(TEXT_FAINT),
            );
        });
        return;
    }

    for stored in list {
        swap_card(app, ui, ctx, &stored);
        ui.add_space(14.0);
    }
}

fn swap_card(
    app: &mut App,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    stored: &nightfall_swap::StoredSwap,
) {
    let state = &stored.state;
    let id = state.id().to_string();
    let tl = logic::timeline(state);
    let depths = app.swap_depths();

    card(ui, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(short_hex(&id))
                    .monospace()
                    .size(11.5)
                    .color(TEXT_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, colour) = if logic::is_finished(state) {
                    ("Settled", SUCCESS)
                } else if tl.track == logic::Track::Abort {
                    ("Unwinding", WARN)
                } else {
                    ("Running", ACCENT_HI)
                };
                pill(ui, text, colour);
            });
        });

        ui.add_space(12.0);
        timeline(
            ui,
            tl.steps,
            tl.current,
            tl.settled,
            if tl.track == logic::Track::Abort {
                TimelineStyle::abort()
            } else {
                TimelineStyle::progress()
            },
        );

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            stat(ui, "NIGHT", &night(stored.night_darks), TEXT);
            ui.add_space(28.0);
            stat(
                ui,
                "Bitcoin",
                &format!("{} sat", format_int(stored.btc_sats)),
                TEXT,
            );
        });

        ui.add_space(10.0);
        ui.label(
            RichText::new(next_action_text(state))
                .size(12.5)
                .color(TEXT),
        );

        // Deadlines. `None` means the node could not be asked — never zero.
        let deadlines = logic::deadlines(
            state,
            depths,
            app.btc_lock_confirms(stored),
            app.night_lock_confirms(stored),
        );
        if !deadlines.is_empty() {
            ui.add_space(14.0);
            for d in deadlines {
                deadline_bar(
                    ui,
                    d.label,
                    &d.detail,
                    d.fraction,
                    urgency_colour(d.urgency),
                );
                ui.add_space(10.0);
            }
        }

        // Bob's side of the Bitcoin lock: fund it, hand it over for signing,
        // check what comes back.
        if state.role() == nightfall_swap::Role::Bob && !logic::is_finished(state) {
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(10.0);
            lock_section(app, ui, ctx, stored);
        }

        let actions = logic::actions(state);
        if !actions.is_empty() {
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(10.0);
            action_row(app, ui, ctx, &id, &actions, stored);
        }
    });
}

fn action_row(
    app: &mut App,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: &str,
    actions: &[logic::Action],
    stored: &nightfall_swap::StoredSwap,
) {
    // A destructive button asks first, and the question names the cost.
    if let Some((pending, act)) = app.swap_confirm.clone() {
        if pending == id {
            ui.label(RichText::new(act.consequence()).size(11.5).color(WARN));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ghost_button(ui, "  Yes, do it  ").clicked() {
                    let msg = app.run_swap_action(id, &act);
                    app.swap_confirm = None;
                    app.toasts.success(ctx, msg);
                }
                if ghost_button(ui, "  Keep waiting  ").clicked() {
                    app.swap_confirm = None;
                }
            });
            return;
        }
    }

    ui.horizontal_wrapped(|ui| {
        for a in actions {
            let resp = ghost_button(ui, &format!("  {}  ", a.label()));
            let resp = resp.on_hover_text(a.consequence());
            if resp.clicked() {
                match a {
                    logic::Action::ExportPacket => match app.export_packet(stored) {
                        Ok(text) => {
                            ctx.copy_text(text.clone());
                            app.swap_packet_out = text;
                            app.toasts
                                .success(ctx, "Packet copied. Send it to the other side.");
                        }
                        Err(e) => app.toasts.error(ctx, e),
                    },
                    logic::Action::ImportPacket => {
                        app.toasts.info(ctx, "Paste it into the Packets box above.");
                    }
                    logic::Action::CancelNow | logic::Action::Recover => {
                        app.swap_confirm = Some((id.to_string(), *a));
                    }
                    logic::Action::Forget => {
                        let msg = app.run_swap_action(id, a);
                        app.toasts.success(ctx, msg);
                    }
                    logic::Action::SendRefund | logic::Action::SendPunish => {
                        app.swap_confirm = Some((id.to_string(), *a));
                    }
                }
            }
        }
    });
}

fn next_action_text(state: &nightfall_swap::SwapState) -> String {
    use nightfall_swap::{Role, SwapState};
    match state {
        SwapState::Setup { .. } => "Waiting for the counterparty packet. Nothing is locked.".into(),
        SwapState::BtcLocked { .. } => {
            "Bitcoin is locked. Waiting for the NIGHT lock and its confirmations.".into()
        }
        SwapState::NightLocked { .. } => {
            "Both sides are locked. Waiting for depth before the redeem.".into()
        }
        SwapState::ReadyToRedeem { role, .. } if *role == Role::Alice => {
            "Cleared. The Bitcoin redeem goes out on the next tick, unless H1 is too close.".into()
        }
        SwapState::ReadyToRedeem { .. } => {
            "Cleared. Waiting for Alice to redeem the Bitcoin.".into()
        }
        SwapState::Redeeming { role, .. } if *role == Role::Bob => {
            "s_a is public. Claiming the NIGHT.".into()
        }
        SwapState::Redeeming { .. } => {
            "Redeem broadcast. Waiting for the other side to claim the NIGHT.".into()
        }
        SwapState::Done { .. } => "Finished. Both sides have what they came for.".into(),
        SwapState::MustCancel { .. } => {
            "Aborting. TX_cancel goes out; do not redeem from here.".into()
        }
        SwapState::Cancelled { role, .. } if *role == Role::Bob => {
            "Cancel confirmed. Refund now — waiting lets the other side punish.".into()
        }
        SwapState::Cancelled { .. } => {
            "Cancel confirmed. If no refund appears, punish opens after H2.".into()
        }
        SwapState::Refunded { .. } => {
            "Bitcoin refunded. If NIGHT was locked, s_b is public.".into()
        }
        SwapState::Punished { .. } => {
            "Punished. Bitcoin taken. Locked NIGHT stays locked — that is the wart.".into()
        }
        SwapState::Failed { .. } => "Stuck. This one needs a human.".into(),
    }
}

// ------------------------------------------------------- the Bitcoin lock ---

/// Fund, export, sign elsewhere, check what comes back.
///
/// This wallet holds no Bitcoin keys, so the middle step happens in the
/// user's own Bitcoin wallet. The two ends are ours: we build the exact
/// transaction, and we refuse anything that is not it.
fn lock_section(
    app: &mut App,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    stored: &nightfall_swap::StoredSwap,
) {
    let export = app.lock_export(stored).ok();

    match &export {
        None => {
            kicker(ui, "Fund the Bitcoin lock");
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Name an output you already control. Nothing is broadcast — \
                     this only builds the transaction.",
                )
                .size(11.0)
                .color(TEXT_FAINT),
            );
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.add_sized(
                    [95.0, 20.0],
                    egui::Label::new(RichText::new("Transaction").size(11.5).color(TEXT_DIM))
                        .selectable(false),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_funding.txid)
                        .margin(FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("64 hex characters"),
                );
            });
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [95.0, 20.0],
                    egui::Label::new(RichText::new("Output no.").size(11.5).color(TEXT_DIM))
                        .selectable(false),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_funding.vout)
                        .margin(FIELD_MARGIN)
                        .desired_width(70.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("0"),
                );
                ui.add_space(14.0);
                ui.label(RichText::new("holds (sat)").size(11.5).color(TEXT_DIM));
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_funding.value)
                        .margin(FIELD_MARGIN)
                        .desired_width(120.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("500000"),
                );
            });
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [95.0, 20.0],
                    egui::Label::new(RichText::new("Change to").size(11.5).color(TEXT_DIM))
                        .selectable(false),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.swap_funding.change_address)
                        .margin(FIELD_MARGIN)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("leave empty only if there is no change"),
                );
            });

            ui.add_space(8.0);
            // The same amounts the build will use. Reconstructing them from
            // `stored` would mean a zero fee here and the real one there.
            match app.swap_amounts(stored) {
                None => {
                    ui.label(
                        RichText::new("No open session for this swap.")
                            .size(11.5)
                            .color(DANGER),
                    );
                }
                Some(amounts) => match logic::validate_funding(&app.swap_funding, &amounts) {
                    Ok(f) => {
                        let change = logic::change_after_lock(&f, &amounts);
                        ui.label(
                            RichText::new(match change {
                                Some(c) => format!("Change back to you: {} sat.", format_int(c)),
                                None => "No change — the remainder is the fee.".into(),
                            })
                            .size(11.0)
                            .color(TEXT_FAINT),
                        );
                    }
                    Err(e) => {
                        ui.horizontal(|ui| {
                            dot(ui, DANGER, false);
                            ui.add_space(4.0);
                            ui.label(RichText::new(e.to_string()).size(11.5).color(DANGER));
                        });
                    }
                },
            }

            ui.add_space(10.0);
            if ghost_button(ui, "  Build the lock  ").clicked() {
                app.swap_lock_note = Some(match app.build_lock(stored) {
                    Ok(packet) => {
                        app.swap_packet_out = packet;
                        Ok("Lock built. Send the packet, then sign the transaction.".into())
                    }
                    Err(e) => Err(e),
                });
            }
        }

        Some(ex) => {
            kicker(ui, "Sign this in your Bitcoin wallet");
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Transaction id · {}", short_hex(&ex.txid)))
                    .size(11.0)
                    .color(TEXT_FAINT)
                    .monospace(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ghost_button(ui, "  Copy raw hex  ").clicked() {
                    ctx.copy_text(ex.raw_hex.clone());
                    app.toasts.success(ctx, "Unsigned transaction copied.");
                }
                if ghost_button(ui, "  Copy PSBT  ").clicked() {
                    ctx.copy_text(ex.psbt.clone());
                    app.toasts.success(ctx, "PSBT copied.");
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Raw hex works with `signrawtransactionwithwallet`. The PSBT \
                     carries no prevout, so some wallets will want the hex.",
                )
                .size(11.0)
                .color(TEXT_FAINT),
            );

            ui.add_space(14.0);
            kicker(ui, "Then paste the signed transaction back");
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut app.swap_signed_hex)
                    .margin(FIELD_MARGIN)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("0200000000010…"),
            );
            ui.add_space(8.0);
            if ghost_button(ui, "  Check it is the right one  ").clicked() {
                app.swap_lock_note = Some(app.confirm_lock(stored));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Checked before you broadcast, because the other side has \
                     already signed transactions that name this exact id.",
                )
                .size(11.0)
                .color(TEXT_FAINT),
            );
        }
    }

    if let Some(note) = &app.swap_lock_note {
        ui.add_space(10.0);
        match note {
            Ok(msg) => {
                ui.horizontal(|ui| {
                    dot(ui, SUCCESS, false);
                    ui.add_space(4.0);
                    ui.label(RichText::new(msg).size(11.5).color(SUCCESS));
                });
            }
            Err(msg) => {
                ui.horizontal_wrapped(|ui| {
                    dot(ui, DANGER, false);
                    ui.add_space(4.0);
                    ui.label(RichText::new(msg).size(11.5).color(DANGER));
                });
            }
        }
    }
}
