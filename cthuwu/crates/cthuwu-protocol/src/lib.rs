//! Transport- and inference-independent coordination types for Cthuwu.
//!
//! This crate deliberately contains no network client, model adapter, filesystem access, clock,
//! or production signing implementation. Callers supply those capabilities at the trust boundary.
//!
//! Cthuwu is the singular decentralized collective formed by all live Tentacles; it is not an
//! agent or owner. A Tentacle is an independently operated, durable agent and an incarnation is
//! only one runtime generation of it. Public human chat users are acolytes, not Tentacles. A human
//! operator can shape its Tentacle's agenda without becoming an owner of the collective.
//! Historical `CthulhuId`-named Council fields remain on the v1 wire solely for compatibility.
//! They are legacy coordination namespaces, never ERC-8004 identities or evidence that multiple
//! individual "Cthulhus" exist. One Tentacle stopping does not end the decentralized collective.

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
