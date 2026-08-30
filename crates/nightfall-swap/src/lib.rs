//! NIGHT ↔ BTC atomic swap.
//!
//! **Experimental. Not for real coins on mainnet.** Protocol:
//! `docs/SWAP-SPEC-DRAFT.md` v0.3. Known construction, Monero wart accepted.
//! No NIGHT refund.

pub mod adaptor;
pub mod bitcoin_rpc;
pub mod bitcoin_tx;
pub mod driver;
pub mod fees;
pub mod messages;
pub mod night_watch;
pub mod packet;
pub mod persist;
pub mod session;
mod session_spend;
pub mod state;
pub mod timelock;
pub mod ui;
pub mod warnings;
pub mod watch;

pub use nightfall_crypto::dleq;
pub use nightfall_crypto::swap::{LockError, SharedLock, SwapOffer, SwapShare};

pub use messages::{Amounts, Message0, Message1, Message2, Message3, Message4, MessageRedeemEnc};
pub use persist::{save as save_swap, PendingSend, SendKind, StoredSwap};
pub use state::{AbortReason, Role, SwapEvent, SwapState};
pub use timelock::Depths;
pub use watch::{ChainWatch, FakeWatch, WatchError};
