//! Local recovery rehearsal UI. The active wallet is never replaced or saved.
use crate::{theme::*, widgets::*};
use eframe::egui::{self, RichText};
use nightfall_crypto::Address;
use nightfall_wallet::recovery::{verify_phrase, RecoveryError};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub struct RecoveryStudio {
    phrase: Zeroizing<String>,
    result: Option<Result<(), RecoveryError>>,
}

impl RecoveryStudio {
    pub fn clear(&mut self) {
        self.phrase.zeroize();
        self.result = None;
    }

    fn verify(&mut self, expected: &Address) {
        self.result = Some(verify_phrase(expected, &self.phrase));
        self.phrase.zeroize();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, address: Option<&Address>) {
        titled_card(ui, "Recovery Studio", |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Check your backup before you need it")
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(
                "Enter the 24 words from your offline backup. This checks the complete address \
                 against this wallet, locally. It does not replace files or send a transaction."
            ).size(12.0).color(TEXT_DIM));
            ui.add_space(12.0);
            let response = ui.add_enabled(
                address.is_some(),
                egui::TextEdit::singleline(&mut *self.phrase)
                    .id_salt("recovery-rehearsal-words")
                    .password(true)
                    .char_limit(1024)
                    .margin(FIELD_MARGIN)
                    .desired_width(ui.available_width())
                    .hint_text("Your 24 recovery words"),
            );
            if response.changed() {
                self.result = None;
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                let can_verify = address.is_some() && !self.phrase.trim().is_empty();
                let clicked = ui
                    .add_enabled(can_verify, egui::Button::new("Check recovery words"))
                    .clicked();
                let enter = can_verify
                    && response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if clicked || enter {
                    if let Some(expected) = address {
                        self.verify(expected);
                    }
                }
                if ghost_button(ui, "Clear").clicked() {
                    self.clear();
                }
            });
            if let Some(result) = &self.result {
                ui.add_space(10.0);
                match result {
                    Ok(()) => {
                        ui.label(
                            RichText::new("Backup matches this wallet. Entered words cleared.")
                                .color(SUCCESS),
                        );
                    }
                    Err(error) => {
                        ui.label(RichText::new(error.to_string()).color(DANGER));
                    }
                }
            }
            if address.is_none() {
                ui.label(
                    RichText::new("Open your wallet before checking its backup.").color(TEXT_FAINT),
                );
            }
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label(
                RichText::new("What the words do not restore")
                    .size(12.0)
                    .strong(),
            );
            ui.label(
                RichText::new(
                    "Contacts and preferences need a separate backup. Experimental swap secrets \
                 cannot be reconstructed from the 24 words. A successful check does not verify \
                 your balance, synchronization or a swap's recovery state.",
                )
                .size(12.0)
                .color(TEXT_DIM),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(
                "Only enter recovery words in a trusted wallet on your own device. Never share them with support."
            ).size(11.0).color(TEXT_FAINT));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    #[test]
    fn rehearsal_clears_input_after_success_failure_and_cancel() {
        let keys = WalletKeys::from_seed([41; 32]);
        let mut studio = RecoveryStudio::default();
        *studio.phrase = keys.to_mnemonic();
        studio.verify(&keys.address());
        assert_eq!(studio.result, Some(Ok(())));
        assert!(studio.phrase.is_empty());
        *studio.phrase = WalletKeys::from_seed([42; 32]).to_mnemonic();
        studio.verify(&keys.address());
        assert_eq!(studio.result, Some(Err(RecoveryError::DifferentWallet)));
        assert!(studio.phrase.is_empty());
        *studio.phrase = "unfinished sensitive input".into();
        studio.clear();
        assert!(studio.phrase.is_empty());
        assert!(studio.result.is_none());
    }

    #[test]
    fn card_and_feedback_fit_supported_widths() {
        let context = egui::Context::default();
        crate::theme::apply(&context);
        let keys = WalletKeys::from_seed([41; 32]);
        for width in [320.0, 620.0, 760.0] {
            for result in [
                None,
                Some(Ok(())),
                Some(Err(RecoveryError::DifferentWallet)),
            ] {
                let mut studio = RecoveryStudio {
                    result,
                    ..Default::default()
                };
                let _ = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let right = ui.max_rect().right();
                            studio.show(ui, Some(&keys.address()));
                            assert!(
                                ui.min_rect().right() <= right + 1.0,
                                "Recovery Studio overflow at {width}"
                            );
                        });
                    },
                );
            }
        }
    }
}
