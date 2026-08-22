//! The deprecation burn-down (issue #226): who still speaks each retired
//! subject.
//!
//! [`crate::cutover`] proves a whole key family went silent; the append-only
//! `[[deprecated]]` ledger (RFC 08 §3) records dozens of *individual*
//! retirements, and nothing told you which ones are finished. This walks the
//! ledger and reports four facts per entry: is it still on the wire, does a
//! served introspect slice still declare it (the RFC 08 §6.1 lie — an entry
//! the ledger retired that a live build still serves), does any session
//! declare an intersecting subscriber, and does its `replaced_by` carry
//! traffic — the `cutover` pair, per entry.
//!
//! Same honesty rules as everything else here: silence is never a verdict
//! (RFC 05 §3.1), and a fact that was not asked renders as not-asked, never
//! as "no" (RFC 09 §5.1 O4) — which is why every wire field is an `Option`
//! and why the verdict deliberately reuses [`CutoverVerdict`]'s three states
//! instead of minting a fourth vocabulary.
//!
//! Lives beside `cutover` for `cutover`'s own reason (issue #206): the
//! per-entry ladder and the sample bucketing are judgement over bus traffic,
//! and a second explorer must not have to re-derive them. The frontend keeps
//! the session, the rendering and the exit code.

use std::time::Duration;

use anyhow::Result;
use zenkey::slice::{DeprecationDecl, RegistrySlice};
use zenoh::Session;

use crate::report::{CutoverVerdict, RetiredEntry, RetiredReport};

/// The scope sentence the listen phase operates under — rendered by the
/// caller *before* the window opens (O5): a user watching a long silence
/// deserves to know what was and was not being watched.
pub fn scope_note(entries: usize, new_prefix: &str, window: u64) -> String {
    format!(
        "retired check: {window}s window over {entries} ledger entr(y|ies) — \
         watching the retired families and their replacements, with {new_prefix}** \
         as the fleet's proof of life. `**` cannot cross `@`-chunks: verbatim \
         planes and the admin space are outside this watch by construction (O5)."
    )
}

/// The base-relative wire family one ledger entry maps to.
///
/// The ledger records a subject *tail* and no class, so the class position is
/// `*` — a plain chunk wildcard, which by D2/D4 can reach every data class
/// and no verbatim plane. A host producer's family carries its producer
/// chunk; a service origin's does not (RFC 03 §1.5). Assembled by hand
/// rather than through `zenkey::selector` because no typed builder spells a
/// class wildcard — the shape is stated here and pinned by test instead.
pub fn retired_selector(slice: &RegistrySlice, path: &str) -> String {
    let tail = zenkey::pattern::SubjectPattern::parse(path)
        .map(|p| p.selector_tail())
        // A tail the pattern grammar refuses still names a family verbatim —
        // an unparseable ledger line is a fact, not a reason to bail (O1).
        .unwrap_or_else(|_| path.to_string());
    match &slice.service_origin {
        Some(origin) => format!("v1/{origin}/*/{tail}"),
        None => format!("v1/*/*/{}/{tail}", slice.name),
    }
}

/// The per-entry ladder, pure so it can be exercised without a bus — the
/// same three states as [`crate::cutover::verdict`], per ledger line.
///
/// Order matters, exactly as there: **any** sign of life on the retired
/// subject is the failure, whatever else is true — a sample heard on the
/// wire, a served slice still declaring the path active (§6.1), or a session
/// still subscribed to it (a consumer that has not moved is a migration that
/// is not done). The pass needs both halves: the retired family *observed*
/// silent (`Some(0)`, never an unlistened `None`) while the entry's proof of
/// life carried traffic — its replacement when one is declared, the v1 plane
/// otherwise. Everything short of that is `Unproven`: a replacement nobody
/// has heard speak proves nothing, and neither does a window that never ran.
pub fn entry_verdict(
    wire_samples: Option<u64>,
    still_declared: Option<bool>,
    subscribers: Option<usize>,
    life_samples: Option<u64>,
) -> CutoverVerdict {
    if wire_samples.is_some_and(|n| n > 0)
        || still_declared == Some(true)
        || subscribers.is_some_and(|n| n > 0)
    {
        return CutoverVerdict::OldStillSpeaks;
    }
    match (wire_samples, life_samples) {
        (Some(0), Some(n)) if n > 0 => CutoverVerdict::Pass,
        _ => CutoverVerdict::Unproven,
    }
}

/// Worst-of over the entries: any failure fails the run, else any unproven
/// entry leaves it unproven, else pass. An empty ledger passes — nothing was
/// retired, so nothing can still speak — and the report's coverage statement
/// is what keeps that from reading as fleet-wide absolution.
pub fn overall(entries: &[RetiredEntry]) -> CutoverVerdict {
    if entries
        .iter()
        .any(|e| e.verdict == CutoverVerdict::OldStillSpeaks)
    {
        CutoverVerdict::OldStillSpeaks
    } else if entries
        .iter()
        .any(|e| e.verdict == CutoverVerdict::Unproven)
    {
        CutoverVerdict::Unproven
    } else {
        CutoverVerdict::Pass
    }
}

/// Who a ledger entry belongs to on the wire.
enum Identity {
    /// Match by producer base name (instance suffixes share the slice,
    /// RFC 03 §1.5).
    Host(String),
    /// Match by verbatim service origin — these keys have no producer chunk.
    Service(String),
}

/// One ledger entry's wire matchers, parsed once.
struct Matcher {
    identity: Identity,
    old: Option<zenkey::pattern::SubjectPattern>,
    replacement: Option<zenkey::pattern::SubjectPattern>,
}

impl Matcher {
    fn covers(&self, parsed: &zenkey::grammar::StructuralKey<'_>) -> bool {
        match &self.identity {
            Identity::Host(name) => parsed
                .producer
                .as_ref()
                .is_some_and(|p| p.name() == name.as_str()),
            Identity::Service(origin) => {
                parsed.producer.is_none() && parsed.origin.chunk() == origin.as_str()
            }
        }
    }
}

/// Walk the `[[deprecated]]` ledger of `local` and judge every entry.
///
/// - **Introspect** (fact 2) and the **admin sweep** (fact 3) run first, each
///   bounded by `timeout`; the admin sweep runs *before* the listen phase so
///   this tool's own data-plane subscriber cannot appear among the consumers
///   it is counting.
/// - **The listen window** (facts 1 and 4) runs only when `listen_s` is
///   given: no window means every wire field stays `None` — "not asked" must
///   never render as "no" (RFC 09 §5.1 O4).
pub async fn run_retired(
    session: &Session,
    base: &str,
    local: &crate::SliceSet,
    registries: Vec<String>,
    listen_s: Option<u64>,
    timeout: Duration,
) -> Result<RetiredReport> {
    // The ledger: every [[deprecated]] entry the local registries declare,
    // in a stable order.
    let mut ledger: Vec<(&RegistrySlice, &DeprecationDecl)> = local
        .slices()
        .iter()
        .flat_map(|s| s.deprecated.iter().map(move |d| (s, d)))
        .collect();
    ledger.sort_by(|(sa, da), (sb, db)| {
        (sa.name.as_str(), da.path.as_str()).cmp(&(sb.name.as_str(), db.path.as_str()))
    });

    // Fact 2's source: what live builds actually serve (RFC 08 §6).
    let served = crate::SliceSet::from_bus(session, base, timeout).await?;

    // Fact 3's source. `None` = no admin space answered, which is "not
    // available", never "nothing declared" (O4). Our own session is excluded:
    // an explorer counting itself as an unmigrated consumer would be a
    // self-inflicted finding.
    let admin = crate::admin::declared_entities(session, timeout).await?;
    let own_zid = session.zid().to_string();

    let matchers: Vec<Matcher> = ledger
        .iter()
        .map(|(slice, decl)| Matcher {
            identity: match &slice.service_origin {
                Some(origin) => Identity::Service(origin.clone()),
                None => Identity::Host(slice.name.clone()),
            },
            old: zenkey::pattern::SubjectPattern::parse(&decl.path).ok(),
            replacement: decl
                .replaced_by
                .as_deref()
                .and_then(|p| zenkey::pattern::SubjectPattern::parse(p).ok()),
        })
        .collect();

    // Facts 1 and 4: the listen window, when one was asked for.
    let new_prefix = crate::cutover::new_prefix(base);
    let mut old_counts = vec![0u64; ledger.len()];
    let mut repl_counts = vec![0u64; ledger.len()];
    let (mut plane_samples, mut dropped) = (0u64, 0u64);
    if let Some(window) = listen_s {
        let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
        let mut events = monitor.events();
        monitor.watch("**").await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(window);
        loop {
            let item = tokio::select! {
                item = events.recv() => item,
                _ = tokio::time::sleep_until(deadline) => break,
            };
            match item {
                Some(crate::StreamItem::Event(crate::FleetEvent::Sample(s))) => {
                    if s.key.starts_with(&new_prefix) {
                        plane_samples += 1;
                    }
                    let Some(parsed) = zenkey::grammar::parse_full(base, &s.key) else {
                        continue;
                    };
                    // The ledger retires data subjects; a verbatim plane has
                    // no [[subject]] surface to retire (RFC 03 §1.4).
                    if !matches!(parsed.class, zenkey::grammar::ClassOrPlane::Class(_)) {
                        continue;
                    }
                    for (i, m) in matchers.iter().enumerate() {
                        if !m.covers(&parsed) {
                            continue;
                        }
                        if m.old
                            .as_ref()
                            .is_some_and(|p| p.matches(&parsed.subject).is_some())
                        {
                            old_counts[i] += 1;
                        }
                        if m.replacement
                            .as_ref()
                            .is_some_and(|p| p.matches(&parsed.subject).is_some())
                        {
                            repl_counts[i] += 1;
                        }
                    }
                }
                Some(crate::StreamItem::Dropped(n)) => dropped += n,
                Some(_) => continue,
                None => break,
            }
        }
        monitor.stop();
    }

    let entries: Vec<RetiredEntry> = ledger
        .iter()
        .enumerate()
        .map(|(i, (slice, decl))| {
            let selector = retired_selector(slice, &decl.path);
            let wire_samples = listen_s.map(|_| old_counts[i]);
            // Fact 2: the §6.1 check — the ledger says retired, does a served
            // slice still declare the path *active*?
            let still_declared = served
                .get(&slice.name)
                .map(|served| served.serves_subject(&decl.path));
            // Fact 3: intersecting declared subscribers, when an admin space
            // answered at all.
            let subscribers = admin.as_ref().map(|entities| {
                let family = zenkey::grammar::with_base(base, &selector);
                let Ok(family) = zenoh::key_expr::KeyExpr::try_from(family) else {
                    return 0;
                };
                entities
                    .entities
                    .iter()
                    .filter(|e| e.kind == crate::EntityKind::Subscriber)
                    .filter(|e| e.node_zid != own_zid)
                    .filter(|e| {
                        zenoh::key_expr::KeyExpr::try_from(e.keyexpr.as_str())
                            .map(|k| k.intersects(&family))
                            .unwrap_or(false)
                    })
                    .count()
            });
            let replacement_samples = match (&decl.replaced_by, listen_s) {
                (Some(_), Some(_)) => Some(repl_counts[i]),
                _ => None,
            };
            // The entry's proof of life: its replacement when one is
            // declared, the v1 plane otherwise.
            let life = match &decl.replaced_by {
                Some(_) => replacement_samples,
                None => listen_s.map(|_| plane_samples),
            };
            RetiredEntry {
                producer: slice.name.clone(),
                path: decl.path.clone(),
                since: decl.since.clone(),
                replaced_by: decl.replaced_by.clone(),
                selector,
                wire_samples,
                still_declared,
                subscribers,
                replacement_samples,
                verdict: entry_verdict(wire_samples, still_declared, subscribers, life),
            }
        })
        .collect();

    let verdict = overall(&entries);
    Ok(RetiredReport {
        registries,
        entries,
        window_s: listen_s,
        plane_samples: listen_s.map(|_| plane_samples),
        dropped,
        introspect_answered: served.slices().len(),
        admin_entities: admin.as_ref().map(|e| e.entities.len()),
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(toml: &str) -> RegistrySlice {
        zenkey::parse_slice(toml).expect("fixture slice parses")
    }

    /// The issue's acceptance criterion verbatim: an entry whose replacement
    /// is silent produces `Unproven` — the existing three-state discipline,
    /// not a fourth vocabulary.
    #[test]
    fn a_silent_replacement_is_unproven_not_a_pass() {
        assert_eq!(
            entry_verdict(Some(0), Some(false), Some(0), Some(0)),
            CutoverVerdict::Unproven
        );
        // …while a speaking replacement over an observed-silent entry passes.
        assert_eq!(
            entry_verdict(Some(0), Some(false), Some(0), Some(12)),
            CutoverVerdict::Pass
        );
    }

    /// Any sign of life on the retired subject is the failure, whatever else
    /// is true — and each of the three signs fails alone.
    #[test]
    fn any_sign_of_life_beats_everything_else() {
        // Heard on the wire, even against a busy replacement.
        assert_eq!(
            entry_verdict(Some(3), Some(false), Some(0), Some(10_000)),
            CutoverVerdict::OldStillSpeaks
        );
        // Still declared active by a served slice — the §6.1 lie — with no
        // listen window at all.
        assert_eq!(
            entry_verdict(None, Some(true), None, None),
            CutoverVerdict::OldStillSpeaks
        );
        // A session still subscribed: a consumer that has not moved.
        assert_eq!(
            entry_verdict(Some(0), Some(false), Some(1), Some(12)),
            CutoverVerdict::OldStillSpeaks
        );
    }

    /// Not-asked is not "no" (RFC 09 §5.1 O4): without a listen window there
    /// is no silence observation to build a pass on.
    #[test]
    fn an_unlistened_entry_cannot_pass() {
        assert_eq!(
            entry_verdict(None, Some(false), Some(0), None),
            CutoverVerdict::Unproven
        );
        // Even introspect and admin silence on every axis proves nothing.
        assert_eq!(
            entry_verdict(None, None, None, None),
            CutoverVerdict::Unproven
        );
    }

    /// The wire family a ledger entry maps to: class unknown, so `*` — which
    /// D2/D4 keep off the verbatim planes — and the producer chunk present
    /// exactly when the origin is a host (RFC 03 §1.5).
    #[test]
    fn the_selector_states_the_family_shape() {
        let host = slice(
            "[registry]\nversion = \"2.0\"\napp = \"demo\"\nconvention = 1\n\
             [producer]\nname = \"logs\"\n",
        );
        assert_eq!(
            retired_selector(&host, "logs/errors_total"),
            "v1/*/*/logs/logs/errors_total"
        );
        // {var} widens to *, {var...} to ** — the family, not one member.
        assert_eq!(
            retired_selector(&host, "logs/by_unit/{unit}/messages_total"),
            "v1/*/*/logs/logs/by_unit/*/messages_total"
        );
        let mut svc = slice(
            "[registry]\nversion = \"2.0\"\napp = \"demo\"\nconvention = 1\n\
             [producer]\nname = \"catalog\"\n",
        );
        svc.service_origin = Some("@catalog".into());
        assert_eq!(
            retired_selector(&svc, "entity/{id}"),
            "v1/@catalog/*/entity/*"
        );
    }

    /// Worst-of: a failure outranks unproven outranks pass, and an empty
    /// ledger passes — there is nothing left that could still speak.
    #[test]
    fn the_overall_verdict_is_worst_of() {
        let entry = |verdict| RetiredEntry {
            producer: "logs".into(),
            path: "logs/errors_total".into(),
            since: None,
            replaced_by: None,
            selector: "v1/*/*/logs/logs/errors_total".into(),
            wire_samples: None,
            still_declared: None,
            subscribers: None,
            replacement_samples: None,
            verdict,
        };
        assert_eq!(overall(&[]), CutoverVerdict::Pass);
        assert_eq!(
            overall(&[entry(CutoverVerdict::Pass), entry(CutoverVerdict::Pass)]),
            CutoverVerdict::Pass
        );
        assert_eq!(
            overall(&[entry(CutoverVerdict::Pass), entry(CutoverVerdict::Unproven)]),
            CutoverVerdict::Unproven
        );
        assert_eq!(
            overall(&[
                entry(CutoverVerdict::Unproven),
                entry(CutoverVerdict::OldStillSpeaks),
                entry(CutoverVerdict::Pass),
            ]),
            CutoverVerdict::OldStillSpeaks
        );
    }

    #[test]
    fn the_scope_note_states_what_it_cannot_see() {
        let note = scope_note(16, "acme/v1/", 30);
        assert!(note.contains("30s window"));
        assert!(
            note.contains("cannot cross"),
            "a wildcard scope must not be presented as total coverage (O5): {note}"
        );
    }
}
