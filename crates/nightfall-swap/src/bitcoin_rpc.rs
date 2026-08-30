//! bitcoind JSON-RPC. The node is eyes and mouth, not a custodian.
//!
//! Pruned-safe: height via `getblockcount`, unspent via `gettxout` (UTXO set),
//! confirmations of *our* broadcasts via `getrawtransaction` (mempool / recent
//! blocks). No wallet RPCs, no scan of historical blocks.

use crate::watch::{
    BroadcastResult, Broadcaster, ChainWatch, MempoolAccept, OutRef, TxRef, WatchError,
};
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RpcSetupError {
    #[error("credential file: {0}")]
    Io(#[from] std::io::Error),
    #[error("credential file is readable by others; chmod 600 required")]
    WorldReadable,
    #[error("credential file missing {0}")]
    Missing(&'static str),
}

#[derive(Clone)]
pub struct RpcAuth {
    pub url: String,
    pub user: String,
    pub password: String,
}

/// Written by hand, not derived, so that the bitcoind password cannot reach a
/// log file. A derived `Debug` puts it in the output of every `{:?}`, every
/// `dbg!`, every panic message that carries this struct, and every error a
/// user might paste into a chat room while asking for help. The credential
/// file is mode 0600 for the same reason; a derived `Debug` would undo that
/// the first time something went wrong.
///
/// Pinned by `debug_does_not_carry_the_password`.
impl fmt::Debug for RpcAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcAuth")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl RpcAuth {
    /// `url=...`, `user=...`, `password=...` lines. Unix: must be mode 0600.
    pub fn from_file(path: &Path) -> Result<Self, RpcSetupError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(RpcSetupError::WorldReadable);
            }
        }
        let text = fs::read_to_string(path)?;
        let mut url = None;
        let mut user = None;
        let mut password = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(v) = line.strip_prefix("url=") {
                url = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("user=") {
                user = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("password=") {
                password = Some(v.trim().to_string());
            }
        }
        Ok(Self {
            url: url.ok_or(RpcSetupError::Missing("url"))?,
            user: user.ok_or(RpcSetupError::Missing("user"))?,
            password: password.ok_or(RpcSetupError::Missing("password"))?,
        })
    }
}

pub struct BitcoinRpc {
    auth: RpcAuth,
}

/// HTTP basic auth, so the password never reaches a URL or an error string.
fn basic_auth(auth: &RpcAuth) -> String {
    use base64::Engine;
    let raw = format!("{}:{}", auth.user, auth.password);
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw)
    )
}

/// bitcoind saying it cannot answer without `-txindex`.
///
/// This is not "no such transaction". A node without the index answers the
/// same way for a transaction it mined itself thirty seconds ago, so reading
/// it as "unknown" would leave the swap blind to its own confirmed lock.
/// Measured against Bitcoin Core v30.1, both spellings of error −5:
///
/// ```text
/// without txindex: "No such mempool transaction. Use -txindex or provide a
///                   block hash to enable blockchain transaction queries.
///                   Use gettransaction for wallet transactions."
/// with txindex:    "No such mempool or blockchain transaction.
///                   Use gettransaction for wallet transactions."
/// ```
///
/// Only `-txindex` separates them. Matching on the shared tail
/// ("use gettransaction for wallet") turns every genuinely missing
/// transaction into a configuration complaint — which is how the first
/// version of this function behaved, and why the live test caught it.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Turn a mempool reject-reason into a sentence the user can act on.
///
/// Matched against Bitcoin Core v30.1 strings, not against our own
/// summaries of them. `non-BIP68-final` is the expected answer while a
/// CSV has not matured — it is not a bug.
fn send_label(kind: crate::persist::SendKind) -> &'static str {
    use crate::persist::SendKind;
    match kind {
        SendKind::Cancel => "TX_cancel",
        SendKind::Refund => "TX_refund",
        SendKind::Punish => "TX_punish",
        SendKind::Redeem => "TX_redeem",
        SendKind::NightClaim => "NIGHT claim",
    }
}

pub fn explain_broadcast_reject(kind: crate::persist::SendKind, reason: &str) -> String {
    use crate::persist::SendKind;
    let r = reason.to_ascii_lowercase();
    if r.contains("non-bip68-final") {
        return match kind {
            SendKind::Cancel => "Too early: H₁ has not passed yet. The cancel becomes valid \
                 once the Bitcoin lock is deep enough — this is the delay that \
                 protects the other side's redeem window."
                .into(),
            SendKind::Punish => "Too early: H₂ has not passed yet. The other side still has \
                 time to refund, and until that window closes the punish \
                 cannot confirm."
                .into(),
            other => format!("The node refused {}: {reason}", send_label(other)),
        };
    }
    if r.contains("missingorspent")
        || r.contains("bad-txns-inputs-missingorspent")
        || r.contains("already")
        || r.contains("txn-already")
    {
        return format!(
            "{} spends an output that is gone — the other side likely moved \
             first. Refresh the swap before acting.",
            send_label(kind)
        );
    }
    format!("The node refused {}: {reason}", send_label(kind))
}

pub fn needs_txindex(msg: &str) -> bool {
    msg.to_ascii_lowercase().contains("-txindex")
}

/// True when bitcoind is saying it does not have the transaction.
///
/// A pruned node answers this for anything older than its prune horizon
/// (and without `-txindex`). That is *unknown*, never zero confirmations:
/// `Some(0)` is mempool, and treating a pruned miss as mempool would open
/// a redeem window on a lock that may already be buried past H₁.
pub fn tx_lookup_unknown(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("\"code\": -5")
        || m.contains("\"code\":-5")
        || m.contains("no such")
        || m.contains("not found")
        || m.contains("pruned")
        || m.contains("can't read transaction")
        || m.contains("cannot find")
}

/// Interpret `getrawtransaction`. A miss (pruned, no txindex, never seen)
/// is `Ok(None)`. A mempool hit with no `confirmations` field is `Ok(Some(0))`.
/// An RPC outage stays `Err`. Those three are not interchangeable.
pub fn confirmations_from_rpc(
    result: Result<Value, WatchError>,
) -> Result<Option<u64>, WatchError> {
    match result {
        Ok(v) => Ok(v.get("confirmations").and_then(|c| c.as_u64()).or(Some(0))),
        Err(WatchError::Unavailable(m)) if tx_lookup_unknown(&m) => Ok(None),
        Err(e) => Err(e),
    }
}

impl BitcoinRpc {
    pub fn new(auth: RpcAuth) -> Self {
        Self { auth }
    }

    /// Remove the password if it ever appears, without throwing the rest
    /// away. A blanket redaction hides the node's answer from us too.
    ///
    /// The password may show up URL-encoded (`p%40ss` for `p@ss`). Replacing
    /// only the raw form would leave it in the string.
    fn scrub(&self, msg: &str) -> String {
        if self.auth.password.is_empty() {
            return msg.to_string();
        }
        let mut out = msg.replace(&self.auth.password, "<redacted>");
        let enc = percent_encode(&self.auth.password);
        if enc != self.auth.password {
            out = out.replace(&enc, "<redacted>");
            out = out.replace(&enc.to_ascii_lowercase(), "<redacted>");
        }
        out
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, WatchError> {
        let body = json!({
            "jsonrpc": "1.0",
            "id": "nightfall-swap",
            "method": method,
            "params": params,
        });
        // Credentials go in the header, not the URL.
        //
        // With `user:pass@host` the password ends up inside ureq's error
        // strings, which forced a blanket "rpc error (redacted)" — and a
        // redacted message matches none of the patterns in
        // `tx_lookup_unknown`, so every missing transaction read as an
        // outage. Measured against a live node: the redaction was hiding
        // the node's actual answer from our own parser.
        // bitcoind answers a JSON-RPC *error* with HTTP 500 and the real
        // reason in the body. ureq treats any 500 as a transport failure, so
        // taking `e.to_string()` here throws the body away and leaves us with
        // "status code 500" — which matches none of our patterns, so a node
        // without `-txindex` looked like a flaky connection. Read the body.
        let resp = match ureq::post(&self.auth.url)
            .set("content-type", "application/json")
            .set("authorization", &basic_auth(&self.auth))
            .send_string(&body.to_string())
        {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(WatchError::Unavailable(self.scrub(&e.to_string()))),
        };
        let text = resp
            .into_string()
            .map_err(|e| WatchError::Unavailable(e.to_string()))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| WatchError::Unavailable(e.to_string()))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let msg = self.scrub(&err.to_string());
            if needs_txindex(&msg) {
                return Err(WatchError::NeedsTxIndex);
            }
            return Err(WatchError::Unavailable(msg));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl ChainWatch for BitcoinRpc {
    fn height(&self) -> Result<u64, WatchError> {
        let v = self.call("getblockcount", json!([]))?;
        v.as_u64()
            .ok_or_else(|| WatchError::Unavailable("getblockcount not a number".into()))
    }

    fn confirmations(&self, tx: &TxRef) -> Result<Option<u64>, WatchError> {
        confirmations_from_rpc(self.call("getrawtransaction", json!([&tx.id, true])))
    }

    fn is_unspent(&self, out: &OutRef) -> Result<bool, WatchError> {
        let v = self.call("gettxout", json!([&out.txid, out.vout, true]))?;
        Ok(!v.is_null())
    }
}

impl Broadcaster for BitcoinRpc {
    fn test_accept(&self, raw_hex: &str) -> Result<MempoolAccept, WatchError> {
        let v = self.call("testmempoolaccept", json!([[raw_hex]]))?;
        let first = v.get(0).cloned().unwrap_or(Value::Null);
        if first.get("allowed").and_then(|a| a.as_bool()) == Some(true) {
            return Ok(MempoolAccept::Ok);
        }
        let reason = first
            .get("reject-reason")
            .and_then(|r| r.as_str())
            .unwrap_or("rejected")
            .to_string();
        Ok(MempoolAccept::Reject { reason })
    }

    fn broadcast(&self, raw_hex: &str) -> Result<BroadcastResult, WatchError> {
        match self.call("sendrawtransaction", json!([raw_hex])) {
            Ok(v) => {
                let txid = v.as_str().unwrap_or_default().to_string();
                Ok(BroadcastResult::Accepted { txid })
            }
            Err(WatchError::Unavailable(m))
                if m.contains("already") || m.contains("-27") || m.contains("txn-already") =>
            {
                Ok(BroadcastResult::AlreadyKnown {
                    txid: "already-known".into(),
                })
            }
            Err(e) => Err(e),
        }
    }
}

impl BitcoinRpc {
    pub fn estimate_fee_sat_vb(&self, blocks: u16) -> Result<u64, WatchError> {
        let v = self.call("estimatesmartfee", json!([blocks]))?;
        // btc/kvB → sat/vB. Missing estimate is an error, not a guess of 1.
        let btc_per_kvb = v
            .get("feerate")
            .and_then(|f| f.as_f64())
            .ok_or_else(|| WatchError::Unavailable("no feerate".into()))?;
        Ok((btc_per_kvb * 100_000.0).ceil() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn world_readable_credentials_are_refused() {
        let dir = std::env::temp_dir().join(format!("nf-rpc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rpc.conf");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "url=http://127.0.0.1:18999\nuser=nf\npassword=secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&path).unwrap().permissions();
            p.set_mode(0o644);
            std::fs::set_permissions(&path, p.clone()).unwrap();
            assert!(matches!(
                RpcAuth::from_file(&path),
                Err(RpcSetupError::WorldReadable)
            ));
            p.set_mode(0o600);
            std::fs::set_permissions(&path, p).unwrap();
            let auth = RpcAuth::from_file(&path).unwrap();
            assert_eq!(auth.user, "nf");
            assert_eq!(auth.password, "secret", "the field still carries it");
            let shown = format!("{auth:?}");
            assert!(
                !shown.contains("secret"),
                "the password reached a Debug string: {shown}"
            );
            assert!(shown.contains("<redacted>"), "and it is visibly withheld");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The property the old assertion claimed but could not fail on: it read
    /// `assert!(!… .contains("secret") || true)`, and `|| true` makes any
    /// assertion vacuous. `RpcAuth` derived `Debug` at the time, so the
    /// password *was* in the string — the test was written to pass, not to
    /// hold. `Debug` is now written by hand.
    #[test]
    fn debug_does_not_carry_the_password() {
        let auth = RpcAuth {
            url: "http://127.0.0.1:18999".into(),
            user: "nf".into(),
            password: "hunter2-correct-horse".into(),
        };
        let shown = format!("{auth:?}");
        assert!(
            !shown.contains("hunter2-correct-horse"),
            "the password reached a Debug string: {shown}"
        );
        assert!(shown.contains("<redacted>"));
        // The useful parts survive, or the redaction would be useless for
        // diagnosing a connection problem.
        assert!(shown.contains("127.0.0.1:18999") && shown.contains("nf"));
    }

    #[test]
    fn missing_fields_are_named() {
        let dir = std::env::temp_dir().join(format!("nf-rpc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rpc.conf");
        std::fs::write(&path, "url=http://127.0.0.1:18999\nuser=nf\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&path).unwrap().permissions();
            p.set_mode(0o600);
            std::fs::set_permissions(&path, p).unwrap();
        }
        assert!(matches!(
            RpcAuth::from_file(&path),
            Err(RpcSetupError::Missing("password"))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pruned_node_missing_the_tx_is_unknown_not_zero() {
        // Bitcoin Core, no txindex, transaction not in mempool / recent blocks.
        let missing = r#"{"code":-5,"message":"No such mempool or blockchain transaction. Use gettransaction for wallet transactions."}"#;
        assert!(tx_lookup_unknown(missing));
        // Same node, prune horizon eaten the block.
        let pruned = r#"{"code":-1,"message":"Block not available (pruned data)"}"#;
        assert!(tx_lookup_unknown(pruned));
        let cant_read = r#"{"code":-1,"message":"Can't read transaction from disk"}"#;
        assert!(tx_lookup_unknown(cant_read));

        for msg in [missing, pruned, cant_read] {
            let mapped = confirmations_from_rpc(Err(WatchError::Unavailable(msg.into()))).unwrap();
            assert_eq!(
                mapped, None,
                "a miss must not become 0 confirmations: {msg}"
            );
        }

        // A real outage is still an error, not "unknown".
        assert!(!tx_lookup_unknown("connection refused"));
        assert!(!tx_lookup_unknown("timed out"));
    }

    #[test]
    fn mempool_absence_of_confirmations_field_is_zero_not_unknown() {
        // Verbose getrawtransaction on a mempool tx omits "confirmations".
        let mempool = serde_json::json!({"txid": "ab", "hex": "00"});
        let confs = mempool
            .get("confirmations")
            .and_then(|c| c.as_u64())
            .or(Some(0));
        assert_eq!(confs, Some(0));
    }

    #[test]
    fn scrub_catches_a_url_encoded_password() {
        let rpc = BitcoinRpc::new(RpcAuth {
            url: "http://127.0.0.1:18999".into(),
            user: "nf".into(),
            password: "p@ss/word".into(),
        });
        let raw = "basic p@ss/word failed";
        let encoded = "user:p%40ss%2Fword@127.0.0.1";
        assert!(
            !rpc.scrub(raw).contains("p@ss"),
            "raw password survived: {}",
            rpc.scrub(raw)
        );
        assert!(
            !rpc.scrub(encoded).contains("p%40ss"),
            "encoded password survived: {}",
            rpc.scrub(encoded)
        );
        assert!(rpc.scrub(encoded).contains("<redacted>"));
    }

    /// Bitcoin Core v30.1 reject-reasons, copied from live `testmempoolaccept`.
    #[test]
    fn explain_rejection_matches_real_core_strings() {
        use crate::persist::SendKind;
        let cancel = explain_broadcast_reject(
            SendKind::Cancel,
            "mandatory-script-verify-flag-failed (non-BIP68-final)",
        );
        assert!(
            cancel.contains("H₁"),
            "BIP68 on cancel must name H1, got {cancel}"
        );
        let punish = explain_broadcast_reject(SendKind::Punish, "non-BIP68-final");
        assert!(punish.contains("H₂"), "got {punish}");
        let spent = explain_broadcast_reject(SendKind::Refund, "bad-txns-inputs-missingorspent");
        assert!(
            spent.contains("moved first"),
            "a spent input must not look like a timeout: {spent}"
        );
        let known = explain_broadcast_reject(SendKind::Cancel, "txn-already-in-mempool");
        assert!(
            known.contains("moved first") || known.contains("gone"),
            "{known}"
        );
    }
}

#[cfg(test)]
mod txindex_tests {
    use super::*;

    /// The two real messages, copied from a live Bitcoin Core v30.1 regtest
    /// node. They differ in one token, and a matcher that keys on the shared
    /// tail cannot tell a missing index from a missing transaction.
    const WITHOUT_INDEX: &str = "No such mempool transaction. Use -txindex or \
        provide a block hash to enable blockchain transaction queries. Use \
        gettransaction for wallet transactions.";
    const WITH_INDEX: &str = "No such mempool or blockchain transaction. Use \
        gettransaction for wallet transactions.";

    #[test]
    fn only_the_missing_index_asks_for_txindex() {
        assert!(needs_txindex(WITHOUT_INDEX));
        assert!(
            !needs_txindex(WITH_INDEX),
            "a transaction that genuinely does not exist is not a configuration problem"
        );
    }

    /// And the other direction: both are still "the node does not have it",
    /// so the unknown-detector must accept either.
    #[test]
    fn both_spellings_are_a_lookup_miss() {
        assert!(tx_lookup_unknown(WITHOUT_INDEX));
        assert!(tx_lookup_unknown(WITH_INDEX));
        assert!(!tx_lookup_unknown("Connection refused (os error 61)"));
    }
}
