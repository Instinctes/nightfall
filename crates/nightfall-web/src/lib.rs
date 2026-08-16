//! Browser wallet: keys and scanning in WASM, HTTP stays in JavaScript.

use nightfall_crypto::{Address, WalletKeys};
use nightfall_types::{Amount, NetworkId, DARKS_PER_NIGHT};
use nightfall_wallet::{LightOutput, Wallet};
use serde_json::json;
use wasm_bindgen::prelude::*;

const MATURITY: u64 = 1_440;
const DEFAULT_FEE: u64 = DARKS_PER_NIGHT / 1_000;

fn err(e: impl ToString) -> JsError {
    JsError::new(&e.to_string())
}

fn parse_amount(s: &str) -> Result<u64, JsError> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() || s.starts_with('-') {
        return Err(err("enter a positive amount"));
    }
    let (whole, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    if frac.len() > 8 {
        return Err(err("at most 8 decimal places"));
    }
    let w: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| err("not a number"))?
    };
    let mut f = frac.to_string();
    while f.len() < 8 {
        f.push('0');
    }
    let f: u64 = if f.is_empty() {
        0
    } else {
        f.parse().map_err(|_| err("not a number"))?
    };
    w.checked_mul(DARKS_PER_NIGHT)
        .and_then(|x| x.checked_add(f))
        .filter(|&x| x > 0)
        .ok_or_else(|| err("amount too small or too large"))
}

fn lights_from_json(raw: &str) -> Result<Vec<LightOutput>, JsError> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| err(format!("outputs: {e}")))?;
    Ok(arr
        .into_iter()
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
        .collect())
}

#[wasm_bindgen]
pub fn create_wallet(birth_height: u64) -> Result<JsValue, JsError> {
    let keys = WalletKeys::generate();
    let phrase = keys.to_mnemonic();
    let w = Wallet::in_memory(NetworkId::Mainnet, keys, birth_height);
    let out = json!({
        "state": w.export_state().map_err(err)?,
        "address": w.address_string(),
        "phrase": phrase,
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn restore_wallet(phrase: &str, birth_height: u64) -> Result<JsValue, JsError> {
    let keys = WalletKeys::from_mnemonic(phrase).map_err(err)?;
    let w = Wallet::in_memory(NetworkId::Mainnet, keys, birth_height);
    let out = json!({
        "state": w.export_state().map_err(err)?,
        "address": w.address_string(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_address(state: &str) -> Result<String, JsError> {
    Ok(Wallet::import_state(state).map_err(err)?.address_string())
}

#[wasm_bindgen]
pub fn wallet_scan_from(state: &str) -> Result<u64, JsError> {
    Ok(Wallet::import_state(state).map_err(err)?.scan_from())
}

#[wasm_bindgen]
pub fn ingest_page(
    state: &str,
    outputs_json: &str,
    spent_json: &str,
    scanned_to: u64,
) -> Result<JsValue, JsError> {
    let mut w = Wallet::import_state(state).map_err(err)?;
    let lights = lights_from_json(outputs_json)?;
    let spent: Vec<String> = serde_json::from_str(spent_json).unwrap_or_default();
    let found = w.ingest_scan_page(&lights, &spent, scanned_to).map_err(err)?;
    let out = json!({
        "state": w.export_state().map_err(err)?,
        "found": found,
        "scanned_to": w.scanned_to(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_balance(state: &str, tip: u64) -> Result<JsValue, JsError> {
    let w = Wallet::import_state(state).map_err(err)?;
    let b = w.balances(tip, MATURITY);
    let out = json!({
        "available": Amount(b.available).to_string(),
        "immature": Amount(b.immature).to_string(),
        "pending_out": Amount(b.pending_out).to_string(),
        "total": Amount(b.total()).to_string(),
        "scanned_to": w.scanned_to(),
        "tip": tip,
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_history(state: &str) -> Result<JsValue, JsError> {
    let w = Wallet::import_state(state).map_err(err)?;
    let rows: Vec<_> = w
        .history()
        .iter()
        .take(80)
        .map(|e| {
            json!({
                "direction": e.direction.label(),
                "amount": Amount(e.amount).to_string(),
                "memo": e.memo,
                "height": e.height,
                "pending": e.is_pending(),
            })
        })
        .collect();
    JsValue::from_str(&serde_json::to_string(&rows).map_err(err)?).pipe_ok()
}

#[wasm_bindgen]
pub fn build_send(
    state: &str,
    to: &str,
    amount: &str,
    memo: &str,
    tip: u64,
) -> Result<JsValue, JsError> {
    let mut w = Wallet::import_state(state).map_err(err)?;
    let addr = Address::decode(to).map_err(err)?;
    if addr == w.address() {
        return Err(err("that is your own address"));
    }
    let darks = parse_amount(amount)?;
    let tx = w
        .create_payment_at(&addr, darks, DEFAULT_FEE, memo, tip, MATURITY)
        .map_err(err)?;
    w.record_send(&tx, darks, memo.to_string()).map_err(err)?;
    let out = json!({
        "state": w.export_state().map_err(err)?,
        "tx": tx,
        "txid": tx.txid().to_hex(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

trait PipeOk {
    fn pipe_ok(self) -> Result<JsValue, JsError>;
}
impl PipeOk for JsValue {
    fn pipe_ok(self) -> Result<JsValue, JsError> {
        Ok(self)
    }
}
