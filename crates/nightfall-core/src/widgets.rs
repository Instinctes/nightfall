//! Reusable UI components.

use crate::theme::*;
use eframe::egui::{self, Color32, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use std::cell::{Cell, RefCell};

/// Inner padding for every text field in the wallet.
///
/// egui's default is 4px horizontally. Against a square input that is merely
/// tight; against ours, which are rounded to `ROUND_SM`, the text starts
/// inside the corner arc and the first character sits on the curve. Every
/// field in the wallet looked as though the text had been pushed against the
/// wall — most visible on the short hints, "nf1…" and "optional".
///
/// 14px clears a 12px radius with room to spare. The vertical 8 gives the
/// caret space above and below rather than letting it touch the border.
///
/// One constant rather than a number typed at nineteen call sites: they had
/// already drifted apart once, and a field that is padded differently from
/// the field under it is the kind of thing nobody can name but everybody
/// sees.
pub const FIELD_MARGIN: egui::Margin = egui::Margin {
    left: 14.0,
    right: 14.0,
    top: 8.0,
    bottom: 8.0,
};

/// Native vector icons: no font-dependent glyphs or bitmap scaling.
pub fn nav_icon(
    painter: &egui::Painter,
    view: crate::app::View,
    origin: egui::Pos2,
    color: Color32,
) {
    use crate::app::View;
    let stroke = Stroke::new(1.6_f32, color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            pts.iter()
                .map(|(x, y)| origin + Vec2::new(*x, *y))
                .collect(),
            stroke,
        ))
    };
    match view {
        View::Dashboard => {
            painter.rect_stroke(
                Rect::from_min_size(origin + Vec2::new(2.0, 4.0), Vec2::new(18.0, 14.0)),
                Rounding::same(3.0),
                stroke,
            );
            line(&[
                (15.0, 10.0),
                (20.0, 10.0),
                (20.0, 14.0),
                (15.0, 14.0),
                (15.0, 10.0),
            ]);
        }
        View::Send => {
            line(&[(4.0, 18.0), (18.0, 4.0), (9.0, 4.0)]);
            line(&[(18.0, 4.0), (18.0, 13.0)]);
        }
        View::Receive => {
            line(&[(18.0, 4.0), (4.0, 18.0), (13.0, 18.0)]);
            line(&[(4.0, 18.0), (4.0, 9.0)]);
        }
        View::Activity => {
            line(&[(2.0, 16.0), (7.0, 10.0), (11.0, 13.0), (19.0, 5.0)]);
        }
        View::Mining => {
            painter.rect_stroke(
                Rect::from_min_size(origin + Vec2::splat(5.0), Vec2::splat(12.0)),
                Rounding::same(2.0),
                stroke,
            );
            for p in [7.0, 11.0, 15.0] {
                line(&[(p, 2.0), (p, 5.0)]);
                line(&[(p, 17.0), (p, 20.0)]);
                line(&[(2.0, p), (5.0, p)]);
                line(&[(17.0, p), (20.0, p)]);
            }
        }
        View::Network => {
            for (x, y) in [(11.0, 3.0), (3.0, 18.0), (19.0, 18.0)] {
                painter.circle_stroke(origin + Vec2::new(x, y), 2.5, stroke);
            }
            line(&[(10.0, 6.0), (4.0, 15.0)]);
            line(&[(12.0, 6.0), (18.0, 15.0)]);
            line(&[(6.0, 18.0), (16.0, 18.0)]);
        }
        View::Settings => {
            for (y, x) in [(5.0, 8.0), (11.0, 15.0), (17.0, 6.0)] {
                line(&[(2.0, y), (x - 2.0, y)]);
                line(&[(x + 2.0, y), (20.0, y)]);
                painter.circle_stroke(origin + Vec2::new(x, y), 2.0, stroke);
            }
        }
    }
}

// ------------------------------------------------------------------ logo ---

/// Decode the bundled logo once and hand back a texture.
///
/// egui has no image loader by default, so the PNG is decoded here and
/// uploaded as a texture on first use. `Context::load_texture` caches by name,
/// but we keep our own handle so the decode happens exactly once per run.
pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    use std::sync::OnceLock;
    static HANDLE: OnceLock<egui::TextureHandle> = OnceLock::new();

    HANDLE
        .get_or_init(|| {
            const BYTES: &[u8] = include_bytes!("../assets/logo.png");
            let image = image::load_from_memory(BYTES)
                .expect("bundled logo is valid PNG")
                .into_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            ctx.load_texture("nightfall-logo", pixels, egui::TextureOptions::LINEAR)
        })
        .clone()
}

/// Draw the logo at the given height.
pub fn logo(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let tex = logo_texture(ui.ctx());
    ui.add(
        egui::Image::new(&tex)
            .fit_to_exact_size(Vec2::splat(height))
            .sense(Sense::hover()),
    )
}

// ------------------------------------------------------------- gradients ---

/// Fill a rounded rectangle with a linear gradient.
///
/// egui has no gradient primitive: `rect_filled` takes a single colour. This
/// builds the rounded outline by hand and fan-triangulates it from the centre,
/// colouring each vertex by its projection onto the gradient axis. The result
/// is a real mesh, so it stays crisp at any DPI.
pub fn gradient_rect(
    painter: &egui::Painter,
    rect: Rect,
    rounding: f32,
    dir: Vec2,
    sample: impl Fn(f32) -> Color32,
) {
    painter.add(gradient_shape(rect, rounding, dir, sample));
}

/// The mesh behind [`gradient_rect`], as a shape rather than a paint call.
///
/// A card cannot paint its own background before it is drawn: its height is
/// only known once the content has been laid out. So the card reserves a slot
/// in the paint list first, lays out its content, and fills the slot in
/// afterwards — which needs the gradient as a value, not as a side effect.
pub fn gradient_shape(
    rect: Rect,
    rounding: f32,
    dir: Vec2,
    sample: impl Fn(f32) -> Color32,
) -> egui::Shape {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return egui::Shape::Noop;
    }
    let r = rounding
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);

    // Rounded-rect outline, walking the four corner arcs.
    const SEG: usize = 8;
    let corners = [
        (
            egui::pos2(rect.min.x + r, rect.min.y + r),
            180.0f32,
            270.0f32,
        ),
        (egui::pos2(rect.max.x - r, rect.min.y + r), 270.0, 360.0),
        (egui::pos2(rect.max.x - r, rect.max.y - r), 0.0, 90.0),
        (egui::pos2(rect.min.x + r, rect.max.y - r), 90.0, 180.0),
    ];
    let mut pts = Vec::with_capacity(4 * (SEG + 1));
    for (c, a0, a1) in corners {
        for i in 0..=SEG {
            let a = (a0 + (a1 - a0) * (i as f32 / SEG as f32)).to_radians();
            pts.push(egui::pos2(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }

    // Project a point onto the gradient axis, normalised to the rect.
    let dir = if dir.length() < f32::EPSILON {
        Vec2::new(0.0, 1.0)
    } else {
        dir.normalized()
    };
    let corners_v = [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ];
    let proj = |p: egui::Pos2| (p - rect.min).dot(dir);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for c in corners_v {
        let v = proj(c);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = (hi - lo).max(f32::EPSILON);
    let color_at = |p: egui::Pos2| sample(((proj(p) - lo) / span).clamp(0.0, 1.0));

    let mut mesh = egui::Mesh::default();
    let centre = rect.center();
    mesh.colored_vertex(centre, color_at(centre));
    for p in &pts {
        mesh.colored_vertex(*p, color_at(*p));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    egui::Shape::mesh(mesh)
}

/// A soft circular light, opaque at the centre and gone at the rim.
///
/// This is the piece the page was missing. egui has no radial gradient and no
/// blur, so it is a triangle fan: one vertex at the centre carrying the
/// colour, a ring of vertices at the radius carrying the same colour at zero
/// alpha. The hardware interpolates between them, which is exactly the falloff
/// a radial gradient would give — and costs one mesh.
///
/// Alpha, not lightness, does the work: the light has to sit under whatever
/// the page background happens to be without knowing what that is.
pub fn radial_wash(painter: &egui::Painter, centre: egui::Pos2, radius: f32, color: Color32) {
    if radius <= 0.0 {
        return;
    }
    const SEG: usize = 64;
    let edge = Color32::from_rgba_premultiplied(0, 0, 0, 0);

    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, color);
    for i in 0..SEG {
        let a = (i as f32 / SEG as f32) * std::f32::consts::TAU;
        mesh.colored_vertex(
            egui::pos2(centre.x + radius * a.cos(), centre.y + radius * a.sin()),
            edge,
        );
    }
    let n = SEG as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Fixed room lighting, sampled as a mesh so it remains smooth at every DPI.
/// The room's colour at a point, `x` and `y` running 0…1 across the window.
///
/// Lives on its own so the contrast test can walk the same field the painter
/// draws. A test that reimplemented this formula would agree with the screen
/// only until somebody edited one of the two copies.
///
/// The lights are roughly half what they were, and that is a legibility
/// number rather than a taste one. At 0.62/0.65/0.55/0.50 with a violet glow
/// of 0.55, the brightest point of the room reached RGB (160, 149, 219); a
/// frosted panel over it left `TEXT_DIM` at 3.8:1 and `TEXT_FAINT` at 3.4:1,
/// so captions, kickers and the block counter were grey on grey.
///
/// These are the brightest lights the contrast budget allows: the room's
/// brightest point is (120, 110, 164), and on the glass over it the weakest
/// text tone still measures 4.95:1. Turn them up and
/// `text_stays_readable_on_every_glass` will say by how much it costs.
pub fn wash_color(x: f32, y: f32) -> Color32 {
    let top_left = tint(WASH_A, PINK, 0.34);
    let top_right = tint(WASH_B, CYAN, 0.36);
    let bottom_left = tint(BG, ACCENT, 0.38);
    let bottom_right = tint(BG, PINK, 0.34);
    let glow = |cx: f32, cy: f32, sx: f32, sy: f32| {
        (-((x - cx) / sx).powi(2) - ((y - cy) / sy).powi(2)).exp()
    };
    let top = lerp_color(top_left, top_right, x);
    let bottom = lerp_color(bottom_left, bottom_right, x);
    let base = lerp_color(top, bottom, y);
    let violet = tint(base, GRAD_A, 0.29 * glow(0.48, 0.02, 0.34, 0.60));
    tint(violet, CYAN, 0.17 * glow(0.30, 0.55, 0.24, 0.23))
}

/// Room lighting behind every panel. Painted on the background layer so a
/// translucent sidebar, top bar and card can frost the same lights, and so
/// those lights do not scroll with the page.
/// How opaque the page is. The desktop is meant to be sensed through it.
///
/// Chosen by measurement rather than taste: with the page this translucent and
/// a pure white wallpaper behind it — the worst case there is — the weakest
/// text tone on the cards still measures 4.54:1, which clears the 4.5:1 small
/// text needs. `text_stays_readable_on_every_glass` composites exactly that
/// case, so turning this down will say what it costs.
pub const PLATE_ALPHA: u8 = 225;

/// The room's gradient as a small texture, so it can be drawn into a shape.
///
/// egui clips to a rectangle and nothing else. The lights used to be a mesh
/// clipped to the plate's *rect*, which painted straight over the rounded
/// corners and squared them off — leaving the rim stroke to trace a curve that
/// nothing else followed. That is why the corners still looked square after
/// the window itself had been rounded for days.
///
/// A gradient is exactly what a small texture is good at: 64×64 stretched
/// across the window is smoother than the 24×16 mesh it replaces, and a
/// textured `RectShape` takes a rounding, so the shape is right by
/// construction rather than by clipping.
fn wash_texture(ctx: &egui::Context) -> egui::TextureHandle {
    const N: usize = 64;
    let id = egui::Id::new("nightfall-wash-texture");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle;
    }
    let last = (N - 1) as f32;
    let mut pixels = Vec::with_capacity(N * N);
    for row in 0..N {
        for col in 0..N {
            pixels.push(wash_color(col as f32 / last, row as f32 / last));
        }
    }
    let handle = ctx.load_texture(
        "nightfall-wash",
        egui::ColorImage {
            size: [N, N],
            pixels,
        },
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

pub fn paint_room(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let plate = window_plate(ctx);
    // One shape: the room's gradient, drawn into a rounded rectangle at the
    // page's own opacity. Rounded because a `RectShape` takes a rounding,
    // rather than because something was clipped afterwards.
    //
    // No shadow of our own under it either. macOS derives the window's drop
    // shadow from the alpha we paint and draws it *outside* the frame, where
    // there is room; ours had to fit inside a 16-point margin, so a 28-point
    // blur was cut off by the window edge and what remained was its darkest
    // part — a ring of roughly nine percent black hugging the corner.
    let wash = wash_texture(ctx);
    painter.add(egui::Shape::Rect(egui::epaint::RectShape {
        rect: plate,
        rounding: Rounding::same(WINDOW_ROUND),
        // A white tint leaves the texture's own colours alone; the alpha is
        // what lets the desktop through.
        fill: with_alpha(Color32::WHITE, PLATE_ALPHA),
        stroke: Stroke::NONE,
        blur_width: 0.0,
        fill_texture_id: wash.id(),
        uv: Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
    }));
    // A hairline of caught light around the edge, so the sheet has a rim
    // rather than simply stopping.
    painter.rect_stroke(
        plate,
        Rounding::same(WINDOW_ROUND),
        Stroke::new(1.0_f32, with_alpha(BORDER_HI, 90)),
    );
}

/// How far the glass sheet sits inside the window it is drawn in.
///
/// Zero: the window is titled again, so macOS rounds its own corners and casts
/// its own shadow outside the frame. A margin of ours would only show as a
/// transparent band around a window that is already the right shape.
pub const WINDOW_INSET: f32 = 0.0;

/// The window's own corner radius. Larger than a card's, as in the reference.
pub const WINDOW_ROUND: f32 = 30.0;

/// How far the floating rail stands from the window's left edge.
///
/// Small on purpose: the rail is a sheet lying *on* the page, and the closer
/// it sits to the edge the more it reads as standing proud of it rather than
/// being embedded in a margin.
pub const RAIL_LEFT: f32 = 6.0;

/// How far the content sheet is pushed right of the window's own left edge.
///
/// Small, and only on the left. It is the strip the rail hangs over: with the
/// plate flush to the window the rail had nothing to stand proud of and read
/// as embedded in a margin. The window's own buttons are moved right by the
/// same amount in `place_window_buttons`, so they stay on the glass rather
/// than floating beside it in the gap.
pub const PLATE_LEFT: f32 = 40.0;

/// The rounded sheet the page lives on. The rail is deliberately not on it.
pub fn window_plate(ctx: &egui::Context) -> Rect {
    let r = ctx.screen_rect().shrink(WINDOW_INSET);
    Rect::from_min_max(egui::pos2(r.left() + PLATE_LEFT, r.top()), r.max)
}

/// Nudge the window's own buttons in from the corner.
///
/// macOS draws the close, minimise and zoom buttons itself and places them at
/// a fixed offset inside the window frame. With the title bar hidden they land
/// hard in the top-left corner of the glass, tighter than the rest of the
/// layout breathes. eframe exposes no way to ask, so the three views are moved
/// directly.
///
/// Applied once per window size, never once per frame. An offset added on
/// every frame is added *again* every frame, and the buttons would walk off
/// the window within a second. AppKit re-lays them out when the window
/// resizes, so the trigger is the size changing: read where macOS has just put
/// them, move them once, leave them alone until the next resize.
#[cfg(target_os = "macos")]
pub fn place_window_buttons(ctx: &egui::Context) {
    use objc2_app_kit::{NSApplication, NSWindowButton};
    use objc2_foundation::{MainThreadMarker, NSPoint};

    const IN_X: f64 = PLATE_LEFT as f64 + 9.0;
    const DOWN_Y: f64 = 7.0;

    let size = ctx.screen_rect().size();
    let key = (size.x.round() as i32, size.y.round() as i32);
    let id = egui::Id::new("window-buttons-placed-for");
    if ctx.data(|d| d.get_temp::<(i32, i32)>(id)) == Some(key) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    ctx.data_mut(|d| d.insert_temp(id, key));
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        for kind in [
            NSWindowButton::NSWindowCloseButton,
            NSWindowButton::NSWindowMiniaturizeButton,
            NSWindowButton::NSWindowZoomButton,
        ] {
            let Some(button) = window.standardWindowButton(kind) else {
                continue;
            };
            // AppKit measures from the bottom left, so moving a button down
            // the screen means subtracting from y.
            let origin = button.frame().origin;
            unsafe { button.setFrameOrigin(NSPoint::new(origin.x + IN_X, origin.y - DOWN_Y)) };
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn place_window_buttons(_ctx: &egui::Context) {}

/// Claim the margin around the plate so every panel lands *on* the plate.
///
/// Panels measure themselves against the whole window, so without this the
/// lock screen's header bar started at the window's left edge and hung out
/// over the desktop, past the rounded corner. Four transparent gutters take
/// the margin first; everything drawn afterwards sees only the plate.
pub fn plate_gutters(ctx: &egui::Context) {
    let clear = || egui::Frame::none().fill(Color32::TRANSPARENT);
    egui::SidePanel::left("plate-gutter-left")
        .exact_width(WINDOW_INSET + PLATE_LEFT)
        .resizable(false)
        .show_separator_line(false)
        .frame(clear())
        .show(ctx, |_| {});
    egui::SidePanel::right("plate-gutter-right")
        .exact_width(WINDOW_INSET)
        .resizable(false)
        .show_separator_line(false)
        .frame(clear())
        .show(ctx, |_| {});
    egui::TopBottomPanel::top("plate-gutter-top")
        .exact_height(WINDOW_INSET)
        .resizable(false)
        .show_separator_line(false)
        .frame(clear())
        .show(ctx, |_| {});
    egui::TopBottomPanel::bottom("plate-gutter-bottom")
        .exact_height(WINDOW_INSET)
        .resizable(false)
        .show_separator_line(false)
        .frame(clear())
        .show(ctx, |_| {});
}

/// A card whose background is the brand gradient. Used for the balance hero.
pub fn gradient_card<R>(
    ui: &mut egui::Ui,
    height: f32,
    flash: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().add(egui::Shape::Rect(
        glass_card_shadow().as_shape(rect, Rounding::same(ROUND)),
    ));
    gradient_rect(
        ui.painter(),
        rect,
        ROUND,
        Vec2::new(1.0, 0.55),
        brand_gradient,
    );
    gradient_rect(ui.painter(), rect, ROUND, Vec2::new(0.0, 1.0), |t| {
        with_alpha(TEXT, ((1.0 - t).powf(2.2) * 28.0) as u8)
    });
    if flash > 0.02 {
        radial_wash(
            ui.painter(),
            rect.center(),
            rect.width().min(rect.height()) * 0.72,
            with_alpha(PINK, (flash * 90.0) as u8),
        );
        radial_wash(
            ui.painter(),
            egui::pos2(rect.left() + rect.width() * 0.22, rect.center().y),
            rect.height() * 0.85,
            with_alpha(CYAN, (flash * 70.0) as u8),
        );
    }
    // Contour detail stays on the right, behind content and clipped to the hero.
    let painter = ui.painter().with_clip_rect(rect.shrink(12.0));
    for line in 0..10 {
        let points = (0..60)
            .map(|i| {
                let x = i as f32 / 59.0;
                egui::pos2(
                    rect.right() - rect.width() * 0.36 + x * rect.width() * 0.44,
                    rect.bottom() - 12.0 - line as f32 * 8.0 - (x * 5.0).sin() * 20.0,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.0_f32, with_alpha(INK, 22)),
        ));
    }

    let inner = rect.shrink(22.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    add(&mut child)
}

/// Test-only: the outer rect of every outermost [`card`] drawn this frame.
///
/// A card's edges are the one thing a screenshot shows plainly and a layout
/// test could not see: the existing tests assert that nothing overflows the
/// page, which a card that is 50 points narrower than its neighbour passes
/// without complaint. Recording the rects lets a test say the thing the eye
/// says — these two cards do not line up.
///
/// Only the outermost card is recorded. A card nested inside another is
/// meant to be inset, so its edges are not the page's edges.
#[cfg(test)]
pub(crate) mod card_probe {
    use eframe::egui::Rect;
    use std::cell::RefCell;

    thread_local! {
        static PENDING: RefCell<Option<String>> = const { RefCell::new(None) };
        static OPEN: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
        static DRAWN: RefCell<Vec<(Rect, String)>> = const { RefCell::new(Vec::new()) };
    }

    /// The title `titled_card` is about to draw, so a failure can name the card
    /// instead of making somebody count cards down a page.
    pub fn name(title: &str) {
        PENDING.with(|p| *p.borrow_mut() = Some(title.to_string()));
    }

    pub fn enter() {
        let title = PENDING.with(|p| p.borrow_mut().take());
        OPEN.with(|s| s.borrow_mut().push(title));
    }

    pub fn leave(rect: Rect) {
        let title = OPEN.with(|s| s.borrow_mut().pop().flatten());
        if OPEN.with(|s| s.borrow().is_empty()) {
            DRAWN.with(|d| {
                d.borrow_mut()
                    .push((rect, title.unwrap_or_else(|| "untitled".into())))
            });
        }
    }

    /// The cards since the last call, and reset. Call once to discard a warm-up
    /// frame, then again to read the frame you mean to measure.
    pub fn take() -> Vec<(Rect, String)> {
        DRAWN.with(|d| std::mem::take(&mut *d.borrow_mut()))
    }
}

thread_local! {
    static CARD_DEPTH: Cell<u32> = const { Cell::new(0) };
    static CARD_FLOOR: Cell<f32> = const { Cell::new(0.0) };
    static ROW_HEIGHTS: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// A frosted panel: translucent Nightfall fill, luminous rim, lit top edge.
///
/// The page wash has to show through or this is just a rounder opaque card.
/// Notices and banners stay opaque (`tint`) so content never prints through a
/// warning. A 24-point blur still will not fit the 16-point column gap.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let outer = ui.available_width();
    let depth = CARD_DEPTH.get();
    CARD_DEPTH.set(depth + 1);
    let floor = if depth == 0 { CARD_FLOOR.get() } else { 0.0 };
    #[cfg(test)]
    card_probe::enter();
    let drawn = egui::Frame::none()
        .fill(glass_surface())
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND))
        .inner_margin(egui::Margin::same(22.0))
        .shadow(glass_card_shadow())
        .show(ui, |ui| {
            fill_width(ui, (outer - 44.0).max(0.0));
            let y0 = ui.cursor().top();
            let out = add(ui);
            // The card's own content, before it is padded out to match its
            // neighbour. This is what gets reported upwards, and reporting the
            // *padded* height instead is what made a card that was once too
            // tall stay too tall for good: the padding confirmed the floor that
            // produced it, so the row could only ever grow. Dashboard ended up
            // with two cards of metrics stretched down the whole window.
            let natural = ui.cursor().top() - y0 + 44.0;
            if floor > 1.0 {
                let used = ui.cursor().top() - y0;
                let inner = (floor - 44.0).max(0.0);
                ui.add_space((inner - used).max(0.0));
            }
            (out, natural)
        });
    let (out, natural) = drawn.inner;
    if depth == 0 {
        ROW_HEIGHTS.with(|h| h.borrow_mut().push(natural));
    }
    CARD_DEPTH.set(depth);
    #[cfg(test)]
    card_probe::leave(drawn.response.rect);
    out
}

/// One row of a data list: label left, value hard right, hairline under.
///
/// Copied from the phone wallet's NETWORK card, which is the clearest thing
/// in either wallet: every row the same height, values on one right edge,
/// separated by a line rather than by guesswork. `kv` put the value in a
/// fixed 150px column, so long values wrapped in the middle of the card and
/// short ones left a hole.
pub fn data_row(ui: &mut egui::Ui, key: &str, value: RichText, last: bool) {
    // Row rhythm taken from the phone: label at reading size rather than
    // caption size, and enough air that the hairlines separate groups instead
    // of crowding them. Tighter than this and the card reads as a table.
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(14.0).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(14.0));
        });
    });
    ui.add_space(10.0);
    if !last {
        hairline(ui);
    }
}

/// A one-pixel rule at the current position, no padding of its own.
pub fn hairline(ui: &mut egui::Ui) {
    ui.add_space(1.0);
}

/// Pin this UI to `width` so a Frame cannot shrink to its text.
///
/// Without this, a scan warning sizes to its title and a card sizes to the
/// panel, and the two right edges miss each other. Every page surface has
/// to claim the same number.
pub fn fill_width(ui: &mut egui::Ui, width: f32) {
    ui.set_width(width.max(0.0));
}

/// One centred column for banners *and* the page, sharing a single width.
///
/// Glance pages pass `f32::INFINITY` and fill the panel. Form pages pass a
/// cap (Send, Settings) so a text field is not 1400 px wide. Capping
/// the page alone, with the scan warning still full-bleed, is what made
/// Settings look inset and off-centre.
pub fn page_column<R>(
    ui: &mut egui::Ui,
    max_width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let avail = ui.available_width();
    let width = avail.min(max_width);
    let height = ui.available_height();
    let mut out = None;
    // Own the full panel first so the inner column can be centred in it.
    // A horizontal `add_space` + allocate pair shrinks to content and
    // leaves a dead strip down the right — that was the Dashboard bug.
    ui.allocate_ui_with_layout(
        Vec2::new(avail, height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    fill_width(ui, width);
                    out = Some(add(ui));
                },
            );
        },
    );
    out.expect("page_column body always runs")
}

/// Two columns side by side, stacked when the window is too narrow for them.
///
/// `ui.columns` gives each cell its own top-down layout of equal width.
/// Putting `allocate_ui` inside a horizontal layout is what staggered
/// cards diagonally, and wrapping the *hero* in a column is what left
/// the spendable card half-width with a dead strip on the right.
pub fn two_columns<A, B>(
    ui: &mut egui::Ui,
    min_column: f32,
    left: impl FnOnce(&mut egui::Ui) -> A,
    right: impl FnOnce(&mut egui::Ui) -> B,
) {
    let avail = ui.available_width();
    if avail < min_column * 2.0 + GAP_LG {
        left(ui);
        ui.add_space(GAP_LG);
        right(ui);
        return;
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = GAP_LG;
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            fill_width(&mut cols[0], w0);
            left(&mut cols[0]);
            let w1 = cols[1].available_width();
            fill_width(&mut cols[1], w1);
            right(&mut cols[1]);
        });
    });
}

/// Two cards on one row, stretched to the same height so they read as a grid.
///
/// `two_columns` cannot do this: Receive stacks two cards in the right column,
/// and stretching every sheet would leave a half-width card on its own row.
pub fn paired_cards<A, B>(
    ui: &mut egui::Ui,
    key: &'static str,
    min_column: f32,
    left: impl FnOnce(&mut egui::Ui) -> A,
    right: impl FnOnce(&mut egui::Ui) -> B,
) {
    let avail = ui.available_width();
    if avail < min_column * 2.0 + GAP_LG {
        left(ui);
        ui.add_space(GAP_LG);
        right(ui);
        return;
    }
    // Each row needs its own memory, and it has to be the *same* memory every
    // frame. `ui.id().with("pair-h")` was shared by every pair on a page, so
    // the rows fought over one height. `ui.auto_id_with` fixed that and
    // introduced a subtler version of it: an auto id counts widgets, so a
    // banner appearing above these cards renumbers the rows and hands one of
    // them the height that belonged to the other. On Dashboard the other row
    // is Activity, which is tall. The caller names the row instead.
    let id = egui::Id::new(("pair-h", key));
    // The height belongs to the width it was measured at, and is discarded when
    // that width changes. A remembered height applied at a *different* width is
    // how the window flickered when the scan banner appeared: the banner made
    // the page taller, the scrollbar took a few points of width, the text
    // rewrapped to a new natural height, that height was stored, the padding it
    // produced changed the page height back, and the whole thing oscillated
    // once per frame. Padding a row to a height measured for some other width
    // is not a stale value to tolerate — it is an answer to a different
    // question.
    let measured_at = (ui.available_width() * 4.0).round() as i32;
    let floor = ui
        .ctx()
        .data(|d| d.get_temp::<(i32, f32)>(id))
        .filter(|(w, _)| *w == measured_at)
        .map(|(_, h)| h)
        .unwrap_or(0.0);
    ROW_HEIGHTS.with(|h| h.borrow_mut().clear());
    CARD_FLOOR.set(floor);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = GAP_LG;
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            fill_width(&mut cols[0], w0);
            left(&mut cols[0]);
            let w1 = cols[1].available_width();
            fill_width(&mut cols[1], w1);
            right(&mut cols[1]);
        });
    });
    CARD_FLOOR.set(0.0);
    let row_h = ROW_HEIGHTS.with(|h| h.borrow().iter().copied().fold(0.0_f32, f32::max));
    if row_h > 1.0 && (row_h - floor).abs() > 1.0 {
        ui.ctx()
            .data_mut(|d| d.insert_temp(id, (measured_at, row_h)));
    }
}

/// A horizontal rule for separating groups inside a card.
///
/// The web wallet separates rows with a hairline rather than with empty
/// space, and
/// that is most of why its cards read as organised while ours read as a list
/// of things that happen to be near each other.
pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(24.0);
}

/// A quiet section heading. Keep the real text intact for assistive readers.
pub fn kicker(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(13.0).color(TEXT).strong());
}

/// A full-width notice: one colour, one title, one body, optional actions.
///
/// Dashboard and Activity each drew this frame by hand. The fills drifted
/// (0.10 vs 0.12) and so did the title size, so the same kind of fact —
/// "something needs you" — did not look like the same kind of fact.
pub fn status_banner(
    ui: &mut egui::Ui,
    color: Color32,
    title: &str,
    body: &str,
    pulse: bool,
    actions: impl FnOnce(&mut egui::Ui),
) {
    // Outer width is captured *before* the frame's inner margin, so a
    // banner and a card on the same page share one right edge. Measuring
    // inside the frame and subtracting the margin again made banners 32 px
    // narrower than the hero.
    let outer = ui.available_width();
    egui::Frame::none()
        .fill(glass_alert(color))
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            fill_width(ui, (outer - 32.0).max(0.0));
            ui.horizontal(|ui| {
                dot(ui, color, pulse);
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(14.0).color(color).strong());
            });
            if !body.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(body).size(13.0).color(TEXT));
            }
            actions(ui);
        });
}

/// A quiet placeholder when a list has nothing to show.
///
/// A filter box plus a blank card is what made empty pages look unfinished.
/// The title is the state; the hint is the next action, in one sentence.
pub fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(15.0).color(TEXT));
        ui.add_space(4.0);
        ui.label(RichText::new(hint).size(13.0).color(TEXT_DIM));
    });
    ui.add_space(12.0);
}

/// A card with a title row.
///
/// The title uses [`kicker`], so every tab picks up the web wallet's
/// heading rhythm from one place instead of eighteen call sites.
pub fn titled_card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    #[cfg(test)]
    card_probe::name(title);
    card(ui, |ui| {
        kicker(ui, title);
        ui.add_space(10.0);
        add(ui)
    })
}

/// A slightly recessed block inside a card: read-only summaries and totals.
///
/// Flat, like everything else. It reads as inset because it is darker than
/// the card it sits in, which is the only cue the phone wallet uses and the
/// only one needed.
pub fn well<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let outer = ui.available_width();
    egui::Frame::none()
        .fill(glass_inner())
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(16.0, 14.0))
        .show(ui, |ui| {
            fill_width(ui, (outer - 32.0).max(0.0));
            add(ui)
        })
        .inner
}

/// A form field's label, with an optional note pushed to the right.
///
/// Every field had its own hand-rolled label row, and they had drifted: three
/// different sizes and two different colours across one form.
pub fn field_label(ui: &mut egui::Ui, label: &str, note: Option<RichText>) {
    ui.horizontal(|ui| {
        // Same size and colour as the label `text_field` draws, so a form
        // built from both does not have two kinds of field label in it.
        ui.label(RichText::new(label).size(12.5).color(TEXT_DIM));
        if let Some(note) = note {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(note);
            });
        }
    });
    ui.add_space(5.0);
}

/// A capped-width column, centred in whatever space it is given.
///
/// Used on lock and onboarding — a single form in a full window. The main
/// pages fill the central panel instead: capping them left a dead strip down
/// the right side of the window.
pub fn narrow_column<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = width.min(ui.available_width());
    let mut out = None;
    ui.vertical_centered(|ui| {
        fill_width(ui, width);
        out = Some(add(ui));
    });
    out.expect("narrow_column body always runs")
}

/// A fixed-height line for a validation message under a field.
///
/// The height is reserved whether or not there is anything to say. A slot
/// that collapses when valid means the fields below jump up and down as you
/// type — and the field most affected is the amount, where a jump can move
/// the thing you were about to click.
///
/// The first version reserved the space with a label containing a single
/// space. That works, and it leaves a line of text's worth of padding on
/// every side, which is how the Amount card ended up with sixty pixels of
/// nothing in the middle of it.
pub fn message_slot(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 17.0), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    add(&mut child);
}

/// One line of a summary: label left, value hard right.
///
/// Right-aligned and tabular so the digits of a total line up under the digits
/// of the amount above it. A column of numbers that does not line up is a
/// column you have to read twice.
pub fn summary_row(ui: &mut egui::Ui, key: &str, value: RichText, strong: bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(key)
                .size(12.0)
                .color(if strong { TEXT } else { TEXT_DIM }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.monospace());
        });
    });
}

/// Small label above a value.
pub fn metric_grid(ui: &mut egui::Ui, cells: &[(&str, String, Color32)], separate_cards: bool) {
    // Four across from 640 points, not 880. A strip of four figures is one
    // reading; folding it into two rows of two turns it into two readings and
    // leaves half the card empty — which is what Activity's totals did on a
    // perfectly ordinary window.
    // A half-width dashboard column is typically 400–700 points. Four figures
    // across that is a squeezed row, not a grid. Keep 2×2 in a column; four
    // across only on a full-width card.
    let columns = if ui.available_width() >= 780.0 {
        4
    } else if ui.available_width() >= 280.0 {
        2
    } else {
        1
    };
    for (row, chunk) in cells.chunks(columns).enumerate() {
        if row > 0 {
            ui.add_space(12.0);
        }
        ui.columns(columns, |cols| {
            for (i, (label, value, color)) in chunk.iter().enumerate() {
                if separate_cards {
                    card(&mut cols[i], |ui| stat(ui, label, value, *color));
                } else {
                    stat(&mut cols[i], label, value, *color);
                }
            }
        });
    }
}

pub fn stat(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.set_min_height(46.0);
        ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
        ui.add_space(2.0);
        ui.label(
            RichText::new(value)
                .size(17.0)
                .color(color)
                .strong()
                .monospace(),
        );
    });
}

/// Key/value row used in detail lists.
/// Key on the left, value on the right edge, hairline between rows.
///
/// This is the phone wallet's list row, and changing `kv` itself rather than
/// its callers is deliberate: thirty-six call sites across seven tabs pick up
/// the style at once, and none of them can drift away from it later.
///
/// The old version put the value in a fixed 150px column. Long values then
/// wrapped in the middle of the card while short ones left a gap between key
/// and value wide enough that the eye stopped connecting them.
///
/// The separator is drawn ABOVE the row, not below, and suppressed for the
/// first row in its container. That way a list never ends with a stray line
/// hanging above the card's bottom padding — which is what a
/// separator-below version does, and it reads as a table someone forgot to
/// finish. `is_first` is decided by whether anything has been laid out in
/// this Ui yet, so callers do not have to count their own rows.
pub fn kv(ui: &mut egui::Ui, key: &str, value: RichText) {
    let is_first = ui.min_rect().height() <= f32::EPSILON;
    if !is_first {
        ui.add_space(9.0);
        hairline(ui);
    }
    ui.add_space(9.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(13.5).color(TEXT_DIM));
        // The value gets the room that is left, and is truncated inside it.
        //
        // It used to be a right-to-left layout with no width of its own, so a
        // value longer than the space grew *leftwards over the key* and drew
        // on top of it. On the Settings page the Bitcoin config path did
        // exactly that: "Config" and "/Users/hux/…" were painted in the same
        // place, one on top of the other, and the result read as corrupted
        // glyphs. A long value now stops at the key instead of climbing over
        // it — and if it does not fit, the card is the wrong shape for it and
        // `copyable` is the right widget.
        // Asked for after the key is drawn, so egui's own accounting — which
        // includes the spacing it inserts between the two — decides how much
        // is left. Measuring the key by hand and subtracting a guessed gap is
        // how the first attempt at this overflowed the Dashboard by 32 points.
        let room = ui.available_width().max(60.0);
        ui.allocate_ui_with_layout(
            Vec2::new(room, 0.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.set_width(room);
                ui.add(egui::Label::new(value.size(13.5)).truncate());
            },
        );
    });
}

/// A coloured pill.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(11.0), color);
    let size = galley.size() + Vec2::new(16.0, 7.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        Rounding::same(999.0),
        color.gamma_multiply(0.14),
        Stroke::new(1.0_f32, color.gamma_multiply(0.45)),
    );
    let text_pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(text_pos, galley, color);
    resp
}

/// A small live dot, optionally pulsing.
pub fn dot(ui: &mut egui::Ui, color: Color32, animate: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let t = if animate {
        ui.ctx().request_repaint();
        let phase = ui.input(|i| i.time) as f32 * 2.0;
        0.55 + 0.45 * (phase.sin() * 0.5 + 0.5)
    } else {
        1.0
    };
    ui.painter()
        .circle_filled(rect.center(), 4.0, color.gamma_multiply(t));
}

/// Primary action button. Colour is reserved for the page's main action.
pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    primary_button_width(ui, text, enabled, None)
}

/// As [`primary_button`], with a width the caller insists on.
///
/// `button_row` allocates a shared width and then asks for a primary button;
/// without this the button sized itself from its own label and came out
/// narrower than the row had agreed, so "Send" at 75 points sat beside
/// "Receive" at 92 — the exact raggedness the row exists to remove.
pub fn primary_button_width(
    ui: &mut egui::Ui,
    text: &str,
    enabled: bool,
    exact: Option<f32>,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let fg = if enabled { INK } else { TEXT_FAINT };
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(14.0), fg);
    // One height for every button in the product. This was 42 while
    // `ghost_button` was 38, so any row holding one of each — the lock screen,
    // for one — sat four pixels out of line and read as unfinished.
    //
    // The width is clamped to the room available, because a button sized
    // purely from its own label ignores the box it was put in: `button_row`
    // allocated a shared width and this happily drew past it, which is how a
    // clamped row still overflowed a 320-point card.
    let size = Vec2::new(
        exact.unwrap_or_else(|| (galley.size().x + 44.0).min(ui.available_width().max(72.0))),
        CONTROL_H,
    );
    let (rect, resp) = ui
        .add_enabled_ui(enabled, |ui| ui.allocate_exact_size(size, Sense::click()))
        .inner;
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));

    if enabled {
        let hot = resp.hovered();
        gradient_rect(ui.painter(), rect, ROUND_SM, Vec2::new(1.0, 0.0), |t| {
            let c = brand_gradient(t * 0.7);
            if hot {
                lerp_color(c, Color32::WHITE, 0.07)
            } else {
                c
            }
        });
        // Glass buttons: fill only. A 1px top stroke read as a white hairline.
    } else {
        ui.painter()
            .rect_filled(rect, Rounding::same(ROUND_SM), SURFACE_HI);
    }

    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            Rounding::same(ROUND_SM),
            Stroke::new(2.0_f32, ACCENT_HI),
        );
    }
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, fg);

    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A chip for use on top of the gradient card.
///
/// Dark fill, not translucent white: white-on-white over a light gradient stop
/// is unreadable, which is exactly how the first version shipped.
#[allow(dead_code)]
pub fn on_gradient_chip(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.5),
        Color32::WHITE,
    );
    let size = galley.size() + Vec2::new(22.0, 10.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        Rounding::same(ROUND_PILL),
        INK,
        Stroke::new(1.0_f32, Color32::from_white_alpha(45)),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// Secondary / outline button.
/// How wide [`ghost_button`] will draw for this label.
///
/// A row that puts fields beside a button has to know the button's room
/// before it can size the fields. Guessing it is how the Address book row
/// came to overflow its card by 21 points — and an overflowing card widens
/// the page under it, so every card below inherited the wrong width. Asked
/// of the same style the button uses, so it cannot drift.
pub fn ghost_button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, TEXT));
    galley.size().x + ui.spacing().button_padding.x * 2.0
}

pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(TEXT))
            .fill(glass_inner())
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(ROUND_SM))
            .min_size(Vec2::new(0.0, CONTROL_H)),
    )
}

/// A row of buttons that share one width.
///
/// Two buttons side by side whose widths come from their own labels look like
/// two unrelated controls that happen to be adjacent — "Unlock wallet" at 127
/// points beside "Clear password fields" at 161 was the clearest example. When
/// the choices belong together, they get the same box.
///
/// Returns the index of the button pressed, if any. The first is the primary.
pub fn button_row(ui: &mut egui::Ui, labels: &[&str], enabled: bool) -> Option<usize> {
    let mut clicked = None;
    let widest = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_string(), egui::FontId::proportional(14.0), TEXT)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    // …but never wider than the space there is. A shared width taken purely
    // from the longest label pushed a two-button row off a 320-point card,
    // which is a narrower window than this wallet supports but exactly the
    // width the layout tests check — and they were right to.
    let spacing = ui.spacing().item_spacing.x;
    let count = labels.len().max(1) as f32;
    let room = (ui.available_width() - spacing * (count - 1.0)) / count;
    let width = (widest + 44.0).min(room.max(72.0));
    ui.horizontal(|ui| {
        for (index, label) in labels.iter().enumerate() {
            let pressed = if index == 0 {
                primary_button_width(ui, label, enabled, Some(width)).clicked()
            } else {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(RichText::new(*label).color(TEXT))
                        .fill(glass_inner())
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_SM))
                        // `min_size` is a floor, not a ceiling: a long label
                        // grew the button straight past the width this row
                        // had agreed on. Truncating holds the row together.
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .min_size(Vec2::new(width, CONTROL_H)),
                )
                .clicked()
            };
            if pressed {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// A labelled text field.
///
/// Every input in the wallet used to be a bare capsule carrying its own
/// question as placeholder text — which disappears the moment anyone types, so
/// a half-filled form is a column of anonymous boxes and the only way to find
/// out what one is for is to empty it. The label stays.
///
/// `hint` is then free to do the job placeholders are actually good at:
/// showing the shape of a valid answer rather than repeating the label.
pub fn text_field(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    hint: &str,
    value: &mut String,
    password: bool,
) -> egui::Response {
    field_label(ui, label, None);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .id_salt(id)
            .password(password)
            .char_limit(4096)
            .desired_width(ui.available_width())
            .margin(egui::Margin::symmetric(12.0, 9.0))
            .hint_text(RichText::new(hint).color(TEXT_FAINT)),
    );
    ui.add_space(GAP_SM);
    response
}

/// A two- or three-way switch: one row, one decision, one highlighted answer.
///
/// egui's `selectable_label` draws the unselected option with no box at all,
/// so a pair of them reads as one button beside a stray piece of text. Both
/// halves of a switch have to look like halves of a switch.
///
/// Returns the index pressed, if any.
pub fn segmented(ui: &mut egui::Ui, id: &str, labels: &[&str], selected: usize) -> Option<usize> {
    let font = egui::FontId::proportional(12.5);
    let widest = labels
        .iter()
        .map(|l| {
            ui.fonts(|f| f.layout_no_wrap((*l).to_string(), font.clone(), TEXT))
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    const PAD: f32 = 4.0;
    let count = labels.len().max(1) as f32;
    let room = ((ui.available_width() - PAD * 2.0) / count).max(56.0);
    let seg_w = (widest + 34.0).min(room);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(seg_w * count + PAD * 2.0, CONTROL_H),
        Sense::hover(),
    );
    ui.painter().rect(
        rect,
        Rounding::same(ROUND_SM),
        glass_surface(),
        Stroke::NONE,
    );
    let mut clicked = None;
    for (index, label) in labels.iter().enumerate() {
        let seg = Rect::from_min_size(
            egui::pos2(rect.left() + PAD + seg_w * index as f32, rect.top() + PAD),
            Vec2::new(seg_w, rect.height() - PAD * 2.0),
        );
        let resp = ui.interact(seg, egui::Id::new(("segmented", id, index)), Sense::click());
        let on = index == selected;
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                on,
                *label,
            )
        });
        if on {
            // The chosen half carries the accent. At SURFACE_HI on SURFACE_LOW
            // the difference was one step of grey, and which way round a swap
            // runs is not something to leave to a careful look.
            ui.painter().rect(
                seg,
                Rounding::same((ROUND_SM - PAD).max(0.0)),
                glass_inner(),
                Stroke::NONE,
            );
        } else if resp.hovered() {
            ui.painter().rect_filled(
                seg,
                Rounding::same((ROUND_SM - PAD).max(0.0)),
                glass_hover(),
            );
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                seg,
                Rounding::same(ROUND_SM - PAD),
                Stroke::new(2.0_f32, ACCENT_HI),
            );
        }
        ui.painter().text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if on { TEXT } else { TEXT_DIM },
        );
        if resp.hovered() && !on {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}

/// A full-width choice: a label, and one line saying what it is for.
///
/// For a set of alternatives that are one decision. They share a width so the
/// set reads as a set, and each carries its own explanation so the difference
/// between them does not have to be inferred from the verb.
pub fn choice_button(ui: &mut egui::Ui, label: &str, note: &str, primary: bool) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 58.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hot = resp.hovered();
    let fill = if primary {
        if hot {
            lerp_color(GRAD_A, Color32::WHITE, 0.08)
        } else {
            GRAD_A
        }
    } else if hot {
        SURFACE_HOVER
    } else {
        SURFACE_HI
    };
    ui.painter()
        .rect(rect, Rounding::same(ROUND_SM), fill, Stroke::NONE);
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            Rounding::same(ROUND_SM),
            Stroke::new(2.0_f32, ACCENT_HI),
        );
    }
    let (title_colour, note_colour) = if primary {
        (INK, Color32::from_rgb(0x4A, 0x3A, 0x6E))
    } else {
        (TEXT, TEXT_FAINT)
    };
    let left = rect.left() + 18.0;
    ui.painter().text(
        egui::pos2(left, rect.top() + 17.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.5),
        title_colour,
    );
    ui.painter().text(
        egui::pos2(left, rect.top() + 39.0),
        egui::Align2::LEFT_CENTER,
        note,
        egui::FontId::proportional(11.5),
        note_colour,
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// The bar every full-window screen starts with.
///
/// The lock, migration, onboarding and scan screens each used to open with a
/// bare `ui.label("NIGHTFALL · devnet")` in the top-left corner and then a
/// single card floating in an otherwise empty window — more than half the
/// surface carrying nothing. A window that is mostly nothing reads as a
/// program that has not finished loading.
///
/// This gives those screens the same frame the main window has: the mark, the
/// network, and one line of state on the right that is true whether or not a
/// wallet is open. It is deliberately not a card — it is the window's edge.
pub fn screen_header(ui: &mut egui::Ui, network: &str, right: &[(&str, Color32)]) {
    egui::Frame::none()
        .fill(glass_rail())
        .stroke(Stroke::NONE)
        .inner_margin(egui::Margin::symmetric(GAP_LG, GAP_SM + 2.0))
        .rounding(Rounding::same(ROUND))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                logo(ui, 18.0);
                ui.add_space(GAP_SM);
                ui.label(
                    RichText::new("NIGHTFALL")
                        .size(13.0)
                        .color(TEXT)
                        .extra_letter_spacing(1.4),
                );
                ui.add_space(GAP_XS);
                badge(ui, network, CYAN);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (text, colour) in right.iter().rev() {
                        ui.label(RichText::new(*text).size(11.5).color(*colour));
                        ui.add_space(GAP_MD);
                    }
                });
            });
        });
}

/// A tinted block inside a card: a caution that belongs to the card.
///
/// A frame with no width of its own shrink-wraps its text, so a note under a
/// full-width list ends short of it and the card appears to have two right
/// edges — which is what the About card did. The width is taken outside the
/// frame, where the margins have not been added yet.
pub fn inset_note<R>(ui: &mut egui::Ui, tone: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    const PAD: f32 = 12.0;
    let inner = (ui.available_width() - PAD * 2.0).max(60.0);
    egui::Frame::none()
        .fill(glass_alert(tone))
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::same(PAD))
        .show(ui, |ui| {
            ui.set_width(inner);
            add(ui)
        })
        .inner
}

/// A checkbox the wallet draws itself. Returns true when it was just changed.
///
/// egui's own is a hairline square that reads as an empty rectangle on a dark
/// surface. Settings has three of them and one gates an irreversible
/// migration, so "is that ticked?" has to be answerable at a glance. Filled
/// and accented when on, clearly outlined when off, and the label is part of
/// the hit area rather than a separate piece of text beside it.
pub fn check(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    const BOX: f32 = 17.0;
    const GAP: f32 = 10.0;
    let width = ui.available_width();
    let galley = ui.fonts(|f| {
        f.layout(
            label.to_string(),
            egui::FontId::proportional(13.0),
            TEXT,
            (width - BOX - GAP).max(40.0),
        )
    });
    let height = galley.size().y.max(BOX);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let enabled = ui.is_enabled();
    let hot = resp.hovered() && enabled;
    let square = Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + (height - BOX) / 2.0),
        Vec2::splat(BOX),
    );
    let (fill, stroke) = match (*on, hot) {
        (true, _) => (GRAD_A, GRAD_A),
        (false, true) => (SURFACE_HOVER, ACCENT_HI),
        (false, false) => (SURFACE_LOW, with_alpha(TEXT, 35)),
    };
    ui.painter().rect(
        square,
        Rounding::same(5.0),
        if enabled { fill } else { SURFACE_LOW },
        Stroke::new(
            1.0_f32,
            if enabled {
                stroke
            } else {
                with_alpha(TEXT, 25)
            },
        ),
    );
    if *on {
        // A tick drawn as two strokes, not a glyph: the bundled font has no
        // check mark, and a missing glyph is a box — which is exactly the
        // thing an unticked checkbox already looks like.
        let c = square.center();
        ui.painter().add(egui::Shape::line(
            vec![
                egui::pos2(c.x - 4.0, c.y),
                egui::pos2(c.x - 1.0, c.y + 3.2),
                egui::pos2(c.x + 4.2, c.y - 3.4),
            ],
            Stroke::new(2.0_f32, INK),
        ));
    }
    ui.painter().galley(
        egui::pos2(
            square.right() + GAP,
            rect.top() + (height - galley.size().y) / 2.0,
        ),
        galley,
        if enabled { TEXT } else { TEXT_FAINT },
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, label)
    });
    if resp.clicked() && enabled {
        *on = !*on;
        return true;
    }
    false
}

/// A monospace value with a copy button. Returns true when copied.
pub fn copyable(ui: &mut egui::Ui, value: &str, wrap: bool) -> bool {
    let mut copied = false;
    // Measured outside the frame, because inside it `available_width` does not
    // account for the margins the frame is about to add back around whatever
    // is drawn. Taking the inner width from the inner `ui` made the block
    // twelve points wider than the card holding it — which only showed up on
    // the narrowest window, and only once something long was put in one.
    // 24 for the margins, and a few more for the stroke and the rounding —
    // measured against the narrowest supported window rather than derived,
    // because egui charges for a border in more places than one.
    let inner = (ui.available_width() - 30.0).max(80.0);
    egui::Frame::none()
        .fill(glass_inner())
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 8.0;
                // The copy button's room, asked of the style rather than
                // guessed: a guess that is two points short overflows the card
                // the moment the value column claims its full width.
                let icon =
                    ui.fonts(|f| {
                        f.layout_no_wrap("Copy".to_string(), egui::FontId::proportional(12.0), TEXT)
                    })
                    .size()
                    .x + ui.spacing().button_padding.x * 2.0;
                let avail = (inner - icon - 8.0).max(40.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(avail, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        // Claim the column even when the value is short. An
                        // `allocate_ui` advances by what its child used, so a
                        // 15-character path made this box shrink-wrap itself
                        // and two of them in one page were two widths — which
                        // is most of what "the cards are different widths"
                        // turned out to look like on Settings.
                        ui.set_width(avail);
                        let mut text = RichText::new(value).monospace().color(TEXT);
                        if !wrap {
                            text = text.size(12.0);
                        }
                        let label = egui::Label::new(text);
                        ui.add(if wrap { label.wrap() } else { label.truncate() });
                    },
                );
                if ui
                    .add(
                        egui::Button::new(RichText::new("Copy").size(12.0))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                    )
                    .on_hover_text("Copy")
                    .clicked()
                {
                    ui.ctx().copy_text(value.to_string());
                    copied = true;
                }
            });
        });
    copied
}

/// Horizontal progress bar with a label.
pub fn progress(ui: &mut egui::Ui, fraction: f32, label: &str) {
    let f = fraction.clamp(0.0, 1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::same(4.0), SURFACE_LOW);
    if f > 0.0 {
        let filled = Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height()));
        gradient_rect(ui.painter(), filled, 4.0, Vec2::new(1.0, 0.0), |t| {
            brand_gradient(t * 0.6)
        });
    }
    if !label.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new(label).size(11.0).color(TEXT_DIM));
    }
}

/// Render a QR code as crisp rectangles. No image decoding involved.
pub fn qr_code(ui: &mut egui::Ui, data: &str, size: f32) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        ui.label(RichText::new("QR unavailable").color(TEXT_DIM));
        return;
    };
    let width = code.width();
    let quiet = 4usize;
    let modules = width + quiet * 2;
    let scale = size / modules as f32;

    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), Color32::WHITE);

    let colors = code.to_colors();
    for y in 0..width {
        for x in 0..width {
            if colors[y * width + x] == qrcode::Color::Dark {
                let min =
                    rect.min + Vec2::new((x + quiet) as f32 * scale, (y + quiet) as f32 * scale);
                painter.rect_filled(
                    Rect::from_min_size(min, Vec2::splat(scale.ceil())),
                    Rounding::ZERO,
                    Color32::BLACK,
                );
            }
        }
    }
}

/// Transient notification.
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub color: Color32,
    pub created: f64,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn push(&mut self, ctx: &egui::Context, text: impl Into<String>, color: Color32) {
        self.items.push(Toast {
            text: text.into(),
            color,
            created: ctx.input(|i| i.time),
        });
    }

    pub fn success(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.push(ctx, text, SUCCESS);
    }

    pub fn error(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.push(ctx, text, DANGER);
    }

    pub fn info(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.push(ctx, text, ACCENT_HI);
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        const LIFETIME: f64 = 4.0;
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| now - t.created < LIFETIME);
        if self.items.is_empty() {
            return;
        }
        ctx.request_repaint();

        let screen = ctx.screen_rect();
        let mut y = screen.max.y - 24.0;

        for toast in self.items.iter().rev() {
            let age = now - toast.created;
            let fade = if age > LIFETIME - 0.6 {
                ((LIFETIME - age) / 0.6).clamp(0.0, 1.0) as f32
            } else {
                1.0
            };
            let appear = ((age / 0.28).clamp(0.0, 1.0) as f32).powf(0.65);
            let alpha = fade * appear;
            let y_off = (1.0 - appear) * 18.0;

            let id = egui::Id::new(("toast", toast.created.to_bits()));
            egui::Area::new(id)
                .fixed_pos(egui::pos2(screen.max.x - 24.0, y + y_off))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    egui::Frame::none()
                        // Toasts overlap page text: use opaque frost so the
                        // message never merges with the content underneath.
                        .fill(tint(SURFACE, toast.color, 0.14))
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_SM))
                        .inner_margin(egui::Margin::symmetric(14.0, 11.0))
                        .shadow(egui::epaint::Shadow {
                            offset: egui::vec2(0.0, 4.0),
                            blur: 16.0,
                            spread: 0.0,
                            color: Color32::from_black_alpha(120),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.painter().circle_filled(
                                    ui.cursor().min + Vec2::new(3.0, 8.0),
                                    3.5,
                                    toast.color,
                                );
                                ui.add_space(12.0);
                                ui.label(RichText::new(&toast.text).color(TEXT));
                            });
                        });
                });
            y -= 52.0;
        }
    }
}

/// Format a hashrate into human units.
pub fn format_hashrate(h: f64) -> String {
    if h >= 1e9 {
        format!("{:.2} GH/s", h / 1e9)
    } else if h >= 1e6 {
        format!("{:.2} MH/s", h / 1e6)
    } else if h >= 1e3 {
        format!("{:.2} kH/s", h / 1e3)
    } else {
        format!("{h:.0} H/s")
    }
}

/// Format a large integer with thousands separators.
///
/// Plain ASCII comma on purpose: egui's bundled font has no narrow no-break
/// space, and a missing glyph renders as a tofu box.
pub fn format_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shorten a hex string for display.
pub fn short_hex(h: &str) -> String {
    if h.len() <= 20 {
        h.to_string()
    } else {
        format!("{}…{}", &h[..10], &h[h.len() - 6..])
    }
}

/// Relative time such as "3 min ago".
pub fn ago(ts: u64, now: u64) -> String {
    if ts == 0 {
        return "—".into();
    }
    let d = now.saturating_sub(ts);
    match d {
        0..=59 => format!("{d}s ago"),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} d ago", d / 86400),
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    fn click_primary(enabled: bool, parent_enabled: bool) -> bool {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let mut rect = Rect::NOTHING;
        let mut clicked = false;
        for phase in 0..3 {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(700.0, 400.0),
                )),
                ..Default::default()
            };
            if phase > 0 {
                input.events.push(egui::Event::PointerMoved(rect.center()));
                input.events.push(egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: phase == 1,
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(parent_enabled, |ui| {
                        let response = primary_button(ui, "Confirm", enabled);
                        rect = response.rect;
                        clicked |= response.clicked();
                    });
                });
            });
        }
        clicked
    }

    #[test]
    fn disabled_primary_never_dispatches_a_click() {
        assert!(click_primary(true, true));
        assert!(!click_primary(false, true));
        assert!(!click_primary(true, false));
    }

    #[test]
    fn narrow_column_pads_both_sides_equally() {
        for width in [620.0, 900.0, 1400.0, 1800.0] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 700.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        let outer = ui.max_rect();
                        let mut inner = Rect::NOTHING;
                        narrow_column(ui, 620.0, |ui| {
                            inner = ui.max_rect();
                            ui.label("x");
                        });
                        let left = inner.left() - outer.left();
                        let right = outer.right() - inner.right();
                        assert!(
                            (left - right).abs() <= 2.0,
                            "at {width}px left pad {left} != right pad {right}"
                        );
                        let expected = width.min(620.0);
                        assert!(
                            (inner.width() - expected).abs() <= 2.0,
                            "inner {} at window {width}, expected {expected}",
                            inner.width()
                        );
                    });
            });
        }
    }

    #[test]
    fn two_columns_fit_the_allocated_width() {
        for width in [660.0, 900.0, 1180.0] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 700.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right_edge = ui.max_rect().right();
                    two_columns(
                        ui,
                        280.0,
                        |ui| {
                            card(ui, |ui| {
                                ui.label("Left");
                            });
                        },
                        |ui| {
                            card(ui, |ui| {
                                ui.label("Right");
                            });
                            assert!(ui.min_rect().right() <= right_edge + 1.0);
                        },
                    );
                });
            });
        }
    }

    #[test]
    fn two_columns_are_equal_width() {
        for width in [700.0, 900.0, 1180.0] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 700.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        let mut left_w = 0.0;
                        let mut right_w = 0.0;
                        two_columns(
                            ui,
                            280.0,
                            |ui| {
                                left_w = ui.available_width();
                                ui.label("L");
                            },
                            |ui| {
                                right_w = ui.available_width();
                                ui.label("R");
                            },
                        );
                        assert!(
                            (left_w - right_w).abs() <= 2.0,
                            "at {width}px left {left_w} != right {right_w}"
                        );
                    });
            });
        }
    }

    #[test]
    fn two_columns_share_one_row_height() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(900.0, 700.0),
            )),
            ..Default::default()
        };
        for _ in 0..3 {
            let _ = crate::widgets::card_probe::take();
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        paired_cards(
                            ui,
                            "pair-a",
                            280.0,
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("short");
                                });
                            },
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("tall");
                                    ui.label("second line");
                                    ui.label("third line");
                                    ui.label("fourth line");
                                });
                            },
                        );
                    });
            });
        }
        let cards = crate::widgets::card_probe::take();
        assert_eq!(cards.len(), 2, "{cards:?}");
        let dh = (cards[0].0.height() - cards[1].0.height()).abs();
        assert!(
            dh <= 2.0,
            "row heights {} vs {}",
            cards[0].0.height(),
            cards[1].0.height()
        );
        assert!((cards[0].0.top() - cards[1].0.top()).abs() <= 2.0);
    }

    #[test]
    fn two_paired_rows_keep_separate_heights() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(900.0, 900.0),
            )),
            ..Default::default()
        };
        for _ in 0..4 {
            let _ = crate::widgets::card_probe::take();
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        paired_cards(
                            ui,
                            "pair-b",
                            280.0,
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("a");
                                });
                            },
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("b");
                                });
                            },
                        );
                        ui.add_space(24.0);
                        paired_cards(
                            ui,
                            "pair-c",
                            280.0,
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("c");
                                    ui.label("c2");
                                    ui.label("c3");
                                    ui.label("c4");
                                    ui.label("c5");
                                });
                            },
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("d");
                                    ui.label("d2");
                                    ui.label("d3");
                                    ui.label("d4");
                                    ui.label("d5");
                                });
                            },
                        );
                    });
            });
        }
        let cards = crate::widgets::card_probe::take();
        assert_eq!(cards.len(), 4, "{cards:?}");
        let h = |i: usize| cards[i].0.height();
        assert!((h(0) - h(1)).abs() <= 2.0, "row1 {} vs {}", h(0), h(1));
        assert!((h(2) - h(3)).abs() <= 2.0, "row2 {} vs {}", h(2), h(3));
        assert!(
            h(2) > h(0) + 20.0,
            "rows must not share one floor: short {} tall {}",
            h(0),
            h(2)
        );
    }

    /// A row must be able to get shorter again, and a banner above it must not
    /// hand it somebody else's height.
    ///
    /// Both halves of this are the Dashboard bug reported on 19 September: the
    /// node and hashrate cards ran the full height of the window with a field
    /// of empty glass under four numbers. The row height was measured *after*
    /// the shorter card had been padded out to match the taller one, so the
    /// measurement only ever confirmed the padding — once a row was too tall it
    /// stayed too tall forever, and the value persisted in egui memory. The
    /// height it got in the first place came from the row being identified by a
    /// widget counter, which a conditional banner renumbers, so Dashboard's
    /// metrics row inherited the height of its Activity row.
    #[test]
    fn a_row_shrinks_back_and_keeps_its_own_height_when_a_banner_appears() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(900.0, 1200.0),
            )),
            ..Default::default()
        };
        // `lines` drives the first row's content; `banner` adds a widget above
        // it, which is what used to change the row's identity.
        let frame = |lines: usize, banner: bool| {
            let _ = crate::widgets::card_probe::take();
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        if banner {
                            ui.label("wallet scan incomplete");
                        }
                        paired_cards(
                            ui,
                            "metrics",
                            280.0,
                            |ui| {
                                card(ui, |ui| {
                                    for i in 0..lines {
                                        ui.label(format!("metric {i}"));
                                    }
                                });
                            },
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("hashrate");
                                });
                            },
                        );
                        ui.add_space(24.0);
                        paired_cards(
                            ui,
                            "activity",
                            280.0,
                            |ui| {
                                card(ui, |ui| {
                                    for i in 0..14 {
                                        ui.label(format!("row {i}"));
                                    }
                                });
                            },
                            |ui| {
                                card(ui, |ui| {
                                    ui.label("peers");
                                });
                            },
                        );
                    });
            });
            let cards = crate::widgets::card_probe::take();
            assert_eq!(cards.len(), 4, "{cards:?}");
            (cards[0].0.height(), cards[2].0.height())
        };

        // Settle on tall content, then shrink it.
        for _ in 0..3 {
            frame(14, false);
        }
        let (tall, _) = frame(14, false);
        for _ in 0..3 {
            frame(2, false);
        }
        let (short, activity) = frame(2, false);
        assert!(
            short < tall - 20.0,
            "a row that lost content must lose height: was {tall}, still {short}",
        );

        // The banner appears. The metrics row must keep its own height rather
        // than adopting the tall Activity row's.
        for _ in 0..3 {
            frame(2, true);
        }
        let (with_banner, _) = frame(2, true);
        assert!(
            (with_banner - short).abs() <= 2.0,
            "a banner above the row changed its height: {short} -> {with_banner}",
        );
        assert!(
            with_banner < activity - 20.0,
            "the metrics row inherited the activity row's height: {with_banner} vs {activity}",
        );
    }

    #[test]
    fn page_column_centres_a_capped_width() {
        for width in [900.0, 1400.0, 1800.0] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 700.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        let outer = ui.max_rect();
                        let mut inner = Rect::NOTHING;
                        page_column(ui, 720.0, |ui| {
                            inner = ui.max_rect();
                            ui.label("x");
                        });
                        let left = inner.left() - outer.left();
                        let right = outer.right() - inner.right();
                        assert!(
                            (left - right).abs() <= 2.0,
                            "at {width}px left pad {left} != right pad {right}"
                        );
                        assert!(
                            (inner.width() - 720.0).abs() <= 2.0,
                            "inner {} at window {width}",
                            inner.width()
                        );
                    });
            });
        }
    }

    #[test]
    fn banner_and_card_share_the_outer_edge() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(900.0, 700.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    let mut banner = Rect::NOTHING;
                    let mut panel = Rect::NOTHING;
                    ui.scope(|ui| {
                        status_banner(ui, WARN, "Title", "Body", false, |_| {});
                        banner = ui.min_rect();
                    });
                    ui.scope(|ui| {
                        card(ui, |ui| {
                            ui.label("inside");
                        });
                        panel = ui.min_rect();
                    });
                    assert!(
                        (banner.left() - panel.left()).abs() <= 2.0,
                        "left banner {} card {}",
                        banner.left(),
                        panel.left()
                    );
                    assert!(
                        (banner.right() - panel.right()).abs() <= 2.0,
                        "right banner {} card {}",
                        banner.right(),
                        panel.right()
                    );
                });
        });
    }

    #[test]
    fn two_columns_stack_when_the_window_is_narrow() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(500.0, 700.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| {
                    let mut left_w = 0.0;
                    let mut right_w = 0.0;
                    two_columns(
                        ui,
                        320.0,
                        |ui| {
                            left_w = ui.available_width();
                            ui.label("L");
                        },
                        |ui| {
                            right_w = ui.available_width();
                            ui.label("R");
                        },
                    );
                    // Stacked: each child sees the full panel, not a half.
                    assert!(
                        left_w > 400.0 && right_w > 400.0,
                        "stacked columns should fill 500px, got {left_w} / {right_w}"
                    );
                });
        });
    }
}
