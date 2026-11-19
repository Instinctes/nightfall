//! Runs the CUDA kernel source through the host emulator (tools/cuda_emu.cpp)
//! and compares every result with the consensus hash.
//! Usage: cargo run -p nightfall-gpu --example cuda_emu_check -- <path-to-cuda_emu>

use nightfall_gpu::argon::{self, BLOCK_WORDS};
use nightfall_types::NetworkId;
use std::io::Write;
use std::process::{Command, Stdio};

fn check(emu: &str, network: NetworkId, nonces: &[u64], pre: &[u8]) -> usize {
    let params = network.pow_params();
    let m = argon::lane_blocks(params);
    let salt = argon::salt_for(pre);
    let mut init = vec![0u64; nonces.len() * 2 * BLOCK_WORDS];
    for (i, &n) in nonces.iter().enumerate() {
        let h0 = argon::h0(params, pre, n, &salt);
        argon::first_blocks(
            &h0,
            &mut init[i * 2 * BLOCK_WORDS..(i + 1) * 2 * BLOCK_WORDS],
        );
    }
    let mut child = Command::new(emu)
        .args([m.to_string(), (m / 4).to_string(), nonces.len().to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run emulator");
    let bytes: Vec<u8> = init.iter().flat_map(|w| w.to_le_bytes()).collect();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "emulator failed");
    let words: Vec<u64> = out
        .stdout
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| u64::from_le_bytes(*c))
        .collect();
    let mut ok = 0;
    for (i, &n) in nonces.iter().enumerate() {
        let got = argon::final_tag(&words[i * BLOCK_WORDS..(i + 1) * BLOCK_WORDS]);
        let want = nightfall_crypto::nighthash(pre, n, params);
        if got == want {
            ok += 1;
        } else {
            println!("MISMATCH {network:?} nonce {n}");
        }
    }
    ok
}

fn main() {
    let emu = std::env::args().nth(1).expect("path to cuda_emu");
    let pre: Vec<u8> = (0..96u32)
        .map(|i| (i as u8).wrapping_mul(29).wrapping_add(7))
        .collect();
    let dev = [0u64, 1, u64::MAX, 0x9E37_79B9_7F4A_7C15, 42, 7_000_000];
    let a = check(&emu, NetworkId::Devnet, &dev, &pre);
    println!("devnet {a}/{}", dev.len());
    let main_n = [0u64, u64::MAX];
    let b = check(&emu, NetworkId::Mainnet, &main_n, &pre[..77]);
    println!("mainnet {b}/{}", main_n.len());
    assert_eq!(
        a + b,
        dev.len() + main_n.len(),
        "kernel disagrees with consensus"
    );
    println!("PASS");
}
