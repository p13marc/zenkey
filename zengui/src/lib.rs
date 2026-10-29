//! zengui — the graphical bus explorer (issue #18).
//!
//! The GUI sibling of `zenctl`, over the same engine: everything that is not
//! presentation lives in `zenkey-fleet` (`docs/redesign-2026-07.md` §6.3 — "a
//! missing type is a zenkey-fleet issue, not a zengui workaround").
//!
//! **Key-agnostic by construction.** zengui is a useful explorer on *any* Zenoh
//! bus; zk2's grammar is an enrichment overlay that lights up when a key
//! parses, never a precondition. That is the grain of the engine already:
//! [`zenkey_fleet::KeyTreeSnapshot`] groups on a plain `split('/')` with no
//! grammar knowledge, and the lens gives every key an identity — the rung its
//! ladder reached — because for an observer "not a zk2 key" is an answer, not
//! an error (O1). The overlay's reading lives in one module, [`keyfacts`]
//! (#614, FL7: zk2's identity rungs, where v1's facts ladder stood).
//!
//! **Two sessions** since #614 (FL6a), as zenctl and zenwatch run: the data
//! pump watches on a session in **no** namespace (RFC 09 §5) — zengui spells
//! full wire keys, which is what lets it see traffic a namespaced application
//! is blind to — and presence, descriptors and contracts are read on a
//! session **in** the deployment's namespace, the way the deployment's own
//! consumers read them. The namespace is `--namespace` (`--base` its alias),
//! and a namespace switch reopens only the second session. Since FL6b the
//! pump is zk2's monitor, and the tree starts from zk2's contract skeleton,
//! built from the same lens the window resolves keys through.
//!
//! # The Elm loop, in four places
//!
//! `app.rs` is the shell — `new`, `update`, `view`, `subscription`, and
//! nothing else. What used to be a 3,598-line god struct beside them is now:
//!
//! | where | what | rule |
//! |---|---|---|
//! | `state/` | the 64 fields, in six sub-states | each named for what invalidates it |
//! | `update/` | one module per message group, one per pane | a signature is the exhaustive list of sub-states it can move |
//! | [`services`] | every bus call, as a free `fn -> Task<Message>` | the future and its landing; never a state mutation |
//! | [`view`] | every surface, as a free `fn -> Element` | no function here names `Zengui` |
//!
//! `state` and `update` are `pub(crate)`: they are the app's own anatomy, and
//! nothing outside the crate has business naming a sub-state. `services` and
//! `view` are public because the pane tests render surfaces directly.

pub mod admin;
pub mod app;
pub mod budget;
pub mod config;
pub mod configure;
pub mod doctor;
pub mod echo;
pub mod expansion;
pub mod history;
pub mod keyfacts;
pub mod link;
pub mod message;
pub mod metric;
pub mod nodes;
pub mod patharena;
pub mod prefs;
pub mod presence_link;
pub mod replay;
pub mod scope;
pub mod series;
pub mod services;
pub mod shortcuts;
pub(crate) mod state;
pub mod timing;
pub mod traffic;
pub(crate) mod update;
pub mod value;
pub mod verdict;
pub mod view;
