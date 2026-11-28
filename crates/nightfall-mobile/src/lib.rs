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
    pub fee: String,
    pub memo: String,
    pub height: Option<u64>,
    pub pending: bool,
    pub timestamp: u64,
    pub txid: String,
}

#[derive(uniffi::Record, Clone)]
pub struct WalletInfo {
    pub address: String,
    pub birth_height: u64,
    pub scanned_to: u64,
    pub outputs: u32,
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
    Amount(darks).decimal_string()
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

/// The shared parser, not a local one. See `nightfall_wallet::amount_input`
/// for what the three copies used to disagree about.
fn parse_amount(s: &str) -> Result<u64, MobileError> {
    nightfall_wallet::amount_input::parse_night(s)
        .map_err(|e| MobileError::Failed { msg: e.to_string() })
}

// Do not advance a mobile wallet past a malformed page. The server can be
// mistaken or hostile; schema validation is not proof that its chain is real.
const MOBILE_SCAN_LIMIT: u64 = 256;

struct MobileScanPage {
    scanned_to: u64,
    blocks: u64,
    outputs: Vec<LightOutput>,
    spent: Vec<String>,
}

fn malformed_scan(reason: &'static str) -> MobileError {
    MobileError::Failed {
        msg: format!("Invalid scan_feed page ({reason}); this page was not saved."),
    }
}

fn scan_hex(value: Option<&serde_json::Value>, bytes: usize) -> Result<String, MobileError> {
    let text = value
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| malformed_scan("missing hex field"))?;
    if text.len() != bytes * 2 || hex::decode(text).is_err() {
        return Err(malformed_scan("invalid hex field"));
    }
    Ok(text.to_owned())
}

// Wallet ciphertext length is not a consensus rule.
fn scan_payload(value: Option<&serde_json::Value>) -> Result<String, MobileError> {
    let text = value
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| malformed_scan("missing payload"))?;

    if !text.len().is_multiple_of(2)
        || text.len() / 2 > nightfall_types::MAX_MESSAGE_BYTES
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(malformed_scan("invalid payload encoding or size"));
    }

    Ok(text.to_owned())
}

/// Parse the complete reply before changing scanned_to or owned outputs.
/// Accepted metadata mirrors nightfall-node::rpc::scan_feed_snapshot.
fn parse_mobile_scan_page(
    page: &serde_json::Value,
    requested_from: u64,
) -> Result<MobileScanPage, MobileError> {
    let fields = page
        .as_object()
        .ok_or_else(|| malformed_scan("not an object"))?;
    let from = fields
        .get("from")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| malformed_scan("missing from"))?;
    if from != requested_from {
        return Err(malformed_scan("incorrect page origin"));
    }
    let blocks = fields
        .get("blocks")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| malformed_scan("missing block count"))?;
    let scanned_to = fields
        .get("scanned_to")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| malformed_scan("missing scan position"))?;
    if blocks > MOBILE_SCAN_LIMIT {
        return Err(malformed_scan("page larger than requested"));
    }
    let output_rows = fields
        .get("outputs")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| malformed_scan("missing outputs array"))?;
    let spent_rows = fields
        .get("spent")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| malformed_scan("missing spent array"))?;

    if blocks == 0 {
        if scanned_to != from || !output_rows.is_empty() || !spent_rows.is_empty() {
            return Err(malformed_scan("empty page claims progress or data"));
        }
    } else if scanned_to
        .checked_sub(from)
        .and_then(|delta| delta.checked_add(1))
        != Some(blocks)
        || scanned_to == u64::MAX
    {
        return Err(malformed_scan(
            "missing, reversed or overflowing block range",
        ));
    }

    if let Some(value) = fields.get("tip_height") {
        if !value.is_null() {
            let tip = value
                .as_u64()
                .ok_or_else(|| malformed_scan("invalid tip height"))?;
            if blocks > 0 && scanned_to > tip {
                return Err(malformed_scan("scanned above claimed tip"));
            }
        }
    }
    if let Some(value) = fields.get("available_from") {
        let available = value
            .as_u64()
            .ok_or_else(|| malformed_scan("invalid archive horizon"))?;
        if from < available {
            return Err(malformed_scan("node cannot serve requested history"));
        }
    }

    let mut outputs = Vec::with_capacity(output_rows.len());
    for row in output_rows {
        let height = row
            .get("height")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| malformed_scan("output missing height"))?;
        if blocks == 0 || height < from || height > scanned_to {
            return Err(malformed_scan("output outside page range"));
        }
        let timestamp = row
            .get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| malformed_scan("output missing timestamp"))?;
        let view_tag = row
            .get("view_tag")
            .and_then(serde_json::Value::as_u64)
            .and_then(|tag| u8::try_from(tag).ok())
            .ok_or_else(|| malformed_scan("invalid view tag"))?;
        let coinbase = row
            .get("coinbase")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| malformed_scan("invalid coinbase flag"))?;
        outputs.push(LightOutput {
            height,
            timestamp,
            commit: scan_hex(row.get("commit"), 32)?,
            ephemeral_pk: scan_hex(row.get("ephemeral_pk"), 32)?,
            output_pk: scan_hex(row.get("output_pk"), 32)?,
            view_tag,
            payload: scan_payload(row.get("payload"))?,
            coinbase,
        });
    }
    let mut spent = Vec::with_capacity(spent_rows.len());
    for value in spent_rows {
        spent.push(scan_hex(Some(value), 32)?);
    }
    Ok(MobileScanPage {
        scanned_to,
        blocks,
        outputs,
        spent,
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

    pub fn view_key(&self) -> Result<String, MobileError> {
        Ok(self.lock()?.view_key_string())
    }

    pub fn info(&self) -> Result<WalletInfo, MobileError> {
        let w = self.lock()?;
        Ok(WalletInfo {
            address: w.address_string(),
            birth_height: w.birth_height(),
            scanned_to: w.scanned_to(),
            outputs: w.spendable_count() as u32,
        })
    }

    pub fn reset_scan(&self) -> Result<(), MobileError> {
        self.lock()?.reset_scan()?;
        Ok(())
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
            let reply = rpc(&node, "scan_feed", json!({ "from": from, "limit": 256 }))?;
            let page = parse_mobile_scan_page(&reply, from)?;
            // No wallet mutation until the complete page has passed validation.
            let newly_found = w.ingest_scan_page(&page.outputs, &page.spent, page.scanned_to)?;
            found = found
                .checked_add(newly_found)
                .ok_or_else(|| malformed_scan("output counter overflow"))?;
            if page.blocks == 0 {
                break;
            }
            from = page
                .scanned_to
                .checked_add(1)
                .ok_or_else(|| malformed_scan("scan position overflow"))?;
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
                fee: night(e.fee),
                memo: e.memo.clone(),
                height: e.height,
                pending: e.is_pending(),
                timestamp: e.timestamp,
                txid: e.txid.clone(),
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
pub fn wipe_wallet(datadir: String) -> Result<(), MobileError> {
    let dir = std::path::Path::new(&datadir);
    for name in [SEED_FILE, &format!("{SEED_FILE}.outputs.json")] {
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    Ok(())
}

#[uniffi::export]
pub fn node_tip(node: String) -> Result<u64, MobileError> {
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

#[uniffi::export]
pub fn default_node() -> String {
    DEFAULT_NODE.to_string()
}

#[uniffi::export]
pub fn default_fee() -> String {
    Amount(DEFAULT_FEE_DARKS).decimal_string()
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

#[cfg(test)]
mod mobile_scan_feed_tests {
    use super::*;

    fn good_page() -> serde_json::Value {
        json!({
            "from": 7,
            "scanned_to": 8,
            "blocks": 2,
            "tip_height": 12,
            "available_from": 0,
            "outputs": [{
                "height": 8,
                "timestamp": 1234,
                "commit": "11".repeat(32),
                "ephemeral_pk": "22".repeat(32),
                "output_pk": "33".repeat(32),
                "view_tag": 255,
                "payload": "44".repeat(120),
                "coinbase": false
            }],
            "spent": ["55".repeat(32)]
        })
    }

    #[test]
    fn mixed_payloads_keep_owned_payment() {
        use nightfall_crypto::{create_output, WalletKeys};

        let network = NetworkId::Devnet;
        let keys = WalletKeys::from_seed([42; 32]);
        let address = keys.address();
        let context = network.proof_context();

        let (good, _) = create_output(&address, 123_456, "synthetic regression", context).unwrap();

        let (unopenable, _) = create_output(&address, 654_321, "", context).unwrap();

        let row = |output: &nightfall_crypto::Output, height: u64| {
            json!({
                "height": height,
                "timestamp": 1234,
                "commit": hex::encode(output.commit.0),
                "ephemeral_pk":
                    hex::encode(output.ephemeral_pk),
                "output_pk":
                    hex::encode(output.output_pk),
                "view_tag": output.view_tag,
                "payload": hex::encode(&output.payload),
                "coinbase": false
            })
        };

        // Synthetic RPC records, not validated blocks.
        for size in [0usize, 1, 119, 121, 500] {
            let mut unusual = row(&unopenable, 7);
            unusual["payload"] = json!("ab".repeat(size));

            let mut page = good_page();
            page["outputs"] = json!([unusual, row(&good, 8)]);
            page["spent"] = json!([]);

            let parsed = parse_mobile_scan_page(&page, 7).unwrap();

            assert_eq!(parsed.outputs.len(), 2);

            let mut wallet = Wallet::in_memory(network, keys.clone(), 7);

            let found = wallet
                .ingest_scan_page(&parsed.outputs, &parsed.spent, parsed.scanned_to)
                .unwrap();

            assert_eq!(found, 1);
            assert_eq!(wallet.outputs().len(), 1);
            assert_eq!(wallet.outputs()[0].value, 123_456);
            assert_eq!(wallet.outputs()[0].commit.0, good.commit.0);
            assert_eq!(wallet.scanned_to(), 8);
        }
    }

    #[test]
    fn payload_encoding_and_size_boundaries() {
        for valid in ["", "00", "aB", "abcdef"] {
            assert!(scan_payload(Some(&json!(valid))).is_ok());
        }

        for invalid in ["0", "xyz0", "gg", "00 0"] {
            assert!(scan_payload(Some(&json!(invalid))).is_err());
        }

        assert!(scan_payload(None).is_err());
        assert!(scan_payload(Some(&json!(null))).is_err());
        assert!(scan_payload(Some(&json!(123))).is_err());

        let limit = nightfall_types::MAX_MESSAGE_BYTES;

        let at_limit = json!("00".repeat(limit));
        assert!(scan_payload(Some(&at_limit)).is_ok());
        drop(at_limit);

        let above_limit = json!("00".repeat(limit + 1));
        assert!(scan_payload(Some(&above_limit)).is_err());
    }

    #[test]
    fn valid_page_keeps_every_record() {
        let parsed = parse_mobile_scan_page(&good_page(), 7).unwrap();
        assert_eq!(parsed.scanned_to, 8);
        assert_eq!(parsed.blocks, 2);
        assert_eq!(parsed.outputs.len(), 1);
        assert_eq!(parsed.outputs[0].view_tag, 255);
        assert_eq!(parsed.spent.len(), 1);
    }

    #[test]
    fn empty_page_is_allowed_only_without_progress() {
        let empty = json!({"from": 9, "scanned_to": 9, "blocks": 0,
            "outputs": [], "spent": [], "tip_height": 8});
        assert_eq!(parse_mobile_scan_page(&empty, 9).unwrap().blocks, 0);
        let mut broken = empty;
        broken["spent"] = json!(["aa".repeat(32)]);
        assert!(parse_mobile_scan_page(&broken, 9).is_err());
    }

    #[test]
    fn rejects_missing_or_wrong_page_identity() {
        let mut p = good_page();
        assert!(parse_mobile_scan_page(&p, 6).is_err());
        p.as_object_mut().unwrap().remove("from");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_gaps_and_oversized_pages() {
        let mut p = good_page();
        p["scanned_to"] = json!(9);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["blocks"] = json!(257);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["scanned_to"] = json!(u64::MAX);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_unavailable_history_and_false_tip() {
        let mut p = good_page();
        p["available_from"] = json!(8);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["tip_height"] = json!(7);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_malformed_output_instead_of_skipping_it() {
        let mut p = good_page();
        p["outputs"][0]["commit"] = json!("nothex");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["outputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("ephemeral_pk");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["outputs"][0]["height"] = json!(9);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["outputs"][0]["payload"] = json!("0");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_truncated_view_tags_and_ambiguous_coinbase() {
        let mut p = good_page();
        p["outputs"][0]["view_tag"] = json!(256);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["outputs"][0]["coinbase"] = json!("false");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["outputs"][0].as_object_mut().unwrap().remove("timestamp");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_invalid_spend_instead_of_ignoring_it() {
        let mut p = good_page();
        p["spent"] = json!(["ab", "55".repeat(32)]);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        p = good_page();
        p["spent"] = json!([null]);
        assert!(parse_mobile_scan_page(&p, 7).is_err());
    }

    #[test]
    fn rejects_missing_arrays_and_non_objects() {
        let mut p = good_page();
        p.as_object_mut().unwrap().remove("outputs");
        assert!(parse_mobile_scan_page(&p, 7).is_err());
        assert!(parse_mobile_scan_page(&json!([]), 7).is_err());
    }
}
