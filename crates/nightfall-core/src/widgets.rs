//! Reusable UI components.

use crate::theme::*;
use eframe::egui::{self, Color32, Rect, RichText, Rounding, Sense, Stroke, Vec2};

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
    const SEG: usize = 48;
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

/// The two lights across the top of the page.
///
/// Call once per frame, before anything else is drawn. The radii are tied to
/// the window width so the effect looks the same in a small window as in a
/// maximised one — a fixed radius turns into a visible blob when the window
/// is narrow.
pub fn page_wash(painter: &egui::Painter, rect: Rect) {
    let w = rect.width();
    radial_wash(
        painter,
        egui::pos2(rect.min.x + w * 0.12, rect.min.y - w * 0.10),
        w * 0.62,
        WASH_A,
    );
    radial_wash(
        painter,
        egui::pos2(rect.min.x + w * 0.92, rect.min.y + w * 0.02),
        w * 0.48,
        WASH_B,
    );
}

/// A card whose background is the brand gradient. Used for the balance hero.
pub fn gradient_card<R>(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    gradient_rect(
        ui.painter(),
        rect,
        ROUND,
        Vec2::new(1.0, 0.55),
        brand_gradient,
    );

    let inner = rect.shrink(22.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    add(&mut child)
}

/// A surface panel: body that falls off downwards, border, lit top edge.
///
/// Reserving two paint slots before the content and filling them afterwards
/// is the only way round the ordering problem — a card's height depends on
/// what is inside it, and the background has to be underneath. `Shape::Noop`
/// holds the place; `Painter::set` replaces it once the rect is known.
///
/// The three pieces are each nearly invisible alone. Together they are the
/// difference between a panel and a rectangle of a slightly different colour.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    egui::Frame::none()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(ROUND))
        .inner_margin(egui::Margin::same(20.0))
        .show(ui, |ui| {
            ui.set_width(width - 40.0);
            add(ui)
        })
        .inner
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
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0_f32, BORDER.gamma_multiply(0.75)),
    );
}

/// Two columns side by side, stacked when the window is too narrow for them.
///
/// The pages that were not dashboards each drew one narrow card against the
/// left edge and left two thirds of the window empty. Splitting the content
/// in two fills the width when there is width, and folds back to a single
/// column when there is not, rather than squeezing two unreadable columns
/// into a small window.
pub fn two_columns<A, B>(
    ui: &mut egui::Ui,
    min_column: f32,
    left: impl FnOnce(&mut egui::Ui) -> A,
    right: impl FnOnce(&mut egui::Ui) -> B,
) {
    const GAP: f32 = 16.0;
    let avail = ui.available_width();
    if avail < min_column * 2.0 + GAP {
        left(ui);
        ui.add_space(14.0);
        right(ui);
        return;
    }
    let col = (avail - GAP) / 2.0;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(col, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.set_width(col);
                left(ui);
            },
        );
        ui.add_space(GAP);
        ui.allocate_ui_with_layout(
            Vec2::new(col, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.set_width(col);
                right(ui);
            },
        );
    });
}

/// A horizontal rule for separating groups inside a card.
///
/// The web wallet separates rows with a hairline rather than with empty
/// space, and
/// that is most of why its cards read as organised while ours read as a list
/// of things that happen to be near each other.
pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(12.0);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0_f32, BORDER.gamma_multiply(0.8)),
    );
    ui.add_space(12.0);
}

/// Small, spaced, uppercase — the web wallet's section heading.
///
/// egui has no letter-spacing, so the gaps are inserted between characters
/// with a thin space. It is a hack, and it is the difference between a
/// heading that looks designed and one that looks like bold text.
pub fn kicker(ui: &mut egui::Ui, text: &str) {
    // An ordinary space, and a step brighter than TEXT_FAINT.
    //
    // The tracking is wider than it was because the phone's headings are
    // wider; at thin-space tracking the desktop ones read as small bold text
    // rather than as labels. It is a plain U+0020 and not one of the typographic
    // spaces on purpose: U+2005 gives exactly the width wanted and is missing
    // from the bundled font, so every heading in the wallet rendered as
    // "S□A□F□E□ T□O□ R□E□U□S□E". A space that is not in the font is not a
    // space, it is a box.
    let spaced: String = text.to_uppercase().chars().flat_map(|c| [c, ' ']).collect();
    ui.label(
        RichText::new(spaced.trim_end())
            .size(11.0)
            .color(TEXT_FAINT.gamma_multiply(1.35))
            .strong(),
    );
}

/// A bordered chip that sits on the page rather than inside a coloured
/// wash, so it reads as information and not as an alarm.
pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(11.0), color);
    let size = galley.size() + Vec2::new(22.0, 9.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        Rounding::same(ROUND_PILL),
        BG,
        Stroke::new(1.0_f32, color.gamma_multiply(0.4)),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// A card with a title row.
///
/// The title uses [`kicker`], so every tab picks up the web wallet's
/// heading rhythm from one place instead of eighteen call sites.
pub fn titled_card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
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
    let width = ui.available_width();
    egui::Frame::none()
        .fill(SURFACE_LOW)
        .stroke(Stroke::new(1.0_f32, BORDER.gamma_multiply(0.9)))
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(16.0, 14.0))
        .show(ui, |ui| {
            ui.set_width(width - 32.0);
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
        ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
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
/// A form should not be as wide as a dashboard — a line of input 900px long
/// is hard to scan and harder to fill in. But capping the width with
/// `set_max_width` alone leaves the column pinned to the left edge with a
/// third of the window empty beside it, which reads as a page that failed to
/// load rather than as a deliberately narrow form.
pub fn narrow_column<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    let pad = ((avail - width) / 2.0).max(0.0);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui_with_layout(
            Vec2::new(width.min(avail), 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.set_width(width.min(avail));
                out = Some(add(ui));
            },
        );
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
pub fn stat(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.set_min_height(46.0);
        ui.label(RichText::new(label).size(11.0).color(TEXT_FAINT));
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(13.5));
        });
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

/// Primary action button — a gradient pill.
pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(14.0),
        Color32::WHITE,
    );
    let size = Vec2::new(galley.size().x + 44.0, 42.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());

    if enabled {
        let hot = resp.hovered();
        gradient_rect(ui.painter(), rect, ROUND_PILL, Vec2::new(1.0, 0.0), |t| {
            let c = brand_gradient(t * 0.7);
            if hot {
                c
            } else {
                c.gamma_multiply(0.88)
            }
        });
    } else {
        ui.painter()
            .rect_filled(rect, Rounding::same(ROUND_PILL), SURFACE_HI);
    }

    let fg = if enabled { Color32::WHITE } else { TEXT_FAINT };
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
        Color32::from_black_alpha(85),
        Stroke::new(1.0_f32, Color32::from_white_alpha(45)),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// Secondary / outline button.
pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(TEXT))
            .fill(SURFACE_HI)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(ROUND_PILL))
            .min_size(Vec2::new(0.0, 38.0)),
    )
}

/// A monospace value with a copy button. Returns true when copied.
pub fn copyable(ui: &mut egui::Ui, value: &str, wrap: bool) -> bool {
    let mut copied = false;
    egui::Frame::none()
        .fill(SURFACE_LOW)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(ROUND_SM))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 8.0;
                let avail = ui.available_width() - 40.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(avail, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
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
                        egui::Button::new(RichText::new("⧉").size(15.0))
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
    let quiet = 2usize;
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
            let alpha = if age > LIFETIME - 0.6 {
                ((LIFETIME - age) / 0.6).clamp(0.0, 1.0) as f32
            } else {
                1.0
            };

            let id = egui::Id::new(("toast", toast.created.to_bits()));
            egui::Area::new(id)
                .fixed_pos(egui::pos2(screen.max.x - 24.0, y))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    egui::Frame::none()
                        .fill(SURFACE_HI)
                        .stroke(Stroke::new(1.0_f32, toast.color.gamma_multiply(0.6)))
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
