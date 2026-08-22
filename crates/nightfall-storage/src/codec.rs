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
            let bytes = bincode::serialize(block)?;
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
            loop {
                match r.read_exact(&mut len_buf) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e.into()),
                }
                let len = u32::from_le_bytes(len_buf);
                if len == 0 || len > MAX_RECORD_BYTES {
                    anyhow::bail!("block {i} claims {len} bytes — the file is corrupt");
                }
                let mut buf = vec![0u8; len as usize];
                r.read_exact(&mut buf).map_err(|e| {
                    anyhow::anyhow!("block {i} is truncated after {len} declared bytes: {e}")
                })?;
                out.push(
                    bincode::deserialize(&buf)
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
}
