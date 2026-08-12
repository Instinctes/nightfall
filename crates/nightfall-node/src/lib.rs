//! Nightfall full node library — embeddable by Core Wallet and `nightfalld`.

pub mod rpc;
pub mod runtime;

pub use runtime::{NodeConfig, NodeHandle, NodeInner, SharedState, StatusSnap};
