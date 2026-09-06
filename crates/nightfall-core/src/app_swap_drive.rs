//! Drive every open swap: watch both chains, persist, then act.

use crate::app::App;
use crate::swap_worker::{Job, SwapWorker};
use bitcoin::consensus::encode::deserialize;
use nightfall_crypto::Commitment;
use nightfall_swap::driver::{self, Tick};
use nightfall_swap::night_observe;
use nightfall_swap::night_watch::{NightSnap, NightWatch};
use nightfall_swap::persist::{self, SendKind, StoredSwap};
use nightfall_swap::ui as logic;
use nightfall_swap::watch::{BroadcastResult, Broadcaster};
use nightfall_swap::Role;
use std::collections::HashMap;
use std::time::{Duration, Instant};

impl App {
    pub fn tick_swaps(&mut self, ctx: &eframe::egui::Context) {
        if self.swap_job.as_ref().is_some_and(|j| j.is_finished()) {
            match self.swap_job.take().expect("completed job").join() {
                Ok(report) => {
                    if let Some(result) = &report.action {
                        match result {
                            Ok(msg) => self.toasts.success(ctx, msg),
                            Err(e) => self.toasts.error(ctx, e),
                        }
                    }
                    self.swap_tick_note = report.error.clone();
                    self.swap_report = report;
                }
                Err(_) => {
                    self.swap_tick_note = Some(
                        "Swap worker stopped unexpectedly. Saved transactions will be retried."
                            .into(),
                    )
                }
            }
            self.last_swap_tick = Some(Instant::now());
        }
        let due = self
            .last_swap_tick
            .map(|t| t.elapsed() >= Duration::from_secs(5))
            .unwrap_or(true);
        if !due
            || self.swap_job.is_some()
            || self.node.is_none()
            || !logic::availability(self.network).is_enabled()
        {
            return;
        }
        self.last_swap_tick = Some(Instant::now());
        let _ = self.queue_swap_job(Job::Tick);
    }
}

impl SwapWorker {
    pub(crate) fn tick_one_swap(&mut self, mut stored: StoredSwap) -> Result<(), String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let btc = self.btc_rpc()?;
        btc.check_network(self.network).map_err(|e| e.to_string())?;
        if let Some(tx) = &stored.night_lock_tx {
            if self.night_transaction_depth(tx)?.is_none() {
                use nightfall_swap::{
                    watch::{OutRef, TxRef},
                    ChainWatch,
                };
                let session = &self.swap_sessions[&id];
                let lock = session.lock_txid().map_err(|e| e.to_string())?;
                let depth = btc
                    .confirmations(&TxRef { id: lock })
                    .map_err(|e| e.to_string())?;
                let cancel = session.tx_cancel().map_err(|e| e.to_string())?;
                let out = cancel
                    .tx
                    .input
                    .first()
                    .ok_or("Bitcoin lock input missing")?
                    .previous_output;
                let safe = depth.is_some_and(|n| {
                    n >= u64::from(session.depths.bitcoin)
                        && session.depths.may_redeem(n.min(u64::from(u32::MAX)) as u32)
                }) && btc
                    .is_unspent(&OutRef {
                        txid: out.txid.to_string(),
                        vout: out.vout,
                    })
                    .map_err(|e| e.to_string())?;
                if safe {
                    self.wallet
                        .lock()
                        .map_err(|e| e.to_string())?
                        .record_swap_payment(tx, session.amounts.night_darks)
                        .map_err(|e| e.to_string())?;
                    self.submit_night_hex(&serde_json::to_string(tx).map_err(|e| e.to_string())?)?;
                }
            }
        }
        if let Some(raw) = &stored.bitcoin_lock_hex {
            use nightfall_swap::{watch::TxRef, ChainWatch};
            if let Some(txid) = &stored.btc_lock_txid {
                if btc
                    .confirmations(&TxRef { id: txid.clone() })
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    btc.broadcast(raw).map_err(|e| e.to_string())?;
                }
            }
        }
        if stored.btc_lock_txid.is_none() {
            if let Some(s) = self.swap_sessions.get(&id) {
                if let Ok(txid) = s.lock_txid() {
                    stored.btc_lock_txid = Some(txid);
                    persist::save(&self.datadir, &stored).map_err(|e| e.to_string())?;
                }
            }
        }
        if stored.night_lock_id.is_none() {
            if let Some(o) = self.find_night_lock(&stored)? {
                stored.night_lock_id = Some(o.commit.to_hex());
                persist::save(&self.datadir, &stored).map_err(|e| e.to_string())?;
            }
        }

        let night = self.night_watch_for(&stored)?;
        let mut drv = driver::Session::open(
            &self.datadir,
            stored.clone(),
            self.swap_sessions[&id].depths,
        );
        if stored
            .pending
            .as_ref()
            .is_some_and(|p| p.kind == SendKind::Redeem)
        {
            drv.stored.state = nightfall_swap::SwapState::Redeeming {
                id: stored.state.id(),
                role: stored.state.role(),
            };
            persist::save(&self.datadir, &drv.stored).map_err(|e| e.to_string())?;
        }
        self.fill_driver_raw(&id, &stored, &mut drv)?;

        match drv.tick(&btc, &night) {
            Ok(Tick::Broadcast {
                kind,
                raw_hex,
                txid,
            }) => {
                if kind == SendKind::NightClaim {
                    self.submit_night_hex(&raw_hex)?;
                    drv.note_broadcast(BroadcastResult::Accepted { txid });
                } else {
                    match btc.broadcast(&raw_hex) {
                        Ok(BroadcastResult::Accepted { .. })
                        | Ok(BroadcastResult::AlreadyKnown { .. }) => {
                            drv.note_broadcast(BroadcastResult::Accepted { txid });
                        }
                        Err(e) => return Err(e.to_string()),
                    }
                }
            }
            Ok(Tick::NeedsAttention { why }) => return Err(why),
            Ok(Tick::Idle) | Ok(Tick::Advanced) => {}
            Err(e) => return Err(e.to_string()),
        }
        if drv.stored.is_finished() && !drv.stored.reserved_commits.is_empty() {
            self.wallet
                .lock()
                .map_err(|e| e.to_string())?
                .release_commits(&drv.stored.reserved_commits)
                .map_err(|e| e.to_string())?;
            drv.stored.reserved_commits.clear();
            persist::save(&self.datadir, &drv.stored).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn fill_driver_raw(
        &mut self,
        id: &str,
        stored: &StoredSwap,
        drv: &mut driver::Session,
    ) -> Result<(), String> {
        let hexes = {
            let session = self
                .swap_sessions
                .get(id)
                .ok_or("No open session for this swap.")?;
            if let Ok(cancel) = session.tx_cancel() {
                drv.watched
                    .insert(SendKind::Cancel, cancel.tx.compute_txid().to_string());
                if let Ok(tx) = session.tx_refund(&cancel) {
                    drv.watched
                        .insert(SendKind::Refund, tx.tx.compute_txid().to_string());
                }
                if let Ok(tx) = session.tx_punish(&cancel) {
                    drv.watched
                        .insert(SendKind::Punish, tx.tx.compute_txid().to_string());
                }
            }
            if let Ok(tx) = session.tx_redeem() {
                drv.watched
                    .insert(SendKind::Redeem, tx.tx.compute_txid().to_string());
            }
            if let Some(lock) = &stored.night_lock_id {
                drv.watched
                    .insert(SendKind::NightClaim, format!("spent:{lock}"));
            }
            let mut v = Vec::new();
            if let Ok(hex) = session.signed_redeem_hex() {
                v.push((SendKind::Redeem, hex));
            }
            if let Ok(hex) = session.signed_cancel_hex() {
                v.push((SendKind::Cancel, hex));
            }
            if let Ok(hex) = session.signed_refund_hex() {
                v.push((SendKind::Refund, hex));
            }
            if let Ok(hex) = session.signed_punish_hex() {
                v.push((SendKind::Punish, hex));
            }
            v
        };
        for (kind, hex) in hexes {
            if let Ok(txid) = btc_txid_of(&hex) {
                drv.raw.insert(kind, (txid, hex));
            }
        }
        if (matches!(
            stored.state,
            nightfall_swap::SwapState::Redeeming { role, .. } if role == Role::Bob
        ) || matches!(
            stored.state,
            nightfall_swap::SwapState::Refunded {
                role: Role::Alice,
                ..
            }
        )) && stored.night_lock_id.is_some()
            && !stored.night_recovered
        {
            let json = match stored.outgoing.get(&SendKind::NightClaim) {
                Some(raw) => raw.clone(),
                None => serde_json::to_string(&self.build_night_claim(stored)?)
                    .map_err(|e| e.to_string())?,
            };
            let tx: nightfall_ledger::Transaction =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            drv.raw
                .insert(SendKind::NightClaim, (tx.txid().to_hex(), json));
        }
        Ok(())
    }

    pub(crate) fn submit_night_hex(&self, json: &str) -> Result<(), String> {
        let tx: nightfall_ledger::Transaction =
            serde_json::from_str(json).map_err(|e| e.to_string())?;
        let node = self.node.as_ref().ok_or("Node is not running.")?;
        let shared = node.shared();
        let mut guard = shared.lock().map_err(|e| e.to_string())?;
        if guard.mempool.txs.contains_key(&tx.txid().to_hex()) {
            return Ok(());
        }
        guard
            .submit_tx(tx)
            .map_err(|e| format!("node rejected the claim: {e}"))?;
        Ok(())
    }

    fn night_watch_for(&self, stored: &StoredSwap) -> Result<NightWatch, String> {
        let status_owned = self
            .node
            .as_ref()
            .ok_or("NIGHT node is not running.")?
            .status_snapshot()
            .map_err(|e| e.to_string())?;
        let status = Some(&status_owned);
        let loading = status.map(|s| s.loading).unwrap_or(true);
        let height = status.map(|s| s.tip_height).unwrap_or(0);
        let peer = status
            .map(|s| s.tip_height.saturating_add(s.blocks_behind))
            .unwrap_or(0);
        let mut confs = HashMap::new();
        if let Some(id) = &stored.night_lock_id {
            if let Some(n) = self.night_lock_depth(id) {
                confs.insert(id.clone(), n);
            }
            if let Some(n) = self.night_spend_depth(id)? {
                confs.insert(format!("spent:{id}"), n);
            }
        }
        if let Some(raw) = stored.outgoing.get(&SendKind::NightClaim) {
            let tx: nightfall_ledger::Transaction =
                serde_json::from_str(raw).map_err(|e| e.to_string())?;
            if let Some(n) = self.night_transaction_depth(&tx)? {
                confs.insert(tx.txid().to_hex(), n);
            }
        }
        Ok(NightWatch {
            snap: NightSnap {
                loading,
                height,
                best_peer_height: peer,
                confs,
                unspent: HashMap::new(),
            },
        })
    }

    /// Search canonical blocks in bounded pages, without a 2,000-block age cutoff.
    fn matching_depth(
        &self,
        matches: impl Fn(&nightfall_consensus::Block) -> bool,
    ) -> Result<Option<u64>, String> {
        let shared = self
            .node
            .as_ref()
            .ok_or("NIGHT node is not running.")?
            .shared();
        let (tip, mut from) = {
            let guard = shared.lock().map_err(|e| e.to_string())?;
            (
                guard.chain.tip_height().map(|h| h.0).unwrap_or(0),
                guard.chain.first_height,
            )
        };
        while from <= tip {
            let blocks = shared
                .lock()
                .map_err(|e| e.to_string())?
                .chain
                .blocks_from(from, 256);
            if blocks.is_empty() {
                break;
            }
            for block in &blocks {
                if matches(block) {
                    return Ok(Some(tip.saturating_sub(block.header.height.0) + 1));
                }
            }
            from = from.saturating_add(blocks.len() as u64);
        }
        Ok(None)
    }

    pub(crate) fn night_transaction_depth(
        &self,
        tx: &nightfall_ledger::Transaction,
    ) -> Result<Option<u64>, String> {
        self.matching_depth(|b| {
            tx.kernels.iter().all(|k| b.body.kernels.contains(k))
                && tx
                    .outputs
                    .iter()
                    .all(|o| b.body.outputs.iter().any(|x| x.commit == o.commit))
        })
    }

    fn night_spend_depth(&self, id: &str) -> Result<Option<u64>, String> {
        let commit = commit_from_hex(id).ok_or("Invalid NIGHT lock commitment")?;
        self.matching_depth(|b| b.body.inputs.iter().any(|i| i.commit == commit))
    }

    /// Unspent lock only. A spent lock must not keep reporting depth, or
    /// Bob's claim never looks confirmed.
    pub(crate) fn night_lock_depth(&self, commit_hex: &str) -> Option<u64> {
        let commit = commit_from_hex(commit_hex)?;
        let node = self.node.as_ref()?;
        let shared = node.shared();
        let guard = shared.lock().ok()?;
        let tip = guard.chain.tip_height().map(|h| h.0).unwrap_or(0);
        let e = guard.chain.ledger.utxos.get(&commit)?;
        Some(night_observe::confirmations(tip, e.height))
    }
}

fn commit_from_hex(commit_hex: &str) -> Option<Commitment> {
    let bytes = hex::decode(commit_hex).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(Commitment(arr))
}

pub(crate) fn btc_txid_of(raw_hex: &str) -> Result<String, String> {
    let raw = hex::decode(raw_hex.trim()).map_err(|e| e.to_string())?;
    let tx: bitcoin::Transaction = deserialize(&raw).map_err(|e| e.to_string())?;
    Ok(tx.compute_txid().to_string())
}
