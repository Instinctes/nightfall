//! One background owner for chain queries and swap broadcasts. UI uses snapshots.
use crate::app::App;
use crate::app_swap_send::SendWhat;
use crate::wallet_state::WalletState;
use nightfall_node::NodeHandle;
use nightfall_swap::{session::Session, StoredSwap};
use nightfall_types::NetworkId;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub enum Job {
    Tick,
    LockNight(String),
    BroadcastLock(String, String),
    Send(String, SendWhat),
}

#[derive(Default)]
pub struct Report {
    pub bitcoin: ResultLabel,
    pub swaps: HashMap<String, Snapshot>,
    pub action: Option<Result<String, String>>,
    pub error: Option<String>,
}

#[derive(Default)]
pub enum ResultLabel {
    #[default]
    Checking,
    Ready(String),
    Error(String),
}

#[derive(Default)]
pub struct Snapshot {
    pub btc_depth: Option<u32>,
    pub night_depth: Option<u64>,
    pub error: Option<String>,
}

pub struct SwapWorker {
    pub network: NetworkId,
    pub datadir: PathBuf,
    pub node: Option<Arc<NodeHandle>>,
    pub wallet: Arc<Mutex<WalletState>>,
    pub swap_sessions: HashMap<String, Session>,
}

impl SwapWorker {
    pub fn from_app(app: &App) -> Self {
        Self {
            network: app.network,
            datadir: app.datadir.clone(),
            node: app.node.clone(),
            wallet: app.wallet.clone(),
            swap_sessions: HashMap::new(),
        }
    }

    pub fn ensure_session(&mut self, id: &str) -> Result<(), String> {
        if !self.swap_sessions.contains_key(id) {
            let uuid = id.parse().map_err(|_| "Invalid swap ID")?;
            let s = Session::load(&self.datadir, uuid).map_err(|e| e.to_string())?;
            if s.network != self.network {
                return Err("Swap belongs to another network.".into());
            }
            self.swap_sessions.insert(id.into(), s);
        }
        Ok(())
    }

    fn load(&self, id: &str) -> Result<StoredSwap, String> {
        nightfall_swap::persist::load(&self.datadir, id.parse().map_err(|_| "Invalid swap ID")?)
            .map_err(|e| e.to_string())
    }

    pub fn run(&mut self, job: Job) -> Report {
        let mut report = Report::default();
        if !nightfall_swap::ui::availability(self.network).is_enabled() {
            return report;
        }
        report.bitcoin = match self
            .btc_rpc()
            .and_then(|rpc| rpc.check_network(self.network).map_err(|e| e.to_string()))
        {
            Ok(label) => ResultLabel::Ready(label),
            Err(e) => ResultLabel::Error(e),
        };
        report.action = match job {
            Job::Tick => None,
            Job::LockNight(id) => Some(self.load(&id).and_then(|s| self.lock_night(&s))),
            Job::BroadcastLock(id, raw) => {
                Some(self.load(&id).and_then(|s| self.broadcast_lock(&s, &raw)))
            }
            Job::Send(id, what) => Some(self.load(&id).and_then(|s| self.send_swap_tx(&s, what))),
        };
        let list = match nightfall_swap::persist::list(&self.datadir) {
            Ok(v) => v,
            Err(e) => {
                report.error = Some(e.to_string());
                return report;
            }
        };
        for stored in list {
            let id = stored.state.id().to_string();
            let mut snap = Snapshot::default();
            let result = self.ensure_session(&id).and_then(|()| {
                let s = &self.swap_sessions[&id];
                if s.id != stored.state.id()
                    || s.role != stored.state.role()
                    || s.amounts.night_darks != stored.night_darks
                    || s.amounts.btc_sats != stored.btc_sats
                {
                    return Err(
                        "Session and swap record disagree. Restore a matching backup.".into(),
                    );
                }
                if stored.is_finished() {
                    return Ok(());
                }
                self.tick_one_swap(stored.clone())
            });
            if let Err(e) = result {
                snap.error = Some(e);
            }
            let current = self.load(&id).unwrap_or(stored);
            if let (Ok(rpc), Some(txid)) = (self.btc_rpc(), &current.btc_lock_txid) {
                use nightfall_swap::{watch::TxRef, ChainWatch};
                snap.btc_depth = rpc
                    .confirmations(&TxRef { id: txid.clone() })
                    .ok()
                    .flatten()
                    .map(|n| n.min(u64::from(u32::MAX)) as u32);
            }
            snap.night_depth = current
                .night_lock_id
                .as_ref()
                .and_then(|id| self.night_lock_depth(id));
            report.swaps.insert(id, snap);
        }
        report
    }
}

impl App {
    pub fn queue_swap_job(&mut self, job: Job) -> Result<(), String> {
        if self.wallet_paused.load(std::sync::atomic::Ordering::SeqCst)
            || !self
                .wallet
                .lock()
                .map(|wallet| wallet.custody() == crate::wallet_state::Custody::Legacy)
                .unwrap_or(false)
        {
            return Err("Experimental swaps require a legacy wallet; Vault swap-secret integration is not released.".into());
        }
        if self.swap_job.is_some() {
            return Err("The swap worker is busy. Please wait for the current check.".into());
        }
        if !nightfall_swap::ui::availability(self.network).is_enabled() {
            return Err("Swaps are disabled on this network.".into());
        }
        let mut worker = SwapWorker::from_app(self);
        self.swap_job = Some(std::thread::spawn(move || worker.run(job)));
        Ok(())
    }
}
