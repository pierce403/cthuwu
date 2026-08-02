//! Transport- and inference-independent wire and domain types for the Council of Cthulhus.
//!
//! This crate deliberately contains no network client, model adapter, filesystem access, clock,
//! or production signing implementation. Callers supply those capabilities at the trust boundary.

mod capability;
mod envelope;
mod error;
mod identity;
mod ids;
mod messages;
mod signing;
mod tentacle;
mod time;
mod validation;

pub use capability::*;
pub use envelope::*;
pub use error::*;
pub use identity::*;
pub use ids::*;
pub use messages::*;
pub use signing::*;
pub use tentacle::*;
pub use time::*;

/// The only Council wire protocol name accepted by this version of the crate.
pub const COUNCIL_PROTOCOL_NAME: &str = "cthuwu-council";

/// Maximum accepted serialized Council envelope size.
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
