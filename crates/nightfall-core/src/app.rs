//! Application shell: state, background sync, navigation.

use crate::theme::*;
use crate::views;
use crate::wallet_state::WalletState;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use nightfall_node::{NodeConfig, NodeHandle, StatusSnap};
use nightfall_storage::now_unix;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default fee: 0.001 NIGHT. Burned in full.
pub const DEFAULT_FEE_DARKS: u64 = DARKS_PER_NIGHT / 1_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Send,
    Receive,
    Activity,
    Mining,
    Network,
    Settings,
}

impl View {
    pub const ALL: [(View, &'static str); 7] = [
        (View::Dashboard, "Dashboard"),
        (View::Send, "Send"),
        (View::Receive, "Receive"),
        (View::Activity, "Activity"),
        (View::Mining, "Mining"),
        (View::Network, "Network"),
        (View::Settings, "Settings"),
    ];
}

/// Sampled hashrate, derived from the node's cumulative hash counter.
#[derive(Default)]
pub struct HashrateMeter {
    last_total: u64,
    last_at: Option<Instant>,
    pub current: f64,
    pub history: Vec<f64>,
}

impl HashrateMeter {
    pub fn sample(&mut self, total: u64) {
        let now = Instant::now();
        if let Some(prev) = self.last_at {
            let dt = now.duration_since(prev).as_secs_f64();
            if dt >= 0.9 {
                let delta = total.saturating_sub(self.last_total) as f64;
                // Light smoothing so the number is readable rather than jumpy.
                let instant = delta / dt;
                self.current = if self.current == 0.0 {
                    instant
                } else {
                    self.current * 0.6 + instant * 0.4
                };
                self.history.push(self.current);
                if self.history.len() > 120 {
                    self.history.remove(0);
                }
                self.last_total = total;
                self.last_at = Some(now);
            }
        } else {
            self.last_total = total;
            self.last_at = Some(now);
        }
    }
}

pub struct App {
    pub network: NetworkId,
    pub datadir: PathBuf,
    pub node: Option<Arc<NodeHandle>>,
    pub wallet: Arc<Mutex<WalletState>>,
    pub view: View,

    pub status: Option<StatusSnap>,
    pub status_error: Option<String>,
    pub last_status_poll: Option<Instant>,

    pub hashrate: HashrateMeter,
    pub toasts: Toasts,

    /// Set by the background sync thread when new outputs arrive.
    pub sync_signal: Arc<Mutex<Option<Result<u32, String>>>>,
    pub syncing: Arc<AtomicBool>,
    pub last_sync_at: Option<u64>,

    // Send form
    pub send_to: String,
    pub send_amount: String,
    pub send_memo: String,
    pub send_fee: u64,
    pub send_confirm: bool,
    pub send_busy: bool,

    // Settings
    pub reveal_seed: bool,
    pub reveal_view_key: bool,

    // Activity filter
    pub activity_filter: String,

    // Network
    pub peer_input: String,
    pub proxy_input: String,
}

impl App {
    pub fn new(network: NetworkId, datadir: PathBuf) -> Self {
        let wallet = match WalletState::load_or_create(&datadir, network) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("wallet: {e}");
                WalletState::empty()
            }
        };

        let proxy_input = load_proxy(&datadir);
        let mut app = Self {
            network,
            datadir,
            node: None,
            wallet: Arc::new(Mutex::new(wallet)),
            view: View::Dashboard,
            status: None,
            status_error: None,
            last_status_poll: None,
            hashrate: HashrateMeter::default(),
            toasts: Toasts::default(),
            sync_signal: Arc::new(Mutex::new(None)),
            syncing: Arc::new(AtomicBool::new(false)),
            last_sync_at: None,
            send_to: String::new(),
            send_amount: String::new(),
            send_memo: String::new(),
            send_fee: DEFAULT_FEE_DARKS,
            send_confirm: false,
            send_busy: false,
            reveal_seed: false,
            reveal_view_key: false,
            activity_filter: String::new(),
            peer_input: String::new(),
            proxy_input,
        };
        app.start_node();
        app
    }

    fn start_node(&mut self) {
        let miner = self.wallet.lock().ok().and_then(|w| w.address());

        let cfg = NodeConfig {
            network: self.network,
            datadir: self.datadir.clone(),
            p2p_listen: format!("0.0.0.0:{}", self.network.default_p2p_port()),
            rpc_listen: format!("127.0.0.1:{}", self.network.default_rpc_port()),
            connect: std::env::var("SEED_NODE")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            // Mining starts off. The user turns it on deliberately.
            mine: false,
            miner,
            mobile_listen: None,
            peers_url: std::env::var("NIGHTFALL_PEERS_URL").ok(),
            proxy: {
                let from_env = std::env::var("NIGHTFALL_PROXY").ok();
                if from_env.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
                    from_env
                } else if self.proxy_input.trim().is_empty() {
                    None
                } else {
                    Some(self.proxy_input.trim().to_string())
                }
            },
        };

        match NodeHandle::start(cfg) {
            Ok(h) => {
                let handle = Arc::new(h);
                self.node = Some(Arc::clone(&handle));
                self.spawn_sync_worker(handle);
            }
            Err(e) => {
                self.status_error = Some(format!("{e}"));
                tracing::error!("node failed to start: {e}");
            }
        }
    }

    /// Wallet scanning runs off the UI thread — trial-decrypting every output on
    /// the chain must never stall a frame.
    ///
    /// The scan is tied to the node's tip, not to a timer. A 3-second poll
    /// meant a payment could sit on disk for three seconds after the block
    /// arrived, and a reorg could be missed for the same window. `wait_tip_change`
    /// wakes this thread the moment the chain moves; a 30-second timeout is
    /// only a safety net if a notify is lost.
    fn spawn_sync_worker(&self, node: Arc<NodeHandle>) {
        let wallet = Arc::clone(&self.wallet);
        let signal = Arc::clone(&self.sync_signal);
        let syncing = Arc::clone(&self.syncing);

        std::thread::spawn(move || {
            let mut seen = node.tip_generation();
            // First pass: pick up whatever is already on disk.
            run_wallet_scan(&wallet, &node, &signal, &syncing);
            loop {
                seen = node.wait_tip_change(seen, Duration::from_secs(30));
                run_wallet_scan(&wallet, &node, &signal, &syncing);
            }
        });
    }
}

fn run_wallet_scan(
    wallet: &Arc<Mutex<WalletState>>,
    node: &NodeHandle,
    signal: &Arc<Mutex<Option<Result<u32, String>>>>,
    syncing: &Arc<AtomicBool>,
) {
    syncing.store(true, Ordering::SeqCst);
    let result = {
        let mut w = match wallet.lock() {
            Ok(w) => w,
            Err(_) => {
                syncing.store(false, Ordering::SeqCst);
                return;
            }
        };
        w.sync_from_node(node).map_err(|e| e.to_string())
    };
    syncing.store(false, Ordering::SeqCst);
    if let Ok(mut slot) = signal.lock() {
        *slot = Some(result);
    }
}

impl App {
    pub fn is_mining(&self) -> bool {
        self.node.as_ref().map(|n| n.is_mining()).unwrap_or(false)
    }

    pub fn set_mining(&self, on: bool) {
        if let Some(n) = &self.node {
            n.set_mining(on);
        }
    }

    pub fn apply_proxy(&mut self) -> anyhow::Result<()> {
        let trimmed = self.proxy_input.trim().to_string();
        if let Some(n) = &self.node {
            n.set_proxy(if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.as_str())
            })?;
        }
        save_proxy(&self.datadir, &trimmed);
        Ok(())
    }

    pub fn tip_height(&self) -> u64 {
        self.status.as_ref().map(|s| s.tip_height).unwrap_or(0)
    }

    pub fn maturity(&self) -> u64 {
        self.status
            .as_ref()
            .map(|s| s.coinbase_maturity)
            .unwrap_or(1_440)
    }

    fn poll_status(&mut self) {
        let due = self
            .last_status_poll
            .map(|t| t.elapsed() >= Duration::from_millis(700))
            .unwrap_or(true);
        if !due {
            return;
        }
        self.last_status_poll = Some(Instant::now());

        if let Some(node) = &self.node {
            match node.status_snapshot() {
                Ok(s) => {
                    self.hashrate.sample(s.hashes_total);
                    if !self.is_mining() {
                        self.hashrate.current = 0.0;
                    }
                    self.status = Some(s);
                    self.status_error = None;
                }
                Err(e) => self.status_error = Some(e.to_string()),
            }
        }
    }

    fn drain_sync_signal(&mut self, ctx: &egui::Context) {
        let taken = self.sync_signal.lock().ok().and_then(|mut s| s.take());
        if let Some(result) = taken {
            match result {
                Ok(n) => {
                    self.last_sync_at = Some(now_unix());
                    if n > 0 {
                        self.toasts.success(ctx, format!("Found {n} new output(s)"));
                    }
                }
                Err(e) => self.toasts.error(ctx, format!("Sync failed: {e}")),
            }
        }
    }

    pub fn do_send(&mut self, ctx: &egui::Context) {
        let Some(node) = self.node.clone() else {
            self.toasts.error(ctx, "Node is not running");
            return;
        };

        let amount_darks = match parse_amount(&self.send_amount) {
            Ok(a) => a,
            Err(e) => {
                self.toasts.error(ctx, e);
                return;
            }
        };

        self.send_busy = true;
        let result = {
            let mut w = match self.wallet.lock() {
                Ok(w) => w,
                Err(_) => {
                    self.send_busy = false;
                    self.toasts.error(ctx, "Wallet is busy, try again");
                    return;
                }
            };
            w.send(
                &node,
                self.send_to.trim(),
                amount_darks,
                self.send_fee,
                self.send_memo.trim(),
            )
        };
        self.send_busy = false;
        self.send_confirm = false;

        match result {
            Ok(txid) => {
                self.toasts
                    .success(ctx, format!("Sent — {}", short_hex(&txid)));
                self.send_to.clear();
                self.send_amount.clear();
                self.send_memo.clear();
                self.view = View::Activity;
            }
            Err(e) => self.toasts.error(ctx, e.to_string()),
        }
    }

    // ---------------------------------------------------------------- chrome --

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .exact_width(232.0)
            .frame(
                egui::Frame::none()
                    .fill(RAIL)
                    .inner_margin(egui::Margin::symmetric(16.0, 20.0)),
            )
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    logo(ui, 36.0);
                    ui.add_space(9.0);
                    ui.vertical(|ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new("NIGHTFALL").size(17.0).color(TEXT).strong());
                        ui.label(RichText::new("CORE WALLET").size(9.5).color(ACCENT_HI));
                    });
                });

                ui.add_space(22.0);

                for (view, label) in View::ALL {
                    let selected = self.view == view;
                    let w = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(w, 42.0), egui::Sense::click());

                    if selected {
                        gradient_rect(ui.painter(), rect, ROUND_SM, Vec2::new(1.0, 0.0), |t| {
                            brand_gradient(t * 0.5).gamma_multiply(0.30)
                        });
                        // Accent bar marking the current section.
                        let bar = egui::Rect::from_min_size(
                            rect.min + Vec2::new(0.0, 9.0),
                            Vec2::new(3.0, rect.height() - 18.0),
                        );
                        ui.painter()
                            .rect_filled(bar, Rounding::same(2.0), ACCENT_HI);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            Rounding::same(ROUND_SM),
                            SURFACE_HI.gamma_multiply(0.7),
                        );
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    let fg = if selected { TEXT } else { TEXT_DIM };
                    let galley = ui.painter().layout_no_wrap(
                        label.to_string(),
                        egui::FontId::proportional(14.0),
                        fg,
                    );
                    ui.painter().galley(
                        egui::pos2(rect.min.x + 18.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        fg,
                    );

                    if resp.clicked() {
                        self.view = view;
                    }
                    ui.add_space(3.0);
                }

                // Bottom block: network + supply proof.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    let loading = self.status.as_ref().map(|s| s.loading).unwrap_or(false);
                    let supply_ok = self.status.as_ref().map(|s| s.supply_ok).unwrap_or(false);
                    egui::Frame::none()
                        .fill(SURFACE_HI)
                        .rounding(Rounding::same(ROUND_SM))
                        .inner_margin(egui::Margin::same(11.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (label, ok) = if loading {
                                    ("Supply — loading", true)
                                } else if supply_ok {
                                    ("Supply verified", true)
                                } else {
                                    ("Supply UNVERIFIED", false)
                                };
                                dot(ui, status_color(ok), loading);
                                ui.add_space(4.0);
                                ui.label(RichText::new(label).size(11.5).color(status_color(ok)));
                            });
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new("Σ UTXO − Σ excess = supply")
                                    .size(9.5)
                                    .color(TEXT_FAINT),
                            );
                        });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let net = self.network.as_str();
                        let color = match self.network {
                            NetworkId::Mainnet => SUCCESS,
                            NetworkId::Testnet => WARN,
                            NetworkId::Devnet => ACCENT_HI,
                        };
                        badge(ui, &net.to_uppercase(), color);
                        ui.label(
                            RichText::new(format!("v{}", nightfall_types::PROTOCOL_VERSION))
                                .size(10.0)
                                .color(TEXT_FAINT),
                        );
                    });
                });
            });
    }

    fn topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .exact_height(60.0)
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(24.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let title = View::ALL
                        .iter()
                        .find(|(v, _)| *v == self.view)
                        .map(|(_, l)| *l)
                        .unwrap_or("");
                    ui.label(RichText::new(title).size(20.0).strong());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let loading = self.status.as_ref().map(|s| s.loading).unwrap_or(false);
                        let mining = self.is_mining();
                        let btn = egui::Button::new(
                            RichText::new(if loading {
                                "Loading chain…"
                            } else if mining {
                                "Stop mining"
                            } else {
                                "Start mining"
                            })
                            .strong()
                            .color(if mining {
                                TEXT
                            } else {
                                Color32::WHITE
                            }),
                        )
                        .fill(if mining { SURFACE_HI } else { ACCENT })
                        .stroke(if mining {
                            Stroke::new(1.0_f32, BORDER_HI)
                        } else {
                            Stroke::NONE
                        })
                        .rounding(Rounding::same(ROUND_SM))
                        .min_size(Vec2::new(126.0, 34.0));
                        if ui.add_enabled(!loading, btn).clicked() && !loading {
                            self.set_mining(!mining);
                            if mining {
                                self.hashrate.current = 0.0;
                            }
                        }

                        ui.add_space(14.0);

                        // Chain height + sync indicator
                        let syncing = self.syncing.load(Ordering::SeqCst);
                        let blocks = self.status.as_ref().map(|s| s.blocks).unwrap_or(0);
                        ui.horizontal(|ui| {
                            dot(
                                ui,
                                if self.status_error.is_some() {
                                    DANGER
                                } else if syncing {
                                    WARN
                                } else {
                                    SUCCESS
                                },
                                syncing,
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!("{} blocks", format_int(blocks)))
                                    .size(12.5)
                                    .color(TEXT_DIM),
                            );
                        });

                        if mining {
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new(format_hashrate(self.hashrate.current))
                                    .size(12.5)
                                    .color(ACCENT_HI),
                            );
                        }
                    });
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply(ctx);
        self.poll_status();
        self.drain_sync_signal(ctx);

        // Keep the UI live for hashrate and sync animation.
        ctx.request_repaint_after(Duration::from_millis(500));

        self.sidebar(ctx);
        self.topbar(ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin {
                left: 24.0,
                right: 24.0,
                top: 4.0,
                bottom: 16.0,
            }))
            .show(ctx, |ui| {
                if let Some(err) = self.status_error.clone() {
                    egui::Frame::none()
                        .fill(DANGER.gamma_multiply(0.12))
                        .stroke(Stroke::new(1.0_f32, DANGER.gamma_multiply(0.5)))
                        .rounding(Rounding::same(ROUND_SM))
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("Node error: {err}")).color(DANGER));
                        });
                    ui.add_space(12.0);
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Content is capped and centred: full-bleed cards on an
                        // ultra-wide display look sparse and are hard to read.
                        const MAX_CONTENT: f32 = 1180.0;
                        let avail = ui.available_width();
                        let pad = ((avail - MAX_CONTENT) / 2.0).max(0.0);
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            ui.allocate_ui_with_layout(
                                Vec2::new(avail - pad * 2.0, 0.0),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| match self.view {
                                    View::Dashboard => views::dashboard(self, ui),
                                    View::Send => views::send(self, ui, ctx),
                                    View::Receive => views::receive(self, ui, ctx),
                                    View::Activity => views::activity(self, ui),
                                    View::Mining => views::mining(self, ui),
                                    View::Network => views::network(self, ui, ctx),
                                    View::Settings => views::settings(self, ui, ctx),
                                },
                            );
                        });
                    });
            });

        self.toasts.show(ctx);
    }
}

fn proxy_file(datadir: &std::path::Path) -> PathBuf {
    datadir.join("socks_proxy")
}

fn load_proxy(datadir: &std::path::Path) -> String {
    if let Ok(s) = std::env::var("NIGHTFALL_PROXY") {
        if !s.trim().is_empty() {
            return s;
        }
    }
    std::fs::read_to_string(proxy_file(datadir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| nightfall_p2p::DEFAULT_TOR_PROXY.to_string())
}

fn save_proxy(datadir: &std::path::Path, value: &str) {
    let path = proxy_file(datadir);
    if value.is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        let _ = std::fs::write(path, value);
    }
}

/// Parse a decimal NIGHT amount into darks.
///
/// Done on the string, never through `f64` — binary floating point cannot
/// represent 8 decimal places exactly, and silently losing a dark in a payment
/// form is not acceptable.
pub fn parse_amount(s: &str) -> Result<u64, String> {
    let normalised = s.trim().replace(',', ".");
    if normalised.is_empty() {
        return Err("Enter an amount".into());
    }
    if normalised.starts_with('-') {
        return Err("Amount must be positive".into());
    }

    let mut parts = normalised.splitn(2, '.');
    let whole_str = parts.next().unwrap_or("");
    let frac_str = parts.next().unwrap_or("");

    if !whole_str.chars().all(|c| c.is_ascii_digit()) || whole_str.is_empty() {
        return Err("Amount is not a number".into());
    }
    if !frac_str.chars().all(|c| c.is_ascii_digit()) {
        return Err("Amount is not a number".into());
    }
    if frac_str.len() > 8 {
        return Err("At most 8 decimal places".into());
    }

    let whole: u64 = whole_str
        .parse()
        .map_err(|_| "Amount too large".to_string())?;
    let mut darks = whole
        .checked_mul(DARKS_PER_NIGHT)
        .ok_or("Amount too large")?;

    if !frac_str.is_empty() {
        let padded = format!("{frac_str:0<8}");
        let frac: u64 = padded.parse().map_err(|_| "Bad decimals".to_string())?;
        darks = darks.checked_add(frac).ok_or("Amount too large")?;
    }

    if darks == 0 {
        return Err("Amount must be greater than zero".into());
    }
    Ok(darks)
}

#[cfg(test)]
mod tests {
    use super::parse_amount;
    use nightfall_types::DARKS_PER_NIGHT;

    #[test]
    fn parses_whole_and_fractional_amounts() {
        assert_eq!(parse_amount("1").unwrap(), DARKS_PER_NIGHT);
        assert_eq!(parse_amount("0.5").unwrap(), DARKS_PER_NIGHT / 2);
        assert_eq!(
            parse_amount("2.25").unwrap(),
            2 * DARKS_PER_NIGHT + 25_000_000
        );
        assert_eq!(parse_amount("0.00000001").unwrap(), 1);
        assert_eq!(parse_amount("1,5").unwrap(), DARKS_PER_NIGHT + 50_000_000);
    }

    #[test]
    fn rejects_bad_amounts() {
        assert!(parse_amount("").is_err());
        assert!(parse_amount("0").is_err());
        assert!(parse_amount("-1").is_err());
        assert!(parse_amount("abc").is_err());
        assert!(parse_amount("0.123456789").is_err());
    }
}
