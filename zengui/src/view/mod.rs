//! Presentation. Nothing here holds state or talks to the bus.
//!
//! Each surface is a free `fn` taking plain data rather than being a method on
//! the app. That is what lets a surface be rendered standalone in a test
//! (zensight's `tests/ui_tests.rs` shape), which `tests/panes.rs` and its 44
//! headless tests exist to do. **No function under `view/` names `Zengui`**,
//! and that is the contract.
//!
//! ## `pane` versus `section`, and who owns the scroll
//!
//! A **`pane`** returns `Element<'_, Message>` and is something the workspace
//! can show on its own. A **`section`** returns `Column<'_, Message>` and is a
//! piece of one — [`inspector::pane`] stacks four of them and wraps the result
//! in a single `scrollable`. The distinction is not stylistic: a scrollable
//! nested inside a scrollable scrolls neither well, so exactly one caller owns
//! the scroll and the pieces are columns.
//!
//! `detail`, `history`, `blob` and `media` were panes until #182 and are
//! sections now. Nothing they *say* changed — every honesty assertion about
//! them survived with a one-line call-site edit — which is the test that the
//! collapse was composition rather than a rewrite.
//!
//! A **`dock`** (`activity::dock`) is a region that holds streams rather
//! than a subject, and it owns its own tab strip. `location::bar`/`overlay`/
//! `banner`/`open_row` are none of the three: surfaces that are not a region
//! of the workspace at all.
//!
//! ## Where tabs are honest
//!
//! The workspace had eleven, and they were eleven views of one thing — #182
//! collapsed four into the Inspector for that reason and #183 moved two more
//! into the dock. The dock has four, and they are four genuinely parallel
//! streams that run whether or not they are on screen. A tab is honest when
//! only one of the things it switches between can be *read* at a time; it is
//! dishonest when only one can be *shown*.
//!
//! ## Three virtualized lists, one window
//!
//! The tree, the Inspector's timeline and the dock's echo stream all draw
//! O(visible) rows through [`kit::window`]. Each supplies its own row height
//! and its own scroll offset; none supplies its own idea of what "visible"
//! means. Before #183 only the tree had a window at all.
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
//! **A field's *type* carries the same weight as its name.** `DetailData`'s
//! fetch outcome was an `Option`, which made "superseded" indistinguishable
//! from "not asked" and let the pane deny a question it had answered
//! ([`detail::Fetched`], #181). The rule the crate keeps rediscovering is the
//! same one in three places now: if a surface has three states, do not hand it
//! a type with two (#176's `Pane(PaneId, PaneMsg)`, #250's two adjacent watch
//! sets, this).
//!
//! The contract survived the #175 split intact, and that is worth recording:
//! `panes::split` and `location::bar` (the toolbar's successor, #185) moved
//! *out* of `app.rs` and take five sub-states each — never `Zengui`. Nothing
//! under `view/` had to change to make the shell shrink, because `view` takes
//! `&self` and six shared borrows of one struct are six disjoint borrows.
//!
//! Both halves of the paragraph above used to be wrong here. The contract said
//! `…_view`, and every function had been called `pane` for a year; and it said
//! "over a plain state struct", when four surfaces already took an owned data
//! struct instead. A contract nobody reads against the code is the failure this
//! crate keeps finding (#178, #250).

pub mod activity;
pub mod admin;
pub mod blob;
pub mod call;
pub mod contexts;
pub mod detail;
pub mod doctor;
pub mod echo;
pub mod history;
pub mod inspector;
pub mod kit;
pub mod location;
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
pub mod tree;
