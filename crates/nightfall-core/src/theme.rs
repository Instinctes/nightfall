//! Design tokens and global styling.
//!
//! Deep violet, soft elevation, large radii. egui has no gradient primitive, so
//! [`crate::widgets::gradient_rect`] tessellates one by hand.

use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

// --- surfaces -------------------------------------------------------------
//
// # Elevation, and the bug that lived here
//
// These used to run the wrong way. `BG` was #423858 and `SURFACE` — the card
// fill — was #302747, *darker* than the page it sat on. In a dark interface a
// card is expected to catch more light than its background; when it catches
// less, the eye reads it as a hole rather than an object, and every screen
// looks flat and unfinished no matter how the contents are arranged. It was
// the single biggest reason the wallet looked like a prototype.
//
// The ladder now only goes one way, and each step is a real step (roughly 6–8
// points of lightness, enough to see without banding):
//
//   BG        the page, darkest
//   RAIL      navigation, between the page and a card
//   SURFACE   a card, clearly above the page
//   SURFACE_HI a control on a card
//   SURFACE_HOVER the same control under the pointer
//
// `SURFACE_LOW` is the one thing that goes *down*: a text field is a hole you
// put something into, and it should read as sunk below the card.

/// Page background. Not black — a desaturated violet reads warmer and lets the
/// accent gradients sit on it without vibrating.
pub const BG: Color32 = Color32::from_rgb(0x24, 0x1D, 0x36);
/// Navigation rail. Between the page and a card, so the rail reads as part of
/// the window's frame rather than as a very wide card.
pub const RAIL: Color32 = Color32::from_rgb(0x2B, 0x23, 0x40);
/// Card fill. Above the page — see the note above.
pub const SURFACE: Color32 = Color32::from_rgb(0x36, 0x2D, 0x50);
/// Raised element inside a card.
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x44, 0x3A, 0x62);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x51, 0x45, 0x73);
/// Sunken element: text fields, code blocks. Below the card on purpose.
pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x29, 0x21, 0x3D);

pub const BORDER: Color32 = Color32::from_rgb(0x4A, 0x3F, 0x6B);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x80, 0x6D, 0x9E);

// --- depth ----------------------------------------------------------------
// The page wash. Kept close to BG now that the page is the dark end of the
// ladder: a wash that is much lighter than its page competes with the cards
// standing on it, which is what made the old background feel busy and the
// cards feel weak at the same time.
pub const WASH_A: Color32 = Color32::from_rgb(0x2C, 0x24, 0x42);
pub const WASH_B: Color32 = Color32::from_rgb(0x31, 0x23, 0x45);

// --- spacing --------------------------------------------------------------
//
// One scale, so that "a gap" is always one of five numbers rather than
// whatever looked right in the moment. The old code used 4, 6, 8, 10, 12, 14,
// 18, 22 and 28 interchangeably, which is why nothing lined up between cards.
/// Inside a row: label to value, icon to text.
pub const GAP_XS: f32 = 6.0;
/// Between related lines in a card.
pub const GAP_SM: f32 = 10.0;
/// Between a heading and its body, and between rows.
pub const GAP_MD: f32 = 16.0;
/// Between cards.
pub const GAP_LG: f32 = 24.0;
/// Between sections of a page.
pub const GAP_XL: f32 = 36.0;

/// Every button is this tall. Three heights for one kind of control is what
/// made the lock screen's two buttons look ragged.
pub const CONTROL_H: f32 = 38.0;

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
/// Text fields and other things you put something *into*.
///
/// Separate from `ROUND_SM` because a 32-point field at radius 16 is a
/// capsule, and a capsule reads as a button. Every input in the wallet looked
/// like a pill you could press, which is why the lock screen's password field
/// competed with the button under it instead of leading to it.
pub const ROUND_FIELD: f32 = 10.0;

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

    // Text fields inherit these. Buttons override with a pill of their own, so
    // the radius here is the field radius — see `ROUND_FIELD`.
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    w.noninteractive.rounding = Rounding::same(ROUND_FIELD);

    // A field is sunk into the card, not raised off it. It used to be filled
    // with SURFACE_HI — lighter than the card — so an empty input was the
    // brightest object on the screen and pulled the eye away from whatever the
    // screen was actually asking.
    w.inactive.bg_fill = SURFACE_LOW;
    w.inactive.weak_bg_fill = SURFACE_LOW;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = Rounding::same(ROUND_FIELD);

    w.hovered.bg_fill = SURFACE_LOW;
    w.hovered.weak_bg_fill = SURFACE_LOW;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_HI);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.rounding = Rounding::same(ROUND_FIELD);
    w.hovered.expansion = 0.0;

    // Focused. A 2-point accent edge, so which field has the keyboard is
    // visible from across the room rather than inferred from the caret.
    w.active.bg_fill = SURFACE_LOW;
    w.active.weak_bg_fill = SURFACE_LOW;
    w.active.bg_stroke = Stroke::new(2.0_f32, ACCENT_HI);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.rounding = Rounding::same(ROUND_FIELD);

    w.open.bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0_f32, BORDER_HI);

    style.spacing.item_spacing = egui::vec2(GAP_SM, GAP_SM);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.window_margin = egui::Margin::same(18.0);
    style.spacing.interact_size.y = CONTROL_H;
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
