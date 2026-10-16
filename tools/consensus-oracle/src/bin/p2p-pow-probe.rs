use anyhow::{bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use num_bigint::BigUint;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const DOMAIN_BLOCK: &[u8] = b"nightfall:block:v2";
const DOMAIN_NIGHTHASH_SALT: &[u8] = b"nightfall:nighthash:salt:v2";

const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

const MAINNET_MEMORY_KIB: u32 = 32 * 1024;
const MAINNET_ITERATIONS: u32 = 1;
const MAINNET_LANES: u32 = 1;

fn hash_multi(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();

    h.update(&(domain.len() as u64).to_le_bytes());
    h.update(domain);

    h.update(&(parts.len() as u64).to_le_bytes());

    for part in parts {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }

    *h.finalize().as_bytes()
}

fn bytes32(v: &Value, name: &str) -> Result<[u8; 32]> {
    let a = v
        .as_array()
        .with_context(|| format!("{name}: expected byte array"))?;

    if a.len() != 32 {
        bail!("{name}: expected 32 bytes, got {}", a.len());
    }

    let mut out = [0u8; 32];

    for (i, x) in a.iter().enumerate() {
        let n = x
            .as_u64()
            .with_context(|| format!("{name}[{i}]: expected integer"))?;

        if n > 255 {
            bail!("{name}[{i}]: byte out of range");
        }

        out[i] = n as u8;
    }

    Ok(out)
}

fn u64_field(v: &Value, name: &str) -> Result<u64> {
    v.get(name)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing/invalid {name}"))
}

fn u32_field(v: &Value, name: &str) -> Result<u32> {
    let n = u64_field(v, name)?;
    u32::try_from(n).with_context(|| format!("{name} does not fit u32"))
}

fn independent_pow_preimage(header: &Value) -> Result<[u8; 32]> {
    let version = u32_field(header, "version")?;
    let height = u64_field(header, "height")?;
    let timestamp = u64_field(header, "timestamp_unix")?;
    let difficulty = u64_field(header, "difficulty")?;
    let reward = u64_field(header, "reward_darks")?;

    let prev_hash = bytes32(
        header.get("prev_hash").context("missing prev_hash")?,
        "prev_hash",
    )?;

    let utxo_root = bytes32(
        header.get("utxo_root").context("missing utxo_root")?,
        "utxo_root",
    )?;

    let kernel_sum = bytes32(
        header.get("kernel_sum").context("missing kernel_sum")?,
        "kernel_sum",
    )?;

    let body_root = bytes32(
        header.get("body_root").context("missing body_root")?,
        "body_root",
    )?;

    let version_b = version.to_le_bytes();
    let height_b = height.to_le_bytes();
    let timestamp_b = timestamp.to_le_bytes();
    let difficulty_b = difficulty.to_le_bytes();
    let reward_b = reward.to_le_bytes();

    Ok(hash_multi(
        DOMAIN_BLOCK,
        &[
            &version_b,
            &height_b,
            &prev_hash,
            &utxo_root,
            &kernel_sum,
            &body_root,
            &timestamp_b,
            &difficulty_b,
            &reward_b,
        ],
    ))
}

fn independent_block_hash(header: &Value) -> Result<[u8; 32]> {
    let preimage = independent_pow_preimage(header)?;
    let nonce = u64_field(header, "nonce")?;
    let nonce_b = nonce.to_le_bytes();

    Ok(hash_multi(DOMAIN_BLOCK, &[preimage.as_slice(), &nonce_b]))
}

fn independent_nighthash(preimage: &[u8; 32], nonce: u64) -> Result<[u8; 32]> {
    let salt_hash = hash_multi(DOMAIN_NIGHTHASH_SALT, &[preimage.as_slice()]);

    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_hash[..16]);

    let mut password = Vec::with_capacity(40);
    password.extend_from_slice(preimage);
    password.extend_from_slice(&nonce.to_le_bytes());

    let params = Params::new(
        MAINNET_MEMORY_KIB,
        MAINNET_ITERATIONS,
        MAINNET_LANES,
        Some(32),
    )
    .map_err(|e| anyhow::anyhow!("construct Argon2 mainnet parameters: {e}"))?;

    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut out = [0u8; 32];

    argon
        .hash_password_into(&password, &salt, &mut out)
        .map_err(|e| anyhow::anyhow!("Argon2id Nighthash computation: {e}"))?;

    Ok(out)
}

fn independent_meets_difficulty(hash: &[u8; 32], difficulty: u64) -> bool {
    if difficulty == 0 {
        return true;
    }

    // Deliberately different arithmetic from production:
    // interpret H directly as a big-endian integer and test
    //
    //             H * D < 2^256
    //
    // using arbitrary precision arithmetic.
    let h = BigUint::from_bytes_be(hash);
    let d = BigUint::from(difficulty);
    let limit = BigUint::from(1u8) << 256usize;

    h * d < limit
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut s = String::with_capacity(64);

    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }

    s
}

fn write_line(stream: &mut TcpStream, value: &Value) -> Result<()> {
    let line = serde_json::to_string(value)?;

    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    Ok(())
}

fn read_json_line(reader: &mut BufReader<TcpStream>) -> Result<Value> {
    let mut line = String::new();

    reader.read_line(&mut line)?;

    if line.trim().is_empty() {
        bail!("peer closed connection");
    }

    serde_json::from_str(line.trim()).context("decode P2P JSON")
}

fn public_hashes(from: u64, limit: usize) -> Result<BTreeMap<u64, String>> {
    let body = json!({
        "method": "get_headers",
        "params": {
            "from": from,
            "limit": limit
        },
        "id": 777
    });

    let request_body = serde_json::to_string(&body).context("encode get_headers")?;

    let response = ureq::post("http://seed.nightfallcoin.org/")
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("User-Agent", "nightfall-independent-consensus-oracle/0.1")
        .send_string(&request_body)
        .context("public get_headers request")?;

    let response_body = response
        .into_string()
        .context("read public get_headers response")?;

    let root: Value =
        serde_json::from_str(&response_body).context("decode public get_headers response")?;

    if let Some(error) = root.get("error").filter(|v| !v.is_null()) {
        bail!("public get_headers error: {error}");
    }

    let headers = root
        .pointer("/result/headers")
        .and_then(Value::as_array)
        .context("public get_headers returned no headers")?;

    let mut result = BTreeMap::new();

    for h in headers {
        let height = u64_field(h, "height")?;

        let hash = h
            .get("hash")
            .and_then(Value::as_str)
            .context("public header missing hash")?;

        result.insert(height, hash.to_ascii_lowercase());
    }

    Ok(result)
}

fn connect_archive_peer() -> Result<(String, TcpStream, BufReader<TcpStream>, u64)> {
    let peers = [
        "seed3.nightfallcoin.org:17891",
        "seed2.nightfallcoin.org:17891",
        "seed.nightfallcoin.org:17891",
    ];

    let mut errors = Vec::new();

    for peer in peers {
        let attempt = (|| -> Result<_> {
            let mut stream = TcpStream::connect(peer).with_context(|| format!("connect {peer}"))?;

            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            stream.set_nodelay(true)?;

            let mut reader = BufReader::new(stream.try_clone()?);

            write_line(
                &mut stream,
                &json!({
                    "type": "hello",
                    "wire": 6,
                    "network": "mainnet",
                    "genesis": GENESIS,
                    "height": 0,
                    "tip": GENESIS,
                    "agent":
                        "nightfall-independent-consensus-oracle/0.1",
                    "listen_port": 0,
                    "pruned": false,
                    "first_height": 0
                }),
            )?;

            let hello = read_json_line(&mut reader)?;

            if hello.get("type").and_then(Value::as_str) != Some("hello_ok") {
                bail!("unexpected handshake: {hello}");
            }

            let height = u64_field(&hello, "height")?;

            let pruned = hello
                .get("pruned")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let first_height = hello
                .get("first_height")
                .and_then(Value::as_u64)
                .unwrap_or(0);

            if pruned || first_height != 0 {
                bail!(
                    "peer is not an archive: pruned={pruned}, \
                     first_height={first_height}"
                );
            }

            Ok((peer.to_string(), stream, reader, height))
        })();

        match attempt {
            Ok(conn) => return Ok(conn),
            Err(e) => errors.push(format!("{peer}: {e:#}")),
        }
    }

    bail!("no archive seed available:\n{}", errors.join("\n"))
}

fn fetch_blocks(
    stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    from: u64,
    limit: usize,
) -> Result<Vec<Value>> {
    write_line(
        stream,
        &json!({
            "type": "get_blocks",
            "from_height": from,
            "limit": limit
        }),
    )?;

    loop {
        let msg = read_json_line(reader)?;

        match msg.get("type").and_then(Value::as_str) {
            Some("blocks") => {
                return Ok(msg
                    .get("blocks")
                    .and_then(Value::as_array)
                    .context("blocks message has no blocks")?
                    .clone());
            }

            Some("ping") => {
                let nonce = msg.get("nonce").and_then(Value::as_u64).unwrap_or(0);

                write_line(
                    stream,
                    &json!({
                        "type": "pong",
                        "nonce": nonce
                    }),
                )?;
            }

            Some("error") => {
                bail!("P2P peer error: {msg}");
            }

            _ => {}
        }
    }
}

fn main() -> Result<()> {
    println!("NIGHTFALL independent Nighthash-v2 probe");
    println!("Nightfall consensus imports: none");
    println!("Nightfall crypto imports:    none");
    println!("difficulty arithmetic:       independent BigUint");

    let (peer, mut stream, mut reader, peer_height) = connect_archive_peer()?;

    println!("archive P2P peer: {peer}");
    println!("peer height:      {peer_height}");

    let mut starts = vec![0, 24_996, peer_height.saturating_sub(7)];

    starts.sort_unstable();
    starts.dedup();

    let mut verified = 0usize;

    for from in starts {
        println!();
        println!("checking range starting at height {from}...");

        let expected = public_hashes(from, 8)?;
        let blocks = fetch_blocks(&mut stream, &mut reader, from, 8)?;

        if blocks.len() < 2 {
            bail!("insufficient blocks from height {from}: {}", blocks.len());
        }

        for (index, block) in blocks.iter().enumerate() {
            let header = block.get("header").context("P2P block missing header")?;

            let height = u64_field(header, "height")?;
            let difficulty = u64_field(header, "difficulty")?;
            let nonce = u64_field(header, "nonce")?;

            let preimage = independent_pow_preimage(header)?;

            let canonical_hash = independent_block_hash(header)?;

            let canonical_hex = hex32(&canonical_hash);

            let public_hash = expected
                .get(&height)
                .with_context(|| format!("public hash missing at {height}"))?;

            if &canonical_hex != public_hash {
                bail!(
                    "BLOCK HASH MISMATCH at {height}: \
                     independent={canonical_hex} \
                     public={public_hash}"
                );
            }

            let pow_hash = independent_nighthash(&preimage, nonce)?;

            if !independent_meets_difficulty(&pow_hash, difficulty) {
                bail!(
                    "INVALID POW at height {height}: \
                     pow={} difficulty={difficulty}",
                    hex32(&pow_hash)
                );
            }

            if let Some(next) = blocks.get(index + 1) {
                let next_header = next.get("header").context("next block missing header")?;

                let next_prev = bytes32(
                    next_header
                        .get("prev_hash")
                        .context("missing next prev_hash")?,
                    "next.prev_hash",
                )?;

                if canonical_hash != next_prev {
                    bail!(
                        "PARENT LINK MISMATCH {} -> {}",
                        height,
                        u64_field(next_header, "height")?
                    );
                }
            }

            println!(
                "height {:>7}: hash PASS | Nighthash-v2 PASS | D={}",
                height, difficulty
            );

            verified += 1;
        }
    }

    println!();
    println!("========================================");
    println!("PHASE 2B POW PROBE PASS");
    println!("verified blocks......... {verified}");
    println!("sample coverage......... genesis/checkpoint/tip");
    println!("header preimage......... independently rebuilt");
    println!("salt derivation......... independently rebuilt");
    println!("Argon2 mode............. Argon2id v=0x13");
    println!("memory.................. 32768 KiB");
    println!("iterations.............. 1");
    println!("lanes................... 1");
    println!("difficulty arithmetic... independent BigUint");
    println!("canonical block hash.... independently matched");
    println!("parent linkage.......... matched");
    println!("production code......... unchanged");
    println!("========================================");

    Ok(())
}
