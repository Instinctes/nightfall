use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
};

const MAINNET_GENESIS: &str = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de";
const PROTOCOL_VERSION: u64 = 8;

const DARKS_PER_NIGHT: u64 = 100_000_000;
const INITIAL_REWARD_DARKS: u64 = 6 * DARKS_PER_NIGHT;
const HALVING_INTERVAL: u64 = 7_500_000;
const MAX_SUPPLY_DARKS: u128 = 90_000_000u128 * DARKS_PER_NIGHT as u128;

const TARGET_SECS: i64 = 15;
const DIFFICULTY_WINDOW: usize = 90;
const INITIAL_DIFFICULTY: u64 = 5_000;
const MIN_DIFFICULTY: u64 = 2_000;
const LWMA_MIN_FACTOR: i64 = -5;
const LWMA_MAX_FACTOR: i64 = 6;
const MTP_WINDOW: usize = 11;

const CHECKPOINT_HEIGHT: u64 = 25_000;
const CHECKPOINT_HASH: &str = "71c8b97b1e8a20f9f29d9e873110e41223b0c96aa9ce773feaee78c97bdd247e";

fn rpc(addr: &str, method: &str, params: Value, id: u64) -> Result<Value> {
    if let Some(endpoint) = addr.strip_prefix("http://") {
        return rpc_http(endpoint, method, params, id);
    }

    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect RPC {addr}"))?;
    stream.set_nodelay(true).context("set TCP_NODELAY")?;

    let request = json!({
        "method": method,
        "params": params,
        "id": id
    });

    writeln!(stream, "{}", serde_json::to_string(&request)?)?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;

    if line.trim().is_empty() {
        bail!("empty RPC response for {method}");
    }

    let response: Value =
        serde_json::from_str(&line).with_context(|| format!("decode RPC {method}"))?;

    if let Some(error) = response.get("error").filter(|v| !v.is_null()) {
        bail!("RPC {method}: {error}");
    }

    response
        .get("result")
        .cloned()
        .context("RPC response has no result")
}

fn rpc_http(endpoint: &str, method: &str, params: Value, id: u64) -> Result<Value> {
    let url = format!("http://{endpoint}");

    let request_body = serde_json::to_string(&json!({
        "method": method,
        "params": params,
        "id": id
    }))?;

    loop {
        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("User-Agent", "nightfall-independent-consensus-oracle/0.1")
            .send_string(&request_body);

        let body = match response {
            Ok(response) => response
                .into_string()
                .with_context(|| format!("read Light API response for {method}"))?,

            Err(ureq::Error::Status(429, _)) => {
                eprintln!("Light API rate limit reached; continuing automatically after reset...");
                std::thread::sleep(std::time::Duration::from_secs(61));
                continue;
            }

            Err(ureq::Error::Status(code, response)) => {
                let body = response
                    .into_string()
                    .unwrap_or_else(|_| "<unreadable body>".to_string());

                bail!("Light API HTTP {code} for {method}: {body}");
            }

            Err(error) => {
                bail!("Light API transport error for {method}: {error}");
            }
        };

        let response: Value = serde_json::from_str(&body).with_context(|| {
            format!(
                "decode Light API response for {method}; body={}",
                body.chars().take(500).collect::<String>()
            )
        })?;

        if let Some(error) = response.get("error").filter(|v| !v.is_null()) {
            bail!("Light API {method}: {error}");
        }

        return response
            .get("result")
            .cloned()
            .context("Light API response has no result");
    }
}
fn u64_field(v: &Value, name: &str) -> Result<u64> {
    v.get(name)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing/invalid u64 field `{name}`"))
}

fn str_field<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    v.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("missing/invalid string field `{name}`"))
}

fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn subsidy(height: u64) -> u64 {
    let era = height / HALVING_INTERVAL;
    if era >= 64 {
        0
    } else {
        INITIAL_REWARD_DARKS >> era
    }
}

/// Independent implementation of Nightfall's LWMA-1 rule.
///
/// This intentionally does not call nightfall-consensus.
fn expected_difficulty(history: &[(u64, u64)]) -> u64 {
    if history.len() < DIFFICULTY_WINDOW + 1 {
        return INITIAL_DIFFICULTY.max(MIN_DIFFICULTY);
    }

    let w = &history[history.len() - (DIFFICULTY_WINDOW + 1)..];

    let min_st = LWMA_MIN_FACTOR * TARGET_SECS;
    let max_st = LWMA_MAX_FACTOR * TARGET_SECS;

    let mut weighted_solvetime: i128 = 0;
    let mut difficulty_sum: u128 = 0;

    for i in 1..=DIFFICULTY_WINDOW {
        let prev = w[i - 1].0 as i128;
        let current = w[i].0 as i128;

        let raw = current - prev;
        let solve = raw.max(min_st as i128).min(max_st as i128);

        weighted_solvetime += solve * i as i128;
        difficulty_sum += w[i].1 as u128;
    }

    let k = DIFFICULTY_WINDOW as i128 * (DIFFICULTY_WINDOW as i128 + 1) * TARGET_SECS as i128 / 2;

    let minimum_weighted = k / 10;
    if weighted_solvetime < minimum_weighted {
        weighted_solvetime = minimum_weighted;
    }

    let avg = difficulty_sum / DIFFICULTY_WINDOW as u128;

    let next = avg.saturating_mul(k as u128) / weighted_solvetime.max(1) as u128;

    let lower = (avg / 2).max(1);
    let upper = avg.saturating_mul(2);
    let clamped = next.clamp(lower, upper);

    (clamped.min(u64::MAX as u128) as u64).max(MIN_DIFFICULTY)
}

fn median_time_past(history: &[(u64, u64)]) -> u64 {
    if history.is_empty() {
        return 0;
    }

    let take = history.len().min(MTP_WINDOW);
    let mut times: Vec<u64> = history[history.len() - take..]
        .iter()
        .map(|x| x.0)
        .collect();

    times.sort_unstable();
    times[times.len() / 2]
}

fn main() -> Result<()> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:17881".to_string());

    println!("NIGHTFALL independent consensus oracle");
    println!("RPC: {addr}");
    println!("NOTE: no nightfall-consensus or nightfall-ledger code is linked.\n");

    let status = rpc(&addr, "status", json!({}), 1)?;

    if str_field(&status, "network")? != "mainnet" {
        bail!("refusing non-mainnet node");
    }

    if u64_field(&status, "protocol_version")? != PROTOCOL_VERSION {
        bail!(
            "protocol mismatch: expected {}, got {}",
            PROTOCOL_VERSION,
            u64_field(&status, "protocol_version")?
        );
    }

    let genesis = str_field(&status, "genesis")?;
    if genesis != MAINNET_GENESIS {
        bail!("genesis mismatch: {genesis}");
    }

    if status
        .get("loading")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!("node is still loading; oracle requires a stable chain");
    }

    if status
        .get("reorg_in_flight")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!("reorg is in flight; retry on a stable tip");
    }

    if status
        .get("stalled_on_fork")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        bail!("node reports stalled_on_fork");
    }

    let pruned = status
        .get("pruned")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let prune_height = status
        .get("prune_height")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    if pruned || prune_height != 0 {
        bail!(
            "full independent replay requires an archive node; \
             node reports pruned={pruned}, prune_height={prune_height}"
        );
    }

    let tip_height = u64_field(&status, "tip_height")?;
    let advertised_tip = str_field(&status, "tip")?.to_owned();

    let mut next_height = 0u64;
    let mut expected_prev = MAINNET_GENESIS.to_owned();
    let mut history: Vec<(u64, u64)> = Vec::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();

    let mut cumulative_work: u128 = 0;
    let mut minted: u128 = 0;
    let mut last_hash = MAINNET_GENESIS.to_owned();
    let mut checkpoint_verified = tip_height < CHECKPOINT_HEIGHT;

    let mut rpc_id = 10u64;

    while next_height <= tip_height {
        let page = rpc(
            &addr,
            "get_headers",
            json!({
                "from": next_height,
                "limit": 512
            }),
            rpc_id,
        )?;
        rpc_id += 1;

        let headers = page
            .get("headers")
            .and_then(Value::as_array)
            .context("get_headers result has no headers array")?;

        if headers.is_empty() {
            bail!("get_headers returned empty page at height {next_height}");
        }

        for header in headers {
            let height = u64_field(header, "height")?;
            let hash = str_field(header, "hash")?.to_ascii_lowercase();
            let prev = str_field(header, "prev_hash")?.to_ascii_lowercase();
            let timestamp = u64_field(header, "time")?;
            let difficulty = u64_field(header, "difficulty")?;
            let reward = u64_field(header, "reward")?;

            if height > tip_height {
                break;
            }

            if height != next_height {
                bail!(
                    "height discontinuity: expected {}, received {}",
                    next_height,
                    height
                );
            }

            if !valid_hash(&hash) || !valid_hash(&prev) {
                bail!("malformed hash at height {height}");
            }

            if prev != expected_prev {
                bail!(
                    "parent mismatch at height {height}: expected {}, got {}",
                    expected_prev,
                    prev
                );
            }

            if !seen_hashes.insert(hash.clone()) {
                bail!("duplicate block hash at height {height}: {hash}");
            }

            let expected_reward = subsidy(height);
            if reward != expected_reward {
                bail!(
                    "emission mismatch at height {height}: expected {} darks, got {}",
                    expected_reward,
                    reward
                );
            }

            let expected_diff = expected_difficulty(&history);
            if difficulty != expected_diff {
                bail!(
                    "difficulty mismatch at height {height}: expected {}, got {}",
                    expected_diff,
                    difficulty
                );
            }

            let mtp = median_time_past(&history);
            if timestamp <= mtp {
                bail!(
                    "MTP violation at height {height}: timestamp {} <= median {}",
                    timestamp,
                    mtp
                );
            }

            if height == CHECKPOINT_HEIGHT {
                if hash != CHECKPOINT_HASH {
                    bail!(
                        "checkpoint mismatch at {}: expected {}, got {}",
                        CHECKPOINT_HEIGHT,
                        CHECKPOINT_HASH,
                        hash
                    );
                }
                checkpoint_verified = true;
            }

            cumulative_work = cumulative_work
                .checked_add(difficulty.max(1) as u128)
                .context("cumulative work overflow")?;

            minted = minted
                .checked_add(reward as u128)
                .context("minted supply overflow")?;

            if minted > MAX_SUPPLY_DARKS {
                bail!(
                    "maximum supply exceeded at height {height}: {} darks",
                    minted
                );
            }

            history.push((timestamp, difficulty));
            expected_prev = hash.clone();
            last_hash = hash;
            next_height += 1;

            if next_height.is_multiple_of(10_000) {
                eprintln!("{} headers independently checked", next_height);
            }
        }
    }

    if next_height != tip_height + 1 {
        bail!(
            "header count mismatch: reached {}, expected {}",
            next_height,
            tip_height + 1
        );
    }

    if last_hash != advertised_tip.to_ascii_lowercase() {
        bail!(
            "tip mismatch: independently walked {}, node advertises {}",
            last_hash,
            advertised_tip
        );
    }

    let advertised_blocks = u64_field(&status, "blocks")?;
    if advertised_blocks != next_height {
        bail!(
            "block count mismatch: walked {}, node advertises {}",
            next_height,
            advertised_blocks
        );
    }

    let node_work: u128 = str_field(&status, "total_work")?
        .parse()
        .context("invalid status.total_work")?;

    if node_work != cumulative_work {
        bail!(
            "total-work mismatch: oracle {}, node {}",
            cumulative_work,
            node_work
        );
    }

    let node_minted = u64_field(&status, "minted")? as u128;
    if node_minted != minted {
        bail!(
            "minted-supply mismatch: oracle {}, node {}",
            minted,
            node_minted
        );
    }

    if !checkpoint_verified {
        bail!("checkpoint {} was not observed", CHECKPOINT_HEIGHT);
    }

    if status.get("supply_invariant_ok").and_then(Value::as_bool) != Some(true) {
        bail!("node itself reports supply_invariant_ok != true");
    }

    println!("ORACLE PASS");
    println!("blocks............... {}", next_height);
    println!("tip height........... {}", tip_height);
    println!("tip.................. {}", last_hash);
    println!("cumulative work...... {}", cumulative_work);
    println!("minted darks......... {}", minted);
    println!(
        "checkpoint {}..... {}",
        CHECKPOINT_HEIGHT,
        if tip_height >= CHECKPOINT_HEIGHT {
            "verified"
        } else {
            "not reached yet"
        }
    );
    println!("emission............. independently matched");
    println!("LWMA difficulty...... independently matched");
    println!("MTP.................. independently matched");
    println!("parent chain......... independently matched");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emission_boundaries_are_independent() {
        assert_eq!(subsidy(0), 600_000_000);
        assert_eq!(subsidy(7_499_999), 600_000_000);
        assert_eq!(subsidy(7_500_000), 300_000_000);
        assert_eq!(subsidy(15_000_000), 150_000_000);
    }

    #[test]
    fn target_spacing_holds_difficulty() {
        let history: Vec<(u64, u64)> = (0..=DIFFICULTY_WINDOW)
            .map(|i| (1_000_000 + i as u64 * TARGET_SECS as u64, 1_000_000))
            .collect();

        let d = expected_difficulty(&history);
        assert!(
            (990_000..=1_010_000).contains(&d),
            "unexpected target-spacing difficulty: {d}"
        );
    }

    #[test]
    fn fast_chain_raises_difficulty() {
        let history: Vec<(u64, u64)> = (0..=DIFFICULTY_WINDOW)
            .map(|i| (1_000_000 + i as u64 * 5, 1_000_000))
            .collect();

        assert!(expected_difficulty(&history) > 1_000_000);
    }

    #[test]
    fn slow_chain_lowers_difficulty() {
        let history: Vec<(u64, u64)> = (0..=DIFFICULTY_WINDOW)
            .map(|i| (1_000_000 + i as u64 * 60, 1_000_000))
            .collect();

        assert!(expected_difficulty(&history) < 1_000_000);
    }

    #[test]
    fn median_is_not_last_timestamp() {
        let history: Vec<(u64, u64)> = (1..=11).map(|x| (x * 100, 5000)).collect();

        assert_eq!(median_time_past(&history), 600);
    }
}
