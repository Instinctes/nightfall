//! Time a cold load of a datadir, the way a node does it at start-up.
//!
//!   cargo run --release -p nightfall-storage --example load_time -- <datadir> [seconds]
//!
//! With `seconds`, stops after that long and reports the rate so far, which
//! is how to compare against a load that would take an hour to finish.
//! Reads the datadir; does not write a validation record.

use nightfall_storage::ChainStore;
use nightfall_types::NetworkId;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).expect("datadir");
    let limit = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs);

    let start = Instant::now();
    let mut first_progress: Option<(Instant, u64)> = None;
    let mut last_print = Instant::now();
    let store = ChainStore::new(&dir);
    let chain = store.load_or_new_with_progress(NetworkId::Mainnet, |applied, total| {
        let (t0, a0) = *first_progress.get_or_insert((Instant::now(), applied));
        if last_print.elapsed() >= Duration::from_secs(1) {
            last_print = Instant::now();
            eprintln!(
                "{:>7.1}s  {applied} / {total}",
                start.elapsed().as_secs_f64()
            );
        }
        if let Some(limit) = limit {
            if start.elapsed() >= limit {
                let secs = t0.elapsed().as_secs_f64();
                let rate = (applied - a0) as f64 / secs;
                println!(
                    "stopped after {:.1}s: {applied} / {total} blocks, {rate:.0} blocks/s while validating",
                    start.elapsed().as_secs_f64()
                );
                std::process::exit(0);
            }
        }
    })?;
    println!(
        "loaded {} blocks in {:.1}s, tip {}",
        chain.block_count(),
        start.elapsed().as_secs_f64(),
        chain.tip_hash().to_hex()
    );
    Ok(())
}
