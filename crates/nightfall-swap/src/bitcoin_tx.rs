//! Bitcoin transaction tree: lock, redeem, cancel, refund, punish.
//!
//! P2WSH 2-of-2, ECDSA, no Taproot. Copied from the xmr-btc-swap shape:
//! lock is a 2-of-2; cancel spends it with nSequence = H₁ (BIP68, the
//! pre-signed sequence *is* the timelock); refund and punish spend cancel.

use crate::adaptor;
use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::ecdsa::Signature as SecpSig;
use bitcoin::secp256k1::PublicKey as SecpPk;
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, WScriptHash, Witness,
};
use ecdsa_fun::adaptor::EncryptedSignature;
use ecdsa_fun::fun::Point;
use ecdsa_fun::fun::Scalar as SecpScalar;
use ecdsa_fun::Signature;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TxError {
    #[error("keys must differ")]
    SameKeys,
    #[error("bad secp public key")]
    BadKey,
    #[error("value too small for fee")]
    Dust,
    #[error("sighash failed")]
    Sighash,
    /// BIP68 relative height is a u16. Casting a larger H₁ to `u16` wraps
    /// to a smaller sequence — TX_cancel would be immediately minable.
    #[error("relative timelock {0} exceeds BIP68 height (65535)")]
    TimelockTooLarge(u32),
}

/// Canonical 2-of-2: `A CHECKSIGVERIFY B CHECKSIG`, keys sorted.
pub fn two_of_two(a: &Point, b: &Point) -> Result<ScriptBuf, TxError> {
    let pa = point_to_secp(a)?;
    let pb = point_to_secp(b)?;
    if pa.serialize() == pb.serialize() {
        return Err(TxError::SameKeys);
    }
    let (first, second) = if pa.serialize() < pb.serialize() {
        (pa, pb)
    } else {
        (pb, pa)
    };
    Ok(bitcoin::blockdata::script::Builder::new()
        .push_slice(first.serialize())
        .push_opcode(bitcoin::opcodes::all::OP_CHECKSIGVERIFY)
        .push_slice(second.serialize())
        .push_opcode(bitcoin::opcodes::all::OP_CHECKSIG)
        .into_script())
}

pub fn p2wsh(witness_script: &ScriptBuf) -> ScriptBuf {
    ScriptBuf::new_p2wsh(&WScriptHash::hash(witness_script.as_bytes()))
}

/// BIP68 height-based relative lock. Refuses values that would wrap in `u16`.
pub fn csv_height(blocks: u32) -> Result<Sequence, TxError> {
    u16::try_from(blocks)
        .map(Sequence::from_height)
        .map_err(|_| TxError::TimelockTooLarge(blocks))
}

fn point_to_secp(p: &Point) -> Result<SecpPk, TxError> {
    SecpPk::from_slice(&p.to_bytes()).map_err(|_| TxError::BadKey)
}

fn sig_der(sig: &Signature) -> Result<Vec<u8>, TxError> {
    let compact = sig.to_bytes();
    let secp = SecpSig::from_compact(&compact).map_err(|_| TxError::BadKey)?;
    let mut der = secp.serialize_der().to_vec();
    der.push(EcdsaSighashType::All.to_u32() as u8);
    Ok(der)
}

/// Which witness slot a given key occupies after sorting.
fn sig_order(a: &Point, b: &Point, signer: &Point) -> Result<bool, TxError> {
    let pa = point_to_secp(a)?.serialize();
    let pb = point_to_secp(b)?.serialize();
    let ps = point_to_secp(signer)?.serialize();
    // script is CHECKSIGVERIFY (first) then CHECKSIG (second).
    // witness is [sig_second, sig_first] (stack: first pushed is second-checked).
    // We return true if `signer` is the first key (CHECKSIGVERIFY).
    if pa < pb {
        Ok(ps == pa)
    } else {
        Ok(ps == pb)
    }
}

fn attach_two_sigs(
    tx: &mut Transaction,
    script: &ScriptBuf,
    a: &Point,
    sig_a: &Signature,
    b: &Point,
    sig_b: &Signature,
) -> Result<(), TxError> {
    let da = sig_der(sig_a)?;
    let db = sig_der(sig_b)?;
    // Witness bottom-to-top: sig for CHECKSIG (second key), then sig for CHECKSIGVERIFY (first).
    let first_is_a = sig_order(a, b, a)?;
    let (sig_first, sig_second) = if first_is_a { (da, db) } else { (db, da) };
    tx.input[0].witness = Witness::from_slice(&[sig_second, sig_first, script.as_bytes().to_vec()]);
    Ok(())
}

pub fn p2wsh_sighash(
    tx: &Transaction,
    script: &ScriptBuf,
    value: Amount,
) -> Result<[u8; 32], TxError> {
    let mut cache = SighashCache::new(tx);
    let hash = cache
        .p2wsh_signature_hash(0, script, value, EcdsaSighashType::All)
        .map_err(|_| TxError::Sighash)?;
    Ok(*hash.as_byte_array())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxLock {
    pub tx: Transaction,
    pub script: ScriptBuf,
    pub value: Amount,
    pub vout: u32,
}

impl TxLock {
    /// Spend `prev` into a 2-of-2 of `(a,b)`. Change is the caller's problem:
    /// pass a prevout that already equals `value + fee`.
    pub fn from_prevout(
        prev: OutPoint,
        prev_value: Amount,
        a: &Point,
        b: &Point,
        value: Amount,
        fee: Amount,
        change_script: Option<ScriptBuf>,
    ) -> Result<Self, TxError> {
        let script = two_of_two(a, b)?;
        let lock_spk = p2wsh(&script);
        if prev_value < value + fee {
            return Err(TxError::Dust);
        }
        let change = prev_value - value - fee;
        let mut output = vec![TxOut {
            value,
            script_pubkey: lock_spk,
        }];
        if change > Amount::from_sat(546) {
            if let Some(spk) = change_script {
                output.push(TxOut {
                    value: change,
                    script_pubkey: spk,
                });
            }
        }
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: prev,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output,
        };
        Ok(Self {
            tx,
            script,
            value,
            vout: 0,
        })
    }

    pub fn txid(&self) -> Txid {
        self.tx.compute_txid()
    }

    pub fn outpoint(&self) -> OutPoint {
        OutPoint {
            txid: self.txid(),
            vout: self.vout,
        }
    }
}

fn spend_lock(
    lock: &TxLock,
    dest: ScriptBuf,
    fee: Amount,
    sequence: Sequence,
) -> Result<Transaction, TxError> {
    if lock.value <= fee {
        return Err(TxError::Dust);
    }
    Ok(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: lock.outpoint(),
            script_sig: ScriptBuf::new(),
            sequence,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: lock.value - fee,
            script_pubkey: dest,
        }],
    })
}

#[derive(Clone, Debug)]
pub struct TxRedeem {
    pub tx: Transaction,
    pub sighash: [u8; 32],
}

impl TxRedeem {
    pub fn new(lock: &TxLock, alice_dest: ScriptBuf, fee: Amount) -> Result<Self, TxError> {
        let tx = spend_lock(lock, alice_dest, fee, Sequence::MAX)?;
        let sighash = p2wsh_sighash(&tx, &lock.script, lock.value)?;
        Ok(Self { tx, sighash })
    }

    pub fn complete(
        mut self,
        a: &Point,
        sig_a: Signature,
        b: &Point,
        sig_b: Signature,
        lock_script: &ScriptBuf,
    ) -> Result<Transaction, TxError> {
        attach_two_sigs(&mut self.tx, lock_script, a, &sig_a, b, &sig_b)?;
        Ok(self.tx)
    }
}

#[derive(Clone, Debug)]
pub struct TxCancel {
    pub tx: Transaction,
    pub sighash: [u8; 32],
    pub script: ScriptBuf,
    pub value: Amount,
}

impl TxCancel {
    pub fn new(
        lock: &TxLock,
        a: &Point,
        b: &Point,
        cancel_csv: u32,
        fee: Amount,
    ) -> Result<Self, TxError> {
        let script = two_of_two(a, b)?;
        let dest = p2wsh(&script);
        let tx = spend_lock(lock, dest, fee, csv_height(cancel_csv)?)?;
        let sighash = p2wsh_sighash(&tx, &lock.script, lock.value)?;
        let value = tx.output[0].value;
        Ok(Self {
            tx,
            sighash,
            script,
            value,
        })
    }

    pub fn complete(
        mut self,
        a: &Point,
        sig_a: Signature,
        b: &Point,
        sig_b: Signature,
        lock_script: &ScriptBuf,
    ) -> Result<Transaction, TxError> {
        attach_two_sigs(&mut self.tx, lock_script, a, &sig_a, b, &sig_b)?;
        Ok(self.tx)
    }

    pub fn outpoint(&self) -> OutPoint {
        OutPoint {
            txid: self.tx.compute_txid(),
            vout: 0,
        }
    }
}

fn spend_cancel(
    cancel: &TxCancel,
    dest: ScriptBuf,
    fee: Amount,
    sequence: Sequence,
) -> Result<Transaction, TxError> {
    if cancel.value <= fee {
        return Err(TxError::Dust);
    }
    Ok(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: cancel.outpoint(),
            script_sig: ScriptBuf::new(),
            sequence,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: cancel.value - fee,
            script_pubkey: dest,
        }],
    })
}

#[derive(Clone, Debug)]
pub struct TxRefund {
    pub tx: Transaction,
    pub sighash: [u8; 32],
}

impl TxRefund {
    pub fn new(cancel: &TxCancel, bob_dest: ScriptBuf, fee: Amount) -> Result<Self, TxError> {
        let tx = spend_cancel(cancel, bob_dest, fee, Sequence::MAX)?;
        let sighash = p2wsh_sighash(&tx, &cancel.script, cancel.value)?;
        Ok(Self { tx, sighash })
    }

    pub fn complete(
        mut self,
        a: &Point,
        sig_a: Signature,
        b: &Point,
        sig_b: Signature,
        cancel_script: &ScriptBuf,
    ) -> Result<Transaction, TxError> {
        attach_two_sigs(&mut self.tx, cancel_script, a, &sig_a, b, &sig_b)?;
        Ok(self.tx)
    }
}

#[derive(Clone, Debug)]
pub struct TxPunish {
    pub tx: Transaction,
    pub sighash: [u8; 32],
}

impl TxPunish {
    pub fn new(
        cancel: &TxCancel,
        alice_dest: ScriptBuf,
        punish_csv: u32,
        fee: Amount,
    ) -> Result<Self, TxError> {
        let tx = spend_cancel(cancel, alice_dest, fee, csv_height(punish_csv)?)?;
        let sighash = p2wsh_sighash(&tx, &cancel.script, cancel.value)?;
        Ok(Self { tx, sighash })
    }

    pub fn complete(
        mut self,
        a: &Point,
        sig_a: Signature,
        b: &Point,
        sig_b: Signature,
        cancel_script: &ScriptBuf,
    ) -> Result<Transaction, TxError> {
        attach_two_sigs(&mut self.tx, cancel_script, a, &sig_a, b, &sig_b)?;
        Ok(self.tx)
    }
}

/// Pull one party's signature back out of a broadcast witness.
///
/// This is how the secret actually travels. Bob does not receive a
/// `Signature` from Alice — he watches the chain, sees her redeem, and reads
/// her signature off the witness stack. Without this the recovery path only
/// works between two objects in one process, which is not where the swap
/// happens.
///
/// Witness layout, from `attach_two_sigs`:
/// `[sig_for_second_key, sig_for_first_key, witness_script]`, where "first"
/// is the CHECKSIGVERIFY key after canonical sorting.
pub fn signature_from_witness(
    tx: &Transaction,
    a: &Point,
    b: &Point,
    signer: &Point,
) -> Option<Signature> {
    let w = &tx.input.first()?.witness;
    if w.len() != 3 {
        return None;
    }
    let signer_is_first = sig_order(a, b, signer).ok()?;
    // Index 0 is the second-checked key, index 1 the first-checked one.
    let der = w.nth(if signer_is_first { 1 } else { 0 })?;
    // Strip the trailing sighash byte that `sig_der` appended.
    let (_sighash, body) = der.split_last()?;
    let secp = SecpSig::from_der(body).ok()?;
    Signature::from_bytes(secp.serialize_compact())
}

/// Bob's adaptor on TX_redeem, encrypted under Alice's T_a.
pub fn bob_encsign_redeem(
    bob_sk: &SecpScalar,
    t_a: &Point,
    redeem: &TxRedeem,
) -> EncryptedSignature {
    adaptor::encsign(bob_sk, t_a, &redeem.sighash)
}

/// Alice's adaptor on TX_refund, encrypted under Bob's T_b.
pub fn alice_encsign_refund(
    alice_sk: &SecpScalar,
    t_b: &Point,
    refund: &TxRefund,
) -> EncryptedSignature {
    adaptor::encsign(alice_sk, t_b, &refund.sighash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor;

    #[test]
    fn a_timelock_above_u16_is_an_error_not_a_wrap() {
        assert!(matches!(
            csv_height(65_536),
            Err(TxError::TimelockTooLarge(65_536))
        ));
        assert!(csv_height(65_535).is_ok());
        assert!(csv_height(144).is_ok());
        // The old `as u16` would have turned 65536 into 0: immediately minable.
        assert_ne!(
            csv_height(144).unwrap().to_consensus_u32(),
            0,
            "a real H₁ must not collapse to an unlocked sequence"
        );
    }

    #[test]
    fn changing_the_fee_changes_the_sighash() {
        let mut rng = rand::rngs::OsRng;
        let a = adaptor::verification_key(&adaptor::random_bitcoin_sk(&mut rng));
        let b = adaptor::verification_key(&adaptor::random_bitcoin_sk(&mut rng));
        let prev = OutPoint {
            txid: Txid::from_byte_array([9u8; 32]),
            vout: 0,
        };
        let lock = TxLock::from_prevout(
            prev,
            Amount::from_sat(100_000),
            &a,
            &b,
            Amount::from_sat(80_000),
            Amount::from_sat(1_000),
            None,
        )
        .unwrap();
        let low = TxCancel::new(&lock, &a, &b, 4, Amount::from_sat(400)).unwrap();
        let high = TxCancel::new(&lock, &a, &b, 4, Amount::from_sat(4_000)).unwrap();
        assert_ne!(
            low.sighash, high.sighash,
            "fee is inside the pre-signed message; it cannot be raised later"
        );
    }
}

#[cfg(test)]
mod witness_tests {
    use super::*;
    use crate::adaptor;
    use rand::rngs::OsRng;

    /// The signature that goes into the witness must be the signature that
    /// comes back out, for *both* slots.
    ///
    /// This is the seam the whole swap rests on: Bob does not receive
    /// Alice's signature, he reads it off the chain. If the slot order or
    /// the DER round trip is wrong, recovery silently fails and the NIGHT
    /// side never completes — with the Bitcoin already gone.
    #[test]
    fn a_signature_survives_the_witness_round_trip() {
        let sk_a = adaptor::random_bitcoin_sk(&mut OsRng);
        let sk_b = adaptor::random_bitcoin_sk(&mut OsRng);
        let a = adaptor::verification_key(&sk_a);
        let b = adaptor::verification_key(&sk_b);
        let script = two_of_two(&a, &b).unwrap();

        let msg = [7u8; 32];
        let sig_a = adaptor::sign(&sk_a, &msg);
        let sig_b = adaptor::sign(&sk_b, &msg);

        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![],
        };
        attach_two_sigs(&mut tx, &script, &a, &sig_a, &b, &sig_b).unwrap();

        let back_a = signature_from_witness(&tx, &a, &b, &a)
            .expect("A's signature must be readable from the witness");
        let back_b = signature_from_witness(&tx, &a, &b, &b)
            .expect("B's signature must be readable from the witness");

        assert_eq!(back_a, sig_a, "read A's slot, got something else");
        assert_eq!(back_b, sig_b, "read B's slot, got something else");
        assert_ne!(
            back_a, back_b,
            "the two slots must not be the same signature"
        );

        // And they still verify, which is what recovery will rely on.
        assert!(adaptor::verify_sig(&a, &msg, &back_a));
        assert!(adaptor::verify_sig(&b, &msg, &back_b));
    }

    /// The counterparty chooses the witness. Extra elements, a missing
    /// script, or a non-DER blob must not be read as a signature — we would
    /// then feed garbage into adaptor recovery.
    #[test]
    fn a_witness_that_is_not_our_layout_yields_nothing() {
        let mut rng = rand::rngs::OsRng;
        let sk_a = adaptor::random_bitcoin_sk(&mut rng);
        let sk_b = adaptor::random_bitcoin_sk(&mut rng);
        let a = adaptor::verification_key(&sk_a);
        let b = adaptor::verification_key(&sk_b);
        let script = two_of_two(&a, &b).unwrap();
        let sig_a = adaptor::sign(&sk_a, &[7u8; 32]);
        let sig_b = adaptor::sign(&sk_b, &[7u8; 32]);
        let mut tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![],
        };
        attach_two_sigs(&mut tx, &script, &a, &sig_a, &b, &sig_b).unwrap();
        assert_eq!(tx.input[0].witness.len(), 3);

        tx.input[0].witness.push([0u8; 8]);
        assert!(
            signature_from_witness(&tx, &a, &b, &a).is_none(),
            "a fourth stack item must not be parsed as a signature"
        );

        tx.input[0].witness = Witness::from_slice(&[vec![0u8; 8], vec![0u8; 8]]);
        assert!(signature_from_witness(&tx, &a, &b, &a).is_none());

        tx.input[0].witness = Witness::new();
        assert!(signature_from_witness(&tx, &a, &b, &a).is_none());
    }
}
