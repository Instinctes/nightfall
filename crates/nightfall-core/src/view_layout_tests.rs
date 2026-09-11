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
