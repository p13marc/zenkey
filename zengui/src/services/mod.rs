//! Every bus call the app makes, as free functions returning `Task<Message>`
//! (#175).
//!
//! ## Why this is a module and not a habit
//!
//! `message.rs` used to claim it kept "the async/sync boundary readable at a
//! glance" by listing every landing in one enum. #176 retired that claim as
//! already-false and replaced it with a grep: *every async landing is a
//! `Task::perform` in `app.rs` or a `yield` in `link.rs`*. This module is the
//! first half of that grep made into a place.
//!
//! ## The extraction was nearly free, and the reason is worth knowing
//!
//! Every one of these sites already hoisted its `self` reads into owned locals
//! before its `async move` — it had to, because `async move` cannot borrow
//! `&self` across the boundary. So the async surface was *already* a set of
//! functions over owned values; all that was missing was the signature saying
//! so.
//!
//! ## What is here and what is not
//!
//! Here: the future, and the closure that names the message it lands on.
//! Not here: the state mutation that precedes a task — `doctor.in_flight = true`
//! and its kind stay in the handler, because they are what the *app* does, and
//! a service that flipped a pane's flag would be reaching backwards.
//!
//! ## The eight modules, split by what an answer *is*
//!
//! Not by which pane asks: `link` (a session), `sweep` (a fleet), `value` (one
//! key), `watch` (coverage), `write` (a change), `record` (a `.zrec`, either
//! direction), `context` (the shared store), `prefs` (the window's memory).
//! Two panes reach into three of them, which is the honest shape — a pane is
//! a question, not a transport.
//!
//! The last three are #255: the eight blocking filesystem calls that stayed
//! on the update thread through #175, because moving a synchronous call
//! off-thread changes when its result lands — a behaviour change that had no
//! business hiding inside a refactor claiming to make none. Each is now a
//! task with a message landing, and the update tree is gated against growing
//! filesystem calls back (`scripts/check-fs-seam.sh`).

pub mod context;
pub mod link;
pub mod prefs;
pub mod record;
pub mod sweep;
pub mod value;
pub mod watch;
pub mod write;
