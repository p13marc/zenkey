//! The runtime's errors.

use zenkey_model::grammar::{IfaceId, KeyError};

/// What can go wrong building, starting or running a service.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A key could not be built: an address, a template value or a member
    /// that is not a plain chunk.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// Zenoh refused a declaration, a put or a query.
    #[error("zenoh: {0}")]
    Zenoh(String),
    /// The service already implements this interface major.
    #[error("{0} is implemented twice")]
    Duplicate(IfaceId),
    /// The service does not implement this interface.
    #[error("{0} is not implemented by this service")]
    NotImplemented(IfaceId),
    /// No resource `<kind token>/<template>` in the interface's contract.
    #[error("{iface} has no resource {resource:?}")]
    NoResource { iface: IfaceId, resource: String },
    /// A resource is declared that a capability the service does not hold
    /// gates (spec §3.3).
    #[error("{iface} {resource:?} is gated on capability {capability:?}, which is not held")]
    Gated {
        iface: IfaceId,
        resource: String,
        capability: String,
    },
    /// The request breaks a rule of the contract: `unavailable` on a required
    /// resource, a cardinality above the contract's, a member token on an
    /// interface without an `epoch` template.
    #[error("{0}")]
    Contract(String),
    /// Bring-up refused (spec §8.2 step 2): a required resource is not
    /// exposed, or an optional one is neither exposed nor accounted for.
    /// No token was declared.
    #[error("not started: {0}")]
    NotExposed(String),
    /// This owner's clock is ahead of its router beyond the HLC delta: it
    /// stops writing state until the drift is gone (spec §4.3, S7).
    #[error("this owner's clock is ahead of its router: state writes stop (spec §4.3)")]
    ClockAhead,
    /// The descriptor this service would serve fails the descriptor check
    /// (spec §3.3); the codes are listed. A bug in the runtime or its
    /// caller, never a network condition.
    #[error("the descriptor fails its check: {0}")]
    Descriptor(String),
}

pub(crate) fn zenoh(e: impl std::fmt::Display) -> Error {
    Error::Zenoh(e.to_string())
}

/// The runtime's result.
pub type Result<T, E = Error> = std::result::Result<T, E>;
