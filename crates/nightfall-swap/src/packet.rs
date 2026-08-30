//! Copy-paste packets. No server. Version, network, swap id, sequence,
//! checksum. Import fails closed with a distinct error for each mistake.

use crate::messages::Amounts;
use nightfall_types::NetworkId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const PACKET_VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PacketError {
    #[error("unsupported packet version {0}")]
    BadVersion(u32),
    #[error("checksum mismatch")]
    BadChecksum,
    #[error("swap id does not match")]
    WrongId,
    #[error("network {got:?} does not match this wallet ({want:?})")]
    WrongNetwork { got: NetworkId, want: NetworkId },
    #[error("message {got} is not the next expected ({want})")]
    WrongSeq { got: u8, want: u8 },
    #[error("amounts changed since the swap was agreed")]
    AmountChanged,
    #[error("malformed packet")]
    Malformed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Packet {
    pub version: u32,
    pub network: NetworkId,
    pub swap_id: Uuid,
    pub seq: u8,
    pub amounts: Amounts,
    pub body: serde_json::Value,
    pub checksum: String,
}

impl Amounts {
    fn fingerprint(&self) -> (u64, u64, u64) {
        (self.night_darks, self.btc_sats, self.btc_fee_sats)
    }
}

fn checksum(p: &Packet) -> String {
    let mut h = Sha256::new();
    h.update(p.version.to_le_bytes());
    h.update(p.network.as_str().as_bytes());
    h.update(p.swap_id.as_bytes());
    h.update([p.seq]);
    h.update(p.amounts.night_darks.to_le_bytes());
    h.update(p.amounts.btc_sats.to_le_bytes());
    h.update(p.amounts.btc_fee_sats.to_le_bytes());
    h.update(p.body.to_string().as_bytes());
    hex::encode(&h.finalize()[..4])
}

impl Packet {
    pub fn new(
        network: NetworkId,
        swap_id: Uuid,
        seq: u8,
        amounts: Amounts,
        body: serde_json::Value,
    ) -> Self {
        let mut p = Self {
            version: PACKET_VERSION,
            network,
            swap_id,
            seq,
            amounts,
            body,
            checksum: String::new(),
        };
        p.checksum = checksum(&p);
        p
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("packet")
    }

    pub fn decode(s: &str) -> Result<Self, PacketError> {
        serde_json::from_str(s).map_err(|_| PacketError::Malformed)
    }

    pub fn verify_open(
        &self,
        want_id: Uuid,
        want_net: NetworkId,
        want_seq: u8,
        want_amounts: &Amounts,
    ) -> Result<(), PacketError> {
        if self.version != PACKET_VERSION {
            return Err(PacketError::BadVersion(self.version));
        }
        if checksum(self) != self.checksum {
            return Err(PacketError::BadChecksum);
        }
        if self.swap_id != want_id {
            return Err(PacketError::WrongId);
        }
        if self.network != want_net {
            return Err(PacketError::WrongNetwork {
                got: self.network,
                want: want_net,
            });
        }
        if self.seq != want_seq {
            return Err(PacketError::WrongSeq {
                got: self.seq,
                want: want_seq,
            });
        }
        if self.amounts.fingerprint() != want_amounts.fingerprint() {
            return Err(PacketError::AmountChanged);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Packet, Uuid, Amounts) {
        let id = Uuid::nil();
        let amounts = Amounts {
            night_darks: 5,
            btc_sats: 80_000,
            btc_fee_sats: 400,
        };
        let p = Packet::new(
            NetworkId::Devnet,
            id,
            0,
            amounts.clone(),
            serde_json::json!({"hello": 1}),
        );
        (p, id, amounts)
    }

    #[test]
    fn honest_packet_opens() {
        let (p, id, a) = sample();
        p.verify_open(id, NetworkId::Devnet, 0, &a).unwrap();
    }

    #[test]
    fn foreign_id_is_refused() {
        let (p, _, a) = sample();
        assert_eq!(
            p.verify_open(Uuid::from_u128(1), NetworkId::Devnet, 0, &a),
            Err(PacketError::WrongId)
        );
    }

    #[test]
    fn wrong_network_is_refused() {
        let (p, id, a) = sample();
        assert!(matches!(
            p.verify_open(id, NetworkId::Mainnet, 0, &a),
            Err(PacketError::WrongNetwork { .. })
        ));
    }

    #[test]
    fn replayed_seq_is_refused() {
        let (p, id, a) = sample();
        assert_eq!(
            p.verify_open(id, NetworkId::Devnet, 1, &a),
            Err(PacketError::WrongSeq { got: 0, want: 1 })
        );
    }

    #[test]
    fn changed_amount_is_refused() {
        let (p, id, mut a) = sample();
        a.btc_sats = 1;
        assert_eq!(
            p.verify_open(id, NetworkId::Devnet, 0, &a),
            Err(PacketError::AmountChanged)
        );
    }

    #[test]
    fn old_version_is_refused() {
        let (mut p, id, a) = sample();
        p.version = 0;
        p.checksum = checksum(&p);
        assert_eq!(
            p.verify_open(id, NetworkId::Devnet, 0, &a),
            Err(PacketError::BadVersion(0))
        );
    }

    #[test]
    fn flipped_byte_fails_the_checksum() {
        let (mut p, id, a) = sample();
        p.body = serde_json::json!({"hello": 2});
        assert_eq!(
            p.verify_open(id, NetworkId::Devnet, 0, &a),
            Err(PacketError::BadChecksum)
        );
    }
}
