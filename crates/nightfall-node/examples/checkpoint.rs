//! Print a checkpoint line for CHECKPOINTS, computed with the real hasher.
//!
//!   cargo run --release -p nightfall-node --example checkpoint -- <height> [blocks.jsonl]

use nightfall_consensus::Block;
use std::io::BufRead;

fn main() -> anyhow::Result<()> {
    let height: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(25_000);
    let path = std::env::args().nth(2).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Library/Application Support/nightfall/mainnet/n8/blocks.jsonl")
    });

    let f = std::fs::File::open(&path)?;
    let mut tip = 0u64;
    for line in std::io::BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let b: Block = serde_json::from_str(&line)?;
        tip = b.header.height.0;
        if tip == height {
            println!("    ({}, \"{}\"),", height, b.hash().to_hex());
        }
    }
    eprintln!("tip in file: {tip}");
    Ok(())
}
