//! Widgets built for the swap view, in the web wallet's visual rhythm.
//!
//! The web wallet and this one already share a palette; what it has and this
//! did not is the rhythm — spaced uppercase kickers instead of plain titles,
//! bordered pills instead of bare text, and generous radii. These bring that
//! across, plus the one piece the swap view needs and nothing else has: a
//! picture of where the state machine currently stands.

use crate::theme::*;
use crate::widgets::gradient_rect;
use eframe::egui::{self, Color32, Rect, RichText, Rounding, Sense, Stroke, Vec2};

// `kicker` and `pill` used to live here. They moved to `widgets` once every
// tab needed them; the swap view picks them up from there.

/// How to colour a timeline.
#[derive(Clone, Copy)]
pub struct TimelineStyle {
    /// Colour for reached steps when `gradient` is false.
    pub hot: Color32,
    /// Paint the current node with the signature gradient.
    pub gradient: bool,
}

impl TimelineStyle {
    /// The swap is progressing — full brand gradient.
    pub fn progress() -> Self {
        Self {
            hot: ACCENT,
            gradient: true,
        }
    }

    /// The swap is unwinding — amber, no celebration.
    pub fn abort() -> Self {
        Self {
            hot: WARN,
            gradient: false,
        }
    }
}

/// The state machine, drawn.
///
/// Reached steps are filled, the current one is larger and lit, later ones are
/// hollow. The point is that a user can tell at a glance how far along a swap
/// is and how much is left — the thing the old view could not answer at all.
pub fn timeline(
    ui: &mut egui::Ui,
    steps: &[&str],
    current: usize,
    settled: bool,
    style: TimelineStyle,
) {
    const ROW: f32 = 30.0;
    const LABEL: f32 = 30.0;
    const R: f32 = 6.0;
    const R_HOT: f32 = 9.0;

    let width = ui.available_width();
    if steps.is_empty() || width < 60.0 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ROW + LABEL), Sense::hover());
    let painter = ui.painter();

    let n = steps.len();
    let slot = rect.width() / n as f32;
    let cy = rect.top() + ROW / 2.0;
    let centre = |i: usize| rect.left() + slot * (i as f32 + 0.5);

    // Connectors first, so the nodes sit on top of them.
    for i in 0..n.saturating_sub(1) {
        let done = i < current;
        painter.line_segment(
            [
                egui::pos2(centre(i) + R_HOT + 2.0, cy),
                egui::pos2(centre(i + 1) - R_HOT - 2.0, cy),
            ],
            Stroke::new(2.0_f32, if done { style.hot } else { BORDER }),
        );
    }

    for (i, label) in steps.iter().enumerate() {
        let x = centre(i);
        let is_now = i == current;
        let reached = i <= current;

        if is_now {
            // A soft ring so the eye lands here first.
            painter.circle_filled(
                egui::pos2(x, cy),
                R_HOT + 5.0,
                style.hot.gamma_multiply(0.18),
            );
            let node = Rect::from_center_size(egui::pos2(x, cy), Vec2::splat(R_HOT * 2.0));
            if style.gradient {
                gradient_rect(painter, node, R_HOT, Vec2::new(1.0, 0.0), brand_gradient);
            } else {
                painter.circle_filled(egui::pos2(x, cy), R_HOT, style.hot);
            }
        } else if reached {
            painter.circle_filled(egui::pos2(x, cy), R, style.hot.gamma_multiply(0.75));
        } else {
            painter.circle_stroke(egui::pos2(x, cy), R, Stroke::new(1.5_f32, BORDER_HI));
        }

        let colour = if is_now {
            if settled {
                SUCCESS
            } else {
                TEXT
            }
        } else if reached {
            TEXT_DIM
        } else {
            TEXT_FAINT
        };
        let galley = painter.layout(
            (*label).to_string(),
            egui::FontId::proportional(10.5),
            colour,
            slot - 6.0,
        );
        painter.galley(
            egui::pos2(x - galley.size().x / 2.0, rect.top() + ROW + 2.0),
            galley,
            colour,
        );
    }
}

/// A countdown bar with its own heading and explanation.
///
/// Separate from [`crate::widgets::progress`] because a deadline is not a
/// loading indicator: the colour carries meaning, and the sentence under it
/// has to say what happens when the bar fills.
pub fn deadline_bar(ui: &mut egui::Ui, label: &str, detail: &str, fraction: f32, colour: Color32) {
    let f = fraction.clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.5).color(TEXT_DIM).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}%", (f * 100.0).round() as i32))
                    .size(10.5)
                    .color(colour)
                    .monospace(),
            );
        });
    });
    ui.add_space(5.0);

    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 7.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(3.5), SURFACE_LOW);
    if f > 0.0 {
        let filled = Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height()));
        ui.painter()
            .rect_filled(filled, Rounding::same(3.5), colour);
    }
    ui.add_space(5.0);
    ui.label(RichText::new(detail).size(11.0).color(TEXT_FAINT));
}

/// The trade itself, on the brand gradient: what you give, what you get.
pub fn trade_card(ui: &mut egui::Ui, give: &str, give_unit: &str, get: &str, get_unit: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 96.0), Sense::hover());
    gradient_rect(
        ui.painter(),
        rect,
        ROUND,
        Vec2::new(1.0, 0.35),
        brand_gradient,
    );

    let side = |ui: &egui::Ui, x: f32, amount: &str, unit: &str, align_right: bool| {
        let a = ui.painter().layout_no_wrap(
            amount.to_string(),
            egui::FontId::monospace(21.0),
            Color32::WHITE,
        );
        let u = ui.painter().layout_no_wrap(
            unit.to_string(),
            egui::FontId::proportional(11.0),
            Color32::from_white_alpha(190),
        );
        let ax = if align_right { x - a.size().x } else { x };
        let ux = if align_right { x - u.size().x } else { x };
        ui.painter()
            .galley(egui::pos2(ax, rect.center().y - 20.0), a, Color32::WHITE);
        ui.painter().galley(
            egui::pos2(ux, rect.center().y + 6.0),
            u,
            Color32::from_white_alpha(190),
        );
    };

    side(ui, rect.left() + 24.0, give, give_unit, false);
    side(ui, rect.right() - 24.0, get, get_unit, true);

    let arrow = ui.painter().layout_no_wrap(
        "→".to_string(),
        egui::FontId::proportional(20.0),
        Color32::from_white_alpha(150),
    );
    ui.painter().galley(
        rect.center() - arrow.size() / 2.0,
        arrow,
        Color32::from_white_alpha(150),
    );
}
