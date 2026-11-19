//! The GPU must produce exactly the consensus hash, for every nonce.
#![cfg(target_os = "macos")]

use nightfall_crypto::nighthash;
use nightfall_gpu::GpuHasher;
use nightfall_types::NetworkId;

fn header(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn check(network: NetworkId, batch: usize, rounds: u64) {
    let params = network.pow_params();
    // CI machines usually have no usable Metal device. The comparison only
    // means something on real hardware, so it is skipped, loudly, elsewhere.
    let mut gpu = match GpuHasher::new(params, batch) {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("skipped: no usable Metal device ({e})");
            return;
        }
    };
    for r in 0..rounds {
        let pre = header(r as u8 * 7 + 1, 32 + r as usize * 13);
        let nonces: Vec<u64> = (0..gpu.batch() as u64)
            .map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(r))
            .collect();
        let got = gpu.hash_batch(&pre, &nonces);
        for (nonce, h) in nonces.iter().zip(got) {
            assert_eq!(
                h,
                nighthash(&pre, *nonce, params),
                "{network:?} nonce {nonce} round {r}"
            );
        }
    }
}

#[test]
fn devnet_matches_the_cpu_hash() {
    check(NetworkId::Devnet, 16, 3);
}

#[test]
fn mainnet_matches_the_cpu_hash() {
    check(NetworkId::Mainnet, 8, 2);
}

#[test]
fn edge_nonces_match() {
    let params = NetworkId::Mainnet.pow_params();
    let mut gpu = GpuHasher::new(params, 4).unwrap();
    let pre = header(9, 200);
    let nonces = [0u64, 1, u64::MAX, u64::MAX - 1];
    for (n, h) in nonces.iter().zip(gpu.hash_batch(&pre, &nonces)) {
        assert_eq!(h, nighthash(&pre, *n, params));
    }
}
