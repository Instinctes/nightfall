//! Application shell: state, background sync, navigation.

use crate::address_book::AddressBook;
use crate::onboarding::Onboarding;
use crate::theme::*;
use crate::tray::{Tray, TrayAction};
use crate::views;
use crate::wallet_state::Custody;
use crate::wallet_state::WalletState;
use crate::widgets::*;
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2, ViewportCommand};
use nightfall_node::{NodeConfig, NodeHandle, StatusSnap};
use nightfall_storage::now_unix;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

/// Default fee: 0.001 NIGHT. Burned in full.
pub const DEFAULT_FEE_DARKS: u64 = DARKS_PER_NIGHT / 1_000;

/// Desktop build. Not the protocol — that is `PROTOCOL_VERSION`.
pub const WALLET_VERSION: &str = match option_env!("NIGHTFALL_DEV_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
pub const IS_DEV_BUILD: bool = option_env!("NIGHTFALL_DEV_VERSION").is_some()
    || option_env!("NIGHTFALL_DEV_PROFILE").is_some();
/// The isolated preview cannot inherit the historical mainnet opt-in.
pub const IS_ISOLATED_DEV_PROFILE: bool = option_env!("NIGHTFALL_DEV_PROFILE").is_some();
pub const DEV_DATA_SUBDIR: &str = if IS_ISOLATED_DEV_PROFILE {
    "wallet-1.0.0-dev.2"
} else {
    "wallet-1.0-dev"
};

/// A development build that is allowed to open mainnet.
///
/// Off unless `NIGHTFALL_DEV_MAINNET` was set when the binary was compiled, so
/// the permission belongs to one artifact rather than to a command line: a dev
/// build that escapes cannot be talked into mainnet by an argument, and you
/// can tell which kind you are holding without running it.
///
/// This exists because the operator asked for a build that opens their real
/// wallet. It is not a step toward shipping development builds on mainnet.
///
/// # What such a build does to a 0.9.5 wallet
///
/// 1.0 adds a `invoices` field to the wallet file, and that file refuses
/// unknown fields. So once a 1.0 build has *saved*, the released 0.9.5 app can
/// no longer read that wallet — it is a one-way door, and the way back is an
/// encrypted backup made beforehand. The banner in `mainnet_dev_warning` says
/// so on every page, and the build's own README says it first.
pub const IS_DEV_MAINNET: bool =
    !IS_ISOLATED_DEV_PROFILE && option_env!("NIGHTFALL_DEV_MAINNET").is_some();

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

    /// Glance pages fill the panel. Forms stop stretching at a readable width.
    ///
    /// This cap is applied to banners *and* the page together. Capping the
    /// page alone left Settings' cards inset under a full-bleed scan warning,
    /// and wrapping Dashboard in the same cap left a dead strip down the right.
    pub fn content_max_width(self) -> f32 {
        match self {
            View::Send | View::Settings => 720.0,
            _ => f32::INFINITY,
        }
    }
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
    pub vault_ui: crate::vault_ui::VaultUi,
    pub wallet_paused: Arc<AtomicBool>,
    /// Outer lock: scans may hold the wallet for minutes. Frames only try this
    /// gate and render a lock-free progress panel if a scan owns it. Hold it
    /// through the entire frame; probing the wallet mutex alone races a scan.
    pub wallet_scan_access: Arc<Mutex<()>>,
    data_lock: Option<Arc<nightfall_storage::dirlock::DirLock>>,
    pub view: View,

    pub status: Option<StatusSnap>,
    pub status_error: Option<String>,
    pub last_status_poll: Option<Instant>,

    pub hashrate: HashrateMeter,
    pub toasts: Toasts,

    /// Dev-only page capture. `None` in every normal run — see `ui_shots`.
    pub shots: Option<crate::ui_shots::Shots>,

    /// Set by the background sync thread when new outputs arrive.
    pub sync_signal: Arc<Mutex<Option<Result<u32, String>>>>,
    pub syncing: Arc<AtomicBool>,
    pub last_sync_at: Option<u64>,
    /// A detected scan failure remains visible until a successful canonical
    /// scan; it must not disappear with a toast or a successful node poll.
    pub wallet_sync_error: Option<String>,

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
    pub recovery_studio: crate::recovery_studio::RecoveryStudio,
    pub resync_confirm: bool,
    pub close_to_tray: bool,
    pub prune: bool,
    pub mining_threads: usize,
    pub want_quit: bool,
    pub window_hidden: bool,

    // Activity filter
    pub activity_filter: String,

    // Proof Card. `proof_input` holds a receipt someone pasted, or the one
    // this wallet just produced — the owner is shown the same card the person
    // receiving it will see, because "show exactly what is revealed" means
    // showing it before it is handed over, not after.
    pub proof_input: String,
    pub proof_result: Option<Result<nightfall_wallet::ReceiptProof, String>>,
    /// Set when a receipt was just produced, so the card scrolls into view.
    pub proof_scroll: bool,

    // Counter — the till's "add an invoice" form. The invoices themselves live
    // in the wallet's encrypted snapshot, not here.
    pub till_reference: String,
    pub till_amount: String,
    pub till_description: String,

    // Air. `air_pending` is the request this online side wrote and is waiting
    // for an answer to; `air_frames` is whatever this side is currently showing
    // as animated QR. The nonce log is what stops a signed package being
    // broadcast twice — see `nightfall_wallet::air`.
    pub air_input: String,
    pub air_pending: Option<nightfall_wallet::air::Intent>,
    pub air_incoming: Option<nightfall_wallet::air::Intent>,
    pub air_frames: Vec<String>,
    pub air_frame_at: usize,
    pub air_last_tick: Option<Instant>,
    pub air_log: nightfall_wallet::air::NonceLog,
    pub air_note: Option<String>,
    pub air_error: Option<String>,

    // Network
    pub peer_input: String,
    pub proxy_input: String,
    pub chain_check: Option<ChainCheck>,
    pub chain_check_busy: Arc<AtomicBool>,

    // Address book
    pub address_book: AddressBook,
    pub book_name: String,
    pub book_addr: String,

    // Atomic swap between NIGHT and Bitcoin was carried through most of the
    // 1.0.0 cycle and withdrawn before release. `docs/SWAP-WITHDRAWN.md` has
    // the reasoning. Its state lived here; nothing replaced it.
    /// First observation of the chain load: (when, how many blocks).
    ///
    /// The estimate is built from what this machine is actually doing, not
    /// from a constant. Verification speed depends on the CPU and on how
    /// memory-hard the proof of work is, and a hard-coded "about 15 minutes"
    /// is wrong on most machines and insulting on a slow one.
    pub load_started: Option<(Instant, u64)>,

    pub onboarding: Option<Onboarding>,
    /// A failed load must not look like a zero-balance wallet or first run.
    pub wallet_load_error: Option<String>,
    tray: Option<Tray>,
    pending_chain_check: Option<Arc<Mutex<Option<ChainCheck>>>>,
    /// History ids already observed, so a lock/unlock does not chime for old coins.
    activity_seen: std::collections::HashSet<String>,
    activity_primed: bool,
    /// Context time when a receive/mine flash started.
    coin_flash_at: Option<f64>,
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
    #[cfg(test)]
    pub fn new(network: NetworkId, datadir: PathBuf) -> Self {
        Self::construct(network, datadir, None)
    }

    pub fn with_data_lock(
        network: NetworkId,
        lock: Arc<nightfall_storage::dirlock::DirLock>,
    ) -> Self {
        let datadir = lock
            .path()
            .parent()
            .expect("directory lock has a parent")
            .to_path_buf();
        Self::construct(network, datadir, Some(lock))
    }

    fn construct(
        network: NetworkId,
        datadir: PathBuf,
        data_lock: Option<Arc<nightfall_storage::dirlock::DirLock>>,
    ) -> Self {
        let has_seed = WalletState::seed_exists(&datadir);
        let mut wallet_load_error = None;
        let wallet = if has_seed {
            let opened = (|| {
                if nightfall_wallet::vault_required(&datadir, "core.seed")? {
                    let lock = data_lock.clone().ok_or_else(|| anyhow::anyhow!("Vault requires the data-directory lock. Restart Core with a compatible build."))?;
                    WalletState::open_vault(lock, network)
                } else {
                    WalletState::load_or_create(&datadir, network)
                }
            })();
            match opened {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!("wallet: {e}");
                    wallet_load_error = Some(e.to_string());
                    WalletState::empty()
                }
            }
        } else {
            WalletState::empty()
        };

        let proxy_input = load_proxy(&datadir);
        let mining_threads = load_mining_threads(&datadir);
        let custody = wallet.custody();
        let has_snapshot = wallet.has_vault_snapshot();
        let initial_restore = wallet.awaiting_initial_restore();
        let mut vault_ui = crate::vault_ui::VaultUi::default();
        vault_ui.custody = custody;
        vault_ui.has_snapshot = has_snapshot;
        let mut app = Self {
            network,
            datadir: datadir.clone(),
            node: None,
            wallet: Arc::new(Mutex::new(wallet)),
            vault_ui,
            wallet_paused: Arc::new(AtomicBool::new(!matches!(
                custody,
                Custody::Legacy | Custody::Unlocked
            ))),
            data_lock,
            view: View::Dashboard,
            status: None,
            status_error: None,
            last_status_poll: None,
            hashrate: HashrateMeter::default(),
            toasts: Toasts::default(),
            shots: crate::ui_shots::Shots::from_env(),
            sync_signal: Arc::new(Mutex::new(None)),
            wallet_scan_access: Arc::new(Mutex::new(())),
            syncing: Arc::new(AtomicBool::new(false)),
            last_sync_at: None,
            wallet_sync_error: None,
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
            recovery_studio: Default::default(),
            resync_confirm: false,
            close_to_tray: load_close_to_tray(&datadir),
            prune: load_flag(&datadir, "prune", false),
            mining_threads,
            want_quit: false,
            window_hidden: false,
            activity_filter: String::new(),
            proof_input: String::new(),
            proof_result: None,
            proof_scroll: false,
            till_reference: String::new(),
            till_amount: String::new(),
            till_description: String::new(),
            air_input: String::new(),
            air_pending: None,
            air_incoming: None,
            air_frames: Vec::new(),
            air_frame_at: 0,
            air_last_tick: None,
            air_log: nightfall_wallet::air::NonceLog::new(),
            air_note: None,
            air_error: None,
            peer_input: String::new(),
            proxy_input,
            chain_check: None,
            chain_check_busy: Arc::new(AtomicBool::new(false)),
            address_book: AddressBook::load(&datadir),
            book_name: String::new(),
            book_addr: String::new(),
            load_started: None,
            onboarding: if initial_restore {
                Some(Onboarding::resume())
            } else if has_seed {
                None
            } else {
                Some(Onboarding::Choice)
            },
            wallet_load_error,
            tray: None,
            pending_chain_check: None,
            activity_seen: std::collections::HashSet::new(),
            activity_primed: false,
            coin_flash_at: None,
        };
        if custody == Custody::Legacy && app.wallet_load_error.is_none() {
            app.start_node();
        }
        app
    }

    fn start_node(&mut self) {
        let miner = self.wallet.lock().ok().and_then(|w| w.address());

        let mut cfg = NodeConfig {
            network: self.network,
            datadir: self.datadir.clone(),
            p2p_listen: arg_or(
                "--listen",
                format!("0.0.0.0:{}", self.network.default_p2p_port()),
            ),
            rpc_listen: arg_or(
                "--rpc-listen",
                format!("127.0.0.1:{}", self.network.default_rpc_port()),
            ),
            connect: {
                let mut v = args_all("--connect");
                if v.is_empty() {
                    if let Ok(s) = std::env::var("SEED_NODE") {
                        v = s
                            .split(',')
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty())
                            .collect();
                    }
                }
                v
            },
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

        // Local preview builds never inherit production peer/proxy environment
        // settings or bind publicly. Devnet has no hard-coded seed nodes.
        if IS_DEV_BUILD {
            cfg.p2p_listen = "127.0.0.1:0".into();
            cfg.rpc_listen = "127.0.0.1:0".into();
            cfg.connect.clear();
            cfg.peers_url = Some("off".into());
            cfg.proxy = Some("off".into());
        }
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

    pub fn show_onboarding(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(error) = &self.vault_ui.error {
            ui.colored_label(DANGER, error);
        }
        let available = cfg!(unix) && self.data_lock.is_some() && !self.vault_ui.busy();
        let request = self
            .onboarding
            .as_mut()
            .and_then(|setup| setup.show_panel(ui, available, self.network));
        if let Some(request) = request {
            self.vault_ui.start_provision(
                self.wallet.clone(),
                self.wallet_paused.clone(),
                self.data_lock.clone(),
                self.network,
                request,
                ctx.clone(),
            );
        }
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
        let wallet = self.wallet.clone();
        let access = self.wallet_scan_access.clone();
        let paused = self.wallet_paused.clone();
        let syncing = self.syncing.clone();
        let signal = self.sync_signal.clone();
        self.resync_confirm = false;
        std::thread::spawn(move || {
            let Ok(_access) = access.lock() else {
                return;
            };
            syncing.store(true, Ordering::SeqCst);
            let result = (|| -> anyhow::Result<u32> {
                let mut wallet = wallet
                    .lock()
                    .map_err(|_| anyhow::anyhow!("wallet lock poisoned"))?;
                anyhow::ensure!(
                    !paused.load(Ordering::SeqCst) && wallet.can_scan(),
                    "Resync cancelled: wallet is being secured."
                );
                wallet.check_rescan_allowed()?;
                node.resync_chain()?;
                wallet.rescan(&node)
            })()
            .map_err(|e| e.to_string());
            syncing.store(false, Ordering::SeqCst);
            if let Ok(mut slot) = signal.lock() {
                *slot = Some(result);
            }
        });
        self.toasts.info(ctx, "Chain resync queued. The existing chain file is preserved by the node; scan errors will be reported.");
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
        let paused = Arc::clone(&self.wallet_paused);
        let access = Arc::clone(&self.wallet_scan_access);
        let data_lock = self.data_lock.clone();

        std::thread::spawn(move || {
            let _data_lock = data_lock;
            let mut seen = node.tip_generation();
            // First pass: pick up whatever is already on disk.
            run_wallet_scan(&wallet, &node, &signal, &syncing, &paused, &access);
            loop {
                seen = node.wait_tip_change(seen, Duration::from_secs(30));
                run_wallet_scan(&wallet, &node, &signal, &syncing, &paused, &access);
            }
        });
    }
}

fn run_wallet_scan(
    wallet: &Arc<Mutex<WalletState>>,
    node: &NodeHandle,
    signal: &Arc<Mutex<Option<Result<u32, String>>>>,
    syncing: &Arc<AtomicBool>,
    paused: &Arc<AtomicBool>,
    access: &Arc<Mutex<()>>,
) {
    if paused.load(Ordering::SeqCst) {
        return;
    }
    let Ok(_access) = access.lock() else {
        return;
    };
    syncing.store(true, Ordering::SeqCst);
    let result = {
        let mut w = match wallet.lock() {
            Ok(w) => w,
            Err(_) => {
                syncing.store(false, Ordering::SeqCst);
                return;
            }
        };
        if paused.load(Ordering::SeqCst) || !w.can_scan() {
            syncing.store(false, Ordering::SeqCst);
            return;
        }
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

    pub fn coin_flash_amount(&self, ctx: &egui::Context) -> f32 {
        let Some(started) = self.coin_flash_at else {
            return 0.0;
        };
        let t = (ctx.input(|i| i.time) - started) as f32;
        if t >= 1.15 {
            0.0
        } else {
            ctx.request_repaint();
            (1.0 - t / 1.15).clamp(0.0, 1.0).powf(1.25)
        }
    }

    fn note_incoming(&mut self, ctx: &egui::Context) {
        let Ok(wallet) = self.wallet.lock() else {
            return;
        };
        if !matches!(
            wallet.custody(),
            crate::wallet_state::Custody::Unlocked | crate::wallet_state::Custody::Legacy
        ) {
            self.activity_primed = false;
            self.activity_seen.clear();
            return;
        }
        let mut arrived = 0u64;
        let mut mined = false;
        for entry in wallet.history() {
            if !self.activity_seen.insert(entry.txid.clone()) {
                continue;
            }
            match entry.direction {
                nightfall_wallet::Direction::Mined | nightfall_wallet::Direction::Received => {
                    arrived = arrived.saturating_add(entry.amount);
                    mined |= entry.direction == nightfall_wallet::Direction::Mined;
                }
                nightfall_wallet::Direction::Sent => {}
            }
        }
        drop(wallet);
        if !self.activity_primed {
            self.activity_primed = true;
            return;
        }
        if arrived == 0 {
            return;
        }
        crate::feedback::play_coin_chime();
        self.coin_flash_at = Some(ctx.input(|i| i.time));
        ctx.request_repaint();
        let amount = arrived as f64 / DARKS_PER_NIGHT as f64;
        if mined {
            self.toasts.success(ctx, format!("Mined {amount:.8} NIGHT"));
        } else {
            self.toasts
                .success(ctx, format!("Received {amount:.8} NIGHT"));
        }
    }

    fn drain_sync_signal(&mut self, ctx: &egui::Context) {
        let taken = self.sync_signal.lock().ok().and_then(|mut s| s.take());
        if let Some(result) = taken {
            match result {
                Ok(n) => {
                    self.wallet_sync_error = None;
                    self.last_sync_at = Some(now_unix());
                    if n > 0 {
                        self.toasts.success(ctx, format!("Found {n} new output(s)"));
                    }
                }
                Err(e) => {
                    self.send_confirm = false;
                    // The persistent banner is the source of truth for a
                    // scan failure. A simultaneous toast covered the lower
                    // dashboard cards on startup and repeated every retry,
                    // making the warning harder to read rather than clearer.
                    self.wallet_sync_error = Some(e);
                }
            }
        }
    }

    pub fn do_send(&mut self, ctx: &egui::Context) {
        if self.wallet_sync_error.is_some() {
            self.toasts.error(ctx, "Sending is blocked until the wallet completes a valid chain scan. Preserve a backup before recovery.");
            return;
        }
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

        if self.vault_ui.busy() || self.wallet_paused.load(Ordering::SeqCst) {
            return;
        }
        self.vault_ui.start_payment(
            self.wallet.clone(),
            self.wallet_paused.clone(),
            crate::vault_ui::PaymentRequest {
                node,
                to: zeroize::Zeroizing::new(self.send_to.trim().to_string()),
                amount: amount_darks,
                fee: self.send_fee,
                memo: zeroize::Zeroizing::new(self.send_memo.trim().to_string()),
            },
            ctx.clone(),
        );
        self.send_busy = self.vault_ui.busy();
        self.send_confirm = false;
        if !self.send_busy {
            if let Some(error) = &self.vault_ui.error {
                self.toasts.error(ctx, error);
            }
        }
    }

    fn payment_feedback(&mut self, result: Result<String, String>, ctx: &egui::Context) {
        self.send_busy = false;
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
                            "Only {peers} peer(s) connected. Confirmation may be delayed. \
                             The pending payment is saved for retry; check Activity before creating another payment."
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

    /// Room for the overlapping rail plus the gap before the main canvas.
    /// Where the page's content starts, measured inside the plate.
    ///
    /// The rail floats over the plate's left edge, so the reserve is simply
    /// how far the rail reaches past that edge, plus a gap. It used to be a
    /// fraction of the window, which was a different quantity that happened
    /// to look right at one size.
    fn nav_reserve_for(width: f32) -> f32 {
        let rail_right =
            crate::widgets::WINDOW_INSET + crate::widgets::RAIL_LEFT + Self::rail_width_for(width);
        let plate_left = crate::widgets::WINDOW_INSET + crate::widgets::PLATE_LEFT;
        (rail_right - plate_left + 28.0).max(24.0)
    }

    fn rail_width_for(width: f32) -> f32 {
        if width < 1100.0 {
            208.0
        } else {
            224.0
        }
    }

    /// Room at the top of the plate for the window buttons and the drag strip.
    ///
    /// This used to be macOS' own title bar. The window has no title bar now —
    /// it has a strip we draw and handle ourselves — so the clearance is the
    /// same on every platform, and it is the inset plus that strip because the
    /// plate starts inside the window.
    /// Room at the top for the window's own close/minimise/zoom buttons.
    ///
    /// The title bar is hidden but the buttons are still there, drawn by macOS
    /// over our content in the top-left corner. Nothing of ours may sit under
    /// them.
    fn title_clearance() -> f32 {
        #[cfg(target_os = "macos")]
        {
            38.0
        }
        #[cfg(not(target_os = "macos"))]
        {
            0.0
        }
    }

    #[allow(deprecated)]
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height();
        egui::Frame::none()
            .fill(glass_rail())
            // A rim, not a drop shadow.
            //
            // Half of this sheet hangs over the desktop, and a shadow of
            // near-black INK painted onto transparent pixels is a dark haze
            // with nothing behind it to soften into. On a bright wallpaper
            // that is the "black line beside the navigation". macOS already
            // casts one shadow for the whole window silhouette, this sheet
            // included, so a second one here only ever showed as an outline.
            .stroke(Stroke::new(1.0_f32, with_alpha(BORDER_HI, 70)))
            .rounding(Rounding::same(ROUND))
            .inner_margin(egui::Margin::symmetric(14.0, 18.0))
            .show(ui, |ui| {
                fill_width(ui, ui.available_width());
                // `height` is the outside of the sheet, including its margins.
                ui.set_min_height((height - 36.0).max(0.0));
                // Reserve the footer as a real rectangle. A bottom-up layout
                // sharing the navigation's space can paint over Settings when
                // the window is short; separate rectangles make that state
                // impossible.
                let inner = ui.max_rect();
                let footer_height = 122.0_f32.min(inner.height() * 0.30);
                let footer_top = (inner.bottom() - footer_height).max(inner.top());
                let nav_rect =
                    egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), footer_top));
                let footer_rect =
                    egui::Rect::from_min_max(egui::pos2(inner.left(), footer_top), inner.max);
                ui.allocate_ui_at_rect(nav_rect, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        logo(ui, 32.0);
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

                    ui.add_space(12.0);

                    for (view, label) in View::ALL {
                        let group = match view {
                            View::Dashboard => Some("Wallet"),
                            View::Mining => Some("Tools"),
                            View::Settings => Some("Preferences"),
                            _ => None,
                        };
                        if let Some(group) = group {
                            ui.add_space(6.0);
                            ui.label(RichText::new(group).size(11.5).color(TEXT_DIM));
                            ui.add_space(2.0);
                        }
                        let selected = self.view == view;
                        let w = ui.available_width();
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(w, 32.0), egui::Sense::click());
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::SelectableLabel,
                                true,
                                selected,
                                label,
                            )
                        });

                        if selected {
                            ui.painter().rect(
                                rect,
                                Rounding::same(ROUND_SM),
                                glass_inner(),
                                Stroke::NONE,
                            );
                        } else if resp.hovered() {
                            ui.painter().rect(
                                rect,
                                Rounding::same(ROUND_SM),
                                glass_hover(),
                                Stroke::NONE,
                            );
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }

                        let fg = if selected { TEXT } else { TEXT_DIM };
                        nav_icon(
                            ui.painter(),
                            view,
                            egui::pos2(rect.min.x + 15.0, rect.center().y - 11.0),
                            fg,
                        );
                        let galley = ui.painter().layout_no_wrap(
                            label.to_string(),
                            egui::FontId::proportional(14.0),
                            fg,
                        );
                        ui.painter().galley(
                            egui::pos2(rect.min.x + 46.0, rect.center().y - galley.size().y / 2.0),
                            galley,
                            fg,
                        );

                        if resp.has_focus() {
                            ui.painter().rect_stroke(
                                rect,
                                Rounding::same(ROUND_SM),
                                Stroke::new(2.0_f32, ACCENT_HI),
                            );
                        }
                        if resp.clicked() {
                            self.view = view;
                            self.reveal_seed = false;
                            self.reveal_mnemonic = false;
                            self.reveal_view_key = false;
                        }
                    }
                });

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
                ui.allocate_ui_at_rect(footer_rect, |ui| {
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        let loading = self.status.as_ref().map(|s| s.loading).unwrap_or(false);
                        let supply_ok = self.status.as_ref().map(|s| s.supply_ok).unwrap_or(false);

                        // Colour follows the truth. While the chain loads nothing
                        // has been checked yet, and the old code passed ok = true
                        // for that case — a green dot and green lettering next to
                        // the word "loading". Green here means the sum was
                        // recomputed and balanced. Until it has been, this is dim.
                        let (label, colour) = if self.status.is_none() {
                            ("Waiting for node", TEXT_DIM)
                        } else if loading {
                            ("Checking supply…", TEXT_DIM)
                        } else if supply_ok {
                            ("Supply verified", SUCCESS)
                        } else {
                            ("Supply not verified", DANGER)
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
                    });
                });
            });
    }

    fn float_sidebar(&mut self, ctx: &egui::Context) {
        // The rail deliberately stands outside the plate, so it is placed
        // against the window rather than against the gutter-trimmed page.
        let work = ctx.screen_rect();
        // The rail is a floating sheet in the reference composition. Keep a
        // generous breathing space above and below it so it never becomes a
        // full-height strip, even when the window is maximised.
        let edge = (work.height() * 0.10).clamp(40.0, 120.0);
        let top = edge + Self::title_clearance();
        let bottom = edge;
        // Keep the rail near the left edge while retaining a small breathing
        // line at every size. The content axis is reserved independently, so
        // moving this sheet never compresses the page cards.
        // Far enough from the window's own edge that the rail's shadow can
        // finish before it. At 24 points a 28-point blur was cut off by the
        // frame, and a clipped shadow ends on its darkest values — a dark
        // seam down the left side, and an alpha edge macOS then built its own
        // window shadow from.
        let left = crate::widgets::WINDOW_INSET + crate::widgets::RAIL_LEFT;
        let rail_width = Self::rail_width_for(work.width());
        let pos = egui::pos2(work.left() + left, work.top() + top);
        let height = (work.height() - top - bottom).max(240.0);
        egui::Area::new(egui::Id::new("nightfall-nav"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .movable(false)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(Vec2::new(rail_width, height));
                ui.set_max_size(Vec2::new(rail_width, height));
                self.sidebar(ui);
            });
    }

    fn topbar(&mut self, ctx: &egui::Context) {
        let reserve = Self::nav_reserve_for(ctx.available_rect().width());
        egui::TopBottomPanel::top("top")
            .exact_height(94.0 + Self::title_clearance())
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin {
                        left: reserve,
                        right: 24.0,
                        top: Self::title_clearance(),
                        bottom: 8.0,
                    }),
            )
            .show(ctx, |ui| {
                let bar = ui.spacing().scroll.allocated_width();
                let width = (ui.available_width() - bar).max(0.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        fill_width(ui, width);
                        egui::Frame::none()
                            .fill(glass_surface())
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND))
                            .inner_margin(egui::Margin::symmetric(22.0, 12.0))
                            .show(ui, |ui| {
                                fill_width(ui, (width - 44.0).max(0.0));
                                ui.horizontal_centered(|ui| {
                                    let title = View::ALL
                                        .iter()
                                        .find(|(v, _)| *v == self.view)
                                        .map(|(_, l)| *l)
                                        .unwrap_or("");
                                    let detail_width = (ui.available_width() - 255.0).max(220.0);
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(detail_width, 54.0),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            ui.set_width(detail_width);
                                            ui.add_space(-2.0);
                                            ui.label(RichText::new(title).size(24.0).strong());
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(views::page_description(
                                                        self.view,
                                                    ))
                                                    .size(13.0)
                                                    .color(TEXT_DIM),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(views::page_description(self.view));
                                        },
                                    );

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let loading = self
                                                .status
                                                .as_ref()
                                                .map(|s| s.loading)
                                                .unwrap_or(false);
                                            if self.vault_ui.custody == Custody::Unlocked
                            && ghost_button(ui, "Lock wallet")
                                .on_hover_text("Lock immediately · ⌘L on macOS, Ctrl+L elsewhere")
                                .clicked()
                        {
                            self.request_vault(crate::vault_ui::Action::Lock, ctx);
                        }
                                            ui.add_space(4.0);

                                            // Chain height + sync indicator
                                            let syncing = self.syncing.load(Ordering::SeqCst);
                                            let blocks =
                                                self.status.as_ref().map(|s| s.blocks).unwrap_or(0);
                                            ui.horizontal(|ui| {
                                                dot(
                                                    ui,
                                                    if self.status_error.is_some() {
                                                        DANGER
                                                    } else if self.status.is_none() || loading {
                                                        TEXT_DIM
                                                    } else if syncing
                                                        || self.wallet_sync_error.is_some()
                                                    {
                                                        WARN
                                                    } else if IS_DEV_BUILD {
                                                        ACCENT_HI
                                                    } else {
                                                        SUCCESS
                                                    },
                                                    syncing,
                                                );
                                                ui.add_space(2.0);
                                                ui.label(
                                                    RichText::new(if self.status.is_none() {
                                                        "Node starting".into()
                                                    } else if IS_DEV_BUILD && !IS_DEV_MAINNET {
                                                        format!(
                                                            "Local · {} blocks",
                                                            format_int(blocks)
                                                        )
                                                    } else {
                                                        format!("{} blocks", format_int(blocks))
                                                    })
                                                    .size(12.5)
                                                    .color(TEXT_DIM),
                                                );
                                            });
                                        },
                                    );
                                });
                            });
                    },
                );
            });
    }
}

impl eframe::App for App {
    /// Clear to nothing, so the desktop shows through outside the plate.
    ///
    /// This is what kept the window a rectangle. `with_transparent(true)` only
    /// asks for a surface with an alpha channel; what is *written* into that
    /// channel every frame is this. eframe's default is `rgba(12, 12, 12, 180)`
    /// — a dark grey at seventy percent — so the whole window was flooded with
    /// it and the rounded plate sat inside a visible dark box. The rounded
    /// corners, the shadow and the rail's overhang were all being drawn
    /// correctly the whole time, onto an opaque background.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply(ctx);
        paint_room(ctx);
        // Before any early return below: a screen that cannot be moved or
        // closed is a trap, and the lock screen and onboarding are screens.

        // Claims the margin so every panel below lands on the plate rather
        // than on the window. Without it the lock screen's header hung out
        // past the rounded corner.
        crate::widgets::place_window_buttons(ctx);
        crate::widgets::plate_gutters(ctx);
        // Out of sight is *observed*, never remembered.
        //
        // This was a remembered flag, set when the window was minimised and
        // cleared only by the menu-bar item's Show. Restore it from the Dock
        // instead — which is the way the operating system offers, and the one
        // an owner reaches for — and nothing cleared it. `observe_activity`
        // begins with `if !focused || hidden { self.clear_fields() }`, so a
        // stale flag wiped the password field on every single frame: the
        // wallet came back and could not be typed into, as fast as anyone
        // could type. Asking the window each frame makes the whole question of
        // who resets it disappear.
        self.window_hidden = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        if let Some(error) = &self.wallet_load_error {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(Color32::TRANSPARENT)
                        .inner_margin(egui::Margin::same(28.0)),
                )
                .show(ctx, |ui| wallet_load_error_panel(ui, error));
            return;
        }
        self.handle_tray(ctx);
        if self.vault_gate(ctx) {
            return;
        }
        if self.vault_ui.custody == Custody::Unlocked
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::L))
        {
            self.request_vault(crate::vault_ui::Action::Lock, ctx);
            return;
        }
        let access = self.wallet_scan_access.clone();
        let Ok(_frame_access) = access.try_lock() else {
            self.scan_wait_panel(ctx);
            return;
        };
        if self.view != View::Settings || self.window_hidden || !ctx.input(|i| i.raw.focused) {
            self.recovery_studio.clear();
        }
        self.poll_status();
        self.note_incoming(ctx);
        self.drain_sync_signal(ctx);

        // Keep the UI live for hashrate, tray clicks, and sync animation.
        ctx.request_repaint_after(Duration::from_millis(500));

        if self.onboarding.is_some() {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(Color32::TRANSPARENT)
                        .inner_margin(egui::Margin::same(28.0)),
                )
                .show(ctx, |ui| {
                    screen_header(
                        ui,
                        &self.network.to_string(),
                        &[
                            (
                                "Nothing is written until your words are confirmed",
                                TEXT_FAINT,
                            ),
                            (WALLET_VERSION, TEXT_FAINT),
                        ],
                    );
                    ui.add_space(GAP_LG);
                    // Onboarding is the one full-window screen that never had a
                    // scroll area, and it is also the tallest: on a 812-point
                    // window the restore card ran past the bottom edge, so
                    // "Save encrypted wallet" sat on the last visible line and
                    // "Cancel setup" was simply not reachable. A person setting
                    // up a wallet could not back out of it.
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .show(ui, |ui| views::onboarding(self, ui, ctx));
                });
            self.toasts.show(ctx);
            return;
        }

        self.float_sidebar(ctx);
        self.topbar(ctx);
        // The toolbar's lock must conceal page data in this same frame.
        if self.vault_ui.busy() || self.vault_ui.needs_lock() {
            ctx.request_repaint();
            return;
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin {
                        left: Self::nav_reserve_for(ctx.available_rect().width()),
                        right: 24.0,
                        top: 16.0,
                        bottom: 28.0,
                    }),
            )
            .show(ctx, |ui| {
                let forced = self.shots.as_ref().and_then(|s| s.offset());
                let mut area = egui::ScrollArea::vertical()
                    .id_salt(("page-scroll", self.view as u8))
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                    );
                if let Some(offset) = forced {
                    area = area.vertical_scroll_offset(offset);
                }
                let drawn = area.show(ui, |ui| {
                page_column(ui, self.view.content_max_width(), |ui| {
                    let banner_w = ui.available_width();

                    if let Some(error) = &self.wallet_sync_error {
                        let detailed = matches!(self.view, View::Send);
                        egui::Frame::none()
                            .fill(glass_alert(WARN))
                            .stroke(Stroke::NONE)
                            .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                            .rounding(Rounding::same(ROUND))
                            .show(ui, |ui| {
                                fill_width(ui, (banner_w - 32.0).max(0.0));
                                egui::CollapsingHeader::new(
                                    RichText::new(
                                        "Wallet scan incomplete — balances and confirmations may be stale",
                                    )
                                    .color(WARN)
                                    .size(12.5),
                                )
                                // Per page, so the open state on Dashboard does not
                                // decide the state on Receive — a shared id made
                                // `default_open` apply once, to whichever page was
                                // shown first, and the rest inherited it.
                                .id_salt(("scan-warning", self.view as u8))
                                .default_open(detailed)
                                .show(ui, |ui| {
                                    ui.label(RichText::new(error).size(13.0).color(TEXT));
                                    ui.label(RichText::new("Sending is blocked until a valid scan completes. Preserve an encrypted backup; do not clear pending payments or reservations to bypass this warning.").size(13.0).color(TEXT));
                                });
                            });
                        ui.add_space(GAP_SM);
                    }

                    // A development build looking at real money says so, on every
                    // page, in the colour reserved for things that cannot be
                    // undone. It is not a toast and it does not dismiss: the risk
                    // lasts as long as the build does.
                    if IS_DEV_MAINNET {
                        egui::Frame::none()
                            .fill(glass_alert(DANGER))
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND))
                            .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                            .show(ui, |ui| {
                                fill_width(ui, (banner_w - 28.0).max(0.0));
                                ui.label(
                                    RichText::new(format!(
                                        "Development build {WALLET_VERSION} on mainnet, using your \
                                         real wallet directory. Once it saves, the released 0.9.5 \
                                         app can no longer read this wallet — an encrypted backup \
                                         is the only way back. Never run both at once.",
                                    ))
                                    .color(DANGER)
                                    .size(12.5),
                                );
                            });
                        ui.add_space(GAP_SM);
                    }

                    if let Some(err) = self.status_error.clone() {
                        egui::Frame::none()
                            .fill(glass_alert(DANGER))
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND))
                            .inner_margin(egui::Margin::same(14.0))
                            .show(ui, |ui| {
                                fill_width(ui, (banner_w - 24.0).max(0.0));
                                ui.label(RichText::new(format!("Node error: {err}")).color(DANGER));
                            });
                        ui.add_space(12.0);
                    }

                    fill_width(ui, ui.available_width());
                    views::page_intro(self.view, ui);
                    match self.view {
                        View::Dashboard => views::dashboard(self, ui),
                        View::Send => views::send(self, ui, ctx),
                        View::Receive => views::receive(self, ui, ctx),
                        View::Activity => views::activity(self, ui),
                        View::Mining => views::mining(self, ui),
                        View::Network => views::network(self, ui, ctx),
                        View::Settings => views::settings(self, ui, ctx),
                    }
                });
                });
                if let Some(shots) = self.shots.as_mut() {
                    shots.note(crate::ui_shots::Area {
                        viewport: drawn.inner_rect,
                        content: drawn.content_size.y,
                        offset: drawn.state.offset.y,
                    });
                }
            });

        self.toasts.show(ctx);
        crate::ui_shots::step(self, ctx);
    }
}

fn wallet_load_error_panel(ui: &mut egui::Ui, error: &str) {
    titled_card(ui, "WALLET FILES PRESERVED", |ui| {
        ui.heading("Wallet could not be opened");
        ui.add_space(16.0);
        ui.colored_label(DANGER, error);
        ui.add_space(16.0);
        ui.label("No replacement wallet was created. Keep the original wallet files and resolve this error before restarting.");
        ui.add_space(8.0);
        ui.label("Do not delete a vault directory to bypass this protection. Use a compatible build and preserve the original files for recovery.");
    });
}

impl App {
    fn scan_wait_panel(&mut self, ctx: &egui::Context) -> egui::Rect {
        ctx.request_repaint_after(Duration::from_millis(100));
        // Do not read wallet/node state here: the scanner may own both locks.
        // Conceal any previously revealed secrets while displaying progress.
        self.reveal_seed = false;
        self.reveal_mnemonic = false;
        self.reveal_view_key = false;
        self.recovery_studio.clear();
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
            ui.add_space(28.0);
            narrow_column(ui, 620.0, |ui| {
                titled_card(ui, "UPDATING WALLET", |ui| {
                    ui.spinner();
                    ui.heading("Scanning the local chain");
                    ui.label("Balances and Activity will return after the scan is saved. No estimated balance is shown while it is being updated.");
                    ui.add_space(12.0);
                    if self.vault_ui.custody == Custody::Unlocked && ui.button("Lock wallet").clicked() {
                        self.request_vault(crate::vault_ui::Action::Lock, ctx);
                    }
                    ui.label("Locking hides the wallet immediately; an active scan finishes before its in-memory keys are released.");
                });
            });
            ui.min_rect()
        }).inner
    }

    pub fn request_vault(&mut self, action: crate::vault_ui::Action, ctx: &egui::Context) {
        if action == crate::vault_ui::Action::Lock || action == crate::vault_ui::Action::Prepare {
            self.clear_wallet_views();
            ctx.data_mut(|data| data.remove::<String>(egui::Id::new("counter-confirm-remove")));
        }
        self.vault_ui.start(
            action,
            self.wallet.clone(),
            self.wallet_paused.clone(),
            self.data_lock.clone(),
            self.network,
            ctx.clone(),
        );
    }

    fn clear_wallet_views(&mut self) {
        self.reveal_seed = false;
        self.reveal_mnemonic = false;
        self.reveal_view_key = false;
        self.recovery_studio.clear();
        // A Proof Card names an address, an amount and a memo. None of them is
        // a key, and all of them are this wallet's business rather than the
        // next person's at the same screen.
        self.proof_input.zeroize();
        self.proof_result = None;
        self.till_reference.zeroize();
        self.till_amount.zeroize();
        self.till_description.zeroize();
        // An Air package names an address and an amount, and the frames on
        // screen are a transaction. None of it is a key; all of it is this
        // wallet's business rather than the next person's at the same screen.
        self.air_input.zeroize();
        self.air_incoming = None;
        self.air_frames.clear();
        self.air_frame_at = 0;
        self.air_note = None;
        self.air_error = None;
        self.send_to.zeroize();
        self.send_amount.zeroize();
        self.send_memo.zeroize();
        self.send_confirm = false;
        self.book_name.zeroize();
        self.book_addr.zeroize();
        self.activity_filter.zeroize();
        self.toasts.clear();
        if let Ok(mut signal) = self.sync_signal.try_lock() {
            *signal = None;
        }
    }

    pub fn show_vault_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.vault_ui.refresh(&self.wallet);
        let available = cfg!(unix) && self.data_lock.is_some();
        if let Some(action) = self.vault_ui.show(ui, available) {
            self.request_vault(action, ctx);
        }
    }

    fn vault_gate(&mut self, ctx: &egui::Context) -> bool {
        self.vault_ui.refresh(&self.wallet);
        let (focused, activity) = ctx.input(|input| (input.raw.focused, !input.events.is_empty()));
        if let Some(setup) = &mut self.onboarding {
            setup.observe_activity(focused, self.window_hidden, activity, Instant::now());
        }
        let auto_lock =
            self.vault_ui
                .observe_activity(focused, self.window_hidden, activity, Instant::now());
        if auto_lock || self.vault_ui.needs_lock() {
            self.request_vault(crate::vault_ui::Action::Lock, ctx);
        }
        if self.vault_ui.poll(&self.wallet, &self.wallet_paused) {
            if self.vault_ui.has_snapshot && self.onboarding.is_some() {
                self.onboarding = None;
                if self.vault_ui.custody == Custody::Locked && self.vault_ui.error.is_none() {
                    self.ack_backup();
                }
            }
            if self.vault_ui.custody == Custody::Unlocked && self.node.is_none() {
                self.start_node();
            }
        }
        if let Some(result) = self.vault_ui.payment_result.take() {
            self.send_busy = false;
            if matches!(self.vault_ui.custody, Custody::Unlocked | Custody::Legacy) {
                self.payment_feedback(result, ctx);
            } else {
                self.vault_ui.error = Some("A payment operation finished while the wallet was being secured. Unlock and check Activity before creating another payment.".into());
            }
        }
        let blocked = self.vault_ui.busy()
            || self.vault_ui.needs_lock()
            || matches!(self.vault_ui.custody, Custody::Locked | Custody::Failed)
            || (self.vault_ui.custody == Custody::Migration && self.onboarding.is_none())
            || (self.vault_ui.custody == Custody::Empty && self.onboarding.is_none());
        if blocked {
            if self.shots.is_some() {
                // Dev capture only: photograph the live chrome with empty
                // fixture data. Production never takes this branch.
                return false;
            }
            self.wallet_paused.store(true, Ordering::SeqCst);
            ctx.request_repaint_after(Duration::from_millis(100));
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(Color32::TRANSPARENT)
                        .inner_margin(egui::Margin {
                            left: 28.0,
                            right: 28.0,
                            top: 28.0 + Self::title_clearance(),
                            bottom: 28.0,
                        }),
                )
                .show(ctx, |ui| {
                    let tip = self.tip_height();
                    let peers = self.status.as_ref().map(|s| s.peers).unwrap_or(0);
                    screen_header(
                        ui,
                        &self.network.to_string(),
                        &[
                            (
                                &if tip > 0 {
                                    format!("Chain height {}", format_int(tip))
                                } else {
                                    "Reading the chain".to_owned()
                                },
                                if tip > 0 { SUCCESS } else { TEXT_FAINT },
                            ),
                            (
                                &format!("{peers} peers"),
                                if peers > 0 { TEXT_DIM } else { TEXT_FAINT },
                            ),
                            (WALLET_VERSION, TEXT_FAINT),
                        ],
                    );
                    ui.add_space(GAP_XL);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .show(ui, |ui| {
                            narrow_column(ui, 620.0, |ui| self.show_vault_settings(ui, ctx));
                        });
                });
        } else if self.view != View::Settings {
            self.vault_ui.clear_fields();
        }
        blocked
    }

    /// Rough time left on the chain load, phrased as a person would say it.
    ///
    /// Measured, not assumed: the first sighting of the load is remembered,
    /// and the rate since then is extrapolated. Returns `None` until there is
    /// enough of a sample to be worth showing — an estimate from two seconds
    /// of data swings between "1 minute" and "3 hours" and teaches people to
    /// ignore the number.
    pub fn load_eta(&mut self, done: u64, total: u64) -> Option<String> {
        if total == 0 || done >= total {
            return None;
        }
        let (started, from) = *self.load_started.get_or_insert((Instant::now(), done));
        let elapsed = started.elapsed().as_secs_f64();
        let progressed = done.saturating_sub(from);
        if elapsed < 20.0 || progressed < 200 {
            return None;
        }
        let per_sec = progressed as f64 / elapsed;
        if per_sec <= 0.0 {
            return None;
        }
        let secs = (total - done) as f64 / per_sec;
        Some(if secs < 90.0 {
            "a minute".to_string()
        } else if secs < 5400.0 {
            format!("{} minutes", (secs / 60.0).round() as u64)
        } else {
            format!("{:.1} hours", secs / 3600.0)
        })
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            self.tray = Tray::new();
        }
        if let Some(tray) = &self.tray {
            match tray.poll() {
                Some(TrayAction::Show) => {
                    // No flag to clear here: it is read from the window.
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
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
        if ctx.input(|i| i.viewport().close_requested()) && self.close_to_tray {
            // Minimised, not hidden — and that is a measured distinction, not
            // a preference.
            //
            // `Visible(false)` took the last window off the screen, and with
            // it the event loop: a probe logging a heartbeat every forty
            // frames printed nothing more after the window went away, and the
            // process was gone seconds later. So "close to tray" did not keep
            // the wallet running in the background at all — it ended it. And
            // the way back was supposed to be the menu-bar item, whose polling
            // lives in `update`, in the very loop that had just stopped. A
            // window nobody could restore and a menu that could never answer.
            //
            // A minimised window keeps the loop, the process and the mining
            // alive — verified the same way — and the operating system itself
            // brings it back from the Dock or the taskbar, with no polling of
            // ours in the path.
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
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
    // Off by default, so the red button and the yellow one mean different
    // things: red closes the wallet, yellow puts it in the Dock and mining
    // carries on. With this on they both minimised, which is one button too
    // many for one behaviour.
    load_flag(datadir, "close_to_tray", false)
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

fn arg_or(flag: &str, default: String) -> String {
    args_all(flag).into_iter().next().unwrap_or(default)
}

fn args_all(flag: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    let eq = format!("{flag}=");
    let mut out = Vec::new();
    for (i, a) in args.iter().enumerate() {
        if a == flag {
            if let Some(v) = args.get(i + 1) {
                if !v.starts_with('-') {
                    out.push(v.clone());
                }
            }
        } else if let Some(v) = a.strip_prefix(&eq) {
            out.push(v.to_string());
        }
    }
    out
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
/// The rules and the wording live in `nightfall_wallet::amount_input`, which
/// Core, the browser wallet and the mobile wallet all call. They used to have
/// one of these each, and the copies had drifted: `.5` was half a NIGHT in two
/// of them and an error in this one, and `+5` was five NIGHT in two of them and
/// an error here. Two wallets from one project disagreeing about what a typed
/// amount means is a defect regardless of which reading is the better one.
pub fn parse_amount(s: &str) -> Result<u64, String> {
    nightfall_wallet::amount_input::parse_night(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_amount, App};
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

    /// The page never runs under the rail that floats over it.
    ///
    /// This used to pin the old formula's output — a fraction of the window
    /// width — which stopped meaning anything once the rail became a sheet
    /// standing outside the plate. The number to hold is not 286.7; it is that
    /// the content starts clear of wherever the rail actually reaches, at
    /// every width, with a gap wide enough to read as a gap.
    #[test]
    fn the_page_starts_clear_of_the_floating_rail() {
        for width in [940.0_f32, 1024.0, 1100.0, 1280.0, 1600.0, 1920.0, 2560.0] {
            let rail_right = crate::widgets::WINDOW_INSET
                + crate::widgets::RAIL_LEFT
                + App::rail_width_for(width);
            let plate_left = crate::widgets::WINDOW_INSET + crate::widgets::PLATE_LEFT;
            let overhang = rail_right - plate_left;
            let reserve = App::nav_reserve_for(width);
            assert!(
                reserve >= overhang + 20.0,
                "at {width}px the page starts {reserve} into the plate while the \
                 rail reaches {overhang} — the first card would sit under it",
            );
        }
        // The rail itself gets out of the way on a narrow window.
        assert_eq!(App::rail_width_for(1024.0), 208.0);
        assert_eq!(App::rail_width_for(1280.0), 224.0);
        assert!(App::nav_reserve_for(1024.0) < App::nav_reserve_for(1280.0));
    }

    /// The two spellings that used to depend on which wallet you were holding.
    /// Core now reads `.5` as half a NIGHT, where it used to refuse it, and the
    /// browser and mobile wallets now refuse `+5`, where they used to read it
    /// as five. Pinned here as well as in `amount_input` because this is where
    /// someone looking for Core's behaviour will look.
    #[test]
    fn agrees_with_the_other_wallets_about_the_two_awkward_spellings() {
        assert_eq!(parse_amount(".5").unwrap(), DARKS_PER_NIGHT / 2);
        assert!(parse_amount("+5").is_err());
    }

    #[test]
    fn scan_failure_stays_visible_until_success_and_discards_confirmation() {
        use super::*;
        let root =
            std::env::temp_dir().join(format!("nightfall-scan-warning-{}", std::process::id()));
        let mut app = App::new(NetworkId::Devnet, root);
        let ctx = egui::Context::default();
        app.send_confirm = true;
        *app.sync_signal.lock().unwrap() =
            Some(Err("public test: canonical anchor changed".into()));
        app.drain_sync_signal(&ctx);
        assert!(!app.send_confirm);
        assert!(app.wallet_sync_error.is_some());
        app.drain_sync_signal(&ctx);
        app.poll_status();
        assert!(app.wallet_sync_error.is_some());
        assert!(app.last_sync_at.is_none());
        *app.sync_signal.lock().unwrap() = Some(Ok(0));
        app.drain_sync_signal(&ctx);
        assert!(app.wallet_sync_error.is_none());
        assert!(app.last_sync_at.is_some());
        assert!(app.node.is_none());
    }

    #[test]
    fn scan_progress_renders_without_waiting_for_wallet_and_conceals_secrets() {
        use super::*;
        let root = std::env::temp_dir().join(format!(
            "nightfall-scan-ui-{}-{}",
            std::process::id(),
            nightfall_storage::now_unix()
        ));
        let mut app = App::new(NetworkId::Devnet, root);
        assert!(app.node.is_none());
        let wallet = app.wallet.clone();
        let access = app.wallet_scan_access.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let scanner = std::thread::spawn(move || {
            let _access = access.lock().unwrap();
            let _wallet = wallet.lock().unwrap();
            ready_tx.send(()).unwrap();
            // Bounds a regression that accidentally takes either lock in UI.
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(app.wallet_scan_access.try_lock().is_err());
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let started = Instant::now();
        for width in [320.0, 620.0, 1180.0] {
            app.reveal_seed = true;
            app.reveal_mnemonic = true;
            app.reveal_view_key = true;
            let mut used = egui::Rect::NOTHING;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 780.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    used = app.scan_wait_panel(ctx);
                },
            );
            assert!(!app.reveal_seed && !app.reveal_mnemonic && !app.reveal_view_key);
            assert!(
                used.right() <= width + 1.0,
                "scan panel overflow: width={width}, rect={used:?}"
            );
        }
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        scanner.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(5),
            "UI waited for the scan: {elapsed:?}"
        );
        assert!(app.wallet_scan_access.try_lock().is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn onboarding_publication_discards_setup_and_never_starts_a_node() {
        use super::*;
        // The clock alone is not a unique name: these run in parallel
        // threads of one process, and the platform does not hand each of
        // them a distinct nanosecond. Two fixtures then race for the same
        // directory and one loses. The counter makes the name unique by
        // construction; the clock stays so leftovers remain readable.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nightfall-onboarding-shell-{}-{nonce}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());
        let mut app = App::with_data_lock(NetworkId::Devnet, lock.clone());
        assert!(app.onboarding.is_some() && app.node.is_none());
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        app.vault_ui.start_provision(
            app.wallet.clone(),
            app.wallet_paused.clone(),
            Some(lock.clone()),
            NetworkId::Devnet,
            crate::onboarding::ProvisionRequest {
                source: crate::onboarding::ProvisionSource::Words(zeroize::Zeroizing::new(
                    nightfall_crypto::WalletKeys::from_seed([0; 32]).to_mnemonic(),
                )),
                password: zeroize::Zeroizing::new("public unfunded shell test password".into()),
            },
            ctx.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        while app.vault_ui.busy() {
            let _ = ctx.run(
                egui::RawInput {
                    focused: false,
                    ..Default::default()
                },
                |ctx| {
                    assert!(app.vault_gate(ctx));
                },
            );
            assert!(app.node.is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.vault_ui.error.is_none());
        assert!(app.onboarding.is_none() && app.backup_acked);
        assert_eq!(app.vault_ui.custody, Custody::Locked);
        assert!(app.wallet_paused.load(Ordering::SeqCst));
        drop(app);
        let app = App::with_data_lock(NetworkId::Devnet, lock.clone());
        assert!(app.node.is_none() && app.onboarding.is_none());
        assert!(app.backup_acked && app.wallet_paused.load(Ordering::SeqCst));
        drop(app);
        drop(lock);
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("nightfall-onboarding-shell-"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wallet_startup_errors_do_not_start_a_node_or_offer_a_replacement() {
        use super::App;
        use nightfall_types::NetworkId;
        use std::fs;
        for case in ["vault", "broken-db", "orphan-db", "interrupted-save"] {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "nightfall-startup-test-{}-{nonce}-{case}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            match case {
                "vault" => {
                    fs::create_dir(root.join("core.seed.vault")).unwrap();
                }
                "broken-db" => {
                    fs::write(root.join("core.seed"), "00".repeat(32)).unwrap();
                    fs::write(root.join("core.seed.outputs.json"), b"broken JSON").unwrap();
                }
                "orphan-db" => {
                    fs::write(
                        root.join("core.seed.outputs.json"),
                        br#"{"outputs":[],"scanned_to":75}"#,
                    )
                    .unwrap();
                }
                "interrupted-save" => {
                    fs::write(
                        root.join("core.seed.outputs.json.tmp"),
                        b"unfinished database",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let app = App::new(NetworkId::Devnet, root.clone());
            assert!(app.wallet_load_error.is_some(), "{case}");
            assert!(app.onboarding.is_none(), "{case}");
            assert!(app.node.is_none(), "{case}");
            assert!(app.wallet.lock().unwrap().address().is_none(), "{case}");
            assert!(!root.join("blocks.bin").exists());
            if case != "broken-db" {
                assert!(!root.join("core.seed").exists());
            } else {
                assert_eq!(
                    fs::read(root.join("core.seed.outputs.json")).unwrap(),
                    b"broken JSON"
                );
            }
            drop(app);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn wallet_startup_error_card_fits_supported_widths() {
        use eframe::egui;
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for width in [320.0, 620.0, 884.0] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let right = ui.max_rect().right();
                    super::wallet_load_error_panel(ui, "Encrypted wallet or interrupted vault migration found. Plaintext fallback and replacement seeds are disabled.");
                    assert!(ui.min_rect().right() <= right + 1.0, "overflow at {width}");
                });
            });
        }
    }
}
