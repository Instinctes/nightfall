//! Wallet adapter for the Core GUI.
//!
//! Thin wrapper over [`nightfall_wallet::Wallet`] so the GUI and the CLI share
//! one implementation of scanning, coin selection and spending.

use nightfall_crypto::Address;
use nightfall_ledger::Transaction;
use nightfall_node::NodeHandle;
use nightfall_types::NetworkId;
use nightfall_wallet::{Balances, HistoryEntry, OwnedOutput, Wallet};
use std::path::{Path, PathBuf};

pub struct WalletState {
    inner: Option<Wallet>,
    pub seed_path: PathBuf,
}

impl WalletState {
    /// Placeholder before a datadir is known.
    pub fn empty() -> Self {
        Self {
            inner: None,
            seed_path: PathBuf::new(),
        }
    }

    pub fn seed_exists(datadir: &Path) -> bool {
        datadir.join("core.seed").exists()
    }

    pub fn load_or_create(datadir: &Path, network: NetworkId) -> anyhow::Result<Self> {
        let wallet = Wallet::open(datadir, network, "core.seed")?;
        let seed_path = wallet.seed_path.clone();
        Ok(Self {
            inner: Some(wallet),
            seed_path,
        })
    }

    pub fn restore_from_phrase(
        datadir: &Path,
        network: NetworkId,
        phrase: &str,
    ) -> anyhow::Result<Self> {
        let wallet = Wallet::restore_from_phrase(datadir, network, "core.seed", phrase, 0)?;
        let seed_path = wallet.seed_path.clone();
        Ok(Self {
            inner: Some(wallet),
            seed_path,
        })
    }

    pub fn recovery_phrase(&self) -> String {
        self.wallet()
            .map(|w| w.recovery_phrase())
            .unwrap_or_default()
    }

    fn wallet(&self) -> Option<&Wallet> {
        self.inner.as_ref()
    }

    pub fn address(&self) -> Option<Address> {
        self.wallet().map(|w| w.address())
    }

    pub fn address_string(&self) -> String {
        self.wallet()
            .map(|w| w.address_string())
            .unwrap_or_else(|| "(no wallet)".into())
    }

    pub fn view_key_string(&self) -> String {
        self.wallet()
            .map(|w| w.view_key_string())
            .unwrap_or_else(|| "(no wallet)".into())
    }

    pub fn receipt_json(&self, txid_or_commit: &str) -> anyhow::Result<String> {
        let w = self
            .inner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("wallet not initialised"))?;
        let r = w
            .prove_history(txid_or_commit)
            .or_else(|_| w.prove_output(txid_or_commit))?;
        r.to_json()
    }

    /// Seed as hex — shown only behind an explicit reveal in the UI.
    pub fn seed_hex(&self) -> String {
        self.wallet()
            .map(|w| hex::encode(w.keys.seed))
            .unwrap_or_default()
    }

    pub fn balances(&self, tip_height: u64, maturity: u64) -> Balances {
        self.wallet()
            .map(|w| w.balances(tip_height, maturity))
            .unwrap_or_default()
    }

    pub fn output_count(&self) -> usize {
        self.wallet().map(|w| w.spendable_count()).unwrap_or(0)
    }

    pub fn scanned_to(&self) -> u64 {
        self.wallet().map(|w| w.scanned_to()).unwrap_or(0)
    }

    pub fn history(&self) -> &[HistoryEntry] {
        self.wallet().map(|w| w.history()).unwrap_or(&[])
    }

    pub fn outputs(&self) -> &[OwnedOutput] {
        self.wallet().map(|w| w.outputs()).unwrap_or(&[])
    }

    pub fn blocks_until_mature(&self, o: &OwnedOutput, tip: u64, maturity: u64) -> Option<u64> {
        self.wallet()
            .and_then(|w| w.blocks_until_mature(o, tip, maturity))
    }

    /// Scan the node's chain for outputs belonging to this wallet.
    pub fn sync_from_node(&mut self, node: &NodeHandle) -> anyhow::Result<u32> {
        let Some(wallet) = self.inner.as_mut() else {
            anyhow::bail!("wallet not initialised");
        };
        // Snapshot the blocks under the lock, then release it before scanning:
        // trial-decrypting every output is not something to do while holding
        // the node's state mutex.
        let from = wallet.scan_from();
        let blocks = {
            let shared = node.shared();
            let guard = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
            if guard.chain.is_pruned() && from < guard.chain.first_height {
                anyhow::bail!(
                    "this node is pruned; bodies start at height {}. \
                     Cannot rescan from {from}. Use an archive node or the light API",
                    guard.chain.first_height
                );
            }
            guard.chain.blocks_from(from, usize::MAX)
        };
        let n = wallet.scan_blocks(&blocks)?;
        Self::resend_pending(wallet, node);
        Ok(n)
    }

    /// Put unconfirmed payments back on the wire.
    ///
    /// A transaction is handed to one randomly chosen peer and nothing repeats
    /// it, so a single dropped hop used to end a payment quietly — the wallet
    /// said "pending" and no node in the world still held it. Re-submitting on
    /// every sync closes that: the mempool forgets after six hours, this puts
    /// it back, and the loop ends when a block takes it or the sender gives up.
    ///
    /// Failures are ignored on purpose. The node rejects a transaction whose
    /// inputs are already spent, which is exactly what happens the moment it
    /// confirms — that is a success wearing an error's clothes, and the next
    /// scan will notice properly.
    fn resend_pending(wallet: &mut nightfall_wallet::Wallet, node: &NodeHandle) {
        let pending = wallet.resendable();
        if pending.is_empty() {
            return;
        }
        let shared = node.shared();
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        for (_txid, tx) in pending {
            let _ = guard.submit_tx(tx);
        }
    }

    /// Build and submit a payment.
    pub fn send(
        &mut self,
        node: &NodeHandle,
        to: &str,
        amount_darks: u64,
        fee_darks: u64,
        memo: &str,
    ) -> anyhow::Result<String> {
        let Some(wallet) = self.inner.as_mut() else {
            anyhow::bail!("wallet not initialised");
        };

        let to_addr =
            Address::decode(to).map_err(|e| anyhow::anyhow!("recipient address rejected: {e}"))?;

        if to_addr == wallet.address() {
            anyhow::bail!("that is your own address");
        }

        let (tip, maturity) = {
            let shared = node.shared();
            let guard = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
            (
                guard.chain.tip_height().map(|h| h.0).unwrap_or(0),
                guard.chain.ledger.coinbase_maturity,
            )
        };

        let tx: Transaction =
            wallet.create_payment_at(&to_addr, amount_darks, fee_darks, memo, tip, maturity)?;
        let txid = tx.txid().to_hex();

        {
            let shared = node.shared();
            let mut guard = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
            guard
                .submit_tx(tx.clone())
                .map_err(|e| anyhow::anyhow!("node rejected the transaction: {e}"))?;
        }

        wallet.record_send(&tx, amount_darks, memo.to_string())?;
        Ok(txid)
    }

    pub fn rescan(&mut self, node: &NodeHandle) -> anyhow::Result<u32> {
        if let Some(w) = self.inner.as_mut() {
            w.reset_scan()?;
        }
        self.sync_from_node(node)
    }
}
