//! Completing the pre-signed transactions, ready for the wire.
//!
//! Every one of these needs two signatures on a 2-of-2. The handshake already
//! collected the counterparty's half; this adds ours and assembles the
//! witness. Nothing here talks to a node — the caller broadcasts, and the
//! caller is the one who has to decide whether it is time.
//!
//! Who can complete what is not symmetric, and the asymmetry is the design:
//!
//! ```text
//!   TX_cancel   both — each side holds the other's plain signature
//!   TX_refund   Bob only — Alice's half arrives as an adaptor under T_b,
//!               and decrypting it is what publishes s_b
//!   TX_punish   Alice only — Bob signed it plainly, she adds her own
//!   TX_redeem   Alice only — Bob's half is an adaptor under T_a, and
//!               decrypting it is what publishes s_a
//! ```
//!
//! Asking for one you cannot complete is an error rather than a silent
//! empty result: a wallet that quietly does nothing at H₁ is worse than one
//! that says why.

use crate::adaptor;
use crate::session::{Session, SessionError};
use crate::state::Role;
use bitcoin::consensus::serialize;

impl Session {
    /// TX_cancel, signed by both. Either side may broadcast it once H₁ has
    /// passed; the BIP68 sequence is what stops it before then.
    pub fn signed_cancel_hex(&self) -> Result<String, SessionError> {
        let cancel = self.tx_cancel()?;
        let (a, b) = self.spend_keys()?;
        let mine = adaptor::sign(&self.secrets().btc_sk, &cancel.sighash);
        let theirs = self
            .peer_cancel_signature()
            .ok_or(SessionError::MissingSignature("TX_cancel"))?;
        let (sig_a, sig_b) = self.order_sigs(mine, theirs);
        let lock_script = self.lock_script()?;
        let tx = cancel
            .complete(&a, sig_a, &b, sig_b, &lock_script)
            .map_err(|e| SessionError::Tx(e.to_string()))?;
        Ok(hex::encode(serialize(&tx)))
    }

    /// TX_refund. Bob only.
    ///
    /// Alice's half is an adaptor encrypted under `T_b`. Decrypting it with
    /// Bob's own share is what makes the signature valid — and publishing
    /// the result puts `s_b` on chain, which is the price of the refund and
    /// the reason Alice can stop worrying once she sees it.
    pub fn signed_refund_hex(&self) -> Result<String, SessionError> {
        if self.role != Role::Bob {
            return Err(SessionError::WrongRole);
        }
        let cancel = self.tx_cancel()?;
        let refund = self.tx_refund(&cancel)?;
        let enc = self
            .peer_refund_adaptor()
            .ok_or(SessionError::MissingSignature("TX_refund"))?;
        let theirs = adaptor::decrypt(&self.secrets().share.secret(), enc);
        let mine = adaptor::sign(&self.secrets().btc_sk, &refund.sighash);
        let (a, b) = self.spend_keys()?;
        let (sig_a, sig_b) = self.order_sigs(mine, theirs);
        let tx = refund
            .complete(&a, sig_a, &b, sig_b, &cancel.script)
            .map_err(|e| SessionError::Tx(e.to_string()))?;
        Ok(hex::encode(serialize(&tx)))
    }

    /// TX_punish. Alice only, and only after H₂.
    ///
    /// Compensation for NIGHT that is stuck because Bob walked away after
    /// the cancel. Not a second payout — the NIGHT stays stuck either way.
    pub fn signed_punish_hex(&self) -> Result<String, SessionError> {
        if self.role != Role::Alice {
            return Err(SessionError::WrongRole);
        }
        let cancel = self.tx_cancel()?;
        let punish = self.tx_punish(&cancel)?;
        let theirs = self
            .peer_punish_signature()
            .ok_or(SessionError::MissingSignature("TX_punish"))?;
        let mine = adaptor::sign(&self.secrets().btc_sk, &punish.sighash);
        let (a, b) = self.spend_keys()?;
        let (sig_a, sig_b) = self.order_sigs(mine, theirs);
        let tx = punish
            .complete(&a, sig_a, &b, sig_b, &cancel.script)
            .map_err(|e| SessionError::Tx(e.to_string()))?;
        Ok(hex::encode(serialize(&tx)))
    }

    /// TX_redeem. Alice only.
    ///
    /// Broadcasting this is the irreversible step: the decrypted signature
    /// carries `s_a`, and once it is in a block Bob can take the NIGHT
    /// whether or not Alice does anything else.
    pub fn signed_redeem_hex(&self) -> Result<String, SessionError> {
        if self.role != Role::Alice {
            return Err(SessionError::WrongRole);
        }
        let redeem = self.tx_redeem()?;
        let theirs = self.decrypt_redeem()?;
        let mine = adaptor::sign(&self.secrets().btc_sk, &redeem.sighash);
        let (a, b) = self.spend_keys()?;
        let (sig_a, sig_b) = self.order_sigs(mine, theirs);
        let lock_script = self.lock_script()?;
        let tx = redeem
            .complete(&a, sig_a, &b, sig_b, &lock_script)
            .map_err(|e| SessionError::Tx(e.to_string()))?;
        Ok(hex::encode(serialize(&tx)))
    }

    /// `(mine, theirs)` sorted into `(alice, bob)`, which is the order
    /// `complete` pairs with the keys it is given.
    fn order_sigs(
        &self,
        mine: ecdsa_fun::Signature,
        theirs: ecdsa_fun::Signature,
    ) -> (ecdsa_fun::Signature, ecdsa_fun::Signature) {
        match self.role {
            Role::Alice => (mine, theirs),
            Role::Bob => (theirs, mine),
        }
    }
}
