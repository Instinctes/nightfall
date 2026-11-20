//! Motion, and the signature pieces of the Core 2.0 look.
//!
//! Everything that moves lives here so that one switch can stop it: an owner
//! who turns on "Reduce motion" in Settings, and the screenshot walk in
//! `ui_shots`, both need every spring to land on its target in the frame it is
//! asked for. A spring that is still travelling when a picture is taken is a
//! picture of a half-drawn page.
//!
//! None of these widgets decides anything about money. They draw state the
//! caller already has and report gestures back; the slide that sends a payment
//! returns `true` and the caller does exactly what its button used to do.

use crate::theme::*;
use crate::widgets::radial_wash;
use eframe::egui::{self, Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};

// ------------------------------------------------------------------ motion ---

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Stop (or allow) every animation in the wallet.
pub fn set_reduce_motion(on: bool) {
    REDUCE_MOTION.store(on, Ordering::Relaxed);
}

/// Whether things may move.
pub fn motion() -> bool {
    !REDUCE_MOTION.load(Ordering::Relaxed)
}

/// A spring towards `target`, remembered under `id`.
///
/// A little under critical damping, so a selection overshoots by a hair and
/// settles — the feel of a native control rather than of a tween. Integrated
/// in two half steps per frame, and the frame time is capped, so a slow frame
/// cannot launch the value past its target.
pub fn spring(ctx: &egui::Context, id: egui::Id, target: f32) -> f32 {
    spring_with(ctx, id, target, 190.0, 19.0)
}

pub fn spring_with(
    ctx: &egui::Context,
    id: egui::Id,
    target: f32,
    stiffness: f32,
    damping: f32,
) -> f32 {
    if !motion() {
        ctx.data_mut(|d| d.insert_temp(id, (target, 0.0_f32)));
        return target;
    }
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
    let (mut x, mut v) = ctx
        .data(|d| d.get_temp::<(f32, f32)>(id))
        .unwrap_or((target, 0.0));
    let h = dt / 2.0;
    for _ in 0..2 {
        let a = stiffness * (target - x) - damping * v;
        v += a * h;
        x += v * h;
    }
    if (target - x).abs() < 0.05 && v.abs() < 0.5 {
        x = target;
        v = 0.0;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, (x, v)));
    x
}

/// 0 → 1 over `secs` since `since` (seconds of `ctx` time), eased out.
pub fn ease_since(ctx: &egui::Context, since: f64, secs: f64) -> f32 {
    if !motion() {
        return 1.0;
    }
    let now = ctx.input(|i| i.time);
    let p = ((now - since) / secs).clamp(0.0, 1.0) as f32;
    if p < 1.0 {
        ctx.request_repaint();
    }
    1.0 - (1.0 - p).powi(3)
}

/// The time to hand to anything that drifts on its own.
pub fn clock(ctx: &egui::Context) -> f32 {
    if motion() {
        ctx.input(|i| i.time) as f32
    } else {
        0.0
    }
}

// ---------------------------------------------------------------- odometer ---

/// A tween on an exact integer, kept in `f64` because a balance in darks does
/// not fit an `f32` without losing its last digits.
fn tween_u64(ctx: &egui::Context, id: egui::Id, target: u64, intro: bool) -> f64 {
    #[derive(Clone, Copy)]
    struct T {
        from: f64,
        to: u64,
        start: f64,
        secs: f64,
    }
    let now = ctx.input(|i| i.time);
    // The first time this number is seen it counts up from zero — once per
    // run, not on every visit to the page: the state outlives the page.
    let mut t = ctx.data(|d| d.get_temp::<T>(id)).unwrap_or(if intro {
        T {
            from: 0.0,
            to: target,
            start: now,
            secs: 1.9,
        }
    } else {
        T {
            from: target as f64,
            to: target,
            start: now - 10.0,
            secs: 1.25,
        }
    });
    let shown = |t: &T| {
        let p = ((now - t.start) / t.secs).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - p).powi(4);
        t.from + (t.to as f64 - t.from) * e
    };
    if t.to != target {
        t = T {
            from: shown(&t),
            to: target,
            start: now,
            secs: 1.25,
        };
    }
    let value = if motion() { shown(&t) } else { target as f64 };
    if (value - target as f64).abs() > 0.5 {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, t));
    value
}

/// A balance that rolls like a mechanical counter when it changes.
///
/// `units` is the amount in its smallest unit and `decimals` how many of the
/// last digits are the fraction. The whole part is drawn at `size`, the
/// fraction smaller and quieter, and every digit sits in a cell of one width
/// so the number does not shuffle sideways as it rolls.
pub fn odometer(
    ui: &mut egui::Ui,
    id: egui::Id,
    units: u64,
    decimals: u32,
    size: f32,
    color: Color32,
) -> egui::Response {
    let value = tween_u64(ui.ctx(), id, units, true);
    let scale = 10_u64.pow(decimals) as f64;
    // Laid out for the larger of where it is going and where it is, so the
    // number keeps one width while it counts and the label beside it stays
    // put. Places the count has not reached yet are simply not drawn.
    let whole = ((value.max(units as f64)) / scale).floor() as u64;
    let whole_text = crate::widgets::format_int(whole);

    let big = egui::FontId::proportional(size);
    let small = egui::FontId::proportional((size * 0.46).max(12.0));
    let cell = |font: &egui::FontId| {
        ui.fonts(|f| {
            (0..10)
                .map(|d| f.glyph_width(font, char::from(b'0' + d)))
                .fold(0.0_f32, f32::max)
        })
    };
    let (cell_big, cell_small) = (cell(&big), cell(&small));
    let comma = ui.fonts(|f| f.glyph_width(&big, ','));
    let dot = ui.fonts(|f| f.glyph_width(&small, '.'));

    // Places, most significant first: (power of ten in units, big?)
    let mut layout: Vec<(Option<u32>, bool)> = Vec::new();
    let whole_digits = whole_text.chars().filter(|c| c.is_ascii_digit()).count() as u32;
    let mut k = whole_digits;
    for ch in whole_text.chars() {
        if ch.is_ascii_digit() {
            k -= 1;
            layout.push((Some(k + decimals), true));
        } else {
            layout.push((None, true));
        }
    }
    let width: f32 = layout
        .iter()
        .map(|(p, _)| if p.is_some() { cell_big } else { comma })
        .sum::<f32>()
        + dot
        + cell_small * decimals as f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, size * 1.12), Sense::hover());
    let painter = ui
        .painter()
        .with_clip_rect(rect.expand2(Vec2::new(2.0, 0.0)));
    let base = rect.bottom() - size * 0.12;

    let draw_digit = |x: f32, place: u32, font: &egui::FontId, w: f32, h: f32, alpha: u8| {
        let pow = 10_f64.powi(place as i32);
        if place > decimals && value < pow {
            return;
        }
        let x_val = value / pow;
        let digit = (x_val.floor() as u64 % 10) as u8;
        // A place only turns while every place below it is passing 9 → 0.
        let roll = if place == 0 {
            x_val.fract() as f32
        } else {
            ((value % pow) - (pow - 1.0)).clamp(0.0, 1.0) as f32
        };
        let top = base - h;
        let clip = Rect::from_min_max(egui::pos2(x, top - 2.0), egui::pos2(x + w, base + 4.0));
        let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let c = with_alpha(color, alpha);
        for (d, dy) in [(digit, -roll * h), ((digit + 1) % 10, (1.0 - roll) * h)] {
            if dy.abs() >= h {
                continue;
            }
            p.text(
                egui::pos2(x + w / 2.0, base + dy),
                egui::Align2::CENTER_BOTTOM,
                char::from(b'0' + d).to_string(),
                font.clone(),
                c,
            );
        }
    };

    let mut x = rect.left();
    for (x_index, (place, _)) in layout.iter().enumerate() {
        match place {
            Some(p) => {
                draw_digit(x, *p, &big, cell_big, size, 255);
                x += cell_big;
            }
            None => {
                // A separator belongs to the digit before it.
                if value >= 10_f64.powi((next_place(&layout, x_index) + 1) as i32) {
                    painter.text(
                        egui::pos2(x, base),
                        egui::Align2::LEFT_BOTTOM,
                        ",",
                        big.clone(),
                        color,
                    );
                }
                x += comma;
            }
        }
    }
    painter.text(
        egui::pos2(x, base - size * 0.04),
        egui::Align2::LEFT_BOTTOM,
        ".",
        small.clone(),
        with_alpha(color, 200),
    );
    x += dot;
    for i in (0..decimals).rev() {
        draw_digit(x, i, &small, cell_small, small.size, 200);
        x += cell_small;
    }
    resp
}

/// The place of the first digit after position `from` in an odometer layout.
fn next_place(layout: &[(Option<u32>, bool)], from: usize) -> u32 {
    layout[from..].iter().find_map(|(p, _)| *p).unwrap_or(0)
}

// -------------------------------------------------------------- the hero ---

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// How dark the hero is at `u` (0 left … 1 right) while night falls:
/// `progress` 0 is every light on, 1 is night everywhere. The front is wide
/// and eased so the change reads as a dimming, not a wipe.
fn darkness(u: f32, progress: f32) -> f32 {
    let front = -0.45 + progress * 1.9;
    1.0 - smooth(front - 0.45, front, u)
}

/// How far night has fallen on the hero since it was first shown: once per
/// run, eased in and out over 1.6 seconds.
fn nightfall_progress(ctx: &egui::Context) -> f32 {
    if !motion() {
        return 1.0;
    }
    let id = egui::Id::new("hero-nightfall-start");
    let now = ctx.input(|i| i.time);
    let start = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    let p = ((now - start) / 1.6).clamp(0.0, 1.0) as f32;
    if p < 1.0 {
        ctx.request_repaint();
    }
    smooth(0.0, 1.0, p)
}

/// The hero's moving sky, rendered into a small texture every frame.
///
/// Everything that lights the sheet — the drifting glows, the aurora band,
/// the glare under the pointer, the flash when coins arrive — is baked into
/// this one image, and the image is drawn as a *rounded* rectangle. A light
/// painted on top of the sheet would be clipped by a rectangle and show square
/// corners wherever it reaches the edge; a light inside the texture cannot.
///
/// The left half, where the balance is written, is kept to the plain gradient
/// so white text stays at the contrast `white_reads_on_the_hero` pins.
fn hero_sky(
    ctx: &egui::Context,
    t: f32,
    aspect: f32,
    flash: f32,
    pointer: Option<(f32, f32)>,
    nightfall: f32,
) -> egui::TextureHandle {
    const W: usize = 160;
    const H: usize = 48;
    // A light that comes up quickly and then goes out, softly, every few
    // seconds — each glow on its own beat.
    let beat = |period: f32, phase: f32| {
        let k = (t / period + phase).fract();
        smooth(0.0, 0.16, k) * (1.0 - smooth(0.16, 0.86, k))
    };
    let glows = [
        (
            0.78 + 0.10 * (t * 0.21).sin(),
            0.30 + 0.25 * (t * 0.33).sin(),
            PINK,
            0.30 + 0.30 * beat(6.0, 0.0),
        ),
        (
            0.58 + 0.16 * (t * 0.17).cos(),
            0.85 + 0.12 * (t * 0.29).sin(),
            CYAN,
            0.26 + 0.30 * beat(7.5, 0.37),
        ),
        (
            0.95 + 0.06 * (t * 0.13).sin(),
            0.55 + 0.20 * (t * 0.23).cos(),
            GRAD_A,
            0.24 + 0.28 * beat(5.2, 0.71),
        ),
    ];
    let mut pixels = Vec::with_capacity(W * H);
    for row in 0..H {
        let v = row as f32 / (H - 1) as f32;
        for col in 0..W {
            let u = col as f32 / (W - 1) as f32;
            let mut c = hero_gradient((u * 0.78 + v * 0.22).clamp(0.0, 1.0));
            // Night settles from the bottom left: a long, soft fall from the
            // lit top right into deep violet where the balance is written.
            let deep = (1.0 - smooth(0.0, 1.0, u * 0.7 + (1.0 - v) * 0.3)) * 0.30;
            c = lerp_color(c, INK, deep);
            // Lights live on the right; the writing on the left keeps its sky.
            let room = 0.18 + 0.82 * smooth(0.30, 0.72, u);
            for (cx, cy, colour, strength) in glows {
                let dx = (u - cx) * aspect;
                let dy = v - cy;
                let w = (-(dx * dx + dy * dy) / (2.0 * 0.55_f32.powi(2))).exp();
                c = lerp_color(c, colour, w * strength * room);
            }
            // The aurora: two soft ribbons rippling along the sheet at
            // different speeds, so the light never repeats the same shape.
            // The ribbons begin well to the right of the balance and fade in
            // over a long stretch, so the middle of the sheet stays calm.
            let fade_in = smooth(0.50, 0.92, u);
            for (base, amp, k1, k2, speed, width, strength) in [
                (0.60, 0.16, 5.0, 11.0, 0.55, 0.11, 0.55),
                (0.36, 0.10, 3.3, 8.0, -0.38, 0.08, 0.32),
            ] {
                let y0 = base
                    + amp * (u * k1 + t * speed).sin()
                    + amp * 0.4 * (u * k2 - t * speed * 1.4).sin();
                let band = (-((v - y0) / width).powi(2)).exp() * fade_in;
                // Cyan into pink and back along a sine, never by wrapping a
                // fraction: a wrap jumps from pink straight to cyan and
                // leaves a hard vertical seam in the light.
                let hue = 0.5 + 0.5 * (u * 3.2 + t * 0.15 + base * 4.0).sin();
                let ribbon = lerp_color(CYAN, PINK, hue);
                c = lerp_color(c, ribbon, band * strength);
            }
            if flash > 0.02 {
                c = lerp_color(c, PINK, flash * 0.28);
            }
            // When the lights go out: the sheet opens lit, and darkness sweeps
            // across it from the left in a soft, wide front.
            let dark = darkness(u, nightfall);
            if dark < 1.0 {
                let lit = lerp_color(lerp_color(GRAD_A, PINK, v * 0.7), Color32::WHITE, 0.30);
                c = lerp_color(lit, c, dark);
            }
            if let Some((pu, pv)) = pointer {
                let dx = (u - pu) * aspect;
                let dy = v - pv;
                let w = (-(dx * dx + dy * dy) / (2.0 * 0.45_f32.powi(2))).exp();
                c = lerp_color(c, Color32::WHITE, w * 0.10);
            }
            pixels.push(Color32::from_rgb(c.r(), c.g(), c.b()));
        }
    }
    let image = egui::ColorImage {
        size: [W, H],
        pixels,
    };
    let id = egui::Id::new("hero-sky-texture");
    match ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        Some(mut handle) => {
            handle.set(image, egui::TextureOptions::LINEAR);
            handle
        }
        None => {
            let handle = ctx.load_texture("hero-sky", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
            handle
        }
    }
}

/// The balance card: a deep violet-to-teal sheet with white text and a slowly
/// moving aurora sky, a glare that follows the pointer and a few twinkling
/// stars — all inside its rounded corners.
///
/// The pastel brand gradient carries dark ink and stays the colour of the
/// main buttons; the hero is the one surface deep enough for white, which is
/// pinned by `white_reads_on_the_hero` in `theme`.
pub fn hero_card<R>(
    ui: &mut egui::Ui,
    height: f32,
    flash: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter().clone();
    let t = clock(ui.ctx());

    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: egui::vec2(0.0, 14.0),
            blur: 34.0,
            spread: -6.0,
            color: with_alpha(ACCENT, 120),
        }
        .as_shape(rect, Rounding::same(ROUND)),
    ));
    let pointer = resp.hover_pos().map(|p| {
        (
            (p.x - rect.left()) / rect.width().max(1.0),
            (p.y - rect.top()) / rect.height().max(1.0),
        )
    });
    let night = nightfall_progress(ui.ctx());
    let sky = hero_sky(
        ui.ctx(),
        t,
        rect.width() / rect.height().max(1.0),
        flash,
        pointer,
        night,
    );
    painter.add(egui::Shape::Rect(egui::epaint::RectShape {
        rect,
        rounding: Rounding::same(ROUND),
        fill: Color32::WHITE,
        stroke: Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
        blur_width: 0.0,
        fill_texture_id: sky.id(),
        uv: Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
    }));

    // Stars on the right, well inside the corners so none sits on the rim.
    let field = Rect::from_min_max(
        egui::pos2(rect.left() + rect.width() * 0.50, rect.top() + ROUND * 0.7),
        egui::pos2(rect.right() - ROUND * 0.9, rect.bottom() - ROUND * 0.7),
    );
    for i in 0..22_u32 {
        let h = i.wrapping_mul(2_654_435_761).rotate_left(11) ^ 0x5bd1_e995;
        let x = (h & 0xFFFF) as f32 / 65535.0;
        let y = ((h >> 16) & 0xFFFF) as f32 / 65535.0;
        // A star only comes out once the dark has reached it.
        let out = darkness(0.50 + 0.50 * x, night);
        let twinkle = (0.5 + 0.5 * (t * (0.8 + x) + i as f32 * 1.7).sin()) * out;
        let p = egui::pos2(
            field.left() + field.width() * x,
            field.top() + field.height() * y,
        );
        let r = 0.8 + 1.2 * ((h >> 7) & 3) as f32 / 3.0;
        radial_wash(
            &painter,
            p,
            r * 4.0,
            Color32::from_white_alpha((twinkle * 40.0) as u8),
        );
        painter.circle_filled(
            p,
            r,
            Color32::from_white_alpha(((90.0 + twinkle * 150.0) * out) as u8),
        );
    }
    if motion() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    let inner = rect.shrink2(Vec2::new(26.0, 22.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    add(&mut child)
}

/// A white pill for the main action on the hero.
pub fn hero_button(ui: &mut egui::Ui, id: &str, text: &str, filled: bool) -> egui::Response {
    let font = egui::FontId::proportional(14.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let size = Vec2::new((galley.size().x + 40.0).max(108.0), 40.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let pressed = resp.is_pointer_button_down_on();
    let lift = if resp.hovered() { -1.5 } else { 0.0 };
    let s = spring(
        ui.ctx(),
        egui::Id::new(("hero-btn", id)),
        if pressed { 0.94 } else { 1.0 },
    );
    let r = Rect::from_center_size(rect.center() + Vec2::new(0.0, lift), rect.size() * s);
    let (fill, fg) = if filled {
        (Color32::WHITE, HERO_INK)
    } else {
        (Color32::from_black_alpha(70), Color32::WHITE)
    };
    ui.painter().rect(
        r,
        Rounding::same(ROUND_SM - 2.0),
        fill,
        if filled {
            Stroke::NONE
        } else {
            Stroke::new(1.0_f32, Color32::from_white_alpha(70))
        },
    );
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(14.5 * s),
        fg,
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(
            r.expand(3.0),
            Rounding::same(ROUND_SM),
            Stroke::new(2.0_f32, Color32::WHITE),
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

// ------------------------------------------------------------------ charts ---

/// A smooth area chart: a line with a fading fill under it.
pub fn area_chart(ui: &mut egui::Ui, data: &[f64], height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    area_chart_painter(
        ui.painter(),
        rect,
        data,
        GRAD_A,
        with_alpha(GRAD_A, 70),
        true,
    );
}

pub fn area_chart_painter(
    painter: &egui::Painter,
    rect: Rect,
    data: &[f64],
    line: Color32,
    fill: Color32,
    dot: bool,
) {
    let n = data.len();
    if n < 2 {
        return;
    }
    let max = data.iter().cloned().fold(f64::MIN, f64::max);
    if max <= 0.0 {
        return;
    }
    let min = data.iter().cloned().fold(f64::MAX, f64::min).min(max * 0.6);
    let span = (max - min).max(1e-9);
    let pts: Vec<Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            egui::pos2(
                rect.left() + rect.width() * (i as f32 / (n - 1) as f32),
                rect.bottom() - 4.0 - (rect.height() - 10.0) * ((v - min) / span) as f32,
            )
        })
        .collect();
    // Catmull-Rom through the samples so the line reads as a curve.
    let mut curve = Vec::with_capacity(n * 6);
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        for s in 0..6 {
            let t = s as f32 / 6.0;
            let t2 = t * t;
            let t3 = t2 * t;
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (-a + c) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            curve.push(egui::pos2(
                f(p0.x, p1.x, p2.x, p3.x),
                f(p0.y, p1.y, p2.y, p3.y).min(rect.bottom()),
            ));
        }
    }
    curve.push(pts[n - 1]);

    // Fill: a strip of quads from the curve to the floor, fading downward.
    let mut mesh = egui::Mesh::default();
    let clear = Color32::from_rgba_premultiplied(0, 0, 0, 0);
    for p in &curve {
        mesh.colored_vertex(*p, fill);
        mesh.colored_vertex(egui::pos2(p.x, rect.bottom()), clear);
    }
    for i in 0..(curve.len() as u32 - 1) {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::line(curve, Stroke::new(2.0_f32, line)));
    if dot {
        let last = pts[n - 1];
        painter.circle_filled(last, 6.0, with_alpha(line, 60));
        painter.circle_filled(last, 3.2, line);
    }
}

// ----------------------------------------------------------- power button ---

/// The mining switch: a large round button with a turning ring of light while
/// it is on.
pub fn power_button(ui: &mut egui::Ui, on: bool, diameter: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(diameter + 36.0), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            on,
            if on { "Stop mining" } else { "Start mining" },
        )
    });
    let ctx = ui.ctx().clone();
    let press = spring(
        &ctx,
        egui::Id::new("power-press"),
        if resp.is_pointer_button_down_on() {
            0.93
        } else if resp.hovered() {
            1.02
        } else {
            1.0
        },
    );
    let glow = spring(
        &ctx,
        egui::Id::new("power-glow"),
        if on { 1.0 } else { 0.0 },
    );
    let c = rect.center();
    let r = diameter / 2.0 * press;
    let painter = ui.painter();
    let t = clock(&ctx);

    if glow > 0.01 {
        // A ring of soft lights in the brand colours, turning.
        for i in 0..18 {
            let a = i as f32 / 18.0 * TAU + t * 0.9;
            let p = c + Vec2::angled(a) * (r + 6.0);
            radial_wash(
                painter,
                p,
                r * 0.55,
                with_alpha(brand_gradient(i as f32 / 17.0), (glow * 60.0) as u8),
            );
        }
        ctx.request_repaint();
    }
    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: egui::vec2(0.0, 10.0),
            blur: 28.0,
            spread: -4.0,
            color: with_alpha(INK, 140),
        }
        .as_shape(
            Rect::from_center_size(c, Vec2::splat(r * 2.0)),
            Rounding::same(r),
        ),
    ));
    // Body: glass when off, the hero's violet when on.
    let off = glass_inner();
    let body_top = lerp_color(off, HERO_A, glow);
    let body_bottom = lerp_color(SURFACE_HI, lerp_color(HERO_A, INK, 0.35), glow);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c - Vec2::new(0.0, r * 0.35), body_top);
    const SEG: usize = 72;
    for i in 0..=SEG {
        let a = i as f32 / SEG as f32 * TAU;
        let p = c + Vec2::angled(a) * r;
        let k = ((p.y - (c.y - r)) / (2.0 * r)).clamp(0.0, 1.0);
        mesh.colored_vertex(p, lerp_color(body_top, body_bottom, k));
    }
    for i in 0..SEG as u32 {
        mesh.add_triangle(0, 1 + i, 2 + i);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(
        c,
        r,
        Stroke::new(
            1.0_f32,
            with_alpha(Color32::WHITE, (30.0 + glow * 40.0) as u8),
        ),
    );

    // The power glyph.
    let icon = lerp_color(TEXT_DIM, Color32::WHITE, glow);
    let ir = r * 0.30;
    let arc: Vec<Pos2> = (0..=40)
        .map(|i| {
            let a = -TAU * 0.20 + (i as f32 / 40.0) * TAU * 0.80 - TAU * 0.25;
            c + Vec2::angled(a + TAU * 0.25 + TAU * 0.05) * ir
        })
        .collect();
    let stroke = Stroke::new((r * 0.075).max(3.0), icon);
    painter.add(egui::Shape::line(arc, stroke));
    painter.line_segment(
        [c - Vec2::new(0.0, ir * 1.25), c - Vec2::new(0.0, ir * 0.25)],
        stroke,
    );

    if resp.has_focus() {
        painter.circle_stroke(c, r + 5.0, Stroke::new(2.0_f32, ACCENT_HI));
    }
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

// ------------------------------------------------------------------ slider ---

/// A whole-number slider in the macOS manner: a thin track filled up to a
/// white knob, and the value written beside it. Arrow keys step it when it
/// has focus. Returns the new value on the frame it changes.
pub fn slider_int(
    ui: &mut egui::Ui,
    id: &str,
    value: i32,
    min: i32,
    max: i32,
    width: f32,
) -> Option<i32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click_and_drag());
    let span = (max - min).max(1) as f32;
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 24.0, 6.0));
    let mut next = value;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let t = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
            next = min + (t * span).round() as i32;
        }
    }
    if resp.has_focus() {
        let (left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if left {
            next -= 1;
        }
        if right {
            next += 1;
        }
    }
    next = next.clamp(min, max);
    resp.widget_info(|| egui::WidgetInfo::slider(true, next as f64, format!("{id} {next}")));

    let t = spring(
        ui.ctx(),
        egui::Id::new(("slider", id)),
        (next - min) as f32 / span,
    );
    let painter = ui.painter();
    painter.rect_filled(track, Rounding::same(3.0), SURFACE_LOW);
    let filled = Rect::from_min_max(
        track.min,
        egui::pos2(
            track.left() + track.width() * t.clamp(0.0, 1.0),
            track.bottom(),
        ),
    );
    crate::widgets::gradient_rect(painter, filled, 3.0, Vec2::new(1.0, 0.0), |k| {
        brand_gradient(k * 0.6)
    });
    let knob = egui::pos2(track.left() + track.width() * t, track.center().y);
    let r = if resp.is_pointer_button_down_on() {
        12.5
    } else {
        11.0
    };
    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: egui::vec2(0.0, 2.0),
            blur: 8.0,
            spread: 0.0,
            color: Color32::from_black_alpha(90),
        }
        .as_shape(
            Rect::from_center_size(knob, Vec2::splat(r * 2.0)),
            Rounding::same(r),
        ),
    ));
    painter.circle_filled(knob, r, Color32::WHITE);
    if resp.has_focus() {
        painter.circle_stroke(knob, r + 3.0, Stroke::new(2.0_f32, ACCENT_HI));
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.label(
        egui::RichText::new(next.to_string())
            .size(15.0)
            .strong()
            .color(TEXT),
    );
    (next != value).then_some(next)
}

// ------------------------------------------------------------------- globe ---

/// A slowly turning globe with this node at its heart and one light for each
/// peer it is talking to. The positions are made up — a node does not know
/// where its peers are, and this does not pretend to — so the picture is
/// "how many", never "where".
pub fn globe(ui: &mut egui::Ui, height: f32, peers: usize, reachable: usize) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    let t = clock(ui.ctx());
    if motion() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    let c = rect.center() + Vec2::new(0.0, 6.0);
    let r = (height * 0.40).min(width * 0.30);
    // Kept inside the globe's own rectangle by its rim rather than by the
    // clip, so the glow fades out instead of ending in a square.
    painter.add(crate::widgets::radial_shape(
        c,
        r * 1.7,
        with_alpha(ACCENT, 70),
        Some(rect),
    ));

    let grid = with_alpha(ACCENT_HI, 28);
    for i in -2..=2 {
        let lat = i as f32 * 0.5;
        let rr = r * lat.cos();
        let y = c.y - r * lat.sin();
        ellipse(
            &painter,
            egui::pos2(c.x, y),
            rr,
            rr * 0.2,
            Stroke::new(1.0_f32, grid),
        );
    }
    for i in 0..6 {
        let a = t * 0.25 + i as f32 * TAU / 12.0;
        ellipse(
            &painter,
            c,
            (r * a.cos()).abs(),
            r,
            Stroke::new(1.0_f32, grid),
        );
    }

    // Deterministic pseudo-random sphere points.
    let point = |i: usize| {
        let h = (i as u32).wrapping_mul(2_654_435_761).rotate_left(13) ^ 0x9E37_79B9;
        let u = (h & 0xFFFF) as f32 / 65535.0;
        let v = ((h >> 16) & 0xFFFF) as f32 / 65535.0;
        ((u - 0.5) * 2.4, v * TAU)
    };
    let mut drawn: Vec<(Pos2, f32, bool)> = (0..peers.min(160))
        .map(|i| {
            let (lat, lon) = point(i);
            let lon = lon + t * 0.25;
            let x = lat.cos() * lon.sin();
            let y = lat.sin();
            let z = lat.cos() * lon.cos();
            (egui::pos2(c.x + x * r, c.y - y * r), z, i < reachable)
        })
        .collect();
    drawn.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (p, z, _) in &drawn {
        if *z < -0.2 {
            continue;
        }
        let mid = egui::pos2((c.x + p.x) / 2.0, (c.y + p.y) / 2.0 - 30.0);
        let pts: Vec<Pos2> = (0..=16)
            .map(|k| {
                let s = k as f32 / 16.0;
                let a = c.lerp(mid, s);
                let b = mid.lerp(*p, s);
                a.lerp(b, s)
            })
            .collect();
        painter.add(egui::Shape::line(
            pts,
            Stroke::new(
                1.0_f32,
                with_alpha(GRAD_A, (18.0 + z.max(0.0) * 50.0) as u8),
            ),
        ));
    }
    // Travelling pulses along a few of the links.
    if motion() && !drawn.is_empty() {
        for k in 0..4 {
            let idx = ((t * 0.7) as usize + k * 7) % drawn.len();
            let (p, z, _) = drawn[idx];
            if z < 0.0 {
                continue;
            }
            let s = (t * 0.7).fract();
            let mid = egui::pos2((c.x + p.x) / 2.0, (c.y + p.y) / 2.0 - 30.0);
            let at = c.lerp(mid, s).lerp(mid.lerp(p, s), s);
            radial_wash(&painter, at, 9.0, with_alpha(CYAN, 200));
            painter.circle_filled(at, 2.2, CYAN);
        }
    }
    for (p, z, reach) in &drawn {
        let a = 0.3 + z.max(0.0) * 0.7;
        let colour = if *reach { PINK } else { ACCENT_HI };
        let size = if *reach { 4.0 } else { 2.6 } * (0.75 + z.max(0.0) * 0.35);
        painter.circle_filled(*p, size, with_alpha(colour, (a * 255.0) as u8));
    }
    radial_wash(&painter, c, 22.0, with_alpha(GRAD_A, 230));
    painter.circle_filled(c, 5.0, Color32::WHITE);
    painter.text(
        c + Vec2::new(0.0, 26.0),
        egui::Align2::CENTER_CENTER,
        "This node",
        egui::FontId::proportional(12.0),
        TEXT,
    );
}

fn ellipse(painter: &egui::Painter, c: Pos2, rx: f32, ry: f32, stroke: Stroke) {
    let pts: Vec<Pos2> = (0..=64)
        .map(|i| {
            let a = i as f32 / 64.0 * TAU;
            egui::pos2(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(pts, stroke));
}

// ----------------------------------------------------------- slide to send ---

/// "Slide to send": the last gesture before a payment leaves.
///
/// Returns `true` once, on the frame the knob is released past the end. With
/// keyboard focus on the knob, Enter does the same, so the control never
/// becomes a pointer-only door.
pub fn slide_to_confirm(ui: &mut egui::Ui, id: &str, label: &str, enabled: bool) -> bool {
    const H: f32 = 56.0;
    let width = ui.available_width().max(200.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, H), Sense::hover());
    let knob_id = egui::Id::new(("slide", id));
    let travel = rect.width() - H;
    let mut pos: f32 = ui.ctx().data(|d| d.get_temp::<f32>(knob_id)).unwrap_or(0.0);
    let knob_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 4.0 + pos, rect.top() + 4.0),
        Vec2::splat(H - 8.0),
    );
    let resp = ui.interact(
        knob_rect,
        knob_id.with("knob"),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let mut done = false;
    if enabled && resp.dragged() {
        pos = (pos + resp.drag_delta().x).clamp(0.0, travel);
    } else if enabled && resp.drag_stopped() {
        if pos >= travel * 0.9 {
            done = true;
        }
        pos = 0.0;
    } else if enabled
        && resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space))
    {
        done = true;
    }
    if done {
        pos = 0.0;
    }
    ui.ctx().data_mut(|d| d.insert_temp(knob_id, pos));
    let shown = if resp.dragged() {
        pos
    } else {
        spring(ui.ctx(), knob_id.with("spring"), pos)
    };

    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(H / 2.0), glass_inner());
    let fill = Rect::from_min_size(rect.min, Vec2::new(H + shown, H));
    if enabled {
        crate::widgets::gradient_rect(painter, fill, H / 2.0, Vec2::new(1.0, 0.0), |t| {
            hero_gradient(t * 0.6)
        });
    }
    // The label shimmers left to right while it waits.
    let t = clock(ui.ctx());
    let shimmer = (t * 0.6).fract();
    let label_colour = if enabled {
        let k = (1.0 - ((shimmer - 0.5).abs() * 2.0)).powi(2);
        lerp_color(TEXT_DIM, Color32::WHITE, k)
    } else {
        TEXT_FAINT
    };
    if enabled && motion() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    painter.text(
        rect.center() + Vec2::new(H * 0.35, 0.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(15.0),
        with_alpha(
            label_colour,
            ((1.0 - shown / travel.max(1.0)) * 255.0) as u8,
        ),
    );
    let knob = Rect::from_min_size(
        egui::pos2(rect.left() + 4.0 + shown, rect.top() + 4.0),
        Vec2::splat(H - 8.0),
    );
    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: egui::vec2(0.0, 3.0),
            blur: 10.0,
            spread: 0.0,
            color: Color32::from_black_alpha(90),
        }
        .as_shape(knob, Rounding::same(knob.height() / 2.0)),
    ));
    painter.circle_filled(
        knob.center(),
        knob.height() / 2.0,
        if enabled { Color32::WHITE } else { SURFACE_HI },
    );
    let a = knob.center();
    let arrow = Stroke::new(2.4_f32, if enabled { HERO_A } else { TEXT_FAINT });
    painter.line_segment([a - Vec2::new(8.0, 0.0), a + Vec2::new(8.0, 0.0)], arrow);
    painter.add(egui::Shape::line(
        vec![
            a + Vec2::new(2.0, -6.0),
            a + Vec2::new(8.0, 0.0),
            a + Vec2::new(2.0, 6.0),
        ],
        arrow,
    ));
    if resp.has_focus() {
        painter.circle_stroke(
            a,
            knob.height() / 2.0 + 3.0,
            Stroke::new(2.0_f32, ACCENT_HI),
        );
    }
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    done
}

// --------------------------------------------------------------- confetti ---

#[derive(Clone, Copy)]
struct Bit {
    p: Pos2,
    v: Vec2,
    life: f32,
    spin: f32,
    turn: f32,
    colour: Color32,
    star: bool,
    size: f32,
}

/// A burst of stars and paper, for a found block or a payment that left.
#[derive(Default)]
pub struct Confetti {
    bits: Vec<Bit>,
    seed: u32,
}

impl Confetti {
    pub fn burst(&mut self, at: Pos2, colours: &[Color32]) {
        if !motion() || colours.is_empty() {
            return;
        }
        for i in 0..70 {
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let r1 = (self.seed >> 8) as f32 / 16_777_216.0;
            self.seed = self
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let r2 = (self.seed >> 8) as f32 / 16_777_216.0;
            let a = r1 * TAU;
            let speed = 180.0 + r2 * 420.0;
            self.bits.push(Bit {
                p: at,
                v: Vec2::angled(a) * speed - Vec2::new(0.0, 220.0),
                life: 1.0,
                spin: r1 * TAU,
                turn: (r2 - 0.5) * 12.0,
                colour: colours[i % colours.len()],
                star: i % 3 == 0,
                size: 3.0 + r2 * 3.0,
            });
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if self.bits.is_empty() {
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("confetti"),
        ));
        for b in &mut self.bits {
            b.v.y += 900.0 * dt;
            b.v *= 1.0 - 0.9 * dt;
            b.p += b.v * dt;
            b.spin += b.turn * dt;
            b.life -= dt / 1.6;
        }
        self.bits.retain(|b| b.life > 0.0);
        for b in &self.bits {
            let c = with_alpha(b.colour, (b.life.clamp(0.0, 1.0) * 255.0) as u8);
            if b.star {
                let pts: Vec<Pos2> = (0..10)
                    .map(|k| {
                        let rad = if k % 2 == 0 {
                            b.size * 1.7
                        } else {
                            b.size * 0.75
                        };
                        b.p + Vec2::angled(b.spin + k as f32 * TAU / 10.0) * rad
                    })
                    .collect();
                // A star is not convex: fan it from its centre.
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(b.p, c);
                for p in &pts {
                    mesh.colored_vertex(*p, c);
                }
                for k in 0..10_u32 {
                    mesh.add_triangle(0, 1 + k, 1 + (k + 1) % 10);
                }
                painter.add(egui::Shape::mesh(mesh));
            } else {
                let d = Vec2::angled(b.spin);
                let n = d.rot90();
                let (w, h) = (b.size * 0.6, b.size * 1.3);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        b.p - d * h - n * w,
                        b.p + d * h - n * w,
                        b.p + d * h + n * w,
                        b.p - d * h + n * w,
                    ],
                    c,
                    Stroke::NONE,
                ));
            }
        }
        ctx.request_repaint();
    }
}

// ------------------------------------------------------------------ island ---

/// The black pill at the top of the window: the node's state at a glance, and
/// for a few seconds the news when something happens.
pub fn island(
    ctx: &egui::Context,
    centre: Pos2,
    status: &str,
    status_colour: Color32,
    news: Option<(&str, f64)>,
) {
    let now = ctx.input(|i| i.time);
    let showing = news.filter(|(_, at)| now - at < 4.0);
    let open = spring(
        ctx,
        egui::Id::new("island-open"),
        if showing.is_some() { 1.0 } else { 0.0 },
    )
    .clamp(-0.1, 1.15);
    let text = showing.map(|(t, _)| t).unwrap_or(status);
    let font = egui::FontId::proportional(12.5 + open.max(0.0) * 1.5);
    let galley = ctx.fonts(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE));
    let w = (galley.size().x + 44.0).max(180.0 + open * 60.0);
    let h = 28.0 + open * 10.0;
    let rect = Rect::from_center_size(centre + Vec2::new(0.0, open * 4.0), Vec2::new(w, h));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("island"),
    ));
    painter.add(egui::Shape::Rect(
        egui::epaint::Shadow {
            offset: egui::vec2(0.0, 6.0),
            blur: 18.0,
            spread: 0.0,
            color: Color32::from_black_alpha(110),
        }
        .as_shape(rect, Rounding::same(h / 2.0)),
    ));
    painter.rect_filled(rect, Rounding::same(h / 2.0), Color32::from_rgb(12, 9, 20));
    let t = clock(ctx);
    if showing.is_none() {
        let pulse = 0.65 + 0.35 * (t * 2.6).sin().abs();
        let dot = egui::pos2(rect.left() + 16.0, rect.center().y);
        radial_wash(
            &painter,
            dot,
            9.0,
            with_alpha(status_colour, (pulse * 120.0) as u8),
        );
        painter.circle_filled(dot, 3.5, status_colour);
        painter.galley(
            egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
            galley,
            Color32::WHITE,
        );
        if motion() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    } else {
        painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::darkness;

    /// The hero opens with every light on, ends in full night, and the dark
    /// only ever deepens as it falls — a dimming, never a flicker.
    #[test]
    fn night_falls_across_the_whole_hero_and_only_deepens() {
        for step in 0..=20 {
            let u = step as f32 / 20.0;
            assert!(darkness(u, 0.0) < 0.001, "lit at the start ({u})");
            assert!(darkness(u, 1.0) > 0.999, "dark at the end ({u})");
            let mut last = 0.0;
            for p in 0..=100 {
                let d = darkness(u, p as f32 / 100.0);
                assert!(d + 1e-6 >= last, "darkness went back at u={u}, p={p}");
                last = d;
            }
        }
    }
}
