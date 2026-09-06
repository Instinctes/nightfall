//! Durable NIGHT lock and both directions of claim/recovery.
use crate::{
    app::{App, DEFAULT_FEE_DARKS},
    swap_worker::{Job, SwapWorker},
};
use nightfall_crypto::Output;
use nightfall_ledger::swap::build_claim;
use nightfall_swap::{night_observe, Role, StoredSwap};

impl App {
    pub fn lock_night(&mut self, stored: &StoredSwap) -> Result<String, String> {
        self.queue_swap_job(Job::LockNight(stored.state.id().to_string()))?;
        Ok("Checking both chains before locking NIGHT…".into())
    }
}

impl SwapWorker {
    pub fn lock_night(&mut self, stored: &StoredSwap) -> Result<String, String> {
        use nightfall_swap::{watch::TxRef, ChainWatch, SwapState};
        if !nightfall_swap::ui::availability(self.network).is_enabled() {
            return Err("Swaps are disabled on this network.".into());
        }
        if stored.state.role() != Role::Alice
            || !matches!(stored.state, SwapState::BtcLocked { .. })
        {
            return Err(
                "NIGHT can only be locked by the NIGHT seller after the Bitcoin lock.".into(),
            );
        }
        if stored.night_lock_id.is_some() || stored.night_lock_tx.is_some() {
            return Err(
                "This swap already has a NIGHT lock. Its original payment will be retried.".into(),
            );
        }
        let node = self.node.clone().ok_or("NIGHT node is not running.")?;
        let status = node.status_snapshot().map_err(|e| e.to_string())?;
        if status.loading || status.blocks_behind > 0 || status.pruned {
            return Err(
                "Wait for an unpruned NIGHT node to finish syncing before locking funds.".into(),
            );
        }
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let session = &self.swap_sessions[&id];
        // All exit signatures and the redeem adaptor must exist before Alice pays.
        session.signed_cancel_hex().map_err(|e| e.to_string())?;
        session.signed_punish_hex().map_err(|e| e.to_string())?;
        session.signed_redeem_hex().map_err(|e| e.to_string())?;
        let rpc = self.btc_rpc()?;
        rpc.check_network(self.network).map_err(|e| e.to_string())?;
        let txid = session.lock_txid().map_err(|e| e.to_string())?;
        let depth = rpc
            .confirmations(&TxRef { id: txid })
            .map_err(|e| e.to_string())?
            .ok_or("The Bitcoin lock is not on the Bitcoin chain.")?;
        if depth < u64::from(session.depths.bitcoin)
            || !session
                .depths
                .may_redeem(depth.min(u64::from(u32::MAX)) as u32)
        {
            return Err(
                "The Bitcoin lock has too few confirmations or its cancel deadline is too close."
                    .into(),
            );
        }
        let cancel = session.tx_cancel().map_err(|e| e.to_string())?;
        let out = cancel
            .tx
            .input
            .first()
            .ok_or("Bitcoin cancel has no input")?
            .previous_output;
        if !rpc
            .is_unspent(&nightfall_swap::watch::OutRef {
                txid: out.txid.to_string(),
                vout: out.vout,
            })
            .map_err(|e| e.to_string())?
        {
            return Err(
                "The Bitcoin lock has already been spent. NIGHT will not be locked.".into(),
            );
        }
        let shared = session.shared_lock().map_err(|e| e.to_string())?;
        let value = session.amounts.night_darks;
        let tx = self
            .wallet
            .lock()
            .map_err(|e| e.to_string())?
            .prepare_from_commits(
                &node,
                &stored.reserved_commits,
                &shared.address(),
                value,
                DEFAULT_FEE_DARKS,
                "swap-lock",
            )
            .map_err(|e| e.to_string())?;
        let lock =
            night_observe::lock_output_in_tx(&tx, &shared, value).map_err(|e| format!("{e:?}"))?;
        let mut rec = stored.clone();
        rec.btc_lock_txid = Some(session.lock_txid().map_err(|e| e.to_string())?);
        rec.night_lock_id = Some(lock.commit.to_hex());
        rec.night_lock_tx = Some(tx.clone());
        // Durable intent, then wallet history, then broadcast. A retry uses these bytes.
        nightfall_swap::persist::save(&self.datadir, &rec).map_err(|e| e.to_string())?;
        self.wallet
            .lock()
            .map_err(|e| e.to_string())?
            .record_swap_payment(&tx, value)
            .map_err(|e| e.to_string())?;
        self.submit_night_hex(&serde_json::to_string(&tx).map_err(|e| e.to_string())?)?;
        Ok("NIGHT lock submitted. Waiting for on-chain confirmations.".into())
    }

    pub fn find_night_lock(&self, stored: &StoredSwap) -> Result<Option<Output>, String> {
        let id = stored.state.id().to_string();
        let session = self.swap_sessions.get(&id).ok_or("Missing swap session")?;
        let shared_lock = session.shared_lock().map_err(|e| e.to_string())?;
        let shared = self
            .node
            .as_ref()
            .ok_or("NIGHT node is not running.")?
            .shared();
        let (tip, mut from) = {
            let g = shared.lock().map_err(|e| e.to_string())?;
            (
                g.chain.tip_height().map(|h| h.0).unwrap_or(0),
                g.chain.first_height,
            )
        };
        // The recorded commitment fixes the identity after discovery.
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
                for o in &block.body.outputs {
                    if stored
                        .night_lock_id
                        .as_ref()
                        .is_some_and(|id| *id != o.commit.to_hex())
                    {
                        continue;
                    }
                    if shared_lock
                        .verify_lock(o, session.amounts.night_darks)
                        .is_ok()
                    {
                        return Ok(Some(o.clone()));
                    }
                }
            }
            from = from.saturating_add(blocks.len() as u64);
        }
        Ok(None)
    }

    pub fn build_night_claim(
        &mut self,
        stored: &StoredSwap,
    ) -> Result<nightfall_ledger::Transaction, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let lock = self
            .find_night_lock(stored)?
            .ok_or("The NIGHT lock is not available on this chain.")?;
        let session = &self.swap_sessions[&id];
        let peer_txid = match session.role {
            Role::Bob => session
                .tx_redeem()
                .map_err(|e| e.to_string())?
                .tx
                .compute_txid(),
            Role::Alice => {
                let cancel = session.tx_cancel().map_err(|e| e.to_string())?;
                session
                    .tx_refund(&cancel)
                    .map_err(|e| e.to_string())?
                    .tx
                    .compute_txid()
            }
        };
        // Look up the agreed ID directly: gettxspendingprevout only sees the mempool.
        let raw = self
            .btc_rpc()?
            .raw_tx_hex(&peer_txid.to_string())
            .map_err(|e| e.to_string())?;
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(raw).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let peer = session
            .recover_peer_from_transaction(&tx)
            .map_err(|e| e.to_string())?;
        let own = session.own_night_secret();
        let (a, b) = match session.role {
            Role::Bob => (peer, own),
            Role::Alice => (own, peer),
        };
        let to = self
            .wallet
            .lock()
            .map_err(|e| e.to_string())?
            .address()
            .ok_or("Wallet is not initialised.")?;
        build_claim(
            &session.shared_lock().map_err(|e| e.to_string())?,
            &lock,
            session.amounts.night_darks,
            &a,
            &b,
            &to,
            DEFAULT_FEE_DARKS,
            self.network.proof_context(),
        )
        .map_err(|e| e.to_string())
    }
}
