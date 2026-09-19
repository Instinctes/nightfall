//! Design tokens and global styling.
//!
//! Original Nightfall violet palette with native macOS typography.
//! egui has no gradient primitive, so
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

/// Pinned by `original_nightfall_palette_is_preserved`. Its last consumer was
/// the withdrawn swap page; the token stays because the brand does.
#[allow(dead_code)]
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
/// Glass sheets: large corners like the frosted slabs in the UI language.
pub const ROUND: f32 = 28.0;
/// Nested tiles, nav pills, chips. Not a capsule — inputs use ROUND_FIELD.
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

/// Use fonts supplied by macOS without redistributing Apple's font files.
/// Other platforms (and machines without these fonts) keep egui's fallbacks.
/// Called once at startup, never on the repaint path.
pub fn install_platform_fonts(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    {
        let mut fonts = egui::FontDefinitions::default();
        for (name, path, family) in [
            (
                "macOS UI",
                "/System/Library/Fonts/SFNS.ttf",
                FontFamily::Proportional,
            ),
            (
                "macOS Mono",
                "/System/Library/Fonts/Menlo.ttc",
                FontFamily::Monospace,
            ),
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes));
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, name.into());
            }
        }
        ctx.set_fonts(fonts);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = ctx;
}

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
    // Transparent, so that a panel which forgets its own frame cannot flood
    // the whole window rectangle with an opaque fill — which is what several
    // of them did, including the lock screen, and what put a hard square
    // corner outside the rounded plate. `widgets::paint_room` is the one
    // place the page's background comes from.
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = SURFACE;
    v.extreme_bg_color = glass_surface();
    v.faint_bg_color = glass_inner();
    v.window_stroke = Stroke::NONE;
    v.window_rounding = Rounding::same(ROUND);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_HI);
    v.hyperlink_color = ACCENT_HI;

    // Text fields inherit these. Buttons override with a pill of their own, so
    // the radius here is the field radius — see `ROUND_FIELD`.
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = glass_surface();
    w.noninteractive.weak_bg_fill = glass_surface();
    w.noninteractive.bg_stroke = Stroke::NONE;
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
    w.noninteractive.rounding = Rounding::same(ROUND_FIELD);

    w.inactive.bg_fill = glass_surface();
    w.inactive.weak_bg_fill = glass_surface();
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = Rounding::same(ROUND_FIELD);

    w.hovered.bg_fill = glass_hover();
    w.hovered.weak_bg_fill = glass_hover();
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.rounding = Rounding::same(ROUND_FIELD);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = glass_inner();
    w.active.weak_bg_fill = glass_inner();
    w.active.bg_stroke = Stroke::new(2.0_f32, ACCENT_HI);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.rounding = Rounding::same(ROUND_FIELD);

    w.open.bg_fill = glass_inner();
    w.open.bg_stroke = Stroke::NONE;

    style.spacing.item_spacing = egui::vec2(GAP_SM, GAP_SM);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.window_margin = egui::Margin::same(18.0);
    style.spacing.interact_size.y = CONTROL_H;
    // The scroll bar, on glass.
    //
    // A solid bar paints its trough with `extreme_bg_color`, and that is
    // `glass_surface()` here — so the trough became a lit panel and read as a
    // bright scratch down the right-hand side of every page, in front of the
    // content rather than behind it.
    //
    // A floating bar has a trough whose opacity can be set, and egui's own
    // documentation names this exact arrangement: allocate a little width so
    // the layout still reserves the column, then keep the handle faintly
    // visible. The reservation is the point — an appearing-and-disappearing
    // bar shifts every right-aligned value sideways as you scroll, which is
    // why this was `AlwaysVisible` in the first place. `allocated_width()`
    // returns `floating_allocated_width` in this mode, so `scroll_gutter`
    // keeps measuring the same thing.
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.floating_width = 5.0;
    style.spacing.scroll.floating_allocated_width = 9.0;
    style.spacing.scroll.bar_inner_margin = 6.0;
    style.spacing.scroll.handle_min_length = 36.0;
    style.spacing.scroll.foreground_color = true;
    // No trough at all until the pointer is on the bar itself.
    style.spacing.scroll.dormant_background_opacity = 0.0;
    style.spacing.scroll.active_background_opacity = 0.0;
    style.spacing.scroll.interact_background_opacity = 0.10;
    // The handle stays readable without becoming a second border.
    style.spacing.scroll.dormant_handle_opacity = 0.30;
    style.spacing.scroll.active_handle_opacity = 0.55;
    style.spacing.scroll.interact_handle_opacity = 0.85;

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

/// An opaque tint of `color` laid over `base`.
///
/// Use this for any filled notice, never `Color32::gamma_multiply`, which
/// lowers the *alpha* and leaves the fill see-through. That is invisible until
/// something passes behind it — and the page banners are drawn above the
/// scroll area, so the balance card slid underneath one and its buttons showed
/// through the warning. A notice must be a surface, not a gel.
///
/// `base` is whatever the notice sits on: `BG` for a page banner, `SURFACE`
/// for one inside a card.
pub fn tint(base: Color32, color: Color32, amount: f32) -> Color32 {
    lerp_color(base, color, amount)
}

/// Token RGB with an explicit alpha. Palette tests pin the RGB channels;
/// this never changes them. Used for frosted panels so the page lights show
/// through without inventing a second set of brand colours.
pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Violet frost over the room gradient. The pastel pigment softens the
/// sheets without the bright white borders of a conventional glass effect.
///
/// The pigment and the alpha are the two numbers that decide whether anything
/// written on this sheet can be read. Frosted at `GRAD_A 0.30` and alpha 132,
/// a panel over the brightest part of the room came out at RGB (160, 149, 219)
/// — and `TEXT_DIM` on that is 3.8:1, `TEXT_FAINT` 3.4:1, both under the 4.5:1
/// that small text needs. On screen that read as grey-on-grey, and the warning
/// banner was the worst of it. `text_stays_readable_on_every_glass` pins these
/// two numbers against the room's brightest point; change one and it will say
/// so with the ratio it measured.
pub fn glass_surface() -> Color32 {
    with_alpha(tint(SURFACE, GRAD_A, 0.06), 205)
}

/// Light caught by selected navigation items and secondary controls.
pub fn glass_inner() -> Color32 {
    with_alpha(tint(SURFACE_HI, GRAD_A, 0.02), 232)
}

/// Hover differs by *density*, not by pigment.
///
/// A hover state that adds pigment lightens the sheet, and a lighter sheet is
/// a worse background for the text already on it — hover was the last thing
/// left failing the contrast test. Raising the alpha instead always moves the
/// sheet towards its opaque token, which already passes, so this cue cannot
/// make anything harder to read whatever the room is doing behind it.
pub fn glass_hover() -> Color32 {
    with_alpha(tint(SURFACE_HI, GRAD_A, 0.02), 240)
}

/// The floating navigation sheet is denser than the content cards.
pub fn glass_rail() -> Color32 {
    // Opaque, on purpose. The page is the sheet the desktop shows through
    // now; if the rail were translucent too, the only thing behind its
    // overhanging half would be the wallpaper, and the navigation would sit
    // on whatever photograph the owner happens to use.
    tint(RAIL, GRAD_A, 0.08)
}

/// Alert glass: dark surface washed with the status colour.
///
/// Washed *lightly*. Every status colour in this palette is brighter than the
/// surface, so tinting the panel towards the tone lifts the panel and the tone
/// is then written on top of it: at `0.22` the warning banner was `WARN` on a
/// panel that `WARN` had helped to lighten, 2.7:1, and a warning nobody can
/// read is worse than no warning. The tone belongs in the rim and the text.
pub fn glass_alert(tone: Color32) -> Color32 {
    with_alpha(tint(SURFACE, tone, 0.02), 246)
}

/// Contained card shadow: kept inside the column gap.
pub fn glass_card_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 16.0,
        spread: 0.0,
        color: with_alpha(INK, 48),
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
    #[test]
    fn original_nightfall_palette_is_preserved() {
        // UI refinements must not silently turn Nightfall into a graphite theme.
        for (actual, expected) in [
            (BG, [0x24, 0x1D, 0x36]),
            (RAIL, [0x2B, 0x23, 0x40]),
            (SURFACE, [0x36, 0x2D, 0x50]),
            (SURFACE_HI, [0x44, 0x3A, 0x62]),
            (SURFACE_HOVER, [0x51, 0x45, 0x73]),
            (SURFACE_LOW, [0x29, 0x21, 0x3D]),
            (BORDER, [0x4A, 0x3F, 0x6B]),
            (BORDER_HI, [0x80, 0x6D, 0x9E]),
            (WASH_A, [0x2C, 0x24, 0x42]),
            (WASH_B, [0x31, 0x23, 0x45]),
            (TEXT, [0xF6, 0xF2, 0xFF]),
            (TEXT_DIM, [0xD4, 0xC9, 0xE4]),
            (TEXT_FAINT, [0xC8, 0xBC, 0xDB]),
            (ACCENT, [0x76, 0x50, 0xD8]),
            (ACCENT_HI, [0xDE, 0xC4, 0xFF]),
            (ACCENT_DIM, [0x58, 0x40, 0x7B]),
            (INK, [0x25, 0x1C, 0x3A]),
            (GRAD_A, [0xBD, 0xA2, 0xFF]),
            (PINK, [0xE7, 0x9C, 0xEF]),
            (CYAN, [0x78, 0xDB, 0xEC]),
        ] {
            assert_eq!([actual.r(), actual.g(), actual.b()], expected);
        }
    }

    #[test]
    fn glass_keeps_nightfall_rgb() {
        // Opaque round-trip is exact. Translucent values are stored
        // premultiplied by egui; RGB tokens themselves stay untouched.
        assert_eq!(with_alpha(SURFACE, 255), SURFACE);
        assert_eq!(with_alpha(RAIL, 255), RAIL);
        assert_eq!(with_alpha(PINK, 36).a(), 36);
        assert_eq!(with_alpha(CYAN, 30).a(), 30);
        assert!(glass_surface().a() < 255);
        assert!(glass_inner().a() < 255);
        assert!(glass_alert(WARN).a() < 255);
        assert!(glass_hover().a() < 255);
        // The rail is the one sheet that is *not* see-through, and that is the
        // decision rather than an oversight. Half of it hangs over the
        // desktop: were it translucent, the only thing behind that half would
        // be the owner's wallpaper, and the navigation would be legible or not
        // depending on which photograph they happen to use. The page is what
        // the desktop shows through — see `widgets::PLATE_ALPHA`.
        assert_eq!(
            glass_rail().a(),
            255,
            "the rail carries navigation over the desktop and stays opaque",
        );
    }

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

    /// The elevation ladder only goes one way.
    ///
    /// It did not. `SURFACE` — the card fill — was darker than `BG`, the page
    /// it sits on, for the whole life of this theme. A dark interface reads a
    /// card that catches *less* light than its background as a hole rather
    /// than an object, and that single inversion is why every screen looked
    /// flat no matter how its contents were arranged.
    ///
    /// It is not visible in the source: `BG` and `SURFACE` are two names, and
    /// nothing says which should be lighter. This says it.
    #[test]
    fn surfaces_get_lighter_as_they_come_closer() {
        let ladder = [
            ("BG", BG),
            ("RAIL", RAIL),
            ("SURFACE", SURFACE),
            ("SURFACE_HI", SURFACE_HI),
            ("SURFACE_HOVER", SURFACE_HOVER),
        ];
        for pair in ladder.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert!(
                luminance(upper.1) > luminance(lower.1),
                "{} must be lighter than {} — a card darker than its page reads \
                 as a hole, not as a card",
                upper.0,
                lower.0,
            );
            // …and by enough to see. Two surfaces a rounding error apart are
            // one surface with two names.
            assert!(
                luminance(upper.1) - luminance(lower.1) > 0.004,
                "{} and {} are too close to tell apart",
                upper.0,
                lower.0,
            );
        }

        // The one thing that goes down: a field is something you put
        // something into, so it is sunk below the card that holds it.
        assert!(
            luminance(SURFACE_LOW) < luminance(SURFACE),
            "a text field must read as sunk into its card, not raised off it",
        );
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

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Lay a translucent colour over an opaque one, the way the screen does.
    ///
    /// egui keeps translucent colours premultiplied, so the source term is
    /// already `colour × alpha` and only the destination is weighted.
    fn over(fg: Color32, bg: Color32) -> Color32 {
        let a = f32::from(fg.a()) / 255.0;
        let mix = |f: u8, b: u8| (f32::from(f) + f32::from(b) * (1.0 - a)).round() as u8;
        Color32::from_rgb(
            mix(fg.r(), bg.r()),
            mix(fg.g(), bg.g()),
            mix(fg.b(), bg.b()),
        )
    }

    /// The brightest point the room lighting reaches.
    fn brightest_wash() -> Color32 {
        let mut best = crate::widgets::wash_color(0.0, 0.0);
        for row in 0..=60 {
            for col in 0..=60 {
                let c = crate::widgets::wash_color(col as f32 / 60.0, row as f32 / 60.0);
                if luminance(c) > luminance(best) {
                    best = c;
                }
            }
        }
        best
    }

    /// Text has to be readable on the glass, not on the token behind it.
    ///
    /// `readable_text_tokens_meet_normal_text_contrast` above passed the whole
    /// time the interface was unreadable, because it measures `TEXT_DIM`
    /// against `SURFACE` — and `SURFACE` is not what anybody sees. What they
    /// see is `glass_surface()`, translucent, lying over a lit room. Measured
    /// on the running window, a caption came out at 3.5:1, the block counter
    /// at 2.4:1 and the warning banner at 2.7:1, while this file's own
    /// contrast test was green.
    ///
    /// So this one composites: the brightest point the lighting reaches, the
    /// glass over it, and the text on that. Every tone that carries words must
    /// clear 4.5:1, which is what small text needs — and the small text is
    /// exactly what failed.
    #[test]
    fn text_stays_readable_on_every_glass() {
        // The page is translucent now, so the worst background is no longer
        // the brightest point of our own room — it is that point with a pure
        // white wallpaper showing through it. A wallpaper is the one thing in
        // this picture nobody here chooses, so it has to be assumed hostile.
        let room = over(
            with_alpha(brightest_wash(), crate::widgets::PLATE_ALPHA),
            Color32::WHITE,
        );
        let mut faults = Vec::new();
        let sheets = [
            ("glass_surface", glass_surface()),
            ("glass_inner", glass_inner()),
            ("glass_hover", glass_hover()),
            ("glass_rail", glass_rail()),
        ];
        for (sheet_name, sheet) in sheets {
            let panel = over(sheet, room);
            for (text_name, text) in [
                ("TEXT", TEXT),
                ("TEXT_DIM", TEXT_DIM),
                ("TEXT_FAINT", TEXT_FAINT),
            ] {
                let ratio = contrast(text, panel);
                if ratio < 4.5 {
                    faults.push(format!(
                        "{text_name} on {sheet_name} is {ratio:.2}:1 over the room's \
                         brightest point {room:?} (panel {panel:?})"
                    ));
                }
            }
        }
        // A status banner writes its own colour on its own glass, and every
        // status colour here is lighter than the surface — so a panel tinted
        // towards the tone lifts itself towards the very text it carries.
        for (tone_name, tone) in [("WARN", WARN), ("DANGER", DANGER), ("SUCCESS", SUCCESS)] {
            let panel = over(glass_alert(tone), room);
            for (text_name, text) in [("its own tone", tone), ("TEXT", TEXT)] {
                let ratio = contrast(text, panel);
                if ratio < 4.5 {
                    faults.push(format!(
                        "{text_name} on glass_alert({tone_name}) is {ratio:.2}:1 \
                         (panel {panel:?})"
                    ));
                }
            }
        }
        assert!(
            faults.is_empty(),
            "unreadable on glass:\n{}",
            faults.join("\n")
        );
    }
}
