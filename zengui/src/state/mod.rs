//! `Zengui`\'s 64 fields, in six groups, each named for what invalidates it
//! (#175).
//!
//! ## What the split is for
//!
//! `&mut Zengui` said nothing. A handler that took it could move any field,
//! and the only way to find out which ones it *did* move was to read it. Six
//! sub-states turn that into a signature: `(&mut CallForm, Ctx)` says
//! one-plus-reads, and `(dep, obs, sub, tree, work)` says five-of-six — which
//! is a measurement rather than a shrug, and what #180 needs in order to dock
//! a pane.
//!
//! ## The six, and what invalidates each
//!
//! | group | invalidated by |
//! |---|---|
//! | [`Chrome`] | nothing the bus does — it is the window |
//! | [`Deployment`] | pointing at a different fleet |
//! | [`Observation`] | the pump restarting; its coverage, by a base change |
//! | [`SubjectState`] | the user pointing the workspace somewhere else |
//! | [`TreeState`] | a new merge input, or a presentation change |
//! | [`Workspace`] | per sub-group — see [`workspace`] |
//!
//! ## Placement is what the test now guards
//!
//! The old gate test read this file\'s source and compared the fields
//! `forget_deployment` did *not* touch against a checklist. The split kills
//! that job: every reset below is a struct replacement or a delegating
//! `forget`, so "a field was added and the reset was not told" is no longer
//! representable. What is still representable — and what nothing else
//! catches — is **placement**: `expanded` landing in `Deployment` (cleared,
//! but for the wrong reason) or `facts` in `Chrome` (never cleared at all).
//! So the test asserts the 64 `(group, field)` pairs against a table, through
//! a macro rather than by reading source text.

/// Declare a sub-state, and record its field names for the placement test.
///
/// The generated `FIELDS` is the only reason this is a macro. The old gate
/// test read `app.rs`'s source text and scanned for `name:` — a habit that
/// broke five times over as the struct grew, and one struct per file would
/// have turned one fragile anchor into six. `stringify!` cannot be wrong
/// about a field it is expanding.
macro_rules! sub_state {
    (
        $(#[$outer:meta])*
        $vis:vis struct $name:ident {
            $($(#[$inner:meta])* $fvis:vis $field:ident : $ty:ty),* $(,)?
        }
    ) => {
        $(#[$outer])*
        $vis struct $name {
            $($(#[$inner])* $fvis $field: $ty,)*
        }

        // Test-only: it exists for one assertion, and a struct that carried
        // its own field list into the binary would be paying for a claim
        // nothing at runtime reads.
        #[cfg(test)]
        impl $name {
            /// Every field, in declaration order (see [`sub_state`]).
            pub(crate) const FIELDS: &'static [&'static str] = &[$(stringify!($field)),*];
        }
    };
}

pub(crate) mod chrome;
pub(crate) mod deployment;
pub(crate) mod observation;
pub(crate) mod subject;
pub(crate) mod tree;
pub(crate) mod workspace;

pub(crate) use chrome::Chrome;
pub(crate) use deployment::Deployment;
pub(crate) use observation::Observation;
pub(crate) use subject::SubjectState;
pub(crate) use tree::TreeState;
pub(crate) use workspace::Workspace;

#[cfg(test)]
mod tests {
    use super::workspace::{EchoPane, ReplayMode, Verdicts, Workbench};
    use super::*;

    /// Where each of the 64 fields lives, as a claim rather than a reading.
    ///
    /// This is what survives of `every_zengui_field_is_either_forgotten_or_\
    /// deliberately_kept`. That test guarded two things and the split kills
    /// one of them: every reset is now a struct replacement or a delegating
    /// `forget`, so "a field was added and the reset was not told" — #179 and
    /// #194's shared shape — is no longer representable, and the compiler
    /// makes it so rather than a test.
    ///
    /// What is still representable is **placement**, and nothing else catches
    /// it. `expanded` in `Deployment` would be cleared on a base change, but
    /// for the wrong reason, and would then be cleared again by whatever
    /// #180 does to a docked tree. `facts` in `Chrome` would never be cleared
    /// at all, and the symptom would be a key description from a fleet the
    /// window is no longer pointed at — plausible, wrong, and invisible.
    ///
    /// So: adding a field is still a decision recorded in a diff. It is just
    /// a different decision now — not "does this survive?" but "what is this
    /// a fact *about*?"
    #[test]
    fn every_field_is_in_the_group_that_invalidates_it() {
        const PLACED: &[(&str, &[&str])] = &[
            // The window, and nothing the bus does touches it.
            (
                "chrome",
                &["prefs", "window_dirty", "prefs_note", "palette"],
            ),
            // What fleet this is, and what is claimed about it.
            (
                "dep",
                &[
                    "settings",
                    "session",
                    "schema_store",
                    "skeleton",
                    "slices",
                    "slice_source",
                    "bases",
                    "base_options",
                    "facts",
                ],
            ),
            // What the bus said, and what we asked it to keep saying.
            (
                "obs",
                &[
                    "monitor",
                    "epoch",
                    "link",
                    "observed",
                    "watched",
                    "my_watches",
                    "my_watch_paths",
                    "scope_watches",
                    "seeding",
                    "seeding_paths",
                    "seed_totals",
                    "seeded_watches",
                    "keys",
                    "keys_evicted",
                    "keys_unwatched",
                    "totals",
                ],
            ),
            // The selection and everything derived from it.
            (
                "sub",
                &[
                    "current",
                    "selected_latency",
                    "fetched",
                    "decoded",
                    "history",
                    "rate_series",
                    "series_leaf",
                    "series",
                ],
            ),
            // The rows, how they are grouped, and what is open. `flat` and
            // the two shape counters left the old list's "instrumentation"
            // bucket for this one: they are facts about the tree.
            (
                "tree",
                &[
                    "flat",
                    "pivot",
                    "tree_search",
                    "tree_scroll",
                    "expanded",
                    "merged_cache",
                    "shape_reused",
                    "shape_rebuilt",
                ],
            ),
            (
                "work",
                &["right_pane", "verdicts", "bench", "echo", "replay"],
            ),
            (
                "work.verdicts",
                &["roster", "node_detail", "doctor", "blob", "admin"],
            ),
            (
                "work.bench",
                &[
                    "context_form",
                    "call_form",
                    "publish_form",
                    "publication",
                    "media",
                ],
            ),
            ("work.echo", &["echo", "echo_view"]),
            (
                "work.replay",
                &[
                    "replay",
                    "replay_open",
                    "replay_note",
                    "recording",
                    "recorded",
                ],
            ),
        ];

        let actual: Vec<(&str, &[&str])> = vec![
            ("chrome", Chrome::FIELDS),
            ("dep", Deployment::FIELDS),
            ("obs", Observation::FIELDS),
            ("sub", SubjectState::FIELDS),
            ("tree", TreeState::FIELDS),
            ("work", Workspace::FIELDS),
            ("work.verdicts", Verdicts::FIELDS),
            ("work.bench", Workbench::FIELDS),
            ("work.echo", EchoPane::FIELDS),
            ("work.replay", ReplayMode::FIELDS),
        ];
        assert_eq!(
            actual, PLACED,
            "a sub-state's fields changed. Add it above with the reason it \
             belongs there — a field's group is the answer to \"what \
             invalidates this?\", and getting it wrong is how a stale fact \
             about one fleet ends up shown against another."
        );

        // The count is its own claim: a split that quietly dropped a field
        // would still pass every assertion above. 64 at the #175 split, minus
        // `node_selected`, which #181 replaced with the one subject.
        let leaves: usize = actual
            .iter()
            .map(|(g, f)| if *g == "work" { 1 } else { f.len() })
            .sum();
        assert_eq!(leaves, 63, "the split must place every field exactly once");
    }
}
