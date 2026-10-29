//! Non-custodial payment-request WASM boundary.
//! Never sign or broadcast from request parsing.
use nightfall_crypto::Address;
use nightfall_types::NetworkId;
use nightfall_wallet::payment_request::{PaymentRequest, MAX_INVOICE, MAX_LEN, MAX_MEMO};
use serde_json::json;
use wasm_bindgen::prelude::*;

fn number(s: &str) -> Result<Option<u64>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    if (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Non-canonical unsigned integer".into());
    }
    s.parse().map(Some).map_err(|_| "Integer overflow".into())
}

fn make(a: &str, n: &str, m: &str, id: &str, expiry: &str) -> Result<String, String> {
    if a.len() > 256 || m.chars().count() > MAX_MEMO || id.chars().count() > MAX_INVOICE {
        return Err("Payment request field too long".into());
    }
    let r = PaymentRequest {
        address: Address::decode(a).map_err(|e| e.to_string())?,
        network: NetworkId::Mainnet,
        amount_darks: number(n)?,
        memo: m.to_owned(),
        invoice: id.to_owned(),
        expires_unix: number(expiry)?,
    };
    let uri = r.to_uri();
    if uri.len() > MAX_LEN || PaymentRequest::parse(&uri).ok().as_ref() != Some(&r) {
        return Err("Request failed canonical round-trip".into());
    }
    Ok(uri)
}

fn inspect(uri: &str, now: u64) -> Result<String, String> {
    let r = PaymentRequest::parse(uri).map_err(|e| e.to_string())?;
    r.require_network(NetworkId::Mainnet)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "address": r.address.encode(),
        "network": "mainnet",
        "amount_darks": r.amount_darks.map(|n| n.to_string()),
        "amount_text": r.amount_text(),
        "memo": r.memo,
        "invoice": r.invoice,
        "expires_unix": r.expires_unix.map(|n| n.to_string()),
        "expired": r.is_expired(now),
        "requires_confirmation": true,
        "broadcasts_payment": false
    })
    .to_string())
}

#[wasm_bindgen(js_name = buildPaymentRequest)]
pub fn build_payment_request(
    address: &str,
    amount_darks: &str,
    memo: &str,
    invoice: &str,
    expiry: &str,
) -> Result<String, JsError> {
    make(address, amount_darks, memo, invoice, expiry).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen(js_name = previewPaymentRequest)]
pub fn preview_payment_request(uri: &str, now_seconds: f64) -> Result<String, JsError> {
    if !now_seconds.is_finite()
        || now_seconds < 0.0
        || now_seconds.fract() != 0.0
        || now_seconds > 9_007_199_254_740_991.0
    {
        return Err(JsError::new("Invalid Unix timestamp"));
    }
    inspect(uri, now_seconds as u64).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    fn addr() -> String {
        WalletKeys::from_seed([23u8; 32]).address().encode()
    }
    #[test]
    fn bridge_round_trip_and_human_authorization() {
        let uri = make(
            &addr(),
            "150000000",
            "Café & Tee",
            "invoice-1",
            "1800000000",
        )
        .unwrap();
        let p: serde_json::Value = serde_json::from_str(&inspect(&uri, 100).unwrap()).unwrap();
        assert_eq!(p["amount_darks"], "150000000");
        assert_eq!(p["memo"], "Café & Tee");
        assert_eq!(p["requires_confirmation"], true);
        assert_eq!(p["broadcasts_payment"], false);
        assert_eq!(p["expired"], false);
        assert!(inspect(&uri, 1900000000)
            .unwrap()
            .contains("\"expired\":true"));
    }
    #[test]
    fn bridge_rejects_unsafe_requests() {
        let uri = make(&addr(), "1", "", "", "").unwrap();
        for bad in [
            uri.replace("mainnet", "testnet"),
            format!("{uri}&amount=2"),
            format!("{uri}&unknown=1"),
            uri.replace("mainnet", "MAINNET"),
        ] {
            assert!(inspect(&bad, 0).is_err());
        }
    }
    #[test]
    fn bridge_rejects_invalid_amounts_and_lengths() {
        for n in [
            "-1",
            "+1",
            "01",
            "1.1",
            "18446744073709551616",
            "9000000000000001",
        ] {
            assert!(make(&addr(), n, "", "", "").is_err());
        }
        assert!(make(&addr(), "1", &"x".repeat(MAX_MEMO + 1), "", "").is_err());
        assert!(make(&addr(), "1", "", "", "01").is_err());
        assert!(make("nf1invalid", "1", "", "", "").is_err());
    }
}
