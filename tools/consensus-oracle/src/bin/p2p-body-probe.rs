use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";

const DOMAIN_TXBODY: &[u8] = b"nightfall:txbody:v2";
const DOMAIN_KERNEL: &[u8] = b"nightfall:kernel:v2";

const MAX_BLOCK_INPUTS: usize = 4096;
const MAX_BLOCK_OUTPUTS: usize = 4096;
const MAX_BLOCK_KERNELS: usize = 1024;

const INITIAL_REWARD: u64 = 6 * 100_000_000;
const HALVING_INTERVAL: u64 = 7_500_000;

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
    let v = bytes(v, name)?;

    if v.len() != 32 {
        bail!("{name}: expected 32 bytes, got {}", v.len());
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&v);

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

fn kernel_signing_message(k: &Value) -> Result<[u8; 32]> {
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

fn kernel_id(k: &Value) -> Result<[u8; 32]> {
    let excess = bytes32(
        k.get("excess").context("kernel excess missing")?,
        "kernel.excess",
    )?;

    let sig = k.get("excess_sig").context("kernel excess_sig missing")?;

    let (r, _) = signature(sig, "kernel.excess_sig")?;

    Ok(hash_multi(DOMAIN_KERNEL, &[&excess, &r]))
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
        bail!("output.view_tag outside byte range");
    }

    let proof = bytes(
        o.get("range_proof").context("output range_proof missing")?,
        "output.range_proof",
    )?;

    let payload = bytes(
        o.get("payload").context("output payload missing")?,
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

fn body_hash(body: &Value) -> Result<[u8; 32]> {
    let inputs = array_field(body, "inputs")?;
    let outputs = array_field(body, "outputs")?;
    let kernels = array_field(body, "kernels")?;

    let mut parts: Vec<Vec<u8>> = Vec::with_capacity(inputs.len() + outputs.len() + kernels.len());

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
        let message = kernel_signing_message(kernel)?;

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

    Ok(hash_multi(DOMAIN_TXBODY, &refs))
}

fn reward_at(height: u64) -> u64 {
    let halvings = height / HALVING_INTERVAL;

    if halvings >= 64 {
        0
    } else {
        INITIAL_REWARD >> halvings
    }
}

fn verify_canonical(body: &Value) -> Result<()> {
    let inputs = array_field(body, "inputs")?;
    let outputs = array_field(body, "outputs")?;
    let kernels = array_field(body, "kernels")?;

    let mut previous: Option<[u8; 32]> = None;

    for input in inputs {
        let current = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        if let Some(prev) = previous {
            if current < prev {
                bail!("inputs are not canonically sorted");
            }
        }

        previous = Some(current);
    }

    previous = None;

    for output in outputs {
        let current = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        if let Some(prev) = previous {
            if current < prev {
                bail!("outputs are not canonically sorted");
            }
        }

        previous = Some(current);
    }

    previous = None;

    for kernel in kernels {
        let current = kernel_id(kernel)?;

        if let Some(prev) = previous {
            if current < prev {
                bail!("kernels are not canonically sorted");
            }
        }

        previous = Some(current);
    }

    Ok(())
}

fn verify_body(block: &Value) -> Result<()> {
    let header = block.get("header").context("block header missing")?;

    let body = block.get("body").context("block body missing")?;

    let height = u64_field(header, "height")?;

    let inputs = array_field(body, "inputs")?;
    let outputs = array_field(body, "outputs")?;
    let kernels = array_field(body, "kernels")?;

    if outputs.is_empty() {
        bail!("height {height}: empty output set");
    }

    if kernels.is_empty() {
        bail!("height {height}: empty kernel set");
    }

    if inputs.len() > MAX_BLOCK_INPUTS
        || outputs.len() > MAX_BLOCK_OUTPUTS
        || kernels.len() > MAX_BLOCK_KERNELS
    {
        bail!(
            "height {height}: block size limit exceeded: \
             inputs={} outputs={} kernels={}",
            inputs.len(),
            outputs.len(),
            kernels.len()
        );
    }

    verify_canonical(body).with_context(|| format!("height {height}"))?;

    let calculated_body = body_hash(body)?;

    let expected_body = bytes32(
        header
            .get("body_root")
            .context("header body_root missing")?,
        "header.body_root",
    )?;

    if calculated_body != expected_body {
        bail!("height {height}: BODY ROOT MISMATCH");
    }

    let mut input_set = BTreeSet::new();

    for input in inputs {
        let commit = bytes32(
            input.get("commit").context("input commit missing")?,
            "input.commit",
        )?;

        if !input_set.insert(commit) {
            bail!("height {height}: duplicate input commitment");
        }
    }

    let mut output_set = BTreeSet::new();
    let mut coinbase_outputs = 0usize;

    for output in outputs {
        let commit = bytes32(
            output.get("commit").context("output commit missing")?,
            "output.commit",
        )?;

        if !output_set.insert(commit) {
            bail!("height {height}: duplicate output commitment");
        }

        let feature = feature_byte(
            output.get("features").context("output features missing")?,
            "output.features",
        )?;

        if feature == 1 {
            coinbase_outputs += 1;
        }
    }

    if coinbase_outputs != 1 {
        bail!(
            "height {height}: expected exactly one \
             coinbase output, got {coinbase_outputs}"
        );
    }

    let mut coinbase_kernels = 0usize;
    let mut coinbase_reward = None;
    let mut total_fees = 0u64;

    for kernel in kernels {
        let feature = feature_byte(
            kernel.get("feature").context("kernel feature missing")?,
            "kernel.feature",
        )?;

        let fee = u64_field(kernel, "fee_darks")?;
        let reward = u64_field(kernel, "reward_darks")?;
        let lock = u64_field(kernel, "lock_height")?;

        total_fees = total_fees.checked_add(fee).context("fee total overflow")?;

        match feature {
            0 => {
                if reward != 0 {
                    bail!("height {height}: reward on plain kernel");
                }

                if lock > height {
                    bail!(
                        "height {height}: plain kernel locked \
                         until {lock}"
                    );
                }
            }

            1 => {
                coinbase_kernels += 1;

                if fee != 0 {
                    bail!("height {height}: fee on coinbase kernel");
                }

                if lock != height {
                    bail!(
                        "height {height}: coinbase lock height \
                         {lock} != {height}"
                    );
                }

                coinbase_reward = Some(reward);
            }

            _ => unreachable!(),
        }
    }

    if coinbase_kernels != 1 {
        bail!(
            "height {height}: expected exactly one \
             coinbase kernel, got {coinbase_kernels}"
        );
    }

    let subsidy = reward_at(height);

    let due = if subsidy > 0 { subsidy } else { total_fees };

    let got = coinbase_reward.context("coinbase reward missing")?;

    if got != due {
        bail!(
            "height {height}: coinbase reward mismatch: \
             got={got} expected={due}"
        );
    }

    let header_reward = u64_field(header, "reward_darks")?;

    if header_reward != due {
        bail!(
            "height {height}: header reward mismatch: \
             got={header_reward} expected={due}"
        );
    }

    Ok(())
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

fn connect_archive() -> Result<(String, TcpStream, BufReader<TcpStream>, u64)> {
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

            let first = hello
                .get("first_height")
                .and_then(Value::as_u64)
                .unwrap_or(0);

            if pruned || first != 0 {
                bail!(
                    "not archive: pruned={pruned}, \
                     first_height={first}"
                );
            }

            Ok((peer.to_string(), stream, reader, height))
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
        let msg = read_json_line(reader)?;

        match msg.get("type").and_then(Value::as_str) {
            Some("blocks") => {
                return Ok(msg
                    .get("blocks")
                    .and_then(Value::as_array)
                    .context("blocks response has no blocks")?
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
                bail!("P2P error: {msg}");
            }

            _ => {}
        }
    }
}

fn main() -> Result<()> {
    println!("NIGHTFALL independent block-body probe");

    println!("Nightfall consensus imports: none");
    println!("Nightfall ledger imports:    none");
    println!("Nightfall crypto imports:    none");

    let (peer, mut stream, mut reader, peer_height) = connect_archive()?;

    println!("archive peer: {peer}");
    println!("peer height:  {peer_height}");

    let mut starts = vec![0, 24_996, peer_height.saturating_sub(7)];

    starts.sort_unstable();
    starts.dedup();

    let mut verified = 0usize;

    for from in starts {
        println!();
        println!("checking bodies from height {from}...");

        let blocks = fetch_blocks(&mut stream, &mut reader, from, 8)?;

        if blocks.is_empty() {
            bail!("no blocks returned from height {from}");
        }

        for block in &blocks {
            let height = block
                .pointer("/header/height")
                .and_then(Value::as_u64)
                .context("block height missing")?;

            verify_body(block)?;

            let body = block.get("body").context("body missing")?;

            println!(
                "height {:>7}: BODY PASS | \
                 inputs={} outputs={} kernels={}",
                height,
                array_field(body, "inputs")?.len(),
                array_field(body, "outputs")?.len(),
                array_field(body, "kernels")?.len(),
            );

            verified += 1;
        }
    }

    println!();
    println!("========================================");
    println!("PHASE 2C-1 BODY PROBE PASS");
    println!("verified blocks......... {verified}");
    println!("coverage................ genesis/checkpoint/tip");
    println!("body root............... independently rebuilt");
    println!("canonical ordering...... independently checked");
    println!("aggregate limits........ independently checked");
    println!("duplicate inputs........ independently checked");
    println!("duplicate outputs....... independently checked");
    println!("coinbase kernel count... independently checked");
    println!("coinbase output count... independently checked");
    println!("kernel shape............ independently checked");
    println!("reward schedule......... independently checked");
    println!("coinbase height......... independently checked");
    println!("production code......... unchanged");
    println!("========================================");

    Ok(())
}
