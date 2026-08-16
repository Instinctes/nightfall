//! Thin FFI over `nightfall-wallet`. No extra consensus logic lives here.

use nightfall_crypto::Address;
use nightfall_types::{Amount, NetworkId, DARKS_PER_NIGHT};
use nightfall_wallet::{LightOutput, Wallet};
use serde_json::json;
use std::sync::Mutex;

uniffi::setup_scaffolding!();

pub const DEFAULT_NODE: &str = "http://seed.nightfallcoin.org:17888";
const SEED_FILE: &str = "mobile.seed";
const MATURITY: u64 = 1_440;
const DEFAULT_FEE_DARKS: u64 = DARKS_PER_NIGHT / 1_000;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    #[error("{msg}")]
    Failed { msg: String },
}

impl From<anyhow::Error> for MobileError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed { msg: e.to_string() }
    }
}

impl From<ureq::Error> for MobileError {
    fn from(e: ureq::Error) -> Self {
        Self::Failed {
            msg: format!("node: {e}"),
        }
    }
}

impl From<std::io::Error> for MobileError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed {
            msg: format!("io: {e}"),
        }
    }
}

#[derive(uniffi::Record, Clone)]
pub struct BalanceView {
    pub available: String,
    pub immature: String,
    pub pending_out: String,
    pub total: String,
    pub tip_height: u64,
    pub scanned_to: u64,
}

#[derive(uniffi::Record, Clone)]
pub struct HistoryView {
    pub direction: String,
    pub amount: String,
    pub memo: String,
    pub height: Option<u64>,
    pub pending: bool,
}

#[derive(uniffi::Record, Clone)]
pub struct CreatedWallet {
    pub address: String,
    pub recovery_phrase: String,
}

fn net(s: &str) -> Result<NetworkId, MobileError> {
    match s {
        "mainnet" | "" => Ok(NetworkId::Mainnet),
        "testnet" => Ok(NetworkId::Testnet),
        "devnet" => Ok(NetworkId::Devnet),
        other => Err(MobileError::Failed {
            msg: format!("unknown network {other}"),
        }),
    }
}

fn night(darks: u64) -> String {
    Amount(darks).to_string()
}

fn post_json(url: &str, body: &serde_json::Value) -> Result<ureq::Response, MobileError> {
    ureq::post(url)
        .timeout(std::time::Duration::from_secs(30))
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(MobileError::from)
}

fn rpc(
    node: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, MobileError> {
    let url = node.trim_end_matches('/');
    let body = json!({ "method": method, "params": params, "id": 1 });
    let resp = match post_json(url, &body) {
        Ok(r) => r,
        Err(_) => post_json(&format!("{url}/rpc"), &body)?,
    };
    let v: serde_json::Value = resp.into_json().map_err(MobileError::from)?;
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        return Err(MobileError::Failed { msg: e.to_string() });
    }
    Ok(v.get("result").cloned().unwrap_or(v))
}

fn parse_amount(s: &str) -> Result<u64, MobileError> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() || s.starts_with('-') {
        return Err(MobileError::Failed {
            msg: "enter a positive amount".into(),
        });
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s.as_str(), ""),
    };
    if frac.len() > 8 {
        return Err(MobileError::Failed {
            msg: "at most 8 decimal places".into(),
        });
    }
    let w: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| MobileError::Failed {
            msg: "not a number".into(),
        })?
    };
    let mut f = frac.to_string();
    while f.len() < 8 {
        f.push('0');
    }
    let f: u64 = if f.is_empty() {
        0
    } else {
        f.parse().map_err(|_| MobileError::Failed {
            msg: "not a number".into(),
        })?
    };
    w.checked_mul(DARKS_PER_NIGHT)
        .and_then(|x| x.checked_add(f))
        .filter(|&x| x > 0)
        .ok_or(MobileError::Failed {
            msg: "amount too small or too large".into(),
        })
}

#[derive(uniffi::Object)]
pub struct MobileWallet {
    inner: Mutex<Wallet>,
}

#[uniffi::export]
impl MobileWallet {
    #[uniffi::constructor]
    pub fn open(datadir: String, network: String) -> Result<Self, MobileError> {
        let w = Wallet::open(std::path::Path::new(&datadir), net(&network)?, SEED_FILE)?;
        Ok(Self {
            inner: Mutex::new(w),
        })
    }

    #[uniffi::constructor]
    pub fn create(
        datadir: String,
        network: String,
        birth_height: u64,
    ) -> Result<Self, MobileError> {
        let w = Wallet::create_at_height(
            std::path::Path::new(&datadir),
            net(&network)?,
            SEED_FILE,
            birth_height,
        )?;
        Ok(Self {
            inner: Mutex::new(w),
        })
    }

    #[uniffi::constructor]
    pub fn restore(
        datadir: String,
        network: String,
        phrase: String,
        birth_height: u64,
    ) -> Result<Self, MobileError> {
        let w = Wallet::restore_from_phrase(
            std::path::Path::new(&datadir),
            net(&network)?,
            SEED_FILE,
            &phrase,
            birth_height,
        )?;
        Ok(Self {
            inner: Mutex::new(w),
        })
    }

    pub fn address(&self) -> Result<String, MobileError> {
        Ok(self.lock()?.address_string())
    }

    pub fn recovery_phrase(&self) -> Result<String, MobileError> {
        Ok(self.lock()?.recovery_phrase())
    }

    pub fn fetch_tip(&self, node: String) -> Result<u64, MobileError> {
        let r = rpc(&node, "status", json!({}))?;
        Ok(r.get("tip_height")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                r.get("blocks")
                    .and_then(|v| v.as_u64())
                    .map(|b| b.saturating_sub(1))
            })
            .unwrap_or(0))
    }

    pub fn sync(&self, node: String) -> Result<u32, MobileError> {
        let mut w = self.lock()?;
        let mut from = w.scan_from();
        let mut found = 0u32;
        loop {
            let page = rpc(&node, "scan_feed", json!({ "from": from, "limit": 256 }))?;
            let scanned_to = page
                .get("scanned_to")
                .and_then(|v| v.as_u64())
                .unwrap_or(from);
            let nblocks = page.get("blocks").and_then(|v| v.as_u64()).unwrap_or(0);
            let outputs = page
                .get("outputs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let spent = page
                .get("spent")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let lights: Vec<LightOutput> = outputs
                .iter()
                .filter_map(|o| {
                    Some(LightOutput {
                        height: o.get("height")?.as_u64()?,
                        timestamp: o.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0),
                        commit: o.get("commit")?.as_str()?.to_string(),
                        ephemeral_pk: o.get("ephemeral_pk")?.as_str()?.to_string(),
                        output_pk: o.get("output_pk")?.as_str()?.to_string(),
                        view_tag: o.get("view_tag")?.as_u64()? as u8,
                        payload: o.get("payload")?.as_str()?.to_string(),
                        coinbase: o.get("coinbase").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect();
            found += w.ingest_scan_page(&lights, &spent, scanned_to)?;
            if nblocks == 0 {
                break;
            }
            let next = scanned_to.saturating_add(1);
            if next <= from {
                break;
            }
            from = next;
        }
        Ok(found)
    }

    pub fn balance(&self, node: String) -> Result<BalanceView, MobileError> {
        let tip = self.fetch_tip(node).unwrap_or(0);
        let w = self.lock()?;
        let b = w.balances(tip, MATURITY);
        Ok(BalanceView {
            available: night(b.available),
            immature: night(b.immature),
            pending_out: night(b.pending_out),
            total: night(b.total()),
            tip_height: tip,
            scanned_to: w.scanned_to(),
        })
    }

    pub fn history(&self) -> Result<Vec<HistoryView>, MobileError> {
        let w = self.lock()?;
        Ok(w.history()
            .iter()
            .take(80)
            .map(|e| HistoryView {
                direction: e.direction.label().to_string(),
                amount: night(e.amount),
                memo: e.memo.clone(),
                height: e.height,
                pending: e.is_pending(),
            })
            .collect())
    }

    pub fn send(
        &self,
        node: String,
        to: String,
        amount: String,
        memo: String,
    ) -> Result<String, MobileError> {
        let to_addr = Address::decode(&to).map_err(|e| MobileError::Failed {
            msg: format!("address: {e}"),
        })?;
        let amount_darks = parse_amount(&amount)?;
        let tip = self.fetch_tip(node.clone())?;
        let tx = {
            let w = self.lock()?;
            if to_addr == w.address() {
                return Err(MobileError::Failed {
                    msg: "that is your own address".into(),
                });
            }
            w.create_payment_at(
                &to_addr,
                amount_darks,
                DEFAULT_FEE_DARKS,
                &memo,
                tip,
                MATURITY,
            )?
        };
        let raw =
            serde_json::to_value(&tx).map_err(|e| MobileError::Failed { msg: e.to_string() })?;
        let res = rpc(&node, "submit_tx", json!({ "tx": raw }))?;
        {
            let mut w = self.lock()?;
            w.record_send(&tx, amount_darks, memo)?;
        }
        Ok(res
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }
}

#[uniffi::export]
pub fn wallet_exists(datadir: String) -> bool {
    std::path::Path::new(&datadir).join(SEED_FILE).exists()
}

#[uniffi::export]
pub fn default_node() -> String {
    DEFAULT_NODE.to_string()
}

#[uniffi::export]
pub fn privacy_warning() -> String {
    "This phone trusts a node for what it shows you. A hostile node can hide a payment or invent one on the screen. It cannot spend your coins — the seed never leaves the device. The node you send through is probably the first to see the transaction.".into()
}

impl MobileWallet {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Wallet>, MobileError> {
        self.inner.lock().map_err(|_| MobileError::Failed {
            msg: "wallet lock poisoned".into(),
        })
    }
}
