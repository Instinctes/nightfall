//! Design tokens and global styling.
//!
//! Deep violet, soft elevation, large radii. egui has no gradient primitive, so
//! [`crate::widgets::gradient_rect`] tessellates one by hand.

use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

// --- surfaces -------------------------------------------------------------
/// Page background. Not black — a desaturated violet reads warmer and lets the
/// accent gradients sit on it without vibrating.
pub const BG: Color32 = Color32::from_rgb(0x14, 0x10, 0x22);
/// Navigation rail, one step darker than the page.
pub const RAIL: Color32 = Color32::from_rgb(0x18, 0x13, 0x28);
/// Card fill.
pub const SURFACE: Color32 = Color32::from_rgb(0x20, 0x1A, 0x35);
/// Raised element inside a card.
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x28, 0x21, 0x40);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x32, 0x2A, 0x4E);
/// Sunken element: text fields, code blocks.
pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x1A, 0x15, 0x2C);

pub const BORDER: Color32 = Color32::from_rgb(0x2E, 0x26, 0x48);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x45, 0x39, 0x68);

// --- text -----------------------------------------------------------------
pub const TEXT: Color32 = Color32::from_rgb(0xF0, 0xEC, 0xFA);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xA8, 0xA0, 0xC4);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x74, 0x6C, 0x94);

// --- accents --------------------------------------------------------------
pub const ACCENT: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xB4, 0x9B, 0xFF);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x4A, 0x2F, 0x8C);

/// The signature gradient: violet into magenta into teal.
///
/// Deliberately dark. The first attempt used pastel tones and white text on
/// them was barely legible — a balance you cannot read is a broken balance.
/// Every stop here keeps a contrast ratio above 4.5:1 against white.
pub const GRAD_A: Color32 = Color32::from_rgb(0x56, 0x33, 0xC4);
pub const PINK: Color32 = Color32::from_rgb(0x8E, 0x31, 0xB2);
pub const CYAN: Color32 = Color32::from_rgb(0x27, 0x6F, 0x8E);

pub const SUCCESS: Color32 = Color32::from_rgb(0x4A, 0xE0, 0xA8);
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x5C);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x7B, 0x8A);

// --- geometry -------------------------------------------------------------
/// Cards. Generous, matching the reference look.
pub const ROUND: f32 = 20.0;
/// Buttons, inputs, chips.
pub const ROUND_SM: f32 = 12.0;
/// Fully rounded pills.
pub const ROUND_PILL: f32 = 999.0;

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(24.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE_LOW;
    v.faint_bg_color = SURFACE_HI;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(ROUND);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT_HI);
    v.hyperlink_color = ACCENT_HI;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.rounding = Rounding::same(ROUND_SM);

    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE_HI;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.rounding = Rounding::same(ROUND_SM);

    w.hovered.bg_fill = SURFACE_HOVER;
    w.hovered.weak_bg_fill = SURFACE_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, BORDER_HI);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.rounding = Rounding::same(ROUND_SM);
    w.hovered.expansion = 1.0;

    w.active.bg_fill = ACCENT_DIM;
    w.active.weak_bg_fill = ACCENT_DIM;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.rounding = Rounding::same(ROUND_SM);

    w.open.bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0, BORDER_HI);

    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.window_margin = egui::Margin::same(18.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.scroll.bar_width = 8.0;

    ctx.set_style(style);
}

/// Colour for a status indicator.
pub fn status_color(ok: bool) -> Color32 {
    if ok {
        SUCCESS
    } else {
        DANGER
    }
}

/// Linear interpolation between two colours.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_unmultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

/// Sample the brand gradient at `t ∈ [0,1]`: violet → pink → cyan.
pub fn brand_gradient(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.55 {
        lerp_color(GRAD_A, PINK, t / 0.55)
    } else {
        lerp_color(PINK, CYAN, (t - 0.55) / 0.45)
    }
}
