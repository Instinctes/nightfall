use crate::{
    app::{App, View},
    views::*,
};
use eframe::egui::{self, Vec2};

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
                        View::Swap => crate::views_swap::swap(&mut app, ui, ctx),
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
    let states: [(&str, Box<dyn Fn(&mut App)>); 4] = [
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
        (base.clone(), vec![quoting(1, 1_500), anonymous.clone()], 1_600), // short + candidate
        (base.clone(), vec![quoting(9_999_999_999, 1_500)], 1_600), // overpaid
        (closed, vec![], 1_600),
        (
            Invoice { amount_darks: None, ..base.clone() },
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
    assert!(seen.iter().any(|s| matches!(s, InvoiceState::Underpaid { .. })));
    assert!(seen.iter().any(|s| matches!(s, InvoiceState::Overpaid { .. })));
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
