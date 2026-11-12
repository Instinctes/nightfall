//! Print a checkpoint line for CHECKPOINTS, computed with the real hasher.
//!
//!   cargo run --release -p nightfall-node --example checkpoint -- <height> [blocks.bin|blocks.jsonl]
//!
//! Reads the chain file and nothing else. Run it on a copy if the node that
//! owns the file is running: a block being appended at that moment would read
//! as a truncated record.

use nightfall_consensus::Block;
use nightfall_storage::codec::{self, Format};
use std::io::BufRead;
use std::path::PathBuf;

fn default_path() -> PathBuf {
    let dir = nightfall_storage::default_data_dir(nightfall_types::NetworkId::Mainnet);
    dir.join(codec::detect(&dir).file_name())
}

fn print_if(height: u64, b: &Block) {
    if b.header.height.0 == height {
        println!("    ({}, \"{}\"),", height, b.hash().to_hex());
        eprintln!(
            "utxo_root {}  prev {}",
            b.header.utxo_root.to_hex(),
            b.header.prev_hash.to_hex()
        );
    }
}

fn main() -> anyhow::Result<()> {
    let height: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(25_000);
    let path = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(default_path);

    let f = std::fs::File::open(&path)?;
    let mut tip = 0u64;
    if path.extension().and_then(|e| e.to_str()) == Some("bin") {
        for b in codec::read_blocks(f, Format::Binary)? {
            tip = b.header.height.0;
            print_if(height, &b);
        }
    } else {
        for line in std::io::BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let b: Block = serde_json::from_str(&line)?;
            tip = b.header.height.0;
            print_if(height, &b);
        }
    }
    eprintln!("tip in file: {tip}");
    Ok(())
}
