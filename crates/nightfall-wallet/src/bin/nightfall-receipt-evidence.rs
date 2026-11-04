//! Offline, anchored evidence for one voluntarily disclosed NIGHT payment.
//!
//! This verifies a v2 receipt, its amount opening, a canonical block body,
//! the header's body commitment and membership in a *specific* block hash.
//! The anchor MUST come from an independently validated chain source.
//! This does NOT validate header ancestry, difficulty adjustment, proof of
//! work, consensus, finality, confirmation depth, ownership or unspentness.
//!
//! Unlike a receipt-only success message, this tool distinguishes signed
//! payment claims from conditional evidence of inclusion in a given block.

use anyhow::{bail, Context, Result};
use clap::Parser;
use nightfall_consensus::Block;
use nightfall_types::Hash256;
use nightfall_wallet::{verify_receipt, PaymentReceipt, ReceiptKind, ReceiptProof};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_RECEIPT_BYTES: usize = 64 * 1024;
const MAX_BLOCK_JSON_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "nightfall-receipt-evidence",
    about = "Verify a disclosed payment against an independently anchored block"
)]
struct Cli {
    /// Version 2 receipt JSON exported by the wallet.
    #[arg(long)]
    receipt: PathBuf,

    /// One full JSON Block object, not a get_blocks array or an RPC wrapper.
    #[arg(long)]
    block: PathBuf,

    /// Exact block hash obtained separately from an independently verified chain.
    #[arg(long)]
    anchor: String,
}

/// Facts established by comparing a receipt to one *specified* block.
/// No field in this result asserts that the block belongs to the best chain.
#[derive(Debug)]
struct AnchoredEvidence {
    receipt: ReceiptProof,
    block_hash: Hash256,
    output_index: usize,
}

fn verify_anchored(
    receipt: &PaymentReceipt,
    block: &Block,
    independently_checked_hash: Hash256,
) -> Result<AnchoredEvidence> {
    // In v1, "kind" was NOT signed. Do not let an attacker relabel a sent
    // claim as received and then advertise an inclusion result.
    if receipt.v != 2 {
        bail!("anchored evidence requires a v2 receipt with signed kind");
    }
    let proof = verify_receipt(receipt).context("receipt cryptography failed")?;
    if !matches!(proof.kind, ReceiptKind::Received | ReceiptKind::Mined) {
        bail!("only received/mined commitment-opening receipts are supported");
    }
    if !proof.amount_proven {
        bail!("receipt contains no proven commitment opening");
    }

    if block.hash() != independently_checked_hash {
        bail!("supplied block does not match the independent hash anchor");
    }
    if block.header.height.0 != receipt.height {
        bail!("receipt height does not match the anchored block");
    }
    if !block.body.is_canonical() {
        bail!("block body is not in canonical order");
    }
    if block.header.body_root != block.body.hash() {
        bail!("block body is not committed by the supplied header");
    }

    // verify_receipt has already checked that commit is 32 bytes, and has
    // cryptographically checked its opening and the owner's signature.
    let target = hex::decode(&receipt.commit).context("receipt commitment hex")?;
    let mut index = None;
    for (i, output) in block.body.outputs.iter().enumerate() {
        if output.commit.0.as_slice() == target.as_slice() && index.replace(i).is_some() {
            bail!("duplicate output commitment in supplied block");
        }
    }
    let output_index = index.context("receipt commitment absent from block outputs")?;

    Ok(AnchoredEvidence {
        receipt: proof,
        block_hash: independently_checked_hash,
        output_index,
    })
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.len() > limit {
        bail!("input exceeds safe JSON size limit ({limit} bytes)");
    }
    Ok(bytes)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let anchor =
        Hash256::from_hex(&cli.anchor).context("anchor must be an exact 32-byte hex block hash")?;
    let receipt: PaymentReceipt =
        serde_json::from_slice(&read_bounded(&cli.receipt, MAX_RECEIPT_BYTES)?)
            .context("invalid receipt JSON")?;
    let block: Block = serde_json::from_slice(&read_bounded(&cli.block, MAX_BLOCK_JSON_BYTES)?)
        .context("invalid Block JSON (extract one element from get_blocks.result)")?;

    let evidence = verify_anchored(&receipt, &block, anchor)?;
    println!("RECEIPT + ANCHORED BLOCK BODY: VERIFIED");
    println!("receipt version .... {}", evidence.receipt.version);
    println!("kind ............... {}", evidence.receipt.kind.as_str());
    println!(
        "opened amount ...... {} darks",
        evidence.receipt.amount_darks
    );
    println!("block height ....... {}", evidence.receipt.height);
    println!("block hash ......... {}", evidence.block_hash);
    println!("output index ....... {}", evidence.output_index);
    println!();
    println!("Established: signature, commitment opening, canonical body,");
    println!("header/body binding and matching output in the specified block.");
    println!("NOT established: independent validity of the hash anchor, PoW,");
    println!("canonical-chain membership, confirmations, finality, or unspentness.");
    println!("Never derive the independent anchor from the same untrusted block file.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_consensus::BlockHeader;
    use nightfall_crypto::{generator_g, hash_multi, scan_output, Commitment, WalletKeys};
    use nightfall_ledger::{build_coinbase, BlockBody};
    use nightfall_types::{Height, NetworkId, DARKS_PER_NIGHT, PROTOCOL_VERSION};

    // Exercise v2 signing independently of the wallet's private test helpers.
    fn sign_v2(keys: &WalletKeys, receipt: &mut PaymentReceipt) {
        let msg = hash_multi(
            b"nightfall:receipt:v2",
            &[
                receipt.kind.as_bytes(),
                receipt.address.as_bytes(),
                &receipt.amount_darks.to_le_bytes(),
                receipt.commit.as_bytes(),
                receipt.blind.as_bytes(),
                receipt.memo.as_bytes(),
                &receipt.height.to_le_bytes(),
                &receipt.timestamp.to_le_bytes(),
            ],
        )
        .0;
        let signature = nightfall_crypto::sig::sign(&keys.spend_secret(), &generator_g(), &msg);
        receipt.sig_r = hex::encode(signature.r);
        receipt.sig_s = hex::encode(signature.s);
    }

    fn fixture() -> (PaymentReceipt, Block) {
        let keys = WalletKeys::from_seed([41u8; 32]); // TEST ONLY, never funded.
        let height = 4u64;
        let reward = 6 * DARKS_PER_NIGHT;
        let coinbase = build_coinbase(
            &keys.address(),
            reward,
            height,
            NetworkId::Devnet.proof_context(),
        )
        .unwrap();
        let opened = scan_output(&keys.view_key(), &coinbase.outputs[0]).unwrap();
        let body = BlockBody::aggregate(&[coinbase]);
        let mut receipt = PaymentReceipt {
            v: 2,
            kind: "mined".to_owned(),
            address: keys.address().encode(),
            amount_darks: opened.value,
            memo: String::new(),
            commit: hex::encode(opened.commit.0),
            blind: hex::encode(opened.blind.to_bytes()),
            height,
            timestamp: 0,
            sig_r: String::new(),
            sig_s: String::new(),
        };
        sign_v2(&keys, &mut receipt);
        let block = Block {
            header: BlockHeader {
                version: PROTOCOL_VERSION,
                height: Height(height),
                prev_hash: Hash256::ZERO,
                utxo_root: Hash256::ZERO,
                kernel_sum: Commitment([0u8; 32]),
                body_root: body.hash(),
                timestamp_unix: 0,
                difficulty: 1,
                nonce: 0,
                reward_darks: reward,
            },
            body,
        };
        (receipt, block)
    }

    #[test]
    fn verified_anchored_receipt_and_mandatory_anchor() {
        let (receipt, block) = fixture();
        let proof = verify_anchored(&receipt, &block, block.hash()).unwrap();
        assert_eq!(proof.receipt.amount_darks, 6 * DARKS_PER_NIGHT);
        assert_eq!(proof.output_index, 0);
        assert!(proof.receipt.amount_proven);
        assert!(verify_anchored(&receipt, &block, Hash256::ZERO).is_err());
    }

    #[test]
    fn tampered_body_wrong_height_and_wrong_output_fail_closed() {
        let (receipt, block) = fixture();
        let mut tampered = block.clone();
        tampered.body.outputs.clear();
        assert!(verify_anchored(&receipt, &tampered, tampered.hash()).is_err());
        tampered.header.body_root = tampered.body.hash();
        assert!(verify_anchored(&receipt, &tampered, tampered.hash()).is_err());

        let mut duplicate = block.clone();
        duplicate
            .body
            .outputs
            .push(duplicate.body.outputs[0].clone());
        duplicate.header.body_root = duplicate.body.hash();
        assert!(verify_anchored(&receipt, &duplicate, duplicate.hash()).is_err());

        let mut wrong_height = block;
        wrong_height.header.height.0 += 1;
        assert!(verify_anchored(&receipt, &wrong_height, wrong_height.hash()).is_err());
    }

    #[test]
    fn unsigned_legacy_kind_sent_claim_and_bad_opening_are_rejected() {
        let (receipt, block) = fixture();
        let anchor = block.hash();

        let mut legacy = receipt.clone();
        legacy.v = 1;
        assert!(verify_anchored(&legacy, &block, anchor).is_err());

        let mut sent = receipt.clone();
        sent.kind = "sent".into();
        assert!(verify_anchored(&sent, &block, anchor).is_err());

        let mut fabricated = receipt;
        fabricated.amount_darks += 1;
        assert!(verify_anchored(&fabricated, &block, anchor).is_err());
    }

    #[test]
    fn bounded_reader_refuses_large_inputs() {
        let dir = std::env::temp_dir().join(format!(
            "nightfall-receipt-evidence-test-{}",
            std::process::id()
        ));
        // Keep the test deterministic and avoid touching any wallet datadir.
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("oversized.json");
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        assert!(read_bounded(&path, 99).is_err());
        assert_eq!(read_bounded(&path, 100).unwrap().len(), 100);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
