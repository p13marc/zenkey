//! The vocabulary every judge is written in.
//!
//! What belongs here: the things **more than one judge must agree about** —
//! today, the ceilings on how many offenders a report names, and when an
//! owner's own stamp proves S1 (the doctor's `state-stamp-foreign` and
//! `check conform`'s `state-stamp`). What does not:
//! the id vocabularies, which went to [`crate::report`] with #347 because
//! they are serde-pinned wire shapes and that is where those live; any
//! check's own logic; and any sentence written in one judge's voice.
//!
//! v1's shared judgements (the synthetic attachment marker, a key's
//! producer, "the new plane" and the data-plane scopes) left with v1's
//! judges at #612's FJ9; the `v1` branch keeps them.

use std::collections::BTreeSet;

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

// ─── S1 from outside ────────────────────────────────────────────────────────

/// `Ok` when an owner's own stamp proves S1 here, or why it proves nothing
/// (spec §4.2, "A tool's S1 check", 0.16–0.17). A stamp is clean only
/// against the routers a run verified, none of them the owner: an owner
/// that is its own router stamps with that router's id. `owner` holds the
/// owner's `meta.zid`, by value. A foreign stamp never asks this: it is the
/// finding whatever was read.
pub(crate) fn s1_premise(
    admin: Option<&Result<crate::judge::doctor::AdminSpace, String>>,
    owner: &BTreeSet<String>,
) -> Result<(), String> {
    let unseen = |why: String| {
        Err(format!(
            "{why}, so the owner may be its own router, which this run cannot see: its stamp \
             proves nothing (§4.2, \"A tool's S1 check\")"
        ))
    };
    match admin {
        None => unseen("the admin space was not read".to_owned()),
        Some(Err(e)) => unseen(format!("the admin space could not be read: {e}")),
        Some(Ok(a)) if a.routers.is_empty() => {
            unseen("no router's admin answer was verified".to_owned())
        }
        Some(Ok(a)) if owner.iter().any(|z| a.router_zids.contains(z)) => Err(
            "the owner's session is a router this run knows: its stamp and the router's carry \
             one id, so S1 cannot be observed here (§4.2, \"A tool's S1 check\")"
                .to_owned(),
        ),
        Some(Ok(_)) => Ok(()),
    }
}
