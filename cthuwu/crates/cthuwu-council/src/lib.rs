//! Deterministic, transport-independent coordination for the Council of Cthulhus.
//!
//! This crate intentionally contains no XMTP SDK or model-provider dependency. The in-memory
//! transport and local registry are complete; live XMTP groups and ERC-8004 remain adapter
//! boundaries until their concrete deployments are selected and tested.

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
