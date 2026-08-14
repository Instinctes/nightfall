//! How long a full-chain reorg holds the node's state lock.
//!
//! Loads a real `blocks.jsonl` and times `rebuild_from_blocks`, which is what
//! `maybe_reorg_to` runs — with the global mutex held — every time a peer's
//! blocks fail to connect.
//!
//!   cargo run --release -p nightfall-node --example reorgcost -- <blocks.jsonl>

use nightfall_consensus::Block;
use std::io::BufRead;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Library/Application Support/nightfall/mainnet/blocks.jsonl")
    });

    let file = std::fs::File::open(&path)?;
    let mut blocks: Vec<Block> = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        blocks.push(serde_json::from_str(&line)?);
    }
    println!("loaded {} blocks from {path}", blocks.len());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let start = Instant::now();
    let chain = nightfall_consensus::Chain::rebuild_from_blocks(
        nightfall_types::NetworkId::Mainnet,
        blocks.clone(),
        now,
    )?;
    let elapsed = start.elapsed();

    println!(
        "rebuild_from_blocks: {:.2} s for {} blocks ({:.1} ms/block)",
        elapsed.as_secs_f64(),
        chain.block_count(),
        elapsed.as_secs_f64() * 1000.0 / blocks.len() as f64,
    );
    println!(
        "\nThe node holds its global state mutex for this entire duration.\n\
         No RPC, no GUI frame, no peer thread and no block submission runs\n\
         while it does — and every peer thread that fails to connect a block\n\
         starts its own copy of it."
    );
    Ok(())
}
