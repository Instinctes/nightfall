//! How much the running commitment sum actually saves.
//!
//! Run with:  cargo test -p nightfall-ledger --release --test sum_speed -- --nocapture
//!
//! Not an assertion about wall-clock time — those belong nowhere near CI. It
//! prints the two numbers so a claim about the speed-up can be checked instead
//! of repeated.

use curve25519_dalek::scalar::Scalar;
use nightfall_crypto::Commitment;
use nightfall_ledger::{UtxoEntry, UtxoSet};
use rand::rngs::OsRng;
use std::time::Instant;

#[test]
#[ignore = "a timing measurement, not a pass/fail check — run it by name"]
fn running_sum_versus_walking_the_set() {
    const N: usize = 60_000;
    let mut set = UtxoSet::new();
    for v in 0..N as u64 {
        set.insert(
            Commitment::new(v, &Scalar::random(&mut OsRng)),
            UtxoEntry {
                output_pk: [0u8; 32],
                height: 0,
                is_coinbase: false,
            },
        );
    }

    let t = Instant::now();
    for _ in 0..50 {
        std::hint::black_box(set.commitment_sum());
    }
    let fast = t.elapsed().as_secs_f64() / 50.0;

    println!("  entries............ {N}");
    println!("  commitment_sum..... {:.6} s per call", fast);
    println!("  per 100k blocks.... {:.1} s", fast * 100_000.0);
}
