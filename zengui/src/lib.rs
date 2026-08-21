//! zengui — the graphical bus explorer (issue #18).
//!
//! The GUI sibling of `zenctl`, over the same engine: everything that is not
//! presentation lives in `zenkey-fleet` (`docs/redesign-2026-07.md` §6.3 — "a
//! missing type is a zenkey-fleet issue, not a zengui workaround").
//!
//! **Key-agnostic by construction.** zengui is a useful explorer on *any* Zenoh
//! bus; the keyspace-v2 convention is an enrichment overlay that lights up when
//! a key parses, never a precondition. That is the grain of the engine already:
//! [`zenkey_fleet::KeyTreeSnapshot`] groups on a plain `split('/')` with no
//! grammar knowledge, and `zenkey::grammar::parse_full` returns `Option` rather
//! than `Result` because for an observer "does not parse" is an answer, not an
//! error. The overlay lives in exactly one module, [`keyfacts`].
//!
//! Like every explorer, the session is **un-namespaced** (RFC 09 §5): zengui
//! spells full wire keys and does its own base handling, which is what lets it
//! see traffic a namespaced application is blind to.

pub mod admin;
pub mod app;
pub mod blob;
pub mod config;
pub mod doctor;
pub mod echo;
pub mod expansion;
pub mod history;
pub mod keyfacts;
pub mod link;
pub mod message;
pub mod nodes;
pub mod prefs;
pub mod replay;
pub mod scope;
pub mod series;
pub mod services;
pub mod shortcuts;
pub(crate) mod state;
pub mod view;
