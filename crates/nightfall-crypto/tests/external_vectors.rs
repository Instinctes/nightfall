//! External conformance vectors for the primitives Nightfall builds on.
//!
//! # What this file is
//!
//! A set of tests that pin each cryptographic primitive `nightfall-crypto`
//! depends on against vectors published by that primitive's own
//! specification. Every test cites its source in a doc comment. Nothing
//! here is invented; if a test fails, either the upstream crate has a
//! regression or the citation is wrong, and both are worth knowing.
//!
//! # What this file is not
//!
//! It is not an audit. It does not prove that Nightfall's *use* of these
//! primitives is correct, only that the primitives themselves are. The
//! internal review in `docs/AUDIT-2026-08-16.md` is explicit (finding
//! I-01) that no internal document can substitute for an outside review;
//! this file does not attempt to. It is one of the inputs an external
//! auditor needs, named in `docs/AUDIT-READINESS.md` §6 item 4.
//!
//! # Why it exists
//!
//! `CONTRIBUTING.md` names *"Independent review of the cryptography"* as
//! "the most valuable contribution anyone could make right now." The
//! same document asks for tests written from an attacker's point of view,
//! not from the honest builder's. External vectors are the smallest
//! concrete step in that direction that does not require new APIs or a
//! consensus change.
//!
//! # Coverage
//!
//! Covered, with source:
//!
//! - BIP-39 — Trezor reference implementation `vectors.json`
//! - BLAKE3 — `BLAKE3-team/BLAKE3` `test_vectors.json`
//! - Ed25519 — RFC 8032 §7.1, TEST 1
//! - X25519  — RFC 7748 §6.1
//! - XChaCha20-Poly1305 — draft-irtf-cfrg-xchacha-03 §A.1
//!
//! Not covered here, and why:
//!
//! - **Nightfall's Schnorr-over-Ristretto** has no external vector to
//!   compare against: the construction is generator-parameterised and
//!   the second generator H is derived from a Nightfall domain string.
//!   Property-based tests for this live in a separate PR.
//! - **Bulletproofs in Nightfall's configuration** — the `bulletproofs`
//!   crate has its own internal tests; the parameter set here (RANGE_BITS,
//!   generator choice) is Nightfall-specific.
//! - **Argon2id at the primitive level** — RFC 9106 §5.3 uses `Secret[8]`
//!   and `Associated data[12]`, which the stable `argon2 0.5` public API
//!   does not expose. Nightfall's `nighthash` wrapper has its own
//!   coverage in `pow.rs`; a follow-up can add the RFC vector to the
//!   same file once the crate exposes the low-level entry point.
//!
//! # Citation rule
//!
//! Every `assert_eq!` on a constant below must be traceable to the source
//! cited at the top of its section. If a constant needs changing, the
//! citation changes with it, in the same commit.

use nightfall_crypto::{domain, hash_multi, WalletKeys};

// ---------------------------------------------------------------------------
// BIP-39
// ---------------------------------------------------------------------------
//
// Source: https://github.com/trezor/python-mnemonic/blob/master/vectors.json
//         (English section, "entropy → mnemonic" direction)
//
// Trezor's vector file is the reference implementation maintained by the
// Trezor team. It is what any BIP-39 implementation is measured against.
//
// Only the entropy → mnemonic direction is tested here, because Nightfall
// uses BIP-39 *entropy* as the wallet seed. See `mnemonic.rs` for why the
// PBKDF2 seed and the BIP-39 passphrase are deliberately not used.

/// Trezor vector for `7f 7f … 7f` (32 bytes of 0x7f).
#[test]
fn bip39_trezor_entropy_7f() {
    let keys = WalletKeys::from_seed([0x7fu8; 32]);
    assert_eq!(
        keys.to_mnemonic(),
        "legal winner thank year wave sausage worth useful legal winner \
         thank year wave sausage worth useful legal winner thank year \
         wave sausage worth title"
    );
}

/// Trezor vector for `80 80 … 80` (32 bytes of 0x80).
#[test]
fn bip39_trezor_entropy_80() {
    let keys = WalletKeys::from_seed([0x80u8; 32]);
    assert_eq!(
        keys.to_mnemonic(),
        "letter advice cage absurd amount doctor acoustic avoid letter \
         advice cage absurd amount doctor acoustic avoid letter advice \
         cage absurd amount doctor acoustic bless"
    );
}

/// Trezor vector for `ff ff … ff` (32 bytes of 0xff).
#[test]
fn bip39_trezor_entropy_ff() {
    let keys = WalletKeys::from_seed([0xffu8; 32]);
    assert_eq!(
        keys.to_mnemonic(),
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo \
         zoo zoo zoo zoo zoo zoo zoo zoo vote"
    );
}

/// Round-trip direction: the phrase recovered above must produce a wallet
/// whose address matches the wallet the entropy would have produced
/// directly. `WalletKeys` hides the seed, so we compare by address.
#[test]
fn bip39_phrase_recovers_the_same_wallet() {
    let original = WalletKeys::from_seed([0x7fu8; 32]);
    let phrase = original.to_mnemonic();
    let restored = WalletKeys::from_mnemonic(&phrase).expect("vector must parse");
    assert_eq!(
        restored.address().encode(),
        original.address().encode(),
        "same entropy must yield the same address"
    );
}

// ---------------------------------------------------------------------------
// BLAKE3
// ---------------------------------------------------------------------------
//
// Source: https://github.com/BLAKE3-team/BLAKE3/blob/master/test_vectors/test_vectors.json
//
// The official BLAKE3 test vector file. Field `input_len` is the number
// of zero bytes fed in; the first `hash` entry is the digest of those
// zero bytes. Nightfall uses BLAKE3 through `hash_multi`, but testing
// the primitive directly first rules out an upstream regression before
// any Nightfall-specific encoding is involved.

#[test]
fn blake3_official_zero_input_0_bytes() {
    let digest = blake3::hash(b"");
    assert_eq!(
        hex::encode(digest.as_bytes()),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
}

#[test]
fn blake3_official_zero_input_1_byte() {
    let digest = blake3::hash(&[0u8]);
    assert_eq!(
        hex::encode(digest.as_bytes()),
        "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"
    );
}

// ---------------------------------------------------------------------------
// Ed25519
// ---------------------------------------------------------------------------
//
// Source: RFC 8032 §7.1, "TEST 1" (empty message)
//
// Secret key, public key, and signature are the three constants RFC 8032
// publishes for this input. Nightfall does not currently use Ed25519 in
// consensus, but the crate is a dependency and this pins its behaviour
// for any future use.

#[test]
fn ed25519_rfc8032_test_1() {
    use ed25519_dalek::{Signer, SigningKey};

    let sk_hex = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    let pk_hex = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    let sig_hex = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
         5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";

    let mut sk_bytes = [0u8; 32];
    sk_bytes.copy_from_slice(&hex::decode(sk_hex).unwrap());
    let signing_key = SigningKey::from_bytes(&sk_bytes);

    assert_eq!(hex::encode(signing_key.verifying_key().as_bytes()), pk_hex);

    let signature = signing_key.sign(b"");
    assert_eq!(hex::encode(signature.to_bytes()), sig_hex);
}

// ---------------------------------------------------------------------------
// X25519
// ---------------------------------------------------------------------------
//
// Source: RFC 7748 §6.1
//
// Alice's private/public key pair, Bob's private/public key pair, and
// the shared secret both parties compute. Nightfall's stealth output
// uses X25519 for the ephemeral sender key, so this pins the Diffie-
// Hellman path.

#[test]
fn x25519_rfc7748_6_1() {
    use x25519_dalek::{PublicKey, StaticSecret};

    let mut a_bytes = [0u8; 32];
    a_bytes.copy_from_slice(
        &hex::decode("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a").unwrap(),
    );
    let alice_secret = StaticSecret::from(a_bytes);
    let alice_public = PublicKey::from(&alice_secret);

    assert_eq!(
        hex::encode(alice_public.as_bytes()),
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    );

    let mut b_bytes = [0u8; 32];
    b_bytes.copy_from_slice(
        &hex::decode("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb").unwrap(),
    );
    let bob_secret = StaticSecret::from(b_bytes);
    let bob_public = PublicKey::from(&bob_secret);

    assert_eq!(
        hex::encode(bob_public.as_bytes()),
        "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
    );

    let alice_shared = alice_secret.diffie_hellman(&bob_public);
    let bob_shared = bob_secret.diffie_hellman(&alice_public);

    assert_eq!(
        alice_shared.as_bytes(),
        bob_shared.as_bytes(),
        "both sides must reach the same shared secret"
    );
    assert_eq!(
        hex::encode(alice_shared.as_bytes()),
        "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
    );
}

// ---------------------------------------------------------------------------
// XChaCha20-Poly1305 (AEAD)
// ---------------------------------------------------------------------------
//
// Source: draft-irtf-cfrg-xchacha-03 §A.1
//         https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
//
// The complete vector: key, nonce (24 bytes), AAD, plaintext, expected
// ciphertext, expected Poly1305 tag. Nightfall uses this construction
// in `stealth.rs` to encrypt the value and memo of every output.

#[test]
fn xchacha20_poly1305_draft_a1() {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

    let key_bytes =
        hex::decode("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f").unwrap();
    let nonce_bytes = hex::decode("404142434445464748494a4b4c4d4e4f5051525354555657").unwrap();
    let aad = hex::decode("50515253c0c1c2c3c4c5c6c7").unwrap();
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

    let expected_ct = hex::decode(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb\
         731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452\
         2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff\
         921f9664c97637da9768812f615c68b13b52e",
    )
    .unwrap();
    let expected_tag = hex::decode("c0875924c1c7987947deafd8780acf49").unwrap();

    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let nonce = XNonce::from_slice(&nonce_bytes);

    let produced = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .expect("encryption must succeed");

    let mut expected_full = expected_ct.clone();
    expected_full.extend_from_slice(&expected_tag);
    assert_eq!(
        produced, expected_full,
        "ciphertext and tag must match draft A.1"
    );

    let decrypted = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &expected_full,
                aad: &aad,
            },
        )
        .expect("decryption must succeed");
    assert_eq!(decrypted, plaintext);
}

// ---------------------------------------------------------------------------
// Nightfall's own encoding on top of BLAKE3
// ---------------------------------------------------------------------------
//
// This section does not test an external vector — no external vector
// exists for a Nightfall-specific encoding. It pins the property that
// `CONTRIBUTING.md` names explicitly:
//
//   "The length-prefixed encoding is what stops two different inputs
//    colliding by concatenation — plain blake3(a || b) is not equivalent
//    and will eventually bite."
//
// If the encoding in `hash_multi` ever changes silently, the first
// assert below stays true (it is a property, not a value) but the
// property itself is the reason the encoding exists. The second assert
// documents that domain separation, likewise, is not accidental.

#[test]
fn hash_multi_length_prefixes_are_injective() {
    // Without length prefixes these two would hash identically. The
    // existing unit test in `lib.rs` asserts this; the point of repeating
    // it here is that this file is the one a reviewer reads first, and
    // this is the property the encoding exists to provide.
    assert_ne!(
        hash_multi(domain::TX, &[b"ab", b"c"]),
        hash_multi(domain::TX, &[b"a", b"bc"]),
        "length-prefixed encoding must distinguish these two inputs"
    );
    assert_ne!(
        hash_multi(domain::TX, &[b"abc"]),
        hash_multi(domain::TX, &[b"ab", b"c"]),
        "one part vs. two parts must hash differently"
    );
}

#[test]
fn hash_multi_domain_separation_is_not_accidental() {
    assert_ne!(
        hash_multi(domain::TX, &[b"payload"]),
        hash_multi(domain::BLOCK, &[b"payload"]),
        "the same bytes under different domains must hash differently"
    );
}
