//! A fail-closed boundary for machine-generated payment intentions.
//!
//! This grants no signing power. A human-operated wallet reviews each request,
//! and retains full custody. See docs/AGENT-INTENTS.md.
use anyhow::{bail, Context, Result};
use nightfall_crypto::Address;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::Path;

const FORMAT: &str = "nightfall-agent-intent-v1";
const MAX_JSON_BYTES: u64 = 4096;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPolicy {
    pub format: String,
    pub network: String,
    pub wallet: String,
    pub agent_id: String,
    pub recipient: String,
    pub max_amount_darks: u64,
    pub max_fee_darks: u64,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIntent {
    pub format: String,
    pub network: String,
    pub wallet: String,
    pub agent_id: String,
    pub recipient: String,
    pub amount_darks: u64,
    pub fee_darks: u64,
    pub memo: String,
    pub nonce: String,
    pub created_at: u64,
    pub expires_at: u64,
}

pub struct CheckedIntent {
    pub recipient: Address,
    pub amount_darks: u64,
    pub fee_darks: u64,
    pub memo: String,
    pub nonce: String,
    pub expires_at: u64,
}

fn read_json<T: DeserializeOwned>(path: &Path, owner_policy: bool) -> Result<T> {
    let meta = std::fs::symlink_metadata(path).context("cannot inspect agent JSON file")?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_JSON_BYTES {
        bail!("agent JSON must be a regular file of at most 4096 bytes");
    }
    #[cfg(unix)]
    if owner_policy {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            bail!("owner policy requires private permissions (chmod 600)");
        }
    }
    #[cfg(not(unix))]
    let _ = owner_policy;
    let bytes = std::fs::read(path).context("cannot read agent JSON file")?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        bail!("agent JSON exceeds the size limit");
    }
    // Unknown/duplicate fields are rejected by serde. Never echo hostile JSON.
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid agent JSON schema"))
}

pub fn read_policy(path: &Path) -> Result<AgentPolicy> {
    read_json(path, true)
}

pub fn read_intent(path: &Path) -> Result<AgentIntent> {
    read_json(path, false)
}

fn valid_agent_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
}

fn valid_nonce(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && s.bytes().any(|b| b != b'0')
}

/// Check before any transaction construction or signing.
/// The machine clock and the owner-controlled policy file are trusted inputs.
/// This is a per-request check, NOT aggregate budgeting or replay prevention.
pub fn authorize(
    policy: &AgentPolicy,
    intent: AgentIntent,
    network: &str,
    owner_wallet: &str,
    now: u64,
) -> Result<CheckedIntent> {
    if policy.format != FORMAT || intent.format != FORMAT {
        bail!("unsupported agent format");
    }
    if policy.network != network || intent.network != network {
        bail!("agent intent network mismatch");
    }
    let owner = Address::decode(owner_wallet).context("owner address invalid")?;
    let allowed_wallet = Address::decode(&policy.wallet).context("policy address invalid")?;
    let claimed_wallet = Address::decode(&intent.wallet).context("intent address invalid")?;
    if owner != allowed_wallet || owner != claimed_wallet {
        bail!("agent intent wallet mismatch");
    }
    if !valid_agent_id(&policy.agent_id)
        || !valid_agent_id(&intent.agent_id)
        || policy.agent_id != intent.agent_id
    {
        bail!("agent identity mismatch");
    }
    let permitted = Address::decode(&policy.recipient).context("policy recipient invalid")?;
    let requested = Address::decode(&intent.recipient).context("intent recipient invalid")?;
    if permitted != requested {
        bail!("recipient not permitted");
    }
    if policy.expires_at <= now
        || intent.created_at > now
        || intent.expires_at <= now
        || intent.expires_at > policy.expires_at
        || intent.expires_at <= intent.created_at
        || intent.expires_at - intent.created_at > 300
    {
        bail!("intent is expired or outside its permitted window");
    }
    if intent.amount_darks == 0
        || intent.fee_darks == 0
        || policy.max_amount_darks == 0
        || policy.max_fee_darks == 0
        || intent.amount_darks > policy.max_amount_darks
        || intent.fee_darks > policy.max_fee_darks
        || intent.amount_darks.checked_add(intent.fee_darks).is_none()
    {
        bail!("amount or fee outside owner limits");
    }
    if intent.memo.len() > 64
        || !intent
            .memo
            .bytes()
            .all(|b| b == b' ' || (0x21..=0x7e).contains(&b))
    {
        bail!("unsafe memo characters or length");
    }
    if !valid_nonce(&intent.nonce) {
        bail!("nonce must be 32 lower-case hexadecimal characters, not all zero");
    }
    Ok(CheckedIntent {
        recipient: requested,
        amount_darks: intent.amount_darks,
        fee_darks: intent.fee_darks,
        memo: intent.memo,
        nonce: intent.nonce,
        expires_at: intent.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    fn address(seed: u8) -> String {
        WalletKeys::from_seed([seed; 32]).address().encode()
    }

    fn good() -> (AgentPolicy, AgentIntent) {
        let wallet = address(1);
        let recipient = address(2);
        (
            AgentPolicy {
                format: FORMAT.into(),
                network: "devnet".into(),
                wallet: wallet.clone(),
                agent_id: "test-agent".into(),
                recipient: recipient.clone(),
                max_amount_darks: 1000,
                max_fee_darks: 10,
                expires_at: 1200,
            },
            AgentIntent {
                format: FORMAT.into(),
                network: "devnet".into(),
                wallet,
                agent_id: "test-agent".into(),
                recipient,
                amount_darks: 900,
                fee_darks: 5,
                memo: "Invoice 17".into(),
                nonce: "1234567890abcdef1234567890abcdef".into(),
                created_at: 1000,
                expires_at: 1100,
            },
        )
    }

    fn allowed(p: &AgentPolicy, i: AgentIntent, now: u64) -> bool {
        authorize(p, i, "devnet", &address(1), now).is_ok()
    }

    #[test]
    fn accepts_bounded_request_without_signing() {
        let (p, i) = good();
        let c = authorize(&p, i, "devnet", &address(1), 1050).unwrap();
        assert_eq!((c.amount_darks, c.fee_darks), (900, 5));
        assert_eq!(c.memo, "Invoice 17");
    }

    #[test]
    fn rejects_wrong_wallet_network_agent_and_recipient() {
        let (p, mut i) = good();
        i.wallet = address(3);
        assert!(!allowed(&p, i, 1050));
        let (p, mut i) = good();
        i.network = "mainnet".into();
        assert!(!allowed(&p, i, 1050));
        let (p, mut i) = good();
        i.agent_id = "other".into();
        assert!(!allowed(&p, i, 1050));
        let (p, mut i) = good();
        i.recipient = address(3);
        assert!(!allowed(&p, i, 1050));
    }

    #[test]
    fn rejects_amount_fee_and_overflow() {
        for (amount, fee) in [(0, 1), (1, 0), (1001, 1), (1, 11), (u64::MAX, 10)] {
            let (p, mut i) = good();
            i.amount_darks = amount;
            i.fee_darks = fee;
            assert!(!allowed(&p, i, 1050));
        }
    }

    #[test]
    fn rejects_invalid_time_windows() {
        for (created, expires, now) in [
            (1000, 1100, 1100),
            (1100, 1150, 1050),
            (1000, 1301, 1050),
            (1000, 1000, 1000),
        ] {
            let (p, mut i) = good();
            i.created_at = created;
            i.expires_at = expires;
            assert!(!allowed(&p, i, now));
        }
        let (mut p, i) = good();
        p.expires_at = 1000;
        assert!(!allowed(&p, i, 1050));
    }

    #[test]
    fn rejects_review_spoofing_and_bad_nonces() {
        for memo in [
            "\nAPPROVE".to_string(),
            "\u{1b}[2J".into(),
            "a\u{202e}b".into(),
            "x".repeat(65),
        ] {
            let (p, mut i) = good();
            i.memo = memo;
            assert!(!allowed(&p, i, 1050));
        }
        for nonce in ["".to_string(), "0".repeat(32), "Z".repeat(32)] {
            let (p, mut i) = good();
            i.nonce = nonce;
            assert!(!allowed(&p, i, 1050));
        }
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_keys() {
        let (p, _) = good();
        let duplicate =
            format!(r#"{{"format":"{FORMAT}","format":"{FORMAT}","network":"devnet"}}"#);
        assert!(serde_json::from_str::<AgentIntent>(&duplicate).is_err());
        let extra = format!(
            r#"{{"format":"{FORMAT}","network":"devnet","wallet":"{}","agent_id":"{}","recipient":"{}","amount_darks":1,"fee_darks":1,"memo":"","nonce":"1234567890abcdef1234567890abcdef","created_at":1000,"expires_at":1100,"execute_shell":"bad"}}"#,
            p.wallet, p.agent_id, p.recipient
        );
        assert!(serde_json::from_str::<AgentIntent>(&extra).is_err());
    }
}
