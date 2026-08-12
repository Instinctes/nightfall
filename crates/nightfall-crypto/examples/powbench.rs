fn main() {
    use nightfall_crypto::*;
    use nightfall_types::NetworkId;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    for net in [NetworkId::Devnet, NetworkId::Testnet, NetworkId::Mainnet] {
        let params = net.pow_params();
        // single-hash verification cost
        let t = Instant::now();
        for i in 0..20u64 {
            let _ = nighthash(b"header-preimage-32-bytes-long!!!", i, params);
        }
        let per_verify = t.elapsed().as_secs_f64() / 20.0;

        // parallel hashrate
        let threads = default_threads();
        let counter = AtomicU64::new(0);
        let start = Instant::now();
        let stop = move || start.elapsed().as_secs_f64() > 3.0;
        let _ = mine_parallel(
            b"bench",
            u64::MAX,
            0,
            params,
            threads,
            &stop,
            Some(&counter),
        );
        let elapsed = start.elapsed().as_secs_f64();
        let hs = counter.load(Ordering::Relaxed) as f64 / elapsed;

        println!(
            "{:<8} mem={:>5} MiB  verify={:>6.2} ms  {:>7.1} H/s on {threads} threads  mem_total={} MiB",
            net.as_str(),
            params.memory_kib / 1024,
            per_verify * 1000.0,
            hs,
            mining_memory_bytes(params, threads) / (1024*1024)
        );
        println!(
            "         -> difficulty for 15s blocks on THIS machine: {:.0}",
            hs * 15.0
        );
    }
}
