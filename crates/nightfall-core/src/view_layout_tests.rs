use crate::{
    app::{App, View},
    views::*,
};
use eframe::egui::{self, Vec2};

#[test]
fn settings_shortcuts_override_the_previous_section() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let dir = std::env::temp_dir().join(format!("nf-settings-links-{}", std::process::id()));
    let mut app = App::new(nightfall_types::NetworkId::Devnet, dir);
    for section in [3, 1] {
        select_workspace_for_test(&ctx, "settings", 4);
        open_settings_section(&mut app, &ctx, section);
        assert!(app.view == View::Settings);
        assert_eq!(
            ctx.data(|data| data.get_temp::<usize>(egui::Id::new(("page-workspace", "settings")))),
            Some(section)
        );
        assert!(!app.reveal_mnemonic);
    }
    assert!(app.node.is_none());
}

/// The withheld-payment notice is the longest block of prose on the Activity
/// page, and the page test above never renders it: that App has no wallet, so
/// it has no history. Render it directly against a public fixture instead.
#[test]
fn the_withheld_payment_notice_fits_supported_content_widths() {
    use nightfall_wallet::{Direction, HistoryEntry};
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let entry = |quarantined| HistoryEntry {
        direction: Direction::Sent,
        amount: 1_000,
        fee: 10,
        memo: "public fixture".into(),
        height: None,
        txid: "cd".repeat(32),
        timestamp: 1_000,
        spent_commits: Vec::new(),
        raw: None,
        quarantined,
    };
    // One withheld and one ordinary pending send: the notice must count only
    // the first, or an owner would be told to check a payment they just made.
    let entries = vec![entry(true), entry(false)];
    for width in [620.0, 884.0, 1180.0] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let right = ui.max_rect().right();
                assert!(!withheld_notice(ui, &entries, 9_000_000));
                assert!(
                    ui.min_rect().right() <= right + 1.0,
                    "withheld notice overflow at {width}px: {} > {right}",
                    ui.min_rect().right()
                );
                // Nothing withheld means no notice and no vertical space taken.
                let before = ui.min_rect().bottom();
                assert!(!withheld_notice(ui, &[entry(false)], 9_000_000));
                assert_eq!(ui.min_rect().bottom(), before);
            });
        });
    }
}

#[test]
fn all_eight_pages_fit_supported_content_widths() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let dir = std::env::temp_dir().join(format!("nf-ui-layout-{}", nightfall_storage::now_unix()));
    let mut app = App::new(nightfall_types::NetworkId::Devnet, dir);
    // No seed exists: construction starts no node and touches no user wallet.
    for width in [620.0, 884.0, 1180.0] {
        for (view, name) in View::ALL {
            app.view = view;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    page_intro(view, ui);
                    match view {
                        View::Dashboard => dashboard(&mut app, ui),
                        View::Send => send(&mut app, ui, ctx),
                        View::Receive => receive(&mut app, ui, ctx),
                        View::Activity => activity(&mut app, ui),
                        View::Mining => mining(&mut app, ui),
                        View::Network => network(&mut app, ui, ctx),
                        View::Settings => settings(&mut app, ui, ctx),
                    }
                    assert!(
                        ui.min_rect().right() <= right + 1.0,
                        "{name} overflow at {width}px: {} > {right}",
                        ui.min_rect().right()
                    );
                });
            });
        }
    }
}

/// The new task selectors must not hide half the wallet from layout coverage.
/// Exercise both sides with the shipped fallback font and the native Mac font.
#[test]
fn all_workspaces_fit_with_both_font_sets() {
    for native in [false, true] {
        let ctx = egui::Context::default();
        if native {
            crate::theme::install_platform_fonts(&ctx);
        }
        crate::theme::apply(&ctx);
        let dir =
            std::env::temp_dir().join(format!("nf-subview-layout-{}-{native}", std::process::id()));
        let mut app = App::new(nightfall_types::NetworkId::Devnet, dir);
        for width in [620.0, 884.0, 1180.0] {
            for (view, key, count) in [
                (View::Send, "send", 2),
                (View::Receive, "receive", 2),
                (View::Activity, "activity", 2),
                (View::Settings, "settings", 5),
            ] {
                for selected in 0..count {
                    select_workspace_for_test(&ctx, key, selected);
                    for _ in 0..2 {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(width, 1600.0),
                            )),
                            ..Default::default()
                        };
                        let _ = ctx.run(input, |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                let right = ui.max_rect().right();
                                match view {
                                    View::Send => send(&mut app, ui, ctx),
                                    View::Receive => receive(&mut app, ui, ctx),
                                    View::Activity => activity(&mut app, ui),
                                    View::Settings => settings(&mut app, ui, ctx),
                                    _ => unreachable!(),
                                }
                                assert!(ui.min_rect().right() <= right + 1.0,
                                    "{key}/{selected} overflow with native={native} at {width}px: {} > {right}", ui.min_rect().right());
                            });
                        });
                    }
                }
            }
        }
        assert!(app.node.is_none(), "layout tests never start a node");
    }
}

/// The pasted-payment-request card, which the page test above never reaches:
/// it renders only when the address field holds a request, and that App has no
/// wallet and no typing. Rendered directly here instead.
#[test]
fn the_payment_request_card_fits_supported_content_widths() {
    use nightfall_crypto::WalletKeys;
    use nightfall_types::NetworkId;
    use nightfall_wallet::payment_request::PaymentRequest;

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let address = WalletKeys::from_seed([5; 32]).address();
    let full = PaymentRequest {
        address,
        network: NetworkId::Devnet,
        amount_darks: Some(150_000_000),
        memo: "x".repeat(nightfall_wallet::payment_request::MAX_MEMO),
        invoice: "y".repeat(nightfall_wallet::payment_request::MAX_INVOICE),
        expires_unix: Some(1),
    }
    .to_uri();
    // A request for another network, so the refusal text is measured too — it
    // is the longest sentence this card can show.
    let cases = [
        full.as_str(),
        "nightfall:not-an-address?network=devnet",
        "nightfall:",
    ];

    for width in [620.0, 884.0, 1180.0] {
        for case in cases {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    // Mainnet against a devnet request: shows, and refuses.
                    assert!(payment_request_card(ui, case, NetworkId::Mainnet).is_none());
                    assert!(
                        ui.min_rect().right() <= right + 1.0,
                        "payment request card overflow at {width}px for {case:?}"
                    );
                });
            });
        }
    }

    // An ordinary address is not a request and must render nothing at all.
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let before = ui.min_rect().bottom();
            assert!(payment_request_card(ui, &address.encode(), NetworkId::Mainnet).is_none());
            assert_eq!(ui.min_rect().bottom(), before);
        });
    });
}

/// A long value in a data row stays on its own side of the label.
///
/// From a screenshot of Settings: "Config" and the path beginning
/// "/Users/hux/…" were painted in the same place, one over the other, and the
/// result read as corrupted glyphs. The value was drawn in a right-to-left
/// layout with no width of its own, so anything longer than the free space
/// grew leftwards across the label.
#[test]
fn a_long_value_never_climbs_over_its_label() {
    use crate::widgets::{card, kv};
    use eframe::egui::RichText;

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let long = "/Users/hux/Library/Application Support/nightfall/devnet/n8/wallet-1.0-dev/bitcoin-rpc.conf";

    for width in [380.0, 620.0, 884.0, 1180.0] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let right = ui.max_rect().right();
                let drawn = ui
                    .scope(|ui| {
                        card(ui, |ui| {
                            kv(ui, "Config", RichText::new(long).monospace().size(11.0));
                            kv(ui, "Short", RichText::new("0").monospace());
                        });
                    })
                    .response
                    .rect;
                assert!(
                    drawn.right() <= right + 1.0,
                    "a long value pushed the card past the panel at {width}px: {} > {right}",
                    drawn.right(),
                );
                assert!(
                    drawn.left() >= -1.0,
                    "a long value grew leftwards out of the card at {width}px: {}",
                    drawn.left(),
                );
            });
        });
    }
}

/// A handful of history entries, one of each shape.
fn rows_fixture() -> Vec<nightfall_wallet::HistoryEntry> {
    use nightfall_wallet::{Direction, HistoryEntry};
    let row = |direction, fee, memo: &str, height| HistoryEntry {
        direction,
        amount: 150_000_000,
        fee,
        memo: memo.into(),
        height,
        txid: "ab".repeat(32),
        timestamp: 1_000,
        spent_commits: Vec::new(),
        raw: None,
        quarantined: false,
    };
    vec![
        row(Direction::Received, 0, "A-17", Some(1_280)),
        row(Direction::Received, 0, "", Some(1_281)),
        row(Direction::Mined, 0, "", Some(1_278)),
        row(Direction::Sent, 100_000, "Rent — March", Some(1_282)),
        row(Direction::Sent, 100_000, "", None),
    ]
}

/// Two columns do not overlap, and they leave a gap.
///
/// From a screenshot of the Dashboard: the right edge of "Recent activity"
/// sat to the *right* of the left edge of "Network supply" — the two cards
/// were drawn on top of each other. Measured here rather than looked at,
/// because a card border crossing another card is easy to read as a divider.
#[test]
fn two_columns_do_not_overlap() {
    use crate::widgets::{card, two_columns};

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    for width in [620.0, 884.0, 1180.0, 1600.0] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let panel = ui.max_rect();
                let mut left_rect = egui::Rect::NOTHING;
                let mut right_rect = egui::Rect::NOTHING;
                two_columns(
                    ui,
                    320.0,
                    |ui| {
                        left_rect = ui
                            .scope(|ui| {
                                card(ui, |ui| {
                                    // The real content, not a label. A card
                                    // holding a label cannot push its column
                                    // wider; a card holding rows that size
                                    // themselves from `available_width` can,
                                    // and on the Dashboard it did — the
                                    // activity list ran 72 points under the
                                    // card beside it.
                                    for entry in &rows_fixture() {
                                        activity_row_for_test(ui, entry, 9_000);
                                    }
                                });
                            })
                            .response
                            .rect;
                    },
                    |ui| {
                        right_rect = ui
                            .scope(|ui| {
                                card(ui, |ui| {
                                    ui.label("right");
                                });
                            })
                            .response
                            .rect;
                    },
                );
                // Folded to one column on a narrow window: then they stack,
                // and "no overlap" is about vertical order instead.
                if right_rect.top() >= left_rect.bottom() - 1.0 {
                    return;
                }
                assert!(
                    left_rect.right() <= right_rect.left() + 0.5,
                    "columns overlap at {width}px: left ends at {}, right starts at {}",
                    left_rect.right(),
                    right_rect.left(),
                );
                assert!(
                    right_rect.right() <= panel.right() + 1.0,
                    "right column runs past the panel at {width}px: {} > {}",
                    right_rect.right(),
                    panel.right(),
                );
            });
        });
    }
}

/// Every row in a list is the same width, whichever direction it is.
///
/// From a screenshot of the Dashboard: the received, mined and sent rows
/// looked like three different widths inside one card. Three tints and three
/// stroke colours make an edge easier or harder to see, so the eye is not a
/// reliable instrument here — this measures instead.
#[test]
fn activity_rows_in_one_list_share_one_width() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let entries = rows_fixture();

    for width in [620.0, 884.0, 1180.0] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 1400.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut widths = Vec::new();
                crate::widgets::card(ui, |ui| {
                    for entry in &entries {
                        // Each row inside its own scope: `ui.min_rect()` is the
                        // running total of everything drawn so far, so reading
                        // it after each row reports the same number whatever
                        // the rows do. The first version of this test did that
                        // and passed while the screenshot plainly disagreed.
                        let drawn = ui
                            .scope(|ui| activity_row_for_test(ui, entry, 9_000))
                            .response
                            .rect;
                        widths.push((drawn.width() * 100.0).round() / 100.0);
                    }
                });
                let first = widths[0];
                for (index, w) in widths.iter().enumerate() {
                    assert_eq!(
                        *w, first,
                        "row {index} is {w} wide, row 0 is {first}, at {width}px",
                    );
                }
            });
        });
    }
}

/// The Air card, in each of the states it can be in.
///
/// The eight-page sweep reaches it only empty. Its populated states carry the
/// longest text on the page — a full address in monospace, a signed package as
/// text, and an animated code — and those are the ones that would overflow.
#[test]
fn the_air_card_fits_supported_content_widths_in_every_state() {
    use nightfall_crypto::WalletKeys;
    use nightfall_types::NetworkId;
    use nightfall_wallet::air::{self, Intent, Signed};

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let dir = std::env::temp_dir().join(format!(
        "nf-air-layout-{}-{:?}",
        std::process::id(),
        std::thread::current().id(),
    ));
    let mut app = App::new(NetworkId::Devnet, dir.clone());

    let intent = Intent {
        network: NetworkId::Devnet,
        to: WalletKeys::from_seed([61; 32]).address(),
        amount_darks: 1_234_567_890,
        fee_darks: 100_000,
        tip_height: 1_284,
        nonce: air::new_nonce(),
        expires_unix: nightfall_storage::now_unix() + 3_600,
    };
    let signed = Signed {
        network: NetworkId::Devnet,
        nonce: intent.nonce.clone(),
        payload: vec![0xab; 2_000],
    };

    // Empty, waiting for an answer, reading a request, and showing frames.
    type AirScenario = (&'static str, Box<dyn Fn(&mut App)>);
    let states: [AirScenario; 4] = [
        ("empty", Box::new(|_: &mut App| {})),
        (
            "waiting",
            Box::new({
                let intent = intent.clone();
                move |app: &mut App| {
                    app.air_pending = Some(intent.clone());
                    app.air_frames = air::frames(intent.to_text().as_bytes()).unwrap();
                }
            }),
        ),
        (
            "reading a request",
            Box::new({
                let intent = intent.clone();
                move |app: &mut App| app.air_incoming = Some(intent.clone())
            }),
        ),
        (
            "showing a signed answer",
            Box::new({
                let text = signed.to_text();
                move |app: &mut App| {
                    app.air_input = text.clone();
                    app.air_frames = air::frames(text.as_bytes()).unwrap();
                    app.air_note = Some("Signed.".into());
                    app.air_error = Some("An error long enough to wrap onto a second line, because that is the one that overflows.".into());
                }
            }),
        ),
    ];

    for width in [620.0, 884.0, 1180.0] {
        for (name, set_up) in &states {
            app.air_pending = None;
            app.air_incoming = None;
            app.air_frames.clear();
            app.air_note = None;
            app.air_error = None;
            app.air_input.clear();
            set_up(&mut app);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 1600.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    app.view = View::Send;
                    send(&mut app, ui, ctx);
                    assert!(
                        ui.min_rect().right() <= right + 1.0,
                        "air card overflow at {width}px while {name}: {} > {right}",
                        ui.min_rect().right(),
                    );
                });
            });
        }
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// The till's invoice rows, at every width and in every state.
///
/// The candidate warning is the longest sentence this screen can produce and
/// the one that must never be pushed off: it is what stands between a
/// merchant and treating an unidentified payment as settlement.
#[test]
fn the_till_rows_fit_supported_content_widths() {
    use crate::views::invoice_row;
    use nightfall_wallet::counter::{status, Incoming, Invoice, InvoiceState};

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);

    let base = Invoice {
        reference: "R".repeat(nightfall_wallet::counter::MAX_REFERENCE),
        amount_darks: Some(1_234_567_890),
        description: "d".repeat(nightfall_wallet::counter::MAX_DESCRIPTION),
        created_unix: 1_000,
        expires_unix: Some(2_000),
        closed_note: None,
    };
    let quoting = |amount: u64, at: u64| Incoming {
        amount_darks: amount,
        memo: base.reference.clone(),
        height: Some(1),
        timestamp: at,
        reference_id: "c".into(),
    };
    let anonymous = Incoming {
        amount_darks: 1_234_567_890,
        memo: String::new(),
        height: Some(1),
        timestamp: 1_500,
        reference_id: "d".into(),
    };

    let mut closed = base.clone();
    closed.closed_note = Some("n".repeat(nightfall_wallet::counter::MAX_DESCRIPTION));

    let cases = [
        // open, and with an unidentified payment sitting next to it
        (base.clone(), vec![anonymous.clone()], 1_500u64),
        (base.clone(), vec![], 9_999), // expired
        (base.clone(), vec![quoting(1_234_567_890, 1_500)], 1_600), // paid
        (base.clone(), vec![quoting(1_234_567_890, 9_000)], 9_999), // paid late
        (
            base.clone(),
            vec![quoting(1, 1_500), anonymous.clone()],
            1_600,
        ), // short + candidate
        (base.clone(), vec![quoting(9_999_999_999, 1_500)], 1_600), // overpaid
        (closed, vec![], 1_600),
        (
            Invoice {
                amount_darks: None,
                ..base.clone()
            },
            vec![quoting(7, 1_500)],
            1_600,
        ),
    ];

    let mut seen = Vec::new();
    for width in [620.0, 884.0, 1180.0] {
        for (invoice, payments, now) in &cases {
            let state = status(invoice, payments, *now);
            if width == 620.0 {
                seen.push(state.state.clone());
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    invoice_row(ui, invoice, &state);
                    assert!(
                        ui.min_rect().right() <= right + 1.0,
                        "invoice row overflow at {width}px for {:?}",
                        state.state,
                    );
                });
            });
        }
    }

    // Every state the till can be in was actually drawn, so this test cannot
    // quietly stop covering one when the cases above are edited.
    for expected in [
        InvoiceState::Open,
        InvoiceState::Expired,
        InvoiceState::Closed,
        InvoiceState::Paid { late: false },
        InvoiceState::Paid { late: true },
    ] {
        assert!(seen.contains(&expected), "{expected:?} was never drawn");
    }
    assert!(seen
        .iter()
        .any(|s| matches!(s, InvoiceState::Underpaid { .. })));
    assert!(seen
        .iter()
        .any(|s| matches!(s, InvoiceState::Overpaid { .. })));
}

/// The state behind the Proof Card: a verdict must never outlive the document
/// it was about.
///
/// This is the same defect that turned up in the browser wallet's request
/// reader, where a refused parse left the previous request's address and
/// verdict on screen. Pinned here before it can happen a second time on the
/// screen whose entire purpose is saying what a document proves.
#[test]
fn a_proof_verdict_never_outlives_the_receipt_it_was_about() {
    use crate::app::App;
    use crate::views::check_receipt;
    use nightfall_crypto::WalletKeys;
    use nightfall_wallet::{PaymentReceipt, ReceiptKind};

    let root = std::env::temp_dir().join(format!(
        "nightfall-proof-card-{}-{:?}",
        std::process::id(),
        std::thread::current().id(),
    ));
    let mut app = App::new(NetworkId::Devnet, root.clone());

    // A real receipt, produced the way the wallet produces one: mint a
    // coinbase to this wallet in a block that exists only in this process,
    // scan it, and prove the output. No test-only hook into the wallet — the
    // point is that this is the ordinary path.
    use nightfall_consensus::{Block, BlockHeader};
    use nightfall_ledger::{build_coinbase, BlockBody, LedgerState};
    use nightfall_types::{Hash256, Height, NetworkId, PROTOCOL_VERSION};

    let keys = WalletKeys::from_seed([21; 32]);
    let mut wallet = nightfall_wallet::Wallet::in_memory(NetworkId::Devnet, keys.clone(), 0);
    let ctx = NetworkId::Devnet.proof_context();
    let reward = 20 * nightfall_types::DARKS_PER_NIGHT;
    let coinbase = build_coinbase(&wallet.address(), reward, 0, ctx).unwrap();
    let body = BlockBody::aggregate(&[coinbase]);
    let mut ledger = LedgerState::genesis();
    ledger.apply_block(&body, Height(0), reward, ctx).unwrap();
    let block = Block {
        header: BlockHeader {
            version: PROTOCOL_VERSION,
            height: Height(0),
            prev_hash: Hash256::ZERO,
            utxo_root: ledger.utxo_root(),
            kernel_sum: ledger.kernel_sum(),
            body_root: body.hash(),
            timestamp_unix: 1_800_000_000,
            difficulty: 1,
            nonce: 0,
            reward_darks: reward,
        },
        body,
    };
    assert_eq!(wallet.scan_blocks(&[block]).unwrap(), 1);
    let commit = hex::encode(wallet.outputs()[0].commit.0);
    let json = wallet.prove_output(&commit).unwrap().to_json().unwrap();

    // Nothing typed: no verdict at all, rather than a stale or default one.
    check_receipt(&mut app);
    assert!(app.proof_result.is_none());

    app.proof_input = json.clone();
    check_receipt(&mut app);
    let good = app.proof_result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(good.kind, ReceiptKind::Mined);
    assert_eq!(good.amount_darks, reward);
    assert!(good.amount_proven);

    // A tampered document must replace that verdict, not sit beside it.
    let mut broken: PaymentReceipt = serde_json::from_str(&json).unwrap();
    broken.amount_darks += 1;
    app.proof_input = serde_json::to_string(&broken).unwrap();
    check_receipt(&mut app);
    assert!(app.proof_result.as_ref().unwrap().is_err());

    // So must text that is not a receipt at all …
    app.proof_input = "not json".into();
    check_receipt(&mut app);
    assert!(app.proof_result.as_ref().unwrap().is_err());

    // … and emptying the field must leave no verdict behind.
    app.proof_input = "   ".into();
    check_receipt(&mut app);
    assert!(app.proof_result.is_none());

    std::fs::remove_dir_all(&root).ok();
}

/// The Proof Card, at every width and for each kind of claim it can make.
///
/// The sent case matters most: it carries the longest text, because it has to
/// say both that the amount is unproven and that the chain was not consulted.
/// A card that overflowed would push exactly that sentence off the screen.
#[test]
fn the_proof_card_fits_supported_content_widths() {
    use crate::views::proof_card;
    use nightfall_crypto::WalletKeys;
    use nightfall_wallet::{ReceiptKind, ReceiptProof};

    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let address = WalletKeys::from_seed([6; 32]).address().encode();

    // Reports, not receipts: whether a signature verifies is settled in
    // `receipt.rs`, and what this test measures is the card drawn from the
    // answer. Every combination that changes how much text appears.
    let mut proofs = Vec::new();
    for kind in [
        ReceiptKind::Received,
        ReceiptKind::Mined,
        ReceiptKind::Sent,
        ReceiptKind::Other,
    ] {
        for unsigned_fields in [&[][..], &["kind", "timestamp"][..]] {
            proofs.push(ReceiptProof {
                version: if unsigned_fields.is_empty() { 2 } else { 1 },
                kind,
                address: address.clone(),
                amount_darks: 1_234_567_890,
                memo: "m".repeat(64),
                height: 123_456,
                timestamp: 1_700_000_000,
                reference: hex::encode([7u8; 32]),
                amount_proven: kind != ReceiptKind::Sent,
                unsigned_fields,
            });
        }
    }

    for width in [620.0, 884.0, 1180.0] {
        for proof in &proofs {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 1400.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    proof_card(ui, proof);
                    assert!(
                        ui.min_rect().right() <= right + 1.0,
                        "proof card overflow at {width}px for {:?} v{}",
                        proof.kind,
                        proof.version,
                    );
                });
            });
        }
    }

    // Whatever the card says, it never claims the chain was consulted, and a
    // receipt whose amount is unproven says so.
    for proof in &proofs {
        let open = proof.not_established().join(" ");
        assert!(open.contains("in the chain"), "{open}");
        if !proof.amount_proven {
            assert!(open.contains("amount is what the receipt says"), "{open}");
        }
    }
}

/// Render one page exactly as `App::update` does — centred column, the page's
/// own width cap, the always-visible scrollbar — and return the outer rect of
/// every top-level card on it.
///
/// The shell matters. `settings()` called straight onto a CentralPanel gets a
/// different width from the real thing, and the difference is the scrollbar:
/// measuring without it is measuring a page nobody sees.
fn card_edges(
    ctx: &egui::Context,
    app: &mut App,
    view: View,
    width: f32,
) -> Vec<(egui::Rect, String)> {
    let mut run = || {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().inner_margin(egui::Margin {
                    left: 24.0,
                    right: 24.0,
                    top: 4.0,
                    bottom: 16.0,
                }))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .show(ui, |ui| {
                            crate::widgets::page_column(ui, view.content_max_width(), |ui| {
                                crate::widgets::fill_width(ui, ui.available_width());
                                page_intro(view, ui);
                                match view {
                                    View::Settings => settings(app, ui, ctx),
                                    View::Send => send(app, ui, ctx),
                                    View::Receive => receive(app, ui, ctx),
                                    View::Dashboard => dashboard(app, ui),
                                    View::Activity => activity(app, ui),
                                    View::Mining => mining(app, ui),
                                    View::Network => network(app, ui, ctx),
                                }
                            });
                        });
                });
        });
    };
    // First frame warms galleys and scroll state; the second is the one that
    // reflects what a person looking at the window would see.
    run();
    let _ = crate::widgets::card_probe::take();
    run();
    crate::widgets::card_probe::take()
}

/// Cards on one page stand in one column, or the page looks broken.
///
/// This is the fault the owner reported on Settings and no test could see:
/// every card fitted inside the page, so `all_eight_pages_fit…` passed, while
/// on screen one card started 50 points to the right of the one above it. An
/// overflow test measures the page; this measures the cards against each other.
#[test]
fn cards_on_one_page_share_one_left_and_right_edge() {
    let ctx = egui::Context::default();
    crate::theme::apply(&ctx);
    let dir = std::env::temp_dir().join(format!("nf-ui-edges-{}", nightfall_storage::now_unix()));
    let mut app = App::new(nightfall_types::NetworkId::Devnet, dir);
    // An empty wallet is the one case where nothing can overflow. Give the
    // page the content a used wallet has — including a contact name longer
    // than today's limit, because a file written by an older build still has
    // to draw — or the test only ever measures a wallet nobody owns.
    app.address_book.entries = vec![
        crate::address_book::AddressBookEntry {
            name: "Kiosk".into(),
            address: format!("nf1{}", "a".repeat(70)),
        },
        crate::address_book::AddressBookEntry {
            name: "x".repeat(200),
            address: format!("nf1{}", "b".repeat(70)),
        },
    ];
    app.book_name = "y".repeat(120);
    app.book_addr = format!("nf1{}", "c".repeat(70));
    // Every page at every width, reported together. Stopping at the first
    // ragged card hides the rest, and these faults come in families.
    let mut ragged: Vec<String> = Vec::new();
    // 620 is narrower than the window can be made (940 minus the rail), so it
    // is the stress case; 1728 is a 1920 display, where the capped pages have
    // to stay centred instead of drifting.
    for width in [620.0, 884.0, 1180.0, 1728.0] {
        for (view, name) in View::ALL {
            app.view = view;
            let rects = card_edges(&ctx, &mut app, view, width);
            if rects.len() < 2 {
                continue;
            }
            // Two columns put two cards side by side, and those two are meant
            // to have different x. Group the cards into rows by vertical
            // overlap first; what must line up is the *row*, left edge of the
            // leftmost card and right edge of the rightmost.
            struct Row {
                bottom: f32,
                left: f32,
                right: f32,
                titles: Vec<String>,
            }
            let mut rows: Vec<Row> = Vec::new();
            for (r, title) in &rects {
                match rows.iter_mut().find(|row| r.top() < row.bottom - 1.0) {
                    Some(row) => {
                        row.bottom = row.bottom.max(r.bottom());
                        row.left = row.left.min(r.left());
                        row.right = row.right.max(r.right());
                        row.titles.push(title.clone());
                    }
                    None => rows.push(Row {
                        bottom: r.bottom(),
                        left: r.left(),
                        right: r.right(),
                        titles: vec![title.clone()],
                    }),
                }
            }
            let Some(first) = rows.first() else { continue };
            let (first_left, first_right) = (first.left, first.right);
            // Cards that touch read as one block, and cards that overlap read
            // as a bug. Consecutive rows keep at least a small gap.
            for pair in rects.windows(2) {
                let (above, below) = (&pair[0], &pair[1]);
                let gap = below.0.top() - above.0.bottom();
                if below.0.top() > above.0.bottom() - 1.0 && gap < 8.0 {
                    ragged.push(format!(
                        "{name} at {width}px: only {gap:.0} points between \"{}\" and \"{}\"",
                        above.1, below.1,
                    ));
                }
            }
            for row in &rows {
                if (row.left - first_left).abs() > 0.5 || (row.right - first_right).abs() > 0.5 {
                    ragged.push(format!(
                        "{name} at {width}px: \"{}\" is [{:.0}..{:.0}], the page is [{first_left:.0}..{first_right:.0}]",
                        row.titles.join("\" + \""),
                        row.left,
                        row.right,
                    ));
                }
            }
        }
    }
    assert!(
        ragged.is_empty(),
        "cards do not line up:\n{}",
        ragged.join("\n")
    );
}
