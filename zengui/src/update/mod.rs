//! The handlers, one module per message group and one per pane (#175).
//!
//! ## What a signature says here
//!
//! `&mut Zengui` said nothing: a handler that took it could move any of 64
//! fields, and the only way to find out which it *did* move was to read it.
//! Every function below instead names, in its parameter list, the exhaustive
//! set of sub-states it can move — and reads the rest through [`Ctx`], which
//! is shared.
//!
//! Two of them name five of six, and that is the measurement rather than a
//! lapse: [`bus`] moves five because a tick moves everything the bus can move,
//! and [`deployment`] moves five because pointing at a different fleet
//! invalidates four sub-states and remembers a preference. `pane::replay`
//! names the same five for the same reason as `bus` — a replayed tick moves
//! what a live one moves.
//!
//! ## `Ctx` is not `Status`
//!
//! They look alike and are opposites. `Ctx` is an update-side type whose whole
//! point is being *narrow*: what a handler may consult but not move. `Status`
//! is view-side, where nothing is narrow — a status strip reads five of six
//! sub-states because that is what a status strip *is*.

use crate::state::{Deployment, Observation, Subject};

pub(crate) mod bus;
pub(crate) mod pane;

/// What a handler may read but not move.
///
/// Three shared references, `Copy`, and deliberately not four: a handler that
/// needs the tree or the workspace is asking to change how something is
/// *presented*, and that is what being handed it mutably is for.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub(crate) dep: &'a Deployment,
    pub(crate) obs: &'a Observation,
    pub(crate) sub: &'a Subject,
}
