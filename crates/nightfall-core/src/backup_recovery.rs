//! Explicit backup recovery; never publish source pending transactions.
//!
//! Two ways in, and the difference matters enough to be a choice the owner
//! makes rather than a default they discover afterwards:
//!
//! * Keys only — takes the seed and nothing else. The wallet rescans the chain
//!   from genesis and rebuilds what it finds. Slow, and it forgets any payment
//!   the chain cannot show, which is precisely what makes it safe.
//! * Full state — takes the scan position, coins, history and reservations, so
//!   there is no rescan. Everything the backup carries comes with it, including
//!   payments that were unconfirmed when it was written. Those are withheld:
//!   see `Wallet::quarantine_imported_sends`.
use crate::{
    onboarding::{ProvisionRequest, ProvisionSource},
    theme::*,
    widgets::*,
};
use eframe::egui;
use nightfall_types::NetworkId;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// What a verified backup handed back. The variant decides what can be built
/// from it, so a full-state preview can never be provisioned as keys-only or
/// the other way round.
enum Recovered {
    Keys(Zeroizing<String>),
    State(Box<nightfall_wallet::Wallet>),
}

impl Recovered {
    fn is_full_state(&self) -> bool {
        matches!(self, Self::State(_))
    }
}

struct Preview {
    recovered: Recovered,
    address: String,
    /// For keys-only these four describe the SOURCE, which is not copied.
    /// For a full import they describe what is being carried over.
    scanned_to: u64,
    history: usize,
    pending: usize,
    reservations: usize,
}

pub struct BackupRecovery {
    pub resume_only: bool,
    path: String,
    old_password: Zeroizing<String>,
    new_password: Zeroizing<String>,
    confirmation: Zeroizing<String>,
    /// Chosen before verification, because it decides which reader runs.
    full_state: bool,
    acknowledged: bool,
    preview: Option<Preview>,
    job: Option<std::thread::JoinHandle<Result<Preview, String>>>,
    discard: bool,
    back: bool,
    error: Option<String>,
    last_activity: Instant,
}

impl BackupRecovery {
    pub fn new(resume_only: bool) -> Self {
        Self {
            resume_only,
            path: String::new(),
            old_password: Zeroizing::new(String::new()),
            new_password: Zeroizing::new(String::new()),
            confirmation: Zeroizing::new(String::new()),
            full_state: false,
            acknowledged: false,
            preview: None,
            job: None,
            discard: false,
            back: false,
            error: None,
            last_activity: Instant::now(),
        }
    }

    fn clear(&mut self) {
        self.path.zeroize();
        self.old_password.zeroize();
        self.new_password.zeroize();
        self.confirmation.zeroize();
        self.acknowledged = false;
        self.preview = None;
        // Back to the cautious reading. After an idle timeout the owner should
        // have to choose the wider import again rather than inherit it.
        self.full_state = false;
    }

    pub fn observe_activity(&mut self, focused: bool, hidden: bool, activity: bool, now: Instant) {
        if !focused
            || hidden
            || now.saturating_duration_since(self.last_activity) >= Duration::from_secs(300)
        {
            self.clear();
            self.discard = true;
            self.error = None;
        }
        if activity {
            self.last_activity = now;
        }
    }

    fn inspect(&mut self, network: NetworkId, ctx: egui::Context) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        if self.path.len() > 4096
            || !std::path::Path::new(&self.path).is_absolute()
            || self.old_password.is_empty()
        {
            self.error =
                Some("Enter an absolute backup filename and that backup's password.".into());
            return;
        }
        let path = std::path::PathBuf::from(std::mem::take(&mut self.path));
        let password = Zeroizing::new(std::mem::take(&mut *self.old_password));
        let full_state = self.full_state;
        self.clear();
        self.discard = false;
        let job = std::thread::Builder::new()
            .name("nightfall-backup-preview".into())
            .spawn(move || {
                let result = if full_state {
                    nightfall_wallet::vault_store::recover_backup_state(&path, &password, network)
                        .map(|recovered| Preview {
                            address: recovered.wallet.address().encode(),
                            scanned_to: recovered.scanned_to,
                            history: recovered.history_entries,
                            pending: recovered.quarantined_sends,
                            reservations: recovered.reservations,
                            recovered: Recovered::State(Box::new(recovered.wallet)),
                        })
                        .map_err(|error| error.to_string())
                } else {
                    nightfall_wallet::vault_store::recover_backup_keys(&path, &password, network)
                        .map(|recovered| Preview {
                            recovered: Recovered::Keys(Zeroizing::new(
                                recovered.wallet.recovery_phrase(),
                            )),
                            address: recovered.wallet.address().encode(),
                            scanned_to: recovered.source_scanned_to,
                            history: recovered.source_history_entries,
                            pending: recovered.source_pending_sends,
                            reservations: recovered.source_reservations,
                        })
                        .map_err(|error| error.to_string())
                };
                ctx.request_repaint();
                result
            });
        match job {
            Ok(job) => self.job = Some(job),
            Err(_) => {
                self.error =
                    Some("Could not start backup verification. Nothing was written.".into())
            }
        }
    }

    fn poll(&mut self) {
        if !self.job.as_ref().is_some_and(|job| job.is_finished()) {
            return;
        }
        let result = self.job.take().unwrap().join();
        if self.discard {
            return;
        }
        match result {
            Ok(Ok(preview)) => self.preview = Some(preview),
            Ok(Err(error)) => self.error = Some(error),
            Err(_) => {
                self.error =
                    Some("Backup verification stopped unexpectedly. Nothing was written.".into())
            }
        }
    }

    fn request(&mut self) -> anyhow::Result<ProvisionRequest> {
        anyhow::ensure!(
            self.job.is_none() && self.preview.is_some() && !self.discard,
            "Verify the encrypted backup first."
        );
        anyhow::ensure!(
            self.acknowledged,
            "Confirm the address and the recovery notice."
        );
        anyhow::ensure!(
            self.new_password == self.confirmation,
            "The new passwords do not match."
        );
        nightfall_wallet::vault::Vault::validate_password(&self.new_password)?;
        let preview = self.preview.take().unwrap();
        let source = match preview.recovered {
            Recovered::Keys(phrase) => ProvisionSource::Words(phrase),
            Recovered::State(wallet) => {
                // Withholding is decided when the backup is read, not here.
                // Refusing rather than re-marking keeps that single, so a
                // reader that ever stops quarantining cannot be papered over.
                anyhow::ensure!(
                    wallet.resendable().is_empty(),
                    "This backup still holds payments that would be broadcast automatically. Nothing was written."
                );
                ProvisionSource::Restored(wallet)
            }
        };
        let request = ProvisionRequest {
            source,
            password: Zeroizing::new(std::mem::take(&mut *self.new_password)),
        };
        self.clear();
        Ok(request)
    }

    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, network: NetworkId) -> Option<ProvisionRequest> {
        self.poll();
        ui.heading("Recover from a backup");
        ui.label(format!(
            "Selected network: {network}. The original backup is never modified."
        ));
        if self.job.is_some() {
            ui.spinner();
            ui.label("Authenticating the backup locally. No wallet files are written.");
            if ghost_button(ui, "Discard verification result").clicked() {
                self.discard = true;
                self.clear();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(100));
            return None;
        }
        if let Some(preview) = &self.preview {
            let full = preview.recovered.is_full_state();
            ui.label("Verified backup address — compare with your own records:");
            ui.add(egui::Label::new(egui::RichText::new(&preview.address).monospace()).wrap());
            if full {
                ui.label(format!(
                    "Carried over — scan height: {} · History entries: {} · Reservations: {}",
                    preview.scanned_to, preview.history, preview.reservations
                ));
                ui.colored_label(WARN, "Full import: the saved scan position, coins, history and reservations all come across, so there is no rescan from genesis. Everything the backup was missing when it was written is still missing.");
                if preview.pending > 0 {
                    ui.colored_label(DANGER, format!("{} unresolved payment(s) come with this backup and are WITHHELD. They will not be broadcast again on their own. A backup is a photograph: those payments may since have confirmed, expired, or had their coins spent another way, and this file cannot tell which. Check each one against the chain before spending.", preview.pending));
                } else {
                    ui.label("No unresolved payments in this backup.");
                }
                ui.label("Reservations come across untouched. They hold coins a swap may still be relying on; this import cannot tell whether it is. Keep the original backup.");
            } else {
                ui.label(format!("In the source, not copied — scan height: {} · History entries: {} · Pending sends: {} · Reservations: {}",
                    preview.scanned_to, preview.history, preview.pending, preview.reservations));
                ui.colored_label(WARN, "Keys-only recovery: old history, pending sends and reservations are NOT copied. The new wallet scans from genesis after a separate unlock. Already broadcast payments are not cancelled.");
                ui.label("Keep the original backup. Review earlier payments and any unfinished swaps before spending. The saved height and counts are local metadata, not verified chain state.");
            }
            ui.add_space(12.0);
            field(
                ui,
                &mut self.new_password,
                "backup-recovery-new",
                "New Vault password (at least 12 characters)",
            );
            field(
                ui,
                &mut self.confirmation,
                "backup-recovery-confirm",
                "Repeat new Vault password",
            );
            let consent = if full {
                "I checked the address and want the full saved state. I understand the withheld payments are unresolved, and I will check them against the chain before spending."
            } else {
                "I checked the address and want keys-only recovery. I will keep the original backup and review prior payments before spending."
            };
            ui.checkbox(&mut self.acknowledged, consent);
            let action = if full {
                "Import full state into new Vault"
            } else {
                "Recover keys into new Vault"
            };
            if primary_button(ui, action, self.acknowledged).clicked() {
                match self.request() {
                    Ok(request) => return Some(request),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        } else {
            ui.label("Open an encrypted .nfv backup to preview its address. Existing wallets cannot be replaced, and the backup file is never modified.");
            ui.add_space(8.0);
            ui.label("What should be recovered?");
            // `radio_value` looks right and cannot be operated by assistive
            // technology in this build: macOS sends the accessibility press,
            // egui exposes the control with the correct role and bounds, and
            // nothing happens — while the same press on an ordinary button
            // works. Measured during GUI review on 11 September 2026, not
            // assumed. A choice that only a mouse can make is not a choice
            // everyone can make, and this one decides whether old payments
            // come across, so it uses controls that actually respond.
            ui.selectable_value(
                &mut self.full_state,
                false,
                "Keys only — rescan the chain from genesis",
            );
            ui.selectable_value(
                &mut self.full_state,
                true,
                "Full saved state — no rescan, unresolved payments withheld",
            );
            ui.colored_label(
                if self.full_state { WARN } else { TEXT_DIM },
                if self.full_state {
                    "Faster, and it keeps history the chain alone cannot show you. It also carries whatever the backup was wrong about."
                } else {
                    "Slower, and it forgets any payment the chain cannot show. That is what makes it the cautious choice."
                },
            );
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.path)
                    .id_salt("backup-recovery-path")
                    .char_limit(4096)
                    .desired_width(ui.available_width())
                    .margin(FIELD_MARGIN)
                    .hint_text("Absolute path to your .nfv backup (no ~ expansion)"),
            );
            field(
                ui,
                &mut self.old_password,
                "backup-recovery-old",
                "Password used by this backup",
            );
            if primary_button(
                ui,
                "Preview backup",
                !self.path.is_empty() && !self.old_password.is_empty(),
            )
            .clicked()
            {
                self.inspect(network, ui.ctx().clone());
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(DANGER, error);
        }
        ui.add_space(8.0);
        if ghost_button(ui, "Back — discard recovery input").clicked() {
            self.clear();
            self.discard = true;
            self.back = true;
        }
        None
    }
}

fn field(ui: &mut egui::Ui, value: &mut Zeroizing<String>, id: &str, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(&mut **value)
            .id_salt(id)
            .password(true)
            .char_limit(1024)
            .desired_width(ui.available_width())
            .margin(FIELD_MARGIN)
            .hint_text(hint),
    );
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    use nightfall_wallet::{vault::Vault, Wallet};
    use std::{fs, path::PathBuf, sync::Arc};
    const PASSWORD: &str = "public unfunded backup password";
    const NEW: &str = "public new recovery vault password";
    const NETWORK: NetworkId = NetworkId::Devnet;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            // The clock alone is not a unique name. Tests run in parallel
            // threads of one process and macOS does not hand out a distinct
            // nanosecond to each of them, so two fixtures raced for the same
            // directory and one lost. The counter makes the name unique by
            // construction; the clock stays only to keep leftovers readable.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "nightfall-backup-recovery-{}-{nonce}-{seq}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("nightfall-backup-recovery-"));
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn preview() -> Preview {
        let keys = WalletKeys::from_seed([0; 32]);
        Preview {
            recovered: Recovered::Keys(Zeroizing::new(keys.to_mnemonic())),
            address: keys.address().encode(),
            scanned_to: 123,
            history: 4,
            pending: 2,
            reservations: 1,
        }
    }

    /// An in-memory wallet at scan height 75 holding one resendable payment.
    /// Public unfunded fixture: empty transaction, no node, no files.
    fn pending_send_wallet() -> Wallet {
        let mut wallet = Wallet::in_memory(NETWORK, WalletKeys::from_seed([7; 32]), 75);
        let tx = nightfall_ledger::Transaction {
            version: nightfall_types::PROTOCOL_VERSION,
            inputs: vec![],
            outputs: vec![],
            kernels: vec![],
        };
        wallet
            .record_send(&tx, 1_000, "public fixture".into())
            .unwrap();
        assert_eq!(wallet.resendable().len(), 1, "fixture must be resendable");
        wallet
    }

    /// A full-state preview whose wallet still holds a resendable payment.
    /// Only a fixture: the real reader withholds before it ever gets here.
    fn unsafe_full_state_preview() -> Preview {
        let wallet = pending_send_wallet();
        Preview {
            address: wallet.address().encode(),
            scanned_to: 75,
            history: 1,
            pending: 1,
            reservations: 0,
            recovered: Recovered::State(Box::new(wallet)),
        }
    }
    fn wait(state: &mut BackupRecovery) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while state.job.is_some() {
            state.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn backup_recovery_preview_and_new_vault_preserve_source_and_remain_locked() {
        let f = Fixture::new();
        let path = f.0.join("original.nfv");
        let original = Wallet::in_memory(NETWORK, WalletKeys::from_seed([0; 32]), 100);
        let sealed = Vault::create(&original, PASSWORD).unwrap();
        fs::write(&path, sealed.sealed_bytes()).unwrap();
        let root = f.0.join("recovered");
        fs::create_dir(&root).unwrap();
        let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());
        let mut state = BackupRecovery::new(false);
        state.path = path.display().to_string();
        *state.old_password = "wrong password for test".into();
        state.inspect(NETWORK, egui::Context::default());
        wait(&mut state);
        assert!(state.preview.is_none() && state.error.is_some());
        assert!(!root.join("core.seed.vault").exists());
        state.path = path.display().to_string();
        *state.old_password = PASSWORD.into();
        state.inspect(NETWORK, egui::Context::default());
        assert!(state.path.is_empty() && state.old_password.is_empty());
        wait(&mut state);
        assert!(state.error.is_none());
        assert_eq!(
            state.preview.as_ref().unwrap().address,
            original.address().encode()
        );
        assert!(!root.join("core.seed.vault").exists());
        *state.new_password = NEW.into();
        *state.confirmation = NEW.into();
        assert!(state.request().is_err());
        state.acknowledged = true;
        *state.confirmation = "mismatch".into();
        assert!(state.request().is_err());
        *state.confirmation = NEW.into();
        let request = state.request().unwrap();
        assert!(state.preview.is_none() && state.new_password.is_empty());
        let mut wallet = crate::wallet_state::WalletState::empty();
        wallet
            .initialize_vault(lock.clone(), NETWORK, request.source, &request.password)
            .unwrap();
        assert_eq!(wallet.custody(), crate::wallet_state::Custody::Locked);
        drop(wallet);
        let app = crate::app::App::with_data_lock(NETWORK, lock.clone());
        assert!(app.node.is_none() && app.onboarding.is_none());
        let mut wallet = app.wallet.lock().unwrap();
        assert!(wallet.unlock_vault(PASSWORD).is_err());
        wallet.unlock_vault(NEW).unwrap();
        assert_eq!(wallet.address().unwrap(), original.address());
        assert_eq!(wallet.scanned_to(), 0);
        assert!(wallet.history().is_empty() && wallet.outputs().is_empty());
        assert_eq!(fs::read(&path).unwrap(), sealed.sealed_bytes());
    }

    /// The full import keeps what keys-only throws away — and still withholds.
    ///
    /// This is the whole reason the second mode exists: no rescan from genesis.
    /// It is also the reason it needs a gate, because everything the backup was
    /// carrying comes with it, including a payment that was in flight.
    #[test]
    fn full_state_recovery_keeps_the_scan_position_and_withholds_old_sends() {
        let f = Fixture::new();
        let path = f.0.join("full.nfv");
        let original = pending_send_wallet();
        let sealed = Vault::create(&original, PASSWORD).unwrap();
        fs::write(&path, sealed.sealed_bytes()).unwrap();
        let root = f.0.join("restored");
        fs::create_dir(&root).unwrap();
        let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());

        let mut state = BackupRecovery::new(false);
        state.full_state = true;
        state.path = path.display().to_string();
        *state.old_password = PASSWORD.into();
        state.inspect(NETWORK, egui::Context::default());
        wait(&mut state);
        assert!(state.error.is_none(), "{:?}", state.error);
        let preview = state.preview.as_ref().unwrap();
        assert!(preview.recovered.is_full_state());
        assert_eq!(preview.address, original.address().encode());
        assert_eq!(preview.scanned_to, 75, "the position must come across");
        assert_eq!(preview.history, 1);
        assert_eq!(preview.pending, 1, "the in-flight payment must be counted");

        *state.new_password = NEW.into();
        *state.confirmation = NEW.into();
        state.acknowledged = true;
        let request = state.request().unwrap();
        assert!(
            matches!(request.source, ProvisionSource::Restored(_)),
            "a full-state preview must not provision as keys-only"
        );

        let mut wallet = crate::wallet_state::WalletState::empty();
        wallet
            .initialize_vault(lock.clone(), NETWORK, request.source, &request.password)
            .unwrap();
        drop(wallet);

        // Reopen from disk: the point is that this survives the seal.
        let app = crate::app::App::with_data_lock(NETWORK, lock.clone());
        let mut wallet = app.wallet.lock().unwrap();
        wallet.unlock_vault(NEW).unwrap();
        assert_eq!(wallet.address().unwrap(), original.address());
        assert_eq!(wallet.scanned_to(), 75, "no rescan from genesis");
        assert_eq!(wallet.history().len(), 1, "history came across");
        assert!(
            wallet.history().iter().all(|entry| entry.quarantined),
            "the imported payment must stay withheld after a restart"
        );
        assert_eq!(fs::read(&path).unwrap(), sealed.sealed_bytes());
    }

    /// The gate before provisioning, not just the reader, has to hold.
    ///
    /// `recover_backup_state` withholds imported sends, so in practice this
    /// never triggers. It exists so that a future reader which forgets to is
    /// caught here, with the wallet still unwritten, rather than on the wire.
    #[test]
    fn a_restored_state_that_could_still_broadcast_is_refused() {
        let f = Fixture::new();
        let root = f.0.join("refused");
        fs::create_dir(&root).unwrap();
        let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());

        let mut state = BackupRecovery::new(false);
        state.preview = Some(unsafe_full_state_preview());
        state.acknowledged = true;
        *state.new_password = NEW.into();
        *state.confirmation = NEW.into();
        // No `unwrap_err`: ProvisionRequest deliberately has no Debug, because
        // it carries a seed and a password. Match instead of loosening that.
        let error = match state.request() {
            Ok(_) => panic!("a state that could still broadcast must be refused"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("broadcast automatically"), "{error}");
        assert!(
            !root.join("core.seed.vault").exists(),
            "nothing was written"
        );

        // And the same refusal again one layer down, so neither gate alone is
        // load-bearing.
        let mut wallet = crate::wallet_state::WalletState::empty();
        let error = wallet
            .initialize_vault(
                lock,
                NETWORK,
                ProvisionSource::Restored(Box::new(pending_send_wallet())),
                NEW,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("broadcast automatically"), "{error}");
        assert!(
            !root.join("core.seed.vault").exists(),
            "nothing was written"
        );
    }

    /// A restored state from another network must not be provisioned.
    #[test]
    fn a_restored_state_from_another_network_is_refused() {
        let f = Fixture::new();
        let root = f.0.join("network");
        fs::create_dir(&root).unwrap();
        let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());
        let foreign = Wallet::in_memory(NetworkId::Testnet, WalletKeys::from_seed([9; 32]), 0);
        let mut wallet = crate::wallet_state::WalletState::empty();
        let error = wallet
            .initialize_vault(
                lock,
                NETWORK,
                ProvisionSource::Restored(Box::new(foreign)),
                NEW,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("different network"), "{error}");
        assert!(!root.join("core.seed.vault").exists());
    }

    /// Losing focus must not leave the wider import armed for the next attempt.
    #[test]
    fn the_full_import_choice_does_not_survive_concealment() {
        let mut state = BackupRecovery::new(false);
        state.full_state = true;
        state.observe_activity(false, true, false, Instant::now());
        assert!(
            !state.full_state,
            "after concealment the cautious mode must be the one on offer"
        );
    }

    #[test]
    fn backup_recovery_discards_late_preview_after_focus_loss() {
        let mut state = BackupRecovery::new(true);
        let (send, recv) = std::sync::mpsc::channel();
        state.job = Some(std::thread::spawn(move || {
            recv.recv().unwrap();
            Ok(preview())
        }));
        *state.new_password = NEW.into();
        state.observe_activity(false, true, false, Instant::now());
        send.send(()).unwrap();
        wait(&mut state);
        assert!(state.preview.is_none() && state.new_password.is_empty() && state.discard);
        assert!(state.request().is_err());
    }

    #[test]
    fn backup_recovery_requires_verified_preview_and_conceals_on_idle() {
        let mut state = BackupRecovery::new(false);
        state.path = "relative.nfv".into();
        *state.old_password = PASSWORD.into();
        state.inspect(NETWORK, egui::Context::default());
        assert!(state.job.is_none() && state.error.is_some());
        assert!(state.request().is_err());
        state.preview = Some(preview());
        state.acknowledged = true;
        *state.new_password = "short".into();
        *state.confirmation = "short".into();
        assert!(state.request().is_err());
        state.observe_activity(
            true,
            false,
            true,
            state.last_activity + Duration::from_secs(300),
        );
        assert!(
            state.preview.is_none()
                && state.new_password.is_empty()
                && state.old_password.is_empty()
        );
        assert!(state.path.is_empty() && !state.acknowledged);
    }

    #[test]
    fn backup_recovery_cards_fit_narrow_windows_and_full_address() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for width in [320.0, 620.0, 760.0] {
            // 3 and 4 cover the full-import copy, which is the longest text on
            // this screen and the easiest to push past a narrow window.
            for case in 0..5 {
                let mut recovery = BackupRecovery::new(case == 1);
                if (1..=2).contains(&case) {
                    recovery.preview = Some(preview());
                }
                if case == 2 {
                    recovery.error =
                        Some("Wrong password or damaged backup. Nothing was written.".into());
                }
                if case == 3 {
                    recovery.preview = Some(unsafe_full_state_preview());
                }
                if case == 4 {
                    recovery.full_state = true;
                }
                let mut state = crate::onboarding::Onboarding::Backup(Box::new(recovery));
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 620.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let bounds = ui.max_rect();
                            assert!(state.show_panel(ui, true, NETWORK).is_none());
                            assert!(
                                ui.min_rect().right() <= bounds.right() + 1.0,
                                "recovery width overflow at {width}"
                            );
                            assert!(ui.min_rect().bottom() <= bounds.bottom() + 1.0);
                        });
                    },
                );
            }
        }
    }
}
