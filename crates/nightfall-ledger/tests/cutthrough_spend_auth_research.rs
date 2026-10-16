//! Research-only prototype for cut-through-compatible one-sided spend auth.
//!
//! IMPORTANT:
//! - not consensus active
//! - not used by Transaction/Input validation
//! - does not change v8
//!
//! This models the core LIP-0004 ownership and stealth-balance algebra using
//! Nightfall's existing Ristretto primitives and real Nightfall one-time output
//! keys (Ko).

use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
};
use nightfall_crypto::{
    blind_from_bytes, create_output, generator_g, scan_output, sig, WalletKeys,
};
use nightfall_ledger::Transaction;
use nightfall_types::NetworkId;
use rand::rngs::OsRng;

const INPUT_CHALLENGE_DOMAIN: &[u8] = b"nightfall:research:cutthrough:input-challenge:v1";

fn input_challenge(ki: &[u8; 32], ko: &[u8; 32]) -> Scalar {
    let mut material = [0u8; 64];
    material[..32].copy_from_slice(ki);
    material[32..].copy_from_slice(ko);

    blind_from_bytes(INPUT_CHALLENGE_DOMAIN, &material)
}

/// Prototype equivalent of:
///
///     K_input = Ki + H(Ki || Ko) * Ko
///
/// `Ko` is intentionally supplied from the canonical output being spent.
fn input_verification_key(ki: &[u8; 32], ko: &[u8; 32]) -> Option<RistrettoPoint> {
    let ki_point = CompressedRistretto(*ki).decompress()?;
    let ko_point = CompressedRistretto(*ko).decompress()?;
    let h = input_challenge(ki, ko);

    Some(ki_point + ko_point * h)
}

fn input_signing_secret(
    ki_secret: Scalar,
    ko_secret: Scalar,
    ki: &[u8; 32],
    ko: &[u8; 32],
) -> Scalar {
    ki_secret + input_challenge(ki, ko) * ko_secret
}

fn fresh_scalar() -> Scalar {
    Scalar::random(&mut OsRng)
}

struct OwnedKo {
    commit: nightfall_crypto::Commitment,
    ko: [u8; 32],
    ko_secret: Scalar,
    sender_known_blind: Scalar,
}

fn make_owned_output(receiver: &WalletKeys) -> OwnedKo {
    let ctx = NetworkId::Devnet.proof_context();

    let (output, sender_secrets) =
        create_output(&receiver.address(), 50_000, "", ctx).expect("create output");

    let discovered =
        scan_output(&receiver.view_key(), &output).expect("receiver must detect output");

    let ko_secret = discovered.spend_secret(receiver);

    assert_eq!(
        (generator_g() * ko_secret).compress().to_bytes(),
        output.output_pk,
        "recovered spend secret must open canonical Ko"
    );

    OwnedKo {
        commit: output.commit,
        ko: output.output_pk,
        ko_secret,
        sender_known_blind: sender_secrets.blind,
    }
}

#[test]
fn legitimate_receiver_can_construct_lip4_style_input_authorization() {
    let receiver = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let ki_secret = fresh_scalar();
    let ki = (generator_g() * ki_secret).compress().to_bytes();

    let signing_secret = input_signing_secret(ki_secret, owned.ko_secret, &ki, &owned.ko);

    let verification_key = input_verification_key(&ki, &owned.ko).expect("valid points");

    assert_eq!(
        generator_g() * signing_secret,
        verification_key,
        "private and public forms of K_input must agree"
    );

    let msg = Transaction::input_message(&owned.commit);
    let signature = sig::sign(&signing_secret, &generator_g(), &msg);

    assert!(
        sig::verify(&verification_key, &generator_g(), &msg, &signature),
        "legitimate receiver authorization must verify"
    );
}

#[test]
fn sender_known_output_blind_cannot_replace_receiver_spend_secret() {
    let receiver = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let ki_secret = fresh_scalar();
    let ki = (generator_g() * ki_secret).compress().to_bytes();

    let challenge = input_challenge(&ki, &owned.ko);

    // The output creator knows the commitment blind in Nightfall today.
    // Treating that knowledge as if it were the receiver's Ko secret must fail.
    let forged_secret = ki_secret + challenge * owned.sender_known_blind;

    let verification_key = input_verification_key(&ki, &owned.ko).expect("valid points");

    let msg = Transaction::input_message(&owned.commit);
    let forged_signature = sig::sign(&forged_secret, &generator_g(), &msg);

    assert!(
        !sig::verify(&verification_key, &generator_g(), &msg, &forged_signature),
        "sender-known commitment blind must not authorize receiver's Ko"
    );
}

#[test]
fn unrelated_wallet_cannot_spend_receiver_output() {
    let receiver = WalletKeys::generate();
    let attacker = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let ki_secret = fresh_scalar();
    let ki = (generator_g() * ki_secret).compress().to_bytes();
    let challenge = input_challenge(&ki, &owned.ko);

    let attacker_guess = ki_secret + challenge * attacker.spend_secret();

    let verification_key = input_verification_key(&ki, &owned.ko).expect("valid points");

    let msg = Transaction::input_message(&owned.commit);
    let forged = sig::sign(&attacker_guess, &generator_g(), &msg);

    assert!(
        !sig::verify(&verification_key, &generator_g(), &msg, &forged),
        "unrelated wallet must not authorize canonical Ko"
    );
}

#[test]
fn authorization_is_bound_to_ki_ko_and_commitment() {
    let receiver = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let ki_secret = fresh_scalar();
    let ki = (generator_g() * ki_secret).compress().to_bytes();

    let signing_secret = input_signing_secret(ki_secret, owned.ko_secret, &ki, &owned.ko);

    let key = input_verification_key(&ki, &owned.ko).expect("valid key");

    let msg = Transaction::input_message(&owned.commit);
    let signature = sig::sign(&signing_secret, &generator_g(), &msg);

    assert!(sig::verify(&key, &generator_g(), &msg, &signature));

    // Substitute Ki.
    let replacement_ki_secret = fresh_scalar();
    let replacement_ki = (generator_g() * replacement_ki_secret)
        .compress()
        .to_bytes();

    let replacement_key =
        input_verification_key(&replacement_ki, &owned.ko).expect("valid replacement Ki");

    assert!(
        !sig::verify(&replacement_key, &generator_g(), &msg, &signature),
        "changing Ki must invalidate authorization"
    );

    // Substitute canonical Ko.
    let replacement_ko_secret = fresh_scalar();
    let replacement_ko = (generator_g() * replacement_ko_secret)
        .compress()
        .to_bytes();

    let replacement_key =
        input_verification_key(&ki, &replacement_ko).expect("valid replacement Ko");

    assert!(
        !sig::verify(&replacement_key, &generator_g(), &msg, &signature),
        "changing Ko must invalidate authorization"
    );

    // Substitute commitment.
    let other_receiver = WalletKeys::generate();
    let other = make_owned_output(&other_receiver);
    let other_msg = Transaction::input_message(&other.commit);

    assert!(
        !sig::verify(&key, &generator_g(), &other_msg, &signature),
        "changing spent commitment must invalidate authorization"
    );
}

#[derive(Clone)]
struct StealthTerm {
    ks: Scalar,
    ki: Scalar,
    ko: Scalar,
    excess: Scalar,
    offset: Scalar,
}

impl StealthTerm {
    fn new(ko: Scalar) -> Self {
        let ks = fresh_scalar();
        let ki = fresh_scalar();
        let excess = fresh_scalar();

        // LIP-0004-style scalar form:
        //
        // Ks + Ki - Ko = E' + x'G
        //
        // therefore:
        //
        // x' = ks + ki - ko - e'
        let offset = ks + ki - ko - excess;

        Self {
            ks,
            ki,
            ko,
            excess,
            offset,
        }
    }

    fn lhs(&self) -> RistrettoPoint {
        generator_g() * self.ks + generator_g() * self.ki - generator_g() * self.ko
    }

    fn rhs(&self) -> RistrettoPoint {
        generator_g() * self.excess + generator_g() * self.offset
    }
}

#[test]
fn stealth_balance_holds_for_real_nightfall_ko() {
    let receiver = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let term = StealthTerm::new(owned.ko_secret);

    assert_eq!(
        term.lhs(),
        term.rhs(),
        "stealth authorization equation must balance exactly"
    );
}

#[test]
fn stealth_balances_compose_under_aggregation() {
    let alice = WalletKeys::generate();
    let bob = WalletKeys::generate();

    let a = StealthTerm::new(make_owned_output(&alice).ko_secret);
    let b = StealthTerm::new(make_owned_output(&bob).ko_secret);

    assert_eq!(a.lhs(), a.rhs());
    assert_eq!(b.lhs(), b.rhs());

    let aggregated_lhs = a.lhs() + b.lhs();
    let aggregated_rhs = a.rhs() + b.rhs();

    assert_eq!(
        aggregated_lhs, aggregated_rhs,
        "independent stealth balances must aggregate additively"
    );

    // Aggregation ordering must carry no semantic information.
    assert_eq!(a.lhs() + b.lhs(), b.lhs() + a.lhs());
    assert_eq!(a.rhs() + b.rhs(), b.rhs() + a.rhs());
}

#[test]
fn stealth_balance_breaks_if_canonical_ko_is_substituted() {
    let receiver = WalletKeys::generate();
    let owned = make_owned_output(&receiver);

    let term = StealthTerm::new(owned.ko_secret);

    let forged_ko = fresh_scalar();

    let forged_lhs = generator_g() * term.ks + generator_g() * term.ki - generator_g() * forged_ko;

    assert_ne!(
        forged_lhs,
        term.rhs(),
        "replacing canonical Ko must break stealth balance"
    );
}
