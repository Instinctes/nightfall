//! Reusable UI components.

use crate::theme::*;
use eframe::egui::{self, Color32, Rect, RichText, Rounding, Sense, Stroke, Vec2};

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
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
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
    painter.add(egui::Shape::mesh(mesh));
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

/// A bordered surface panel.
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

/// A card with a title row.
pub fn titled_card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card(ui, |ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .color(TEXT_FAINT)
                .strong(),
        );
        ui.add_space(10.0);
        add(ui)
    })
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
pub fn kv(ui: &mut egui::Ui, key: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [150.0, 20.0],
            egui::Label::new(RichText::new(key).color(TEXT_DIM)).selectable(false),
        );
        ui.label(value);
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
