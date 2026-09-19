//! Dev-only: walk every page in the real window and save a picture of each.
//!
//! Screen capture on this machine fails for days at a time, and a wallet's
//! looks cannot be checked by reading code — the two worst layout faults in
//! this project were both invisible in source and obvious on screen. This
//! draws each page with the real renderer, scrolls it a screen at a time,
//! and writes a numbered PNG of each screen: `settings-0.png`, `-1`, `-2`.
//!
//! Requires a development build and `NIGHTFALL_UI_SHOTS` naming a directory.
//! It only reads the window it is already drawing: no
//! wallet file is opened, written or migrated by anything here.

use crate::app::{App, View};
use eframe::egui::{self, ViewportCommand};
use std::path::PathBuf;

/// Frames to let a page settle before the shutter. A page drawn for the first
/// time lays out its text on the frame after it appears, and a scroll offset
/// applied this frame is only honoured on the next one.
const SETTLE: u32 = 8;

/// A page taller than this many screens is a page nobody scrolls to the end
/// of; the cap stops a runaway from filling the disk.
const MAX_SCREENS: u32 = 10;

/// What `App::update` reports back about the scroll area it has just drawn.
#[derive(Clone, Copy)]
pub struct Area {
    /// Where the scrolling part of the page sits in the window, in points.
    pub viewport: egui::Rect,
    /// How tall the page's content is, in points.
    pub content: f32,
    /// The offset the scroll area actually used — which is not the offset
    /// asked for once the page runs out of content to scroll.
    pub offset: f32,
}

type CapturePage = (View, &'static str, Option<(&'static str, usize)>);

pub struct Shots {
    dir: PathBuf,
    /// Pages left to capture, in reverse so `pop` walks them in order.
    todo: Vec<CapturePage>,
    page: Option<(View, &'static str)>,
    /// The scroll offset the page is being drawn at for the tile in hand.
    asked: f32,
    /// A shutter is open and its image has not come back yet.
    ///
    /// The image arrives one frame after the command, and every frame in
    /// between still satisfies "settled" — so without this the shutter fired
    /// twice and the spare image was consumed by the *next* page, which is
    /// how one page came to be photographed showing the one before it.
    waiting: bool,
    settle: u32,
    area: Option<Area>,
    /// Screens written for the page in hand, which is also their number.
    taken: u32,
}

impl Shots {
    /// `None` unless both the development build and environment opt in.
    pub fn from_env() -> Option<Self> {
        if !crate::app::IS_DEV_BUILD {
            return None;
        }
        let dir = PathBuf::from(std::env::var_os("NIGHTFALL_UI_SHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        let mut todo: Vec<_> = View::ALL
            .into_iter()
            .map(|(v, name)| (v, name, None))
            .collect();
        todo.extend([
            (View::Send, "Air", Some(("send", 1))),
            (View::Receive, "Counter", Some(("receive", 1))),
            (View::Activity, "Proof", Some(("activity", 1))),
            (View::Settings, "Recovery", Some(("settings", 1))),
            (View::Settings, "Contacts", Some(("settings", 2))),
            (View::Settings, "Node-settings", Some(("settings", 3))),
            (View::Settings, "About", Some(("settings", 4))),
        ]);
        todo.reverse();
        Some(Self {
            dir,
            todo,
            page: None,
            asked: 0.0,
            waiting: false,
            settle: SETTLE,
            area: None,
            taken: 0,
        })
    }

    /// The scroll offset the page should be drawn at this frame.
    pub fn offset(&self) -> Option<f32> {
        self.page.map(|_| self.asked)
    }

    /// Called by `App::update` once the scroll area has been drawn.
    pub fn note(&mut self, area: Area) {
        self.area = Some(area);
    }
}

/// One step of the walk. Call at the end of `App::update`.
pub fn step(app: &mut App, ctx: &egui::Context) {
    if app.shots.is_none() {
        return;
    }
    // Nothing here waits on user input, so nothing here can rely on the
    // window being repainted by one.
    ctx.request_repaint();

    let arrived = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.as_ref().clone()),
            _ => None,
        })
    });

    let Some(shots) = app.shots.as_mut() else {
        return;
    };

    // An image nobody is waiting for is a leftover, and belongs to no page.
    if let Some(image) = arrived.filter(|_| shots.waiting) {
        shots.waiting = false;
        let at = shots.area.map_or(shots.asked, |a| a.offset);
        if let Some((_, name)) = shots.page {
            let path = shots
                .dir
                .join(format!("{}-{}.png", name.to_lowercase(), shots.taken));
            write_screen(&path, &image);
        }
        shots.taken += 1;
        // One file per screenful, and no stitching. Sewing the screens into
        // one tall page needed the scroll offset to be exact in both
        // directions; where it was not, a row of the About card fell into a
        // seam and simply was not in the picture — the one failure a picture
        // must not have. A numbered screen cannot lose a row, and reading the
        // one screen that matters is cheaper than reading a whole page.
        let (view_h, content) = shots
            .area
            .map_or((0.0, 0.0), |a| (a.viewport.height(), a.content));
        let next = at + view_h;
        if view_h > 1.0 && next < content - 1.0 && shots.taken < MAX_SCREENS {
            shots.asked = next;
            shots.settle = SETTLE;
        } else {
            shots.page = None;
            shots.area = None;
        }
        return;
    }

    // Start the next page.
    if shots.page.is_none() {
        shots.waiting = false;
        match shots.todo.pop() {
            Some(next) => {
                app.view = next.0;
                if let Some((key, selected)) = next.2 {
                    ctx.data_mut(|data| {
                        data.insert_temp(egui::Id::new(("page-workspace", key)), selected)
                    });
                }
                let Some(shots) = app.shots.as_mut() else {
                    return;
                };
                shots.page = Some((next.0, next.1));
                shots.asked = 0.0;
                shots.settle = SETTLE;
                shots.area = None;
                shots.taken = 0;
            }
            None => {
                app.shots = None;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
        return;
    }

    if shots.settle > 0 || shots.waiting {
        shots.settle = shots.settle.saturating_sub(1);
        return;
    }

    shots.waiting = true;
    ctx.send_viewport_cmd(ViewportCommand::Screenshot);
}

/// Write one screenful exactly as it was drawn.
fn write_screen(path: &PathBuf, image: &egui::ColorImage) {
    let (w, h) = (image.size[0] as u32, image.size[1] as u32);
    // Alpha is kept. The window is transparent outside its rounded plate, and
    // writing 255 there would claim the desktop is black — a picture that
    // quietly misstates what it shows is the one thing this module must not
    // do.
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for p in &image.pixels {
        buf.extend_from_slice(&[p.r(), p.g(), p.b(), p.a()]);
    }
    match image::RgbaImage::from_raw(w, h, buf) {
        Some(img) => match img.save(path) {
            Ok(()) => tracing::info!("ui shot {path:?} {w}x{h}"),
            Err(e) => tracing::warn!("ui shot {path:?}: {e}"),
        },
        None => tracing::warn!("ui shot {path:?}: image did not fit its own size"),
    }
}
