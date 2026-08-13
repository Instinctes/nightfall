//! What a view tag actually saves, measured rather than asserted.
//!
//!     cargo run --release -p nightfall-crypto --example scanbench
//!
//! Scanning cost is the thing that decides whether a phone wallet is possible.
//! There is no index from an address to its outputs — that absence *is* the
//! privacy property — so a wallet tests every output on chain against its view
//! key.
//!
//! Two costs matter, and they are very different:
//!
//! - **A stranger's output.** The overwhelming majority. With a view tag the
//!   scanner computes `Ke·a`, hashes it, compares one byte, and stops.
//! - **Your own output.** Rare. Every step runs: both scalar multiplications,
//!   the point compression, the AEAD open, and the commitment check.
//!
//! To measure what the tag saves *honestly*, the pre-tag path is reproduced
//! exactly: a foreign output is given the tag byte this view key would have
//! derived for it, so the scan runs past the first gate and fails where it used
//! to — at the one-time key comparison. That is the real "before" cost. It is
//! not the same as the cost of an owned output, which additionally opens the
//! AEAD payload and re-derives the commitment; a stranger never reached those
//! steps even without a tag, and counting them would inflate the result.

use nightfall_crypto::{create_output, scan_output, WalletKeys};
use nightfall_types::NetworkId;
use std::time::Instant;

fn main() {
    let ctx = NetworkId::Mainnet.proof_context();
    let me = WalletKeys::generate();
    let view = me.view_key();

    // Building outputs is dominated by Bulletproof generation, which is slow
    // and has nothing to do with what is being measured, so the sample is kept
    // small and reused.
    const STRANGERS: usize = 400;
    const MINE: usize = 100;
    const REPEATS: usize = 20;

    eprint!("building {STRANGERS} foreign and {MINE} owned outputs… ");
    let foreign: Vec<_> = (0..STRANGERS)
        .map(|_| {
            let other = WalletKeys::generate();
            create_output(&other.address(), 1, "", ctx)
                .expect("output")
                .0
        })
        .collect();
    let owned: Vec<_> = (0..MINE)
        .map(|i| {
            create_output(&me.address(), 1_000 + i as u64, "", ctx)
                .expect("output")
                .0
        })
        .collect();
    eprintln!("done\n");

    let start = Instant::now();
    let mut hits = 0usize;
    for _ in 0..REPEATS {
        for o in &foreign {
            if scan_output(&view, o).is_some() {
                hits += 1;
            }
        }
    }
    let foreign_ns = start.elapsed().as_nanos() as f64 / (REPEATS * STRANGERS) as f64;
    assert_eq!(hits, 0, "a foreign output was claimed");

    // "Before": same foreign outputs, tag forged to match, so the scan spends
    // the second scalar multiplication and the compression before rejecting on
    // the one-time key — exactly what every stranger cost prior to the tag.
    let pre_tag: Vec<_> = foreign
        .iter()
        .map(|o| {
            let mut o = o.clone();
            o.view_tag = view
                .expected_view_tag(&o.ephemeral_pk)
                .expect("ephemeral key decompresses");
            o
        })
        .collect();

    let start = Instant::now();
    for _ in 0..REPEATS {
        for o in &pre_tag {
            assert!(scan_output(&view, o).is_none(), "claimed a foreign output");
        }
    }
    let pre_tag_ns = start.elapsed().as_nanos() as f64 / (REPEATS * STRANGERS) as f64;

    let start = Instant::now();
    let mut found = 0usize;
    for _ in 0..REPEATS {
        for o in &owned {
            if scan_output(&view, o).is_some() {
                found += 1;
            }
        }
    }
    let owned_ns = start.elapsed().as_nanos() as f64 / (REPEATS * MINE) as f64;
    assert_eq!(found, REPEATS * MINE, "an owned output was missed");

    println!("per foreign output — this is what dominates a scan");
    println!("  before view tags              {pre_tag_ns:8.0} ns");
    println!("  with view tags                {foreign_ns:8.0} ns");
    println!(
        "  saving                        {:8.2}x",
        pre_tag_ns / foreign_ns
    );
    println!();
    println!("for reference, an output you own  {owned_ns:8.0} ns");
    println!("  (adds the AEAD open and the commitment check; rare by definition)");
    println!();

    // Real chains are almost entirely other people's money.
    for outputs in [100_000u64, 1_000_000, 10_000_000] {
        let with = outputs as f64 * foreign_ns / 1e9;
        let without = outputs as f64 * pre_tag_ns / 1e9;
        println!(
            "{outputs:>10} outputs   with tag {with:7.1} s   without {without:7.1} s   \
             saved {:5.1} s",
            without - with
        );
    }

    println!(
        "\nA phone is roughly 3-4x slower than this machine. The point of the\n\
         birth height is that a new wallet scans none of it."
    );
}
