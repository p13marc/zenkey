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
//! The count, honestly: [`deployment`] names **all six** — a base change
//! clears four, remembers a preference in `chrome`, and drops the selected
//! key's decode in `sub`. [`bus`], [`subject`] and [`workspace`] name five,
//! each for a traceable reason: a tick moves everything the bus can move; a
//! selection is a causal chain that ends in a fetch; and the workspace has one
//! arm that hands to [`pane::replay`], which is a bus in disguise. [`chrome`]
//! names four and cannot move a row or a watch, and [`pane`] hands each pane
//! only its own state.
//!
//! Five is not a failure to decompose. `&mut Zengui` said *nothing*;
//! `(dep, obs, sub, tree, work)` says five-of-six, and `(&mut CallForm, Ctx)`
//! says one-plus-reads. The measurement is the deliverable.
//!
//! ## `Ctx` is not `Status`
//!
//! They look alike and are opposites. `Ctx` is an update-side type whose whole
//! point is being *narrow*: what a handler may consult but not move. `Status`
//! is view-side, where nothing is narrow — a status strip reads five of six
//! sub-states because that is what a status strip *is*.

use crate::state::{Deployment, Observation, SubjectState};

pub(crate) mod bus;
pub(crate) mod chrome;
pub(crate) mod deployment;
pub(crate) mod pane;
pub(crate) mod subject;
pub(crate) mod workspace;

/// What a handler may read but not move.
///
/// Three shared references, `Copy`, and deliberately not four: a handler that
/// needs the tree or the workspace is asking to change how something is
/// *presented*, and that is what being handed it mutably is for.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub(crate) dep: &'a Deployment,
    pub(crate) obs: &'a Observation,
    pub(crate) sub: &'a SubjectState,
}
