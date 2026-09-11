//! 1.0 development API. The published 0.9.5 JS continues to use legacy exports.
//! A host migration must persist/verify ciphertext before retiring legacy state.
//! No API here exports the seed-containing plaintext wallet state to JavaScript.
use super::{err, lights_from_json, parse_amount, DEFAULT_FEE, MATURITY};
use nightfall_crypto::{Address, WalletKeys};
use nightfall_types::{Amount, NetworkId};
use nightfall_wallet::{vault::Vault, Wallet};
use serde_json::json;
use wasm_bindgen::prelude::*;

fn height(value: f64) -> Result<u64, JsError> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 9_007_199_254_740_991.0
    {
        return Err(err("Expected a non-negative safe integer."));
    }
    Ok(value as u64)
}

#[wasm_bindgen]
pub struct BrowserVault {
    inner: Vault,
}

#[wasm_bindgen]
impl BrowserVault {
    #[wasm_bindgen(constructor)]
    pub fn from_encrypted(bytes: &[u8]) -> Result<BrowserVault, JsError> {
        Ok(Self {
            inner: Vault::from_bytes(bytes).map_err(err)?,
        })
    }

    /// This is preparation, not migration completion. Never removes old storage.
    #[wasm_bindgen(js_name = fromLegacy)]
    pub fn from_legacy(state: &str, password: &str) -> Result<BrowserVault, JsError> {
        let wallet =
            Wallet::import_state(state).map_err(|_| err("Invalid legacy wallet state."))?;
        if wallet.network != NetworkId::Mainnet {
            return Err(err("Expected a mainnet wallet."));
        }
        Ok(Self {
            inner: Vault::create(&wallet, password).map_err(err)?,
        })
    }

    pub fn create(password: &str, birth_height: f64) -> Result<BrowserVault, JsError> {
        let birth = height(birth_height)?;
        let wallet = Wallet::in_memory(NetworkId::Mainnet, WalletKeys::generate(), birth);
        Ok(Self {
            inner: Vault::create(&wallet, password).map_err(err)?,
        })
    }

    pub fn restore(
        phrase: &str,
        password: &str,
        birth_height: f64,
    ) -> Result<BrowserVault, JsError> {
        let birth = height(birth_height)?;
        if phrase.len() > 1024 {
            return Err(err("Invalid recovery phrase."));
        }
        let keys =
            WalletKeys::from_mnemonic(phrase).map_err(|_| err("Invalid recovery phrase."))?;
        let wallet = Wallet::in_memory(NetworkId::Mainnet, keys, birth);
        Ok(Self {
            inner: Vault::create(&wallet, password).map_err(err)?,
        })
    }

    #[wasm_bindgen(js_name = isLocked)]
    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), JsError> {
        self.inner.unlock(password, NetworkId::Mainnet).map_err(err)
    }

    /// Retains ciphertext in this object if browser storage subsequently fails.
    pub fn lock(&mut self) -> Result<Vec<u8>, JsError> {
        self.inner.lock().map_err(err)?;
        Ok(self.inner.sealed_bytes().to_vec())
    }

    pub fn snapshot(&mut self) -> Result<Vec<u8>, JsError> {
        Ok(self.inner.snapshot().map_err(err)?.to_vec())
    }

    #[wasm_bindgen(js_name = changePassword)]
    pub fn change_password(&mut self, password: &str) -> Result<Vec<u8>, JsError> {
        self.inner.change_password(password).map_err(err)?;
        Ok(self.inner.sealed_bytes().to_vec())
    }

    pub fn address(&self) -> Result<String, JsError> {
        Ok(self.inner.wallet().map_err(err)?.address_string())
    }

    /// Explicit reveal only. Caller must clear its own JS/DOM copies afterward.
    #[wasm_bindgen(js_name = recoveryWords)]
    pub fn recovery_words(&self) -> Result<String, JsError> {
        Ok(self.inner.wallet().map_err(err)?.recovery_phrase())
    }

    #[wasm_bindgen(js_name = checkRecovery)]
    pub fn check_recovery(&self, phrase: &str) -> Result<(), JsError> {
        let address = self.inner.wallet().map_err(err)?.address();
        nightfall_wallet::recovery::verify_phrase(&address, phrase).map_err(err)
    }

    #[wasm_bindgen(js_name = viewKey)]
    pub fn view_key(&self) -> Result<String, JsError> {
        Ok(self.inner.wallet().map_err(err)?.view_key_string())
    }

    pub fn info(&self) -> Result<String, JsError> {
        let w = self.inner.wallet().map_err(err)?;
        Ok(
            json!({"address":w.address_string(), "birth_height":w.birth_height(),
            "scanned_to":w.scanned_to(), "scan_from":w.scan_from(), "outputs":w.spendable_count()})
            .to_string(),
        )
    }

    pub fn balance(&self, tip: f64) -> Result<String, JsError> {
        let tip = height(tip)?;
        let w = self.inner.wallet().map_err(err)?;
        let b = w.balances(tip, MATURITY);
        Ok(json!({"available":Amount(b.available).decimal_string(), "immature":Amount(b.immature).decimal_string(),
            "pending_out":Amount(b.pending_out).decimal_string(), "total":Amount(b.total()).decimal_string(),
            "scanned_to":w.scanned_to(), "tip":tip}).to_string())
    }

    pub fn history(&self) -> Result<String, JsError> {
        let rows: Vec<_> = self.inner.wallet().map_err(err)?.history().iter().take(80).map(|e| json!({
            "direction":e.direction.label(), "amount":Amount(e.amount).decimal_string(), "fee":Amount(e.fee).decimal_string(),
            "memo":e.memo, "height":e.height, "pending":e.is_pending(), "timestamp":e.timestamp, "txid":e.txid,
        })).collect();
        serde_json::to_string(&rows).map_err(err)
    }

    #[wasm_bindgen(js_name = resetScan)]
    pub fn reset_scan(&mut self) -> Result<(), JsError> {
        self.inner
            .wallet_mut()
            .map_err(err)?
            .reset_scan()
            .map_err(err)
    }

    #[wasm_bindgen(js_name = ingestPage)]
    pub fn ingest_page(
        &mut self,
        outputs: &str,
        spent: &str,
        scanned_to: f64,
    ) -> Result<String, JsError> {
        let scanned_to = height(scanned_to)?;
        if outputs.len() > 4 * 1024 * 1024 || spent.len() > 4 * 1024 * 1024 {
            return Err(err("Scan page exceeds the size limit."));
        }
        let lights = lights_from_json(outputs)?;
        let spent: Vec<String> =
            serde_json::from_str(spent).map_err(|_| err("Invalid spent-output list."))?;
        let wallet = self.inner.wallet_mut().map_err(err)?;
        let found = wallet
            .ingest_scan_page(&lights, &spent, scanned_to)
            .map_err(err)?;
        Ok(json!({"found":found, "scanned_to":wallet.scanned_to()}).to_string())
    }

    /// Build a payment against a copy of this session, changing nothing here.
    ///
    /// Takes `&self` on purpose: preparing a payment must not be able to move
    /// this wallet. The returned [`PreparedSend`] carries the transaction and
    /// the ciphertext that has to be stored before anyone acts on it.
    ///
    /// The order is not advice:
    ///
    /// 1. `const p = vault.prepareSend(…)`
    /// 2. read `p.tx`, `p.txid`, `p.fee`
    /// 3. store `p.sealed` durably
    /// 4. `vault.commit(p)`
    /// 5. only now broadcast the transaction
    ///
    /// Anything that goes wrong before step 3 costs nothing: this wallet never
    /// moved and the transaction was never on the wire. That is the property
    /// the old `buildSend` could not offer — it recorded the send into the
    /// live session and handed the transaction back in the same call, so a
    /// failure to store afterwards left a spendable transaction in the
    /// caller's hands while the stored wallet still showed the coins unspent.
    #[wasm_bindgen(js_name = prepareSend)]
    pub fn prepare_send(
        &self,
        to: &str,
        amount: &str,
        memo: &str,
        tip: f64,
        now: f64,
    ) -> Result<PreparedSend, JsError> {
        let tip = height(tip)?;
        let now = height(now)?;
        if to.len() > 256 || amount.len() > 64 || memo.len() > 64 {
            return Err(err("Payment field exceeds the size limit."));
        }
        let addr = Address::decode(to).map_err(err)?;
        let darks = parse_amount(amount)?;

        let mut candidate = self.inner.begin_edit().map_err(err)?;
        let wallet = candidate.wallet_mut().map_err(err)?;
        if addr == wallet.address() {
            return Err(err("That is your own address."));
        }
        let tx = wallet
            .create_payment_at(&addr, darks, DEFAULT_FEE, memo, tip, MATURITY)
            .map_err(err)?;
        wallet
            .record_send_at(&tx, darks, memo.to_owned(), now)
            .map_err(err)?;
        let txid = tx.txid().to_hex();
        let tx_json = serde_json::to_string(&tx).map_err(err)?;
        let sealed = candidate.snapshot().map_err(err)?.to_vec();
        Ok(PreparedSend {
            candidate,
            tx: tx_json,
            txid,
            fee: Amount(DEFAULT_FEE).decimal_string(),
            sealed,
        })
    }

    /// Adopt a prepared payment whose ciphertext is already stored.
    ///
    /// Call this only after `prepared.sealed` is durably written, and before
    /// broadcasting. Committing without storing first reverses the order this
    /// exists to enforce; nothing here can detect that, which is why it is
    /// written down rather than merely implied.
    pub fn commit(&mut self, prepared: PreparedSend) -> Result<(), JsError> {
        self.inner.adopt(prepared.candidate).map_err(err)
    }
}

/// A payment built against a copy of a session, not against the session.
///
/// Holding one of these means the wallet has moved in a copy and nowhere
/// else. The transaction inside must not be broadcast until `sealed` has been
/// stored durably and `commit` has accepted it: until then the stored wallet
/// still shows the coins unspent, and a crash in between would leave a
/// payment in the world that the wallet does not know it made.
///
/// Dropping this without committing discards it completely — the wallet it
/// was prepared from never changed, so there is nothing to undo.
#[wasm_bindgen]
pub struct PreparedSend {
    candidate: Vault,
    tx: String,
    txid: String,
    fee: String,
    sealed: Vec<u8>,
}

#[wasm_bindgen]
impl PreparedSend {
    /// The ciphertext to store durably BEFORE committing or broadcasting.
    #[wasm_bindgen(getter)]
    pub fn sealed(&self) -> Vec<u8> {
        self.sealed.clone()
    }

    /// The transaction, as JSON. Safe to broadcast only after `commit`.
    #[wasm_bindgen(getter)]
    pub fn tx(&self) -> String {
        self.tx.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn txid(&self) -> String {
        self.txid.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn fee(&self) -> String {
        self.fee.clone()
    }

    /// Throw this away. Equivalent to letting it go out of scope; here so the
    /// intent reads as a decision rather than as forgetting to use it.
    pub fn discard(self) {}
}
