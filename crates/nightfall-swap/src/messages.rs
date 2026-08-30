//! Messages exchanged between Alice and Bob. Transport is out of scope —
//! these are the bytes, not the pipe. A mailbox that can withhold one of
//! them is an operator; do not add one.

use crate::timelock::Depths;
use ecdsa_fun::adaptor::EncryptedSignature;
use ecdsa_fun::Signature;
use nightfall_crypto::swap::SwapOffer;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Amounts {
    pub night_darks: u64,
    pub btc_sats: u64,
    pub btc_fee_sats: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message0 {
    pub swap_id: Uuid,
    /// Bob's Bitcoin 2-of-2 key (compressed 33 bytes).
    pub b_btc: Vec<u8>,
    pub offer_b: SwapOffer,
    pub refund_spk: Vec<u8>,
    pub amounts: Amounts,
    pub depths: Depths,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message1 {
    pub swap_id: Uuid,
    pub a_btc: Vec<u8>,
    pub offer_a: SwapOffer,
    pub redeem_spk: Vec<u8>,
    pub punish_spk: Vec<u8>,
    pub scan_secret: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message2 {
    pub swap_id: Uuid,
    /// TX_lock, bitcoin consensus encoding.
    pub tx_lock: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message3 {
    pub swap_id: Uuid,
    pub tx_cancel_sig: Signature,
    pub tx_refund_encsig: EncryptedSignature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message4 {
    pub swap_id: Uuid,
    pub tx_punish_sig: Signature,
    pub tx_cancel_sig: Signature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageRedeemEnc {
    pub swap_id: Uuid,
    pub tx_redeem_encsig: EncryptedSignature,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role {
    Alice,
    Bob,
}
