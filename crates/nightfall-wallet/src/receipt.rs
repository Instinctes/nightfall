//! Selective payment disclosure.
//!
//! A receipt opens one output (amount + blinding) and is signed by the
//! wallet's spend key. An auditor verifies the commitment matches and the
//! signature belongs to `nf1…` — without receiving the view key that would
//! reveal every other payment.

use anyhow::{bail, Context};
use curve25519_dalek::ristretto::CompressedRistretto;
use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::{
    generator_g, hash_multi, Address, Commitment, SchnorrSig, WalletKeys,
};
use serde::{Deserialize, Serialize};

use crate::{Direction, HistoryEntry, OwnedOutput, Wallet};

const RECEIPT_DOMAIN: &[u8] = b"nightfall:receipt:v1";

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

fn sign_receipt(keys: &WalletKeys, r: &mut PaymentReceipt) {
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

impl Wallet {
    /// Prove a received (or mined) output without handing over the view key.
    pub fn prove_output(&self, commit_hex: &str) -> anyhow::Result<PaymentReceipt> {
        let needle = commit_hex.trim().to_ascii_lowercase();
        let out = self
            .db
            .outputs
            .iter()
            .find(|o| hex::encode(o.commit.0) == needle || hex::encode(o.commit.0).starts_with(&needle))
            .context("no owned output matches that commitment")?;
        let hist = self.db.history.iter().find(|e| e.txid == hex::encode(out.commit.0));
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

    fn receipt_from_output(&self, out: &OwnedOutput, hist: Option<&HistoryEntry>) -> PaymentReceipt {
        let kind = if out.is_coinbase {
            "mined"
        } else {
            "received"
        };
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

/// Verify a receipt. Opening is checked when a blinding factor is present.
pub fn verify_receipt(receipt: &PaymentReceipt) -> anyhow::Result<()> {
    if receipt.v != 1 {
        bail!("unsupported receipt version {}", receipt.v);
    }
    let addr = Address::decode(&receipt.address).context("receipt address")?;
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
    }
    let msg = receipt_msg(
        &receipt.address,
        receipt.amount_darks,
        &receipt.commit,
        &receipt.blind,
        &receipt.memo,
        receipt.height,
    );
    let r = hex::decode(&receipt.sig_r).context("sig_r")?;
    let s = hex::decode(&receipt.sig_s).context("sig_s")?;
    if r.len() != 32 || s.len() != 32 {
        bail!("signature fields must be 32 bytes");
    }
    let mut sig = SchnorrSig { r: [0; 32], s: [0; 32] };
    sig.r.copy_from_slice(&r);
    sig.s.copy_from_slice(&s);
    let pk = CompressedRistretto(addr.spend_pk)
        .decompress()
        .context("spend public key")?;
    if !nightfall_crypto::sig::verify(&pk, &generator_g(), &msg, &sig) {
        bail!("receipt signature is not valid for this address");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    #[test]
    fn opening_and_signature_roundtrip() {
        let keys = WalletKeys::generate();
        let blind = Scalar::from(7u64);
        let amount = 20 * 100_000_000u64;
        let commit = Commitment::new(amount, &blind);
        let mut r = PaymentReceipt {
            v: 1,
            kind: "received".into(),
            address: keys.address().encode(),
            amount_darks: amount,
            memo: "invoice 42".into(),
            commit: hex::encode(commit.0),
            blind: hex::encode(blind.to_bytes()),
            height: 100,
            timestamp: 1,
            sig_r: String::new(),
            sig_s: String::new(),
        };
        sign_receipt(&keys, &mut r);
        verify_receipt(&r).unwrap();
        r.amount_darks += 1;
        assert!(verify_receipt(&r).is_err());
    }
}
