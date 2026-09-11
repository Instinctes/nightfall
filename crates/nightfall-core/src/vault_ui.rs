//! Native Vault controls. Password/KDF work owns the wallet off the UI thread.
//! Workers cannot publish an unlocked wallet; only the foreground poll can.
use crate::{
    theme::*,
    wallet_state::{Custody, WalletState},
    widgets::*,
};
use eframe::egui;
use nightfall_storage::dirlock::DirLock;
use nightfall_types::NetworkId;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

const AUTO_LOCK: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Prepare,
    Retire,
    Unlock,
    Lock,
    ChangePassword,
    ExportBackup,
    VerifyBackup,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::onboarding::ProvisionSource;
    use nightfall_crypto::WalletKeys;
    use nightfall_wallet::Wallet;
    use std::{fs, path::PathBuf};

    /// The words path, spelled once so the call sites stay readable.
    fn word_source(seed: u8) -> ProvisionSource {
        ProvisionSource::Words(Zeroizing::new(
            WalletKeys::from_seed([seed; 32]).to_mnemonic(),
        ))
    }

    const PASSWORD: &str = "public unfunded core vault password";
    const OTHER: &str = "another public core vault password";
    const NETWORK: NetworkId = NetworkId::Devnet;

    struct Fixture {
        root: PathBuf,
        lock: Arc<DirLock>,
    }
    impl Fixture {
        fn new() -> Self {
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
                "nightfall-core-vault-test-{}-{nonce}-{seq}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let lock = Arc::new(nightfall_storage::dirlock::acquire(&root).unwrap());
            Self { root, lock }
        }
        fn legacy(&self, funded_fixture: bool) -> WalletState {
            let mut wallet = Wallet::in_memory(NETWORK, WalletKeys::from_seed([0; 32]), 0);
            if funded_fixture {
                use nightfall_consensus::{Block, BlockHeader};
                use nightfall_ledger::{build_coinbase, BlockBody, LedgerState};
                use nightfall_types::{Hash256, Height, PROTOCOL_VERSION};
                let reward = 1_000_000;
                for height in 0..2 {
                    let cb =
                        build_coinbase(&wallet.address(), reward, height, NETWORK.proof_context())
                            .unwrap();
                    let body = BlockBody::aggregate(&[cb]);
                    let mut ledger = LedgerState::genesis();
                    ledger
                        .apply_block(&body, Height(height), reward, NETWORK.proof_context())
                        .unwrap();
                    let block = Block {
                        header: BlockHeader {
                            version: PROTOCOL_VERSION,
                            height: Height(height),
                            prev_hash: Hash256::ZERO,
                            utxo_root: ledger.utxo_root(),
                            kernel_sum: ledger.kernel_sum(),
                            body_root: body.hash(),
                            timestamp_unix: height + 1,
                            difficulty: 1,
                            nonce: 0,
                            reward_darks: reward,
                        },
                        body,
                    };
                    wallet.scan_blocks(&[block]).unwrap();
                }
            }
            let export: serde_json::Value =
                serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
            fs::write(
                self.root.join("core.seed"),
                export["seed"].as_str().unwrap(),
            )
            .unwrap();
            fs::write(
                self.root.join("core.seed.outputs.json"),
                serde_json::to_vec(&export["db"]).unwrap(),
            )
            .unwrap();
            WalletState::load_or_create(&self.root, NETWORK).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self
                .root
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("nightfall-core-vault-test-"));
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn credentials(ui: &mut VaultUi, password: &str) {
        *ui.password = password.into();
        *ui.confirmation = password.into();
        ui.acknowledged = true;
    }

    fn backup_fields(ui: &mut VaultUi, path: &std::path::Path) {
        ui.backup_path = path.to_str().unwrap().into();
        *ui.backup_password = PASSWORD.into();
        ui.backup_acknowledged = true;
    }

    #[test]
    fn backup_worker_exports_verifies_and_respects_focus_loss_without_node_or_import() {
        let f = Fixture::new();
        let external = Fixture::new();
        let path = external.root.join("offline.nfv");
        let mut state = WalletState::empty();
        state
            .initialize_vault(f.lock.clone(), NETWORK, word_source(0), PASSWORD)
            .unwrap();
        state.unlock_vault(PASSWORD).unwrap();
        let wallet = Arc::new(Mutex::new(state));
        let paused = Arc::new(AtomicBool::new(false));
        let mut ui = VaultUi::default();
        ui.refresh(&wallet);
        backup_fields(&mut ui, &path);
        let held = wallet.lock().unwrap();
        ui.start(
            Action::ExportBackup,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            egui::Context::default(),
        );
        assert!(ui.busy() && paused.load(Ordering::SeqCst));
        assert!(ui.backup_password.is_empty() && ui.backup_path.is_empty());
        ui.observe_activity(false, true, false, Instant::now());
        assert!(!path.exists());
        drop(held);
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_none(), "{:?}", ui.error);
        assert!(ui
            .notice
            .as_ref()
            .unwrap()
            .contains("saved, reopened and verified"));
        assert_eq!(ui.custody, Custody::Locked);
        assert!(paused.load(Ordering::SeqCst));
        let original = fs::read(f.root.join("core.seed.vault/wallet.nfv")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        wallet.lock().unwrap().unlock_vault(PASSWORD).unwrap();
        ui.refresh(&wallet);
        backup_fields(&mut ui, &path);
        ui.start(
            Action::VerifyBackup,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            egui::Context::default(),
        );
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_none());
        assert!(ui
            .notice
            .as_ref()
            .unwrap()
            .contains("matched the complete wallet state"));
        assert_eq!(ui.custody, Custody::Unlocked);
        assert_eq!(fs::read(&path).unwrap(), original);
        backup_fields(&mut ui, &path);
        ui.start(
            Action::ExportBackup,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            egui::Context::default(),
        );
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_some() && ui.notice.is_none());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            fs::read(f.root.join("core.seed.vault/wallet.nfv")).unwrap(),
            original
        );
    }

    #[test]
    fn backup_controls_refuse_unconfirmed_relative_or_locked_requests_and_clear_on_hide() {
        let f = Fixture::new();
        let wallet = Arc::new(Mutex::new(WalletState::empty()));
        let paused = Arc::new(AtomicBool::new(false));
        let mut ui = VaultUi::default();
        for case in 0..4 {
            ui.custody = Custody::Unlocked;
            backup_fields(&mut ui, &f.root.join("not-created.nfv"));
            match case {
                0 => ui.backup_acknowledged = false,
                1 => ui.backup_path = "relative.nfv".into(),
                2 => ui.backup_password.zeroize(),
                _ => ui.custody = Custody::Locked,
            }
            ui.start(
                Action::ExportBackup,
                wallet.clone(),
                paused.clone(),
                Some(f.lock.clone()),
                NETWORK,
                egui::Context::default(),
            );
            assert!(ui.error.is_some() && !ui.busy());
            assert!(!paused.load(Ordering::SeqCst));
            assert!(!f.root.join("not-created.nfv").exists());
        }
        backup_fields(&mut ui, &f.root.join("not-created.nfv"));
        ui.observe_activity(false, true, false, Instant::now());
        assert!(
            ui.backup_path.is_empty() && ui.backup_password.is_empty() && !ui.backup_acknowledged
        );
    }

    fn wait(ui: &mut VaultUi, wallet: &Arc<Mutex<WalletState>>, paused: &Arc<AtomicBool>) {
        let deadline = Instant::now() + Duration::from_secs(180);
        while ui.busy() {
            ui.poll(wallet, paused);
            assert!(Instant::now() < deadline, "vault worker did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn ui_migration_unlock_focus_loss_and_lock_wait_for_exclusive_access() {
        let f = Fixture::new();
        let state = f.legacy(false);
        let address = state.address().unwrap();
        let wallet = Arc::new(Mutex::new(state));
        let paused = Arc::new(AtomicBool::new(false));
        let ctx = egui::Context::default();
        let mut ui = VaultUi::default();
        ui.refresh(&wallet);
        credentials(&mut ui, PASSWORD);
        // Simulate an in-flight scanner holding the mutex. Starting the worker
        // and rendering its card must not wait for that scanner on the UI thread.
        let scanner = wallet.lock().unwrap();
        ui.start(
            Action::Prepare,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        assert!(ui.busy() && paused.load(Ordering::SeqCst));
        assert!(ui.password.is_empty() && ui.confirmation.is_empty());
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |egui_ui| {
                ui.show(egui_ui, true);
            });
        });
        drop(scanner);
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Migration);
        assert!(f.root.join("core.seed.outputs.json").exists());
        assert!(!wallet.lock().unwrap().can_scan());
        credentials(&mut ui, OTHER);
        ui.start(
            Action::Retire,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_some());
        assert!(f.root.join("core.seed.outputs.json").exists());
        credentials(&mut ui, PASSWORD);
        ui.start(
            Action::Retire,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Locked);
        assert!(ui.error.is_none());
        assert!(!f.root.join("core.seed.outputs.json").exists());
        credentials(&mut ui, PASSWORD);
        ui.start(
            Action::Unlock,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        ui.observe_activity(false, false, false, Instant::now());
        wait(&mut ui, &wallet, &paused);
        assert_eq!(
            ui.custody,
            Custody::Locked,
            "a background unlock must not publish keys after focus loss"
        );
        assert!(wallet.lock().unwrap().address().is_none());
        credentials(&mut ui, PASSWORD);
        ui.start(
            Action::Unlock,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Unlocked);
        assert_eq!(wallet.lock().unwrap().address(), Some(address));
        assert!(!paused.load(Ordering::SeqCst));
        credentials(&mut ui, OTHER);
        ui.start(
            Action::ChangePassword,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        ui.observe_activity(false, false, false, Instant::now());
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Locked);
        assert!(ui.error.is_none());
        credentials(&mut ui, PASSWORD);
        ui.start(
            Action::Unlock,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_some() && ui.custody == Custody::Locked);
        credentials(&mut ui, OTHER);
        ui.start(
            Action::Unlock,
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            ctx.clone(),
        );
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Unlocked);
        let scanner = wallet.lock().unwrap();
        ui.start(
            Action::Lock,
            wallet.clone(),
            paused.clone(),
            None,
            NETWORK,
            ctx,
        );
        assert!(ui.busy());
        assert_ne!(
            ui.custody,
            Custody::Locked,
            "waiting for a scan is not yet a completed lock"
        );
        drop(scanner);
        wait(&mut ui, &wallet, &paused);
        assert_eq!(ui.custody, Custody::Locked);
        assert!(!wallet.lock().unwrap().can_scan());
    }

    #[test]
    fn invalid_ui_credentials_do_not_enroll_or_pause_legacy_storage() {
        let f = Fixture::new();
        let wallet = Arc::new(Mutex::new(f.legacy(false)));
        let paused = Arc::new(AtomicBool::new(false));
        let mut ui = VaultUi::default();
        ui.refresh(&wallet);
        for invalid in ["short", PASSWORD] {
            credentials(&mut ui, invalid);
            if invalid == PASSWORD {
                *ui.confirmation = OTHER.into();
            }
            ui.start(
                Action::Prepare,
                wallet.clone(),
                paused.clone(),
                Some(f.lock.clone()),
                NETWORK,
                egui::Context::default(),
            );
            assert!(ui.error.is_some() && !ui.busy());
            assert!(!paused.load(Ordering::SeqCst));
            assert!(!f.root.join("core.seed.vault").exists());
        }
        credentials(&mut ui, PASSWORD);
        ui.acknowledged = false;
        ui.start(
            Action::Prepare,
            wallet,
            paused,
            Some(f.lock.clone()),
            NETWORK,
            egui::Context::default(),
        );
        assert!(!ui.busy() && !f.root.join("core.seed.vault").exists());
    }

    #[test]
    fn auto_lock_expiry_focus_and_hidden_windows_clear_password_fields() {
        let mut ui = VaultUi {
            custody: Custody::Unlocked,
            ..Default::default()
        };
        let start = ui.last_activity;
        assert!(!ui.observe_activity(true, false, false, start + Duration::from_secs(299)));
        assert!(ui.observe_activity(true, false, true, start + AUTO_LOCK));
        credentials(&mut ui, PASSWORD);
        assert!(ui.observe_activity(false, false, false, start));
        assert!(ui.password.is_empty() && ui.confirmation.is_empty());
        credentials(&mut ui, PASSWORD);
        assert!(ui.observe_activity(true, true, false, start));
        assert!(!ui.acknowledged && ui.password.is_empty());
    }

    #[test]
    fn custody_cards_fit_mobile_sized_and_desktop_content_widths() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for width in [320.0, 620.0, 760.0] {
            for custody in [
                Custody::Empty,
                Custody::Legacy,
                Custody::Migration,
                Custody::Locked,
                Custody::Unlocked,
                Custody::Failed,
            ] {
                for has_snapshot in [true, false] {
                    let mut ui_state = VaultUi { custody, has_snapshot, error: Some(
                        "Wrong password or damaged vault. Original wallet files were preserved."
                            .into(),
                    ), ..Default::default() };
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1100.0),
                        )),
                        ..Default::default()
                    };
                    let _ = ctx.run(input, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let right = ui.max_rect().right();
                            ui_state.show(ui, true);
                            assert!(
                                ui.min_rect().right() <= right + 1.0,
                                "{custody:?} overflow at {width}"
                            );
                        });
                    });
                }
            }
        }
    }

    #[test]
    fn unfinished_swaps_block_migration_before_any_vault_marker() {
        let f = Fixture::new();
        let mut wallet = f.legacy(false);
        let swap = nightfall_swap::persist::StoredSwap::new(
            nightfall_swap::SwapState::new(nightfall_swap::Role::Bob),
            1,
            2,
        );
        nightfall_swap::persist::save(&f.root, &swap).unwrap();
        assert!(wallet
            .prepare_vault(f.lock.clone(), NETWORK, PASSWORD)
            .is_err());
        assert_eq!(wallet.custody(), Custody::Legacy);
        assert!(!f.root.join("core.seed.vault").exists());
    }

    #[test]
    fn enrollment_rejects_a_different_directory_or_network_without_dropping_the_wallet() {
        let f = Fixture::new();
        let other = Fixture::new();
        let mut wallet = f.legacy(false);
        let address = wallet.address();
        assert!(wallet
            .prepare_vault(other.lock.clone(), NETWORK, PASSWORD)
            .is_err());
        assert!(wallet
            .prepare_vault(f.lock.clone(), NetworkId::Mainnet, PASSWORD)
            .is_err());
        assert_eq!(wallet.address(), address);
        assert_eq!(wallet.custody(), Custody::Legacy);
        assert!(
            !f.root.join("core.seed.vault").exists()
                && !other.root.join("core.seed.vault").exists()
        );
    }

    #[test]
    fn core_payment_is_durable_before_return_and_failed_save_yields_no_transaction() {
        let f = Fixture::new();
        let mut wallet = f.legacy(true);
        let address = wallet.address();
        wallet
            .prepare_vault(f.lock.clone(), NETWORK, PASSWORD)
            .unwrap();
        wallet.finish_vault_migration(PASSWORD).unwrap();
        wallet.unlock_vault(PASSWORD).unwrap();
        let recipient = WalletKeys::from_seed([1; 32]).address();
        let tx = wallet
            .prepare_payment(&recipient, 100, 10, "public test", 100, 0)
            .unwrap();
        let txid = tx.txid().to_hex();
        drop(wallet);
        let mut wallet = WalletState::open_vault(f.lock.clone(), NETWORK).unwrap();
        wallet.unlock_vault(PASSWORD).unwrap();
        assert_eq!(wallet.address(), address);
        assert_eq!(wallet.history()[0].txid, txid);
        assert!(wallet.history()[0].raw.is_some());
        assert_eq!(
            wallet
                .outputs()
                .iter()
                .filter(|output| output.spent)
                .count(),
            1
        );
        // A failed encrypted edit cannot release an output or yield a payment.
        let path = f.root.join("core.seed.vault/wallet.nfv");
        fs::write(&path, b"changed outside the store").unwrap();
        assert!(wallet.reserve_commits(&["ab".repeat(32)]).is_err());
        assert!(wallet
            .prepare_payment(&recipient, 100, 10, "not broadcast", 100, 0)
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"changed outside the store");
        assert_eq!(wallet.history().len(), 3); // two mined fixtures and one pending payment
        wallet.lock_vault();
        assert!(wallet.reserve_commits(&[]).is_err());
        assert!(wallet.release_commits(&[]).is_err());
    }

    #[test]
    fn onboarding_worker_saves_encrypted_only_and_reopens_locked() {
        let f = Fixture::new();
        let keys = WalletKeys::from_seed([0; 32]);
        let phrase = Zeroizing::new(keys.to_mnemonic());
        let wallet = Arc::new(Mutex::new(WalletState::empty()));
        let paused = Arc::new(AtomicBool::new(true));
        let mut ui = VaultUi::default();
        // Hold the shared state: spawning and rendering must not block on it.
        let held = wallet.lock().unwrap();
        ui.start_provision(
            wallet.clone(),
            paused.clone(),
            Some(f.lock.clone()),
            NETWORK,
            crate::onboarding::ProvisionRequest {
                source: crate::onboarding::ProvisionSource::Words(phrase.clone()),
                password: Zeroizing::new(PASSWORD.into()),
            },
            egui::Context::default(),
        );
        assert!(ui.busy() && paused.load(Ordering::SeqCst));
        ui.observe_activity(false, true, false, Instant::now());
        assert!(!f.root.join("core.seed.vault").exists());
        drop(held);
        wait(&mut ui, &wallet, &paused);
        assert!(ui.error.is_none(), "{:?}", ui.error);
        assert_eq!(ui.custody, Custody::Locked);
        assert!(paused.load(Ordering::SeqCst));
        assert!(wallet.lock().unwrap().address().is_none());
        assert!(!f.root.join("core.seed.outputs.json").exists());
        let marker = fs::read_to_string(f.root.join("core.seed")).unwrap();
        assert!(marker.starts_with("NIGHTFALL ENCRYPTED VAULT v1\n"));
        assert!(!marker.contains(phrase.as_str()));
        let snapshot = fs::read(f.root.join("core.seed.vault/wallet.nfv")).unwrap();
        assert!(snapshot.starts_with(b"NFVAULT\0"));
        assert!(!snapshot
            .windows(phrase.len())
            .any(|window| window == phrase.as_bytes()));
        drop(wallet);
        let app = crate::app::App::with_data_lock(NETWORK, f.lock.clone());
        assert!(app.node.is_none() && app.onboarding.is_none());
        let mut state = app.wallet.lock().unwrap();
        state.unlock_vault(PASSWORD).unwrap();
        assert_eq!(state.address().unwrap(), keys.address());
        assert_eq!(state.recovery_phrase(), *phrase);
        assert!(state.history().is_empty());
        let original = fs::read(f.root.join("core.seed.vault/wallet.nfv")).unwrap();
        assert!(state
            .initialize_vault(f.lock.clone(), NETWORK, word_source(1), PASSWORD)
            .is_err());
        assert_eq!(
            fs::read(f.root.join("core.seed.vault/wallet.nfv")).unwrap(),
            original
        );
        assert_eq!(state.address().unwrap(), keys.address());
    }

    #[test]
    fn onboarding_preflight_refuses_invalid_input_and_existing_wallet_files() {
        let f = Fixture::new();
        let phrase = WalletKeys::from_seed([0; 32]).to_mnemonic();
        let mut state = WalletState::empty();
        for (words, password) in [("bad secret input", PASSWORD), (&phrase, "short")] {
            assert!(state
                .initialize_vault(
                    f.lock.clone(),
                    NETWORK,
                    ProvisionSource::Words(Zeroizing::new(words.to_string())),
                    password
                )
                .is_err());
            assert!(!f.root.join("core.seed.vault").exists());
        }
        // Orphaned/corrupt files count as existing data, not a new wallet.
        fs::write(
            f.root.join("core.seed.outputs.json"),
            b"corrupt existing data",
        )
        .unwrap();
        assert!(state
            .initialize_vault(
                f.lock.clone(),
                NETWORK,
                ProvisionSource::Words(Zeroizing::new(phrase.clone())),
                PASSWORD
            )
            .is_err());
        assert!(!f.root.join("core.seed.vault").exists());
        assert_eq!(
            fs::read(f.root.join("core.seed.outputs.json")).unwrap(),
            b"corrupt existing data"
        );
    }

    #[test]
    fn onboarding_empty_marker_restart_offers_original_words_not_new_seed() {
        let f = Fixture::new();
        // Simulated interruption before the first snapshot commit.
        let state = WalletState::open_vault(f.lock.clone(), NETWORK).unwrap();
        assert!(state.awaiting_initial_restore());
        drop(state);
        let app = crate::app::App::with_data_lock(NETWORK, f.lock.clone());
        assert!(app.wallet_load_error.is_none() && app.node.is_none());
        assert!(matches!(
            app.onboarding,
            Some(crate::onboarding::Onboarding::Setup(_))
        ));
        let phrase = WalletKeys::from_seed([0; 32]).to_mnemonic();
        let mut state = app.wallet.lock().unwrap();
        state
            .initialize_vault(
                f.lock.clone(),
                NETWORK,
                ProvisionSource::Words(Zeroizing::new(phrase.clone())),
                PASSWORD,
            )
            .unwrap();
        assert!(!state.awaiting_initial_restore());
        assert_eq!(state.custody(), Custody::Locked);
    }

    #[test]
    fn onboarding_snapshot_without_tombstone_resumes_verification_not_replacement() {
        let f = Fixture::new();
        let mut state = WalletState::empty();
        let phrase = WalletKeys::from_seed([0; 32]).to_mnemonic();
        state
            .initialize_vault(
                f.lock.clone(),
                NETWORK,
                ProvisionSource::Words(Zeroizing::new(phrase.clone())),
                PASSWORD,
            )
            .unwrap();
        drop(state);
        // Simulate a crash after encrypted commit and before tombstone creation.
        fs::remove_file(f.root.join("core.seed")).unwrap();
        let app = crate::app::App::with_data_lock(NETWORK, f.lock.clone());
        assert!(app.node.is_none() && app.onboarding.is_none());
        assert_eq!(app.vault_ui.custody, Custody::Migration);
        let mut state = app.wallet.lock().unwrap();
        assert!(state.finish_vault_migration(OTHER).is_err());
        assert!(!f.root.join("core.seed").exists());
        state.finish_vault_migration(PASSWORD).unwrap();
        assert_eq!(state.custody(), Custody::Locked);
        state.unlock_vault(PASSWORD).unwrap();
        assert_eq!(state.recovery_phrase(), phrase);
    }

    #[test]
    fn core_reopens_vault_locked_without_starting_a_node() {
        let f = Fixture::new();
        let mut wallet = f.legacy(false);
        wallet
            .prepare_vault(f.lock.clone(), NETWORK, PASSWORD)
            .unwrap();
        wallet.finish_vault_migration(PASSWORD).unwrap();
        drop(wallet);
        let app = crate::app::App::with_data_lock(NETWORK, f.lock.clone());
        assert!(app.wallet_load_error.is_none() && app.onboarding.is_none());
        assert!(app.node.is_none());
        assert_eq!(app.vault_ui.custody, Custody::Locked);
        assert!(app.wallet_paused.load(Ordering::SeqCst));
        drop(app);
        fs::write(
            f.root.join("core.seed.vault/wallet.nfv"),
            b"damaged snapshot",
        )
        .unwrap();
        let app = crate::app::App::with_data_lock(NETWORK, f.lock.clone());
        assert!(app.wallet_load_error.is_some());
        assert!(app.node.is_none() && app.onboarding.is_none());
    }
}

struct JobResult {
    wallet: Option<WalletState>,
    result: Result<(), String>,
    payment: Option<Result<String, String>>,
    notice: Option<String>,
}

pub struct PaymentRequest {
    pub node: Arc<nightfall_node::NodeHandle>,
    pub to: Zeroizing<String>,
    pub amount: u64,
    pub fee: u64,
    pub memo: Zeroizing<String>,
}

pub struct VaultUi {
    pub custody: Custody,
    pub has_snapshot: bool,
    password: Zeroizing<String>,
    confirmation: Zeroizing<String>,
    acknowledged: bool,
    backup_path: String,
    backup_password: Zeroizing<String>,
    backup_acknowledged: bool,
    notice: Option<String>,
    pub error: Option<String>,
    pub payment_result: Option<Result<String, String>>,
    payment_active: bool,
    job: Option<std::thread::JoinHandle<JobResult>>,
    completed: Option<JobResult>,
    discard_unlock: bool,
    last_activity: Instant,
}

impl Default for VaultUi {
    fn default() -> Self {
        Self {
            custody: Custody::Empty,
            has_snapshot: false,
            password: Zeroizing::new(String::new()),
            confirmation: Zeroizing::new(String::new()),
            acknowledged: false,
            backup_path: String::new(),
            backup_password: Zeroizing::new(String::new()),
            backup_acknowledged: false,
            notice: None,
            error: None,
            payment_result: None,
            payment_active: false,
            job: None,
            completed: None,
            discard_unlock: false,
            last_activity: Instant::now(),
        }
    }
}

impl VaultUi {
    pub fn needs_lock(&self) -> bool {
        self.discard_unlock && self.custody == Custody::Unlocked
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.completed.is_some()
    }

    pub fn clear_fields(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
        self.acknowledged = false;
        self.backup_path.zeroize();
        self.backup_password.zeroize();
        self.backup_acknowledged = false;
    }

    pub fn observe_activity(
        &mut self,
        focused: bool,
        hidden: bool,
        activity: bool,
        now: Instant,
    ) -> bool {
        if !focused || hidden {
            self.clear_fields();
            self.discard_unlock = true;
            return self.custody == Custody::Unlocked;
        }
        // Check expiry before accepting a newly arriving input event.
        let expired = now.saturating_duration_since(self.last_activity) >= AUTO_LOCK;
        if activity {
            self.last_activity = now;
        }
        expired && self.custody == Custody::Unlocked
    }

    pub fn refresh(&mut self, wallet: &Arc<Mutex<WalletState>>) {
        if !self.busy() {
            if let Ok(wallet) = wallet.try_lock() {
                if self.custody == Custody::Failed && wallet.custody() == Custody::Empty {
                    return;
                }
                self.custody = wallet.custody();
                self.has_snapshot = wallet.has_vault_snapshot();
            }
        }
    }

    /// Returns true when a worker result was published to the shared wallet.
    pub fn poll(&mut self, wallet: &Arc<Mutex<WalletState>>, paused: &Arc<AtomicBool>) -> bool {
        if self.job.as_ref().is_some_and(|job| job.is_finished()) {
            self.completed = Some(match self.job.take().unwrap().join() {
                Ok(result) => result,
                Err(_) => JobResult { wallet: None, payment: None, notice: None, result: Err("Wallet worker stopped unexpectedly. Restart the wallet to inspect the saved state.".into()) },
            });
        }
        let Some(result) = self.completed.as_mut() else {
            return false;
        };
        let Ok(mut shared) = wallet.try_lock() else {
            return false;
        };
        if let Some(mut state) = result.wallet.take() {
            if self.discard_unlock {
                state.lock_vault();
            }
            self.custody = state.custody();
            self.has_snapshot = state.has_vault_snapshot();
            *shared = state;
        } else {
            self.custody = Custody::Failed;
        }
        self.error = result.result.as_ref().err().cloned();
        self.notice = result.notice.take();
        self.payment_result = result.payment.take();
        self.payment_active = false;
        self.completed = None;
        self.last_activity = Instant::now();
        paused.store(
            !matches!(self.custody, Custody::Legacy | Custody::Unlocked),
            Ordering::SeqCst,
        );
        true
    }

    pub fn start(
        &mut self,
        action: Action,
        wallet: Arc<Mutex<WalletState>>,
        paused: Arc<AtomicBool>,
        data_lock: Option<Arc<DirLock>>,
        network: NetworkId,
        ctx: egui::Context,
    ) {
        if action == Action::Lock {
            self.discard_unlock = true;
            self.clear_fields();
            if self.busy() {
                return;
            }
            paused.store(true, Ordering::SeqCst);
            if let Ok(mut state) = wallet.try_lock() {
                state.lock_vault();
                self.custody = state.custody();
                return;
            }
        } else if self.busy() {
            return;
        }
        let backup_action = matches!(action, Action::ExportBackup | Action::VerifyBackup);
        if backup_action
            && (self.custody != Custody::Unlocked
                || !self.backup_acknowledged
                || self.backup_password.is_empty()
                || self.backup_path.len() > 4096
                || !std::path::Path::new(&self.backup_path).is_absolute())
        {
            self.error = Some("Unlock Vault, enter an absolute backup filename and its password, and confirm the backup notice.".into());
            return;
        }
        if matches!(action, Action::Prepare | Action::ChangePassword) {
            if self.password != self.confirmation {
                self.error = Some("The passwords do not match.".into());
                return;
            }
            if let Err(error) = nightfall_wallet::vault::Vault::validate_password(&self.password) {
                self.error = Some(error.to_string());
                return;
            }
        }
        if matches!(action, Action::Prepare | Action::Retire) && !self.acknowledged {
            self.error = Some("Confirm the recovery and backup notice first.".into());
            return;
        }
        if action == Action::Prepare && data_lock.is_none() {
            self.error = Some(
                "The wallet directory lock is unavailable. Restart Core before migration.".into(),
            );
            return;
        }
        self.discard_unlock = action == Action::Lock;
        self.payment_active = false;
        let password = Zeroizing::new(if backup_action {
            std::mem::take(&mut *self.backup_password)
        } else {
            std::mem::take(&mut *self.password)
        });
        let backup_path = std::path::PathBuf::from(std::mem::take(&mut self.backup_path));
        self.clear_fields();
        self.error = None;
        self.notice = None;
        paused.store(true, Ordering::SeqCst);
        let result = std::thread::Builder::new()
            .name("nightfall-vault".into())
            .spawn(move || {
                let mut state = match wallet.lock() {
                    Ok(mut shared) => std::mem::replace(&mut *shared, WalletState::empty()),
                    Err(_) => {
                        return JobResult {
                            wallet: None,
                            result: Err("Wallet state lock failed. Restart Core.".into()),
                            payment: None,
                            notice: None,
                        }
                    }
                };
                let mut notice = None;
                let result = match action {
                    Action::Prepare => state.prepare_vault(data_lock.unwrap(), network, &password),
                    Action::Retire => state.finish_vault_migration(&password),
                    Action::Unlock => state.unlock_vault(&password),
                    Action::ChangePassword => state.change_vault_password(&password),
                    Action::ExportBackup => state.export_vault_backup(&backup_path, &password).map(|()| {
                        notice = Some("Encrypted backup saved, reopened and verified against the complete wallet state. Keep it offline with its password; future password changes do not update this copy.".into());
                    }),
                    Action::VerifyBackup => state.verify_vault_backup(&backup_path, &password).map(|same| {
                        notice = Some(if same {
                            "Backup opened successfully and matched the complete wallet state at the time of this check. Nothing was imported or broadcast."
                        } else {
                            "Backup opened successfully and belongs to this wallet, but its saved state differs from the current wallet. It may be older. Nothing was imported or broadcast."
                        }.into());
                    }),
                    Action::Lock => {
                        state.lock_vault();
                        Ok(())
                    }
                }
                .map_err(|error| error.to_string());
                ctx.request_repaint();
                JobResult {
                    wallet: Some(state),
                    result,
                    payment: None,
                    notice,
                }
            });
        match result {
            Ok(job) => self.job = Some(job),
            Err(_) => {
                self.error = Some("Could not start the vault worker. Nothing was changed.".into());
                paused.store(
                    action == Action::Lock
                        || !matches!(self.custody, Custody::Legacy | Custody::Unlocked),
                    Ordering::SeqCst,
                );
            }
        }
    }

    pub fn start_provision(
        &mut self,
        wallet: Arc<Mutex<WalletState>>,
        paused: Arc<AtomicBool>,
        data_lock: Option<Arc<DirLock>>,
        network: NetworkId,
        request: crate::onboarding::ProvisionRequest,
        ctx: egui::Context,
    ) {
        if self.busy() {
            return;
        }
        let Some(data_lock) = data_lock.filter(|_| cfg!(unix)) else {
            self.error = Some(
                "Encrypted setup requires a supported platform and the wallet directory lock."
                    .into(),
            );
            return;
        };
        self.clear_fields();
        self.error = None;
        self.notice = None;
        self.discard_unlock = true;
        self.payment_active = false;
        paused.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("nightfall-vault-setup".into())
            .spawn(move || {
                let mut state = match wallet.lock() {
                    Ok(mut shared) => std::mem::replace(&mut *shared, WalletState::empty()),
                    Err(_) => {
                        return JobResult {
                            wallet: None,
                            payment: None,
                            notice: None,
                            result: Err("Wallet state lock failed. Restart Core.".into()),
                        }
                    }
                };
                let result = state
                    .initialize_vault(data_lock, network, request.source, &request.password)
                    .map_err(|error| error.to_string());
                state.lock_vault();
                ctx.request_repaint();
                JobResult {
                    wallet: Some(state),
                    result,
                    payment: None,
                    notice: None,
                }
            });
        match spawned {
            Ok(job) => self.job = Some(job),
            Err(_) => {
                self.error = Some("Could not start encrypted setup. No wallet was created.".into())
            }
        }
    }

    pub fn start_payment(
        &mut self,
        wallet: Arc<Mutex<WalletState>>,
        paused: Arc<AtomicBool>,
        request: PaymentRequest,
        ctx: egui::Context,
    ) {
        if self.busy() {
            return;
        }
        self.discard_unlock = false;
        self.payment_active = true;
        self.clear_fields();
        self.error = None;
        self.payment_result = None;
        self.notice = None;
        paused.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name("nightfall-payment".into())
            .spawn(move || {
                let mut state = match wallet.lock() {
                    Ok(mut shared) => std::mem::replace(&mut *shared, WalletState::empty()),
                    Err(_) => {
                        return JobResult {
                            wallet: None,
                            result: Err("Wallet state lock failed. Restart Core.".into()),
                            payment: None,
                            notice: None,
                        }
                    }
                };
                let payment = state
                    .send(
                        &request.node,
                        &request.to,
                        request.amount,
                        request.fee,
                        &request.memo,
                    )
                    .map_err(|error| error.to_string());
                let result = payment.as_ref().map(|_| ()).map_err(Clone::clone);
                ctx.request_repaint();
                JobResult {
                    wallet: Some(state),
                    result,
                    payment: Some(payment),
                    notice: None,
                }
            });
        match spawned {
            Ok(job) => self.job = Some(job),
            Err(_) => {
                self.error =
                    Some("Could not start payment preparation. No payment was created.".into());
                paused.store(
                    !matches!(self.custody, Custody::Legacy | Custody::Unlocked),
                    Ordering::SeqCst,
                );
            }
        }
    }

    /// Shared card for Settings and the full-window locked/migration screen.
    pub fn show(&mut self, ui: &mut egui::Ui, available: bool) -> Option<Action> {
        let mut action = None;
        titled_card(
            ui,
            if self.payment_active {
                "Payment status"
            } else {
                "Nightfall Vault"
            },
            |ui| {
                if self.busy() || self.needs_lock() {
                    ui.spinner();
                    ui.heading(if self.discard_unlock {
                        "Securing your wallet…"
                    } else if self.payment_active {
                        "Preparing your payment…"
                    } else {
                        "Verifying and saving…"
                    });
                    ui.label(if self.payment_active {
                    "Let the current payment operation finish. It may already have been submitted; check Activity before trying again."
                } else {
                    "Waiting for any current wallet operation, then completing the vault operation. Do not force quit during migration."
                });
                    if let Some(error) = &self.error {
                        ui.colored_label(DANGER, error);
                    }
                    return;
                }
                let (title, description) = match self.custody {
                Custody::Legacy => ("Protect this wallet", "This wallet still uses legacy storage. Vault encrypts the seed, outputs and history without changing your address."),
                Custody::Migration if self.has_snapshot => ("Verify your encrypted copy", "Your encrypted copy is saved. Re-enter its password to verify it against the original files before replacing the legacy secrets."),
                Custody::Migration => ("Resume wallet encryption", "Enrollment was interrupted before a complete encrypted copy was saved. Your legacy files will be read and verified; no new seed will be generated."),
                Custody::Locked => ("Your wallet is locked", "Unlock to scan your balance and prepare payments. An already running node or miner can continue while wallet access is locked."),
                Custody::Unlocked => ("Vault is unlocked", "Automatic lock: five minutes without input, loss of focus or hiding the window. Existing backup copies retain their original protection."),
                Custody::Failed => ("Wallet needs attention", "A storage operation could not be completed safely. Restart Core to inspect the saved state. Do not delete wallet files or create a replacement seed."),
                Custody::Empty => ("No wallet is open", "Create or restore a wallet before enabling Vault."),
            };
                ui.heading(title);
                ui.add_space(8.0);
                ui.label(description);
                if let Some(error) = &self.error {
                    ui.add_space(8.0);
                    ui.colored_label(DANGER, error);
                }
                if let Some(notice) = &self.notice {
                    ui.add_space(8.0);
                    ui.label(notice);
                }
                if matches!(self.custody, Custody::Failed | Custody::Empty) {
                    return;
                }
                if !available {
                    ui.add_space(8.0);
                    ui.label("Vault enrollment is not available in this build/platform or while a swap operation is running.");
                    return;
                }
                if self.custody == Custody::Unlocked {
                    ui.add_space(12.0);
                    if ghost_button(ui, "Lock wallet now").clicked() {
                        action = Some(Action::Lock);
                    }
                    ui.add_space(8.0);
                    ui.label("Change vault password");
                }
                let new_password = matches!(self.custody, Custody::Legacy | Custody::Unlocked)
                    || (self.custody == Custody::Migration && !self.has_snapshot);
                ui.add_space(GAP_MD);
                text_field(
                    ui,
                    "vault-password",
                    if new_password {
                        "New password"
                    } else {
                        "Password"
                    },
                    if new_password {
                        "At least 12 characters"
                    } else {
                        ""
                    },
                    &mut self.password,
                    true,
                );
                if new_password {
                    text_field(
                        ui,
                        "vault-password-confirm",
                        "Password again",
                        "",
                        &mut self.confirmation,
                        true,
                    );
                    ui.label(egui::RichText::new("Use a password manager or a long unique passphrase — never your recovery words.").size(11.5).color(TEXT_FAINT));
                }
                let migrating = matches!(self.custody, Custody::Legacy | Custody::Migration);
                if migrating {
                    ui.add_space(12.0);
                    ui.checkbox(&mut self.acknowledged, "I have my offline recovery backup and understand that old backups and experimental swap secrets are not erased or encrypted by this migration.");
                    ui.label("Contacts and preferences are outside Vault. Interrupted legacy save files must be resolved before continuing.");
                }
                let (label, requested) = match self.custody {
                    Custody::Unlocked => ("Save new password", Action::ChangePassword),
                    Custody::Locked => ("Unlock wallet", Action::Unlock),
                    Custody::Migration if self.has_snapshot => {
                        ("Verify and replace legacy secrets", Action::Retire)
                    }
                    _ => ("Encrypt a verified copy", Action::Prepare),
                };
                // Why this is not a disabled button.
                //
                // It was one, and on the lock screen a disabled primary button
                // is almost indistinguishable from an enabled one — same pill,
                // same size, a slightly different fill. Pressing it did
                // nothing and said nothing, so the wallet looked broken rather
                // than incomplete. The control now stays live and answers:
                // press it with an empty field and it tells you the field is
                // empty. A reason is worth more than a grey rectangle.
                let blocker = if self.password.is_empty() {
                    Some(if new_password {
                        "Choose a password first."
                    } else {
                        "Enter your password first."
                    })
                } else if new_password && *self.password != *self.confirmation {
                    Some("The two passwords are not the same.")
                } else if migrating && !self.acknowledged {
                    Some("Confirm the backup notice above first.")
                } else {
                    None
                };
                ui.add_space(GAP_MD);
                match button_row(ui, &[label, "Clear fields"], true) {
                    Some(0) => match blocker {
                        Some(reason) => self.error = Some(reason.to_owned()),
                        None => action = Some(requested),
                    },
                    Some(1) => {
                        self.clear_fields();
                        self.error = None;
                    }
                    _ => {}
                }
                if self.custody == Custody::Unlocked {
                    ui.add_space(20.0);
                    ui.separator();
                    ui.heading("Encrypted backups");
                    ui.label("Save a separate .nfv file with the committed seed, outputs and history. No existing file is overwritten. Contacts, preferences and experimental swap secrets are not included.");
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.backup_path)
                            .id_salt("vault-backup-path")
                            .char_limit(4096)
                            .desired_width(ui.available_width())
                            .margin(FIELD_MARGIN)
                            .hint_text(
                                "Absolute filename, e.g. /Volumes/Offline/Nightfall-2026-09-10.nfv",
                            ),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut *self.backup_password)
                            .id_salt("vault-backup-password")
                            .password(true)
                            .char_limit(1024)
                            .desired_width(ui.available_width())
                            .margin(FIELD_MARGIN)
                            .hint_text("Current password to save; backup password to check"),
                    );
                    ui.label("Use a new filename outside the wallet data folder, preferably on an offline device. A failed write may leave an incomplete new file; retry with another filename. No ~ expansion is applied.");
                    ui.checkbox(&mut self.backup_acknowledged, "I understand that backups retain their own password and may contain old state. Checking does not import or send transactions.");
                    let ready = self.backup_acknowledged
                        && !self.backup_password.is_empty()
                        && std::path::Path::new(&self.backup_path).is_absolute();
                    ui.add_space(8.0);
                    if primary_button(ui, "Save encrypted backup", ready).clicked() {
                        action = Some(Action::ExportBackup);
                    }
                    ui.add_space(8.0);
                    if primary_button(ui, "Check backup (read only)", ready).clicked() {
                        action = Some(Action::VerifyBackup);
                    }
                    ui.label("A successful check proves decryption and wallet identity, not chain validity, current balances or freshness. Importing old snapshots is not enabled in this development build.");
                }
            },
        );
        action
    }
}
