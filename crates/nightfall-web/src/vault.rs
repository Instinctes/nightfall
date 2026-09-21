//! 1.0 development API. The published 0.9.5 JS continues to use legacy exports.
//! A host migration must persist/verify ciphertext before retiring legacy state.
//! No API here exports the seed-containing plaintext wallet state to JavaScript.
use super::{err, lights_from_json, parse_amount, DEFAULT_FEE, MATURITY};
use nightfall_crypto::{Address, WalletKeys};
use nightfall_types::{Amount, NetworkId};
use nightfall_wallet::{payment_request::PaymentRequest, vault::Vault, Wallet};
use serde_json::json;
use wasm_bindgen::prelude::*;

/// The browser has no swap executor, deadline watcher or chain reconciliation.
/// Never publish a session that would make its owner believe these obligations
/// are being serviced. Keep this check independent of JS errors for native tests.
fn require_browser_wallet(wallet: &Wallet) -> Result<(), &'static str> {
    if wallet.network != NetworkId::Mainnet {
        return Err("Expected a mainnet wallet.");
    }
    if wallet.has_swap_recovery() {
        return Err("This vault contains swap recovery material. The browser cannot monitor or recover swaps. Open the original vault in a compatible native wallet; preserve this backup.");
    }
    Ok(())
}

fn import_browser_legacy(state: &str) -> Result<Wallet, String> {
    let wallet =
        Wallet::import_state(state).map_err(|_| "Invalid legacy wallet state.".to_owned())?;
    require_browser_wallet(&wallet).map_err(str::to_owned)?;
    Ok(wallet)
}

fn unlock_browser_vault(vault: &mut Vault, password: &str) -> Result<(), String> {
    if !vault.is_locked() {
        return Err("The wallet is already unlocked.".into());
    }
    // Authenticate and inspect an isolated candidate. A rejected swap-bearing
    // vault must stay locked with the exact original ciphertext, not be unlocked
    // briefly and then re-encrypted during an attempted rollback.
    let mut candidate = Vault::from_bytes(vault.sealed_bytes()).map_err(|e| e.to_string())?;
    candidate
        .unlock(password, NetworkId::Mainnet)
        .map_err(|e| e.to_string())?;
    require_browser_wallet(candidate.wallet().map_err(|e| e.to_string())?)
        .map_err(str::to_owned)?;
    *vault = candidate;
    Ok(())
}

fn restore_browser_backup(bytes: &[u8], password: &str) -> Result<Vault, String> {
    let mut vault = Vault::from_bytes(bytes).map_err(|error| error.to_string())?;
    unlock_browser_vault(&mut vault, password)?;
    vault
        .wallet_mut()
        .map_err(|error| error.to_string())?
        .quarantine_imported_sends()
        .map_err(|error| error.to_string())?;
    vault.snapshot().map_err(|error| error.to_string())?;
    Ok(vault)
}

fn require_current_scan(wallet: &Wallet, tip: u64) -> Result<(), &'static str> {
    if wallet.scan_anchor().is_empty() || wallet.needs_light_rebuild() {
        return Err("Complete an anchored chain scan before sending.");
    }
    if wallet.scanned_to() != tip {
        return Err(
            "The wallet scan does not match the node's current height. Synchronize before sending.",
        );
    }
    Ok(())
}

fn pending_transaction(wallet: &Wallet, txid: &str) -> Result<String, &'static str> {
    let entry = wallet
        .history()
        .iter()
        .find(|entry| entry.txid == txid)
        .ok_or("This wallet has no payment with that transaction ID.")?;
    if entry.direction != nightfall_wallet::Direction::Sent
        || !entry.is_pending()
        || entry.quarantined
        || entry.memo == "swap-lock"
    {
        return Err("This payment cannot be broadcast again from this wallet.");
    }
    // The stored record, its raw transaction and the requested ID must agree.
    let valid = wallet
        .resendable()
        .into_iter()
        .any(|(id, transaction)| id == txid && transaction.txid().to_hex() == txid);
    if !valid {
        return Err("The saved payment has no valid transaction to broadcast.");
    }
    entry
        .raw
        .clone()
        .ok_or("The saved payment has no transaction to broadcast.")
}

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
    #[wasm_bindgen(js_name = mainnetGenesis)]
    pub fn mainnet_genesis() -> Result<String, JsError> {
        let config = nightfall_types::GenesisConfig::fair_launch(NetworkId::Mainnet);
        let bytes = serde_json::to_vec(&config).map_err(err)?;
        Ok(nightfall_crypto::genesis_commitment(&bytes).to_hex())
    }

    #[wasm_bindgen(constructor)]
    pub fn from_encrypted(bytes: &[u8]) -> Result<BrowserVault, JsError> {
        Ok(Self {
            inner: Vault::from_bytes(bytes).map_err(err)?,
        })
    }

    /// This is preparation, not migration completion. Never removes old storage.
    #[wasm_bindgen(js_name = fromLegacy)]
    pub fn from_legacy(state: &str, password: &str) -> Result<BrowserVault, JsError> {
        let wallet = import_browser_legacy(state).map_err(err)?;
        Ok(Self {
            inner: Vault::create(&wallet, password).map_err(err)?,
        })
    }

    /// Backup recovery never grants permission to rebroadcast old payments.
    #[wasm_bindgen(js_name = fromBackup)]
    pub fn from_backup(bytes: &[u8], password: &str) -> Result<BrowserVault, JsError> {
        Ok(Self {
            inner: restore_browser_backup(bytes, password).map_err(err)?,
        })
    }

    /// Independent candidate. Persist its snapshot before replacing the live
    /// object; no operation on the fork publishes to the source session.
    pub fn fork(&self) -> Result<BrowserVault, JsError> {
        Ok(Self {
            inner: self.inner.begin_edit().map_err(err)?,
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
        // The same road in as Core and as the words check on the create screen.
        // This used to say "Invalid recovery phrase." while the create screen
        // said the words were incomplete and Core said the checksum was wrong —
        // three wordings for one typo, none of which told the owner which of
        // their 24 words to look at.
        let keys = nightfall_wallet::recovery::keys_from_phrase(phrase).map_err(err)?;
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
        unlock_browser_vault(&mut self.inner, password).map_err(err)
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

    /// Write a payment request for this wallet's own address.
    ///
    /// Amounts arrive as the owner typed them, in NIGHT, and are converted by
    /// the wallet's own amount parser rather than a second one written for this
    /// screen — two parsers for one quantity is two chances to disagree about
    /// somebody's money. An empty amount means the payer chooses.
    ///
    /// `expires_unix` is the payee's own deadline and nothing more. The chain
    /// has no opinion about it and neither does this function.
    ///
    /// The request is built and then read back with the same parser a payer
    /// will use, and a mismatch is an error. That round trip is what validates
    /// it: the memo length, the amount bound and the encoding rules are all
    /// enforced once, in the parser, instead of being restated here where they
    /// could drift.
    #[wasm_bindgen(js_name = paymentRequest)]
    pub fn payment_request(
        &self,
        amount: &str,
        memo: &str,
        invoice: &str,
        expires_unix: &str,
    ) -> Result<String, JsError> {
        let wallet = self.inner.wallet().map_err(err)?;
        let amount = amount.trim();
        let request = PaymentRequest {
            address: wallet.address(),
            network: wallet.network,
            amount_darks: if amount.is_empty() {
                None
            } else {
                Some(parse_amount(amount)?)
            },
            memo: memo.trim().to_owned(),
            invoice: invoice.trim().to_owned(),
            expires_unix: match expires_unix.trim() {
                "" => None,
                text => Some(
                    text.parse()
                        .map_err(|_| err("The expiry must be a whole number of seconds."))?,
                ),
            },
        };
        let uri = request.to_uri();
        match PaymentRequest::parse(&uri) {
            Ok(read_back) if read_back == request => Ok(uri),
            Ok(_) => Err(err(
                "This request did not survive being read back, so it has not been \
                 produced. Please report this.",
            )),
            Err(problem) => Err(err(problem)),
        }
    }

    /// Read a payment request and say what it asks for.
    ///
    /// A request for another network is *described*, not rejected: a wallet
    /// that merely fails to understand a testnet request teaches its owner to
    /// distrust the message rather than the request, and the one thing they
    /// need to be told is precisely why paying it here would be wrong.
    /// `payable` is false and `network_problem` carries the explanation.
    ///
    /// `now_unix` comes from the caller because a browser's clock is the
    /// caller's, not this module's — and an expiry is measured against the
    /// reader's clock, which is the honest thing to say about it.
    #[wasm_bindgen(js_name = readPaymentRequest)]
    pub fn read_payment_request(&self, text: &str, now_unix: f64) -> Result<String, JsError> {
        let now = height(now_unix)?;
        let wallet = self.inner.wallet().map_err(err)?;
        let request = PaymentRequest::parse(text).map_err(err)?;
        let network_problem = request.require_network(wallet.network).err();
        Ok(json!({
            "address": request.address.encode(),
            "network": request.network.as_str(),
            "amount_darks": request.amount_darks.map(|d| d.to_string()),
            "amount_text": request.amount_text(),
            "memo": request.memo,
            "invoice": request.invoice,
            "expires_unix": request.expires_unix.map(|e| e.to_string()),
            "expired": request.is_expired(now),
            "payable": network_problem.is_none(),
            "network_problem": network_problem.map(|e| e.to_string()),
            "mine": request.address == wallet.address(),
            // The canonical spelling, so a payer can compare what they were
            // given with what this wallet read it as.
            "uri": request.to_uri(),
        })
        .to_string())
    }

    pub fn info(&self) -> Result<String, JsError> {
        let w = self.inner.wallet().map_err(err)?;
        Ok(
            json!({"address":w.address_string(), "birth_height":w.birth_height(),
            "scanned_to":w.scanned_to(), "scan_from":w.scan_from(), "outputs":w.spendable_count(),
            "scan_anchor":w.scan_anchor(), "needs_rebuild":w.needs_light_rebuild(),
            "pending":w.unconfirmed_sends().len(), "withheld":w.quarantined().len()})
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
        let wallet = self.inner.wallet().map_err(err)?;
        let rows: Vec<_> = wallet.history().iter().take(80).map(|e| json!({
            "direction":e.direction.label(), "amount":Amount(e.amount).decimal_string(), "fee":Amount(e.fee).decimal_string(),
            "memo":e.memo, "height":e.height, "pending":e.is_pending(), "timestamp":e.timestamp, "txid":e.txid,
            "quarantined":e.quarantined, "retryable":pending_transaction(wallet, &e.txid).is_ok(),
        })).collect();
        serde_json::to_string(&rows).map_err(err)
    }

    #[wasm_bindgen(js_name = resetScan)]
    pub fn reset_scan(&mut self) -> Result<(), JsError> {
        self.begin_rescan()
    }

    /// Mutate a fork, persist it, then replace the live session. Old outgoing
    /// records and invoices survive; every old send is withheld until scanned.
    #[wasm_bindgen(js_name = beginRescan)]
    pub fn begin_rescan(&mut self) -> Result<(), JsError> {
        self.inner
            .wallet_mut()
            .map_err(err)?
            .begin_light_rescan()
            .map_err(err)
    }

    #[wasm_bindgen(js_name = pendingTransaction)]
    pub fn pending_transaction(&self, txid: &str) -> Result<String, JsError> {
        pending_transaction(self.inner.wallet().map_err(err)?, txid).map_err(err)
    }

    /// Ingest only pages whose headers the host checked against one trusted
    /// node. This authenticates continuity, not proof of work. Use on a fork.
    #[wasm_bindgen(js_name = ingestAnchoredPage)]
    pub fn ingest_anchored_page(
        &mut self,
        outputs: &str,
        spent: &str,
        from: f64,
        scanned_to: f64,
        from_hash: &str,
        scanned_hash: &str,
    ) -> Result<String, JsError> {
        let from = height(from)?;
        let scanned_to = height(scanned_to)?;
        if outputs.len() > 4 * 1024 * 1024 || spent.len() > 4 * 1024 * 1024 {
            return Err(err("Scan page exceeds the size limit."));
        }
        let lights = lights_from_json(outputs)?;
        let spent: Vec<String> =
            serde_json::from_str(spent).map_err(|_| err("Invalid spent-output list."))?;
        let wallet = self.inner.wallet_mut().map_err(err)?;
        let found = wallet
            .ingest_anchored_scan_page(&lights, &spent, from, scanned_to, from_hash, scanned_hash)
            .map_err(err)?;
        Ok(json!({"found":found, "scanned_to":wallet.scanned_to(),
            "scan_anchor":wallet.scan_anchor()})
        .to_string())
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
        require_current_scan(wallet, tip).map_err(err)?;
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

    /// Canonical review text without signing or reserving any input.
    #[wasm_bindgen(js_name = paymentDetails)]
    pub fn payment_details(&self, to: &str, amount: &str, memo: &str) -> Result<String, JsError> {
        if to.len() > 256 || amount.len() > 64 || memo.len() > 64 {
            return Err(err(
                "Payment field exceeds the size limit (memo: 64 UTF-8 bytes).",
            ));
        }
        let address = Address::decode(to).map_err(err)?;
        let wallet = self.inner.wallet().map_err(err)?;
        if address == wallet.address() {
            return Err(err("That is your own address."));
        }
        let darks = parse_amount(amount)?;
        let total = darks
            .checked_add(DEFAULT_FEE)
            .ok_or_else(|| err("Amount exceeds the supported range."))?;
        Ok(json!({"address": address.encode(), "amount": Amount(darks).decimal_string(),
            "fee": Amount(DEFAULT_FEE).decimal_string(), "total": Amount(total).decimal_string(), "memo": memo}).to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "public unfunded browser test password";
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    const RECOVERY: &str = "PUBLIC UNFUNDED swap recovery fixture";

    fn wallet() -> Wallet {
        Wallet::in_memory(NetworkId::Mainnet, WalletKeys::from_seed([59; 32]), 0)
    }

    fn funded_wallet() -> Wallet {
        let mut wallet = wallet();
        let (output, _) = nightfall_crypto::create_output(
            &wallet.address(),
            1_000_000,
            "public unfunded fixture",
            wallet.network.proof_context(),
        )
        .unwrap();
        let output = nightfall_wallet::LightOutput {
            height: 0,
            timestamp: 1,
            commit: output.commit.to_hex(),
            ephemeral_pk: output
                .ephemeral_pk
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            output_pk: output
                .output_pk
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            view_tag: output.view_tag,
            payload: output.payload.iter().map(|b| format!("{b:02x}")).collect(),
            coinbase: false,
        };
        let anchor = "42".repeat(32);
        wallet
            .ingest_anchored_scan_page(&[output], &[], 0, 0, &anchor, &anchor)
            .unwrap();
        wallet
    }

    #[test]
    fn browser_send_gate_requires_an_anchor_and_the_current_node_height() {
        assert!(require_current_scan(&wallet(), 0).is_err());
        let wallet = funded_wallet();
        assert!(require_current_scan(&wallet, 0).is_ok());
        assert!(require_current_scan(&wallet, 1).is_err());
        let mut state: serde_json::Value =
            serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
        state["db"]["scanned_to"] = json!(2);
        let ahead = Wallet::import_state(&state.to_string()).unwrap();
        assert!(require_current_scan(&ahead, 1).is_err());
    }

    #[test]
    fn prepared_send_on_a_browser_fork_does_not_move_the_source() {
        let wallet = funded_wallet();
        let original = BrowserVault {
            inner: Vault::create(&wallet, PASSWORD).unwrap(),
        };
        let before = original.inner.wallet().unwrap().export_state().unwrap();
        let mut candidate = original.fork().unwrap();
        let prepared = candidate
            .prepare_send(
                &WalletKeys::from_seed([60; 32]).address().encode(),
                "0.001",
                "public fixture",
                0.0,
                100.0,
            )
            .unwrap();
        let txid = prepared.txid();
        let raw = prepared.tx();
        candidate.commit(prepared).unwrap();
        assert_eq!(
            original.inner.wallet().unwrap().export_state().unwrap(),
            before
        );
        assert_eq!(candidate.pending_transaction(&txid).unwrap(), raw);
        let bytes = candidate.snapshot().unwrap();
        let mut reopened = Vault::from_bytes(&bytes).unwrap();
        reopened.unlock(PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(
            pending_transaction(reopened.wallet().unwrap(), &txid).unwrap(),
            raw
        );
    }

    #[test]
    fn backup_import_withholds_confirmed_and_pending_transactions_without_changing_source() {
        let mut wallet = funded_wallet();
        let transaction = wallet
            .create_payment(
                &WalletKeys::from_seed([60; 32]).address(),
                1000,
                10,
                "public fixture",
            )
            .unwrap();
        wallet
            .record_send_at(&transaction, 1000, "public fixture".into(), 100)
            .unwrap();
        let txid = transaction.txid().to_hex();
        let raw = pending_transaction(&wallet, &txid).unwrap();
        for confirmed in [false, true] {
            let mut state: serde_json::Value =
                serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
            if confirmed {
                state["db"]["history"][0]["height"] = json!(0);
            }
            let wallet = Wallet::import_state(&state.to_string()).unwrap();
            let source = Vault::create(&wallet, PASSWORD).unwrap();
            let bytes = source.sealed_bytes().to_vec();
            let restored = restore_browser_backup(&bytes, PASSWORD).unwrap();
            assert_eq!(source.sealed_bytes(), bytes);
            let entry = restored
                .wallet()
                .unwrap()
                .history()
                .iter()
                .find(|entry| entry.txid == txid)
                .unwrap();
            assert!(entry.quarantined);
            assert_eq!(entry.raw.as_deref(), Some(raw.as_str()));
            assert!(pending_transaction(restored.wallet().unwrap(), &txid).is_err());
        }
        let mut state: serde_json::Value =
            serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
        state["db"]["history"][0]["memo"] = json!("swap-lock");
        assert!(
            pending_transaction(&Wallet::import_state(&state.to_string()).unwrap(), &txid).is_err()
        );
        state["db"]["history"][0]["memo"] = json!("ordinary");
        state["db"]["history"][0]["raw"] = json!("not a transaction");
        assert!(
            pending_transaction(&Wallet::import_state(&state.to_string()).unwrap(), &txid).is_err()
        );
    }

    #[test]
    fn browser_legacy_import_rejects_swap_records_without_exposing_secrets() {
        let mut wallet = wallet();
        assert!(import_browser_legacy(&wallet.export_state().unwrap()).is_ok());
        wallet.put_swap_checkpoint(ID, 0, RECOVERY).unwrap();
        let state = wallet.export_state().unwrap();
        let error = import_browser_legacy(&state).err().unwrap();
        assert!(error.contains("browser cannot monitor or recover swaps"));
        assert!(!error.contains(RECOVERY));
        assert_eq!(wallet.export_state().unwrap(), state);
    }

    #[test]
    fn rejected_browser_unlock_preserves_original_ciphertext_and_lock() {
        let mut wallet = wallet();
        wallet.put_swap_checkpoint(ID, 0, RECOVERY).unwrap();
        let original = Vault::create(&wallet, PASSWORD).unwrap();
        let bytes = original.sealed_bytes().to_vec();
        let mut vault = Vault::from_bytes(&bytes).unwrap();
        let error = unlock_browser_vault(&mut vault, PASSWORD).unwrap_err();
        assert!(error.contains("browser cannot monitor or recover swaps"));
        assert!(!error.contains(RECOVERY));
        assert!(vault.is_locked());
        assert!(vault.wallet().is_err());
        assert_eq!(vault.sealed_bytes(), bytes);

        // Refusal by an unsupported host must not damage native recovery.
        vault.unlock(PASSWORD, NetworkId::Mainnet).unwrap();
        assert_eq!(
            vault.wallet().unwrap().swap_checkpoint(ID),
            Some((1, RECOVERY))
        );
    }

    #[test]
    fn ordinary_browser_unlock_still_authenticates_and_rejects_duplicate_unlock() {
        let wallet = wallet();
        let original = Vault::create(&wallet, PASSWORD).unwrap();
        let bytes = original.sealed_bytes().to_vec();
        let mut vault = Vault::from_bytes(&bytes).unwrap();
        assert!(unlock_browser_vault(&mut vault, "wrong public password").is_err());
        assert!(vault.is_locked());
        assert_eq!(vault.sealed_bytes(), bytes);
        unlock_browser_vault(&mut vault, PASSWORD).unwrap();
        assert_eq!(vault.wallet().unwrap().address(), wallet.address());
        assert!(unlock_browser_vault(&mut vault, PASSWORD).is_err());
        assert_eq!(vault.wallet().unwrap().address(), wallet.address());
        assert_eq!(vault.sealed_bytes(), bytes);
    }
}
