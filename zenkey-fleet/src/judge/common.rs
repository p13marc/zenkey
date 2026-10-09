//! The vocabulary every judge is written in.
//!
//! What belongs here: the things **more than one judge must agree about** —
//! today, the ceilings on how many offenders a report names. What does not:
//! the id vocabularies, which went to [`crate::report`] with #347 because
//! they are serde-pinned wire shapes and that is where those live; any
//! check's own logic; and any sentence written in one judge's voice.
//!
//! v1's shared judgements (the synthetic attachment marker, a key's
//! producer, "the new plane" and the data-plane scopes) left with v1's
//! judges at #612's FJ9; the `v1` branch keeps them.

// ─── the caps ───────────────────────────────────────────────────────────────
//
// Two ceilings over two different populations. They are here because more
// than one judge uses each — not because they are one number: the values
// are separate policies that today happen to be 20 and 3, and a future
// change to one must not drag the other. The *mechanism*
// (count everything offered, keep the first `cap`) is
// [`Examples`](crate::model::examples::Examples)'s and always was.

/// How many offending keys a check names before it says "… and N more".
///
/// Shared by the doctor's per-check findings, `field`'s per-path ones and
/// `expect`'s violations — judges that had constants of the same value
/// under different names.
pub(crate) const FINDING_CAP: usize = 20;

/// How many example expansions a budget finding or cell carries — enough to
/// recognise the family member that exploded, without pasting the
/// population.
pub const EXPANSION_CAP: usize = 3;
