//! Shared outputs for atomic swaps — the NIGHT half, and nothing else.
//!
//! **Experimental. Not wired into any wallet. Do not use with real coins.**
//! See `docs/SWAP-SPEC-DRAFT.md` v0.2 §4.
//!
//! # What this is
//!
//! An atomic swap needs a NIGHT output that neither party can spend alone. On
//! this chain an output key is *not* freely chosen — it is
//!
//! ```text
//! Ko = B + offset·G        offset = derive_key_offset(H(r·A))
//! ```
//!
//! so a shared output needs a shared **address**, not merely a shared key, and
//! the on-chain spend secret is `s_a + s_b + offset` — never `s_a + s_b`. Draft
//! v0.1 of the specification stated the latter in one section and the former in
//! another; an implementation following the wrong one produces signatures that
//! silently fail to verify. That inconsistency is the reason this module exists
//! as tested code rather than as prose.
//!
//! # What this is not
//!
//! No Bitcoin side, no adaptor signatures, no timelocks, no protocol. Those
//! belong in later phases and one of them — the pre-signed NIGHT refund of
//! v0.1 — has already been withdrawn as broken. This module deliberately does
//! only the part that can be verified against the real ledger primitives, here,
//! now, with nothing at stake.
//!
//! # The one thing you must not remove
//!
//! [`SwapShare`] carries no proof that its holder knows the secret. Without a
//! discrete-log-equality proof exchanged and verified *before* anything is
//! locked, the sum is not a 2-of-2 at all: see
//! [`tests::a_rogue_key_lets_one_party_spend_alone`], which performs the theft.

use crate::commit::{generator_g, Commitment};
use crate::dleq::{self, DleqProof};
use crate::keys::{Address, ViewKey};
use crate::stealth::{
    derive_blind, derive_key_offset, scan_output, shared_secret, Output, OutputFeature,
};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// One party's half of a shared spend key.
///
/// The secret is drawn below `2^252` on purpose. The same scalar has to be
/// meaningful on secp256k1 for the cross-curve proof, and the two group orders
/// differ (`≈2^252` and `≈2^256`); a value above the smaller order is a
/// different integer on each curve. Sampling below `2^252` is safely under
/// both.
#[derive(Clone)]
pub struct SwapShare {
    secret: Scalar,
    public: RistrettoPoint,
}

impl SwapShare {
    pub fn generate() -> Self {
        loop {
            let mut b = [0u8; 32];
            OsRng.fill_bytes(&mut b);
            // Little-endian: byte 31 is bits 248..255. Clearing the high nibble
            // yields a uniform integer in [0, 2^252).
            //
            // That bound is a power of two, so masking is identical to
            // rejection sampling below 2^252 — same distribution, no wrap, no
            // bias. 2^252 is strictly less than the Ristretto order
            // ℓ = 2^252 + 27742317777372353535851937790883648493, so a
            // canonical decode does not reduce, and the same integer is a
            // valid secp256k1 scalar. The discarded slice [2^252, ℓ) is
            // ~2^128 values; statistical distance from uniform-in-F_ℓ is
            // 2^{-124}.
            //
            // Do not "fix" this to look like Ed25519 clamping (bit 254 set,
            // low 3 bits cleared). That would destroy uniformity and make
            // the two curves disagree on the integer.
            b[31] &= 0x0F;
            if let Some(share) = Self::from_bytes(b) {
                return share;
            }
        }
    }

    /// Rebuild a share from its 32-byte little-endian secret.
    ///
    /// The `< 2^252` bound is checked **again** here, not only in
    /// [`Self::generate`]. A value that was honest when sampled can be
    /// replaced on disk by one that is a valid Ristretto scalar and a
    /// *different* secp256k1 scalar; loading that would split the two
    /// curves and the adaptor would extract garbage. Fail closed.
    pub fn from_bytes(b: [u8; 32]) -> Option<Self> {
        if b[31] & 0xF0 != 0 {
            return None;
        }
        let secret = Option::<Scalar>::from(Scalar::from_canonical_bytes(b))?;
        if secret == Scalar::ZERO {
            return None;
        }
        Some(Self {
            public: generator_g() * secret,
            secret,
        })
    }

    /// Same as [`Self::from_bytes`], from an already-decoded scalar.
    /// Still re-checks the bound: `Scalar` is reduced mod ℓ, which is
    /// larger than `2^252`.
    pub fn from_secret(secret: Scalar) -> Option<Self> {
        Self::from_bytes(secret.to_bytes())
    }

    /// The half that goes to the counterparty. Never send [`Self::secret`].
    pub fn public(&self) -> RistrettoPoint {
        self.public
    }

    pub fn secret(&self) -> Scalar {
        self.secret
    }

    /// secp256k1 point `T = s · G_secp`. Same integer as [`Self::secret`].
    pub fn secp_bytes(&self) -> [u8; 33] {
        dleq::secp_point_bytes(&self.secret)
    }

    /// Build the message the counterparty sees, including a DLEQ proof.
    /// Expensive (252-bit proof). Call once per swap, then persist the offer.
    pub fn offer(&self) -> SwapOffer {
        let mut rng = OsRng;
        let (proof, s, t) = dleq::prove(&self.secret, &mut rng);
        debug_assert_eq!(s, self.public);
        SwapOffer {
            s: s.compress().to_bytes(),
            t: t.to_bytes().to_vec(),
            proof,
        }
    }
}

/// Public half of a swap share. Verify before treating it as a 2-of-2 key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwapOffer {
    pub s: [u8; 32],
    pub t: Vec<u8>,
    pub proof: DleqProof,
}

impl SwapOffer {
    /// Fail closed. A rogue `S_b = P − S_a` cannot produce a valid proof.
    pub fn verify(&self) -> bool {
        if self.t.len() != 33 {
            return false;
        }
        let mut t = [0u8; 33];
        t.copy_from_slice(&self.t);
        dleq::verify(&self.proof, &self.s, &t)
    }

    pub fn spend_point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.s).decompress()
    }
}

/// Everything both parties need to recognise and spend the locked output.
///
/// The scan secret is shared deliberately. It grants visibility, never spending
/// authority — and it is *required*: without it the counterparty can derive
/// neither `offset` nor the output's blinding factor, and so can neither find
/// nor spend the output even holding both key halves.
///
/// Consequence, which belongs on screen and not only in a doc comment: both
/// parties can see this output. A swap lock carries no privacy from the
/// counterparty.
#[derive(Clone)]
pub struct SharedLock {
    spend_public: RistrettoPoint,
    scan_secret: Scalar,
    scan_public: RistrettoPoint,
}

/// Why a proposed lock output was refused.
///
/// Distinct variants because the tests assert *which* check fired. A single
/// boolean would let a broken check hide behind a passing one.
#[derive(Debug, PartialEq, Eq)]
pub enum LockError {
    /// The ephemeral key is not a curve point.
    BadEphemeralKey,
    /// `Ko` is not `B_shared + offset·G`. The output is not spendable by the
    /// two halves — whoever built it kept control.
    NotOurOutput,
    /// The commitment does not open to the agreed value under the blinding
    /// factor derived from the shared secret.
    WrongAmount,
    /// Key and amount are right but the view tag is not, so a scanning wallet
    /// will skip this output. Not a theft; a lost output.
    BadViewTag,
    /// The output is labelled coinbase. A swap lock is a plain payment; a
    /// coinbase flag would impose a maturity delay the claim does not expect.
    NotPlain,
    /// The sender signature does not verify. Consensus will refuse this
    /// output, so it can never enter the UTXO set.
    BadSenderSig,
    /// `Ko` and the commitment check out from `t`, but the sealed payload
    /// does not open to the same value and blind. Spendable via
    /// [`SharedLock::lock_blind`] — not via [`scan_output`]. Spec v0.2 §8
    /// phase 2a says abort.
    BadPayload,
}

impl SharedLock {
    /// Combine the two halves. `scan_secret` must be freshly generated per swap
    /// and known to both parties.
    ///
    /// This does **not** check DLEQ proofs. Production code must call
    /// [`Self::from_verified_offers`]. Kept so the rogue-key test can still
    /// demonstrate the theft that DLEQ exists to stop.
    pub fn new(a: RistrettoPoint, b: RistrettoPoint, scan_secret: Scalar) -> Self {
        Self {
            spend_public: a + b,
            scan_secret,
            scan_public: generator_g() * scan_secret,
        }
    }

    /// Production constructor. Both offers must verify or this returns `None`.
    pub fn from_verified_offers(a: &SwapOffer, b: &SwapOffer, scan_secret: Scalar) -> Option<Self> {
        if !a.verify() || !b.verify() {
            return None;
        }
        let pa = a.spend_point()?;
        let pb = b.spend_point()?;
        Some(Self::new(pa, pb, scan_secret))
    }

    /// A fresh scan secret. One per swap: reusing it across swaps between the
    /// same pair makes both locks visible to the same key and links them.
    pub fn fresh_scan_secret() -> Scalar {
        let mut b = [0u8; 64];
        OsRng.fill_bytes(&mut b);
        Scalar::from_bytes_mod_order_wide(&b)
    }

    /// The address the paying party sends to.
    pub fn address(&self) -> Address {
        Address {
            scan_pk: self.scan_public.compress().to_bytes(),
            spend_pk: self.spend_public.compress().to_bytes(),
        }
    }

    /// Watch-only credential for this lock. Both parties hold it; that is
    /// the visibility leak the spec puts on screen.
    pub fn view_key(&self) -> ViewKey {
        ViewKey {
            scan_sk: self.scan_secret,
            spend_pk: self.spend_public.compress().to_bytes(),
        }
    }

    /// The shared secret `t` for a given output, from the scan side.
    fn shared_t(&self, ephemeral_pk: &[u8; 32]) -> Option<[u8; 32]> {
        let ke = CompressedRistretto(*ephemeral_pk).decompress()?;
        Some(shared_secret(&(ke * self.scan_secret)))
    }

    /// The secret that actually spends the locked output: `s_a + s_b + offset`.
    ///
    /// Returns `None` only if the ephemeral key is malformed. Both halves are
    /// required; see the tests.
    pub fn claim_secret(&self, a: &Scalar, b: &Scalar, ephemeral_pk: &[u8; 32]) -> Option<Scalar> {
        let t = self.shared_t(ephemeral_pk)?;
        Some(a + b + derive_key_offset(&t))
    }

    /// The blinding factor of the locked output, needed to build the kernel of
    /// the transaction that spends it.
    pub fn lock_blind(&self, ephemeral_pk: &[u8; 32]) -> Option<Scalar> {
        Some(derive_blind(&self.shared_t(ephemeral_pk)?))
    }

    /// The verification duty of the party who did **not** build the lock.
    ///
    /// Specification v0.2 §8 phase 2a. The paying party chooses `r` and builds
    /// the output, so nothing stops her constructing one the counterparty can
    /// never spend, or one that holds less than agreed. Releasing a swap secret
    /// before this passes is how the counterparty loses their side.
    ///
    /// Deliberately takes the output itself rather than a chain scan: a wrong
    /// view tag makes a scanner skip the output entirely, so a scan-based check
    /// can be steered by the very party it is meant to police.
    pub fn verify_lock(&self, output: &Output, expected_value: u64) -> Result<(), LockError> {
        let t = self
            .shared_t(&output.ephemeral_pk)
            .ok_or(LockError::BadEphemeralKey)?;

        let expected_ko = self.spend_public + generator_g() * derive_key_offset(&t);
        if output.output_pk != expected_ko.compress().to_bytes() {
            return Err(LockError::NotOurOutput);
        }

        if output.commit != Commitment::new(expected_value, &derive_blind(&t)) {
            return Err(LockError::WrongAmount);
        }

        if output.view_tag != crate::stealth::derive_view_tag_pub(&t) {
            return Err(LockError::BadViewTag);
        }

        // The three checks above are the spendability argument: if Ko and
        // the commitment match t, then `claim_secret` is the discrete log of
        // Ko and `lock_blind` is the kernel's b_in. Everything below is
        // "will this output actually confirm, and will a scanner find it".

        if output.features != OutputFeature::Plain {
            return Err(LockError::NotPlain);
        }
        if !output.verify_sender_sig() {
            return Err(LockError::BadSenderSig);
        }
        // Spec v0.2 §8 phase 2a: abort if the payload does not open. A lying
        // payload does *not* make the output unspendable — b_in comes from t,
        // not from the plaintext — but a wallet that claims via scan_output
        // would then fail to build the kernel. Fail closed here so that
        // path cannot be wired by accident.
        match scan_output(&self.view_key(), output) {
            Some(d) if d.value == expected_value => Ok(()),
            _ => Err(LockError::BadPayload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::WalletKeys;

    use crate::stealth::create_output;

    const CTX: &[u8] = b"nightfall:swap:test";
    const VALUE: u64 = 42_000_000_000;

    /// Alice pays the shared address; returns the output she published.
    fn lock(shared: &SharedLock, value: u64) -> Output {
        create_output(&shared.address(), value, "", CTX)
            .expect("build lock output")
            .0
    }

    fn setup() -> (SwapShare, SwapShare, SharedLock) {
        let a = SwapShare::generate();
        let b = SwapShare::generate();
        let shared = SharedLock::new(a.public(), b.public(), SharedLock::fresh_scan_secret());
        (a, b, shared)
    }

    /// The property the whole NIGHT side rests on: `s_a + s_b + offset` is the
    /// spend secret, and `s_a + s_b` is not.
    #[test]
    fn both_halves_together_spend_the_lock() {
        let (a, b, shared) = setup();
        let out = lock(&shared, VALUE);

        let claim = shared
            .claim_secret(&a.secret(), &b.secret(), &out.ephemeral_pk)
            .expect("claim secret");
        assert_eq!(
            (generator_g() * claim).compress().to_bytes(),
            out.output_pk,
            "s_a + s_b + offset must be the output key's discrete log"
        );

        // The formula draft v0.1 gave in its overview section.
        let without_offset = a.secret() + b.secret();
        assert_ne!(
            (generator_g() * without_offset).compress().to_bytes(),
            out.output_pk,
            "s_a + s_b alone must NOT verify — v0.1 said it would"
        );
    }

    #[test]
    fn neither_half_alone_spends_the_lock() {
        let (a, b, shared) = setup();
        let out = lock(&shared, VALUE);
        let zero = Scalar::ZERO;

        for (name, half) in [("alice", a.secret()), ("bob", b.secret())] {
            let solo = shared
                .claim_secret(&half, &zero, &out.ephemeral_pk)
                .expect("claim secret");
            assert_ne!(
                (generator_g() * solo).compress().to_bytes(),
                out.output_pk,
                "{name} must not be able to spend alone"
            );
        }
    }

    /// Direct check that `lock_blind` recovers the sender's `b_in`.
    ///
    /// The previous version of this test computed `t` twice from the scan
    /// secret and asserted they were equal — a tautology. This compares the
    /// scan-side derivation against the blinding factor `create_output`
    /// actually used for the commitment, which is the value the claim kernel
    /// needs. The ledger test in `nightfall-ledger` then sends that kernel
    /// through `apply_block`.
    #[test]
    fn lock_blind_matches_the_senders_blind() {
        let (_, _, shared) = setup();
        let (out, secrets) = create_output(&shared.address(), VALUE, "", CTX).unwrap();
        assert_eq!(
            shared.lock_blind(&out.ephemeral_pk).unwrap(),
            secrets.blind,
            "scan-side lock_blind must equal the sender's output blind"
        );
        assert_eq!(
            Commitment::new(VALUE, &secrets.blind),
            out.commit,
            "sender blind must open the published commitment"
        );
    }

    #[test]
    fn an_honest_lock_verifies() {
        let (_, _, shared) = setup();
        let out = lock(&shared, VALUE);
        assert_eq!(shared.verify_lock(&out, VALUE), Ok(()));
    }

    /// The attack phase 2a exists to stop: the paying party locks to an address
    /// only she controls, and hopes the counterparty releases anyway.
    #[test]
    fn a_lock_to_someone_elses_address_is_refused() {
        let (_, _, shared) = setup();
        let alice_only = WalletKeys::generate().address();
        let out = create_output(&alice_only, VALUE, "", CTX).unwrap().0;

        assert_eq!(
            shared.verify_lock(&out, VALUE),
            Err(LockError::NotOurOutput),
            "an output keyed to one party must be refused"
        );
    }

    #[test]
    fn a_lock_for_the_wrong_amount_is_refused() {
        let (_, _, shared) = setup();
        let out = lock(&shared, VALUE - 1);
        assert_eq!(shared.verify_lock(&out, VALUE), Err(LockError::WrongAmount));
    }

    #[test]
    fn a_broken_view_tag_is_reported_separately() {
        let (_, _, shared) = setup();
        let mut out = lock(&shared, VALUE);
        out.view_tag ^= 0xFF;
        // Key and amount are still right — this output is spendable, but a
        // scanning wallet would never find it.
        assert_eq!(shared.verify_lock(&out, VALUE), Err(LockError::BadViewTag));
    }

    #[test]
    fn a_tampered_payload_fails_the_sender_signature() {
        let (_, _, shared) = setup();
        let mut out = lock(&shared, VALUE);
        assert!(!out.payload.is_empty());
        out.payload[0] ^= 0xFF;
        assert_eq!(
            shared.verify_lock(&out, VALUE),
            Err(LockError::BadSenderSig)
        );
    }

    #[test]
    fn a_coinbase_flagged_lock_is_refused() {
        let (_, _, shared) = setup();
        let out = crate::stealth::create_output_with_features(
            &shared.address(),
            VALUE,
            "",
            CTX,
            OutputFeature::Coinbase,
        )
        .unwrap()
        .0;
        assert_eq!(shared.verify_lock(&out, VALUE), Err(LockError::NotPlain));
    }

    /// **This test performs a theft, on purpose.**
    ///
    /// `SwapShare` carries no proof of knowledge. A party who receives the
    /// other half first can answer with `S_b = P − S_a` for a `P` whose
    /// discrete log he knows, making the sum `P` and the lock his alone.
    ///
    /// The only thing standing between this construction and that theft is a
    /// discrete-log-equality proof, exchanged and verified before anything is
    /// locked. If this test ever starts failing, someone has changed the key
    /// combination — and if it keeps passing while the DLEQ check is missing
    /// from the protocol, the protocol is broken.
    #[test]
    fn a_rogue_key_lets_one_party_spend_alone() {
        let alice = SwapShare::generate();

        // Bob sees S_a and answers with P - S_a.
        let p = SwapShare::generate();
        let rogue_public = p.public() - alice.public();

        let shared = SharedLock::new(
            alice.public(),
            rogue_public,
            SharedLock::fresh_scan_secret(),
        );
        let out = lock(&shared, VALUE);

        // Bob spends with p + offset. Alice's half never enters the sum.
        let t = shared.shared_t(&out.ephemeral_pk).unwrap();
        let bob_alone = p.secret() + derive_key_offset(&t);
        assert_eq!(
            (generator_g() * bob_alone).compress().to_bytes(),
            out.output_pk,
            "without a DLEQ proof the sum is not a 2-of-2"
        );

        // And it still passes the lock check — verify_lock proves the output is
        // spendable by the combined key, not that both parties know a half.
        assert_eq!(
            shared.verify_lock(&out, VALUE),
            Ok(()),
            "verify_lock cannot detect this; only the DLEQ can"
        );
    }

    /// Reconstruction must refuse anything that would disagree on secp256k1.
    /// Pinned so a load path that skips the bound cannot ship.
    #[test]
    fn a_share_at_or_above_two_to_the_252_is_refused() {
        let mut too_big = [0u8; 32];
        too_big[31] = 0x10; // 2^252 — canonical on Ristretto, forbidden here
        assert!(
            Option::<Scalar>::from(Scalar::from_canonical_bytes(too_big)).is_some(),
            "precondition: 2^252 itself is a canonical Ristretto scalar"
        );
        assert!(
            SwapShare::from_bytes(too_big).is_none(),
            "loading 2^252 must fail, not wrap onto secp"
        );
        let as_scalar = Option::<Scalar>::from(Scalar::from_canonical_bytes(too_big)).unwrap();
        assert!(
            SwapShare::from_secret(as_scalar).is_none(),
            "from_secret must re-check, not trust the Scalar type"
        );
        assert!(
            SwapShare::from_bytes([0u8; 32]).is_none(),
            "zero is not a share"
        );

        let mut max_ok = [0xFF; 32];
        max_ok[31] = 0x0F; // 2^252 - 1
        let ok = SwapShare::from_bytes(max_ok).expect("2^252-1 is the largest honest share");
        assert_eq!(ok.secret().to_bytes(), max_ok);
        assert_eq!(
            SwapShare::from_secret(ok.secret()).unwrap().secret(),
            ok.secret()
        );
    }

    #[test]
    fn generate_round_trips_through_from_bytes() {
        for _ in 0..16 {
            let s = SwapShare::generate();
            let rebuilt = SwapShare::from_bytes(s.secret().to_bytes()).expect("honest share");
            assert_eq!(rebuilt.secret(), s.secret());
            assert_eq!(rebuilt.public(), s.public());
        }
    }

    /// Shares must stay below 2^252 so the same integer is meaningful on
    /// secp256k1 too.
    #[test]
    fn shares_are_small_enough_for_both_curves() {
        for _ in 0..64 {
            let s = SwapShare::generate();
            let b = s.secret().to_bytes();
            assert_eq!(b[31] & 0xF0, 0, "share exceeds 2^252");
            assert_ne!(s.secret(), Scalar::ZERO);
            // Canonical decode of the encoded secret must succeed: if
            // from_bytes_mod_order had wrapped, to_bytes() would still look
            // small after reduction only by accident.
            assert!(
                Option::<Scalar>::from(Scalar::from_canonical_bytes(b)).is_some(),
                "share is not a canonical scalar"
            );
        }
    }

    /// Masking to 2^252 never reduces modulo ℓ, so it cannot map two
    /// bit-patterns onto one scalar.
    #[test]
    fn two_to_the_252_minus_one_is_canonical() {
        let mut b = [0xFF; 32];
        b[31] = 0x0F;
        assert!(
            Option::<Scalar>::from(Scalar::from_canonical_bytes(b)).is_some(),
            "2^252 - 1 must be strictly below the Ristretto order, otherwise \
             the mask in SwapShare::generate wraps and the two curves disagree"
        );
        // And 2^252 itself — high nibble 0x10 — is what the mask forbids.
        // It is still below ℓ (ℓ = 2^252 + ~2^128), which is why the
        // discarded slice is a bias of size 2^{-124}, not a wrap.
        let mut c = [0u8; 32];
        c[31] = 0x10; // 2^252
        assert!(
            Option::<Scalar>::from(Scalar::from_canonical_bytes(c)).is_some(),
            "2^252 itself is still below ℓ; the mask is conservative"
        );
    }

    /// A fresh scan secret per swap. Reuse links the two locks for anyone
    /// holding it — which is both parties, by construction.
    #[test]
    fn scan_secrets_do_not_repeat() {
        let a = SharedLock::fresh_scan_secret();
        let b = SharedLock::fresh_scan_secret();
        assert_ne!(a, b);
    }

    /// B3: a verified offer is the only way into a shared lock.
    #[test]
    fn verified_offers_build_a_lock() {
        let a = SwapShare::generate();
        let b = SwapShare::generate();
        let oa = a.offer();
        let ob = b.offer();
        assert!(oa.verify());
        assert!(ob.verify());
        let shared = SharedLock::from_verified_offers(&oa, &ob, SharedLock::fresh_scan_secret())
            .expect("honest offers");
        assert_eq!(
            shared.address().spend_point().unwrap(),
            a.public() + b.public()
        );
    }

    /// B3: a rogue public without a matching DLEQ cannot enter the lock.
    #[test]
    fn a_rogue_offer_is_refused() {
        let alice = SwapShare::generate();
        let p = SwapShare::generate();
        let rogue = p.public() - alice.public();
        let mut fake = p.offer();
        fake.s = rogue.compress().to_bytes();
        assert!(!fake.verify(), "proof is bound to the proven point");
        assert!(SharedLock::from_verified_offers(
            &alice.offer(),
            &fake,
            SharedLock::fresh_scan_secret()
        )
        .is_none());
    }
}

/// Tests for the branch a well-formed but dishonest sender can reach.
///
/// Separated from the module above because they need to forge an output rather
/// than build one: the payload is re-sealed with a value the commitment does
/// not hold, and the sender signature is then re-made over the forgery. Only a
/// party who knows `r` can do this — which is exactly the paying party.
#[cfg(test)]
mod hostile_payload_tests {
    use super::*;

    use crate::stealth::{
        create_output, derive_aead_pub, derive_view_tag_pub, Output, OutputFeature,
    };
    use crate::{rangeproofs, schnorr};
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

    const CTX: &[u8] = b"nightfall:swap:test";
    const VALUE: u64 = 42_000_000_000;
    const MEMO_LEN: usize = 64;

    /// Build a lock output that is honest in every field the first three
    /// checks look at, and lies in the sealed payload.
    ///
    /// `Ko`, the commitment and the view tag all derive from the real `t`, so
    /// the output is genuinely spendable with `claim_secret` and `lock_blind`.
    /// The payload seals a different value. A counterparty who builds the claim
    /// from the payload instead of from `t` builds an unbalanced kernel and
    /// loses the swap.
    fn forge_lying_payload(shared: &SharedLock, real_value: u64, claimed_value: u64) -> Output {
        let addr = shared.address();
        let spend_point = addr.spend_point().unwrap();

        // A sender-chosen r, exactly as create_output does it.
        let mut rb = [0u8; 64];
        OsRng.fill_bytes(&mut rb);
        let r = Scalar::from_bytes_mod_order_wide(&rb);
        let ephemeral_pk = (generator_g() * r).compress().to_bytes();

        let t = shared_secret(&(addr.scan_point().unwrap() * r));
        let blind = derive_blind(&t);
        let offset = derive_key_offset(&t);
        let output_pk = (spend_point + generator_g() * offset).compress().to_bytes();

        // The commitment holds the REAL value.
        let (range_proof, commit) = rangeproofs::prove(real_value, &blind, CTX).unwrap();

        // The payload claims a different one.
        let mut plain = Vec::new();
        plain.extend_from_slice(&claimed_value.to_le_bytes());
        plain.extend_from_slice(&blind.to_bytes());
        plain.extend_from_slice(&[0u8; MEMO_LEN]);
        let (key, nonce) = derive_aead_pub(&t);
        let payload = XChaCha20Poly1305::new(Key::from_slice(&key))
            .encrypt(XNonce::from_slice(&nonce), plain.as_ref())
            .unwrap();

        let mut out = Output {
            features: OutputFeature::Plain,
            commit,
            range_proof,
            ephemeral_pk,
            output_pk,
            view_tag: derive_view_tag_pub(&t),
            payload,
            sender_sig: schnorr::SchnorrSig {
                r: [0u8; 32],
                s: [0u8; 32],
            },
        };
        // Re-sign over the forgery. The sender can always do this.
        out.sender_sig = schnorr::sign(&r, &generator_g(), &out.sig_message());
        out
    }

    fn setup() -> (SwapShare, SwapShare, SharedLock) {
        let a = SwapShare::generate();
        let b = SwapShare::generate();
        let shared = SharedLock::new(a.public(), b.public(), SharedLock::fresh_scan_secret());
        (a, b, shared)
    }

    /// The gap the review left open: a *signed* lying payload. The three
    /// spendability checks pass, the sender signature verifies, and only the
    /// payload check catches it.
    #[test]
    fn a_signed_lying_payload_is_refused() {
        let (_, _, shared) = setup();
        let out = forge_lying_payload(&shared, VALUE, VALUE * 2);

        // Everything the first three checks look at is honest.
        assert!(
            out.verify_sender_sig(),
            "the forgery must be properly signed"
        );
        assert_eq!(out.features, OutputFeature::Plain);

        assert_eq!(
            shared.verify_lock(&out, VALUE),
            Err(LockError::BadPayload),
            "a payload that disagrees with the commitment must abort the swap"
        );
    }

    /// And the point that makes this a swap bug rather than a chain bug: the
    /// forged output *is* spendable. The coins are not lost — the counterparty
    /// simply must not derive the kernel from the payload.
    #[test]
    fn the_lying_output_is_still_spendable_from_t() {
        let (a, b, shared) = setup();
        let out = forge_lying_payload(&shared, VALUE, VALUE * 2);

        let claim = shared
            .claim_secret(&a.secret(), &b.secret(), &out.ephemeral_pk)
            .unwrap();
        assert_eq!(
            (generator_g() * claim).compress().to_bytes(),
            out.output_pk,
            "the key path is unaffected by a lying payload"
        );

        let blind = shared.lock_blind(&out.ephemeral_pk).unwrap();
        assert_eq!(
            out.commit,
            Commitment::new(VALUE, &blind),
            "lock_blind opens the commitment to the real value, not the claimed one"
        );
    }

    /// An honest output must survive the same forge path, or the test above
    /// proves nothing about the check and everything about the forger.
    #[test]
    fn the_honest_control_still_passes() {
        let (_, _, shared) = setup();
        let honest = create_output(&shared.address(), VALUE, "", CTX).unwrap().0;
        assert_eq!(shared.verify_lock(&honest, VALUE), Ok(()));
    }
}

/// Which generator the swap's discrete-log-equality proof must use.
///
/// The attack report of 28 August left this open: `generator_g()` is
/// `bulletproofs::PedersenGens::default().B`, and whether that coincides with
/// the Ristretto basepoint decides whether an off-the-shelf cross-curve proof
/// can be adopted or has to be re-instantiated. Guessing either way would be
/// the same mistake as v0.1. These tests answer it.
#[cfg(test)]
mod generator_tests {
    use crate::commit::{generator_g, generator_h};
    use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
    use curve25519_dalek::ristretto::RistrettoPoint;

    /// Measured 29 Aug 2026: they are the same point,
    /// `e2f2ae0a…2d76`. A proof written against the Ristretto basepoint is
    /// therefore a proof about `S_x`, and the reference cross-curve
    /// construction carries over. Spec v0.2 said the opposite; it was an
    /// assumption and this test is why it is no longer in the document.
    ///
    /// If it ever fails, the cross-curve proof has to be re-instantiated and
    /// every borrowed vector is void.
    #[test]
    fn night_g_is_or_is_not_the_ristretto_basepoint() {
        let g = generator_g();
        let same = g == RISTRETTO_BASEPOINT_POINT;
        // Not an assertion — a recorded measurement. The value decides the
        // shape of task B2, so it is printed and pinned rather than assumed.
        println!(
            "generator_g() == RISTRETTO_BASEPOINT_POINT: {same}\n  \
             generator_g()  = {}\n  basepoint      = {}",
            hex(&g),
            hex(&RISTRETTO_BASEPOINT_POINT)
        );
        assert!(
            same,
            "generator_g() is NOT the Ristretto basepoint — the cross-curve \
             DLEQ must be re-instantiated on it, and xmr-btc-swap's test \
             vectors do not apply. See docs/SWAP-SPEC-DRAFT.md v0.2 §5."
        );
    }

    /// The blinding generator must *not* be the basepoint, or commitments
    /// would be openable. Guards the pair, not just one side.
    #[test]
    fn night_h_is_independent_of_the_basepoint() {
        assert_ne!(generator_h(), RISTRETTO_BASEPOINT_POINT);
        assert_ne!(generator_h(), generator_g());
    }

    fn hex(p: &RistrettoPoint) -> String {
        p.compress()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
