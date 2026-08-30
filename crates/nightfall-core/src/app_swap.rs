//! What the swap view asks the app to do.
//!
//! The view draws; `nightfall_swap::ui` decides; this moves things. Anything
//! that touches the wallet, the disk or a live session is here.
//!
//! Handshake secrets live in `{datadir}/swaps/{id}.secret` (mode 0600) and
//! are reloaded after a restart. A crash after the Bitcoin lock is still
//! recoverable; a crash with a world-readable secret file is refused.

use crate::app::App;
use nightfall_swap::messages::Amounts;
use nightfall_swap::packet::Packet;
use nightfall_swap::session::Session;
use nightfall_swap::state::Role;
use nightfall_swap::timelock::Depths;
use nightfall_swap::{StoredSwap, SwapState};
use nightfall_types::NetworkId;
use std::str::FromStr;

impl App {
    /// Depths matched to the network. Testdrive numbers never reach a user
    /// on testnet; they do on devnet, because those blocks are instant and
    /// a 12-block CSV is seconds, not hours.
    pub fn swap_depths(&self) -> Depths {
        match self.network {
            NetworkId::Mainnet => Depths::mainnet(),
            NetworkId::Testnet => Depths::testnet(),
            NetworkId::Devnet => Depths::devnet(),
        }
    }

    /// Balance minus anything already promised to a swap.
    ///
    /// The start form must not offer coins another swap has reserved, or the
    /// same output gets promised to two counterparties.
    pub fn unreserved_darks(&self) -> u64 {
        let tip = self.tip_height();
        let maturity = self.maturity();
        self.wallet
            .lock()
            .map(|w| w.balances(tip, maturity).available)
            .unwrap_or(0)
    }

    /// Create the swap and its session. Nothing is broadcast and nothing is
    /// locked; this only reserves coins so they cannot be spent twice.
    pub fn create_swap(&mut self, role: Role, amounts: Amounts) -> Result<String, String> {
        if !nightfall_swap::ui::availability(self.network).is_enabled() {
            return Err("Swaps are disabled on this network.".into());
        }

        let depths = self.swap_depths();
        let mut session = match role {
            Role::Bob => Session::open_as_bob(
                self.network,
                amounts.clone(),
                depths,
                self.swap_script("refund")?,
            ),
            Role::Alice => {
                return Err(
                    "Alice joins an existing offer. Paste the packet the other side sent you."
                        .into(),
                )
            }
        };

        let id = session.id.to_string();
        // From here the session writes itself down before handing out any
        // packet. The explicit save below is the first one; after that the
        // discipline is structural, not the caller's memory.
        session.persist_to(&self.datadir);
        session.save(&self.datadir).map_err(|e| e.to_string())?;
        let stored = StoredSwap::new(SwapState::new(role), amounts.night_darks, amounts.btc_sats);
        nightfall_swap::persist::save(&self.datadir, &stored).map_err(|e| e.to_string())?;
        self.swap_sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Turn one of the user's typed Bitcoin addresses into a script.
    ///
    /// This wallet holds NIGHT, not Bitcoin, so the addresses come from the
    /// user's own Bitcoin wallet. Refusing an unparseable one matters more
    /// than usual: a swap pays these scripts without asking again, and an
    /// address nobody holds the key to is a burn.
    fn swap_script(&self, which: &str) -> Result<bitcoin::ScriptBuf, String> {
        let (text, label) = match which {
            "refund" => (&self.swap_btc_refund, "refund"),
            "redeem" => (&self.swap_btc_redeem, "redeem"),
            _ => (&self.swap_btc_punish, "punish"),
        };
        let text = text.trim();
        if text.is_empty() {
            return Err(format!(
                "Enter the Bitcoin address your {label} should pay to."
            ));
        }
        let net = match self.network {
            NetworkId::Mainnet => bitcoin::Network::Bitcoin,
            NetworkId::Testnet => bitcoin::Network::Testnet,
            NetworkId::Devnet => bitcoin::Network::Regtest,
        };
        let addr = bitcoin::Address::from_str(text)
            .map_err(|e| format!("The {label} address is not a Bitcoin address: {e}"))?
            .require_network(net)
            .map_err(|_| format!("The {label} address belongs to a different Bitcoin network."))?;
        Ok(addr.script_pubkey())
    }

    /// Verify and apply a pasted packet.
    ///
    /// Every refusal names its reason. A user who pasted the wrong window's
    /// text should be told that, not handed "invalid".
    pub fn import_packet(&mut self) -> Result<String, String> {
        let text = self.swap_packet_in.trim();
        if text.is_empty() {
            return Err("Nothing pasted.".into());
        }
        let packet = Packet::decode(text).map_err(|e| e.to_string())?;
        let id = packet.swap_id.to_string();

        if self.ensure_session(&id).is_ok() {
            if let Some(session) = self.swap_sessions.get_mut(&id) {
                let what = session.accept_packet(&packet).map_err(|e| e.to_string())?;
                session.save(&self.datadir).map_err(|e| e.to_string())?;
                return Ok(what.describe().to_string());
            }
        }

        // An opening packet starts a new swap on our side.
        if packet.seq == 0 {
            return self.join_swap(&packet);
        }

        Err(format!(
            "No open session for swap {}. If this wallet created it, the \
             secret file is missing from the data directory.",
            &id[..8.min(id.len())]
        ))
    }

    /// Alice's entry: the opening packet *is* the offer.
    fn join_swap(&mut self, packet: &Packet) -> Result<String, String> {
        if !nightfall_swap::ui::availability(self.network).is_enabled() {
            return Err("Swaps are disabled on this network.".into());
        }
        let session = Session::join_from_packet(
            self.network,
            self.swap_script("redeem")?,
            self.swap_script("punish")?,
            packet,
        )
        .map_err(|e| e.to_string())?;
        let mut session = session;
        session.persist_to(&self.datadir);
        session.save(&self.datadir).map_err(|e| e.to_string())?;
        let id = session.id.to_string();
        let stored = StoredSwap::new(
            SwapState::new(Role::Alice),
            session.amounts.night_darks,
            session.amounts.btc_sats,
        );
        nightfall_swap::persist::save(&self.datadir, &stored).map_err(|e| e.to_string())?;
        let msg = nightfall_swap::session::Accepted::Opening
            .describe()
            .to_string();
        self.swap_sessions.insert(id, session);
        Ok(msg)
    }

    pub(crate) fn ensure_session(&mut self, id: &str) -> Result<(), String> {
        if self.swap_sessions.contains_key(id) {
            return Ok(());
        }
        let uuid = id.parse().map_err(|_| "Not a swap id".to_string())?;
        let session = Session::load(&self.datadir, uuid).map_err(|e| e.to_string())?;
        self.swap_sessions.insert(id.to_string(), session);
        Ok(())
    }

    /// The next packet for the counterparty, as text to copy.
    pub fn export_packet(&mut self, stored: &StoredSwap) -> Result<String, String> {
        let id = stored.state.id().to_string();
        self.ensure_session(&id)?;
        let session = self
            .swap_sessions
            .get_mut(&id)
            .ok_or("No open session for this swap.")?;
        match session.next_packet() {
            Ok(packet) => {
                session.save(&self.datadir).map_err(|e| e.to_string())?;
                Ok(packet.encode())
            }
            Err(_) => session.last_packet().map(|p| p.encode()).ok_or_else(|| {
                "Nothing to copy yet. If you are Bob, the Bitcoin lock is \
                     exported separately once you have a funding outpoint."
                    .into()
            }),
        }
    }

    /// Buttons that change a swap. Deliberately narrow: the driver moves
    /// swaps, a human only aborts or gives up on one.
    pub fn run_swap_action(&mut self, id: &str, action: &nightfall_swap::ui::Action) -> String {
        use nightfall_swap::ui::Action;
        match action {
            Action::Forget => match self.forget_swap(id) {
                Ok(()) => "Swap removed.".into(),
                Err(e) => e,
            },
            Action::CancelNow => match self.stored_by_id(id) {
                Ok(stored) => {
                    match self.send_swap_tx(&stored, crate::app_swap_send::SendWhat::Cancel) {
                        Ok(msg) => msg,
                        Err(e) => e,
                    }
                }
                Err(e) => e,
            },
            Action::SendRefund | Action::SendPunish => match self.stored_by_id(id) {
                Ok(stored) => {
                    let what = if matches!(action, Action::SendRefund) {
                        crate::app_swap_send::SendWhat::Refund
                    } else {
                        crate::app_swap_send::SendWhat::Punish
                    };
                    match self.send_swap_tx(&stored, what) {
                        Ok(msg) => msg,
                        Err(e) => e,
                    }
                }
                Err(e) => e,
            },
            Action::Recover => "Recovery is manual for now: the swap file is in your data \
                 directory under swaps/."
                .into(),
            _ => String::new(),
        }
    }

    /// Load a swap from disk by its id.
    ///
    /// The action buttons carry an id rather than the whole record, because
    /// the record on screen may be a frame old and acting on stale state is
    /// how a swap gets told to do something it already did.
    fn stored_by_id(&self, id: &str) -> Result<StoredSwap, String> {
        let uuid = id.parse().map_err(|_| "Not a swap id".to_string())?;
        nightfall_swap::persist::load(&self.datadir, uuid).map_err(|e| e.to_string())
    }

    fn forget_swap(&mut self, id: &str) -> Result<(), String> {
        let uuid = id.parse().map_err(|_| "Not a swap id".to_string())?;
        let path = nightfall_swap::persist::path(&self.datadir, uuid);
        // Release the coins first: a removed file with reserved coins would
        // strand them, and nothing would ever release them again.
        if let Ok(stored) = nightfall_swap::persist::load(&self.datadir, uuid) {
            if let Ok(mut w) = self.wallet.lock() {
                let _ = w.release_commits(&stored.reserved_commits);
            }
        }
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        let secret = nightfall_swap::persist::secret_path(&self.datadir, uuid);
        let _ = std::fs::remove_file(secret);
        self.swap_sessions.remove(id);
        Ok(())
    }

    /// Confirmations of the Bitcoin lock, or `None` when we could not ask.
    ///
    /// `None` is not zero. The view renders it as "unreachable", never as a
    /// calm nearly-full window — the same rule the driver follows, and the
    /// mistake a swallowed error would make.
    pub fn btc_lock_confirms(&self, _stored: &StoredSwap) -> Option<u32> {
        // No Bitcoin node is wired to the wallet yet.
        None
    }

    pub fn night_lock_confirms(&self, _stored: &StoredSwap) -> Option<u64> {
        None
    }
}
