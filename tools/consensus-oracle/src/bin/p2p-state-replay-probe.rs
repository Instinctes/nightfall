use anyhow::{bail, Context, Result};
use bulletproofs::PedersenGens;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

const END_HEIGHT: u64 = 2326;
const P2P_PAGE: usize = 128;
const HTTP_PAGE: u64 = 512;

const COINBASE_MATURITY: u64 = 1_440;

const DOMAIN_BLOCK: &[u8] = b"nightfall:block:v2";

const DOMAIN_TXBODY: &[u8] = b"nightfall:txbody:v2";

const DOMAIN_KERNEL: &[u8] = b"nightfall:kernel:v2";

const DOMAIN_INPUT: &[u8] = b"nightfall:input:v2";

const DOMAIN_SCHNORR: &[u8] = b"nightfall:schnorr:v2";

const DOMAIN_MERKLE: &[u8] = b"nightfall:merkle:v2";

const DOMAIN_MERKLE_LEAF: &[u8] = b"nightfall:merkle:leaf:v2";

#[derive(Clone, Debug)]
struct UtxoEntry {
    output_pk: [u8; 32],
    height: u64,
    is_coinbase: bool,
}

#[derive(Default)]
struct Stats {
    blocks: u64,
    root_checks: u64,
    kernel_sum_checks: u64,
    inputs_verified: u64,
    coinbase_spends: u64,
    plain_spends: u64,
    transfer_blocks: u64,
}

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

fn bytes(v: &Value, name: &str) -> Result<Vec<u8>> {
    let a = v
        .as_array()
        .with_context(|| format!("{name}: expected byte array"))?;

    let mut out = Vec::with_capacity(a.len());

    for (i, x) in a.iter().enumerate() {
        let n = x
            .as_u64()
            .with_context(|| format!("{name}[{i}]: expected byte"))?;

        if n > 255 {
            bail!("{name}[{i}]: byte out of range");
        }

        out.push(n as u8);
    }

    Ok(out)
}

fn bytes32(v: &Value, name: &str) -> Result<[u8; 32]> {
    let raw = bytes(v, name)?;

    if raw.len() != 32 {
        bail!("{name}: expected 32 bytes, got {}", raw.len());
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);

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

fn array_field<'a>(v: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    v.get(name)
        .and_then(Value::as_array)
        .with_context(|| format!("missing/invalid {name}"))
}

fn feature_byte(v: &Value, name: &str) -> Result<u8> {
    match v.as_str() {
        Some("plain") => Ok(0),
        Some("coinbase") => Ok(1),

        other => bail!("{name}: unknown feature {other:?}"),
    }
}

fn signature(v: &Value, name: &str) -> Result<([u8; 32], [u8; 32])> {
    let r = bytes32(
        v.get("r").with_context(|| format!("{name}.r missing"))?,
        &format!("{name}.r"),
    )?;

    let s = bytes32(
        v.get("s").with_context(|| format!("{name}.s missing"))?,
        &format!("{name}.s"),
    )?;

    Ok((r, s))
}

fn hex32(b: &[u8; 32]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";

    let mut s = String::with_capacity(64);

    for &x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 0x0f) as usize] as char);
    }

    s
}

fn hex_value(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => bail!("invalid hex digit"),
    }
}

fn parse_hex32(s: &str) -> Result<[u8; 32]> {
    let raw = s.as_bytes();

    if raw.len() != 64 {
        bail!("expected 64 hex characters");
    }

    let mut out = [0u8; 32];

    for i in 0..32 {
        out[i] = (hex_value(raw[i * 2])? << 4) | hex_value(raw[i * 2 + 1])?;
    }

    Ok(out)
}

fn schnorr_challenge(r: &[u8; 32], p: &[u8; 32], msg: &[u8]) -> Scalar {
    let a = hash_multi(DOMAIN_SCHNORR, &[r, p, msg, b"c0"]);

    let b = hash_multi(DOMAIN_SCHNORR, &[r, p, msg, b"c1"]);

    let mut wide = [0u8; 64];

    wide[..32].copy_from_slice(&a);
    wide[32..].copy_from_slice(&b);

    Scalar::from_bytes_mod_order_wide(&wide)
}

fn verify_schnorr(
    public: &RistrettoPoint,
    generator: &RistrettoPoint,
    msg: &[u8],
    r_bytes: [u8; 32],
    s_bytes: [u8; 32],
) -> bool {
    let Some(r_point) = CompressedRistretto(r_bytes).decompress() else {
        return false;
    };

    let Some(s) = Option::<Scalar>::from(Scalar::from_canonical_bytes(s_bytes)) else {
        return false;
    };

    let public_bytes = public.compress().to_bytes();

    let e = schnorr_challenge(&r_bytes, &public_bytes, msg);

    generator * s == r_point + public * e
}

fn verify_input_signature(input: &Value, output_pk: &[u8; 32]) -> Result<()> {
    let commit = bytes32(
        input.get("commit").context("input commit missing")?,
        "input.commit",
    )?;

    let public = CompressedRistretto(*output_pk)
        .decompress()
        .context("stored output_pk malformed")?;

    let msg = hash_multi(DOMAIN_INPUT, &[&commit]);

    let sig = input.get("sig").context("input sig missing")?;

    let (r, s) = signature(sig, "input.sig")?;

    let g = PedersenGens::default().B;

    if !verify_schnorr(&public, &g, &msg, r, s) {
        bail!("invalid input ownership signature");
    }

    Ok(())
}

fn negative_signature_control(input: &Value, output_pk: &[u8; 32]) -> Result<()> {
    let commit = bytes32(
        input.get("commit").context("input commit missing")?,
        "input.commit",
    )?;

    let public = CompressedRistretto(*output_pk)
        .decompress()
        .context("stored output_pk malformed")?;

    let msg = hash_multi(DOMAIN_INPUT, &[&commit]);

    let sig = input.get("sig").context("input sig missing")?;

    let (r, mut s) = signature(sig, "input.sig")?;

    s[0] ^= 1;

    let g = PedersenGens::default().B;

    if verify_schnorr(&public, &g, &msg, r, s) {
        bail!("NEGATIVE CONTROL FAILED: tampered input signature accepted");
    }

    Ok(())
}

fn kernel_message(k: &Value) -> Result<[u8; 32]> {
    let feature = feature_byte(
        k.get("feature").context("kernel feature missing")?,
        "kernel.feature",
    )?;

    let fee = u64_field(k, "fee_darks")?;

    let reward = u64_field(k, "reward_darks")?;

    let lock = u64_field(k, "lock_height")?;

    let excess = bytes32(
        k.get("excess").context("kernel excess missing")?,
        "kernel.excess",
    )?;

    Ok(hash_multi(
        DOMAIN_KERNEL,
        &[
            &[feature],
            &fee.to_le_bytes(),
            &reward.to_le_bytes(),
            &lock.to_le_bytes(),
            &excess,
        ],
    ))
}

fn output_commitment_bytes(o: &Value) -> Result<Vec<u8>> {
    let feature = feature_byte(
        o.get("features").context("output features missing")?,
        "output.features",
    )?;

    let commit = bytes32(
        o.get("commit").context("output commit missing")?,
        "output.commit",
    )?;

    let ephemeral = bytes32(
        o.get("ephemeral_pk")
            .context("output ephemeral_pk missing")?,
        "output.ephemeral_pk",
    )?;

    let output_pk = bytes32(
        o.get("output_pk").context("output output_pk missing")?,
        "output.output_pk",
    )?;

    let view_tag = u64_field(o, "view_tag")?;

    if view_tag > 255 {
        bail!("output view_tag outside byte range");
    }

    let proof = bytes(
        o.get("range_proof").context("range_proof missing")?,
        "output.range_proof",
    )?;

    let payload = bytes(
        o.get("payload").context("payload missing")?,
        "output.payload",
    )?;

    let mut out = Vec::new();

    out.push(feature);
    out.extend_from_slice(&commit);
    out.extend_from_slice(&ephemeral);
    out.extend_from_slice(&output_pk);
    out.push(view_tag as u8);
    out.extend_from_slice(&proof);
    out.extend_from_slice(&payload);

    Ok(out)
}

fn body_hash(body: &Value) -> Result<[u8; 32]> {
    let inputs = array_field(body, "inputs")?;

    let outputs = array_field(body, "outputs")?;

    let kernels = array_field(body, "kernels")?;

    let mut parts = Vec::<Vec<u8>>::new();

    for input in inputs {
        let commit = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        let sig = input.get("sig").context("input sig missing")?;

        let (r, s) = signature(sig, "input.sig")?;

        let mut p = Vec::with_capacity(96);

        p.extend_from_slice(&commit);
        p.extend_from_slice(&r);
        p.extend_from_slice(&s);

        parts.push(p);
    }

    for output in outputs {
        let mut p = output_commitment_bytes(output)?;

        let sig = output.get("sender_sig").context("sender_sig missing")?;

        let (r, s) = signature(sig, "output.sender_sig")?;

        p.extend_from_slice(&r);
        p.extend_from_slice(&s);

        parts.push(p);
    }

    for kernel in kernels {
        let msg = kernel_message(kernel)?;

        let sig = kernel.get("excess_sig").context("excess_sig missing")?;

        let (r, s) = signature(sig, "kernel.excess_sig")?;

        let mut p = Vec::with_capacity(96);

        p.extend_from_slice(&msg);
        p.extend_from_slice(&r);
        p.extend_from_slice(&s);

        parts.push(p);
    }

    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();

    Ok(hash_multi(DOMAIN_TXBODY, &refs))
}

fn block_hash(h: &Value) -> Result<[u8; 32]> {
    let version = u32_field(h, "version")?;

    let height = u64_field(h, "height")?;

    let prev_hash = bytes32(
        h.get("prev_hash").context("prev_hash missing")?,
        "header.prev_hash",
    )?;

    let utxo_root = bytes32(
        h.get("utxo_root").context("utxo_root missing")?,
        "header.utxo_root",
    )?;

    let kernel_sum = bytes32(
        h.get("kernel_sum").context("kernel_sum missing")?,
        "header.kernel_sum",
    )?;

    let body_root = bytes32(
        h.get("body_root").context("body_root missing")?,
        "header.body_root",
    )?;

    let time = u64_field(h, "timestamp_unix")?;

    let difficulty = u64_field(h, "difficulty")?;

    let nonce = u64_field(h, "nonce")?;

    let reward = u64_field(h, "reward_darks")?;

    let version_b = version.to_le_bytes();

    let height_b = height.to_le_bytes();

    let time_b = time.to_le_bytes();

    let difficulty_b = difficulty.to_le_bytes();

    let reward_b = reward.to_le_bytes();

    let preimage = hash_multi(
        DOMAIN_BLOCK,
        &[
            &version_b,
            &height_b,
            &prev_hash,
            &utxo_root,
            &kernel_sum,
            &body_root,
            &time_b,
            &difficulty_b,
            &reward_b,
        ],
    );

    let nonce_b = nonce.to_le_bytes();

    Ok(hash_multi(DOMAIN_BLOCK, &[&preimage, &nonce_b]))
}

fn utxo_root(utxos: &BTreeMap<[u8; 32], UtxoEntry>) -> [u8; 32] {
    if utxos.is_empty() {
        return [0u8; 32];
    }

    let mut level: Vec<[u8; 32]> = utxos
        .iter()
        .map(|(commit, entry)| {
            let h = entry.height.to_le_bytes();

            hash_multi(DOMAIN_MERKLE_LEAF, &[commit, &entry.output_pk, &h])
        })
        .collect();

    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));

        for pair in level.chunks(2) {
            let right = if pair.len() == 2 { pair[1] } else { pair[0] };

            next.push(hash_multi(DOMAIN_MERKLE, &[&pair[0], &right]));
        }

        level = next;
    }

    level[0]
}

fn http_header_hashes() -> Result<BTreeMap<u64, String>> {
    let mut result = BTreeMap::new();

    let mut from = 0u64;

    while from <= END_HEIGHT {
        let request = serde_json::to_string(&json!({
            "method": "get_headers",
            "params": {
                "from": from,
                "limit": HTTP_PAGE
            },
            "id": 773
        }))?;

        let response = loop {
            match ureq::post("http://seed.nightfallcoin.org/")
                .set("Content-Type", "application/json")
                .set("Accept", "application/json")
                .set("User-Agent", "nightfall-independent-consensus-oracle/0.1")
                .send_string(&request)
            {
                Ok(v) => break v,

                Err(ureq::Error::Status(429, _)) => {
                    eprintln!("Light API rate limit; retrying after reset...");

                    std::thread::sleep(Duration::from_secs(61));
                }

                Err(ureq::Error::Status(code, r)) => {
                    let body = r.into_string().unwrap_or_default();

                    bail!("HTTP {code}: {body}");
                }

                Err(e) => {
                    bail!("HTTP transport error: {e}");
                }
            }
        };

        let text = response
            .into_string()
            .context("read get_headers response")?;

        let root: Value = serde_json::from_str(&text).context("decode get_headers")?;

        if let Some(e) = root.get("error").filter(|v| !v.is_null()) {
            bail!("get_headers error: {e}");
        }

        let headers = root
            .pointer("/result/headers")
            .and_then(Value::as_array)
            .context("get_headers returned no headers")?;

        if headers.is_empty() {
            bail!("unexpected empty header page at {from}");
        }

        let mut last = from;

        for h in headers {
            let height = u64_field(h, "height")?;

            if height > END_HEIGHT {
                break;
            }

            let hash = h
                .get("hash")
                .and_then(Value::as_str)
                .context("public header hash missing")?;

            result.insert(height, hash.to_ascii_lowercase());

            last = height;
        }

        if last >= END_HEIGHT {
            break;
        }

        from = last.checked_add(1).context("header height overflow")?;

        eprintln!("canonical headers loaded through {last}/{END_HEIGHT}");
    }

    if result.len() != (END_HEIGHT as usize + 1) {
        bail!(
            "canonical header set incomplete: got {}, expected {}",
            result.len(),
            END_HEIGHT + 1,
        );
    }

    Ok(result)
}

fn write_line(stream: &mut TcpStream, value: &Value) -> Result<()> {
    let line = serde_json::to_string(value)?;

    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    Ok(())
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Value> {
    let mut line = String::new();

    reader.read_line(&mut line)?;

    if line.trim().is_empty() {
        bail!("peer closed connection");
    }

    serde_json::from_str(line.trim()).context("decode P2P JSON")
}

fn connect_archive() -> Result<(String, TcpStream, BufReader<TcpStream>)> {
    let peers = [
        "seed3.nightfallcoin.org:17891",
        "seed2.nightfallcoin.org:17891",
        "seed.nightfallcoin.org:17891",
    ];

    let mut errors = Vec::new();

    for peer in peers {
        let attempt = (|| -> Result<_> {
            let mut stream = TcpStream::connect(peer).with_context(|| format!("connect {peer}"))?;

            stream.set_read_timeout(Some(Duration::from_secs(45)))?;

            stream.set_write_timeout(Some(Duration::from_secs(45)))?;

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

            let hello = read_line(&mut reader)?;

            if hello.get("type").and_then(Value::as_str) != Some("hello_ok") {
                bail!("unexpected handshake: {hello}");
            }

            let pruned = hello
                .get("pruned")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let first = hello
                .get("first_height")
                .and_then(Value::as_u64)
                .unwrap_or(0);

            if pruned || first != 0 {
                bail!("peer is not archive: pruned={pruned} first_height={first}");
            }

            Ok((peer.to_string(), stream, reader))
        })();

        match attempt {
            Ok(v) => return Ok(v),

            Err(e) => errors.push(format!("{peer}: {e:#}")),
        }
    }

    bail!("no archive peer available:\n{}", errors.join("\n"))
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
        let msg = read_line(reader)?;

        match msg.get("type").and_then(Value::as_str) {
            Some("blocks") => {
                return Ok(msg
                    .get("blocks")
                    .and_then(Value::as_array)
                    .context("blocks response missing blocks")?
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

#[allow(clippy::too_many_arguments)]
fn process_block(
    block: &Value,
    expected_height: u64,
    expected_prev: &[u8; 32],
    canonical_hashes: &BTreeMap<u64, String>,
    utxos: &mut BTreeMap<[u8; 32], UtxoEntry>,
    kernel_sum: &mut RistrettoPoint,
    negative_control_done: &mut bool,
    stats: &mut Stats,
) -> Result<[u8; 32]> {
    let header = block.get("header").context("block header missing")?;

    let body = block.get("body").context("block body missing")?;

    let height = u64_field(header, "height")?;

    if height != expected_height {
        bail!("height mismatch: expected {expected_height}, got {height}");
    }

    let prev = bytes32(
        header.get("prev_hash").context("prev_hash missing")?,
        "header.prev_hash",
    )?;

    if &prev != expected_prev {
        bail!("height {height}: parent linkage mismatch");
    }

    let canonical = block_hash(header)?;

    let expected_public = canonical_hashes
        .get(&height)
        .with_context(|| format!("canonical public hash missing at {height}"))?;

    if &hex32(&canonical) != expected_public {
        bail!("height {height}: block hash differs from public mainnet header");
    }

    let independent_body = body_hash(body)?;

    let header_body = bytes32(
        header.get("body_root").context("body_root missing")?,
        "header.body_root",
    )?;

    if independent_body != header_body {
        bail!("height {height}: BODY ROOT MISMATCH");
    }

    let inputs = array_field(body, "inputs")?;

    let outputs = array_field(body, "outputs")?;

    let kernels = array_field(body, "kernels")?;

    let mut spent = BTreeSet::<[u8; 32]>::new();

    for input in inputs {
        let commit = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        if !spent.insert(commit) {
            bail!("height {height}: duplicate input within block");
        }
    }

    /*
     * Production performs output collision
     * checks against the pre-spend state.
     */
    let mut created = BTreeSet::<[u8; 32]>::new();

    for output in outputs {
        let commit = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        if CompressedRistretto(commit).decompress().is_none() {
            bail!("height {height}: malformed output commitment");
        }

        let output_pk = bytes32(
            output.get("output_pk").context("output_pk missing")?,
            "output.output_pk",
        )?;

        if CompressedRistretto(output_pk).decompress().is_none() {
            bail!("height {height}: malformed output_pk");
        }

        if !created.insert(commit) {
            bail!("height {height}: duplicate output commitment");
        }

        if utxos.contains_key(&commit) {
            bail!("height {height}: output collides with existing UTXO");
        }
    }

    let mut block_coinbase_spends = 0u64;
    let mut block_plain_spends = 0u64;

    /*
     * Validate every input against the
     * pre-block UTXO set before modifying it.
     */
    for input in inputs {
        let commit = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        let entry = utxos
            .get(&commit)
            .cloned()
            .with_context(|| format!("height {height}: UNKNOWN INPUT {}", hex32(&commit)))?;

        if entry.is_coinbase {
            let mature_at = entry.height.saturating_add(COINBASE_MATURITY);

            if height < mature_at {
                bail!(
                    "height {height}: IMMATURE COINBASE SPEND \
                     created={} mature_at={mature_at}",
                    entry.height
                );
            }

            block_coinbase_spends += 1;
            stats.coinbase_spends += 1;
        } else {
            block_plain_spends += 1;
            stats.plain_spends += 1;
        }

        verify_input_signature(input, &entry.output_pk)
            .with_context(|| format!("height {height}: input ownership"))?;

        if !*negative_control_done {
            negative_signature_control(input, &entry.output_pk)?;

            *negative_control_done = true;

            println!("negative control: tampered input signature correctly REJECTED");
        }

        stats.inputs_verified += 1;
    }

    /*
     * Commit state transition only after
     * every input has passed.
     */
    for commit in &spent {
        let removed = utxos.remove(commit);

        if removed.is_none() {
            bail!("height {height}: state removal failed");
        }
    }

    for output in outputs {
        let commit = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        let output_pk = bytes32(
            output.get("output_pk").context("output_pk missing")?,
            "output.output_pk",
        )?;

        let feature = feature_byte(
            output.get("features").context("output features missing")?,
            "output.features",
        )?;

        let old = utxos.insert(
            commit,
            UtxoEntry {
                output_pk,
                height,
                is_coinbase: feature == 1,
            },
        );

        if old.is_some() {
            bail!("height {height}: unexpected UTXO replacement");
        }
    }

    for kernel in kernels {
        let excess = bytes32(
            kernel.get("excess").context("kernel excess missing")?,
            "kernel.excess",
        )?;

        let point = CompressedRistretto(excess)
            .decompress()
            .with_context(|| format!("height {height}: malformed kernel excess"))?;

        *kernel_sum += point;
    }

    let calculated_root = utxo_root(utxos);

    let expected_root = bytes32(
        header.get("utxo_root").context("utxo_root missing")?,
        "header.utxo_root",
    )?;

    if calculated_root != expected_root {
        bail!(
            "height {height}: UTXO ROOT MISMATCH \
             independent={} header={}",
            hex32(&calculated_root),
            hex32(&expected_root),
        );
    }

    stats.root_checks += 1;

    let calculated_kernel_sum = kernel_sum.compress().to_bytes();

    let expected_kernel_sum = bytes32(
        header.get("kernel_sum").context("kernel_sum missing")?,
        "header.kernel_sum",
    )?;

    if calculated_kernel_sum != expected_kernel_sum {
        bail!(
            "height {height}: KERNEL SUM MISMATCH \
             independent={} header={}",
            hex32(&calculated_kernel_sum),
            hex32(&expected_kernel_sum),
        );
    }

    stats.kernel_sum_checks += 1;
    stats.blocks += 1;

    if !inputs.is_empty() {
        stats.transfer_blocks += 1;

        println!(
            "height {:>7}: STATE PASS | inputs={} \
             coinbase_spends={} plain_spends={} \
             utxos={}",
            height,
            inputs.len(),
            block_coinbase_spends,
            block_plain_spends,
            utxos.len(),
        );
    } else if height % 250 == 0 || height == END_HEIGHT {
        println!("height {:>7}: replay PASS | utxos={}", height, utxos.len(),);
    }

    Ok(canonical)
}

fn main() -> Result<()> {
    println!("NIGHTFALL independent state replay");

    println!("Nightfall consensus imports: none");

    println!("Nightfall ledger imports:    none");

    println!("Nightfall crypto imports:    none");

    println!("replay range................. 0..={END_HEIGHT}");

    println!("coinbase maturity............ {COINBASE_MATURITY}");

    println!();
    println!("loading canonical public header hashes...");

    let canonical_hashes = http_header_hashes()?;

    println!("canonical headers loaded..... {}", canonical_hashes.len());

    let (peer, mut stream, mut reader) = connect_archive()?;

    println!("archive P2P peer............. {peer}");

    let mut utxos = BTreeMap::<[u8; 32], UtxoEntry>::new();

    let mut kernel_sum = RistrettoPoint::identity();

    let mut stats = Stats::default();

    let mut negative_control_done = false;

    let mut expected_prev = parse_hex32(GENESIS)?;

    let mut next_height = 0u64;

    while next_height <= END_HEIGHT {
        let remaining = END_HEIGHT.saturating_sub(next_height).saturating_add(1);

        let limit =
            usize::try_from(remaining.min(P2P_PAGE as u64)).context("page size conversion")?;

        let blocks = fetch_blocks(&mut stream, &mut reader, next_height, limit)?;

        if blocks.is_empty() {
            bail!("archive returned empty block page at height {next_height}");
        }

        for block in &blocks {
            if next_height > END_HEIGHT {
                break;
            }

            let hash = process_block(
                block,
                next_height,
                &expected_prev,
                &canonical_hashes,
                &mut utxos,
                &mut kernel_sum,
                &mut negative_control_done,
                &mut stats,
            )?;

            expected_prev = hash;

            next_height = next_height.checked_add(1).context("height overflow")?;
        }

        eprintln!(
            "state replay progress: {}/{} blocks",
            next_height,
            END_HEIGHT + 1,
        );
    }

    if next_height != END_HEIGHT + 1 {
        bail!("replay incomplete: stopped at {next_height}");
    }

    if stats.inputs_verified == 0 {
        bail!("no real input was statefully verified");
    }

    if !negative_control_done {
        bail!("negative signature control never executed");
    }

    println!();
    println!("========================================");
    println!("PHASE 2C-3 STATE REPLAY PASS");
    println!("blocks replayed............. {}", stats.blocks);
    println!("transfer blocks observed.... {}", stats.transfer_blocks);
    println!("input signatures verified... {}", stats.inputs_verified);
    println!("coinbase spends verified.... {}", stats.coinbase_spends);
    println!("plain-output spends......... {}", stats.plain_spends);
    println!("UTXO roots matched.......... {}", stats.root_checks);
    println!("kernel sums matched......... {}", stats.kernel_sum_checks);
    println!("unknown/double spends....... independently rejected");
    println!("coinbase maturity........... independently enforced");
    println!("input ownership............. independently verified");
    println!("body anchoring.............. independently verified");
    println!("canonical block anchoring... public Mainnet matched");
    println!("tampered signature control.. correctly rejected");
    println!("production code............. unchanged");
    println!("========================================");

    Ok(())
}
