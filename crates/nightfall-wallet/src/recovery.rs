//! Read-only recovery rehearsal. No filesystem, network or wallet replacement.
//!
//! Every place that turns typed words into keys goes through [`keys_from_phrase`]
//! so that one mistake has one wording. Three code paths used to say three
//! different things about the same typo, and the differences were not
//! improvements — see the note on [`RecoveryError`].
use nightfall_crypto::{Address, MnemonicError, WalletKeys, MNEMONIC_WORDS};
use thiserror::Error;

/// 24 BIP-39 words are at most about 200 bytes. This bounds what the parser is
/// asked to normalise and copy, and is far above any real phrase.
const MAX_PHRASE_BYTES: usize = 1024;

/// Deliberately does not contain the entered phrase, seed, or another address.
///
/// The split between the first two variants is not cosmetic. They used to be
/// one variant reading "Enter this wallet's complete 24 recovery words", which
/// is true advice for a phrase that is 23 words long and actively misleading
/// for one that is 24 words with a single wrong word: the owner counts to 24,
/// reads that the words are incomplete, and concludes the wallet is broken
/// rather than that they misread their own handwriting. The word count is the
/// owner's own input echoed back and is not a secret.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("That is {0} words. Recovery needs all 24, in the order you wrote them down.")]
    WrongLength(usize),
    #[error(
        "These 24 words do not check out together, so at least one of them is \
         not right. Compare each word with what you wrote down."
    )]
    NotAValidPhrase,
    #[error("These valid words belong to a different wallet. Nothing was changed.")]
    DifferentWallet,
}

/// Turn a typed phrase into keys, saying which kind of mistake was made.
///
/// The caller owns clearing the UI text and must not log it or send it anywhere.
pub fn keys_from_phrase(phrase: &str) -> Result<WalletKeys, RecoveryError> {
    // Counting allocates nothing, so it is safe to do before the length bound —
    // and doing it first is what lets a 23-word phrase be named as such rather
    // than lumped in with a corrupt one.
    let count = phrase.split_whitespace().count();
    if count != MNEMONIC_WORDS {
        return Err(RecoveryError::WrongLength(count));
    }
    // Twenty-four "words" that do not fit in a kilobyte are not a phrase; no
    // BIP-39 word is longer than eight characters.
    if phrase.len() > MAX_PHRASE_BYTES {
        return Err(RecoveryError::NotAValidPhrase);
    }
    WalletKeys::from_mnemonic(phrase).map_err(|error| match error {
        // Unreachable in practice: the parser splits on whitespace exactly as
        // the count above does. Mapped rather than ignored so that a future
        // change to either side cannot silently become "not a valid phrase".
        MnemonicError::WrongLength(n) => RecoveryError::WrongLength(n),
        MnemonicError::Invalid(_) => RecoveryError::NotAValidPhrase,
    })
}

/// Check full key-derived address equality, not merely a mnemonic checksum.
/// `expected` must come from the currently selected wallet, not untrusted input.
/// The caller owns clearing the UI text and must not log it or send it anywhere.
pub fn verify_phrase(expected: &Address, phrase: &str) -> Result<(), RecoveryError> {
    if keys_from_phrase(phrase)?.address() != *expected {
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
        for (phrase, expected) in [
            // 24 real words whose checksum does not hold.
            ("abandon ".repeat(24), RecoveryError::NotAValidPhrase),
            ("secret sentence".into(), RecoveryError::WrongLength(2)),
            ("a".repeat(1025), RecoveryError::WrongLength(1)),
            // 24 tokens that are not words, and do not fit the byte bound.
            (
                vec!["a".repeat(64); 24].join(" "),
                RecoveryError::NotAValidPhrase,
            ),
        ] {
            let error = verify_phrase(&keys.address(), &phrase).unwrap_err();
            assert_eq!(error, expected, "phrase of {} bytes", phrase.len());
            assert!(!format!("{error:?} {error}").contains(&phrase));
        }
    }

    /// The regression this file exists to prevent from coming back: a phrase
    /// that is the right length and wrong in one word must not be described as
    /// incomplete, and must not be confused with a phrase for another wallet.
    #[test]
    fn one_wrong_word_is_named_as_a_wrong_word_and_not_as_a_missing_one() {
        let keys = WalletKeys::from_seed([31; 32]);
        let phrase = keys.to_mnemonic();
        let mut words: Vec<&str> = phrase.split_whitespace().collect();

        let short = words[..23].join(" ");
        assert_eq!(
            verify_phrase(&keys.address(), &short),
            Err(RecoveryError::WrongLength(23)),
        );
        assert!(format!("{}", RecoveryError::WrongLength(23)).contains("23 words"));

        // Replace the last word with another real word from the same phrase, so
        // the input is 24 known words that fail only the checksum.
        words[23] = words[0];
        let mistyped = words.join(" ");
        let error = verify_phrase(&keys.address(), &mistyped).unwrap_err();
        assert_eq!(error, RecoveryError::NotAValidPhrase);
        let text = error.to_string();
        assert!(!text.contains("complete"), "{text}");
        assert!(!text.contains("different wallet"), "{text}");

        // …and a valid phrase for another wallet still gets its own message.
        assert_eq!(
            verify_phrase(&keys.address(), &WalletKeys::from_seed([32; 32]).to_mnemonic()),
            Err(RecoveryError::DifferentWallet),
        );
    }

    #[test]
    fn keys_from_phrase_is_the_one_road_in() {
        let keys = WalletKeys::from_seed([31; 32]);
        assert_eq!(
            keys_from_phrase(&keys.to_mnemonic()).unwrap().address(),
            keys.address(),
        );
        // Whatever verify_phrase accepts, keys_from_phrase accepts identically.
        for phrase in ["", "abandon", &"abandon ".repeat(24), &keys.to_mnemonic()] {
            let a = keys_from_phrase(phrase).err();
            let b = match verify_phrase(&keys.address(), phrase) {
                Err(RecoveryError::DifferentWallet) => None,
                other => other.err(),
            };
            assert_eq!(a, b, "phrase {phrase:?}");
        }
    }
}
