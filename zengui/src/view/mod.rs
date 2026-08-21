//! Presentation. Nothing here holds state or talks to the bus.
//!
//! Each surface is a free `fn` returning `Element<'_, Message>` — `pane` where
//! it is one, `strip`/`overlay`/`banner`/`open_row` where it is not — taking
//! plain data rather than being a method on the app. That is what lets a pane
//! be rendered standalone in a test (zensight's `tests/ui_tests.rs` shape),
//! which `tests/panes.rs` and its 42 headless tests exist to do. **No function
//! under `view/` names `Zengui`**, and that is the contract.
//!
//! A surface with more than a few inputs takes one borrowed-fields struct by
//! value — [`detail::DetailData`], [`nodes::NodesData`], [`history::HistoryData`],
//! [`status::Status`], [`tree::TreeData`] — rather than a long argument list.
//! Nine positional arguments hid two adjacent `&BTreeSet<String>` watch sets
//! whose transposition compiled and drew every watched subtree as "seeding…"
//! (#250).
//!
//! A builder was rejected for those: the test call sites want a literal, and a
//! literal missing a field is a compile error where a builder missing a
//! `.with_…` is a silent default.
//!
//! The contract survived the #175 split intact, and that is worth recording:
//! `panes::split` and `toolbar::strip` moved *out* of `app.rs` and take five
//! and four sub-states respectively — never `Zengui`. Nothing under `view/`
//! had to change to make the shell shrink, because `view` takes `&self` and
//! six shared borrows of one struct are six disjoint borrows.
//!
//! Both halves of the paragraph above used to be wrong here. The contract said
//! `…_view`, and every function had been called `pane` for a year; and it said
//! "over a plain state struct", when four surfaces already took an owned data
//! struct instead. A contract nobody reads against the code is the failure this
//! crate keeps finding (#178, #250).

pub mod admin;
pub mod blob;
pub mod call;
pub mod contexts;
pub mod detail;
pub mod doctor;
pub mod echo;
pub mod history;
pub mod kit;
pub mod media;
pub mod nodes;
pub mod palette;
pub mod panes;
pub mod publish;
pub mod replay;
pub mod spark;
pub mod status;
pub mod theme;
pub mod tokens;
pub mod toolbar;
pub mod tree;
