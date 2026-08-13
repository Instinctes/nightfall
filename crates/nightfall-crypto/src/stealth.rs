//! One-sided (non-interactive) stealth outputs.
//!
//! Plain Mimblewimble needs sender and receiver online at the same time. That
//! is unusable for the "paste a payment ID and send" flow the Core Wallet
//! offers, so this module adds the one-sided payment construction used by
//! Litecoin MWEB.
//!
//! # Construction
//!
//! Sender picks an ephemeral scalar `r` and publishes `Ke = r·G`.
//!
//! ```text
//!   t   = H(r·A)              shared secret     (receiver recomputes as H(a·Ke))
//!   b   = H("blind" ‖ t)      blinding factor of the commitment
//!   o   = H("ko"    ‖ t)      one-time key offset
//!   Ko  = B + o·G             one-time output key, published
//!   key = H("aead"  ‖ t)      AEAD key for the value/memo payload
//! ```
//!
//! # What this fixes
//!
//! * **The recipient is no longer on chain.** The old `OutputDescription`
//!   carried `recipient_spend_pk` in cleartext, so the entire payment graph was
//!   public and the anonymity set was exactly 1 (audit finding P-01). `Ko` is a
//!   fresh unlinkable point for every single output, even for repeat payments
//!   to the same address.
//! * **The sender cannot spend what they sent.** Spending requires the secret
//!   for `Ko`, which is `b_spend + o`. The sender knows `o` but not `b_spend`.
//!   This is what makes non-interactive MW safe; without it the sender keeps
//!   the blinding factor and can sweep the funds back.
//! * **Scanning needs only the scan key**, so a view key is a real, useful,
//!   non-spending credential.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::commit::{blind_from_bytes, generator_g, Commitment};
use crate::keys::{Address, CryptoError, ViewKey, WalletKeys};
use crate::schnorr::{self, SchnorrSig};
use crate::{hash_multi, rangeproof};

/// Domain for the sender's output signature.
pub const OUTPUT_SIG_DOMAIN: &[u8] = b"nightfall:output:sig:v2";

/// Maximum memo length in bytes. Fixed-size padding below hides memo length,
/// which would otherwise be a metadata leak.
pub const MEMO_LEN: usize = 64;

/// Plaintext payload sealed to the receiver: value + blinding + memo.
/// Always exactly this long after padding, so ciphertext size reveals nothing.
const PAYLOAD_LEN: usize = 8 + 32 + MEMO_LEN;

fn shared_secret(point: &RistrettoPoint) -> [u8; 32] {
    hash_multi(
        b"nightfall:stealth:shared:v2",
        &[point.compress().as_bytes()],
    )
    .0
}

fn derive_blind(t: &[u8; 32]) -> Scalar {
    blind_from_bytes(b"nightfall:stealth:blind:v2", t)
}

fn derive_key_offset(t: &[u8; 32]) -> Scalar {
    blind_from_bytes(b"nightfall:stealth:ko:v2", t)
}

/// One byte of the shared secret, published with the output.
///
/// # What it buys
///
/// Scanning is unavoidably one scalar multiplication per output — `Ke·a` has to
/// be computed before anything can be said about ownership. What *is* avoidable
/// is the second one. Without a tag the scanner must derive the key offset,
/// compute `B + o·G`, compress it, and compare 32 bytes, for every output on
/// chain including the overwhelming majority that belong to strangers.
///
/// With a tag, one byte is compared instead, and 255 of 256 foreign outputs are
/// discarded right there. Measured on an M3 Pro with
/// `cargo run --release -p nightfall-crypto --example scanbench`:
///
/// ```text
/// per foreign output    before 61,590 ns    after 30,321 ns    2.03x
/// ```
///
/// That is the difference between a phone that syncs and one whose battery
/// report singles out the wallet.
///
/// # What it costs
///
/// One byte per output on chain, and an honest privacy caveat: an observer
/// holding a view key gets the same speedup when testing it against the chain.
/// That is symmetric — it helps a legitimate wallet and an attacker who already
/// has the credential equally, and someone with your view key can already see
/// everything you receive. To an observer *without* the key the tag is
/// indistinguishable from random, and it is fresh per output, so two payments
/// to the same address still share no visible field.
///
/// Monero adopted the identical construction in 2022 and needed a hard fork to
/// do it. Adding it here while the chain is being reset costs nothing.
fn derive_view_tag(t: &[u8; 32]) -> u8 {
    hash_multi(b"nightfall:stealth:viewtag:v3", &[t]).0[0]
}

fn derive_aead(t: &[u8; 32]) -> ([u8; 32], [u8; 24]) {
    let key = hash_multi(b"nightfall:stealth:aead:v2", &[t]).0;
    let nonce_full = hash_multi(b"nightfall:stealth:nonce:v2", &[t]).0;
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&nonce_full[..24]);
    (key, nonce)
}

/// What kind of output this is.
///
/// Public by necessity: coinbase outputs are subject to a maturity delay, and
/// after aggregation there is no transaction structure left from which to
/// infer it. Grin carries the same flag for the same reason. It reveals only
/// "this was a block subsidy", which the block header already announces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFeature {
    #[default]
    Plain,
    Coinbase,
}

impl OutputFeature {
    pub fn byte(self) -> u8 {
        match self {
            OutputFeature::Plain => 0,
            OutputFeature::Coinbase => 1,
        }
    }

    pub fn is_coinbase(self) -> bool {
        matches!(self, OutputFeature::Coinbase)
    }
}

/// An output as it appears on chain. Contains no recipient identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Plain or coinbase. Signed, so a miner cannot relabel a subsidy as an
    /// ordinary output to escape the maturity delay.
    #[serde(default)]
    pub features: OutputFeature,
    /// Value commitment `v·G + b·H`.
    pub commit: Commitment,
    /// Bulletproof that `v ∈ [0, 2^64)`.
    pub range_proof: rangeproof::RangeProofBytes,
    /// Ephemeral public key `Ke = r·G`.
    pub ephemeral_pk: [u8; 32],
    /// One-time output key `Ko = B + o·G`. Unlinkable across payments.
    pub output_pk: [u8; 32],
    /// One byte of the shared secret — see [`derive_view_tag`].
    ///
    /// Covered by `sender_sig`. It has to be: a relay that could flip this byte
    /// would make the output invisible to its recipient, and funds nobody can
    /// find are funds destroyed.
    #[serde(default)]
    pub view_tag: u8,
    /// Sealed `(value ‖ blind ‖ memo)`.
    pub payload: Vec<u8>,
    /// Signature by the ephemeral secret `r`, verifiable against `Ke`.
    ///
    /// This makes the output self-authenticating. Under kernel aggregation the
    /// kernel no longer signs the transaction body, so without this a relay
    /// could corrupt `payload` and permanently destroy the recipient's funds —
    /// audit finding C-05. Only the sender knows `r`, and altering any field
    /// invalidates the signature.
    pub sender_sig: SchnorrSig,
}

impl Output {
    pub fn output_point(&self) -> Option<RistrettoPoint> {
        CompressedRistretto(self.output_pk).decompress()
    }

    /// Stable identifier for this output within the UTXO set.
    pub fn id(&self) -> [u8; 32] {
        hash_multi(
            b"nightfall:output:id:v2",
            &[&self.commit.0, &self.output_pk, &self.ephemeral_pk],
        )
        .0
    }

    /// Every field except the signature itself. This is what the sender signs
    /// and what canonical block hashing folds in.
    pub fn commitment_bytes(&self) -> Vec<u8> {
        let mut v =
            Vec::with_capacity(1 + 32 + 32 + 32 + self.payload.len() + self.range_proof.0.len());
        v.push(self.features.byte());
        v.extend_from_slice(&self.commit.0);
        v.extend_from_slice(&self.ephemeral_pk);
        v.extend_from_slice(&self.output_pk);
        v.push(self.view_tag);
        v.extend_from_slice(&self.range_proof.0);
        v.extend_from_slice(&self.payload);
        v
    }

    fn sig_message(&self) -> Vec<u8> {
        hash_multi(OUTPUT_SIG_DOMAIN, &[&self.commitment_bytes()])
            .0
            .to_vec()
    }

    /// Verify the sender's signature.
    ///
    /// Any tampering with the commitment, the range proof, either key or the
    /// encrypted payload fails this check. Under kernel aggregation the kernel
    /// no longer covers the transaction body, so this is what closes the
    /// griefing hole from audit finding C-05.
    pub fn verify_sender_sig(&self) -> bool {
        let Some(ke) = CompressedRistretto(self.ephemeral_pk).decompress() else {
            return false;
        };
        schnorr::verify(&ke, &generator_g(), &self.sig_message(), &self.sender_sig)
    }
}

/// What the sender must keep in order to build the kernel.
pub struct SenderSecrets {
    pub blind: Scalar,
    pub value: u64,
}

/// What the receiver recovers when an output is theirs.
#[derive(Clone, Debug)]
pub struct DiscoveredOutput {
    pub value: u64,
    pub blind: Scalar,
    /// `o` such that the spend secret is `b_spend + o`.
    pub key_offset: Scalar,
    pub memo: String,
    pub commit: Commitment,
    pub output_pk: [u8; 32],
}

impl DiscoveredOutput {
    /// Secret key authorising the spend of this output.
    pub fn spend_secret(&self, keys: &WalletKeys) -> Scalar {
        keys.spend_secret() + self.key_offset
    }
}

/// Create an output paying `value` to `to`.
pub fn create_output(
    to: &Address,
    value: u64,
    memo: &str,
    ctx: &[u8],
) -> Result<(Output, SenderSecrets), CryptoError> {
    create_output_with_features(to, value, memo, ctx, OutputFeature::Plain)
}

/// Create an output with an explicit feature flag.
pub fn create_output_with_features(
    to: &Address,
    value: u64,
    memo: &str,
    ctx: &[u8],
    features: OutputFeature,
) -> Result<(Output, SenderSecrets), CryptoError> {
    let scan_point = to.scan_point().ok_or(CryptoError::BadAddress)?;
    let spend_point = to.spend_point().ok_or(CryptoError::BadAddress)?;

    let mut r_bytes = [0u8; 64];
    OsRng.fill_bytes(&mut r_bytes);
    let r = Scalar::from_bytes_mod_order_wide(&r_bytes);
    let ephemeral_pk = (generator_g() * r).compress().to_bytes();

    let t = shared_secret(&(scan_point * r));
    let blind = derive_blind(&t);
    let offset = derive_key_offset(&t);
    let output_pk = (spend_point + generator_g() * offset).compress().to_bytes();

    let (range_proof, commit) =
        rangeproof::prove(value, &blind, ctx).map_err(|_| CryptoError::Encrypt)?;

    // Fixed-length plaintext: value ‖ blind ‖ padded memo.
    let mut plain = Vec::with_capacity(PAYLOAD_LEN);
    plain.extend_from_slice(&value.to_le_bytes());
    plain.extend_from_slice(&blind.to_bytes());
    let mut memo_buf = [0u8; MEMO_LEN];
    let mb = memo.as_bytes();
    let n = mb.len().min(MEMO_LEN);
    memo_buf[..n].copy_from_slice(&mb[..n]);
    plain.extend_from_slice(&memo_buf);

    let (key, nonce) = derive_aead(&t);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let payload = cipher
        .encrypt(XNonce::from_slice(&nonce), plain.as_ref())
        .map_err(|_| CryptoError::Encrypt)?;

    let mut output = Output {
        features,
        commit,
        range_proof,
        ephemeral_pk,
        output_pk,
        view_tag: derive_view_tag(&t),
        payload,
        sender_sig: SchnorrSig {
            r: [0u8; 32],
            s: [0u8; 32],
        },
    };
    output.sender_sig = schnorr::sign(&r, &generator_g(), &output.sig_message());

    Ok((output, SenderSecrets { blind, value }))
}

/// The parts of an [`Output`] a scanner actually reads.
///
/// A light client is served these fields alone — the range proof is ~672 bytes
/// and irrelevant to discovering ownership, so shipping it to a phone is
/// several times the bandwidth for no benefit. See the `scan_feed` RPC method.
///
/// This type exists so such a client never has to fabricate an [`Output`] with
/// a placeholder range proof and signature just to call a scan function. That
/// dummy would be a real object with fields that are lies, and would sooner or
/// later be passed to something that checks them.
#[derive(Clone, Debug)]
pub struct ScanCandidate {
    pub commit: Commitment,
    pub ephemeral_pk: [u8; 32],
    pub output_pk: [u8; 32],
    pub view_tag: u8,
    pub payload: Vec<u8>,
}

impl From<&Output> for ScanCandidate {
    fn from(o: &Output) -> Self {
        Self {
            commit: o.commit,
            ephemeral_pk: o.ephemeral_pk,
            output_pk: o.output_pk,
            view_tag: o.view_tag,
            payload: o.payload.clone(),
        }
    }
}

impl ViewKey {
    /// The view tag this key derives for an output with the given ephemeral
    /// key, or `None` if the key does not decompress.
    ///
    /// Exposed for light clients that want to pre-filter a batch before
    /// committing to a full scan, and for benchmarks that need to reproduce
    /// the pre-tag scanning path. It performs the scalar multiplication, so it
    /// saves nothing over [`scan_candidate`] on its own.
    pub fn expected_view_tag(&self, ephemeral_pk: &[u8; 32]) -> Option<u8> {
        let ke = CompressedRistretto(*ephemeral_pk).decompress()?;
        Some(derive_view_tag(&shared_secret(&(ke * self.scan_sk))))
    }
}

/// Test whether `output` belongs to this wallet and, if so, open it.
///
/// Works with a view key alone — no spend secret required.
pub fn scan_output(view: &ViewKey, output: &Output) -> Option<DiscoveredOutput> {
    scan_candidate(view, &ScanCandidate::from(output))
}

/// [`scan_output`] against the loose fields a light client receives.
///
/// Identical checks, including that the commitment opens to the stated value —
/// a light client trusts its node for what exists on chain, but it does not
/// have to trust anyone for what a payment is worth.
pub fn scan_candidate(view: &ViewKey, output: &ScanCandidate) -> Option<DiscoveredOutput> {
    let ke = CompressedRistretto(output.ephemeral_pk).decompress()?;
    let spend_point = view.spend_point()?;

    let t = shared_secret(&(ke * view.scan_sk));

    // First gate, one byte. Wrong for 255 of every 256 foreign outputs, and
    // rejecting here skips the scalar multiplication below — which is the whole
    // point of carrying the tag. Not a security check: a sender could put any
    // byte here, and the key comparison that follows is what actually decides
    // ownership.
    if derive_view_tag(&t) != output.view_tag {
        return None;
    }

    let offset = derive_key_offset(&t);

    // Second gate: does the one-time key match what we would have produced?
    let expected_ko = (spend_point + generator_g() * offset).compress().to_bytes();
    if expected_ko != output.output_pk {
        return None;
    }

    let (key, nonce) = derive_aead(&t);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let plain = cipher
        .decrypt(XNonce::from_slice(&nonce), output.payload.as_ref())
        .ok()?;
    if plain.len() != PAYLOAD_LEN {
        return None;
    }

    let mut v = [0u8; 8];
    v.copy_from_slice(&plain[..8]);
    let value = u64::from_le_bytes(v);
    let mut b = [0u8; 32];
    b.copy_from_slice(&plain[8..40]);
    let blind = Option::<Scalar>::from(Scalar::from_canonical_bytes(b))?;

    // Critical: verify the commitment actually opens to the stated value.
    // A malicious sender could otherwise claim "I paid you 100" while the
    // commitment holds 1, and the wallet would show phantom funds.
    if Commitment::new(value, &blind) != output.commit {
        return None;
    }

    let memo = String::from_utf8_lossy(&plain[40..])
        .trim_end_matches('\0')
        .to_string();

    Some(DiscoveredOutput {
        value,
        blind,
        key_offset: offset,
        memo,
        commit: output.commit,
        output_pk: output.output_pk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag must never cost the recipient their own coins. If it produced a
    /// single false negative the wallet would simply not see a payment, and
    /// there would be nothing on chain to indicate why.
    #[test]
    fn the_view_tag_never_hides_an_output_from_its_owner() {
        use nightfall_types::NetworkId;
        let ctx = NetworkId::Devnet.proof_context();
        let me = crate::WalletKeys::generate();
        let view = me.view_key();

        // Sixteen full outputs, not hundreds. Each costs a Bulletproof, which
        // dominates the runtime and tests nothing about tags. The property is
        // deterministic, so a handful of independent shared secrets exercises
        // it as well as a thousand would — and a release should not wait ten
        // minutes on a slow runner for the difference.
        for i in 0..16u32 {
            let (out, _) =
                create_output(&me.address(), 1_000 + u64::from(i), "", ctx).expect("output");
            assert!(
                scan_output(&view, &out).is_some(),
                "output {i} was hidden from its own recipient"
            );
        }
    }

    /// And it must actually reject. A tag that matched everything would be a
    /// byte of overhead buying nothing.
    #[test]
    fn the_view_tag_rejects_almost_every_stranger() {
        use nightfall_types::NetworkId;
        let ctx = NetworkId::Devnet.proof_context();
        let me = crate::WalletKeys::generate();
        let view = me.view_key();

        // The rate needs a large sample; it does not need range proofs. The tag
        // depends only on the ECDH, so deriving both sides directly measures
        // the same thing thousands of times over for less than a handful of
        // full outputs would cost.
        let mut survived_the_tag = 0u32;
        let total = 4_096u32;
        for _ in 0..total {
            let stranger = crate::WalletKeys::generate();
            let mut wide = [0u8; 64];
            OsRng.fill_bytes(&mut wide);
            let ephemeral = Scalar::from_bytes_mod_order_wide(&wide);
            let ephemeral_pk = (generator_g() * ephemeral).compress().to_bytes();

            // What a sender paying that stranger would publish.
            let theirs = derive_view_tag(&shared_secret(
                &(stranger.address().scan_point().unwrap() * ephemeral),
            ));
            // What our scanner derives for the same output.
            let ours = view.expected_view_tag(&ephemeral_pk).unwrap();

            if ours == theirs {
                survived_the_tag += 1;
            }
        }

        // Expected 4096/256 = 16. Sixty is far enough out to catch a tag that
        // is not derived from the shared secret at all, and loose enough never
        // to fail on an unlucky seed.
        assert!(
            survived_the_tag < 60,
            "{survived_the_tag} of {total} strangers passed the tag — \
             it is not discriminating"
        );

        // A few complete outputs, to confirm the gate is wired into the scan
        // and not merely derivable on its own.
        for _ in 0..8 {
            let stranger = crate::WalletKeys::generate();
            let (out, _) = create_output(&stranger.address(), 1, "", ctx).expect("output");
            assert!(
                scan_output(&view, &out).is_none(),
                "claimed a foreign output"
            );
        }
    }

    /// The tag is signed, so a relay cannot flip it. If it could, the recipient
    /// would never find the output and the funds would be gone with no trace of
    /// tampering.
    #[test]
    fn tampering_with_the_view_tag_breaks_the_signature() {
        use nightfall_types::NetworkId;
        let ctx = NetworkId::Devnet.proof_context();
        let me = crate::WalletKeys::generate();

        let (mut out, _) = create_output(&me.address(), 500, "", ctx).expect("output");
        assert!(out.verify_sender_sig());

        out.view_tag ^= 0xff;
        assert!(
            !out.verify_sender_sig(),
            "a flipped view tag must invalidate the sender signature"
        );
        assert!(scan_output(&me.view_key(), &out).is_none());
    }

    /// A light client scanning the stripped fields must reach exactly the same
    /// conclusion as a full node scanning the whole output. If these ever
    /// diverge, a phone silently misses payments a desktop can see.
    #[test]
    fn stripped_fields_scan_identically() {
        use nightfall_types::NetworkId;
        let ctx = NetworkId::Devnet.proof_context();
        let me = crate::WalletKeys::generate();
        let stranger = crate::WalletKeys::generate();

        let (mine, _) = create_output(&me.address(), 7_777, "hallo", ctx).expect("output");
        let (theirs, _) = create_output(&stranger.address(), 1, "", ctx).expect("output");

        let view = me.view_key();
        for output in [&mine, &theirs] {
            let full = scan_output(&view, output);
            let light = scan_candidate(&view, &ScanCandidate::from(output));
            assert_eq!(full.is_some(), light.is_some());
            if let (Some(a), Some(b)) = (full, light) {
                assert_eq!(a.value, b.value);
                assert_eq!(a.commit, b.commit);
                assert_eq!(a.memo, b.memo);
                assert_eq!(a.blind, b.blind);
            }
        }
    }

    const CTX: &[u8] = b"nightfall:test";

    #[test]
    fn receiver_finds_and_opens_own_output() {
        let bob = WalletKeys::generate();
        let (out, secrets) = create_output(&bob.address(), 12_345, "pizza", CTX).unwrap();

        let found = scan_output(&bob.view_key(), &out).expect("bob must find his output");
        assert_eq!(found.value, 12_345);
        assert_eq!(found.memo, "pizza");
        assert_eq!(found.blind, secrets.blind);
    }

    #[test]
    fn stranger_cannot_detect_the_output() {
        let bob = WalletKeys::generate();
        let eve = WalletKeys::generate();
        let (out, _) = create_output(&bob.address(), 1, "", CTX).unwrap();
        assert!(scan_output(&eve.view_key(), &out).is_none());
    }

    #[test]
    fn repeat_payments_are_unlinkable() {
        // The whole point: two payments to the SAME address must share no
        // on-chain field. This is what the old recipient_spend_pk destroyed.
        let bob = WalletKeys::generate();
        let (a, _) = create_output(&bob.address(), 100, "", CTX).unwrap();
        let (b, _) = create_output(&bob.address(), 100, "", CTX).unwrap();

        assert_ne!(a.output_pk, b.output_pk);
        assert_ne!(a.ephemeral_pk, b.ephemeral_pk);
        assert_ne!(a.commit, b.commit);
        assert_ne!(a.payload, b.payload);

        // ...yet Bob still finds both.
        assert!(scan_output(&bob.view_key(), &a).is_some());
        assert!(scan_output(&bob.view_key(), &b).is_some());
    }

    #[test]
    fn view_key_sees_but_the_spend_key_is_separate() {
        let bob = WalletKeys::generate();
        let (out, _) = create_output(&bob.address(), 500, "memo", CTX).unwrap();
        let found = scan_output(&bob.view_key(), &out).unwrap();
        assert_eq!(found.value, 500);

        // The spend secret for this output requires bob's spend key.
        let sk = found.spend_secret(&bob);
        let expected_ko = generator_g() * sk;
        assert_eq!(expected_ko.compress().to_bytes(), out.output_pk);
    }

    #[test]
    fn lying_sender_is_caught() {
        // Sender crafts a payload claiming a huge value while committing to 1.
        let bob = WalletKeys::generate();
        let (mut out, _) = create_output(&bob.address(), 1, "", CTX).unwrap();

        let ke = CompressedRistretto(out.ephemeral_pk).decompress().unwrap();
        let t = shared_secret(&(ke * bob.scan_secret()));
        let blind = derive_blind(&t);
        let (key, nonce) = derive_aead(&t);

        let mut plain = Vec::new();
        plain.extend_from_slice(&9_999_999u64.to_le_bytes()); // the lie
        plain.extend_from_slice(&blind.to_bytes());
        plain.extend_from_slice(&[0u8; MEMO_LEN]);
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        out.payload = cipher
            .encrypt(XNonce::from_slice(&nonce), plain.as_ref())
            .unwrap();

        assert!(
            scan_output(&bob.view_key(), &out).is_none(),
            "wallet must reject a payload that does not open the commitment"
        );
    }

    #[test]
    fn payload_length_is_constant() {
        let bob = WalletKeys::generate();
        let (short, _) = create_output(&bob.address(), 1, "", CTX).unwrap();
        let (long, _) = create_output(&bob.address(), 1, &"x".repeat(MEMO_LEN), CTX).unwrap();
        assert_eq!(
            short.payload.len(),
            long.payload.len(),
            "ciphertext length must not leak memo length"
        );
    }

    #[test]
    fn sender_signature_seals_the_output() {
        let bob = WalletKeys::generate();
        let (out, _) = create_output(&bob.address(), 42, "memo", CTX).unwrap();
        assert!(out.verify_sender_sig());

        // Corrupting the payload must invalidate it. This is the griefing
        // attack that killed funds in v4.
        let mut grief = out.clone();
        let last = grief.payload.len() - 1;
        grief.payload[last] ^= 0xFF;
        assert!(!grief.verify_sender_sig());

        // So must swapping the one-time key or the commitment.
        let mut swapped = out.clone();
        swapped.output_pk = [7u8; 32];
        assert!(!swapped.verify_sender_sig());
    }

    #[test]
    fn range_proof_is_attached_and_valid() {
        let bob = WalletKeys::generate();
        let (out, _) = create_output(&bob.address(), 777, "", CTX).unwrap();
        assert!(rangeproof::verify(&out.range_proof, &out.commit, CTX));
    }
}
