//! Browser wallet: keys and scanning in WASM, HTTP stays in JavaScript.

use nightfall_crypto::{Address, WalletKeys};
use nightfall_types::{Amount, NetworkId, DARKS_PER_NIGHT};
use nightfall_wallet::{LightOutput, Wallet};
use serde_json::json;
use wasm_bindgen::prelude::*;

const MATURITY: u64 = 1_440;
const DEFAULT_FEE: u64 = DARKS_PER_NIGHT / 1_000;

#[wasm_bindgen(start)]
pub fn wasm_start() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
}

fn err(e: impl ToString) -> JsError {
    JsError::new(&e.to_string())
}

/// JS `Number` in, never `BigInt`. Safari's `ToBigInt` rejects a plain
/// number and shows "Invalid argument type in ToBigInt operation".
fn height(n: f64) -> u64 {
    if !n.is_finite() || n < 0.0 {
        0
    } else if n >= u64::MAX as f64 {
        u64::MAX
    } else {
        n.trunc() as u64
    }
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

fn load(state: &str) -> Result<Wallet, JsError> {
    Wallet::import_state(state).map_err(err)
}

fn dump(w: &Wallet) -> Result<String, JsError> {
    w.export_state().map_err(err)
}

#[wasm_bindgen]
pub fn create_wallet(birth_height: f64) -> Result<JsValue, JsError> {
    let keys = WalletKeys::generate();
    let phrase = keys.to_mnemonic();
    let w = Wallet::in_memory(NetworkId::Mainnet, keys, height(birth_height));
    let out = json!({
        "state": dump(&w)?,
        "address": w.address_string(),
        "phrase": phrase,
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn restore_wallet(phrase: &str, birth_height: f64) -> Result<JsValue, JsError> {
    let keys = WalletKeys::from_mnemonic(phrase).map_err(err)?;
    let w = Wallet::in_memory(NetworkId::Mainnet, keys, height(birth_height));
    let out = json!({
        "state": dump(&w)?,
        "address": w.address_string(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_address(state: &str) -> Result<String, JsError> {
    Ok(load(state)?.address_string())
}

#[wasm_bindgen]
pub fn wallet_phrase(state: &str) -> Result<String, JsError> {
    Ok(load(state)?.recovery_phrase())
}

#[wasm_bindgen]
pub fn wallet_view_key(state: &str) -> Result<String, JsError> {
    Ok(load(state)?.view_key_string())
}

#[wasm_bindgen]
pub fn wallet_scan_from(state: &str) -> Result<f64, JsError> {
    Ok(load(state)?.scan_from() as f64)
}

#[wasm_bindgen]
pub fn wallet_info(state: &str) -> Result<JsValue, JsError> {
    let w = load(state)?;
    let out = json!({
        "address": w.address_string(),
        "birth_height": w.birth_height(),
        "scanned_to": w.scanned_to(),
        "scan_from": w.scan_from(),
        "outputs": w.spendable_count(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn reset_scan(state: &str) -> Result<JsValue, JsError> {
    let mut w = load(state)?;
    w.reset_scan().map_err(err)?;
    let out = json!({
        "state": dump(&w)?,
        "scan_from": w.scan_from(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn address_qr_svg(address: &str) -> Result<String, JsError> {
    let code = qrcode::QrCode::new(address.as_bytes()).map_err(err)?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#12091c"))
        .light_color(qrcode::render::svg::Color("#f4f0ff"))
        .quiet_zone(true)
        .build())
}

#[wasm_bindgen]
pub fn ingest_page(
    state: &str,
    outputs_json: &str,
    spent_json: &str,
    scanned_to: f64,
) -> Result<JsValue, JsError> {
    let mut w = load(state)?;
    let lights = lights_from_json(outputs_json)?;
    let spent: Vec<String> = serde_json::from_str(spent_json).unwrap_or_default();
    let found = w
        .ingest_scan_page(&lights, &spent, height(scanned_to))
        .map_err(err)?;
    let out = json!({
        "state": dump(&w)?,
        "found": found,
        "scanned_to": w.scanned_to(),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_balance(state: &str, tip: f64) -> Result<JsValue, JsError> {
    let w = load(state)?;
    let b = w.balances(height(tip), MATURITY);
    let out = json!({
        "available": Amount(b.available).decimal_string(),
        "immature": Amount(b.immature).decimal_string(),
        "pending_out": Amount(b.pending_out).decimal_string(),
        "total": Amount(b.total()).decimal_string(),
        "scanned_to": w.scanned_to(),
        "tip": height(tip),
    });
    JsValue::from_str(&out.to_string()).pipe_ok()
}

#[wasm_bindgen]
pub fn wallet_history(state: &str) -> Result<JsValue, JsError> {
    let w = load(state)?;
    let rows: Vec<_> = w
        .history()
        .iter()
        .take(80)
        .map(|e| {
            json!({
                "direction": e.direction.label(),
                "amount": Amount(e.amount).decimal_string(),
                "fee": Amount(e.fee).decimal_string(),
                "memo": e.memo,
                "height": e.height,
                "pending": e.is_pending(),
                "timestamp": e.timestamp,
                "txid": e.txid,
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
    tip: f64,
) -> Result<JsValue, JsError> {
    let mut w = load(state)?;
    let addr = Address::decode(to).map_err(err)?;
    if addr == w.address() {
        return Err(err("that is your own address"));
    }
    let darks = parse_amount(amount)?;
    let tx = w
        .create_payment_at(&addr, darks, DEFAULT_FEE, memo, height(tip), MATURITY)
        .map_err(err)?;
    w.record_send(&tx, darks, memo.to_string()).map_err(err)?;
    let tx_val = serde_json::to_value(&tx).map_err(err)?;
    let out = json!({
        "state": dump(&w)?,
        "tx": tx_val,
        "txid": tx.txid().to_hex(),
        "fee": Amount(DEFAULT_FEE).decimal_string(),
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
