//! Design tokens and global styling.
//!
//! Deep violet, soft elevation, large radii. egui has no gradient primitive, so
//! [`crate::widgets::gradient_rect`] tessellates one by hand.

use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

// --- surfaces -------------------------------------------------------------
/// Page background. Not black — a desaturated violet reads warmer and lets the
/// accent gradients sit on it without vibrating.
pub const BG: Color32 = Color32::from_rgb(0x42, 0x38, 0x58);
/// Navigation rail, one step darker than the page.
pub const RAIL: Color32 = Color32::from_rgb(0x33, 0x2C, 0x4D);
/// Card fill.
pub const SURFACE: Color32 = Color32::from_rgb(0x30, 0x27, 0x47);
/// Raised element inside a card.
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x4B, 0x40, 0x64);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x54, 0x47, 0x70);
/// Sunken element: text fields, code blocks.
pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x40, 0x36, 0x57);

pub const BORDER: Color32 = Color32::from_rgb(0x55, 0x48, 0x70);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x80, 0x6D, 0x9E);

// --- depth ----------------------------------------------------------------
// Softer, layered violet surfaces based on the user's visual reference.
pub const WASH_A: Color32 = Color32::from_rgb(0x4A, 0x40, 0x65);
pub const WASH_B: Color32 = Color32::from_rgb(0x50, 0x38, 0x65);

// --- text -----------------------------------------------------------------
pub const TEXT: Color32 = Color32::from_rgb(0xF6, 0xF2, 0xFF);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xD4, 0xC9, 0xE4);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0xC8, 0xBC, 0xDB);

// --- accents --------------------------------------------------------------
pub const ACCENT: Color32 = Color32::from_rgb(0x76, 0x50, 0xD8);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xDE, 0xC4, 0xFF);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x58, 0x40, 0x7B);

/// Pastel brand gradient. Always pair with INK, never white text.
pub const INK: Color32 = Color32::from_rgb(0x25, 0x1C, 0x3A);
pub const GRAD_A: Color32 = Color32::from_rgb(0xBD, 0xA2, 0xFF);
pub const PINK: Color32 = Color32::from_rgb(0xE7, 0x9C, 0xEF);
pub const CYAN: Color32 = Color32::from_rgb(0x78, 0xDB, 0xEC);

pub const SUCCESS: Color32 = Color32::from_rgb(0x4A, 0xE0, 0xA8);
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xC8, 0x5C);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x7B, 0x8A);

// --- geometry -------------------------------------------------------------
/// Cards. Matches the web wallet's `--r-lg`, so the two surfaces read as
/// one product rather than two that happen to share a palette.
pub const ROUND: f32 = 28.0;
/// Buttons, inputs, chips.
pub const ROUND_SM: f32 = 16.0;
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
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = Rounding::same(ROUND);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_HI);
    v.hyperlink_color = ACCENT_HI;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    w.noninteractive.rounding = Rounding::same(ROUND_SM);

    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE_HI;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = Rounding::same(ROUND_SM);

    w.hovered.bg_fill = SURFACE_HOVER;
    w.hovered.weak_bg_fill = SURFACE_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_HI);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.rounding = Rounding::same(ROUND_SM);
    w.hovered.expansion = 1.0;

    w.active.bg_fill = ACCENT_DIM;
    w.active.weak_bg_fill = ACCENT_DIM;
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.rounding = Rounding::same(ROUND_SM);

    w.open.bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0_f32, BORDER_HI);

    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.window_margin = egui::Margin::same(18.0);
    style.spacing.interact_size.y = 38.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let n = f64::from(v) / 255.0;
            if n <= 0.04045 {
                n / 12.92
            } else {
                ((n + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    #[test]
    fn pastel_gradient_uses_readable_dark_ink() {
        for step in 0..=100 {
            let background = brand_gradient(step as f32 / 100.0);
            assert!((luminance(background) + 0.05) / (luminance(INK) + 0.05) >= 4.5);
        }
    }
    #[test]
    fn readable_text_tokens_meet_normal_text_contrast() {
        for text in [TEXT, TEXT_DIM, TEXT_FAINT] {
            for surface in [BG, RAIL, SURFACE, SURFACE_HI, SURFACE_LOW] {
                assert!((luminance(text) + 0.05) / (luminance(surface) + 0.05) >= 4.5);
            }
        }
    }
}
