//! Mainnet Nighthash-v2 on this Mac: GPU alone, CPU alone, both at once.
//!
//!   cargo run --release -p nightfall-gpu --example gpubench [batch ...]
use nightfall_crypto::PowHasher;
use nightfall_gpu::GpuHasher;
use nightfall_types::NetworkId;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SECS: u64 = 12;

fn gpu_rate(batch: usize, stop: Option<&AtomicBool>) -> (f64, usize) {
    let params = NetworkId::Mainnet.pow_params();
    let mut gpu = GpuHasher::new(params, batch).expect("Metal");
    let pre = vec![7u8; 180];
    let mut nonce = 0u64;
    gpu.hash_batch(&pre, &[0]); // warm-up: shader + first touch
    let t = Instant::now();
    let mut done = 0u64;
    while t.elapsed() < Duration::from_secs(SECS)
        && !stop.is_some_and(|s| s.load(Ordering::Relaxed))
    {
        let nonces: Vec<u64> = (nonce..nonce + gpu.batch() as u64).collect();
        nonce += gpu.batch() as u64;
        done += gpu.hash_batch(&pre, &nonces).len() as u64;
    }
    (done as f64 / t.elapsed().as_secs_f64(), gpu.batch())
}

fn cpu_rate(threads: usize) -> f64 {
    let params = NetworkId::Mainnet.pow_params();
    let count = AtomicU64::new(0);
    let t = Instant::now();
    std::thread::scope(|s| {
        for w in 0..threads {
            let count = &count;
            s.spawn(move || {
                let mut h = PowHasher::new(params);
                let pre = vec![7u8; 180];
                let mut n = 1u64 << 40 | w as u64;
                while t.elapsed() < Duration::from_secs(SECS) {
                    h.hash(&pre, n);
                    n += threads as u64;
                    count.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    count.load(Ordering::Relaxed) as f64 / t.elapsed().as_secs_f64()
}

fn main() {
    let batches: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let batches = if batches.is_empty() {
        vec![16, 32, 64, 128]
    } else {
        batches
    };
    let threads = nightfall_crypto::default_threads();
    let probe = GpuHasher::new(NetworkId::Mainnet.pow_params(), 1).expect("Metal");
    println!(
        "device: {}   cpu mining threads: {threads}   mainnet 32 MiB/hash",
        probe.device_name()
    );
    drop(probe);

    let mut best = (0.0, 0);
    for b in batches {
        let (r, used) = gpu_rate(b, None);
        println!("GPU  batch {used:>4} ({:>5} MiB): {r:>8.1} H/s", used * 32);
        if r > best.0 {
            best = (r, used);
        }
    }
    let cpu = cpu_rate(threads);
    println!("CPU  {threads} threads:              {cpu:>8.1} H/s");

    let stop = AtomicBool::new(false);
    let (g, c) = std::thread::scope(|s| {
        let g = s.spawn(|| gpu_rate(best.1, Some(&stop)).0);
        let c = cpu_rate(threads);
        stop.store(true, Ordering::Relaxed);
        (g.join().unwrap(), c)
    });
    println!(
        "BOTH GPU batch {} + CPU {threads}: {:>8.1} H/s  (GPU {g:.1} + CPU {c:.1})",
        best.1,
        g + c
    );
}
