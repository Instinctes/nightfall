//! Read-only recovery rehearsal. No filesystem, network or wallet replacement.
use nightfall_crypto::{Address, WalletKeys};
use thiserror::Error;

/// Deliberately does not contain the entered phrase, seed, or another address.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("Enter this wallet's complete 24 recovery words.")]
    InvalidPhrase,
    #[error("These valid words belong to a different wallet. Nothing was changed.")]
    DifferentWallet,
}

/// Check full key-derived address equality, not merely a mnemonic checksum.
/// `expected` must come from the currently selected wallet, not untrusted input.
/// The caller owns clearing the UI text and must not log it or send it anywhere.
pub fn verify_phrase(expected: &Address, phrase: &str) -> Result<(), RecoveryError> {
    // Bound user input before normalization/allocation in the mnemonic parser.
    if phrase.len() > 1024 {
        return Err(RecoveryError::InvalidPhrase);
    }
    let candidate = WalletKeys::from_mnemonic(phrase).map_err(|_| RecoveryError::InvalidPhrase)?;
    if candidate.address() != *expected {
        return Err(RecoveryError::DifferentWallet);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Wallet;
    use nightfall_types::NetworkId;

    #[test]
    fn correct_words_do_not_modify_wallet_state() {
        let wallet = Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([31; 32]), 123);
        let before = wallet.export_state().unwrap();
        assert_eq!(
            verify_phrase(&wallet.address(), &wallet.recovery_phrase()),
            Ok(())
        );
        assert_eq!(wallet.export_state().unwrap(), before);
    }

    #[test]
    fn valid_but_wrong_wallet_is_not_a_successful_backup() {
        let a = WalletKeys::from_seed([31; 32]);
        let b = WalletKeys::from_seed([32; 32]);
        assert_eq!(
            verify_phrase(&a.address(), &b.to_mnemonic()),
            Err(RecoveryError::DifferentWallet)
        );
    }

    #[test]
    fn normalizes_case_and_spacing_without_changing_derivation() {
        let keys = WalletKeys::from_seed([31; 32]);
        let words = format!(
            "  {}\n",
            keys.to_mnemonic().to_uppercase().replace(' ', " \n ")
        );
        assert_eq!(verify_phrase(&keys.address(), &words), Ok(()));
    }

    #[test]
    fn bad_length_checksum_and_oversized_input_are_rejected_without_echoing_secrets() {
        let keys = WalletKeys::from_seed([31; 32]);
        for phrase in [
            "abandon ".repeat(24),
            "secret sentence".into(),
            "a".repeat(1025),
        ] {
            let error = verify_phrase(&keys.address(), &phrase).unwrap_err();
            assert_eq!(error, RecoveryError::InvalidPhrase);
            assert!(!format!("{error:?} {error}").contains(&phrase));
        }
    }
}
