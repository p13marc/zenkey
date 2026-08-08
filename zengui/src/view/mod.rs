//! Presentation. Nothing here holds state or talks to the bus.
//!
//! Each pane is a free `fn …_view(&State, …) -> Element<'_, Message>` over a
//! plain state struct rather than a method on the app, which is what lets a
//! pane be rendered standalone in a test (zensight's `tests/ui_tests.rs` shape).

pub mod echo;
pub mod kit;
pub mod status;
pub mod theme;
pub mod tokens;
pub mod tree;
