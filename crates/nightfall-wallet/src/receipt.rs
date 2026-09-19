//! Selective payment disclosure.
//!
//! A receipt opens one output (amount + blinding) and is signed by the
//! wallet's spend key. An auditor verifies the commitment matches and the
//! signature belongs to `nf1…` — without receiving the view key that would
//! reveal every other payment.
//!
//! # What a receipt proves, and what it does not
//!
//! This distinction is the whole value of the document, so it is stated here
//! rather than left to whoever renders it. [`verify_receipt`] returns a
//! [`ReceiptProof`] saying which of these actually held, and every screen that
//! shows a receipt must show that report rather than the raw fields.
//!
//! - **Proven, offline, always:** the signature belongs to the spend key of
//!   the address named in the receipt. Somebody holding that key wrote this.
//! - **Proven, offline, only when a blinding factor is present:** the
//!   commitment opens to exactly the stated amount. A *sent* receipt carries
//!   no blinding factor and no commitment — its `commit` field is a
//!   transaction id — so it proves nothing about an amount beyond the signer's
//!   own word for it. That is a far weaker claim than a received receipt makes
//!   and a merchant must not be left to guess which one they are holding.
//! - **Not proven here, ever:** that any of this is on the chain. Nothing in
//!   this module consults a node. Inclusion and confirmation depth are a
//!   separate question with a separate answer, and a wallet that blurred the
//!   two would be telling its owner that a signed sentence is a settled
//!   payment.
//!
//! # Version 2, and why version 1 is still accepted
//!
//! Version 1 signed six of its ten fields. `kind` and `timestamp` were not
//! among them, so a valid v1 receipt could have `sent` rewritten to `received`
//! — or its date moved by a year — and [`verify_receipt`] would still have
//! returned success, with the verifier then printing the rewritten words as
//! though they were part of what was checked. Those two words make very
//! different claims about who is owed what.
//!
//! Version 2 signs every field the document displays, under its own domain
//! string so a v1 signature can never be replayed as a v2 one. Version 1
//! receipts remain verifiable, because people may have kept them as proof of
//! payments already made and breaking that would destroy the only evidence
//! they have — but the report says plainly which fields the signature did not
//! cover, and every screen must show that.

use anyhow::{bail, Context};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::{generator_g, hash_multi, Address, Commitment, SchnorrSig, WalletKeys};
use serde::{Deserialize, Serialize};

use crate::{Direction, HistoryEntry, OwnedOutput, Wallet};

const RECEIPT_DOMAIN: &[u8] = b"nightfall:receipt:v1";
/// A separate domain, so a v1 signature cannot be presented as a v2 one.
const RECEIPT_DOMAIN_V2: &[u8] = b"nightfall:receipt:v2";
/// What this wallet writes. Older versions are read, never written.
pub const RECEIPT_VERSION: u32 = 2;

/// The fields a version 1 signature does not cover.
const V1_UNSIGNED: &[&str] = &["kind", "timestamp"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaymentReceipt {
    pub v: u32,
    pub kind: String,
    pub address: String,
    pub amount_darks: u64,
    pub memo: String,
    pub commit: String,
    pub blind: String,
    pub height: u64,
    pub timestamp: u64,
    pub sig_r: String,
    pub sig_s: String,
}

impl PaymentReceipt {
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

fn receipt_msg(
    address: &str,
    amount: u64,
    commit: &str,
    blind: &str,
    memo: &str,
    height: u64,
) -> [u8; 32] {
    hash_multi(
        RECEIPT_DOMAIN,
        &[
            address.as_bytes(),
            &amount.to_le_bytes(),
            commit.as_bytes(),
            blind.as_bytes(),
            memo.as_bytes(),
            &height.to_le_bytes(),
        ],
    )
    .0
}

/// Everything the document shows, under a domain of its own.
fn receipt_msg_v2(r: &PaymentReceipt) -> [u8; 32] {
    hash_multi(
        RECEIPT_DOMAIN_V2,
        &[
            r.kind.as_bytes(),
            r.address.as_bytes(),
            &r.amount_darks.to_le_bytes(),
            r.commit.as_bytes(),
            r.blind.as_bytes(),
            r.memo.as_bytes(),
            &r.height.to_le_bytes(),
            &r.timestamp.to_le_bytes(),
        ],
    )
    .0
}

fn sign_receipt(keys: &WalletKeys, r: &mut PaymentReceipt) {
    r.v = RECEIPT_VERSION;
    let sig = nightfall_crypto::sig::sign(&keys.spend_secret(), &generator_g(), &receipt_msg_v2(r));
    r.sig_r = hex::encode(sig.r);
    r.sig_s = hex::encode(sig.s);
}

/// What kind of payment a receipt describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptKind {
    Received,
    Mined,
    Sent,
    /// A word this version does not know. Shown as unknown rather than guessed.
    Other,
}

impl ReceiptKind {
    fn parse(text: &str) -> Self {
        match text {
            "received" => Self::Received,
            "mined" => Self::Mined,
            "sent" => Self::Sent,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Mined => "mined",
            Self::Sent => "sent",
            Self::Other => "unrecognised",
        }
    }
}

/// What a receipt was actually found to prove.
///
/// Returned instead of `()` so that no screen has to decide for itself what a
/// successful verification meant. The three questions are kept apart on
/// purpose — see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptProof {
    pub version: u32,
    pub kind: ReceiptKind,
    pub address: String,
    pub amount_darks: u64,
    pub memo: String,
    pub height: u64,
    pub timestamp: u64,
    /// The commitment, for a received or mined output; the transaction id for
    /// a sent one. Two different things, which is why `amount_proven` exists.
    pub reference: String,
    /// The commitment was opened and matches `amount_darks` exactly.
    ///
    /// False for a sent receipt, where the amount is the signer's own word.
    pub amount_proven: bool,
    /// Fields the signature does not cover. Empty for version 2.
    pub unsigned_fields: &'static [&'static str],
}

impl ReceiptProof {
    /// One sentence a person can act on, written once so that the desktop
    /// wallet, the command line and anything added later cannot describe the
    /// same document differently.
    pub fn summary(&self) -> String {
        let who = &self.address;
        if self.amount_proven {
            format!(
                "The holder of {who}'s spend key signed this, and the commitment \
                 opens to exactly the amount shown."
            )
        } else {
            format!(
                "The holder of {who}'s spend key signed this. The amount is their \
                 own statement: this kind of receipt carries no commitment to open, \
                 so nothing here confirms the figure."
            )
        }
    }

    /// What this document does not establish, whatever it says.
    ///
    /// Always non-empty: chain inclusion is never checked by verification, and
    /// a screen that omitted this line would be presenting a signed sentence
    /// as a settled payment.
    pub fn not_established(&self) -> Vec<String> {
        let mut open = vec![
            "That this payment is in the chain. Verification never contacts a \
             node, so inclusion and confirmation depth are a separate question."
                .to_owned(),
        ];
        if !self.amount_proven {
            open.push(
                "That the amount is what the receipt says. There is no commitment \
                 here to check it against."
                    .to_owned(),
            );
        }
        if !self.unsigned_fields.is_empty() {
            open.push(format!(
                "That {} {} what the signer wrote. This is an older receipt \
                 whose signature does not cover {}, so {} can be changed without \
                 breaking it. Ask for a new receipt if that matters.",
                self.unsigned_fields.join(" and "),
                if self.unsigned_fields.len() == 1 { "is" } else { "are" },
                if self.unsigned_fields.len() == 1 { "it" } else { "them" },
                if self.unsigned_fields.len() == 1 { "it" } else { "they" },
            ));
        }
        open
    }
}

impl Wallet {
    /// Prove a received (or mined) output without handing over the view key.
    pub fn prove_output(&self, commit_hex: &str) -> anyhow::Result<PaymentReceipt> {
        let needle = commit_hex.trim().to_ascii_lowercase();
        let out = self
            .db
            .outputs
            .iter()
            .find(|o| {
                hex::encode(o.commit.0) == needle || hex::encode(o.commit.0).starts_with(&needle)
            })
            .context("no owned output matches that commitment")?;
        let hist = self
            .db
            .history
            .iter()
            .find(|e| e.txid == hex::encode(out.commit.0));
        Ok(self.receipt_from_output(out, hist))
    }

    pub fn prove_history(&self, txid: &str) -> anyhow::Result<PaymentReceipt> {
        let needle = txid.trim().to_ascii_lowercase();
        let hist = self
            .db
            .history
            .iter()
            .find(|e| e.txid == needle || e.txid.starts_with(&needle))
            .context("no history entry matches")?;
        match hist.direction {
            Direction::Received | Direction::Mined => self.prove_output(&hist.txid),
            Direction::Sent => {
                let mut r = PaymentReceipt {
                    v: 1,
                    kind: "sent".into(),
                    address: self.address_string(),
                    amount_darks: hist.amount,
                    memo: hist.memo.clone(),
                    commit: hist.txid.clone(),
                    blind: String::new(),
                    height: hist.height.unwrap_or(0),
                    timestamp: hist.timestamp,
                    sig_r: String::new(),
                    sig_s: String::new(),
                };
                sign_receipt(&self.keys, &mut r);
                Ok(r)
            }
        }
    }

    fn receipt_from_output(
        &self,
        out: &OwnedOutput,
        hist: Option<&HistoryEntry>,
    ) -> PaymentReceipt {
        let kind = if out.is_coinbase { "mined" } else { "received" };
        let mut r = PaymentReceipt {
            v: 1,
            kind: kind.into(),
            address: self.address_string(),
            amount_darks: out.value,
            memo: out.memo.clone(),
            commit: hex::encode(out.commit.0),
            blind: out.blind_hex.clone(),
            height: out.height,
            timestamp: hist.map(|h| h.timestamp).unwrap_or(0),
            sig_r: String::new(),
            sig_s: String::new(),
        };
        sign_receipt(&self.keys, &mut r);
        r
    }
}

/// Verify a receipt and report what that established.
///
/// Opening is checked when a blinding factor is present. No node is contacted
/// and none ever will be from here: see the module documentation for why
/// inclusion is deliberately a separate question.
pub fn verify_receipt(receipt: &PaymentReceipt) -> anyhow::Result<ReceiptProof> {
    let unsigned_fields: &'static [&'static str] = match receipt.v {
        1 => V1_UNSIGNED,
        2 => &[],
        other => bail!(
            "This receipt is version {other}, which this wallet does not know how \
             to check. A newer version may say more than this one can read, so it \
             is refused rather than half-understood."
        ),
    };
    let addr = Address::decode(&receipt.address).context("receipt address")?;
    let mut amount_proven = false;
    if !receipt.blind.is_empty() {
        let raw = hex::decode(&receipt.commit).context("commit hex")?;
        if raw.len() != 32 {
            bail!("commit must be 32 bytes");
        }
        let mut c = [0u8; 32];
        c.copy_from_slice(&raw);
        let commit = Commitment(c);
        let blind_raw = hex::decode(&receipt.blind).context("blind hex")?;
        if blind_raw.len() != 32 {
            bail!("blind must be 32 bytes");
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&blind_raw);
        let blind = Option::<Scalar>::from(Scalar::from_canonical_bytes(b))
            .context("non-canonical blinding factor")?;
        if Commitment::new(receipt.amount_darks, &blind) != commit {
            bail!("commitment does not open to the stated amount");
        }
        amount_proven = true;
    }
    let msg = if receipt.v == 1 {
        receipt_msg(
            &receipt.address,
            receipt.amount_darks,
            &receipt.commit,
            &receipt.blind,
            &receipt.memo,
            receipt.height,
        )
    } else {
        receipt_msg_v2(receipt)
    };
    let r = hex::decode(&receipt.sig_r).context("sig_r")?;
    let s = hex::decode(&receipt.sig_s).context("sig_s")?;
    if r.len() != 32 || s.len() != 32 {
        bail!("signature fields must be 32 bytes");
    }
    let mut sig = SchnorrSig {
        r: [0; 32],
        s: [0; 32],
    };
    sig.r.copy_from_slice(&r);
    sig.s.copy_from_slice(&s);
    let pk = CompressedRistretto(addr.spend_pk)
        .decompress()
        .context("spend public key")?;
    if !nightfall_crypto::sig::verify(&pk, &generator_g(), &msg, &sig) {
        bail!("receipt signature is not valid for this address");
    }
    Ok(ReceiptProof {
        version: receipt.v,
        kind: ReceiptKind::parse(&receipt.kind),
        address: receipt.address.clone(),
        amount_darks: receipt.amount_darks,
        memo: receipt.memo.clone(),
        height: receipt.height,
        timestamp: receipt.timestamp,
        reference: receipt.commit.clone(),
        amount_proven,
        unsigned_fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    fn received(keys: &WalletKeys) -> PaymentReceipt {
        let blind = Scalar::from(7u64);
        let amount = 20 * 100_000_000u64;
        let mut r = PaymentReceipt {
            v: 1,
            kind: "received".into(),
            address: keys.address().encode(),
            amount_darks: amount,
            memo: "invoice 42".into(),
            commit: hex::encode(Commitment::new(amount, &blind).0),
            blind: hex::encode(blind.to_bytes()),
            height: 100,
            timestamp: 1,
            sig_r: String::new(),
            sig_s: String::new(),
        };
        sign_receipt(keys, &mut r);
        r
    }

    /// Sign the way version 1 did, so the old format can still be exercised.
    fn sign_as_v1(keys: &WalletKeys, r: &mut PaymentReceipt) {
        r.v = 1;
        let msg = receipt_msg(
            &r.address,
            r.amount_darks,
            &r.commit,
            &r.blind,
            &r.memo,
            r.height,
        );
        let sig = nightfall_crypto::sig::sign(&keys.spend_secret(), &generator_g(), &msg);
        r.sig_r = hex::encode(sig.r);
        r.sig_s = hex::encode(sig.s);
    }

    #[test]
    fn opening_and_signature_roundtrip() {
        let keys = WalletKeys::generate();
        let mut r = received(&keys);
        let proof = verify_receipt(&r).unwrap();
        assert_eq!(proof.version, RECEIPT_VERSION);
        assert_eq!(proof.kind, ReceiptKind::Received);
        assert!(proof.amount_proven);
        assert!(proof.unsigned_fields.is_empty());
        r.amount_darks += 1;
        assert!(verify_receipt(&r).is_err());
    }

    /// The defect version 2 exists to close.
    ///
    /// A version 1 signature covers six of the ten fields. `kind` is not one
    /// of them, so "sent" could be rewritten to "received" — turning a signed
    /// statement about an amount into an apparent proof of one — and the old
    /// `verify_receipt` returned plain success, after which the verifier
    /// printed the rewritten word as though it had been checked.
    #[test]
    fn version_one_lets_kind_and_timestamp_be_rewritten_and_says_so() {
        let keys = WalletKeys::generate();
        let mut r = received(&keys);
        sign_as_v1(&keys, &mut r);
        assert!(verify_receipt(&r).is_ok(), "old receipts must stay verifiable");

        for (field, rewrite) in [
            ("kind", (|r: &mut PaymentReceipt| r.kind = "sent".into())
                as fn(&mut PaymentReceipt)),
            ("timestamp", |r: &mut PaymentReceipt| r.timestamp += 31_536_000),
        ] {
            let mut tampered = r.clone();
            rewrite(&mut tampered);
            let proof = verify_receipt(&tampered)
                .unwrap_or_else(|e| panic!("v1 still verifies after {field} is changed: {e}"));
            assert!(
                proof.unsigned_fields.contains(&field),
                "{field} was changed and the report must say the signature does not cover it",
            );
            let warning = proof.not_established().join(" ");
            assert!(warning.contains(field), "{warning}");
        }
    }

    /// …and version 2 does not. Every field the document shows is signed, and
    /// the v1 signature cannot be replayed under the v2 domain.
    #[test]
    fn version_two_covers_every_field_it_displays() {
        let keys = WalletKeys::generate();
        let good = received(&keys);
        assert_eq!(good.v, 2);
        assert!(verify_receipt(&good).unwrap().unsigned_fields.is_empty());

        let changes: [(&str, fn(&mut PaymentReceipt)); 7] = [
            ("kind", |r| r.kind = "sent".into()),
            ("timestamp", |r| r.timestamp += 1),
            ("amount", |r| r.amount_darks += 1),
            ("memo", |r| r.memo.push('!')),
            ("height", |r| r.height += 1),
            ("commit", |r| r.commit = hex::encode([9u8; 32])),
            ("blind", |r| r.blind = hex::encode([0u8; 32])),
        ];
        for (field, rewrite) in changes {
            let mut tampered = good.clone();
            rewrite(&mut tampered);
            assert!(
                verify_receipt(&tampered).is_err(),
                "changing {field} must break a version 2 receipt",
            );
        }

        // A v1 signature presented as v2 must fail: the domains differ.
        let mut replayed = good.clone();
        sign_as_v1(&keys, &mut replayed);
        replayed.v = 2;
        assert!(verify_receipt(&replayed).is_err());
    }

    /// A sent receipt is a signed sentence, not a proof of an amount, and the
    /// report must not let a screen present the two as the same thing.
    #[test]
    fn a_sent_receipt_does_not_prove_its_amount() {
        let keys = WalletKeys::generate();
        let mut r = received(&keys);
        r.kind = "sent".into();
        r.blind = String::new();
        r.commit = hex::encode([4u8; 32]); // a transaction id, not a commitment
        sign_receipt(&keys, &mut r);

        let proof = verify_receipt(&r).unwrap();
        assert_eq!(proof.kind, ReceiptKind::Sent);
        assert!(!proof.amount_proven);
        assert!(proof.summary().contains("their own statement"));
        let open = proof.not_established();
        assert!(open.iter().any(|line| line.contains("amount is what the receipt says")));
        // …and the chain line is there whatever kind it is.
        assert!(open.iter().any(|line| line.contains("in the chain")));
        assert!(verify_receipt(&received(&keys)).unwrap().not_established()
            .iter().any(|line| line.contains("in the chain")));
    }

    /// The auditor's whole path, on a receipt this wallet really produced:
    /// prove an owned output, write it out, read it back from text, verify.
    /// Everything before this test builds receipts by hand; this one does not.
    #[test]
    fn a_real_receipt_survives_being_written_out_and_read_back_by_someone_else() {
        use crate::OwnedOutput;
        use nightfall_types::NetworkId;

        let keys = WalletKeys::from_seed([13; 32]);
        let mut wallet = crate::Wallet::in_memory(NetworkId::Devnet, keys.clone(), 0);
        let blind = Scalar::from(4_242u64);
        let value = 7 * 100_000_000;
        wallet.test_insert_output(OwnedOutput {
            commit: Commitment::new(value, &blind),
            value,
            blind_hex: hex::encode(blind.to_bytes()),
            key_offset_hex: hex::encode([2u8; 32]),
            memo: "invoice 17".into(),
            height: 9,
            is_coinbase: false,
            spent: false,
        });

        let commit_hex = hex::encode(Commitment::new(value, &blind).0);
        let json = wallet.prove_output(&commit_hex).unwrap().to_json().unwrap();

        // Everything the auditor has is this text.
        let received: PaymentReceipt = serde_json::from_str(&json).unwrap();
        let proof = verify_receipt(&received).unwrap();
        assert_eq!(proof.version, RECEIPT_VERSION);
        assert_eq!(proof.kind, ReceiptKind::Received);
        assert_eq!(proof.amount_darks, value);
        assert_eq!(proof.address, keys.address().encode());
        assert_eq!(proof.memo, "invoice 17");
        assert!(proof.amount_proven, "a received receipt opens its commitment");
        assert!(proof.unsigned_fields.is_empty());

        // The seed is not in the document, and neither is the view key. A
        // receipt that leaked either would disclose every other payment, which
        // is the thing it exists to avoid.
        assert!(!json.contains(&hex::encode(keys.seed)));
        assert!(!json.contains(&wallet.view_key_string()));

        // One character changed anywhere in the text and it stops verifying.
        for field in ["\"amount_darks\": 700000000", "\"height\": 9"] {
            assert!(json.contains(field), "{json}");
        }
        let tampered: PaymentReceipt =
            serde_json::from_str(&json.replace("\"height\": 9", "\"height\": 10")).unwrap();
        assert!(verify_receipt(&tampered).is_err());
    }

    #[test]
    fn an_unknown_version_is_refused_rather_than_half_read() {
        let keys = WalletKeys::generate();
        let mut r = received(&keys);
        r.v = 3;
        let error = verify_receipt(&r).unwrap_err().to_string();
        assert!(error.contains("version 3"), "{error}");
        assert!(error.contains("refused"), "{error}");
    }
}
