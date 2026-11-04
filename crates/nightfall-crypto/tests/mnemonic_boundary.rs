//! Deterministic adversarial recovery input tests.
//! Synthetic test seeds only. Never use real recovery words.

use nightfall_crypto::{MnemonicError, WalletKeys};

const CANARY: &str = "sensitive_canary_not_a_bip39_word_7f421";
const TOO_LONG: &str = "recovery phrase exceeds 1024 UTF-8 bytes";

fn invalid(phrase: &str) -> MnemonicError {
    match WalletKeys::from_mnemonic(phrase) {
        Err(error) => error,
        Ok(_) => panic!("malformed recovery input must be refused"),
    }
}

#[test]
fn oversized_input_is_rejected_before_normalization() {
    let oversized = "A".repeat(1025);

    match invalid(&oversized) {
        MnemonicError::Invalid(reason) => {
            assert_eq!(reason, TOO_LONG);
        }
        other => panic!("input should be bounded before splitting: {other:?}"),
    }

    let phrase = vec!["Z".repeat(42); 24].join(" ");
    assert!(phrase.len() > 1024);

    match invalid(&phrase) {
        MnemonicError::Invalid(reason) => {
            assert_eq!(reason, TOO_LONG);
        }
        other => panic!("24-word input bypassed byte limit: {other:?}"),
    }
}

#[test]
fn below_limit_reaches_normal_validation() {
    let input = vec!["q".repeat(41); 24].join(" ");
    assert!(input.len() <= 1024);

    match invalid(&input) {
        MnemonicError::Invalid(reason) => {
            assert_eq!(reason, "invalid word or checksum");
        }
        other => panic!("unexpected classification: {other:?}"),
    }
}

#[test]
fn errors_never_reflect_secret_canaries() {
    let words: Vec<String> = WalletKeys::from_seed([23; 32])
        .to_mnemonic()
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    assert_eq!(words.len(), 24);

    for position in 0..24 {
        let mut altered = words.clone();
        altered[position] = CANARY.to_string();

        let err = invalid(&altered.join(" "));
        let text = format!("{err:?} {err}");

        assert!(!text.contains(CANARY));
        assert!(text.contains("invalid word or checksum"));
    }
}

#[test]
fn unicode_and_control_inputs_fail_closed() {
    let original = WalletKeys::from_seed([45; 32]).to_mnemonic();

    let tokens: Vec<&str> = original.split_whitespace().collect();

    for poison in ["\0", "🔒", "\u{200b}", "\u{202e}", "İ", CANARY] {
        let mut altered = tokens.clone();
        altered[11] = poison;

        let err = invalid(&altered.join(" "));
        let text = format!("{err:?} {err}");

        assert!(!text.contains(poison));
        assert!(text.contains("invalid word or checksum"));
    }
}

#[test]
fn valid_wallet_identity_never_changes() {
    for i in 0u8..=127 {
        let seed = [i; 32];

        let phrase = WalletKeys::from_seed(seed).to_mnemonic();

        assert!(phrase.len() < 1024);

        let recovered = WalletKeys::from_mnemonic(&phrase).expect("valid phrase");

        assert_eq!(recovered.seed, seed);

        let human_input = format!(" \n{}\t", phrase.to_uppercase().replace(' ', "  \t"));

        let restored = WalletKeys::from_mnemonic(&human_input).expect("case/whitespace");

        assert_eq!(restored.seed, seed);
    }
}

#[test]
fn normal_word_count_errors_are_preserved() {
    for count in [0usize, 1, 12, 23, 25] {
        let phrase = vec!["abandon"; count].join(" ");

        match invalid(&phrase) {
            MnemonicError::WrongLength(n) => {
                assert_eq!(n, count);
            }
            other => panic!("word-count {count} misclassified: {other:?}"),
        }
    }
}
