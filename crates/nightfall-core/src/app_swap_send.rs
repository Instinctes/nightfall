//! Putting a swap transaction on the wire.
//!
//! Everything up to here builds and checks. This is the part that cannot be
//! taken back, so it is deliberately narrow: the driver moves a swap forward,
//! and a human only ever *aborts* one or salvages one. There is no button
//! that redeems, because redeeming early is the mistake the H₁ margin exists
//! to prevent and a button invites exactly that.
//!
//! The node comes from a credentials file the user points at, mode 0600,
//! same rule as everywhere else.

use crate::app::App;
use nightfall_swap::bitcoin_rpc::{BitcoinRpc, RpcAuth};
use nightfall_swap::watch::{BroadcastResult, Broadcaster, WatchError};
use nightfall_swap::StoredSwap;
use std::path::PathBuf;

/// Which transaction a human asked us to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendWhat {
    Cancel,
    Refund,
    Punish,
}

impl SendWhat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cancel => "TX_cancel",
            Self::Refund => "TX_refund",
            Self::Punish => "TX_punish",
        }
    }

    fn kind(self) -> nightfall_swap::SendKind {
        match self {
            Self::Cancel => nightfall_swap::SendKind::Cancel,
            Self::Refund => nightfall_swap::SendKind::Refund,
            Self::Punish => nightfall_swap::SendKind::Punish,
        }
    }
}

impl App {
    /// Where the bitcoind credentials live. One file, mode 0600, three lines.
    pub fn btc_rpc_path(&self) -> PathBuf {
        self.datadir.join("bitcoin-rpc.conf")
    }

    /// Connect, or say precisely why not.
    ///
    /// Deliberately re-read each time rather than cached: a user who fixes
    /// their config should not have to restart the wallet to find out.
    pub fn btc_rpc(&self) -> Result<BitcoinRpc, String> {
        let path = self.btc_rpc_path();
        if !path.exists() {
            return Err(format!(
                "No Bitcoin node configured. Create {} with url=, user= and \
                 password= lines, readable only by you (chmod 600).",
                path.display()
            ));
        }
        let auth = RpcAuth::from_file(&path).map_err(|e| e.to_string())?;
        Ok(BitcoinRpc::new(auth))
    }

    /// Build, check against the node, then send.
    ///
    /// `testmempoolaccept` first, because a rejection is far cheaper to read
    /// than a broadcast that silently goes nowhere — and because a timelock
    /// that has not matured is the *expected* answer here, not a failure.
    pub fn send_swap_tx(&mut self, stored: &StoredSwap, what: SendWhat) -> Result<String, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let rpc = self.btc_rpc()?;

        let raw = {
            let session = self
                .swap_sessions
                .get(&id)
                .ok_or("No open session for this swap.")?;
            match what {
                SendWhat::Cancel => session.signed_cancel_hex(),
                SendWhat::Refund => session.signed_refund_hex(),
                SendWhat::Punish => session.signed_punish_hex(),
            }
            .map_err(|e| format!("{} could not be completed: {e}", what.label()))?
        };

        match rpc.test_accept(&raw) {
            Ok(nightfall_swap::watch::MempoolAccept::Reject { reason }) => {
                return Err(nightfall_swap::bitcoin_rpc::explain_broadcast_reject(
                    what.kind(),
                    &reason,
                ));
            }
            Err(e) => return Err(format!("Could not reach the Bitcoin node: {e}")),
            Ok(_) => {}
        }

        match rpc.broadcast(&raw) {
            Ok(BroadcastResult::Accepted { txid }) => {
                Ok(format!("{} broadcast: {txid}", what.label()))
            }
            Ok(BroadcastResult::AlreadyKnown { .. }) => Ok(format!(
                "{} was already on the network. Nothing more to do.",
                what.label()
            )),
            Err(WatchError::NeedsTxIndex) => Err(
                "This node cannot see confirmed transactions. Restart bitcoind \
                 with -txindex=1."
                    .into(),
            ),
            Err(e) => Err(format!("The node refused it: {e}")),
        }
    }
}
