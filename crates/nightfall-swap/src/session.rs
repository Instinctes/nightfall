//! The handshake: turning the message definitions into an actual protocol.
//!
//! Everything under this module already existed as parts — key shares with
//! DLEQ proofs, the 2-of-2 script, the four Bitcoin transactions, adaptor
//! signatures, the state machine. What did not exist was the thing that puts
//! them in order and remembers where it is. Without it there is a wallet view
//! over a state machine that nothing ever drives.
//!
//! Message order, and who speaks:
//!
//! ```text
//!   Bob   → Message0            keys, his refund address, amounts, depths
//!   Alice → Message1            keys, her redeem and punish addresses, scan secret
//!   Bob   → Message2            TX_lock, unsigned, so Alice can rebuild every child
//!   Alice → Message3            her TX_cancel signature, her TX_refund adaptor
//!   Bob   → Message4            his TX_punish and TX_cancel signatures
//!   Bob   → MessageRedeemEnc    his TX_redeem adaptor, encrypted under T_a
//! ```
//!
//! Bob locks Bitcoin, Alice locks NIGHT. Alice decrypting the redeem adaptor
//! publishes `s_a`, which is how Bob learns the scalar that opens the NIGHT
//! lock. That is the whole trick, and it is the reason the redeem adaptor is
//! sent last: before it, neither side can move.
//!
//! Secrets never enter a message. [`Secrets`] stays on the machine that made
//! it, and `Debug` is written by hand so it cannot reach a log.

use crate::adaptor;
use crate::bitcoin_tx::{self, TxCancel, TxLock, TxPunish, TxRedeem, TxRefund};
use crate::messages::{
    Amounts, Message0, Message1, Message2, Message3, Message4, MessageRedeemEnc,
};
use crate::packet::{Packet, PacketError};
use crate::persist::{self, PersistError};
use crate::state::Role;
use crate::timelock::Depths;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction};
use curve25519_dalek::scalar::Scalar;
use ecdsa_fun::adaptor::EncryptedSignature;
use ecdsa_fun::fun::{Point, Scalar as SecpScalar};
use ecdsa_fun::Signature;
use nightfall_crypto::swap::{SharedLock, SwapOffer, SwapShare};
use nightfall_types::NetworkId;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

/// No further message is expected or produced on this side.
const DONE: u8 = u8::MAX;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("this step belongs to the other role")]
    WrongRole,
    #[error("message {got} arrived before {want}")]
    OutOfOrder { got: u8, want: u8 },
    #[error("the counterparty's key proof did not verify")]
    BadOffer,
    #[error("the counterparty's key is not a valid point")]
    BadKey,
    #[error("the counterparty's script is not spendable")]
    BadScript,
    #[error("the counterparty changed the agreed amounts or depths")]
    TermsChanged,
    #[error("their signature does not match the transaction we built")]
    BadSignature,
    #[error("bitcoin transaction: {0}")]
    Tx(String),
    #[error("the Bitcoin lock is not known yet")]
    NoLock,
    #[error("packet: {0}")]
    Packet(#[from] PacketError),
    #[error("packet body is not the message this step expects")]
    WrongBody,
    #[error("the counterparty's timelocks leave no way to finish")]
    BadDepths,
    #[error("session file is readable by others; chmod 600 required")]
    WorldReadable,
    #[error("stored share is not a valid swap secret")]
    BadShare,
    #[error("no session file for this swap")]
    NoSession,
    #[error("session file is corrupt")]
    Corrupt,
    #[error("confirmed Bitcoin lock is not the transaction this session built")]
    LockMismatch,
    /// We hold no counterparty half for that transaction, so it cannot be
    /// completed. Named rather than silent: a wallet that quietly does
    /// nothing at H1 is worse than one that says what is missing.
    #[error("the counterparty's signature for {0} is missing")]
    MissingSignature(&'static str),
}

/// Local key material. Never serialised into a packet.
#[derive(Clone)]
pub struct Secrets {
    pub share: SwapShare,
    pub btc_sk: SecpScalar,
}

/// Written by hand so a panic or a `{:?}` cannot print a key. Same reason
/// `RpcAuth` does it: the moment something goes wrong is exactly the moment
/// this struct ends up in a log the user pastes somewhere public.
impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("share", &"<secret>")
            .field("btc_sk", &"<secret>")
            .finish()
    }
}

impl Secrets {
    pub fn generate() -> Self {
        Self {
            share: SwapShare::generate(),
            btc_sk: adaptor::random_bitcoin_sk(&mut OsRng),
        }
    }

    pub fn btc_pk(&self) -> Point {
        adaptor::verification_key(&self.btc_sk)
    }
}

/// One side of one swap, mid-handshake.
pub struct Session {
    pub id: Uuid,
    pub role: Role,
    pub network: NetworkId,
    pub amounts: Amounts,
    pub depths: Depths,
    /// Sequence number of the next message we expect to receive.
    pub expect: u8,
    /// Sequence number of the next message we will send.
    pub emit: u8,

    secrets: Secrets,

    peer_btc: Option<Point>,
    peer_offer: Option<SwapOffer>,
    scan_secret: Option<Scalar>,

    /// Bob's, where his Bitcoin goes if the swap is refunded.
    refund_spk: Option<ScriptBuf>,
    /// Alice's, where the Bitcoin goes on a successful redeem.
    redeem_spk: Option<ScriptBuf>,
    /// Alice's, where the Bitcoin goes if she punishes.
    punish_spk: Option<ScriptBuf>,

    lock: Option<TxLock>,
    peer_cancel_sig: Option<Signature>,
    peer_refund_encsig: Option<EncryptedSignature>,
    peer_punish_sig: Option<Signature>,
    redeem_encsig: Option<EncryptedSignature>,

    /// Last packet we produced, so a restart can re-copy it. `next_packet`
    /// advances `emit`; without this the other side losing a paste would
    /// stall a swap that has not even locked yet.
    last_packet: Option<Packet>,
    /// Where to write progress. When set, every call that advances the
    /// handshake saves *before* handing the packet back — see `checkpoint`.
    datadir: Option<PathBuf>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("expect", &self.expect)
            .field("emit", &self.emit)
            .field("has_lock", &self.lock.is_some())
            .finish()
    }
}

fn spk(bytes: &[u8]) -> Result<ScriptBuf, SessionError> {
    if bytes.is_empty() || bytes.len() > 42 {
        return Err(SessionError::BadScript);
    }
    Ok(ScriptBuf::from_bytes(bytes.to_vec()))
}

fn point(bytes: &[u8]) -> Result<Point, SessionError> {
    if bytes.len() != 33 {
        return Err(SessionError::BadKey);
    }
    let mut b = [0u8; 33];
    b.copy_from_slice(bytes);
    Point::from_bytes(b).ok_or(SessionError::BadKey)
}

impl Session {
    /// Bob opens: he is the one who will lock Bitcoin.
    pub fn open_as_bob(
        network: NetworkId,
        amounts: Amounts,
        depths: Depths,
        refund_spk: ScriptBuf,
    ) -> Self {
        Self::blank(Uuid::new_v4(), Role::Bob, network, amounts, depths)
            .with(|s| s.refund_spk = Some(refund_spk))
    }

    /// Alice joins an offer Bob made. The id comes from his message, not from
    /// her — two sides inventing ids is two swaps.
    pub fn join_as_alice(
        network: NetworkId,
        redeem_spk: ScriptBuf,
        punish_spk: ScriptBuf,
        m0: &Message0,
    ) -> Result<Self, SessionError> {
        if !m0.depths.alice_can_finish() {
            return Err(SessionError::BadDepths);
        }
        let mut s = Self::blank(
            m0.swap_id,
            Role::Alice,
            network,
            m0.amounts.clone(),
            m0.depths,
        )
        .with(|s| {
            s.redeem_spk = Some(redeem_spk);
            s.punish_spk = Some(punish_spk);
            s.scan_secret = Some(SharedLock::fresh_scan_secret());
        });
        s.accept0(m0)?;
        Ok(s)
    }

    /// Alice's real entry point: she meets the swap as a pasted packet, not
    /// as a struct.
    ///
    /// The opening packet is the only one whose terms she cannot check
    /// against something she already agreed — it *is* the offer. So the
    /// envelope is verified (version, checksum, network, sequence) and the
    /// terms are handed to the human to look at. Everything after this is
    /// checked against what this packet established.
    pub fn join_from_packet(
        network: NetworkId,
        redeem_spk: ScriptBuf,
        punish_spk: ScriptBuf,
        p: &Packet,
    ) -> Result<Self, SessionError> {
        p.verify_open(p.swap_id, network, 0, &p.amounts)?;
        let m0: Message0 = from(p.body.clone())?;
        if m0.amounts != p.amounts || m0.swap_id != p.swap_id {
            // Envelope and body disagreeing means the packet was assembled by
            // hand from two different swaps.
            return Err(SessionError::TermsChanged);
        }
        Self::join_as_alice(network, redeem_spk, punish_spk, &m0)
    }

    fn blank(id: Uuid, role: Role, network: NetworkId, amounts: Amounts, depths: Depths) -> Self {
        Self {
            id,
            role,
            network,
            amounts,
            depths,
            expect: if role == Role::Alice { 0 } else { 1 },
            emit: if role == Role::Bob { 0 } else { 1 },
            secrets: Secrets::generate(),
            peer_btc: None,
            peer_offer: None,
            scan_secret: None,
            refund_spk: None,
            redeem_spk: None,
            punish_spk: None,
            lock: None,
            peer_cancel_sig: None,
            peer_refund_encsig: None,
            peer_punish_sig: None,
            redeem_encsig: None,
            last_packet: None,
            datadir: None,
        }
    }

    fn with(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }

    pub fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    fn need(&self, want_role: Role) -> Result<(), SessionError> {
        if self.role == want_role {
            Ok(())
        } else {
            Err(SessionError::WrongRole)
        }
    }
}

// ------------------------------------------------------------- the messages ---

impl Session {
    /// Bob's opening: his 2-of-2 key, his proven share, where a refund goes,
    /// and the terms. Everything Alice needs to decide whether to play.
    pub fn message0(&self) -> Result<Message0, SessionError> {
        self.need(Role::Bob)?;
        Ok(Message0 {
            swap_id: self.id,
            b_btc: self.secrets.btc_pk().to_bytes().to_vec(),
            offer_b: self.secrets.share.offer(),
            refund_spk: self
                .refund_spk
                .as_ref()
                .ok_or(SessionError::BadScript)?
                .to_bytes(),
            amounts: self.amounts.clone(),
            depths: self.depths,
        })
    }

    fn accept0(&mut self, m: &Message0) -> Result<(), SessionError> {
        self.need(Role::Alice)?;
        // Fail closed. A rogue share that is not the discrete log of both
        // points cannot produce a proof, and without this check the whole
        // 2-of-2 is one party's to spend.
        if !m.offer_b.verify() {
            return Err(SessionError::BadOffer);
        }
        self.peer_btc = Some(point(&m.b_btc)?);
        self.peer_offer = Some(m.offer_b.clone());
        self.refund_spk = Some(spk(&m.refund_spk)?);
        self.expect = 2;
        Ok(())
    }

    /// Alice's reply. The scan secret is in here on purpose: without it Bob
    /// can neither find nor spend the NIGHT output, even holding both key
    /// halves. It grants sight, never authority.
    pub fn message1(&self) -> Result<Message1, SessionError> {
        self.need(Role::Alice)?;
        Ok(Message1 {
            swap_id: self.id,
            a_btc: self.secrets.btc_pk().to_bytes().to_vec(),
            offer_a: self.secrets.share.offer(),
            redeem_spk: self
                .redeem_spk
                .as_ref()
                .ok_or(SessionError::BadScript)?
                .to_bytes(),
            punish_spk: self
                .punish_spk
                .as_ref()
                .ok_or(SessionError::BadScript)?
                .to_bytes(),
            scan_secret: self.scan_secret.ok_or(SessionError::BadKey)?.to_bytes(),
        })
    }

    fn accept1(&mut self, m: &Message1) -> Result<(), SessionError> {
        self.need(Role::Bob)?;
        if !m.offer_a.verify() {
            return Err(SessionError::BadOffer);
        }
        let scan = Option::<Scalar>::from(Scalar::from_canonical_bytes(m.scan_secret))
            .ok_or(SessionError::BadKey)?;
        self.peer_btc = Some(point(&m.a_btc)?);
        self.peer_offer = Some(m.offer_a.clone());
        self.redeem_spk = Some(spk(&m.redeem_spk)?);
        self.punish_spk = Some(spk(&m.punish_spk)?);
        self.scan_secret = Some(scan);
        self.expect = 3;
        Ok(())
    }

    /// Bob builds TX_lock and sends it *unsigned*. Alice needs the exact
    /// transaction, not a promise about it: every child transaction she is
    /// about to sign commits to this txid.
    pub fn build_lock(
        &mut self,
        prev: OutPoint,
        prev_value: Amount,
        change_spk: Option<ScriptBuf>,
    ) -> Result<Message2, SessionError> {
        self.need(Role::Bob)?;
        let (a, b) = self.keys()?;
        let lock = TxLock::from_prevout(
            prev,
            prev_value,
            &a,
            &b,
            Amount::from_sat(self.amounts.btc_sats),
            Amount::from_sat(self.amounts.btc_fee_sats),
            change_spk,
        )
        .map_err(|e| SessionError::Tx(e.to_string()))?;
        let out = Message2 {
            swap_id: self.id,
            tx_lock: serialize(&lock.tx),
        };
        self.lock = Some(lock);
        Ok(out)
    }

    fn accept2(&mut self, m: &Message2) -> Result<(), SessionError> {
        self.need(Role::Alice)?;
        let tx: bitcoin::Transaction =
            deserialize(&m.tx_lock).map_err(|e| SessionError::Tx(e.to_string()))?;
        let (a, b) = self.keys()?;
        let script = bitcoin_tx::two_of_two(&a, &b).map_err(|e| SessionError::Tx(e.to_string()))?;
        let want_spk = bitcoin_tx::p2wsh(&script);
        let value = Amount::from_sat(self.amounts.btc_sats);

        // Rebuild rather than believe. Bob could have sent a transaction that
        // pays a different script or a smaller amount; either would leave
        // Alice signing children of an output she cannot claim.
        let vout = tx
            .output
            .iter()
            .position(|o| o.script_pubkey == want_spk && o.value == value)
            .ok_or(SessionError::TermsChanged)? as u32;

        self.lock = Some(TxLock {
            tx,
            script,
            value,
            vout,
        });
        self.expect = 4;
        Ok(())
    }

    /// Alice hands Bob the two signatures that make his exit possible: the
    /// cancel, and the refund locked behind `T_b`.
    ///
    /// She signs these *before* he broadcasts the lock. That order is the
    /// whole safety property for Bob — his Bitcoin is never in a 2-of-2 he
    /// cannot leave.
    pub fn message3(&self) -> Result<Message3, SessionError> {
        self.need(Role::Alice)?;
        let cancel = self.tx_cancel()?;
        let refund = self.tx_refund(&cancel)?;
        let t_b = self.peer_t()?;
        Ok(Message3 {
            swap_id: self.id,
            tx_cancel_sig: adaptor::sign(&self.secrets.btc_sk, &cancel.sighash),
            tx_refund_encsig: bitcoin_tx::alice_encsign_refund(&self.secrets.btc_sk, &t_b, &refund),
        })
    }

    fn accept3(&mut self, m: &Message3) -> Result<(), SessionError> {
        self.need(Role::Bob)?;
        let cancel = self.tx_cancel()?;
        let refund = self.tx_refund(&cancel)?;
        let a = self.peer_btc.ok_or(SessionError::BadKey)?;
        let t_b = adaptor::encryption_point(&self.secrets.share.secret());

        if !adaptor::verify_sig(&a, &cancel.sighash, &m.tx_cancel_sig) {
            return Err(SessionError::BadSignature);
        }
        if !adaptor::verify_encsig(&a, &t_b, &refund.sighash, &m.tx_refund_encsig) {
            return Err(SessionError::BadSignature);
        }
        self.peer_cancel_sig = Some(m.tx_cancel_sig.clone());
        self.peer_refund_encsig = Some(m.tx_refund_encsig.clone());
        // Bob receives nothing after this.
        self.expect = DONE;
        Ok(())
    }

    /// Bob's half of the abort tree, so Alice can punish if he stalls after a
    /// cancel, and cancel at all.
    pub fn message4(&self) -> Result<Message4, SessionError> {
        self.need(Role::Bob)?;
        let cancel = self.tx_cancel()?;
        let punish = self.tx_punish(&cancel)?;
        Ok(Message4 {
            swap_id: self.id,
            tx_punish_sig: adaptor::sign(&self.secrets.btc_sk, &punish.sighash),
            tx_cancel_sig: adaptor::sign(&self.secrets.btc_sk, &cancel.sighash),
        })
    }

    fn accept4(&mut self, m: &Message4) -> Result<(), SessionError> {
        self.need(Role::Alice)?;
        let cancel = self.tx_cancel()?;
        let punish = self.tx_punish(&cancel)?;
        let b = self.peer_btc.ok_or(SessionError::BadKey)?;
        if !adaptor::verify_sig(&b, &punish.sighash, &m.tx_punish_sig)
            || !adaptor::verify_sig(&b, &cancel.sighash, &m.tx_cancel_sig)
        {
            return Err(SessionError::BadSignature);
        }
        self.peer_punish_sig = Some(m.tx_punish_sig.clone());
        self.peer_cancel_sig = Some(m.tx_cancel_sig.clone());
        self.expect = 5;
        Ok(())
    }

    /// The last message, and the one that starts the clock. Once Alice holds
    /// this she can take the Bitcoin — and doing so publishes `s_a`.
    pub fn message_redeem_enc(&self) -> Result<MessageRedeemEnc, SessionError> {
        self.need(Role::Bob)?;
        let redeem = self.tx_redeem()?;
        let t_a = self.peer_t()?;
        Ok(MessageRedeemEnc {
            swap_id: self.id,
            tx_redeem_encsig: bitcoin_tx::bob_encsign_redeem(&self.secrets.btc_sk, &t_a, &redeem),
        })
    }

    fn accept_redeem_enc(&mut self, m: &MessageRedeemEnc) -> Result<(), SessionError> {
        self.need(Role::Alice)?;
        let redeem = self.tx_redeem()?;
        let b = self.peer_btc.ok_or(SessionError::BadKey)?;
        let t_a = adaptor::encryption_point(&self.secrets.share.secret());
        if !adaptor::verify_encsig(&b, &t_a, &redeem.sighash, &m.tx_redeem_encsig) {
            return Err(SessionError::BadSignature);
        }
        self.redeem_encsig = Some(m.tx_redeem_encsig.clone());
        self.expect = DONE;
        Ok(())
    }
}

// ------------------------------------------- the transactions, rebuilt by both ---

impl Session {
    /// `(alice_key, bob_key)` in that order. `two_of_two` sorts canonically,
    /// so both sides end up with the same script whichever way round it is
    /// called — but the adaptor and signature checks care, so keep it honest.
    fn keys(&self) -> Result<(Point, Point), SessionError> {
        let mine = self.secrets.btc_pk();
        let theirs = self.peer_btc.ok_or(SessionError::BadKey)?;
        Ok(match self.role {
            Role::Alice => (mine, theirs),
            Role::Bob => (theirs, mine),
        })
    }

    /// The counterparty's `T`, the point their half of the secret is behind.
    fn peer_t(&self) -> Result<Point, SessionError> {
        let offer = self.peer_offer.as_ref().ok_or(SessionError::BadOffer)?;
        point(&offer.t)
    }

    /// `(alice_key, bob_key)`, the order `complete` expects.
    pub(crate) fn spend_keys(&self) -> Result<(Point, Point), SessionError> {
        self.keys()
    }

    pub(crate) fn lock_script(&self) -> Result<ScriptBuf, SessionError> {
        Ok(self.lock_ref()?.script.clone())
    }

    pub(crate) fn peer_cancel_signature(&self) -> Option<Signature> {
        self.peer_cancel_sig.clone()
    }

    pub(crate) fn peer_punish_signature(&self) -> Option<Signature> {
        self.peer_punish_sig.clone()
    }

    pub(crate) fn peer_refund_adaptor(&self) -> Option<EncryptedSignature> {
        self.peer_refund_encsig.clone()
    }

    fn lock_ref(&self) -> Result<&TxLock, SessionError> {
        self.lock.as_ref().ok_or(SessionError::NoLock)
    }

    /// One fee for every child of TX_lock.
    ///
    /// Cancel, refund and punish are pre-signed, so this number is frozen
    /// at handshake time — the sighash binds it. Redeem is adaptor-signed
    /// in message 5, so it is frozen then too. [`crate::fees::FeeLadder`]
    /// exists so a later handshake can pre-sign several rungs; this session
    /// still signs one, the amount both sides agreed in the opening packet.
    /// Using the ladder here to pick a *different* fee than `amounts` would
    /// make Alice rebuild children Bob did not sign.
    fn fee(&self) -> Amount {
        Amount::from_sat(self.amounts.btc_fee_sats)
    }

    pub fn tx_redeem(&self) -> Result<TxRedeem, SessionError> {
        let dest = self.redeem_spk.clone().ok_or(SessionError::BadScript)?;
        TxRedeem::new(self.lock_ref()?, dest, self.fee())
            .map_err(|e| SessionError::Tx(e.to_string()))
    }

    pub fn tx_cancel(&self) -> Result<TxCancel, SessionError> {
        let (a, b) = self.keys()?;
        TxCancel::new(self.lock_ref()?, &a, &b, self.depths.cancel, self.fee())
            .map_err(|e| SessionError::Tx(e.to_string()))
    }

    pub fn tx_refund(&self, cancel: &TxCancel) -> Result<TxRefund, SessionError> {
        let dest = self.refund_spk.clone().ok_or(SessionError::BadScript)?;
        TxRefund::new(cancel, dest, self.fee()).map_err(|e| SessionError::Tx(e.to_string()))
    }

    pub fn tx_punish(&self, cancel: &TxCancel) -> Result<TxPunish, SessionError> {
        let dest = self.punish_spk.clone().ok_or(SessionError::BadScript)?;
        TxPunish::new(cancel, dest, self.depths.punish, self.fee())
            .map_err(|e| SessionError::Tx(e.to_string()))
    }

    /// The NIGHT side: the address Alice pays into and Bob later claims from.
    ///
    /// Built from *verified* offers only. `SharedLock::new` still exists so
    /// the rogue-key test can demonstrate the theft; production goes through
    /// here and nowhere else.
    pub fn shared_lock(&self) -> Result<SharedLock, SessionError> {
        let theirs = self.peer_offer.as_ref().ok_or(SessionError::BadOffer)?;
        let mine = self.secrets.share.offer();
        let scan = self.scan_secret.ok_or(SessionError::BadKey)?;
        let (a, b) = match self.role {
            Role::Alice => (&mine, theirs),
            Role::Bob => (theirs, &mine),
        };
        SharedLock::from_verified_offers(a, b, scan).ok_or(SessionError::BadOffer)
    }

    /// Alice: turn Bob's adaptor into a real signature. This is the step that
    /// publishes `s_a` the moment the transaction is broadcast.
    pub fn decrypt_redeem(&self) -> Result<Signature, SessionError> {
        self.need(Role::Alice)?;
        let enc = self
            .redeem_encsig
            .clone()
            .ok_or(SessionError::OutOfOrder { got: 0, want: 6 })?;
        Ok(adaptor::decrypt(&self.secrets.share.secret(), enc))
    }

    /// Bob: pull `s_a` back out of the signature Alice published.
    pub fn recover_from_redeem(&self, published: &Signature) -> Option<Scalar> {
        let enc = self.redeem_encsig.as_ref()?;
        let t_a = point(&self.peer_offer.as_ref()?.t).ok()?;
        adaptor::recover(&t_a, published, enc)
    }

    /// Bob's copy of the redeem adaptor, kept so he can recover from it.
    pub fn remember_redeem_enc(&mut self, m: &MessageRedeemEnc) {
        self.redeem_encsig = Some(m.tx_redeem_encsig.clone());
    }

    /// The scalar that opens the NIGHT lock, once both halves are known.
    pub fn night_claim_secret(
        &self,
        peer_scalar: &Scalar,
        ephemeral_pk: &[u8; 32],
    ) -> Option<Scalar> {
        let lock = self.shared_lock().ok()?;
        let (a, b) = match self.role {
            Role::Alice => (self.secrets.share.secret(), *peer_scalar),
            Role::Bob => (*peer_scalar, self.secrets.share.secret()),
        };
        lock.claim_secret(&a, &b, ephemeral_pk)
    }
}

// ----------------------------------------------------------------- packets ---

/// What a received packet turned out to be, so the wallet can say something
/// specific instead of "imported".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    Opening,
    Reply,
    BitcoinLock,
    AbortSignatures,
    PunishSignatures,
    RedeemAdaptor,
}

impl Accepted {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Opening => "Their offer checked out. Send them your reply.",
            Self::Reply => "Their keys checked out. Build the Bitcoin lock next.",
            Self::BitcoinLock => "Their Bitcoin lock is the one we agreed. Sign the abort tree.",
            Self::AbortSignatures => "Their abort signatures verified. Send yours.",
            Self::PunishSignatures => "Their punish and cancel signatures verified.",
            Self::RedeemAdaptor => "Redeem adaptor verified. The swap can now complete.",
        }
    }
}

impl Session {
    /// Write progress to disk from now on, and do it before returning any
    /// packet.
    ///
    /// Without this, saving is the caller's duty, and a crash between
    /// producing a packet and remembering that we produced it loses the
    /// swap: `emit` has advanced in memory only, so on restart `next_packet`
    /// refuses that sequence number and `last_packet` is the one before.
    /// After the Bitcoin lock that costs the coin. Making it structural is
    /// the same rule the wallet already follows for payments — persist
    /// before send — one layer up.
    pub fn persist_to(&mut self, datadir: &Path) {
        self.datadir = Some(datadir.to_path_buf());
    }

    pub fn datadir(&self) -> Option<&Path> {
        self.datadir.as_deref()
    }

    /// Save, if a datadir is set. Called by every step that advances the
    /// handshake, *after* the mutation and *before* the caller sees the
    /// packet.
    fn checkpoint(&self) -> Result<(), SessionError> {
        match &self.datadir {
            None => Ok(()),
            Some(dir) => self.save(dir).map_err(|e| match e {
                PersistError::WorldReadable => SessionError::WorldReadable,
                _ => SessionError::Corrupt,
            }),
        }
    }

    /// Wrap the next outbound message for the counterparty.
    ///
    /// The sequence number is the session's, not the caller's. A caller who
    /// could choose it could replay message 3 as message 5.
    pub fn next_packet(&mut self) -> Result<Packet, SessionError> {
        let (seq, body) = match (self.role, self.emit) {
            (Role::Bob, 0) => (0, serde_json::to_value(self.message0()?)),
            (Role::Alice, 1) => (1, serde_json::to_value(self.message1()?)),
            (Role::Alice, 3) => (3, serde_json::to_value(self.message3()?)),
            (Role::Bob, 4) => (4, serde_json::to_value(self.message4()?)),
            (Role::Bob, 5) => (5, serde_json::to_value(self.message_redeem_enc()?)),
            _ => return Err(SessionError::WrongRole),
        };
        let body = body.map_err(|_| SessionError::WrongBody)?;
        let packet = Packet::new(self.network, self.id, seq, self.amounts.clone(), body);
        let emit_before = self.emit;
        let last_before = self.last_packet.clone();
        // Written out rather than computed. The two sides do not alternate —
        // Bob sends 0, 2, 4 and 5, Alice sends 1 and 3 — so any arithmetic
        // rule is wrong somewhere, and being wrong here means a packet the
        // other side refuses at exactly the moment coins are at stake.
        self.emit = match (self.role, seq) {
            (Role::Bob, 0) => 2,
            (Role::Alice, 1) => 3,
            (Role::Alice, 3) => DONE,
            (Role::Bob, 4) => 5,
            (Role::Bob, 5) => DONE,
            _ => DONE,
        };
        self.last_packet = Some(packet.clone());

        // If this cannot be written down, the caller must not get it: a sent
        // packet we have no record of is exactly the state we cannot recover
        // from. Roll the advance back so a retry is possible.
        if let Err(e) = self.checkpoint() {
            self.emit = emit_before;
            self.last_packet = last_before;
            return Err(e);
        }
        Ok(packet)
    }

    /// Bob's TX_lock packet, which he can only make once he has funding.
    pub fn lock_packet(
        &mut self,
        prev: OutPoint,
        prev_value: Amount,
        change_spk: Option<ScriptBuf>,
    ) -> Result<Packet, SessionError> {
        let m = self.build_lock(prev, prev_value, change_spk)?;
        let body = serde_json::to_value(m).map_err(|_| SessionError::WrongBody)?;
        let emit_before = self.emit;
        let last_before = self.last_packet.clone();
        self.emit = 4;
        let packet = Packet::new(self.network, self.id, 2, self.amounts.clone(), body);
        self.last_packet = Some(packet.clone());
        if let Err(e) = self.checkpoint() {
            self.emit = emit_before;
            self.last_packet = last_before;
            self.lock = None;
            return Err(e);
        }
        Ok(packet)
    }

    /// Re-copy the last packet we produced. A restart after `emit` advanced
    /// cannot call `next_packet` again for that sequence number.
    pub fn last_packet(&self) -> Option<&Packet> {
        self.last_packet.as_ref()
    }

    /// Check a packet and apply it. Fails closed on every mismatch, and the
    /// error says which one — a user who pasted the wrong window's text
    /// should be told that, not handed "invalid".
    pub fn accept_packet(&mut self, p: &Packet) -> Result<Accepted, SessionError> {
        p.verify_open(self.id, self.network, self.expect, &self.amounts)?;
        let body = p.body.clone();
        let out = match p.seq {
            0 => {
                self.accept0(&from(body)?)?;
                Accepted::Opening
            }
            1 => {
                self.accept1(&from(body)?)?;
                Accepted::Reply
            }
            2 => {
                self.accept2(&from(body)?)?;
                Accepted::BitcoinLock
            }
            3 => {
                self.accept3(&from(body)?)?;
                Accepted::AbortSignatures
            }
            4 => {
                self.accept4(&from(body)?)?;
                Accepted::PunishSignatures
            }
            5 => {
                self.accept_redeem_enc(&from(body)?)?;
                Accepted::RedeemAdaptor
            }
            _ => return Err(SessionError::WrongBody),
        };
        // Applying a packet moves `expect` and stores the counterparty's
        // keys and signatures. Losing that is as bad as losing an outbound
        // packet: on restart we would ask for a message they already sent.
        self.checkpoint()?;
        Ok(out)
    }
}

fn from<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, SessionError> {
    serde_json::from_value(v).map_err(|_| SessionError::WrongBody)
}

// ---------------------------------------------------------- persist (N9) ---

/// On-disk form of a handshake. Lives in `{id}.secret` (mode 0600), not in
/// the swap JSON: the JSON is the state machine and is not a key file.
#[derive(Serialize, Deserialize)]
struct StoredSession {
    id: Uuid,
    role: Role,
    network: NetworkId,
    amounts: Amounts,
    depths: Depths,
    expect: u8,
    emit: u8,
    /// Little-endian 32-byte hex. Rebuilt with [`SwapShare::from_bytes`].
    share_secret_hex: String,
    /// Big-endian 32-byte hex, secp256kfun.
    btc_sk_hex: String,
    peer_btc: Option<Vec<u8>>,
    peer_offer: Option<SwapOffer>,
    scan_secret: Option<[u8; 32]>,
    refund_spk: Option<Vec<u8>>,
    redeem_spk: Option<Vec<u8>>,
    punish_spk: Option<Vec<u8>>,
    lock: Option<TxLock>,
    peer_cancel_sig: Option<Signature>,
    peer_refund_encsig: Option<EncryptedSignature>,
    peer_punish_sig: Option<Signature>,
    redeem_encsig: Option<EncryptedSignature>,
    last_packet: Option<Packet>,
}

impl Session {
    /// Persist secrets and handshake progress. Call after every packet.
    pub fn save(&self, datadir: &Path) -> Result<(), PersistError> {
        let stored = StoredSession {
            id: self.id,
            role: self.role,
            network: self.network,
            amounts: self.amounts.clone(),
            depths: self.depths,
            expect: self.expect,
            emit: self.emit,
            share_secret_hex: hex::encode(self.secrets.share.secret().to_bytes()),
            btc_sk_hex: hex::encode(self.secrets.btc_sk.to_bytes()),
            peer_btc: self.peer_btc.as_ref().map(|p| p.to_bytes().to_vec()),
            peer_offer: self.peer_offer.clone(),
            scan_secret: self.scan_secret.map(|s| s.to_bytes()),
            refund_spk: self.refund_spk.as_ref().map(|s| s.to_bytes()),
            redeem_spk: self.redeem_spk.as_ref().map(|s| s.to_bytes()),
            punish_spk: self.punish_spk.as_ref().map(|s| s.to_bytes()),
            lock: self.lock.clone(),
            peer_cancel_sig: self.peer_cancel_sig.clone(),
            peer_refund_encsig: self.peer_refund_encsig.clone(),
            peer_punish_sig: self.peer_punish_sig.clone(),
            redeem_encsig: self.redeem_encsig.clone(),
            last_packet: self.last_packet.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&stored)?;
        persist::write_secret_file(&persist::secret_path(datadir, self.id), &bytes)
    }

    /// Continue exactly where `expect` / `emit` stood. Refuses a
    /// world-readable file and a share that is not below `2^252`.
    pub fn load(datadir: &Path, id: Uuid) -> Result<Self, SessionError> {
        let path = persist::secret_path(datadir, id);
        if !path.exists() {
            return Err(SessionError::NoSession);
        }
        let bytes = persist::read_secret_file(&path).map_err(|e| match e {
            PersistError::WorldReadable => SessionError::WorldReadable,
            PersistError::Io(_) | PersistError::Json(_) => SessionError::Corrupt,
        })?;
        let stored: StoredSession =
            serde_json::from_slice(&bytes).map_err(|_| SessionError::Corrupt)?;
        if stored.id != id {
            return Err(SessionError::Corrupt);
        }

        let share_bytes: [u8; 32] = hex_32(&stored.share_secret_hex)?;
        let share = SwapShare::from_bytes(share_bytes).ok_or(SessionError::BadShare)?;
        let btc_sk = secp_sk(&stored.btc_sk_hex)?;
        let scan_secret = match stored.scan_secret {
            Some(b) => Some(
                Option::<Scalar>::from(Scalar::from_canonical_bytes(b))
                    .ok_or(SessionError::BadKey)?,
            ),
            None => None,
        };

        Ok(Self {
            id: stored.id,
            role: stored.role,
            network: stored.network,
            amounts: stored.amounts,
            depths: stored.depths,
            // These two are the whole point of load(). A default of 0 would
            // make Bob re-offer after a crash and Alice accept a replay.
            expect: stored.expect,
            emit: stored.emit,
            secrets: Secrets { share, btc_sk },
            peer_btc: match stored.peer_btc {
                Some(b) => Some(point(&b)?),
                None => None,
            },
            peer_offer: stored.peer_offer,
            scan_secret,
            refund_spk: match stored.refund_spk {
                Some(b) => Some(spk(&b)?),
                None => None,
            },
            redeem_spk: match stored.redeem_spk {
                Some(b) => Some(spk(&b)?),
                None => None,
            },
            punish_spk: match stored.punish_spk {
                Some(b) => Some(spk(&b)?),
                None => None,
            },
            lock: stored.lock,
            peer_cancel_sig: stored.peer_cancel_sig,
            peer_refund_encsig: stored.peer_refund_encsig,
            peer_punish_sig: stored.peer_punish_sig,
            redeem_encsig: stored.redeem_encsig,
            last_packet: stored.last_packet,
            // A session read from disk keeps writing to that disk. Anything
            // else would make the crash safety depend on the caller
            // remembering to switch it back on after every restart.
            datadir: Some(datadir.to_path_buf()),
        })
    }

    // ------------------------------------------------------ Bitcoin lock (N10) ---

    /// Unsigned TX_lock as hex, for the user's own Bitcoin wallet to sign.
    /// This wallet does not hold Bitcoin keys.
    pub fn unsigned_lock_hex(&self) -> Result<String, SessionError> {
        let lock = self.lock_ref()?;
        Ok(hex::encode(serialize(&lock.tx)))
    }

    /// PSBT wrapping the same unsigned transaction. Sparrow / Bitcoin Core
    /// `walletprocesspsbt` can sign it; the txid does not change for a
    /// segwit spend once the witness is attached.
    pub fn unsigned_lock_psbt(&self) -> Result<String, SessionError> {
        let lock = self.lock_ref()?;
        let psbt = bitcoin::psbt::Psbt::from_unsigned_tx(lock.tx.clone())
            .map_err(|e| SessionError::Tx(e.to_string()))?;
        Ok(psbt.to_string())
    }

    pub fn lock_txid(&self) -> Result<String, SessionError> {
        Ok(self.lock_ref()?.txid().to_string())
    }

    /// The confirmed (or signed-and-broadcast) transaction must be the one
    /// this session built. Believing a pasted txid without this check is
    /// how Alice signs children of an output she cannot claim.
    pub fn verify_confirmed_lock(&self, confirmed: &Transaction) -> Result<(), SessionError> {
        let built = self.lock_ref()?;
        if confirmed.compute_txid() != built.txid() {
            return Err(SessionError::LockMismatch);
        }
        let out = confirmed
            .output
            .get(built.vout as usize)
            .ok_or(SessionError::LockMismatch)?;
        let want_spk = bitcoin_tx::p2wsh(&built.script);
        if out.script_pubkey != want_spk || out.value != built.value {
            return Err(SessionError::LockMismatch);
        }
        Ok(())
    }

    pub fn verify_confirmed_lock_hex(&self, raw_hex: &str) -> Result<(), SessionError> {
        let bytes = hex::decode(raw_hex.trim()).map_err(|_| SessionError::LockMismatch)?;
        let tx: Transaction = deserialize(&bytes).map_err(|_| SessionError::LockMismatch)?;
        self.verify_confirmed_lock(&tx)
    }
}

fn hex_32(s: &str) -> Result<[u8; 32], SessionError> {
    let v = hex::decode(s).map_err(|_| SessionError::Corrupt)?;
    <[u8; 32]>::try_from(v).map_err(|_| SessionError::Corrupt)
}

fn secp_sk(hex_str: &str) -> Result<SecpScalar, SessionError> {
    let b = hex_32(hex_str)?;
    SecpScalar::from_bytes(b).ok_or(SessionError::BadKey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;
    use bitcoin::{Address, Network, PubkeyHash, ScriptHash, WPubkeyHash, WScriptHash};

    fn p2wpkh(tag: u8) -> ScriptBuf {
        let mut v = vec![0x00, 0x14];
        v.extend_from_slice(&[tag; 20]);
        ScriptBuf::from_bytes(v)
    }

    #[test]
    fn standard_script_types_fit_the_length_bound() {
        let p2pkh = ScriptBuf::new_p2pkh(&PubkeyHash::from_byte_array([0x11; 20]));
        let p2sh = ScriptBuf::new_p2sh(&ScriptHash::from_byte_array([0x22; 20]));
        let p2wpkh = ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0x33; 20]));
        let p2wsh = ScriptBuf::new_p2wsh(&WScriptHash::from_byte_array([0x44; 32]));
        // P2TR: OP_1 + 32-byte x-only key. 34 bytes.
        let mut tap = vec![0x51, 0x20];
        tap.extend_from_slice(&[0x55; 32]);
        let p2tr = ScriptBuf::from_bytes(tap);

        for (name, s, want_len) in [
            ("p2pkh", p2pkh, 25usize),
            ("p2sh", p2sh, 23),
            ("p2wpkh", p2wpkh, 22),
            ("p2wsh", p2wsh, 34),
            ("p2tr", p2tr, 34),
        ] {
            assert_eq!(s.as_bytes().len(), want_len, "{name} length");
            assert!(
                (1..=42).contains(&want_len),
                "{name} is {want_len} bytes, outside the 1..=42 bound"
            );
            assert!(spk(s.as_bytes()).is_ok(), "{name} must be accepted");
        }
        assert_eq!(spk(&[]).unwrap_err(), SessionError::BadScript);
        assert_eq!(spk(&[0u8; 43]).unwrap_err(), SessionError::BadScript);

        // Same scripts as produced from real address strings (BIP173 / BIP350).
        use std::str::FromStr;
        let _ = Network::Bitcoin;
        for a in [
            "1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH",         // p2pkh
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", // p2wpkh
            "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3", // p2wsh
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0", // p2tr
        ] {
            let addr = Address::from_str(a)
                .unwrap_or_else(|e| panic!("not an address: {a}: {e}"))
                .assume_checked();
            let bytes = addr.script_pubkey();
            assert!(
                spk(bytes.as_bytes()).is_ok(),
                "{a} script is {} bytes",
                bytes.as_bytes().len()
            );
        }
        let p2sh_addr =
            Address::p2sh_from_hash(ScriptHash::from_byte_array([0x22; 20]), Network::Bitcoin);
        assert!(spk(p2sh_addr.script_pubkey().as_bytes()).is_ok());
    }

    #[test]
    fn two_of_two_sorts_but_adaptor_checks_do_not() {
        use crate::adaptor;
        let mut rng = OsRng;
        let sk_a = adaptor::random_bitcoin_sk(&mut rng);
        let sk_b = adaptor::random_bitcoin_sk(&mut rng);
        let a = adaptor::verification_key(&sk_a);
        let b = adaptor::verification_key(&sk_b);
        let script_ab = bitcoin_tx::two_of_two(&a, &b).unwrap();
        let script_ba = bitcoin_tx::two_of_two(&b, &a).unwrap();
        assert_eq!(
            script_ab, script_ba,
            "the lock script is canonical — order at the call site must not matter"
        );

        let msg = [9u8; 32];
        let sig = adaptor::sign(&sk_a, &msg);
        assert!(adaptor::verify_sig(&a, &msg, &sig));
        assert!(
            !adaptor::verify_sig(&b, &msg, &sig),
            "verifying Alice's signature under Bob's key must fail — \
             keys() returning (bob, alice) on Alice's side would accept a forgery"
        );
    }

    #[test]
    fn last_packet_survives_after_emit_advances() {
        let bob = Session::open_as_bob(
            NetworkId::Testnet,
            Amounts {
                night_darks: 1,
                btc_sats: 50_000,
                btc_fee_sats: 500,
            },
            Depths::testdrive(),
            p2wpkh(0xbb),
        );
        let mut bob = bob;
        let p0 = bob.next_packet().unwrap();
        assert_eq!(bob.emit, 2);
        assert_eq!(bob.last_packet().unwrap().seq, p0.seq);
        assert_eq!(bob.next_packet().unwrap_err(), SessionError::WrongRole);
        assert_eq!(bob.last_packet().unwrap().encode(), p0.encode());
    }
}
