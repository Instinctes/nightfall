//! How a block is written to disk, and how it is read back.
//!
//! The chain file was JSON, one block per line, and that cost more than it
//! looked. `serde_json` renders a `[u8; 32]` as an array of decimal numbers —
//! `[241,118,175,…]` — so every byte of every hash, commitment, signature and
//! range proof becomes about 3.6 characters. Measured on the live mainnet
//! file: 31,288 blocks, 152.3 MiB, **5.10 KiB per block** with a single output
//! and a single kernel in it. Extrapolated over a year of 15-second blocks
//! that is 10.7 GB of an *empty* chain, before anybody sends anything.
//!
//! The same chain in the compact binary encoding is 42.2 MiB: 1.41 KiB per
//! block, **72.3 % less disk**, converted in about a second. Identical tip,
//! identical UTXO root, supply proof unchanged.
//!
//! Two things this deliberately is not:
//!
//! * **Not a consensus change.** Block hashes are computed over raw field
//!   bytes, never over the serialised form, so how a node stores its file
//!   cannot change what the network agrees on. A binary node and a JSON node
//!   hold the identical chain.
//! * **Not a wire change.** This is the on-disk file only. P2P messages stay
//!   newline-delimited JSON, so a node that converts its storage still speaks
//!   to every other node exactly as before.
//!
//! Nothing converts by itself. `nightfalld migrate-storage` does it on
//! request, verifies the result before swapping, and keeps the old file. A
//! node reads whichever format it finds.

use nightfall_consensus::Block;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

/// Binary chain file. Records are `u32` little-endian length, then bincode.
pub const BLOCKS_BIN: &str = "blocks.bin";
/// Historic chain file: one JSON object per line.
pub const BLOCKS_JSONL: &str = "blocks.jsonl";

/// A single record may not claim to be larger than this.
///
/// A truncated or corrupt length prefix otherwise asks for a multi-gigabyte
/// allocation before anything notices the file is broken. Blocks are capped
/// far below this by `MAX_TXS_PER_BLOCK`; the limit exists for the failure
/// case, not the normal one.
const MAX_RECORD_BYTES: u32 = 8 * 1024 * 1024;

/// Prefix of every binary record written by a build that knows the binding
/// fields.
///
/// bincode is not self-describing: the binding fields change the layout of
/// inputs, kernels and bodies, so a 1.0.x record cannot be read as the new
/// type. A 1.0.x record always starts with the header's protocol version,
/// `08 00 00 00`; this prefix starts with `0xFF` and can never be one.
pub const RECORD_V2_MAGIC: [u8; 8] = *b"\xFFNFBLK\x02\x00";

fn decode_binary_record(buf: &[u8]) -> anyhow::Result<Block> {
    if let Some(rest) = buf.strip_prefix(&RECORD_V2_MAGIC) {
        return Ok(bincode::deserialize(rest)?);
    }
    let legacy: legacy::Block = bincode::deserialize(buf)?;
    Ok(legacy.into())
}

/// The 1.0.x binary layout, kept only to read files written before the
/// binding fields existed.
mod legacy {
    use nightfall_consensus::BlockHeader;
    use nightfall_crypto::{Commitment, KernelFeature, Output, SchnorrSig, TxKernel};
    use nightfall_ledger::{BlockBody, Input};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct Block {
        header: BlockHeader,
        body: Body,
    }

    impl Block {
        /// The 1.0.x shape of `b`. Only for blocks without binding fields.
        pub fn from_current(b: &nightfall_consensus::Block) -> Self {
            Self {
                header: b.header.clone(),
                body: Body {
                    inputs: b
                        .body
                        .inputs
                        .iter()
                        .map(|i| LegacyInput {
                            commit: i.commit,
                            sig: i.sig,
                        })
                        .collect(),
                    outputs: b.body.outputs.clone(),
                    kernels: b
                        .body
                        .kernels
                        .iter()
                        .map(|k| LegacyKernel {
                            feature: k.feature,
                            fee_darks: k.fee_darks,
                            reward_darks: k.reward_darks,
                            lock_height: k.lock_height,
                            excess: k.excess,
                            excess_sig: k.excess_sig,
                        })
                        .collect(),
                },
            }
        }
    }

    #[derive(Serialize, Deserialize)]
    struct Body {
        inputs: Vec<LegacyInput>,
        outputs: Vec<Output>,
        kernels: Vec<LegacyKernel>,
    }

    #[derive(Serialize, Deserialize)]
    struct LegacyInput {
        commit: Commitment,
        sig: SchnorrSig,
    }

    #[derive(Serialize, Deserialize)]
    struct LegacyKernel {
        feature: KernelFeature,
        fee_darks: u64,
        reward_darks: u64,
        lock_height: u64,
        excess: Commitment,
        excess_sig: SchnorrSig,
    }

    impl From<Block> for nightfall_consensus::Block {
        fn from(b: Block) -> Self {
            nightfall_consensus::Block {
                header: b.header,
                body: BlockBody {
                    inputs: b
                        .body
                        .inputs
                        .into_iter()
                        .map(|i| Input::legacy(i.commit, i.sig))
                        .collect(),
                    outputs: b.body.outputs,
                    kernels: b
                        .body
                        .kernels
                        .into_iter()
                        .map(|k| TxKernel {
                            feature: k.feature,
                            fee_darks: k.fee_darks,
                            reward_darks: k.reward_darks,
                            lock_height: k.lock_height,
                            excess: k.excess,
                            excess_sig: k.excess_sig,
                            stealth_excess: None,
                            stealth_sig: None,
                        })
                        .collect(),
                    stealth_offset: None,
                    kernel_offset: None,
                },
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Json,
    Binary,
}

impl Format {
    pub fn file_name(self) -> &'static str {
        match self {
            Format::Json => BLOCKS_JSONL,
            Format::Binary => BLOCKS_BIN,
        }
    }
}

/// Which chain file this directory actually has.
///
/// Binary wins when both exist, which is the state a conversion leaves behind
/// on purpose: the old file stays until the operator deletes it.
///
/// An **empty** datadir gets binary. A node installed today should not spend
/// its first year writing 10.7 GB of decimal digits and then be asked to
/// convert; there is no history to be compatible with yet. An existing
/// `blocks.jsonl` still wins over the default, so nothing already on disk
/// changes format by itself.
///
/// The cost of that default is a one-way door for the *software*, not the
/// chain: a datadir written by this version cannot be read by a nightfalld
/// older than the binary format, which would find no `blocks.jsonl` and
/// resync from genesis. The chain is unaffected either way.
pub fn detect(dir: &Path) -> Format {
    if dir.join(BLOCKS_BIN).exists() {
        Format::Binary
    } else if dir.join(BLOCKS_JSONL).exists() {
        Format::Json
    } else {
        Format::Binary
    }
}

pub fn write_block<W: Write>(w: &mut W, block: &Block, fmt: Format) -> anyhow::Result<()> {
    match fmt {
        Format::Json => {
            serde_json::to_writer(&mut *w, block)?;
            w.write_all(b"\n")?;
        }
        Format::Binary => {
            // Blocks without binding fields — every block below the fork —
            // keep the 1.0.x layout byte for byte, so a datadir or bootstrap
            // archive written now still loads in 1.0.x until the fork.
            let bytes = if block.body.has_any_binding() {
                let mut v = RECORD_V2_MAGIC.to_vec();
                v.extend_from_slice(&bincode::serialize(block)?);
                v
            } else {
                bincode::serialize(&legacy::Block::from_current(block))?
            };
            let len = u32::try_from(bytes.len())
                .map_err(|_| anyhow::anyhow!("block is absurdly large: {} bytes", bytes.len()))?;
            if len > MAX_RECORD_BYTES {
                anyhow::bail!("block of {len} bytes exceeds the record limit");
            }
            w.write_all(&len.to_le_bytes())?;
            w.write_all(&bytes)?;
        }
    }
    Ok(())
}

/// Read every block in the file, in order.
///
/// Errors name the record index, because "block 24,193 is corrupt" is a
/// sentence somebody can act on and "invalid data" is not.
pub fn read_blocks<R: Read>(r: R, fmt: Format) -> anyhow::Result<Vec<Block>> {
    let mut out = Vec::new();
    match fmt {
        Format::Json => {
            for (i, line) in BufReader::new(r).lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                out.push(
                    serde_json::from_str(&line)
                        .map_err(|e| anyhow::anyhow!("block {i} is corrupt: {e}"))?,
                );
            }
        }
        Format::Binary => {
            let mut r = BufReader::new(r);
            let mut len_buf = [0u8; 4];
            let mut i = 0usize;
            let mut first = true;
            loop {
                // EOF is valid only between complete records. Once any byte
                // of the next four-byte length prefix exists, losing the
                // remainder means the file is truncated.
                if r.fill_buf()?.is_empty() {
                    break;
                }
                r.read_exact(&mut len_buf).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        anyhow::anyhow!("block {i} has a truncated length prefix")
                    } else {
                        anyhow::Error::from(e)
                    }
                })?;
                // A JSON chain file in a binary datadir reads its first two
                // characters as part of a length prefix and reports an absurd
                // number, which tells the operator nothing. Name it instead:
                // this happened in the wild, and the fix is a rename.
                if first && &len_buf[..2] == b"{\"" {
                    anyhow::bail!(
                        "this file holds JSON, not the binary format — \
                         rename it to {BLOCKS_JSONL} and the node will read it"
                    );
                }
                first = false;

                let len = u32::from_le_bytes(len_buf);
                if len == 0 || len > MAX_RECORD_BYTES {
                    anyhow::bail!("block {i} claims {len} bytes — the file is corrupt");
                }
                let mut buf = vec![0u8; len as usize];
                r.read_exact(&mut buf).map_err(|e| {
                    anyhow::anyhow!("block {i} is truncated after {len} declared bytes: {e}")
                })?;
                out.push(
                    decode_binary_record(&buf)
                        .map_err(|e| anyhow::anyhow!("block {i} is corrupt: {e}"))?,
                );
                i += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_consensus::Chain;
    use nightfall_crypto::WalletKeys;
    use nightfall_types::NetworkId;

    fn sample_chain() -> Chain {
        let mut c = Chain::new_fair(NetworkId::Devnet).unwrap();
        let miner = WalletKeys::generate().address();
        for i in 0..4u64 {
            c.mine_block(&miner, vec![], 1_800_000_000 + i * 15)
                .unwrap();
        }
        c
    }

    /// Pre-fork blocks keep the 1.0.x byte layout; post-fork blocks carry the
    /// v2 prefix; both read back to the identical chain.
    #[test]
    fn records_switch_layout_exactly_at_the_binding_fork() {
        let mut c = Chain::new_fair(NetworkId::Devnet).unwrap();
        let miner = WalletKeys::generate().address();
        let fork = NetworkId::Devnet.binding_fork_height();
        for i in 0..fork + 3 {
            c.mine_block(&miner, vec![], 1_800_000_000 + i * 15)
                .unwrap();
        }
        let mut buf = Vec::new();
        for b in &c.blocks {
            let mut one = Vec::new();
            write_block(&mut one, b, Format::Binary).unwrap();
            let body = &one[4..];
            if b.header.height.0 < fork {
                assert_eq!(&body[..4], &8u32.to_le_bytes(), "1.0.x layout");
                assert_eq!(
                    one[4..],
                    bincode::serialize(&legacy::Block::from_current(b)).unwrap()[..]
                );
            } else {
                assert_eq!(&body[..8], &RECORD_V2_MAGIC);
            }
            buf.extend_from_slice(&one);
        }
        let back = read_blocks(&buf[..], Format::Binary).unwrap();
        assert_eq!(back, c.blocks);
    }

    /// The property the whole change rests on: the format is a container, not
    /// a meaning. Both files must produce the identical chain, hash for hash.
    #[test]
    fn both_formats_round_trip_to_the_same_blocks() {
        let chain = sample_chain();

        for fmt in [Format::Json, Format::Binary] {
            let mut buf = Vec::new();
            for b in &chain.blocks {
                write_block(&mut buf, b, fmt).unwrap();
            }
            let back = read_blocks(&buf[..], fmt).unwrap();

            assert_eq!(back.len(), chain.blocks.len(), "{fmt:?} lost blocks");
            for (a, b) in chain.blocks.iter().zip(&back) {
                assert_eq!(a.hash(), b.hash(), "{fmt:?} changed a block hash");
            }
        }
    }

    #[test]
    fn binary_is_substantially_smaller() {
        let chain = sample_chain();
        let mut j = Vec::new();
        let mut b = Vec::new();
        for blk in &chain.blocks {
            write_block(&mut j, blk, Format::Json).unwrap();
            write_block(&mut b, blk, Format::Binary).unwrap();
        }
        // Measured on mainnet the saving is ~71 %. Devnet blocks carry the
        // same proofs, so anything under half is a sign the encoding regressed
        // to something verbose.
        assert!(
            b.len() * 2 < j.len(),
            "binary {} B is not meaningfully smaller than json {} B",
            b.len(),
            j.len()
        );
    }

    #[test]
    fn a_truncated_binary_file_is_refused_not_guessed() {
        let chain = sample_chain();
        let mut buf = Vec::new();
        for blk in &chain.blocks {
            write_block(&mut buf, blk, Format::Binary).unwrap();
        }
        buf.truncate(buf.len() - 40);
        let err = read_blocks(&buf[..], Format::Binary)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("truncated"),
            "a short file must say so, got: {err}"
        );
    }

    #[test]
    fn a_partial_binary_length_prefix_is_refused() {
        let chain = sample_chain();
        let mut complete = Vec::new();
        for block in &chain.blocks {
            write_block(&mut complete, block, Format::Binary).unwrap();
        }

        let next_len = 1u32.to_le_bytes();
        for prefix_len in 1..=3 {
            let mut buf = complete.clone();
            buf.extend_from_slice(&next_len[..prefix_len]);

            let err = read_blocks(&buf[..], Format::Binary)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("truncated") && err.contains("length prefix"),
                "{prefix_len}-byte partial prefix returned the wrong error: {err}"
            );
        }
    }

    #[test]
    fn a_nonsense_length_prefix_does_not_allocate_the_world() {
        // Four bytes of 0xFF claim four gigabytes. The reader must refuse on
        // the number rather than try to honour it.
        let bad = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let err = read_blocks(&bad[..], Format::Binary)
            .unwrap_err()
            .to_string();
        assert!(err.contains("corrupt"), "got: {err}");
    }

    /// A datadir with nothing in it is a new install, and a new install has no
    /// history to stay compatible with. It should not start out writing the
    /// format we just spent a release converting away from.
    #[test]
    fn an_empty_datadir_starts_out_binary() {
        let dir = std::env::temp_dir().join(format!("nf-detect-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect(&dir), Format::Binary);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The other half of that: a datadir that already holds a JSON chain keeps
    /// writing JSON. Switching format underneath an existing file would append
    /// bincode records to a text file and corrupt it.
    #[test]
    fn an_existing_json_chain_keeps_its_format() {
        let dir = std::env::temp_dir().join(format!("nf-detect-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(BLOCKS_JSONL), b"{}\n").unwrap();
        assert_eq!(detect(&dir), Format::Json);

        // …until a conversion puts blocks.bin next to it, at which point the
        // new file wins and the old one is only a fallback the operator keeps.
        std::fs::write(dir.join(BLOCKS_BIN), b"").unwrap();
        assert_eq!(detect(&dir), Format::Binary);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The failure that actually happened: a bug wrote JSON into a file named
    /// blocks.bin, and the reader answered "block 0 claims 1701323387 bytes",
    /// which sounds like disk corruption and is not. Say what it is.
    #[test]
    fn json_in_a_binary_file_names_the_problem() {
        let chain = sample_chain();
        let mut buf = Vec::new();
        for b in &chain.blocks {
            write_block(&mut buf, b, Format::Json).unwrap();
        }
        let err = read_blocks(&buf[..], Format::Binary)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("JSON") && err.contains(BLOCKS_JSONL),
            "the error must tell the operator to rename the file, got: {err}"
        );
    }
}
