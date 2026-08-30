//! The Bitcoin lock, from this wallet's side of the fence.
//!
//! This wallet holds NIGHT and no Bitcoin keys, so it cannot fund or sign
//! TX_lock. What it can do is build the exact transaction, hand it over for
//! signing, and then refuse anything that comes back that is not the one it
//! built. That last part is the whole point: Alice has already signed
//! children committing to a specific txid, so a "similar" lock is not a
//! near miss, it is theft.

use crate::app::App;
use nightfall_swap::session::SessionError;
use nightfall_swap::ui as logic;
use nightfall_swap::StoredSwap;
use std::str::FromStr;

/// Everything the view needs to show once a lock exists.
pub struct LockExport {
    pub txid: String,
    pub raw_hex: String,
    pub psbt: String,
}

impl App {
    /// The agreed amounts, from the session rather than the stored state.
    ///
    /// `StoredSwap` carries the NIGHT and Bitcoin figures but not the fee,
    /// so reconstructing `Amounts` from it means guessing a zero — and a
    /// preview computed with the wrong fee can say "this output is enough"
    /// where the build then refuses it. Two answers to the same question is
    /// worse than one slow answer.
    pub fn swap_amounts(&mut self, stored: &StoredSwap) -> Option<nightfall_swap::Amounts> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id).ok()?;
        self.swap_sessions.get(&id).map(|s| s.amounts.clone())
    }

    /// Build TX_lock from the typed funding input and produce message 2.
    ///
    /// Nothing is broadcast. The transaction is unsigned — Alice needs the
    /// exact bytes so she can rebuild every child from them, and she gets
    /// them before anyone has signed anything.
    pub fn build_lock(&mut self, stored: &StoredSwap) -> Result<String, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;

        let amounts = {
            let s = self
                .swap_sessions
                .get(&id)
                .ok_or("No open session for this swap.")?;
            s.amounts.clone()
        };

        let funding =
            logic::validate_funding(&self.swap_funding, &amounts).map_err(|e| e.to_string())?;

        let txid =
            bitcoin::Txid::from_str(&funding.txid).map_err(|e| format!("Transaction id: {e}"))?;
        let outpoint = bitcoin::OutPoint {
            txid,
            vout: funding.vout,
        };

        // Change is optional. An empty address with real change left over is
        // a mistake worth naming rather than quietly paying to miners.
        let change_text = self.swap_funding.change_address.trim().to_string();
        let change_due = logic::change_after_lock(&funding, &amounts);
        let change_spk = if change_text.is_empty() {
            if let Some(amount) = change_due {
                return Err(format!(
                    "This output leaves {amount} sat of change. Enter a change \
                     address, or it all goes to the miner."
                ));
            }
            None
        } else {
            Some(self.parse_btc_address(&change_text, "change")?)
        };

        let session = self
            .swap_sessions
            .get_mut(&id)
            .ok_or("No open session for this swap.")?;
        let packet = session
            .lock_packet(
                outpoint,
                bitcoin::Amount::from_sat(funding.value_sats),
                change_spk,
            )
            .map_err(|e| e.to_string())?;
        Ok(packet.encode())
    }

    /// The unsigned transaction, in the two forms a Bitcoin wallet accepts.
    pub fn lock_export(&mut self, stored: &StoredSwap) -> Result<LockExport, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let session = self
            .swap_sessions
            .get(&id)
            .ok_or("No open session for this swap.")?;
        Ok(LockExport {
            txid: session.lock_txid().map_err(|e| e.to_string())?,
            raw_hex: session.unsigned_lock_hex().map_err(|e| e.to_string())?,
            psbt: session.unsigned_lock_psbt().map_err(|e| e.to_string())?,
        })
    }

    /// Check the transaction that came back from the user's Bitcoin wallet.
    ///
    /// Refusing here is not pedantry. A different change output, a different
    /// input, a different fee — any of them changes the txid, and every
    /// pre-signed child of this lock commits to the old one. Accepting a
    /// mismatched transaction would leave both sides holding signatures for
    /// an output that no longer exists.
    pub fn confirm_lock(&mut self, stored: &StoredSwap) -> Result<String, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let raw = self.swap_signed_hex.trim().to_string();
        if raw.is_empty() {
            return Err("Paste the signed transaction from your Bitcoin wallet.".into());
        }
        let session = self
            .swap_sessions
            .get(&id)
            .ok_or("No open session for this swap.")?;

        match session.verify_confirmed_lock_hex(&raw) {
            Ok(()) => Ok(format!(
                "This is the lock we built ({}). Safe to broadcast.",
                short(&session.lock_txid().unwrap_or_default())
            )),
            Err(SessionError::LockMismatch) => Err(
                "That is not the transaction this swap built. Its id or its \
                 locked output differs, which means every signature the other \
                 side already gave you points somewhere else. Do not broadcast it."
                    .into(),
            ),
            Err(e) => Err(e.to_string()),
        }
    }

    fn parse_btc_address(&self, text: &str, what: &str) -> Result<bitcoin::ScriptBuf, String> {
        let net = match self.network {
            nightfall_types::NetworkId::Mainnet => bitcoin::Network::Bitcoin,
            nightfall_types::NetworkId::Testnet => bitcoin::Network::Testnet,
            nightfall_types::NetworkId::Devnet => bitcoin::Network::Regtest,
        };
        let addr = bitcoin::Address::from_str(text)
            .map_err(|e| format!("The {what} address is not a Bitcoin address: {e}"))?
            .require_network(net)
            .map_err(|_| format!("The {what} address belongs to a different Bitcoin network."))?;
        Ok(addr.script_pubkey())
    }
}

fn short(s: &str) -> String {
    if s.len() <= 16 {
        s.to_string()
    } else {
        format!("{}…{}", &s[..8], &s[s.len() - 8..])
    }
}
