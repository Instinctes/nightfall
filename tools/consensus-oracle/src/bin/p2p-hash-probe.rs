use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const DOMAIN_BLOCK: &[u8] = b"nightfall:block:v2";

const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

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

fn independent_block_hash(header: &Value) -> Result<[u8; 32]> {
    let version = u32_field(header, "version")?;
    let height = u64_field(header, "height")?;
    let timestamp = u64_field(header, "timestamp_unix")?;
    let difficulty = u64_field(header, "difficulty")?;
    let nonce = u64_field(header, "nonce")?;
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

    let pow_preimage = hash_multi(
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
    );

    let nonce_b = nonce.to_le_bytes();

    Ok(hash_multi(DOMAIN_BLOCK, &[&pow_preimage, &nonce_b]))
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

fn light_hashes() -> Result<BTreeMap<u64, String>> {
    let body = json!({
        "method": "get_headers",
        "params": {
            "from": 0,
            "limit": 8
        },
        "id": 777
    });

    let request_body = serde_json::to_string(&body).context("encode get_headers request")?;

    let response = ureq::post("http://seed.nightfallcoin.org/")
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("User-Agent", "nightfall-independent-consensus-oracle/0.1")
        .send_string(&request_body)
        .context("public get_headers request")?;

    let response_body = response
        .into_string()
        .context("read public get_headers response")?;

    let root: Value = serde_json::from_str(&response_body).with_context(|| {
        format!(
            "decode public get_headers response: {}",
            response_body.chars().take(500).collect::<String>()
        )
    })?;

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
fn main() -> Result<()> {
    println!("NIGHTFALL independent P2P hash probe");
    println!("production node code: untouched");
    println!("consensus/ledger imports: none");

    let public_hashes = light_hashes()?;

    println!("public header hashes obtained: {}", public_hashes.len());

    let mut stream =
        TcpStream::connect("seed.nightfallcoin.org:17891").context("connect Nightfall P2P seed")?;

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
            "agent": "nightfall-independent-consensus-oracle/0.1",
            "listen_port": 0,
            "pruned": false,
            "first_height": 0
        }),
    )?;

    let hello = read_json_line(&mut reader)?;

    match hello.get("type").and_then(Value::as_str) {
        Some("hello_ok") => {
            println!("P2P handshake: PASS");
        }

        Some("error") => {
            bail!("P2P handshake rejected: {hello}");
        }

        other => {
            bail!("unexpected P2P handshake response: {other:?}: {hello}");
        }
    }

    write_line(
        &mut stream,
        &json!({
            "type": "get_blocks",
            "from_height": 0,
            "limit": 8
        }),
    )?;

    let blocks = loop {
        let msg = read_json_line(&mut reader)?;

        match msg.get("type").and_then(Value::as_str) {
            Some("blocks") => {
                break msg
                    .get("blocks")
                    .and_then(Value::as_array)
                    .context("blocks message contains no blocks")?
                    .clone();
            }

            Some("ping") => {
                let nonce = msg.get("nonce").and_then(Value::as_u64).unwrap_or(0);

                write_line(
                    &mut stream,
                    &json!({
                        "type": "pong",
                        "nonce": nonce
                    }),
                )?;
            }

            Some("error") => {
                bail!("P2P peer returned error: {msg}");
            }

            _ => {
                // Ignore inventory/status chatter while waiting
                // for the explicit Blocks response.
            }
        }
    };

    if blocks.len() < 2 {
        bail!(
            "need at least 2 blocks for hash/linkage check; got {}",
            blocks.len()
        );
    }

    println!("full P2P blocks obtained: {}", blocks.len());

    let mut independently_verified = 0usize;

    for (index, block) in blocks.iter().enumerate() {
        let header = block.get("header").context("P2P block missing header")?;

        let height = u64_field(header, "height")?;
        let calculated = independent_block_hash(header)?;
        let calculated_hex = hex32(&calculated);

        if let Some(expected) = public_hashes.get(&height) {
            if calculated_hex != *expected {
                bail!(
                    "HASH MISMATCH height {height}: independent={calculated_hex} public={expected}"
                );
            }

            println!(
                "height {:>3}: independent hash == public hash  PASS",
                height
            );

            independently_verified += 1;
        } else {
            bail!("public hash missing for height {height}");
        }

        if let Some(next) = blocks.get(index + 1) {
            let next_header = next
                .get("header")
                .context("next P2P block missing header")?;

            let next_prev = bytes32(
                next_header
                    .get("prev_hash")
                    .context("next header missing prev_hash")?,
                "next.prev_hash",
            )?;

            if calculated != next_prev {
                bail!(
                    "PARENT LINK MISMATCH height {} -> {}",
                    height,
                    u64_field(next_header, "height")?
                );
            }
        }
    }

    if independently_verified != blocks.len() {
        bail!("not every received block was independently cross-checked");
    }

    println!();
    println!("========================================");
    println!("PHASE 2A HASH PROBE PASS");
    println!(
        "{} full block headers independently hashed",
        independently_verified
    );
    println!("BLAKE3 encoding........ independently implemented");
    println!("header preimage........ independently implemented");
    println!("nonce binding.......... independently implemented");
    println!("public hash crosscheck. matched");
    println!("parent linkage......... matched");
    println!("production API......... unchanged");
    println!("========================================");

    Ok(())
}
