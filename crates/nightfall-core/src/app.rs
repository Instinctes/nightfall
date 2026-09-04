//! Application shell: state, background sync, navigation.

use crate::address_book::AddressBook;
use crate::theme::*;
use crate::tray::{Tray, TrayAction};
use crate::views;
use crate::wallet_state::WalletState;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2, ViewportCommand};
use nightfall_crypto::WalletKeys;
use nightfall_node::{NodeConfig, NodeHandle, StatusSnap};
use nightfall_storage::now_unix;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default fee: 0.001 NIGHT. Burned in full.
pub const DEFAULT_FEE_DARKS: u64 = DARKS_PER_NIGHT / 1_000;

/// Desktop build. Not the protocol — that is `PROTOCOL_VERSION`.
pub const WALLET_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Send,
    Receive,
    Activity,
    Mining,
    Network,
    Swap,
    Settings,
}

impl View {
    pub const ALL: [(View, &'static str); 8] = [
        (View::Dashboard, "Dashboard"),
        (View::Send, "Send"),
        (View::Receive, "Receive"),
        (View::Activity, "Activity"),
        (View::Mining, "Mining"),
        (View::Network, "Network"),
        (View::Swap, "Swap"),
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
    pub reveal_mnemonic: bool,
    pub reveal_view_key: bool,
    pub backup_acked: bool,
    pub resync_confirm: bool,
    pub close_to_tray: bool,
    pub prune: bool,
    pub mining_threads: usize,
    pub want_quit: bool,
    pub window_hidden: bool,

    // Activity filter
    pub activity_filter: String,

    // Network
    pub peer_input: String,
    pub proxy_input: String,
    pub chain_check: Option<ChainCheck>,
    pub chain_check_busy: Arc<AtomicBool>,

    // Address book
    pub address_book: AddressBook,
    pub book_name: String,
    pub book_addr: String,

    // Swap
    pub swap_draft: nightfall_swap::ui::Draft,
    /// Packet to hand to the counterparty, rendered for copying.
    pub swap_packet_out: String,
    /// Packet pasted from the counterparty, before it is believed.
    pub swap_packet_in: String,
    /// Why the pasted packet was refused. Every rejection names its reason.
    pub swap_import_error: Option<String>,
    pub swap_start_error: Option<String>,
    /// Live handshakes, by swap id. Reloaded from `{datadir}/swaps/{id}.secret`.
    pub swap_sessions: std::collections::HashMap<String, nightfall_swap::session::Session>,
    /// Bitcoin addresses the user supplies, because this wallet holds NIGHT
    /// and not Bitcoin. Where a refund, a redeem and a punish would pay.
    pub swap_btc_refund: String,
    pub swap_btc_redeem: String,
    pub swap_btc_punish: String,

    /// Bob's funding input for TX_lock, as typed.
    pub swap_funding: nightfall_swap::ui::FundingDraft,
    /// The signed transaction pasted back from the user's Bitcoin wallet.
    pub swap_signed_hex: String,
    pub swap_lock_note: Option<Result<String, String>>,

    /// A destructive button was pressed once; it asks before it acts.
    ///
    /// The action itself is kept, not its wording. Recovering it from the
    /// confirmation text meant every new button had to be added to a string
    /// comparison, and forgetting to would silently run the *wrong*
    /// transaction. The id is text so this crate needs no uuid dependency.
    pub swap_confirm: Option<(String, nightfall_swap::ui::Action)>,

    pub onboarding: Option<Onboarding>,
    tray: Option<Tray>,
    pending_chain_check: Option<Arc<Mutex<Option<ChainCheck>>>>,
}

/// First-run screens. Existing wallets skip this.
pub enum Onboarding {
    Choice,
    Create {
        phrase: String,
        written: bool,
    },
    Restore {
        phrase: String,
        error: Option<String>,
    },
}

#[derive(Clone)]
pub struct ChainCheck {
    pub our_tip: String,
    pub our_height: u64,
    pub public_tip: String,
    pub public_height: u64,
    pub genesis: String,
    pub same: bool,
    pub error: Option<String>,
}

impl App {
    pub fn new(network: NetworkId, datadir: PathBuf) -> Self {
        let has_seed = WalletState::seed_exists(&datadir);
        let wallet = if has_seed {
            match WalletState::load_or_create(&datadir, network) {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!("wallet: {e}");
                    WalletState::empty()
                }
            }
        } else {
            WalletState::empty()
        };

        let proxy_input = load_proxy(&datadir);
        let mining_threads = load_mining_threads(&datadir);
        let mut app = Self {
            network,
            datadir: datadir.clone(),
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
            reveal_mnemonic: false,
            reveal_view_key: false,
            backup_acked: load_backup_acked(&datadir),
            resync_confirm: false,
            close_to_tray: load_close_to_tray(&datadir),
            prune: load_flag(&datadir, "prune", false),
            mining_threads,
            want_quit: false,
            window_hidden: false,
            activity_filter: String::new(),
            peer_input: String::new(),
            proxy_input,
            chain_check: None,
            chain_check_busy: Arc::new(AtomicBool::new(false)),
            address_book: AddressBook::load(&datadir),
            book_name: String::new(),
            book_addr: String::new(),
            swap_draft: nightfall_swap::ui::Draft {
                give_night: true,
                ..Default::default()
            },
            swap_sessions: std::collections::HashMap::new(),
            swap_btc_refund: String::new(),
            swap_btc_redeem: String::new(),
            swap_btc_punish: String::new(),
            swap_funding: nightfall_swap::ui::FundingDraft::default(),
            swap_signed_hex: String::new(),
            swap_lock_note: None,
            swap_packet_out: String::new(),
            swap_packet_in: String::new(),
            swap_import_error: None,
            swap_start_error: None,
            swap_confirm: None,
            onboarding: if has_seed {
                None
            } else {
                Some(Onboarding::Choice)
            },
            tray: None,
            pending_chain_check: None,
        };
        if has_seed {
            app.start_node();
        }
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
            // A desktop wallet wants peers, not petitioners.
            introducer: false,
            prune: load_flag(&self.datadir, "prune", false),
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
                h.set_mining_threads(self.mining_threads);
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

    pub fn begin_create_wallet(&mut self) {
        let keys = WalletKeys::generate();
        self.onboarding = Some(Onboarding::Create {
            phrase: keys.to_mnemonic(),
            written: false,
        });
    }

    pub fn finish_create_wallet(&mut self, phrase: &str) -> anyhow::Result<()> {
        let w = WalletState::restore_from_phrase(&self.datadir, self.network, phrase)?;
        if let Ok(mut slot) = self.wallet.lock() {
            *slot = w;
        }
        self.ack_backup();
        self.onboarding = None;
        self.start_node();
        Ok(())
    }

    pub fn finish_restore_wallet(&mut self, phrase: &str) -> anyhow::Result<()> {
        let w = WalletState::restore_from_phrase(&self.datadir, self.network, phrase)?;
        if let Ok(mut slot) = self.wallet.lock() {
            *slot = w;
        }
        self.ack_backup();
        self.onboarding = None;
        self.start_node();
        Ok(())
    }

    pub fn ack_backup(&mut self) {
        self.backup_acked = true;
        save_flag(&self.datadir, "backup_acked", true);
    }

    pub fn set_close_to_tray(&mut self, on: bool) {
        self.close_to_tray = on;
        save_flag(&self.datadir, "close_to_tray", on);
    }

    pub fn set_prune(&mut self, on: bool, ctx: &egui::Context) {
        if !on && self.prune {
            if let Some(s) = &self.status {
                if s.pruned {
                    self.toasts.error(
                        ctx,
                        "Already pruned — Resync chain from a seed to store full history again",
                    );
                    return;
                }
            }
        }
        self.prune = on;
        save_flag(&self.datadir, "prune", on);
        if let Some(node) = &self.node {
            if let Err(e) = node.set_prune(on) {
                self.toasts.error(ctx, e.to_string());
            } else if on {
                self.toasts.info(
                    ctx,
                    "Prune on — bodies older than 500 blocks will be dropped",
                );
            }
        }
    }

    pub fn set_mining_threads(&mut self, n: usize) {
        let n = n.clamp(1, 64);
        self.mining_threads = n;
        save_mining_threads(&self.datadir, n);
        if let Some(node) = &self.node {
            node.set_mining_threads(n);
        }
    }

    pub fn resync_chain(&mut self, ctx: &egui::Context) {
        let Some(node) = self.node.clone() else {
            self.toasts.error(ctx, "Node is not running");
            return;
        };
        match node.resync_chain() {
            Ok(backup) => {
                if let Ok(mut w) = self.wallet.lock() {
                    let _ = w.rescan(&node);
                }
                self.resync_confirm = false;
                self.toasts.success(
                    ctx,
                    format!(
                        "Chain file set aside at {}. Downloading the live chain.",
                        backup.display()
                    ),
                );
            }
            Err(e) => self.toasts.error(ctx, e.to_string()),
        }
    }

    pub fn start_chain_check(&mut self) {
        if self.chain_check_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let our_tip = self
            .status
            .as_ref()
            .map(|s| s.tip.clone())
            .unwrap_or_default();
        let our_height = self.tip_height();
        let busy = Arc::clone(&self.chain_check_busy);
        let slot: Arc<Mutex<Option<ChainCheck>>> = Arc::new(Mutex::new(None));
        let out = Arc::clone(&slot);
        std::thread::spawn(move || {
            let result = fetch_public_tip();
            let check = match result {
                Ok((public_tip, public_height, genesis)) => {
                    let same = !our_tip.is_empty() && our_tip.eq_ignore_ascii_case(&public_tip);
                    ChainCheck {
                        our_tip,
                        our_height,
                        public_tip,
                        public_height,
                        genesis,
                        same,
                        error: None,
                    }
                }
                Err(e) => ChainCheck {
                    our_tip,
                    our_height,
                    public_tip: String::new(),
                    public_height: 0,
                    genesis: String::new(),
                    same: false,
                    error: Some(e),
                },
            };
            if let Ok(mut g) = out.lock() {
                *g = Some(check);
            }
            busy.store(false, Ordering::SeqCst);
        });
        self.pending_chain_check = Some(slot);
    }

    fn take_chain_check(&mut self) {
        let Some(slot) = &self.pending_chain_check else {
            return;
        };
        let taken = slot.lock().ok().and_then(|mut g| g.take());
        if let Some(check) = taken {
            self.chain_check = Some(check);
            self.pending_chain_check = None;
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
        self.take_chain_check();

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
                // "Sent" was a lie by one word. The transaction has been handed
                // to the local node, not into a block, and the difference is
                // hours of confusion when it never gets there.
                self.toasts.success(
                    ctx,
                    format!("Submitted — {} · waiting for a block", short_hex(&txid)),
                );
                // A new payment goes to exactly one random peer first, so that
                // it cannot be traced back to this node. With almost no peers
                // that one hop is also the only hop, and nothing re-sends it.
                // Say so now, while the sender is still looking at the screen.
                let peers = self.status.as_ref().map(|s| s.peers).unwrap_or(0);
                if peers < 3 {
                    self.toasts.error(
                        ctx,
                        format!(
                            "Only {peers} peer(s) connected — a payment handed to one peer \
                             can be lost. If it has not confirmed in an hour, rescan in \
                             Settings and send it again."
                        ),
                    );
                }
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
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new("CORE WALLET").size(9.5).color(ACCENT_HI));
                            ui.label(RichText::new(WALLET_VERSION).size(9.5).color(TEXT_DIM));
                        });
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

                // Foot of the rail: which network, and whether the supply adds up.
                //
                // This was a filled, rounded box sitting inside a rail that has
                // no other boxes in it, so it read as a widget someone had
                // dropped there — and the network badge floated above it,
                // unrelated to the thing it belongs with. Now it is one quiet
                // block: a rule, the network, the supply state. No fill, because
                // the rail is flat.
                //
                // The formula moved into the hover. It explains the line above
                // it rather than reporting anything, and at 9.5px under a status
                // it competed with the status for the same glance.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    let loading = self.status.as_ref().map(|s| s.loading).unwrap_or(false);
                    let supply_ok = self.status.as_ref().map(|s| s.supply_ok).unwrap_or(false);

                    // Colour follows the truth. While the chain loads nothing
                    // has been checked yet, and the old code passed ok = true
                    // for that case — a green dot and green lettering next to
                    // the word "loading". Green here means the sum was
                    // recomputed and balanced. Until it has been, this is dim.
                    let (label, colour) = if loading {
                        ("Checking supply…", TEXT_DIM)
                    } else if supply_ok {
                        ("Supply verified", SUCCESS)
                    } else {
                        ("Supply UNVERIFIED", DANGER)
                    };

                    ui.add_space(2.0);
                    let resp = ui
                        .horizontal(|ui| {
                            dot(ui, colour, loading);
                            ui.add_space(6.0);
                            ui.label(RichText::new(label).size(11.5).color(colour))
                        })
                        .inner;
                    resp.on_hover_text(
                        "Σ UTXO − Σ excess = (minted − burned)·G\n\nEvery node recomputes this \
                         over the whole UTXO set and refuses a block that breaks it. One coin \
                         minted out of nowhere and the equation stops balancing.",
                    );

                    ui.add_space(9.0);
                    ui.horizontal(|ui| {
                        let net = self.network.as_str();
                        let color = match self.network {
                            NetworkId::Mainnet => SUCCESS,
                            NetworkId::Testnet => WARN,
                            NetworkId::Devnet => ACCENT_HI,
                        };
                        badge(ui, &net.to_uppercase(), color);
                        ui.label(
                            RichText::new(format!(
                                "protocol v{}",
                                nightfall_types::PROTOCOL_VERSION
                            ))
                            .size(10.0)
                            .color(TEXT_FAINT),
                        );
                    });

                    ui.add_space(12.0);
                    let w = ui.available_width();
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), egui::Sense::hover());
                    ui.painter().hline(
                        r.x_range(),
                        r.center().y,
                        Stroke::new(1.0_f32, BORDER.gamma_multiply(0.8)),
                    );
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
        self.handle_tray(ctx);

        // Keep the UI live for hashrate, tray clicks, and sync animation.
        ctx.request_repaint_after(Duration::from_millis(500));

        if self.onboarding.is_some() {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(BG)
                        .inner_margin(egui::Margin::same(28.0)),
                )
                .show(ctx, |ui| {
                    views::onboarding(self, ui, ctx);
                });
            self.toasts.show(ctx);
            return;
        }

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
                // The lights go down first, over the panel's flat fill and
                // under everything else. Painted at panel level rather than
                // inside the scroll area on purpose: this is the lighting of
                // the room, and it must not slide up and down with the
                // content the way a background image would.
                page_wash(ui.painter(), ui.clip_rect());

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
                    // Always reserve the bar. With the default the bar appears
                    // only once the page overflows, which takes ~10px of width
                    // away mid-session — every right-aligned column then shifts
                    // sideways as you scroll. Reserving it costs a sliver of
                    // width and makes the layout stop moving.
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
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
                                    View::Swap => crate::views_swap::swap(self, ui, ctx),
                                    View::Settings => views::settings(self, ui, ctx),
                                },
                            );
                        });
                    });
            });

        self.toasts.show(ctx);
    }
}

impl App {
    fn handle_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            self.tray = Tray::new();
        }
        if let Some(tray) = &self.tray {
            match tray.poll() {
                Some(TrayAction::Show) => {
                    self.window_hidden = false;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                Some(TrayAction::Quit) => {
                    self.want_quit = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                None => {}
            }
        }
        if self.want_quit {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested())
            && self.close_to_tray
            && self.tray.is_some()
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
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

fn save_flag(datadir: &std::path::Path, name: &str, on: bool) {
    let path = datadir.join(name);
    let _ = if on {
        std::fs::write(path, "1")
    } else {
        std::fs::write(path, "0")
    };
}

fn load_flag(datadir: &std::path::Path, name: &str, default: bool) -> bool {
    std::fs::read_to_string(datadir.join(name))
        .ok()
        .map(|s| matches!(s.trim(), "1" | "true" | "yes"))
        .unwrap_or(default)
}

fn load_backup_acked(datadir: &std::path::Path) -> bool {
    load_flag(datadir, "backup_acked", false)
}

fn load_close_to_tray(datadir: &std::path::Path) -> bool {
    load_flag(datadir, "close_to_tray", true)
}

fn load_mining_threads(datadir: &std::path::Path) -> usize {
    if let Ok(s) = std::env::var("NF_MINING_THREADS") {
        if let Ok(n) = s.parse::<usize>() {
            if n > 0 {
                return n.clamp(1, 64);
            }
        }
    }
    std::fs::read_to_string(datadir.join("mining_threads"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or_else(nightfall_crypto::default_threads)
        .clamp(1, 64)
}

fn save_mining_threads(datadir: &std::path::Path, n: usize) {
    let _ = std::fs::write(datadir.join("mining_threads"), n.to_string());
}

fn fetch_public_tip() -> Result<(String, u64, String), String> {
    let body: serde_json::Value = ureq::get("https://nightfallcoin.org/network.json")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    let tip = body
        .get("tip")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let height = body.get("tip_height").and_then(|v| v.as_u64()).unwrap_or(0);
    let genesis = body
        .get("genesis")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if tip.is_empty() && genesis.is_empty() {
        return Err("nightfallcoin.org returned no tip".into());
    }
    Ok((tip, height, genesis))
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
