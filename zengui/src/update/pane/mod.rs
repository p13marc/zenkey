//! One module per pane (#175), each taking the state it can move.
//!
//! Nine of the eleven panes have state of their own. Detail and History do
//! not: they are windows onto the selected key, so they take `&mut Subject`
//! and nothing else. That is worth knowing before #180 docks the panes — two
//! of the eleven docks have nothing to dock.

pub(crate) mod admin;
pub(crate) mod blob;
pub(crate) mod call;
pub(crate) mod context;
pub(crate) mod detail;
pub(crate) mod doctor;
pub(crate) mod echo;
pub(crate) mod media;
pub(crate) mod nodes;
pub(crate) mod publish;
pub(crate) mod replay;
