//! Wallet adapter for the Core GUI.
//!
//! Thin wrapper over [`nightfall_wallet::Wallet`] so the GUI and the CLI share
//! one implementation of scanning, coin selection and spending.

use nightfall_crypto::Address;
use nightfall_ledger::Transaction;
use nightfall_node::{NodeHandle, NodeInner};
use nightfall_storage::dirlock::DirLock;
use nightfall_types::NetworkId;
use nightfall_wallet::vault_store::VaultStore;
use nightfall_wallet::{Balances, HistoryEntry, OwnedOutput, Wallet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Blocks copied out of the node per scan page.
///
/// The size trades memory against bookkeeping. Every page is cloned out of the
/// chain and every page ends in a wallet save, so a small page wastes writes
/// and a large one defeats the point. A thousand blocks is a few megabytes on
/// today's chain and roughly 140 pages for a rescan from genesis.
const SCAN_PAGE: usize = 1024;

fn require_finished_replay(loading: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !loading,
        "The node is still verifying its saved chain. Wait for loading to finish before scanning or sending."
    );
    Ok(())
}

/// Call with the node mutex held. A height alone does not establish that the
/// wallet and node agree on the chain that owns the selected coins.
fn require_canonical_scan(wallet: &Wallet, node: &NodeInner) -> anyhow::Result<()> {
    require_finished_replay(node.is_loading())?;
    let tip = node.chain.tip_height().map(|h| h.0).unwrap_or(0);
    anyhow::ensure!(
        wallet.scan_from() == tip,
        "Wallet scan is at height {}, but the node is at {tip}. Finish scanning the current chain before sending. If the node is behind, wait for it to catch up; if its history changed, review the rescan warning.",
        wallet.scan_from(),
    );
    let anchor = node
        .chain
        .block_by_height(wallet.scanned_to())
        .ok_or_else(|| anyhow::anyhow!("The node cannot supply the wallet's scan anchor. No payment was created. Use an archive node and finish scanning."))?;
    wallet.check_scan_anchor(anchor)
}

enum Backend {
    Empty,
    Legacy(Box<Wallet>),
    Vault(Box<VaultStore>),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Custody {
    Empty,
    Legacy,
    Migration,
    Locked,
    Unlocked,
    Failed,
}

pub struct WalletState {
    inner: Backend,
    pub seed_path: PathBuf,
}

impl WalletState {
    /// Placeholder before a datadir is known.
    pub fn empty() -> Self {
        Self {
            inner: Backend::Empty,
            seed_path: PathBuf::new(),
        }
    }

    pub fn seed_exists(datadir: &Path) -> bool {
        if nightfall_wallet::vault_required(datadir, "core.seed").unwrap_or(true) {
            return true;
        }
        // An orphaned database or unreadable path is not a first-run wallet.
        [
            "core.seed",
            "core.seed.outputs.json",
            "core.seed.outputs.json.tmp",
        ]
        .iter()
        .any(|name| match std::fs::symlink_metadata(datadir.join(name)) {
            Ok(_) => true,
            Err(e) => e.kind() != std::io::ErrorKind::NotFound,
        })
    }

    pub fn load_or_create(datadir: &Path, network: NetworkId) -> anyhow::Result<Self> {
        let wallet = Wallet::open(datadir, network, "core.seed")?;
        let seed_path = wallet.seed_path.clone();
        Ok(Self {
            inner: Backend::Legacy(Box::new(wallet)),
            seed_path,
        })
    }

    /// Create/restore without ever invoking a plaintext wallet writer.
    /// Validation precedes the permanent vault marker. Errors after enrollment
    /// retain the vault backend so no caller can fall back to legacy storage.
    pub fn initialize_vault(
        &mut self,
        lock: Arc<DirLock>,
        network: NetworkId,
        source: crate::onboarding::ProvisionSource,
        password: &str,
    ) -> anyhow::Result<()> {
        use crate::onboarding::ProvisionSource;
        nightfall_wallet::vault::Vault::validate_password(password)?;
        let wallet = match source {
            ProvisionSource::Words(phrase) => {
                let keys = nightfall_wallet::recovery::keys_from_phrase(&phrase)?;
                Wallet::in_memory(network, keys, 0)
            }
            ProvisionSource::Restored(wallet) => {
                anyhow::ensure!(
                    wallet.network == network,
                    "This backup belongs to a different network."
                );
                // The last gate before the state is sealed and becomes live.
                // `recover_backup_state` already withheld every imported send;
                // this catches a future path that forgets to, here rather than
                // on the wire with someone else's money.
                anyhow::ensure!(
                    wallet.resendable().is_empty(),
                    "This restored state still holds payments that would be broadcast automatically. Nothing was written."
                );
                *wallet
            }
        };
        let datadir = lock
            .path()
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing wallet directory"))?;
        match &self.inner {
            Backend::Empty => {
                anyhow::ensure!(!Self::seed_exists(datadir),
                "Wallet files already exist. Reopen Core to inspect them; nothing was replaced.")
            }
            Backend::Vault(_) => anyhow::ensure!(
                self.seed_path
                    .parent()
                    .map(|p| p.canonicalize())
                    .transpose()?
                    == Some(datadir.canonicalize()?),
                "Vault belongs to a different wallet directory."
            ),
            _ => anyhow::bail!("An existing wallet cannot be replaced by onboarding."),
        }
        if !self.is_vault() {
            self.inner = Backend::Failed(
                "Vault creation interrupted. Restart Core to inspect the saved state.".into(),
            );
            *self = Self::open_vault(lock, network)?;
        }
        match &mut self.inner {
            Backend::Vault(store) => store.initialize(&wallet, password),
            _ => unreachable!(),
        }
    }

    /// An empty enrollment marker is not permission to generate another seed.
    /// On restart the UI offers only an explicit restoration of the original words.
    pub fn awaiting_initial_restore(&self) -> bool {
        matches!(&self.inner, Backend::Vault(store)
            if !store.requires_reopen() && !store.has_snapshot()
            && matches!(store.needs_legacy_retirement(), Ok(false))
            && matches!(std::fs::symlink_metadata(&self.seed_path), Err(e) if e.kind() == std::io::ErrorKind::NotFound))
    }

    pub fn recovery_phrase(&self) -> String {
        self.wallet()
            .map(|w| w.recovery_phrase())
            .unwrap_or_default()
    }

    pub fn export_vault_backup(&self, path: &Path, password: &str) -> anyhow::Result<()> {
        match &self.inner {
            Backend::Vault(store) => store.export_backup(path, password),
            _ => anyhow::bail!("Encrypted backup requires an unlocked Vault."),
        }
    }

    pub fn verify_vault_backup(&self, path: &Path, password: &str) -> anyhow::Result<bool> {
        match &self.inner {
            Backend::Vault(store) => store.verify_backup(path, password),
            _ => anyhow::bail!("Encrypted backup verification requires an unlocked Vault."),
        }
    }

    fn wallet(&self) -> Option<&Wallet> {
        self.require_wallet().ok()
    }

    fn require_wallet(&self) -> anyhow::Result<&Wallet> {
        match &self.inner {
            Backend::Legacy(wallet) => Ok(wallet),
            Backend::Vault(store) => store.wallet(),
            Backend::Failed(error) => anyhow::bail!("{error}"),
            Backend::Empty => anyhow::bail!("Wallet is unavailable or busy."),
        }
    }

    fn update<T>(
        &mut self,
        edit: impl FnOnce(&mut Wallet) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        match &mut self.inner {
            Backend::Legacy(wallet) => edit(wallet),
            Backend::Vault(store) => store.update(edit),
            Backend::Failed(error) => anyhow::bail!("{error}"),
            Backend::Empty => anyhow::bail!("Wallet is unavailable or busy."),
        }
    }

    pub fn custody(&self) -> Custody {
        match &self.inner {
            Backend::Empty => Custody::Empty,
            Backend::Legacy(_) => Custody::Legacy,
            Backend::Failed(_) => Custody::Failed,
            Backend::Vault(store) if store.requires_reopen() => Custody::Failed,
            Backend::Vault(store) => match store.needs_legacy_retirement() {
                Err(_) => Custody::Failed,
                Ok(true) => Custody::Migration,
                Ok(false) if !store.has_snapshot() => Custody::Migration,
                Ok(false) if store.is_locked() => Custody::Locked,
                Ok(false) if store.wallet().is_ok() => Custody::Unlocked,
                _ => Custody::Failed,
            },
        }
    }

    pub fn is_vault(&self) -> bool {
        matches!(self.inner, Backend::Vault(_))
    }

    pub fn can_scan(&self) -> bool {
        matches!(self.custody(), Custody::Legacy | Custody::Unlocked)
    }

    pub fn open_vault(lock: Arc<DirLock>, network: NetworkId) -> anyhow::Result<Self> {
        let datadir = lock
            .path()
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing wallet directory"))?;
        let seed_path = datadir.join("core.seed");
        let store = VaultStore::open(lock, "core.seed", network)?;
        Ok(Self {
            inner: Backend::Vault(Box::new(store)),
            seed_path,
        })
    }

    pub fn prepare_vault(
        &mut self,
        lock: Arc<DirLock>,
        network: NetworkId,
        password: &str,
    ) -> anyhow::Result<()> {
        nightfall_wallet::vault::Vault::validate_password(password)?;
        let datadir = lock
            .path()
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing wallet directory"))?;
        anyhow::ensure!(
            self.seed_path
                .parent()
                .map(|path| path.canonicalize())
                .transpose()?
                == Some(datadir.canonicalize()?),
            "Vault enrollment belongs to a different wallet directory."
        );
        if let Backend::Legacy(wallet) = &self.inner {
            anyhow::ensure!(
                wallet.network == network,
                "Vault enrollment belongs to another network."
            );
        }
        anyhow::ensure!(!nightfall_swap::persist::list(datadir)?.iter().any(|swap| !swap.is_finished()),
            "Finish all experimental swaps before enabling Vault. Swap secrets are not covered by Vault.");
        if !self.is_vault() {
            // Retire the in-memory legacy writer before creating the marker.
            // An error must never restore that writer behind the vault's back.
            self.inner = Backend::Failed(
                "Vault enrollment interrupted. Reopen the wallet to resume.".into(),
            );
            *self = Self::open_vault(lock, network)?;
        }
        if let Backend::Vault(store) = &mut self.inner {
            anyhow::ensure!(
                !store.has_snapshot(),
                "Encrypted copy already exists. Verify and finish its migration."
            );
            store.migrate_legacy(password)?;
        }
        Ok(())
    }

    pub fn has_vault_snapshot(&self) -> bool {
        matches!(&self.inner, Backend::Vault(store) if store.has_snapshot())
    }

    pub fn finish_vault_migration(&mut self, password: &str) -> anyhow::Result<()> {
        match &mut self.inner {
            Backend::Vault(store) => store.finish_legacy_retirement(password),
            _ => anyhow::bail!("No vault migration is available."),
        }
    }

    pub fn unlock_vault(&mut self, password: &str) -> anyhow::Result<()> {
        match &mut self.inner {
            Backend::Vault(store) => store.unlock(password),
            _ => anyhow::bail!("No encrypted vault is available."),
        }
    }

    pub fn lock_vault(&mut self) {
        if let Backend::Vault(store) = &mut self.inner {
            store.lock();
        }
    }

    pub fn change_vault_password(&mut self, password: &str) -> anyhow::Result<()> {
        match &mut self.inner {
            Backend::Vault(store) => store.change_password(password),
            _ => anyhow::bail!("No encrypted vault is available."),
        }
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

    // The till. Every one of these needs an unlocked wallet, because the
    // invoices live inside the encrypted snapshot — a till's references and
    // amounts are the shop's business and do not belong in a plaintext file.
    // None of them needs a spend key; see `nightfall_wallet::counter`.

    pub fn till(
        &self,
        now_unix: u64,
    ) -> Vec<(
        nightfall_wallet::counter::Invoice,
        nightfall_wallet::counter::InvoiceStatus,
    )> {
        self.wallet().map(|w| w.till(now_unix)).unwrap_or_default()
    }

    // Through `update`, like every other change: for a Vault wallet that is
    // what writes the new ciphertext and adopts it only once it is stored.
    // Writing the invoice into the in-memory wallet and calling save directly
    // would leave the till one crash away from disagreeing with itself.

    pub fn add_invoice(
        &mut self,
        invoice: nightfall_wallet::counter::Invoice,
    ) -> anyhow::Result<()> {
        self.update(|w| w.add_invoice(invoice))
    }

    pub fn close_invoice(&mut self, reference: &str, note: &str) -> anyhow::Result<()> {
        self.update(|w| w.close_invoice(reference, note))
    }

    pub fn remove_invoice(&mut self, reference: &str) -> anyhow::Result<()> {
        self.update(|w| w.remove_invoice(reference))
    }

    pub fn receipt_json(&self, txid_or_commit: &str) -> anyhow::Result<String> {
        let w = self.require_wallet()?;
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

    /// Free outputs a swap was holding.
    ///
    /// The reserving half lives on `Wallet` and is exercised there; it gets a
    /// wrapper here once the app actually builds a NIGHT lock. Releasing is
    /// needed already: a swap file removed without it would strand the coins
    /// with nothing left to free them.
    pub fn release_commits(&mut self, hexes: &[String]) -> anyhow::Result<()> {
        self.update(|w| w.release_commits(hexes))
    }

    pub fn reserve_commits(&mut self, hexes: &[String]) -> anyhow::Result<()> {
        self.update(|w| w.reserve_commits(hexes))
    }

    pub fn pick_commit_hexes_at(
        &self,
        target: u64,
        tip: u64,
        maturity: u64,
    ) -> anyhow::Result<Vec<String>> {
        let w = self.require_wallet()?;
        w.pick_commit_hexes_at(target, tip, maturity)
    }

    /// Pay `to` from specific outputs (including reserved swap coins).
    pub fn prepare_from_commits(
        &mut self,
        node: &NodeHandle,
        commits: &[String],
        to: &Address,
        amount_darks: u64,
        fee_darks: u64,
        memo: &str,
    ) -> anyhow::Result<Transaction> {
        anyhow::ensure!(
            !self.is_vault(),
            "Experimental swap signing is not integrated with Vault."
        );
        self.require_wallet()?;
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
        self.update(|wallet| {
            wallet.create_payment_from_commits_at(
                commits,
                to,
                amount_darks,
                fee_darks,
                memo,
                tip,
                maturity,
            )
        })
    }

    pub fn record_swap_payment(&mut self, tx: &Transaction, amount: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.is_vault(),
            "Experimental swap payments are not integrated with Vault."
        );
        self.update(|wallet| {
            if !wallet
                .history()
                .iter()
                .any(|h| h.txid == tx.txid().to_hex())
            {
                wallet.record_send(tx, amount, "swap-lock".into())?;
            }
            Ok(())
        })
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
    ///
    /// The chain is read one page at a time. `blocks_from` clones every block
    /// it hands back, so asking for the whole chain at once built a second
    /// copy of it in memory before a single output had been trial-decrypted —
    /// on a 138k-block chain that is hundreds of megabytes, and it grows with
    /// the chain. Pages hold that to one batch. They also make an interrupted
    /// rescan worth something: `scan_blocks` saves each page, so a wallet that
    /// is closed halfway through resumes where it stopped instead of starting
    /// over.
    ///
    /// Each page begins at the wallet's scan position, so its first block is
    /// the last one already scanned. That overlap is deliberate: it is the
    /// anchor `Wallet::scan_blocks` checks to notice a reorg that replaced
    /// history underneath a scan in progress. Rescanning that one block finds
    /// nothing new.
    pub fn sync_from_node(&mut self, node: &NodeHandle) -> anyhow::Result<u32> {
        let mut found: u32 = 0;
        loop {
            let from = self.require_wallet()?.scan_from();
            // Snapshot one page under the lock, then release it before
            // scanning: trial-decrypting every output is not something to do
            // while holding the node's state mutex.
            let (page, tip) = {
                let shared = node.shared();
                let guard = shared
                    .lock()
                    .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
                require_finished_replay(guard.is_loading())?;
                let tip = guard.chain.tip_height().map(|h| h.0).unwrap_or(0);
                anyhow::ensure!(
                    from <= tip || !self.require_wallet()?.has_scanned_history(),
                    "The wallet already scanned height {from}, but the node's current chain ends at {tip}. This is not a completed scan. Wait for the node to catch up, or review the changed chain before rescanning."
                );
                if guard.chain.is_pruned() && from < guard.chain.first_height {
                    anyhow::bail!(
                        "this node is pruned; bodies start at height {}. \
                         Cannot rescan from {from}. Use an archive node or the light API",
                        guard.chain.first_height
                    );
                }
                (guard.chain.blocks_from(from, SCAN_PAGE), tip)
            };

            if page.is_empty() {
                // Nothing at the scan position. Caught up is the ordinary
                // reason; a node that discarded those bodies mid-scan is the
                // other one, and that must not pass for a finished scan.
                anyhow::ensure!(
                    from >= tip,
                    "the node stopped serving blocks at height {from} while the \
                     scan was running, but its chain reaches {tip}. The wallet \
                     has not seen everything and must not be treated as synced. \
                     Try again, or use an archive node."
                );
                break;
            }

            let was_full_page = page.len() == SCAN_PAGE;
            found = found.saturating_add(self.update(|wallet| wallet.scan_blocks(&page))?);

            if !was_full_page {
                break; // the page ran out before the limit: this was the tail
            }

            // A full page always carries blocks above `from`, so the position
            // must have moved. If it ever did not, looping would spin here
            // forever reading the same page; stop and say so instead.
            anyhow::ensure!(
                self.require_wallet()?.scan_from() > from,
                "the scan position stayed at {from} after reading {} blocks. \
                 Refusing to loop. This is a bug, not a chain problem.",
                page.len()
            );
        }
        Self::resend_pending(self.require_wallet()?, node)?;
        Ok(found)
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
    fn resend_pending(wallet: &Wallet, node: &NodeHandle) -> anyhow::Result<()> {
        let shared = node.shared();
        let mut guard = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
        require_finished_replay(guard.is_loading())?;
        let tip = guard.chain.tip_height().map(|h| h.0).unwrap_or(0);
        // A deliberately future birth height is not an already-scanned chain.
        // Such a fresh wallet has no pending payments to put back on the wire.
        if wallet.scan_from() > tip && !wallet.has_scanned_history() {
            return Ok(());
        }
        // The node may have changed while the final page was decrypted and
        // saved. Check again even with an empty mempool before reporting success.
        require_canonical_scan(wallet, &guard)?;
        for (txid, tx) in wallet.resendable() {
            // Swap locks have deadlines; only the swap worker may retry them.
            if wallet
                .history()
                .iter()
                .any(|h| h.txid == txid && h.memo == "swap-lock")
            {
                continue;
            }
            let _ = guard.submit_tx(tx);
        }
        Ok(())
    }

    /// Hand the network a transaction that was built somewhere else.
    ///
    /// For Air. The transaction was built and signed on a machine that is not
    /// on a network, and this side is a relay and nothing more. Nothing is
    /// recorded in this wallet, because these are not this wallet's coins to
    /// record — the wallet here may be locked, may be a different wallet, or
    /// may not exist at all. Requiring one would defeat the point: the whole
    /// idea is that the machine with the network has nothing worth stealing.
    pub fn broadcast_foreign(
        &mut self,
        node: &NodeHandle,
        tx: nightfall_ledger::Transaction,
    ) -> anyhow::Result<String> {
        let txid = tx.txid().to_hex();
        let shared = node.shared();
        let mut guard = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
        guard
            .submit_tx(tx)
            .map_err(|e| anyhow::anyhow!("The node refused this transaction: {e}"))?;
        Ok(txid)
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
        let to_addr =
            Address::decode(to).map_err(|e| anyhow::anyhow!("recipient address rejected: {e}"))?;

        if to_addr == self.require_wallet()?.address() {
            anyhow::bail!("that is your own address");
        }

        let shared = node.shared();
        let mut guard = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("node state lock poisoned"))?;
        require_canonical_scan(self.require_wallet()?, &guard)?;
        let tip = guard.chain.tip_height().map(|h| h.0).unwrap_or(0);
        let maturity = guard.chain.ledger.coinbase_maturity;

        // Keep the chain guard until the saved payment is submitted. Releasing
        // it between the anchor check and persistence would let a reorg turn a
        // checked coin into an orphan while still recording an invalid send.
        let tx = self.prepare_payment(&to_addr, amount_darks, fee_darks, memo, tip, maturity)?;
        let txid = tx.txid().to_hex();

        guard
            .submit_tx(tx)
            .map_err(|e| anyhow::anyhow!("Payment saved as pending, but node submission failed: {e}. Check Activity before creating another payment."))?;

        Ok(txid)
    }

    /// No network effects. A returned transaction already has a persisted
    /// pending record; a failed save never yields a transaction for broadcast.
    pub(crate) fn prepare_payment(
        &mut self,
        to: &Address,
        amount: u64,
        fee: u64,
        memo: &str,
        tip: u64,
        maturity: u64,
    ) -> anyhow::Result<Transaction> {
        anyhow::ensure!(
            *to != self.require_wallet()?.address(),
            "That is your own address."
        );
        self.update(|wallet| {
            let tx = wallet.create_payment_at(to, amount, fee, memo, tip, maturity)?;
            wallet.record_send(&tx, amount, memo.to_string())?;
            Ok(tx)
        })
    }

    /// Validate before changing either the wallet or the node's chain files.
    pub fn check_rescan_allowed(&self) -> anyhow::Result<()> {
        let wallet = self.require_wallet()?;
        anyhow::ensure!(
            !wallet.history().iter().any(|entry| entry.is_pending()),
            "Rescan is blocked while payments are pending. Resolve them and preserve an encrypted backup first."
        );
        anyhow::ensure!(
            !wallet.has_reservations(),
            "Rescan is blocked while outputs are reserved. Resolve the reservations first."
        );
        // A rescan clears reservations. Never unlock coins belonging to an
        // unfinished trade, or another payment could invalidate its recovery.
        let datadir = self
            .seed_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Wallet folder unavailable"))?;
        let swaps = nightfall_swap::persist::list(datadir)?;
        anyhow::ensure!(
            !swaps.iter().any(|s| !s.is_finished()),
            "Finish or safely close active swaps before rescanning the wallet."
        );
        Ok(())
    }

    pub fn rescan(&mut self, node: &NodeHandle) -> anyhow::Result<u32> {
        self.check_rescan_allowed()?;
        self.update(|w| w.reset_scan())?;
        self.sync_from_node(node)
    }
}

#[cfg(test)]
mod scan_readiness_tests {
    use super::require_finished_replay;

    #[test]
    fn chain_replay_must_finish_before_scanning_or_sending() {
        let error = require_finished_replay(true).unwrap_err().to_string();
        assert!(error.contains("still verifying its saved chain"));
        require_finished_replay(false).unwrap();
    }
}
