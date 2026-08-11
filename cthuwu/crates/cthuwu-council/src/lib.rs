//! Deterministic, transport-independent coordination among Tentacles composing Cthuwu.
//!
//! This crate intentionally contains no XMTP SDK or model-provider dependency. The in-memory
//! transport and local registry are complete. The ERC-8004 registry boundary is read-only and
//! pinned to canonical Base mainnet; writes belong to a separate narrow signer workflow.

pub mod clock;
pub mod governance;
pub mod lease;
pub mod liveness;
pub mod persistence;
pub mod propagation;
pub mod registry;
pub mod rendezvous;
pub mod routing;
pub mod simulator;
pub mod transport;

pub use simulator::{SimulationReport, run_deterministic_simulation};
