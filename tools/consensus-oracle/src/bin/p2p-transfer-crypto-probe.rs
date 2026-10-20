use anyhow::{bail, Context, Result};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
use merlin::Transcript;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

const SCHNORR_DOMAIN: &[u8] = b"nightfall:schnorr:v2";

const KERNEL_DOMAIN: &[u8] = b"nightfall:kernel:v2";

const OUTPUT_SIG_DOMAIN: &[u8] = b"nightfall:output:sig:v2";

const TXBODY_DOMAIN: &[u8] = b"nightfall:txbody:v2";

const RANGE_TRANSCRIPT: &[u8] = b"nightfall:rangeproof:v2";

const MAINNET_PROOF_CONTEXT: &[u8] = b"nightfall:mainnet:v8";

const TARGET_TRANSFER_BLOCKS: usize = 8;
const HEADER_PAGE: u64 = 512;

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
    let arr = v
        .as_array()
        .with_context(|| format!("{name}: expected byte array"))?;

    let mut out = Vec::with_capacity(arr.len());

    for (i, n) in arr.iter().enumerate() {
        let n = n
            .as_u64()
            .with_context(|| format!("{name}[{i}]: expected byte"))?;

        if n > 255 {
            bail!("{name}[{i}]: byte outside range");
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

fn schnorr_challenge(r: &[u8; 32], p: &[u8; 32], msg: &[u8]) -> Scalar {
    let a = hash_multi(SCHNORR_DOMAIN, &[r, p, msg, b"c0"]);

    let b = hash_multi(SCHNORR_DOMAIN, &[r, p, msg, b"c1"]);

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

fn kernel_message(kernel: &Value) -> Result<[u8; 32]> {
    let feature = feature_byte(
        kernel.get("feature").context("kernel feature missing")?,
        "kernel.feature",
    )?;

    let fee = u64_field(kernel, "fee_darks")?;

    let reward = u64_field(kernel, "reward_darks")?;

    let lock = u64_field(kernel, "lock_height")?;

    let excess = bytes32(
        kernel.get("excess").context("kernel excess missing")?,
        "kernel.excess",
    )?;

    Ok(hash_multi(
        KERNEL_DOMAIN,
        &[
            &[feature],
            &fee.to_le_bytes(),
            &reward.to_le_bytes(),
            &lock.to_le_bytes(),
            &excess,
        ],
    ))
}

fn kernel_id(kernel: &Value) -> Result<[u8; 32]> {
    let excess = bytes32(
        kernel.get("excess").context("kernel excess missing")?,
        "kernel.excess",
    )?;

    let sig = kernel
        .get("excess_sig")
        .context("kernel excess_sig missing")?;

    let (r, _) = signature(sig, "kernel.excess_sig")?;

    Ok(hash_multi(KERNEL_DOMAIN, &[&excess, &r]))
}

fn output_commitment_bytes(output: &Value) -> Result<Vec<u8>> {
    let feature = feature_byte(
        output.get("features").context("output features missing")?,
        "output.features",
    )?;

    let commit = bytes32(
        output.get("commit").context("output commit missing")?,
        "output.commit",
    )?;

    let ephemeral = bytes32(
        output
            .get("ephemeral_pk")
            .context("output ephemeral_pk missing")?,
        "output.ephemeral_pk",
    )?;

    let output_pk = bytes32(
        output
            .get("output_pk")
            .context("output output_pk missing")?,
        "output.output_pk",
    )?;

    let view_tag = u64_field(output, "view_tag")?;

    if view_tag > 255 {
        bail!("output view_tag outside byte range");
    }

    let proof = bytes(
        output
            .get("range_proof")
            .context("output range_proof missing")?,
        "output.range_proof",
    )?;

    let payload = bytes(
        output.get("payload").context("output payload missing")?,
        "output.payload",
    )?;

    let mut out = Vec::with_capacity(1 + 32 + 32 + 32 + 1 + proof.len() + payload.len());

    out.push(feature);
    out.extend_from_slice(&commit);
    out.extend_from_slice(&ephemeral);
    out.extend_from_slice(&output_pk);
    out.push(view_tag as u8);
    out.extend_from_slice(&proof);
    out.extend_from_slice(&payload);

    Ok(out)
}

fn verify_range_proof(output: &Value) -> Result<()> {
    let commitment = bytes32(
        output.get("commit").context("output commit missing")?,
        "output.commit",
    )?;

    let proof_bytes = bytes(
        output
            .get("range_proof")
            .context("output range_proof missing")?,
        "output.range_proof",
    )?;

    let proof = RangeProof::from_bytes(&proof_bytes)
        .map_err(|e| anyhow::anyhow!("range proof parse failed: {e}"))?;

    let bp = BulletproofGens::new(64, 1);

    let pc = PedersenGens::default();

    let mut transcript = Transcript::new(RANGE_TRANSCRIPT);

    transcript.append_message(b"ctx", MAINNET_PROOF_CONTEXT);

    proof
        .verify_single(
            &bp,
            &pc,
            &mut transcript,
            &CompressedRistretto(commitment),
            64,
        )
        .map_err(|e| anyhow::anyhow!("range proof verification failed: {e}"))
}

fn verify_output_signature(output: &Value) -> Result<()> {
    let ephemeral = bytes32(
        output
            .get("ephemeral_pk")
            .context("output ephemeral_pk missing")?,
        "output.ephemeral_pk",
    )?;

    let public = CompressedRistretto(ephemeral)
        .decompress()
        .context("malformed output ephemeral_pk")?;

    let output_pk = bytes32(
        output
            .get("output_pk")
            .context("output output_pk missing")?,
        "output.output_pk",
    )?;

    if CompressedRistretto(output_pk).decompress().is_none() {
        bail!("malformed output one-time key");
    }

    let commitment = bytes32(
        output.get("commit").context("output commit missing")?,
        "output.commit",
    )?;

    if CompressedRistretto(commitment).decompress().is_none() {
        bail!("malformed output commitment");
    }

    let commitment_bytes = output_commitment_bytes(output)?;

    let msg = hash_multi(OUTPUT_SIG_DOMAIN, &[&commitment_bytes]);

    let sig = output
        .get("sender_sig")
        .context("output sender_sig missing")?;

    let (r, s) = signature(sig, "output.sender_sig")?;

    let g = PedersenGens::default().B;

    if !verify_schnorr(&public, &g, &msg, r, s) {
        bail!("invalid output sender signature");
    }

    Ok(())
}

fn verify_kernel_signature(kernel: &Value, height: u64) -> Result<()> {
    let feature = feature_byte(
        kernel.get("feature").context("kernel feature missing")?,
        "kernel.feature",
    )?;

    let fee = u64_field(kernel, "fee_darks")?;

    let reward = u64_field(kernel, "reward_darks")?;

    let lock = u64_field(kernel, "lock_height")?;

    match feature {
        0 => {
            if reward != 0 {
                bail!("reward on plain kernel");
            }

            if lock > height {
                bail!("plain kernel locked until {lock}");
            }
        }

        1 => {
            if fee != 0 {
                bail!("fee on coinbase kernel");
            }

            if lock != height {
                bail!("coinbase lock height mismatch");
            }
        }

        _ => unreachable!(),
    }

    let excess_bytes = bytes32(
        kernel.get("excess").context("kernel excess missing")?,
        "kernel.excess",
    )?;

    let excess = CompressedRistretto(excess_bytes)
        .decompress()
        .context("malformed kernel excess")?;

    if excess == RistrettoPoint::identity() {
        bail!("identity kernel excess");
    }

    let msg = kernel_message(kernel)?;

    let sig = kernel
        .get("excess_sig")
        .context("kernel excess_sig missing")?;

    let (r, s) = signature(sig, "kernel.excess_sig")?;

    let h = PedersenGens::default().B_blinding;

    if !verify_schnorr(&excess, &h, &msg, r, s) {
        bail!("invalid kernel excess signature");
    }

    Ok(())
}

fn verify_balance(body: &Value) -> Result<()> {
    let inputs = array_field(body, "inputs")?;

    let outputs = array_field(body, "outputs")?;

    let kernels = array_field(body, "kernels")?;

    let gens = PedersenGens::default();

    let mut expected = RistrettoPoint::identity();

    for output in outputs {
        let commit = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        let point = CompressedRistretto(commit)
            .decompress()
            .context("malformed output commitment")?;

        expected += point;
    }

    for input in inputs {
        let commit = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        let point = CompressedRistretto(commit)
            .decompress()
            .context("malformed input commitment")?;

        expected -= point;
    }

    let mut total_fee = 0u64;
    let mut total_reward = 0u64;
    let mut kernel_sum = RistrettoPoint::identity();

    for kernel in kernels {
        total_fee = total_fee
            .checked_add(u64_field(kernel, "fee_darks")?)
            .context("fee total overflow")?;

        total_reward = total_reward
            .checked_add(u64_field(kernel, "reward_darks")?)
            .context("reward total overflow")?;

        let excess = bytes32(
            kernel.get("excess").context("kernel excess missing")?,
            "kernel.excess",
        )?;

        kernel_sum += CompressedRistretto(excess)
            .decompress()
            .context("malformed kernel excess")?;
    }

    expected += gens.B * Scalar::from(total_fee);

    expected -= gens.B * Scalar::from(total_reward);

    if expected != kernel_sum {
        bail!("aggregate Pedersen balance mismatch");
    }

    Ok(())
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

        let mut part = Vec::with_capacity(96);
        part.extend_from_slice(&commit);
        part.extend_from_slice(&r);
        part.extend_from_slice(&s);
        parts.push(part);
    }

    for output in outputs {
        let mut part = output_commitment_bytes(output)?;

        let sig = output
            .get("sender_sig")
            .context("output sender_sig missing")?;

        let (r, s) = signature(sig, "output.sender_sig")?;

        part.extend_from_slice(&r);
        part.extend_from_slice(&s);
        parts.push(part);
    }

    for kernel in kernels {
        let message = kernel_message(kernel)?;

        let sig = kernel
            .get("excess_sig")
            .context("kernel excess_sig missing")?;

        let (r, s) = signature(sig, "kernel.excess_sig")?;

        let mut part = Vec::with_capacity(96);
        part.extend_from_slice(&message);
        part.extend_from_slice(&r);
        part.extend_from_slice(&s);
        parts.push(part);
    }

    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();

    Ok(hash_multi(TXBODY_DOMAIN, &refs))
}

fn verify_canonical(body: &Value) -> Result<()> {
    let inputs = array_field(body, "inputs")?;

    let outputs = array_field(body, "outputs")?;

    let kernels = array_field(body, "kernels")?;

    let mut prev = None::<[u8; 32]>;

    for input in inputs {
        let current = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        if prev.is_some_and(|p| current < p) {
            bail!("non-canonical input order");
        }

        prev = Some(current);
    }

    prev = None;

    for output in outputs {
        let current = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        if prev.is_some_and(|p| current < p) {
            bail!("non-canonical output order");
        }

        prev = Some(current);
    }

    prev = None;

    for kernel in kernels {
        let current = kernel_id(kernel)?;

        if prev.is_some_and(|p| current < p) {
            bail!("non-canonical kernel order");
        }

        prev = Some(current);
    }

    Ok(())
}

fn verify_transfer_block(block: &Value) -> Result<(usize, usize, usize)> {
    let header = block.get("header").context("header missing")?;

    let body = block.get("body").context("body missing")?;

    let height = u64_field(header, "height")?;

    let inputs = array_field(body, "inputs")?;

    let outputs = array_field(body, "outputs")?;

    let kernels = array_field(body, "kernels")?;

    if inputs.is_empty() {
        bail!("height {height}: selected block has no inputs");
    }

    verify_canonical(body)?;

    let independent_body = body_hash(body)?;

    let header_body = bytes32(
        header.get("body_root").context("body_root missing")?,
        "header.body_root",
    )?;

    if independent_body != header_body {
        bail!("height {height}: BODY ROOT MISMATCH");
    }

    let mut seen_inputs = BTreeSet::new();

    for input in inputs {
        let c = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        if !seen_inputs.insert(c) {
            bail!("height {height}: duplicate input");
        }
    }

    let mut seen_outputs = BTreeSet::new();

    for output in outputs {
        let c = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        if !seen_outputs.insert(c) {
            bail!("height {height}: duplicate output");
        }

        verify_range_proof(output).with_context(|| format!("height {height}: range proof"))?;

        verify_output_signature(output)
            .with_context(|| format!("height {height}: output signature"))?;
    }

    let coinbase_kernels = kernels
        .iter()
        .filter(|k| k.get("feature").and_then(Value::as_str) == Some("coinbase"))
        .count();

    if coinbase_kernels != 1 {
        bail!("height {height}: coinbase kernel count={coinbase_kernels}");
    }

    let coinbase_outputs = outputs
        .iter()
        .filter(|o| o.get("features").and_then(Value::as_str) == Some("coinbase"))
        .count();

    if coinbase_outputs != 1 {
        bail!("height {height}: coinbase output count={coinbase_outputs}");
    }

    for kernel in kernels {
        verify_kernel_signature(kernel, height)
            .with_context(|| format!("height {height}: kernel verification"))?;
    }

    verify_balance(body).with_context(|| format!("height {height}: balance equation"))?;

    Ok((inputs.len(), outputs.len(), kernels.len()))
}

fn http_headers(from: u64) -> Result<(u64, Vec<Value>)> {
    let body = json!({
        "method": "get_headers",
        "params": {
            "from": from,
            "limit": HEADER_PAGE
        },
        "id": 991
    });

    let request = serde_json::to_string(&body)?;

    loop {
        let result = ureq::post("http://seed.nightfallcoin.org/")
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("User-Agent", "nightfall-independent-consensus-oracle/0.1")
            .send_string(&request);

        let response = match result {
            Ok(v) => v,

            Err(ureq::Error::Status(429, _)) => {
                eprintln!("Light API rate limit; retrying after reset...");

                std::thread::sleep(Duration::from_secs(61));

                continue;
            }

            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();

                bail!("HTTP {code}: {body}");
            }

            Err(e) => {
                bail!("header API transport error: {e}");
            }
        };

        let text = response
            .into_string()
            .context("read get_headers response")?;

        let root: Value = serde_json::from_str(&text).context("decode get_headers response")?;

        if let Some(err) = root.get("error").filter(|v| !v.is_null()) {
            bail!("get_headers error: {err}");
        }

        let result = root.get("result").context("get_headers missing result")?;

        let tip = u64_field(result, "tip_height")?;

        let headers = result
            .get("headers")
            .and_then(Value::as_array)
            .context("get_headers missing headers")?
            .clone();

        return Ok((tip, headers));
    }
}

fn find_transfer_heights() -> Result<Vec<u64>> {
    println!("searching chain for real transfer blocks...");

    let mut from = 0u64;
    let mut found = Vec::new();
    let mut tip: u64;

    loop {
        let (current_tip, headers) = http_headers(from)?;

        tip = current_tip;

        if headers.is_empty() {
            break;
        }

        for header in &headers {
            let height = u64_field(header, "height")?;

            let inputs = header.get("inputs").and_then(Value::as_u64);

            let kernels = header.get("kernels").and_then(Value::as_u64);

            if inputs.unwrap_or(0) > 0 || kernels.unwrap_or(1) > 1 {
                found.push(height);

                println!(
                    "transfer candidate: height {} inputs={} kernels={}",
                    height,
                    inputs.unwrap_or(0),
                    kernels.unwrap_or(0),
                );

                if found.len() >= TARGET_TRANSFER_BLOCKS {
                    return Ok(found);
                }
            }
        }

        let last = headers
            .last()
            .and_then(|h| h.get("height").and_then(Value::as_u64))
            .context("header page missing final height")?;

        from = last.saturating_add(1);

        if from > tip {
            break;
        }

        if from % 10_000 < HEADER_PAGE {
            eprintln!("searched through height {from}/{tip}");
        }
    }

    println!("chain scan complete at tip {tip}");

    Ok(found)
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
                bail!("peer is not archive");
            }

            Ok((peer.to_string(), stream, reader))
        })();

        match attempt {
            Ok(v) => return Ok(v),

            Err(e) => {
                errors.push(format!("{peer}: {e:#}"));
            }
        }
    }

    bail!("no archive peer:\n{}", errors.join("\n"))
}

fn fetch_block(
    stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    height: u64,
) -> Result<Value> {
    write_line(
        stream,
        &json!({
            "type": "get_blocks",
            "from_height": height,
            "limit": 1
        }),
    )?;

    loop {
        let msg = read_line(reader)?;

        match msg.get("type").and_then(Value::as_str) {
            Some("blocks") => {
                let blocks = msg
                    .get("blocks")
                    .and_then(Value::as_array)
                    .context("blocks response missing blocks")?;

                let block = blocks
                    .first()
                    .with_context(|| format!("no block returned at {height}"))?;

                return Ok(block.clone());
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
                bail!("P2P error: {msg}");
            }

            _ => {}
        }
    }
}

fn main() -> Result<()> {
    println!("NIGHTFALL independent transfer crypto probe");

    println!("Nightfall consensus imports: none");

    println!("Nightfall ledger imports:    none");

    println!("Nightfall crypto imports:    none");

    println!("underlying primitives:       independent crate APIs");

    let heights = find_transfer_heights()?;

    if heights.is_empty() {
        bail!(
            "NO TRANSFER BLOCKS FOUND: \
             stateful/transfer validation cannot be claimed"
        );
    }

    println!();
    println!("transfer blocks selected: {}", heights.len());

    let (peer, mut stream, mut reader) = connect_archive()?;

    println!("archive peer: {peer}");

    let mut verified = 0usize;
    let mut input_total = 0usize;
    let mut output_total = 0usize;
    let mut kernel_total = 0usize;

    for height in heights {
        println!();
        println!("verifying transfer block {height}...");

        let block = fetch_block(&mut stream, &mut reader, height)?;

        let actual_height = block
            .pointer("/header/height")
            .and_then(Value::as_u64)
            .context("returned block missing height")?;

        if actual_height != height {
            bail!("requested {height}, got {actual_height}");
        }

        let (ins, outs, kernels) = verify_transfer_block(&block)?;

        println!(
            "height {:>7}: TRANSFER CRYPTO PASS | inputs={} outputs={} kernels={}",
            height, ins, outs, kernels,
        );

        input_total += ins;
        output_total += outs;
        kernel_total += kernels;
        verified += 1;
    }

    println!();
    println!("========================================");
    println!("PHASE 2C-2 TRANSFER CRYPTO PASS");
    println!("transfer blocks verified... {verified}");
    println!("inputs observed............ {input_total}");
    println!("outputs verified........... {output_total}");
    println!("kernels verified........... {kernel_total}");
    println!("body roots................. independently verified");
    println!("canonical ordering......... independently verified");
    println!("Bulletproof range proofs... independently verified");
    println!("output Schnorr signatures.. independently verified");
    println!("kernel Schnorr signatures.. independently verified");
    println!("Pedersen balance equation.. independently verified");
    println!("input ownership signatures. NOT YET VERIFIED");
    println!("UTXO existence/maturity.... NOT YET VERIFIED");
    println!("UTXO root transition....... NOT YET VERIFIED");
    println!("production code............ unchanged");
    println!("========================================");

    Ok(())
}
