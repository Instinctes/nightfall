//! Encrypted-first setup. Secrets stay in memory until the Vault worker commits.
use crate::{theme::*, widgets::*};
use eframe::egui;
use nightfall_crypto::{Address, WalletKeys};
use nightfall_types::NetworkId;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// Where a new Vault's contents come from.
///
/// The two are mutually exclusive by construction. A request that could carry
/// both words and a restored state would leave someone downstream deciding
/// which one wins, and getting that wrong means either a wallet that silently
/// forgets its history or one that silently resumes payments it should not.
pub enum ProvisionSource {
    /// The 24 recovery words. The new wallet begins at genesis and rescans.
    Words(Zeroizing<String>),
    /// A complete state read out of an encrypted backup: scan position, coins,
    /// history and reservations. Imported sends are already withheld by
    /// `recover_backup_state`, and nothing downstream may undo that.
    Restored(Box<nightfall_wallet::Wallet>),
}

pub struct ProvisionRequest {
    pub source: ProvisionSource,
    pub password: Zeroizing<String>,
}

pub enum Onboarding {
    Choice,
    Setup(Box<Setup>),
    Backup(Box<crate::backup_recovery::BackupRecovery>),
}

pub struct Setup {
    phrase: Zeroizing<String>,
    expected: Option<Address>,
    verification: Zeroizing<String>,
    password: Zeroizing<String>,
    confirmation: Zeroizing<String>,
    reveal: bool,
    verified: bool,
    interrupted: bool,
    acknowledged: bool,
    error: Option<String>,
    last_activity: Instant,
}

impl Setup {
    fn restore(interrupted: bool) -> Self {
        Self {
            phrase: Zeroizing::new(String::new()),
            expected: None,
            verification: Zeroizing::new(String::new()),
            password: Zeroizing::new(String::new()),
            confirmation: Zeroizing::new(String::new()),
            reveal: false,
            verified: false,
            interrupted,
            acknowledged: false,
            error: None,
            last_activity: Instant::now(),
        }
    }

    fn create(keys: WalletKeys) -> Self {
        let mut setup = Self::restore(false);
        setup.phrase = Zeroizing::new(keys.to_mnemonic());
        setup.expected = Some(keys.address());
        setup
    }

    fn conceal(&mut self) {
        self.reveal = false;
        self.verification.zeroize();
        self.password.zeroize();
        self.confirmation.zeroize();
        self.acknowledged = false;
        self.error = None;
        if self.expected.is_none() {
            self.phrase.zeroize();
        }
        // Retain a newly generated identity until explicit cancellation. It has
        // not been saved yet; focus loss must not silently substitute new keys.
    }

    fn verify(&mut self) {
        self.reveal = false;
        self.verified = false;
        self.error = self.expected.as_ref().and_then(|address| {
            nightfall_wallet::recovery::verify_phrase(address, &self.verification)
                .err()
                .map(|error| error.to_string())
        });
        self.verified = self.expected.is_some() && self.error.is_none();
        self.verification.zeroize();
    }

    fn request(&mut self) -> anyhow::Result<ProvisionRequest> {
        anyhow::ensure!(
            self.expected.is_none() || self.verified,
            "Verify your written backup first."
        );
        anyhow::ensure!(
            self.acknowledged,
            "Confirm the recovery and password notice first."
        );
        anyhow::ensure!(
            self.password == self.confirmation,
            "The passwords do not match."
        );
        nightfall_wallet::vault::Vault::validate_password(&self.password)?;
        // One road in, so a 23-word phrase and a mistyped word are told apart
        // here exactly as they are in the web wallet.
        nightfall_wallet::recovery::keys_from_phrase(&self.phrase)?;
        let request = ProvisionRequest {
            // One zeroizing worker copy, never a per-frame display clone.
            source: ProvisionSource::Words(self.phrase.clone()),
            password: Zeroizing::new(std::mem::take(&mut *self.password)),
        };
        self.confirmation.zeroize();
        self.verification.zeroize();
        self.reveal = false;
        Ok(request)
    }
}

impl Onboarding {
    pub fn resume() -> Self {
        Self::Setup(Box::new(Setup::restore(true)))
    }

    pub fn observe_activity(&mut self, focused: bool, hidden: bool, activity: bool, now: Instant) {
        if let Self::Backup(recovery) = self {
            recovery.observe_activity(focused, hidden, activity, now);
        }
        if let Self::Setup(setup) = self {
            if !focused
                || hidden
                || now.saturating_duration_since(setup.last_activity) >= Duration::from_secs(300)
            {
                setup.conceal();
            }
            if activity {
                setup.last_activity = now;
            }
        }
    }

    pub fn show_panel(
        &mut self,
        ui: &mut egui::Ui,
        available: bool,
        network: NetworkId,
    ) -> Option<ProvisionRequest> {
        egui::ScrollArea::vertical()
            .id_salt("vault-onboarding-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                narrow_column(ui, 620.0, |ui| self.show(ui, available, network))
            })
            .inner
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        available: bool,
        network: NetworkId,
    ) -> Option<ProvisionRequest> {
        let mut next = None;
        let mut request = None;
        titled_card(ui, "Welcome to Nightfall Vault", |ui| {
            ui.set_width(ui.available_width());
            if !available {
                ui.label("Encrypted setup is not available on this platform or without the wallet directory lock. No plaintext wallet will be created.");
                return;
            }
            match self {
                Self::Backup(recovery) => {
                    request = recovery.show(ui, network);
                    if recovery.take_back() {
                        next = Some(if recovery.resume_only {
                            Self::resume()
                        } else {
                            Self::Choice
                        });
                    }
                }
                Self::Choice => {
                    ui.heading("Your wallet. Protected from the start.");
                    ui.label("Create a wallet or restore your Nightfall 24-word backup. The seed, outputs and history are encrypted before they reach disk.");
                    ui.add_space(16.0);
                    if primary_button(ui, "Create an encrypted wallet", true).clicked() {
                        next = Some(Self::Setup(Box::new(Setup::create(WalletKeys::generate()))));
                    }
                    ui.add_space(8.0);
                    if ghost_button(ui, "Restore my 24 words").clicked() {
                        next = Some(Self::Setup(Box::new(Setup::restore(false))));
                    }
                    ui.add_space(8.0);
                    if ghost_button(ui, "Recover from encrypted backup").clicked() {
                        next = Some(Self::Backup(Box::new(
                            crate::backup_recovery::BackupRecovery::new(false),
                        )));
                    }
                }
                Self::Setup(setup) => {
                    if setup.interrupted {
                        ui.heading("Resume with your original recovery words");
                        ui.colored_label(WARN, "An encryption marker exists, but no committed wallet snapshot was found. Do not generate a replacement wallet. Use the original words or explicitly recover keys from your encrypted backup.");
                    } else {
                        ui.heading(if setup.expected.is_some() {
                            "Back up, verify, protect"
                        } else {
                            "Restore your Nightfall wallet"
                        });
                    }
                    ui.label("Keep the 24 words offline: they allow spending without your Vault password. Nightfall has no password reset. Words recover keys, not local history, labels or experimental swap secrets. Bitcoin/Ethereum phrases do not restore those coins here; Nightfall derives different keys.");
                    ui.add_space(12.0);
                    if setup.expected.is_some() {
                        ui.label("1 · Write down your recovery words");
                        if !setup.verified {
                            if ghost_button(
                                ui,
                                if setup.reveal {
                                    "Hide words and verify backup"
                                } else {
                                    "Reveal recovery words"
                                },
                            )
                            .clicked()
                            {
                                setup.reveal = !setup.reveal;
                                setup.verification.zeroize();
                            }
                            if setup.reveal {
                                ui.label(egui::RichText::new(setup.phrase.as_str()).monospace());
                            } else {
                                ui.label("Re-enter all 24 words from your offline copy. A different valid phrase does not pass this check.");
                                secret_field(
                                    ui,
                                    &mut setup.verification,
                                    "setup-backup",
                                    "Your written 24 words",
                                );
                                if ghost_button(ui, "Verify written backup").clicked() {
                                    setup.verify();
                                }
                            }
                        } else {
                            ui.colored_label(SUCCESS, "Written backup matches this wallet.");
                        }
                    } else {
                        ui.label("1 · Enter your original 24 recovery words");
                        if ghost_button(ui, "Use an encrypted backup instead").clicked() {
                            next = Some(Self::Backup(Box::new(
                                crate::backup_recovery::BackupRecovery::new(setup.interrupted),
                            )));
                        }
                        secret_field(
                            ui,
                            &mut setup.phrase,
                            "setup-restore",
                            "Nightfall recovery words",
                        );
                        ui.label("Case and whitespace are normalized; the checksum is checked locally. Recovery scans from genesis after you unlock.");
                    }
                    ui.add_space(16.0);
                    ui.label("2 · Choose your Vault password");
                    ui.label("At least 12 characters. Use a unique, long passphrase. Spaces are preserved exactly.");
                    secret_field(
                        ui,
                        &mut setup.password,
                        "setup-password",
                        "New Vault password",
                    );
                    secret_field(
                        ui,
                        &mut setup.confirmation,
                        "setup-confirmation",
                        "Repeat Vault password",
                    );
                    ui.checkbox(&mut setup.acknowledged, "I have an offline copy of my words and understand there is no password reset.");
                    if let Some(error) = &setup.error {
                        ui.colored_label(DANGER, error);
                    }
                    ui.add_space(12.0);
                    if primary_button(
                        ui,
                        "Save encrypted wallet",
                        setup.expected.is_none() || setup.verified,
                    )
                    .clicked()
                    {
                        match setup.request() {
                            Ok(value) => {
                                setup.error = None;
                                request = Some(value);
                            }
                            Err(error) => setup.error = Some(error.to_string()),
                        }
                    }
                    if !setup.interrupted {
                        ui.add_space(8.0);
                        if ghost_button(ui, "Cancel setup — discard unsaved input").clicked() {
                            next = Some(Self::Choice);
                        }
                    }
                    ui.label("Saving ends with a locked wallet. Unlock separately to start the node. Nothing is broadcast during setup.");
                }
            }
        });
        if let Some(next) = next {
            *self = next;
        }
        request
    }
}

fn secret_field(ui: &mut egui::Ui, text: &mut Zeroizing<String>, id: &str, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(&mut **text)
            .id_salt(id)
            .password(true)
            .char_limit(1024)
            .margin(FIELD_MARGIN)
            .desired_width(ui.available_width())
            .hint_text(hint),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    const PASSWORD: &str = "public unfunded onboarding password";

    fn prepared() -> Setup {
        let mut setup = Setup::create(WalletKeys::from_seed([0; 32]));
        *setup.password = PASSWORD.into();
        *setup.confirmation = PASSWORD.into();
        setup.acknowledged = true;
        setup
    }

    #[test]
    fn onboarding_requires_matching_full_backup_before_request() {
        let mut setup = prepared();
        assert!(setup.request().is_err());
        *setup.verification = WalletKeys::from_seed([1; 32]).to_mnemonic();
        setup.verify();
        assert!(!setup.verified && setup.verification.is_empty());
        assert!(setup.request().is_err());
        setup.verification = setup.phrase.clone();
        setup.verify();
        assert!(setup.verified && setup.error.is_none());
        let request = setup.request().unwrap();
        match &request.source {
            ProvisionSource::Words(phrase) => assert_eq!(*phrase, setup.phrase),
            ProvisionSource::Restored(_) => panic!("setup must never produce a restored state"),
        }
        assert_eq!(&*request.password, PASSWORD);
        assert!(setup.password.is_empty() && setup.confirmation.is_empty() && !setup.reveal);
    }

    #[test]
    fn onboarding_invalid_credentials_and_words_never_yield_provision_request() {
        let mut setup = Setup::restore(false);
        *setup.phrase = WalletKeys::from_seed([0; 32]).to_mnemonic();
        *setup.password = PASSWORD.into();
        *setup.confirmation = PASSWORD.into();
        assert!(setup.request().is_err());
        setup.acknowledged = true;
        *setup.confirmation = "different".into();
        assert!(setup.request().is_err());
        *setup.password = "short".into();
        *setup.confirmation = "short".into();
        assert!(setup.request().is_err());
        *setup.password = PASSWORD.into();
        *setup.confirmation = PASSWORD.into();
        for words in [
            "abandon ".repeat(24),
            "a".repeat(1025),
            "private input".into(),
        ] {
            *setup.phrase = words.clone();
            let error = setup.request().err().unwrap().to_string();
            assert!(!error.contains(&words));
        }
    }

    #[test]
    fn onboarding_focus_loss_conceals_generated_identity_and_erases_entered_secrets() {
        let mut setup = prepared();
        let address = setup.expected;
        setup.reveal = true;
        *setup.verification = "temporary input".into();
        setup.conceal();
        assert!(!setup.reveal && !setup.acknowledged);
        assert!(
            setup.password.is_empty()
                && setup.confirmation.is_empty()
                && setup.verification.is_empty()
        );
        assert_eq!(
            WalletKeys::from_mnemonic(&setup.phrase).unwrap().address(),
            address.unwrap()
        );
        let mut restore = Setup::restore(true);
        *restore.phrase = "temporary restore input".into();
        restore.conceal();
        assert!(restore.phrase.is_empty());
        let mut idle = prepared();
        idle.reveal = true;
        let start = idle.last_activity;
        let mut state = Onboarding::Setup(Box::new(idle));
        state.observe_activity(true, false, false, start + Duration::from_secs(299));
        assert!(matches!(&state, Onboarding::Setup(setup) if setup.reveal));
        // Expiry is checked before a newly arriving event resets the timer.
        state.observe_activity(true, false, true, start + Duration::from_secs(300));
        assert!(
            matches!(&state, Onboarding::Setup(setup) if !setup.reveal && setup.password.is_empty() && !setup.phrase.is_empty())
        );
    }

    #[test]
    fn onboarding_cards_fit_supported_widths_including_revealed_words() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        for width in [320.0, 620.0, 760.0] {
            for case in 0..6 {
                let mut state = match case {
                    0 => Onboarding::Choice,
                    1 => Onboarding::resume(),
                    2 => Onboarding::Setup(Box::new(Setup::restore(false))),
                    _ => {
                        let mut setup = prepared();
                        setup.reveal = case == 3;
                        setup.verified = case == 4;
                        setup.error = Some("The passwords do not match.".into());
                        Onboarding::Setup(Box::new(setup))
                    }
                };
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
                            let right = ui.max_rect().right();
                            let bottom = ui.max_rect().bottom();
                            assert!(state.show_panel(ui, true, NetworkId::Devnet).is_none());
                            assert!(
                                ui.min_rect().right() <= right + 1.0,
                                "setup {case} overflow at {width}px"
                            );
                            assert!(
                                ui.min_rect().bottom() <= bottom + 1.0,
                                "setup {case} vertical overflow at {width}px"
                            );
                        });
                    },
                );
            }
        }
    }
}
