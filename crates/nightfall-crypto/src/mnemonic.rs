//! Seed backup as BIP-39 words.
//!
//! # Why this exists
//!
//! Until now a wallet was backed up by copying a hex file. That works on a
//! desktop and does not work anywhere else: there is no file to copy on a
//! phone, and a user cannot read 64 hex characters off a screen onto paper
//! without making a mistake they will not discover until the day it matters.
//!
//! # Why entropy and not the BIP-39 seed
//!
//! BIP-39 defines two different byte strings and conflating them loses money.
//!
//! - **Entropy** — the bytes the words encode. 24 words carry 256 bits plus an
//!   8-bit checksum. The mapping is bijective: entropy → words → entropy
//!   returns exactly what went in.
//! - **Seed** — `PBKDF2-HMAC-SHA512(words, "mnemonic" ‖ passphrase)`, 64 bytes,
//!   one-way. This is what BIP-32 hierarchies are built from.
//!
//! Nightfall's [`WalletKeys`] takes 32 bytes and derives everything from them.
//! So the wallet seed *is* the BIP-39 entropy, and the round-trip is exact.
//! Taking the first 32 bytes of the PBKDF2 output would work equally well
//! going forward and would be impossible to reverse — which matters, because
//! existing wallets have a seed already and must be able to produce words for
//! it. A backup scheme that only works for wallets created after it shipped is
//! not a backup scheme.
//!
//! The cost is that BIP-39 passphrases ("25th word") are not supported. If that
//! is ever wanted it belongs in a deliberate second layer, not smuggled in by
//! switching which byte string we call the seed.
//!
//! # What this is not
//!
//! These words are **not** compatible with a Bitcoin or Ethereum wallet. The
//! encoding is standard BIP-39, but Nightfall derives its keys differently, so
//! entering this phrase elsewhere finds nothing. The reverse is also true and
//! more dangerous: a phrase from another wallet will import here without
//! complaint and produce a valid, empty, unrelated Nightfall wallet.

use crate::WalletKeys;
use bip39::Mnemonic;
use thiserror::Error;
use zeroize::Zeroize;

/// A 256-bit seed is 24 words under BIP-39.
pub const MNEMONIC_WORDS: usize = 24;

#[derive(Debug, Error)]
pub enum MnemonicError {
    /// Covers unknown words, a bad checksum, and unsupported lengths. The
    /// checksum is the useful part: it catches a mistyped or transposed word
    /// before the user concludes their coins are gone.
    #[error("invalid recovery phrase: {0}")]
    Invalid(String),

    /// A phrase of a valid BIP-39 length that is not 24 words. Accepting one
    /// would silently produce a wallet with less entropy than the format
    /// promises.
    #[error("expected {MNEMONIC_WORDS} words, got {0}")]
    WrongLength(usize),
}

impl WalletKeys {
    /// Render this wallet's seed as a BIP-39 recovery phrase.
    ///
    /// Treat the result as secret: anyone holding these words holds the funds.
    pub fn to_mnemonic(&self) -> String {
        // 32 bytes of entropy is always a valid BIP-39 input, so this cannot
        // fail; the expect documents that rather than hiding a panic.
        Mnemonic::from_entropy(&self.seed)
            .expect("32 bytes is a valid BIP-39 entropy length")
            .to_string()
    }

    /// Recover a wallet from a BIP-39 recovery phrase.
    ///
    /// Whitespace and case are normalised, because a user copying words off
    /// paper produces neither consistently.
    pub fn from_mnemonic(phrase: &str) -> Result<Self, MnemonicError> {
        let normalised = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
        let normalised = normalised.to_lowercase();

        let count = normalised.split_whitespace().count();
        if count != MNEMONIC_WORDS {
            return Err(MnemonicError::WrongLength(count));
        }

        let mnemonic = Mnemonic::parse_normalized(&normalised)
            .map_err(|e| MnemonicError::Invalid(e.to_string()))?;

        let (entropy, len) = mnemonic.to_entropy_array();
        if len != 32 {
            return Err(MnemonicError::Invalid(format!(
                "phrase carries {len} bytes of entropy, expected 32"
            )));
        }

        let mut seed = [0u8; 32];
        seed.copy_from_slice(&entropy[..32]);
        let keys = WalletKeys::from_seed(seed);
        seed.zeroize();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_exactly() {
        for _ in 0..32 {
            let keys = WalletKeys::generate();
            let phrase = keys.to_mnemonic();
            assert_eq!(phrase.split_whitespace().count(), MNEMONIC_WORDS);

            let restored = WalletKeys::from_mnemonic(&phrase).unwrap();
            assert_eq!(restored.seed, keys.seed, "seed must survive the round trip");
            assert_eq!(
                restored.address().encode(),
                keys.address().encode(),
                "same seed must yield the same address"
            );
        }
    }

    #[test]
    fn tolerates_how_people_actually_type() {
        let keys = WalletKeys::generate();
        let phrase = keys.to_mnemonic();

        let messy = format!("  {}  ", phrase.replace(' ', "   ").to_uppercase());
        assert_eq!(WalletKeys::from_mnemonic(&messy).unwrap().seed, keys.seed);
    }

    /// A test vector, not a wallet. Generated for this test and never funded.
    const TRANSPOSE_VECTOR: &str = "cereal hedgehog hotel assist warrior learn \
        bamboo expect heart fetch work animal aim must raise wool diet enemy \
        climb drift share valve fringe prison";

    #[test]
    fn rejects_a_transposed_word() {
        // The whole point of the checksum. A user who swaps two words must be
        // told, not handed an empty wallet and left to conclude the coins are
        // gone.
        //
        // Deliberately a fixed vector rather than `generate()`. The checksum
        // is 8 bits, so a transposition of a *random* phrase still validates
        // roughly once in 256 runs — this test failed in a full workspace run
        // and passed on its own, which is the behaviour that teaches people
        // to re-run a red build instead of reading it. A flaky test is worse
        // than no test: it spends the credibility of every other one.
        let words: Vec<&str> = TRANSPOSE_VECTOR.split_whitespace().collect();
        assert_eq!(words.len(), 24, "the vector must be a 24-word phrase");
        assert!(
            WalletKeys::from_mnemonic(&words.join(" ")).is_ok(),
            "precondition: the vector itself is a valid phrase"
        );

        let mut swapped = words.clone();
        swapped.swap(0, 1);
        assert_ne!(swapped[0], swapped[1], "the swap must change something");
        assert!(
            WalletKeys::from_mnemonic(&swapped.join(" ")).is_err(),
            "a transposed phrase must be refused, not silently accepted"
        );
    }

    /// The property the fixed vector cannot show on its own: the checksum
    /// catches transpositions in general, not just that one.
    ///
    /// Stated as a rate rather than an absolute, because it genuinely is one
    /// — 8 checksum bits mean roughly 1 in 256 transpositions slips through,
    /// and a test claiming otherwise would be claiming something false.
    #[test]
    fn transpositions_are_caught_almost_always() {
        let mut tried = 0usize;
        let mut caught = 0usize;
        for _ in 0..300 {
            let keys = WalletKeys::generate();
            let phrase = keys.to_mnemonic();
            let words: Vec<&str> = phrase.split(' ').collect();
            if words[0] == words[1] {
                continue;
            }
            let mut swapped = words.clone();
            swapped.swap(0, 1);
            tried += 1;
            if WalletKeys::from_mnemonic(&swapped.join(" ")).is_err() {
                caught += 1;
            }
        }
        assert!(tried > 250, "expected most draws to be usable, got {tried}");
        // 1/256 slip-through means ~99.6%. A floor of 95% fails loudly if the
        // checksum stops being checked, and never fails for being unlucky.
        let rate = caught as f64 / tried as f64;
        assert!(
            rate > 0.95,
            "the checksum caught only {caught} of {tried} transpositions ({rate:.3})"
        );
    }

    #[test]
    fn rejects_a_word_outside_the_list() {
        let keys = WalletKeys::generate();
        let phrase = keys.to_mnemonic();
        let mut words: Vec<&str> = phrase.split(' ').collect();
        words[5] = "nightfall";
        assert!(WalletKeys::from_mnemonic(&words.join(" ")).is_err());
    }

    #[test]
    fn rejects_short_phrases() {
        // 12 words is valid BIP-39 and only 128 bits. Silently accepting it
        // would halve the security of a wallet whose owner believes it is
        // 256-bit.
        let twelve = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";
        assert!(matches!(
            WalletKeys::from_mnemonic(twelve),
            Err(MnemonicError::WrongLength(12))
        ));
    }

    #[test]
    fn known_vector() {
        // The all-zero entropy vector from the BIP-39 specification. Locks the
        // wordlist and the entropy convention: if a future change switched to
        // the PBKDF2 seed, or to a different language list, this fails.
        let keys = WalletKeys::from_seed([0u8; 32]);
        assert_eq!(
            keys.to_mnemonic(),
            "abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon abandon abandon art"
        );
    }
}
