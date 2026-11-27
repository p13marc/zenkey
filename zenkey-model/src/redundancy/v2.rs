//! `redundancy.v2`'s declaration (`spec/profiles/redundancy/v2.md` §2.2,
//! #987's N1): v1 §2.2 with `redundancy.v2` where it reads
//! `redundancy.v1`, one reason before v1's four, `majors`, and one after
//! them, `explicit_stream`.
//!
//! Everything else this major keeps or replaces is read for both majors in
//! [`super`]: the election, the heal and §5's questions are v1's by
//! reference, and the fence ([`super::Fence`]), a descriptor's run
//! ([`super::read_descriptor`]), the draw ([`super::draw_again`]) and an
//! address's governance ([`super::governance`]) read either major.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Declaration, ELECTION, Invalid, NAME, elections};
use crate::contract::Contract;
use crate::grammar::{IfaceId, KindToken};

/// The major's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "redundancy.v2";

/// Reads an interface's declaration as `redundancy.v2` reads it (§2.2;
/// `v2/declarations.json`) from its contract's `uses`, each of its
/// resources as its resolved annotations (core §9.3, in any order) and
/// whether it is an explicit stream (`@stream`), and whether the
/// deployment's tokenless set names it.
///
/// - **Not governed** where `uses` lists no `redundancy.v2` and either
///   lists another major of the profile, whose text then decides, or no
///   resource carries `redundancy.election`.
/// - **Invalid**, with each reason that holds, checked alone in this
///   order: `majors` (`redundancy.v2` and another major listed; the
///   interface is then read as governed for the checks after it),
///   `not_used`, `value`, `disagrees` (v1 §2.2's, a resource carrying none
///   read as `claim`, one found `value` taking no part), `tokenless` and
///   `explicit_stream`, the last two for a governed interface alone.
/// - **Governed** otherwise, with the one election every resource
///   resolves to, an interface with no resource `claim`.
#[must_use]
pub fn declaration<'a, U, R>(uses: U, resources: R, tokenless: bool) -> Declaration
where
    U: IntoIterator<Item = &'a IfaceId>,
    R: IntoIterator<Item = (&'a BTreeMap<String, Value>, bool)>,
{
    let mut governed = false;
    let mut another_major = false;
    for u in uses {
        if u.name() == NAME {
            if u.major() == 2 {
                governed = true;
            } else {
                another_major = true;
            }
        }
    }
    let resources: Vec<(Option<&Value>, bool)> = resources
        .into_iter()
        .map(|(a, explicit)| (a.get(ELECTION), explicit))
        .collect();
    let values: Vec<Option<&Value>> = resources.iter().map(|(v, _)| *v).collect();
    if !governed && (another_major || values.iter().all(Option::is_none)) {
        return Declaration::NotGoverned;
    }
    let mut why = Vec::new();
    if governed && another_major {
        why.push(Invalid::Majors);
    }
    if !governed {
        // No major of the profile listed at all, and the key written.
        why.push(Invalid::NotUsed);
    }
    let election = elections(&values, &mut why);
    if governed && tokenless {
        why.push(Invalid::Tokenless);
    }
    if governed && resources.iter().any(|(_, explicit)| *explicit) {
        why.push(Invalid::ExplicitStream);
    }
    if why.is_empty() {
        Declaration::Governed(election)
    } else {
        Declaration::Invalid(why)
    }
}

/// [`declaration`] of a contract, `tokenless` saying whether the
/// deployment's tokenless set names its interface: an explicit stream is a
/// resource whose kind token is `@stream`.
#[must_use]
pub fn declaration_of(c: &Contract, tokenless: bool) -> Declaration {
    declaration(
        &c.uses,
        c.resources
            .iter()
            .map(|r| (&r.annotations, r.token == KindToken::ExplicitStream)),
        tokenless,
    )
}
