//! `desired.v1` (#840, DD) and `desired.v2` (#986's J4): §5's four
//! questions of each target's role, each answered in the profile's tokens
//! (`spec/profiles/desired/v1.md` §2.17, §5, `conformance/conclusions.json`;
//! `v2.md` §2.17, §5, `conformance/v2/conclusions.json`).
//!
//! **Every verdict is the model's.** A role's reading, by the major it is
//! read by ([`crate::model::desired::RoleRead::reading`] or
//! [`crate::model::desired::RoleRead::reading_v2`]), is handed to
//! `zenkey_model::desired`'s [`dm::converged`] (or
//! `desired::v2::converged`, by run and seq), [`dm::refused`],
//! [`dm::last_known`] and [`dm::in_force`], and their answer and reason are
//! the row's; this module adds the words, what was read beside them, and
//! the window's finding (text 0.4): a target "not yet" on one version of
//! its author's for at least the window ([`Behind`]), which leaves §5's
//! answer as it is. A `desired.v2` version has no time, so only the
//! readings find such a target behind, never a clock (v2 §2.17).
//!
//! | Question | Here | Finding on |
//! |---|---|---|
//! | Has the target converged on its author's current document? | `answers.converged` | No |
//! | Is its last act refused or failed? | `answers.refused` | Yes |
//! | Is what it acted on last-known? | `answers.last_known` | Neither |
//! | Is a desired document in force? | `answers.in_force` | Neither |
//! | Has it been behind one version for the whole window? | `behind` | Yes |

use std::time::Instant;

use zenkey_model::desired::v2 as d2;
use zenkey_model::desired::{self as dm, Answer, Current, Judged, Listing, Reason, Seen};

use crate::bus::desired::{DesiredSpec, presence_selector};
use crate::model::catalog::Catalog;
use crate::model::desired::{
    AuthorVersion, Behind, BehindBy, DesiredRead, Major, RoleRead, TargetRead, current_dated,
    profile_of,
};
use crate::model::health::Trust;
use crate::report::{
    ClockGround, DesiredAnswerToken, DesiredAnswers, DesiredArchived, DesiredAuthorSeen,
    DesiredBehind, DesiredBehindBy, DesiredCurrentState, DesiredDocument, DesiredFreshness,
    DesiredListing, DesiredPresence, DesiredQuestion, DesiredReport, DesiredReportSeen, DesiredRow,
    FreshnessToken,
};

/// The answer token, as the report spells it.
pub fn token(a: Answer) -> DesiredAnswerToken {
    match a {
        Answer::Yes => DesiredAnswerToken::Yes,
        Answer::No => DesiredAnswerToken::No,
        Answer::NotYet => DesiredAnswerToken::NotYet,
        Answer::Unobservable => DesiredAnswerToken::Unobservable,
        Answer::NotAsked => DesiredAnswerToken::NotAsked,
    }
}

fn seen_word(s: Seen) -> &'static str {
    match s {
        Seen::Present => "present",
        Seen::Absent => "absent",
        Seen::Incomplete => "incomplete",
        Seen::AcrossFace => "across_face",
    }
}

/// What the words of an answer are made of, of either major: the report's
/// versions and the author's as the profile writes them (v1 §2.4:
/// `<time>/<id>`; v2 §2.4: `<run>/<seq>`), and why what was not read was
/// not.
struct Said {
    major: Major,
    /// The report's observed version, or nothing.
    observed: String,
    /// The author's answer: `put <version>`, `delete <version>`, or under
    /// `desired.v2` `snapshot with no entry under run <run>`; nothing when
    /// it gave none.
    current: String,
    /// The author answered a snapshot with no entry (v2 §2.17).
    no_entry: bool,
    /// The report holds an outcome and no observed version: a
    /// `desired.v2` revert on a snapshot with no entry (v2 §2.8).
    reverted_without_version: bool,
    /// The version wholly in force.
    in_force: String,
    /// The refusal: its code, its place and its line.
    refusal: String,
    /// Why the report does not read, or its findings when it is malformed.
    report_problem: Option<String>,
    no_report: Option<String>,
    freshness: zenkey_model::freshness::Judged,
    author_why: Option<String>,
}

/// The contract's word a `desired.v2` index names (v2 §2.10, "A reader
/// reads an index"), `None` where no table in hand names it, or the
/// target wrote `out_of_range`: the index stands as the report shows it.
fn word_of(word: d2::Word<'_>) -> Option<String> {
    match word {
        d2::Word::Entry(w) => Some(w.to_owned()),
        d2::Word::Number(_) | d2::Word::OutOfRange => None,
    }
}

/// A `desired.v2` refusal in words: its code, "the type's own" for the
/// escape (v2 §5), its place as the contract's pointer and its line as the
/// contract's words, an index no table names shown as its number.
fn refusal_v2(f: &d2::Refusal, tables: Option<&d2::Tables>) -> String {
    let mut s = format!("code {}", f.code.as_str().unwrap_or("the type's own"));
    let shown = |w: d2::Word<'_>, what: &str| match w {
        d2::Word::Entry(e) => format!("{e:?}"),
        d2::Word::Number(i) => format!("{what} {i}, which no table in hand names"),
        d2::Word::OutOfRange => format!("a {what} its table does not hold (out_of_range)"),
    };
    if let Some(p) = f.pointer {
        s.push_str(&format!(
            " at {}",
            shown(d2::word(tables.map(|t| t.pointers.as_slice()), p), "place")
        ));
    }
    if let Some(d) = f.detail {
        s.push_str(&match d2::word(tables.map(|t| t.details.as_slice()), d) {
            d2::Word::Entry(e) => format!(": {e}"),
            other => format!(": {}", shown(other, "line")),
        });
    }
    s
}

/// The words of one answer: what the reason means for this role, from
/// what was read.
fn says(j: Judged, r: &RoleRead, t: &TargetRead, said: &Said) -> String {
    let res = &r.resolution;
    let v2 = said.major == Major::V2;
    let (observed, current) = (&said.observed, &said.current);
    let author = res
        .author
        .as_ref()
        .map_or_else(|| "its author".to_owned(), |a| format!("its author {a}"));
    let why = |w: &Option<String>| w.as_ref().map(|w| format!(": {w}")).unwrap_or_default();
    match j.reason {
        Reason::TargetAbsent => {
            "no instance token of the target in a complete presence read: absent (core §8.1)"
                .to_owned()
        }
        Reason::PresenceIncomplete => format!(
            "the target's instance token was not seen{} (core §8.1, 0.37, 0.38)",
            why(&t.seen_why)
        ),
        Reason::NoDescriptor => match (&t.word, &t.descriptor) {
            (None, Err(e)) => format!("its descriptor was not read: {e}"),
            _ => "across a face, the deployment gave no word for its descriptor".to_owned(),
        },
        Reason::NotATarget => format!(
            "its descriptor lists no role {} on a desired resource (§2.3)",
            res.role
        ),
        Reason::Unbound => format!(
            "its descriptor lists role {} with no binding: the kill switch, its baseline in \
             force (§2.12)",
            res.role
        ),
        Reason::NoReport => said.no_report.clone().unwrap_or_else(|| {
            if v2 {
                "no report of the role was read (desired.v2 §2.10)".to_owned()
            } else {
                "no report of the slice was read (§2.10)".to_owned()
            }
        }),
        Reason::NothingCrossed if v2 => {
            "across a face, no report of the role crossed to this reader (§2.14)".to_owned()
        }
        Reason::NothingCrossed => {
            "across a face, no report of the slice crossed to this reader (§2.14)".to_owned()
        }
        Reason::ReportInvalid => said
            .report_problem
            .clone()
            .or_else(|| said.no_report.clone())
            .unwrap_or_else(|| "its report does not read (§2.10)".to_owned()),
        Reason::ReportStale => format!(
            "its report is stale: {} (freshness.v1)",
            said.freshness.reason.says()
        ),
        Reason::ReportUnobservable => format!(
            "its report's age is unobservable: {}{}",
            said.freshness.reason.says(),
            if said.freshness.reason == zenkey_model::freshness::Reason::ClockUntrusted {
                " — a reply is aged only against a clock trusted to the HLC delta \
                 (--clocks-synced, freshness.v1 §2.6)"
            } else {
                ""
            }
        ),
        Reason::NotActed if current.is_empty() && v2 => {
            "the target has acted on nothing in this process (§2.7)".to_owned()
        }
        Reason::NotActed if current.is_empty() => {
            "the target has acted on nothing in this run (§2.7)".to_owned()
        }
        Reason::NotActed if said.reverted_without_version => {
            format!(
                "its report names no version, after a revert on a snapshot with no entry; \
                 {author}'s current version is its {current}: not yet"
            )
        }
        Reason::NotActed => format!(
            "the target has acted on nothing in this {}; {author}'s current version is its \
             {current}: not yet",
            if v2 { "process" } else { "run" }
        ),
        Reason::Reverted if j.answer == Answer::Yes && said.no_entry => format!(
            "its last act reverted on {observed}, and {author} answers a {current}, the run of \
             that version: it holds no document, reverted (desired.v2 §2.17)"
        ),
        Reason::Converged | Reason::Reverted if j.answer == Answer::Yes && !current.is_empty() => {
            format!(
                "its report's observed {observed} is {author}'s current version, {}",
                j.reason
            )
        }
        Reason::Converged | Reason::Reverted if observed.is_empty() => {
            format!("its last act {}, on a snapshot with no entry", j.reason)
        }
        Reason::Converged | Reason::Reverted => {
            format!("its last act, on {observed}, {}", j.reason)
        }
        Reason::Refused | Reason::Failed => {
            let refusal = &said.refusal;
            if j.answer == Answer::No && !current.is_empty() {
                format!(
                    "{author}'s current version, {observed}, was {} by the target: {refusal}",
                    j.reason
                )
            } else {
                format!("its last act, on {observed}, was {}: {refusal}", j.reason)
            }
        }
        Reason::LastKnown => format!(
            "what it acted on, {observed}, came from an archive: last-known, its author did not \
             answer for it (§2.9)"
        ),
        Reason::Current if observed.is_empty() => {
            "what it acted on is its author's own word, a snapshot with no entry".to_owned()
        }
        Reason::Current => format!("what it acted on, {observed}, is its author's own"),
        Reason::Desired => format!("{} is wholly in force", said.in_force),
        Reason::Partial => "a document applied in part after a failed act: no one version is \
             wholly in force (§2.7)"
            .to_owned(),
        Reason::Baseline => "its baseline is in force (§2.8, §2.10)".to_owned(),
        Reason::Call => "a change its own operation made is in force since (§2.7)".to_owned(),
        Reason::AuthorAbsent if v2 && res.slice.is_empty() => format!(
            "{author} is absent by a complete presence read: whether the target converged on a \
             current document is not asked (§2.17 step 6); with no revision of its interface in \
             hand the role has no slice, so no archive's last-known is read (desired.v2 §2.17)"
        ),
        Reason::AuthorAbsent => format!(
            "{author} is absent by a complete presence read: whether the target converged on a \
             current document is not asked (§2.17 step 6); an archive's last-known is shown \
             beside, never current"
        ),
        Reason::AuthorPresenceIncomplete => format!(
            "{author}'s instance token was not seen{} (core §8.1)",
            why(&said.author_why)
        ),
        Reason::AuthorNotRead => format!("no GET of the slice was made{}", why(&said.author_why)),
        Reason::AuthorReadIncomplete => format!(
            "{author}'s GET of the slice could not answer{}",
            why(&said.author_why)
        ),
        Reason::AuthorSilent if said.no_entry => format!(
            "{author} answers a {current}, {}: it restarted, is pacing, or deleted the document, \
             and a tool cannot tell (desired.v2 §2.17)",
            if observed.is_empty() {
                "and the report names no observed version to compare its run with".to_owned()
            } else {
                format!("a run its report's observed {observed} does not name")
            }
        ),
        Reason::AuthorSilent if v2 => {
            format!("{author}'s GET drew no reply for the slice: no verdict (core O5)")
        }
        Reason::AuthorSilent => format!(
            "{author}'s GET drew nothing for the slice: no verdict (core O5); a paced author's \
             restart reads so until it has put the document again (§2.11, text 0.4)"
        ),
        Reason::Behind if said.no_entry => format!(
            "{author} answers a {current}, the run of its report's observed {observed}: it holds \
             no document, deleted beyond its window, and the target has not reverted: not yet \
             (desired.v2 §2.17)"
        ),
        Reason::Behind if v2 => format!(
            "its report's observed {observed} is a lower seq than {author}'s current version, \
             its {current}, of the same run: not yet"
        ),
        Reason::Behind => format!(
            "its report's observed {observed} is earlier than {author}'s current version, its \
             {current}, the same id: not yet"
        ),
        Reason::Ahead => format!(
            "its report's observed {observed} is later than {author}'s answer, its {current}: \
             the reads raced"
        ),
        Reason::OtherEpoch if v2 => format!(
            "{author}'s current version, its {current}, is of another run than its observed \
             {observed}: a run the target has not acted on yet (desired.v2 §2.6)"
        ),
        Reason::OtherEpoch => format!(
            "{author}'s current version, its {current}, is of another id than its observed \
             {observed}: a new epoch the target has not acted on yet (§2.6)"
        ),
    }
}

fn answer(j: Judged, r: &RoleRead, t: &TargetRead, said: &Said) -> DesiredQuestion {
    DesiredQuestion {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: says(j, r, t, said),
    }
}

fn listing(r: &RoleRead, t: &TargetRead) -> DesiredListing {
    let l = match &t.word {
        // A revision in hand shows the role on no desired resource,
        // whatever the deployment's word (text 0.6, F-217).
        Some(_) if r.resolution.listing == Some(Listing::NotListed) => Some(Listing::NotListed),
        Some(w) => Some(w.listing),
        None => r.resolution.listing.filter(|_| t.descriptor.is_ok()),
    };
    match l {
        Some(Listing::Listed { bound: true }) => DesiredListing::Bound,
        Some(Listing::Listed { bound: false }) => DesiredListing::Unbound,
        Some(Listing::NotListed) => DesiredListing::NotListed,
        None => DesiredListing::Unread,
    }
}

fn freshness_of(f: &zenkey_model::freshness::Judged) -> DesiredFreshness {
    DesiredFreshness {
        verdict: match f.verdict {
            zenkey_model::freshness::Verdict::Fresh => FreshnessToken::Fresh,
            zenkey_model::freshness::Verdict::Stale => FreshnessToken::Stale,
            zenkey_model::freshness::Verdict::Unobservable => FreshnessToken::Unobservable,
            zenkey_model::freshness::Verdict::NotAsked => FreshnessToken::NotAsked,
        },
        reason: f.reason.as_str().to_owned(),
        says: f.reason.says().to_owned(),
    }
}

/// What one role's reading gave, of either major, before the row is made.
struct Judged4 {
    answers: [Judged; 4],
    said: Said,
    /// What the author answered, when it answered a version or no entry.
    version: Option<AuthorVersion>,
    current: DesiredCurrentState,
    stamp: Option<String>,
    run: Option<String>,
    report: Option<DesiredReportSeen>,
    /// The slice the row names.
    slice: Option<String>,
}

/// One `desired.v1` role's reading, judged by the model (§2.17).
fn judged_v1(t: &TargetRead, r: &RoleRead, trust: &Trust<'_>) -> Judged4 {
    let rr = r.reading(t, trust);
    let doc = rr.report.and_then(|x| x.read.as_ref().ok());
    let version = match rr.author.current {
        Current::Put(s) | Current::Delete(s) => Some(s),
        _ => None,
    };
    let said = Said {
        major: Major::V1,
        observed: doc
            .and_then(|d| d.observed)
            .map(|o| o.stamp.to_string())
            .unwrap_or_default(),
        current: match rr.author.current {
            Current::Put(s) => format!("put {s}"),
            Current::Delete(s) => format!("delete {s}"),
            _ => String::new(),
        },
        no_entry: false,
        reverted_without_version: false,
        in_force: doc
            .and_then(|d| d.in_force)
            .map(|v| v.stamp.to_string())
            .unwrap_or_default(),
        refusal: doc
            .and_then(|d| d.refusal.as_ref())
            .map(|f| {
                let mut s = format!("code {}", f.code);
                if let Some(p) = &f.pointer {
                    s.push_str(&format!(" at {p:?}"));
                }
                if let Some(d) = &f.detail {
                    s.push_str(&format!(": {d}"));
                }
                s
            })
            .unwrap_or_default(),
        report_problem: match rr.report.map(|x| &x.read) {
            Some(Err(u)) => Some(format!("its report does not read ({}): {u}", u.as_str())),
            Some(Ok(d)) => Some(format!(
                "its report is malformed (§2.10): {}",
                d.check()
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            None => None,
        },
        no_report: rr.no_report.clone(),
        freshness: rr.freshness,
        author_why: rr.author_why.clone(),
    };
    let freshness = freshness_of(&rr.freshness);
    let report = match (rr.report, rr.differing.split_first()) {
        (Some(x), _) => Some(DesiredReportSeen {
            key: x.key.clone(),
            document: x.read.as_ref().ok().cloned().map(DesiredDocument::V1),
            record: None,
            place: None,
            line: None,
            unreadable: x
                .read
                .as_ref()
                .err()
                .map(|u| format!("{}: {u}", u.as_str())),
            differing: vec![],
            findings: x
                .read
                .as_ref()
                .map(|d| d.check().iter().map(|f| f.as_str().to_owned()).collect())
                .unwrap_or_default(),
            stamp: x.stamp.as_ref().map(crate::bus::health::Stamped::report),
            inband: None,
            freshness,
        }),
        // Two that claim the slice and differ are unreadable, each named
        // (§2.10, text 0.5, F-212), never as if none were read: `key` the
        // first claimant's, `document` none, the others in `differing`.
        (None, Some((first, rest))) => Some(DesiredReportSeen {
            key: first.key.clone(),
            document: None,
            record: None,
            place: None,
            line: None,
            unreadable: Some(format!(
                "two_differ: {}",
                rr.no_report.as_deref().unwrap_or(
                    "readable reports of two report resources claim the slice and differ (§2.10)"
                )
            )),
            differing: rest.iter().map(|x| x.key.clone()).collect(),
            findings: vec![],
            stamp: None,
            inband: None,
            freshness,
        }),
        (None, None) => None,
    };
    Judged4 {
        answers: [
            dm::converged(&rr.target, &rr.author),
            dm::refused(&rr.target),
            dm::last_known(&rr.target),
            dm::in_force(&rr.target),
        ],
        said,
        version: version.map(AuthorVersion::V1),
        current: match rr.author.current {
            Current::NotRead => DesiredCurrentState::NotRead,
            Current::Incomplete => DesiredCurrentState::Incomplete,
            Current::Silent => DesiredCurrentState::Silent,
            Current::Put(_) => DesiredCurrentState::Put,
            Current::Delete(_) => DesiredCurrentState::Delete,
        },
        stamp: version.map(|s| s.to_string()),
        run: None,
        report,
        slice: doc
            .and_then(|d| d.key.clone())
            .filter(|k| r.resolution.slice.contains(k))
            .or_else(|| r.resolution.slice.first().cloned()),
    }
}

/// One `desired.v2` role's reading, judged by the model (v2 §2.17): the
/// report found by `desired.report`, the author's version its snapshot's,
/// compared by run and seq.
fn judged_v2(t: &TargetRead, r: &RoleRead, trust: &Trust<'_>) -> Judged4 {
    use d2::Current as C;
    let rr = r.reading_v2(t, trust);
    let read = rr.report.and_then(|x| x.read.as_ref().ok());
    let doc = read.and_then(|x| x.report.as_ref());
    let text = |o: Option<d2::Observed>| o.map(|o| d2::version_text(o.version)).unwrap_or_default();
    let (version, current_state, stamp, run) = match rr.author.current {
        C::NotRead => (None, DesiredCurrentState::NotRead, None, None),
        C::Incomplete => (None, DesiredCurrentState::Incomplete, None, None),
        C::Unstamped => (None, DesiredCurrentState::Unstamped, None, None),
        C::Silent => (None, DesiredCurrentState::Silent, None, None),
        C::Entry(v) => (
            Some(AuthorVersion::V2(v)),
            DesiredCurrentState::Entry,
            Some(d2::version_text(v)),
            None,
        ),
        C::Tombstone(v) => (
            Some(AuthorVersion::V2(v)),
            DesiredCurrentState::Tombstone,
            Some(d2::version_text(v)),
            None,
        ),
        C::NoEntry(run) => (
            Some(AuthorVersion::NoEntry(run)),
            DesiredCurrentState::NoEntry,
            None,
            Some(run.to_string()),
        ),
    };
    let said = Said {
        major: Major::V2,
        observed: text(doc.and_then(|d| d.observed)),
        current: match rr.author.current {
            C::Entry(v) => format!("put {}", d2::version_text(v)),
            C::Tombstone(v) => format!("delete {}", d2::version_text(v)),
            C::NoEntry(run) => format!("snapshot with no entry under run {run}"),
            _ => String::new(),
        },
        no_entry: matches!(rr.author.current, C::NoEntry(_)),
        reverted_without_version: doc
            .is_some_and(|d| d.observed.is_none() && d.outcome != d2::Outcome::NotActed),
        in_force: text(doc.and_then(|d| d.in_force)),
        refusal: doc
            .and_then(|d| d.refusal.as_ref())
            .map(|f| refusal_v2(f, rr.tables))
            .unwrap_or_default(),
        report_problem: match rr.report.map(|x| &x.read) {
            Some(Err(u)) => Some(format!(
                "its report is unreadable ({}): {} (desired.v2 §2.10, core §7.6)",
                u.token, u.says
            )),
            Some(Ok(x)) => Some(format!(
                "its report is malformed (desired.v2 §2.10): {}",
                x.findings
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            None => None,
        },
        no_report: rr.no_report.clone(),
        freshness: rr.freshness,
        author_why: rr.author_why.clone(),
    };
    let freshness = freshness_of(&rr.freshness);
    let words = |f: &d2::Refusal| {
        (
            f.pointer
                .and_then(|p| word_of(d2::word(rr.tables.map(|t| t.pointers.as_slice()), p))),
            f.detail
                .and_then(|d| word_of(d2::word(rr.tables.map(|t| t.details.as_slice()), d))),
        )
    };
    let report = match rr.report {
        Some(x) => {
            let (place, line) = doc
                .and_then(|d| d.refusal.as_ref())
                .map(words)
                .unwrap_or((None, None));
            Some(DesiredReportSeen {
                key: x.key.clone(),
                document: doc.map(|d| DesiredDocument::V2(d.reading())),
                // A malformed report is shown as its record's fields, as
                // core §7.6 reads them, never as the profile's reading.
                record: read
                    .filter(|x| x.report.is_none())
                    .map(|x| x.fields.to_json()),
                place,
                line,
                unreadable: x.read.as_ref().err().map(ToString::to_string),
                differing: vec![],
                findings: read
                    .map(|x| x.findings.iter().map(|f| f.as_str().to_owned()).collect())
                    .unwrap_or_default(),
                stamp: x.stamp.as_ref().map(crate::bus::health::Stamped::report),
                inband: x.inband.map(d2::version_text),
                freshness,
            })
        }
        // Two resources name the role: unreadable (v2 §2.10), and no key
        // is the report's.
        None if rr.two_reports => r.resolution.report_at.as_ref().map(|w| DesiredReportSeen {
            key: format!("zk2/{}/{}", t.addr, w.iface),
            document: None,
            record: None,
            place: None,
            line: None,
            unreadable: Some(format!(
                "two_reports: {}",
                rr.no_report.as_deref().unwrap_or_default()
            )),
            differing: vec![],
            findings: vec![],
            stamp: None,
            inband: None,
            freshness,
        }),
        None => None,
    };
    Judged4 {
        answers: [
            d2::converged(&rr.target, &rr.author),
            dm::refused(&rr.target),
            dm::last_known(&rr.target),
            dm::in_force(&rr.target),
        ],
        said,
        version,
        current: current_state,
        stamp,
        run,
        report,
        slice: r.resolution.slice.first().cloned(),
    }
}

/// One role's row: the model's four answers, what was read, and the
/// window's finding. A version's age is taken at the time its slice's GET
/// was answered, never the reading's end (review of #856), and only a
/// `desired.v1` version has one: a `desired.v2` role is behind by the
/// window alone (v2 §2.17).
fn row(
    t: &TargetRead,
    r: &RoleRead,
    trust: &Trust<'_>,
    spec: &DesiredSpec,
    behind: &mut Behind,
    began: Instant,
) -> DesiredRow {
    let j = match r.major {
        Major::V1 => judged_v1(t, r, trust),
        Major::V2 => judged_v2(t, r, trust),
    };
    let [converged, refused, last_known, in_force] = j.answers;
    let answers = DesiredAnswers {
        converged: answer(converged, r, t, &j.said),
        refused: answer(refused, r, t, &j.said),
        last_known: answer(last_known, r, t, &j.said),
        in_force: answer(in_force, r, t, &j.said),
    };
    let target = t.addr.to_string();
    let role = r.resolution.role.clone();
    behind.note(
        &target,
        &role,
        j.version.filter(|_| converged.answer == Answer::NotYet),
        began,
    );
    let slice = j.slice;
    let found_behind = spec.window.and_then(|w| {
        let clock = (spec.clocks_synced && r.major == Major::V1)
            .then(|| {
                slice
                    .as_deref()
                    .and_then(|s| current_dated(r.current.as_ref(), s))
            })
            .flatten();
        behind.behind(&target, &role, w, began, clock)
    });
    let author = r.resolution.author.as_ref().map(|a| DesiredAuthorSeen {
        address: a.to_string(),
        presence: r
            .author_seen
            .as_ref()
            .map_or("incomplete", |(s, _)| seen_word(*s))
            .to_owned(),
        current: j.current,
        stamp: j.stamp,
        run: j.run,
        why: j.said.author_why.clone(),
    });
    DesiredRow {
        target,
        role,
        profile: profile_of(r.major).to_owned(),
        listing: listing(r, t),
        interface: r.resolution.interface.as_ref().map(ToString::to_string),
        resource: r.resolution.resource.clone(),
        slice,
        slice_from: r.resolution.slice_from.map(|f| f.as_str().to_owned()),
        declaration_invalid: r
            .resolution
            .declaration_invalid
            .iter()
            .map(|i| i.as_str().to_owned())
            .collect(),
        binding_invalid: r
            .resolution
            .binding_problems
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect(),
        requirement_unread: r.resolution.requirement_unread.clone(),
        report: j.report,
        author,
        archived: match &r.archived {
            Some(Ok(a)) => a
                .iter()
                .map(|a| DesiredArchived {
                    archive: a.archive.to_string(),
                    deleted: a.deleted,
                    // An archive's record keeps the envelope (core §4.4): a
                    // desired.v2 document's version is its in-band stamp.
                    stamp: match r.major {
                        Major::V1 => a.stamp.map(|s| s.to_string()),
                        Major::V2 => a.inband.map(d2::version_text),
                    },
                    revision: a.revision,
                    confirmed: a.confirmed,
                })
                .collect(),
            _ => Vec::new(),
        },
        answers,
        behind: found_behind.map(|b| DesiredBehind {
            version: b.version.to_string(),
            since_s: b.since.as_secs_f64(),
            by: match b.by {
                BehindBy::Window => DesiredBehindBy::Window,
                BehindBy::Clock => DesiredBehindBy::Clock,
            },
        }),
    }
}

/// One reading judged: every role's row, the reading's scope, and why it
/// answers nothing when it does not. `behind` follows the roles across a
/// run's readings; `start` is the run's, `readings` how many were made.
pub fn judge(
    namespace: &str,
    target: Option<&zenkey_model::grammar::Addr>,
    spec: &DesiredSpec,
    read: &DesiredRead,
    behind: &mut Behind,
    readings: u32,
) -> DesiredReport {
    let trust = if spec.clocks_synced {
        Trust::Word
    } else {
        Trust::None
    };
    let rows: Vec<DesiredRow> = read
        .targets
        .iter()
        .flat_map(|t| {
            t.roles
                .iter()
                .map(|r| row(t, r, &trust, spec, behind, read.began))
                .collect::<Vec<_>>()
        })
        .collect();
    let presence = match &read.presence {
        Some(Ok(o)) => Some(DesiredPresence {
            selector: o.selector.clone(),
            complete: o.complete,
            cut_off_during: o.cut_off_during,
            services: {
                let c = Catalog::new(o);
                c.addresses().filter(|a| c.has_instance(a)).count()
            },
        }),
        _ => None,
    };
    let unobservable = match (&read.cut_off, &read.presence) {
        (Some(why), _) => Some(format!("no reading was made: {why}")),
        (None, Some(Err(e))) if target.is_none() => {
            Some(format!("the presence read could not be made: {e}"))
        }
        (None, Some(Ok(o))) if rows.is_empty() && !o.complete => Some(format!(
            "no target of desired.v1 found, and the presence read of `{}` {}",
            presence_selector(target),
            o.incomplete_reason("ended at its timeout")
                .unwrap_or("is possibly incomplete")
        )),
        _ => None,
    };
    DesiredReport {
        namespace: namespace.to_owned(),
        target: target.map(ToString::to_string),
        role: spec.role.clone(),
        presence,
        across_face: spec.face.is_some(),
        window_s: spec.window.map(|w| w.as_secs_f64()),
        readings,
        clock: if spec.clocks_synced {
            ClockGround::DeploymentWord
        } else {
            ClockGround::None
        },
        asked: read.asked.clone(),
        rows,
        unobservable,
    }
}

#[cfg(test)]
mod tests {
    //! `conclusions.json`'s readings, through the fleet's projection: a
    //! role read so answers as the fixture does.

    use super::*;
    use crate::model::desired::{
        FaceWord, MemberReply, PackedReply, ReportReply, ReportsGet, RoleResolution, SliceFrom,
        SliceGet, SliceReply,
    };
    use std::time::{Duration, SystemTime};
    use zenkey_model::freshness::Horizon;
    use zenkey_model::grammar::Addr;
    use zenkey_model::seed::Stamp;

    const D: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";
    const RP: &str = "zk2/lab/worker/worker.v1/state/desired/task";

    fn addr(s: &str) -> Addr {
        s.parse().unwrap()
    }

    fn resolution(bound: bool) -> RoleResolution {
        let mut r = RoleResolution::bare("task", Some(Listing::Listed { bound }));
        r.interface = Some("tasking.v1".parse().unwrap());
        if bound {
            r.author = Some(addr("lab/author"));
            r.slice = vec![D.to_owned()];
            r.slice_from = Some(SliceFrom::AuthorContract);
        }
        r
    }

    fn target(report: Option<&str>, role: RoleRead) -> TargetRead {
        let at = SystemTime::now();
        TargetRead {
            addr: addr("lab/worker"),
            seen: Seen::Present,
            seen_why: None,
            word: None,
            descriptor: Ok(Some(
                serde_json::from_value(serde_json::json!({
                    "format": "zk2-descriptor/0.1", "service": "lab/worker",
                    "instance": "0123456789abcdef", "interfaces": []
                }))
                .unwrap(),
            )),
            roles: vec![role],
            reports: Some(Ok(ReportsGet {
                selectors: vec![RP.into()],
                complete: true,
                replies: report
                    .map(|r| ReportReply {
                        key: RP.into(),
                        read: dm::decode(r.as_bytes()),
                        stamp: None,
                        horizon: Horizon::Undeclared,
                    })
                    .into_iter()
                    .collect(),
                packed: vec![],
                at,
            })),
        }
    }

    fn role(bound: bool, author: Seen, current: Option<(bool, u64, &str)>) -> RoleRead {
        RoleRead {
            major: crate::model::desired::Major::V1,
            resolution: resolution(bound),
            author_seen: bound.then_some((author, None)),
            current: current.map(|(delete, time, id)| {
                Ok(SliceGet {
                    selector: D.into(),
                    complete: true,
                    replies: vec![SliceReply {
                        key: D.into(),
                        delete,
                        stamp: Some((Stamp::parse(time, id).unwrap(), SystemTime::now())),
                    }],
                    member: vec![],
                    at: SystemTime::now(),
                })
            }),
            archived: None,
        }
    }

    fn judged(t: &TargetRead, spec: &DesiredSpec) -> DesiredRow {
        let read = DesiredRead {
            began: Instant::now(),
            at: SystemTime::now(),
            cut_off: None,
            presence: None,
            targets: vec![t.clone()],
            asked: vec![],
        };
        let mut b = Behind::default();
        judge("", Some(&t.addr), spec, &read, &mut b, 1)
            .rows
            .remove(0)
    }

    fn tokens(r: &DesiredRow) -> [String; 4] {
        let a = &r.answers;
        [&a.converged, &a.refused, &a.last_known, &a.in_force]
            .map(|x| format!("{}:{}", x.answer.as_str(), x.reason))
    }

    /// `conclusions.json`'s first cases, through the projection: converged,
    /// refused, failed, behind, last-known with its author absent, and the
    /// kill switch.
    #[test]
    fn a_role_answers_as_the_fixture_does() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let converged = format!(
            r#"{{"key":"{D}","source":"desired","in_force":{{"stamp":"100/a1","revision":1}},"observed":{{"stamp":"100/a1","revision":1}},"outcome":"converged"}}"#
        );
        let t = target(
            Some(&converged),
            role(true, Seen::Present, Some((false, 100, "a1"))),
        );
        assert_eq!(
            tokens(&judged(&t, &spec)),
            ["yes:converged", "no:converged", "no:current", "yes:desired"]
        );
        let t = target(
            Some(&converged),
            role(true, Seen::Present, Some((false, 200, "a1"))),
        );
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "not_yet:behind");
        assert!(r.answers.converged.says.contains("100/a1"), "{r:?}");
        let refused = format!(
            r#"{{"key":"{D}","source":"desired","in_force":{{"stamp":"100/a1","revision":1}},"observed":{{"stamp":"200/a1","revision":2}},"outcome":"refused","refusal":{{"code":"reach","pointer":"/endpoint"}}}}"#
        );
        let t = target(
            Some(&refused),
            role(true, Seen::Present, Some((false, 200, "a1"))),
        );
        let r = judged(&t, &spec);
        assert_eq!(
            tokens(&r),
            ["no:refused", "yes:refused", "no:current", "yes:desired"]
        );
        assert!(r.answers.refused.says.contains("\"/endpoint\""), "{r:?}");
        let failed = format!(
            r#"{{"key":"{D}","source":"desired","observed":{{"stamp":"200/a1","revision":2}},"outcome":"failed","refusal":{{"code":"apply"}}}}"#
        );
        let t = target(
            Some(&failed),
            role(true, Seen::Present, Some((false, 200, "a1"))),
        );
        assert_eq!(
            tokens(&judged(&t, &spec)),
            ["no:failed", "yes:failed", "no:current", "yes:partial"]
        );
        let last_known = format!(
            r#"{{"key":"{D}","source":"desired","in_force":{{"stamp":"100/a1","revision":1}},"observed":{{"stamp":"100/a1","revision":1}},"outcome":"converged","last_known":true}}"#
        );
        let t = target(Some(&last_known), role(true, Seen::Absent, None));
        assert_eq!(
            tokens(&judged(&t, &spec)),
            [
                "not_asked:author_absent",
                "no:converged",
                "yes:last_known",
                "yes:desired"
            ]
        );
        let t = target(
            Some(r#"{"source":"baseline"}"#),
            role(false, Seen::Present, None),
        );
        assert_eq!(
            tokens(&judged(&t, &spec)),
            [
                "not_asked:unbound",
                "not_asked:unbound",
                "not_asked:unbound",
                "no:unbound"
            ]
        );
    }

    /// A requirement unread (text 0.8, F-225; round 62's measured shape):
    /// its declaring contract named and not in hand, the author's revision
    /// declaring two desired resources. No binding problem, no slice, so
    /// no report is read: unobservable, `no_report`, the declaring contract
    /// beside as `requirement_unread`, never `two_documents`.
    #[test]
    fn a_requirement_unread_is_no_report_and_no_binding_problem() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let mut ro = role(true, Seen::Present, Some((false, 100, "a1")));
        ro.resolution.slice.clear();
        ro.resolution.slice_from = None;
        ro.resolution.requirement_unread = Some("zk2py_worker9.v1".into());
        let t = target(Some(&format!(r#"{{"key":"{D}","source":"baseline"}}"#)), ro);
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "unobservable:no_report");
        assert!(r.binding_invalid.is_empty(), "{r:?}");
        assert_eq!(r.requirement_unread.as_deref(), Some("zk2py_worker9.v1"));
        assert!(r.report.is_none(), "no report read without a slice");
        assert!(
            r.answers.converged.says.contains("requirement is unread"),
            "{:?}",
            r.answers.converged
        );
        assert_eq!(r.listing, DesiredListing::Bound);
    }

    /// Two readable reports, of two report resources, that claim the slice
    /// and differ: unreadable, `report_invalid`, and the row's report says
    /// so with both keys, never as if none were read (§2.10, text 0.5,
    /// F-212; zk2py round 59's row comparison).
    #[test]
    fn two_reports_that_differ_are_unreadable_in_the_row() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let mut t = target(
            Some(&format!(r#"{{"key":"{D}","source":"baseline"}}"#)),
            role(true, Seen::Present, Some((false, 100, "a1"))),
        );
        let other = "zk2/lab/worker/worker2.v1/state/desired/task";
        if let Some(Ok(g)) = &mut t.reports {
            g.selectors.push(other.into());
            g.replies.push(ReportReply {
                key: other.into(),
                read: dm::decode(format!(r#"{{"key":"{D}","source":"call"}}"#).as_bytes()),
                stamp: None,
                horizon: Horizon::Undeclared,
            });
        }
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "unobservable:report_invalid");
        let rep = r.report.expect("the row carries the two, unreadable");
        assert_eq!(rep.key, RP);
        assert_eq!(rep.differing, [other]);
        assert!(rep.document.is_none());
        let why = rep.unreadable.expect("unreadable");
        assert!(why.starts_with("two_differ: "), "{why}");
        assert!(why.contains(other), "{why}");
        assert!(r.answers.converged.says.contains(other), "{:?}", r.answers);
    }

    /// The premises: no report, an unreadable one, the target absent, a
    /// descriptor not read, the author silent, and across a face the
    /// deployment's word.
    #[test]
    fn the_premises_answer_first() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let t = target(None, role(true, Seen::Present, Some((false, 100, "a1"))));
        assert_eq!(tokens(&judged(&t, &spec))[0], "unobservable:no_report");
        let t = target(Some("{"), role(true, Seen::Present, None));
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "unobservable:report_invalid");
        assert!(r.report.unwrap().unreadable.unwrap().starts_with("json"));
        let mut t = target(None, role(true, Seen::Present, None));
        t.seen = Seen::Absent;
        assert_eq!(tokens(&judged(&t, &spec))[3], "not_asked:target_absent");
        let mut t = target(None, role(true, Seen::Present, None));
        t.descriptor = Err("silent".into());
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "unobservable:no_descriptor");
        assert_eq!(r.listing, DesiredListing::Unread);
        let baseline = format!(r#"{{"key":"{D}","source":"baseline"}}"#);
        let mut ro = role(true, Seen::Present, None);
        ro.current = Some(Ok(SliceGet {
            selector: D.into(),
            complete: true,
            replies: vec![],
            member: vec![],
            at: SystemTime::now(),
        }));
        let t = target(Some(&baseline), ro);
        assert_eq!(
            tokens(&judged(&t, &spec)),
            [
                "unobservable:author_silent",
                "not_asked:not_acted",
                "not_asked:not_acted",
                "no:baseline"
            ]
        );
        let mut t = target(None, role(true, Seen::AcrossFace, None));
        t.seen = Seen::AcrossFace;
        t.descriptor = Ok(None);
        t.word = Some(FaceWord {
            listing: Listing::Listed { bound: false },
            author: None,
            params: Default::default(),
            author_across: true,
        });
        assert_eq!(
            tokens(&judged(&t, &spec)),
            [
                "not_asked:unbound",
                "not_asked:unbound",
                "not_asked:unbound",
                "no:unbound"
            ]
        );
    }

    /// Every reading of `conclusions.json` (§2.17, §5), built as values the
    /// bus would have read, answers through the fleet's projection exactly
    /// as the fixture does: the fleet decides nothing the model does not.
    #[test]
    fn every_conclusion_reads_as_the_fixture() {
        use serde_json::Value;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/desired/conformance/conclusions.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let seen = |v: &Value| match v.as_str().unwrap() {
            "present" => Seen::Present,
            "absent" => Seen::Absent,
            "incomplete" => Seen::Incomplete,
            "across_face" => Seen::AcrossFace,
            other => panic!("{other}"),
        };
        let listing = |d: &Value| match d {
            Value::Null => None,
            d if d["lists"] == false => Some(Listing::NotListed),
            d => Some(Listing::Listed {
                bound: d["bound"] == true,
            }),
        };
        let cases = doc["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 35);
        for case in cases {
            let tv = &case["reading"]["target"];
            let av = &case["reading"]["author"];
            let presence = seen(&tv["presence"]);
            let (word, descriptor_listing) = if presence == Seen::AcrossFace {
                (
                    listing(&tv["deployment"]).map(|l| FaceWord {
                        listing: l,
                        author: Some(addr("lab/author")),
                        params: Default::default(),
                        author_across: true,
                    }),
                    None,
                )
            } else {
                (None, listing(&tv["descriptor"]))
            };
            let bound = matches!(
                word.as_ref().map(|w| w.listing).or(descriptor_listing),
                Some(Listing::Listed { bound: true })
            );
            let mut res = resolution(bound);
            res.listing = word.as_ref().map(|w| w.listing).or(descriptor_listing);
            let now = SystemTime::now();
            let fresh = tv["report_freshness"].as_str().unwrap();
            let horizon = if fresh == "not_asked" {
                Horizon::Undeclared
            } else {
                Horizon::Within(Duration::from_secs(900))
            };
            let stamp = (fresh != "not_asked").then(|| crate::bus::health::Stamped {
                time: if fresh == "stale" {
                    now - Duration::from_secs(2000)
                } else {
                    now
                },
                clock: "a1".into(),
            });
            let replies: Vec<ReportReply> = match &tv["report"] {
                Value::Null => vec![],
                Value::String(s) if s == "undecodable" => vec![ReportReply {
                    key: RP.into(),
                    read: dm::decode(b"not json"),
                    stamp: stamp.clone(),
                    horizon: horizon.clone(),
                }],
                r => vec![ReportReply {
                    key: RP.into(),
                    read: dm::decode(r["document"].to_string().as_bytes()),
                    stamp: stamp.clone(),
                    horizon: horizon.clone(),
                }],
            };
            let current = match &av["current"] {
                Value::Null => None,
                Value::String(s) => Some(Ok(SliceGet {
                    selector: D.into(),
                    complete: s != "incomplete",
                    replies: vec![],
                    member: vec![],
                    at: now,
                })),
                c => {
                    let (delete, st) = match (c.get("put"), c.get("delete")) {
                        (Some(p), _) => (false, p),
                        (_, Some(d)) => (true, d),
                        _ => panic!("{c}"),
                    };
                    let stamp =
                        Stamp::parse(st["time"].as_u64().unwrap(), st["id"].as_str().unwrap())
                            .unwrap();
                    Some(Ok(SliceGet {
                        selector: D.into(),
                        complete: true,
                        replies: vec![SliceReply {
                            key: D.into(),
                            delete,
                            stamp: Some((stamp, now)),
                        }],
                        member: vec![],
                        at: now,
                    }))
                }
            };
            let t = TargetRead {
                addr: addr("lab/worker"),
                seen: presence,
                seen_why: None,
                word,
                descriptor: match (presence, descriptor_listing) {
                    (Seen::AcrossFace, _) | (_, None) => Err("not read".into()),
                    _ => Ok(None),
                },
                roles: vec![RoleRead {
                    major: crate::model::desired::Major::V1,
                    resolution: res,
                    author_seen: Some((seen(&av["presence"]), None)),
                    current,
                    archived: None,
                }],
                reports: Some(Ok(ReportsGet {
                    selectors: vec![RP.into()],
                    complete: true,
                    replies,
                    packed: vec![],
                    at: now,
                })),
            };
            let mut spec = DesiredSpec::new(Duration::from_secs(1));
            spec.clocks_synced = fresh != "unobservable";
            let r = judged(&t, &spec);
            let got = serde_json::json!({
                "converged": {"answer": r.answers.converged.answer.as_str(), "reason": r.answers.converged.reason},
                "refused": {"answer": r.answers.refused.answer.as_str(), "reason": r.answers.refused.reason},
                "last_known": {"answer": r.answers.last_known.answer.as_str(), "reason": r.answers.last_known.reason},
                "in_force": {"answer": r.answers.in_force.answer.as_str(), "reason": r.answers.in_force.reason},
            });
            assert_eq!(got, case["expect"], "{}", case["note"]);
        }
    }

    // ─── desired.v2 (#986's J4) ───────────────────────────────────────

    const D2K: &str = "zk2/lab/author/tasking.v2/state/tasks/lab/worker";
    const RP2: &str = "zk2/lab/worker/worker.v2/state/desired/task";

    fn worker_v2() -> zenkey_model::contract::Contract {
        let l = zenkey_model::contract::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/worker.v2.toml"),
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// A desired.v2 role of `lab/worker`, as the bus resolves it with the
    /// author's revision in hand: its slice, and its report on the
    /// scenarios' `Rp`, read with `worker.v2`'s tables.
    fn role_v2(bound: bool, author: Seen, member: Option<Vec<MemberReply>>) -> RoleRead {
        let mut res = resolution(bound);
        res.interface = Some("tasking.v2".parse().unwrap());
        res.author_major = Some(Major::V2);
        res.report_at = Some(crate::model::desired::ReportWhere::of(
            &addr("lab/worker"),
            &worker_v2(),
            "task",
        ));
        if bound {
            res.slice = vec![D2K.to_owned()];
        }
        let mut r = RoleRead::new(res);
        r.author_seen = bound.then_some((author, None));
        r.current = member.map(|member| {
            Ok(SliceGet {
                selector: D2K.into(),
                complete: true,
                replies: vec![],
                member,
                at: SystemTime::now(),
            })
        });
        r
    }

    /// The target with one reply on `Rp`: a record under a tag.
    fn target_v2(report: Option<(u8, Vec<u8>)>, role: RoleRead) -> TargetRead {
        let mut t = target(None, role);
        if let Some(Ok(g)) = &mut t.reports {
            g.selectors = vec![RP2.into()];
            g.packed = report
                .map(|(tag, bytes)| PackedReply {
                    key: RP2.into(),
                    read: crate::model::desired::packed_read(Some(tag), &bytes),
                    inband: None,
                    stamp: None,
                    horizon: Horizon::Undeclared,
                })
                .into_iter()
                .collect();
        }
        t
    }

    fn version(text: &str) -> zenkey_model::wire::Stamp {
        d2::parse_version(text).expect("a version")
    }

    fn observed(text: &str, revision: u64) -> Option<d2::Observed> {
        Some(d2::Observed {
            version: version(text),
            revision: Some(revision),
        })
    }

    fn record(r: &d2::Report) -> Option<(u8, Vec<u8>)> {
        Some((d2::TAG, r.write().expect("a record").bytes))
    }

    /// A desired.v2 role (v2 §2.17, §5): converged by run and seq, its
    /// versions written `<run>/<seq>`; a refusal's place and line as the
    /// contract's words, an index past the table as its number, and
    /// `out_of_range` said so; a record core §7.6 refuses unreadable with
    /// its token; a malformed one shown as its record.
    #[test]
    fn a_v2_role_answers_in_the_contracts_words() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let converged = d2::Report {
            source: dm::Source::Desired,
            outcome: d2::Outcome::Converged,
            last_known: false,
            observed: observed("0007/12", 2),
            in_force: observed("0007/12", 2),
            refusal: None,
        };
        let entry = |v: &str| Some(vec![MemberReply::Entry(version(v))]);
        let t = target_v2(
            record(&converged),
            role_v2(true, Seen::Present, entry("0007/12")),
        );
        let r = judged(&t, &spec);
        assert_eq!(r.profile, "desired.v2");
        assert_eq!(
            tokens(&r),
            ["yes:converged", "no:converged", "no:current", "yes:desired"]
        );
        assert_eq!(r.slice.as_deref(), Some(D2K));
        let a = r.author.as_ref().unwrap();
        assert_eq!(
            (a.current, a.stamp.as_deref()),
            (DesiredCurrentState::Entry, Some("0007/12"))
        );
        assert!(r.answers.converged.says.contains("0007/12"), "{r:?}");
        let rep = r.report.as_ref().unwrap();
        assert_eq!(rep.key, RP2);
        let doc = serde_json::to_value(rep.document.as_ref().unwrap()).unwrap();
        assert_eq!(doc["observed"]["version"], "0007/12", "{doc}");
        // Refused over a document in force: the place and the line read
        // with worker.v2's tables.
        let refused = |pointer, detail| d2::Report {
            source: dm::Source::Desired,
            outcome: d2::Outcome::Refused,
            last_known: false,
            observed: observed("0007/13", 3),
            in_force: observed("0007/12", 2),
            refusal: Some(d2::Refusal {
                code: d2::Code::Reach,
                pointer,
                detail,
            }),
        };
        let at = |i| Some(d2::Index::At(i));
        let t = target_v2(
            record(&refused(at(3), at(1))),
            role_v2(true, Seen::Present, entry("0007/13")),
        );
        let r = judged(&t, &spec);
        assert_eq!(
            tokens(&r),
            ["no:refused", "yes:refused", "no:current", "yes:desired"]
        );
        let rep = r.report.as_ref().unwrap();
        assert_eq!(rep.place.as_deref(), Some("/endpoint"));
        assert_eq!(
            rep.line.as_deref(),
            Some("names how the worker reaches the bus")
        );
        let says = &r.answers.refused.says;
        assert!(
            says.contains(r#"code reach at "/endpoint": names how the worker reaches the bus"#),
            "{says}"
        );
        assert!(r.answers.in_force.says.starts_with("0007/12 "), "{r:?}");
        // An index no table names is shown as its number, out_of_range
        // said so, and the type's own code named so.
        let mut own = refused(at(40), Some(d2::Index::OutOfRange));
        own.refusal.as_mut().unwrap().code = d2::Code::Own;
        let t = target_v2(record(&own), role_v2(true, Seen::Present, entry("0007/13")));
        let r = judged(&t, &spec);
        let rep = r.report.as_ref().unwrap();
        assert_eq!((rep.place.as_deref(), rep.line.as_deref()), (None, None));
        let doc = serde_json::to_value(rep.document.as_ref().unwrap()).unwrap();
        assert_eq!(doc["refusal"]["pointer"], 40);
        assert_eq!(doc["refusal"]["detail"], "out_of_range");
        let says = &r.answers.refused.says;
        assert!(says.contains("code the type's own at place 40"), "{says}");
        assert!(says.contains("out_of_range"), "{says}");
        // A record of another tag: unreadable, `tag`, nothing answered.
        let bytes = converged.write().unwrap().bytes;
        let t = target_v2(
            Some((3, bytes)),
            role_v2(true, Seen::Present, entry("0007/12")),
        );
        let r = judged(&t, &spec);
        assert_eq!(
            tokens(&r),
            [
                "unobservable:report_invalid",
                "unobservable:report_invalid",
                "unobservable:report_invalid",
                "unobservable:report_invalid"
            ]
        );
        let rep = r.report.as_ref().unwrap();
        assert!(rep.document.is_none() && rep.record.is_none());
        assert!(rep.unreadable.as_ref().unwrap().starts_with("tag: "));
        assert!(r.answers.converged.says.contains("unreadable (tag)"));
        // A malformed one: its record's fields shown, its findings beside.
        let t = target_v2(
            Some((d2::TAG, vec![0x04, 0, 0])),
            role_v2(true, Seen::Present, entry("0007/12")),
        );
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "unobservable:report_invalid");
        let rep = r.report.as_ref().unwrap();
        assert_eq!(rep.findings, ["last_known"]);
        assert!(rep.document.is_none());
        assert_eq!(rep.record.as_ref().unwrap()["last_known"], true);
    }

    /// desired.v2 §2.17: a snapshot with no entry answers only under the
    /// run of the report's observed version, behind or reverted; under
    /// another run it is `author_silent`; and with no revision of the
    /// author's in hand the role has no slice, so no GET: not asked where
    /// the author is absent, else `author_not_read`, the other three
    /// answered from the report.
    #[test]
    fn a_v2_snapshot_with_no_entry_and_a_role_with_no_slice() {
        let spec = DesiredSpec::new(Duration::from_secs(1));
        let converged = d2::Report {
            source: dm::Source::Desired,
            outcome: d2::Outcome::Converged,
            last_known: false,
            observed: observed("0007/12", 1),
            in_force: observed("0007/12", 1),
            refusal: None,
        };
        let no_entry = |run: &str| {
            Some(vec![MemberReply::NoEntry {
                run: version(&format!("{run}/0")).run,
                asof: version("0007/20").seq,
            }])
        };
        let t = target_v2(
            record(&converged),
            role_v2(true, Seen::Present, no_entry("0007")),
        );
        let r = judged(&t, &spec);
        assert_eq!(tokens(&r)[0], "not_yet:behind");
        let a = r.author.as_ref().unwrap();
        assert_eq!(
            (a.current, a.run.as_deref(), a.stamp.as_deref()),
            (DesiredCurrentState::NoEntry, Some("0007"), None)
        );
        assert!(
            r.answers
                .converged
                .says
                .contains("snapshot with no entry under run 0007"),
            "{r:?}"
        );
        let t = target_v2(
            record(&converged),
            role_v2(true, Seen::Present, no_entry("0008")),
        );
        let r = judged(&t, &spec);
        assert_eq!(
            tokens(&r),
            [
                "unobservable:author_silent",
                "no:converged",
                "no:current",
                "yes:desired"
            ]
        );
        assert!(
            r.answers.converged.says.contains("a tool cannot tell"),
            "{r:?}"
        );
        // No revision of the author's interface in hand: no slice.
        for (seen, want) in [
            (Seen::Present, "unobservable:author_not_read"),
            (Seen::Absent, "not_asked:author_absent"),
        ] {
            let mut ro = role_v2(true, seen, None);
            ro.resolution.author_major = None;
            ro.resolution.slice = vec![D2K.to_owned()];
            ro.resolution.slice_from = Some(SliceFrom::Requirement);
            let mut t = target_v2(record(&converged), ro);
            let TargetRead { roles, reports, .. } = &mut t;
            roles[0].settle(reports.as_ref());
            assert_eq!(roles[0].major, Major::V2);
            let r = judged(&t, &spec);
            assert_eq!(
                tokens(&r),
                [want, "no:converged", "no:current", "yes:desired"],
                "{seen:?}"
            );
            assert_eq!((r.slice.as_deref(), r.slice_from.as_deref()), (None, None));
            if seen == Seen::Present {
                assert!(
                    r.answers
                        .converged
                        .says
                        .contains("no revision of its author's interface is in hand"),
                    "{r:?}"
                );
            }
        }
    }

    /// Every reading of `v2/conclusions.json` (desired.v2 §2.17, §5), built
    /// as values the bus would have read, answers through the fleet's
    /// projection exactly as the fixture does: the fleet decides nothing
    /// the model does not.
    #[test]
    fn every_v2_conclusion_reads_as_the_fixture() {
        use serde_json::Value;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/desired/conformance/v2/conclusions.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let seen = |v: &Value| match v.as_str().unwrap() {
            "present" => Seen::Present,
            "absent" => Seen::Absent,
            "incomplete" => Seen::Incomplete,
            "across_face" => Seen::AcrossFace,
            other => panic!("{other}"),
        };
        let listing = |d: &Value| match d {
            Value::Null => None,
            d if d["lists"] == false => Some(Listing::NotListed),
            d => Some(Listing::Listed {
                bound: d["bound"] == true,
            }),
        };
        let unhex = |h: &str| -> Vec<u8> {
            (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                .collect()
        };
        let cases = doc["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 39);
        for case in cases {
            let tv = &case["reading"]["target"];
            let av = &case["reading"]["author"];
            let presence = seen(&tv["presence"]);
            let (word, descriptor_listing) = if presence == Seen::AcrossFace {
                (
                    listing(&tv["deployment"]).map(|l| FaceWord {
                        listing: l,
                        author: Some(addr("lab/author")),
                        params: Default::default(),
                        author_across: true,
                    }),
                    None,
                )
            } else {
                (None, listing(&tv["descriptor"]))
            };
            let now = SystemTime::now();
            let fresh = tv["age"].as_str();
            let horizon = match fresh {
                None => Horizon::Undeclared,
                Some(_) => Horizon::Within(Duration::from_secs(900)),
            };
            let stamp = fresh.map(|f| crate::bus::health::Stamped {
                time: if f == "stale" {
                    now - Duration::from_secs(2000)
                } else {
                    now
                },
                clock: "a1".into(),
            });
            // "not_made" is no GET: with a slice, a GET never made; the
            // fixture's other answers are a GET's.
            let get = |complete: bool, member: Vec<MemberReply>| {
                Some(Ok(SliceGet {
                    selector: D2K.into(),
                    complete,
                    replies: vec![],
                    member,
                    at: now,
                }))
            };
            let run = |v: &Value| version(&format!("{}/0", v.as_str().unwrap())).run;
            let v = |x: &Value| version(x.as_str().unwrap());
            let current = match &av["get"] {
                Value::String(s) => match s.as_str() {
                    "not_made" => None,
                    "timeout" => get(false, vec![]),
                    "silent" => get(true, vec![]),
                    "unstamped" => get(true, vec![MemberReply::Unstamped]),
                    other => panic!("{other}"),
                },
                g => match (g.get("entry"), g.get("tombstone"), g.get("no_entry")) {
                    (Some(x), _, _) => get(true, vec![MemberReply::Entry(v(x))]),
                    (_, Some(x), _) => get(true, vec![MemberReply::Tombstone(v(x))]),
                    (_, _, Some(r)) => get(
                        true,
                        vec![MemberReply::NoEntry {
                            run: run(r),
                            asof: version("0000/0").seq,
                        }],
                    ),
                    _ => panic!("{g}"),
                },
            };
            let mut ro = role_v2(true, seen(&av["presence"]), None);
            ro.current = current;
            ro.resolution.listing = word.as_ref().map(|w| w.listing).or(descriptor_listing);
            let mut t = target_v2(None, ro);
            t.seen = presence;
            t.word = word;
            t.descriptor = match (presence, descriptor_listing) {
                (Seen::AcrossFace, _) | (_, None) => Err("not read".into()),
                _ => Ok(None),
            };
            if let (Some(Ok(g)), r) = (&mut t.reports, &tv["report"])
                && !r.is_null()
            {
                g.packed = vec![PackedReply {
                    key: RP2.into(),
                    read: crate::model::desired::packed_read(
                        Some(u8::try_from(r["tag"].as_u64().unwrap()).unwrap()),
                        &unhex(r["hex"].as_str().unwrap()),
                    ),
                    inband: None,
                    stamp,
                    horizon,
                }];
            }
            let mut spec = DesiredSpec::new(Duration::from_secs(1));
            spec.clocks_synced = fresh != Some("unobservable");
            let r = judged(&t, &spec);
            let got = serde_json::json!({
                "converged": {"answer": r.answers.converged.answer.as_str(), "reason": r.answers.converged.reason},
                "refused": {"answer": r.answers.refused.answer.as_str(), "reason": r.answers.refused.reason},
                "last_known": {"answer": r.answers.last_known.answer.as_str(), "reason": r.answers.last_known.reason},
                "in_force": {"answer": r.answers.in_force.answer.as_str(), "reason": r.answers.in_force.reason},
            });
            assert_eq!(got, case["expect"], "{}", case["note"]);
            // Every word is said: no answer's words are empty.
            for q in [
                &r.answers.converged,
                &r.answers.refused,
                &r.answers.last_known,
                &r.answers.in_force,
            ] {
                assert!(!q.says.is_empty(), "{}: {q:?}", case["note"]);
            }
        }
    }

    /// desired.v2 §2.17, scenarios-v2 §9 step 6: a version has no time, so
    /// one reading finds nothing whatever clock it trusts, and readings a
    /// window apart find the target behind, by the window.
    #[test]
    fn a_v2_role_is_behind_by_the_window_whatever_clock_is_trusted() {
        let mut spec = DesiredSpec::new(Duration::from_secs(1));
        spec.window = Some(Duration::from_secs(2));
        spec.clocks_synced = true;
        let converged = d2::Report {
            source: dm::Source::Desired,
            outcome: d2::Outcome::Converged,
            last_known: false,
            observed: observed("0007/5", 5),
            in_force: observed("0007/5", 5),
            refusal: None,
        };
        let t = target_v2(
            record(&converged),
            role_v2(
                true,
                Seen::Present,
                Some(vec![MemberReply::Entry(version("0007/6"))]),
            ),
        );
        let t0 = Instant::now();
        let mut b = Behind::default();
        let read = |began| DesiredRead {
            began,
            at: SystemTime::now(),
            cut_off: None,
            presence: None,
            targets: vec![t.clone()],
            asked: vec![],
        };
        let first = judge("", Some(&t.addr), &spec, &read(t0), &mut b, 1);
        assert_eq!(tokens(&first.rows[0])[0], "not_yet:behind");
        assert!(first.rows[0].behind.is_none(), "one reading finds nothing");
        assert_eq!(crate::report::judgement_exit_code(&first.judgement()), 2);
        let later = judge(
            "",
            Some(&t.addr),
            &spec,
            &read(t0 + Duration::from_secs(2)),
            &mut b,
            2,
        );
        let behind = later.rows[0].behind.clone().unwrap();
        assert_eq!(behind.by, DesiredBehindBy::Window);
        assert_eq!(behind.version, "0007/6");
        assert_eq!(crate::report::judgement_exit_code(&later.judgement()), 1);
    }

    /// With a window, a role "not yet" on one version at readings the
    /// window apart is behind past it; with a trusted clock, a version
    /// older than the window is at once.
    #[test]
    fn behind_past_the_window() {
        let mut spec = DesiredSpec::new(Duration::from_secs(1));
        spec.window = Some(Duration::from_secs(5));
        let converged = format!(
            r#"{{"key":"{D}","source":"desired","in_force":{{"stamp":"100/a1","revision":1}},"observed":{{"stamp":"100/a1","revision":1}},"outcome":"converged"}}"#
        );
        let t = target(
            Some(&converged),
            role(true, Seen::Present, Some((false, 200, "a1"))),
        );
        let t0 = Instant::now();
        let mut b = Behind::default();
        let read = |began| DesiredRead {
            began,
            at: SystemTime::now(),
            cut_off: None,
            presence: None,
            targets: vec![t.clone()],
            asked: vec![],
        };
        let first = judge("", Some(&t.addr), &spec, &read(t0), &mut b, 1);
        assert!(first.rows[0].behind.is_none());
        assert!(!first.settled());
        let later = judge(
            "",
            Some(&t.addr),
            &spec,
            &read(t0 + Duration::from_secs(5)),
            &mut b,
            2,
        );
        let behind = later.rows[0].behind.clone().unwrap();
        assert_eq!(behind.by, DesiredBehindBy::Window);
        assert_eq!(behind.version, "200/a1");
        assert_eq!(crate::report::judgement_exit_code(&later.judgement()), 1);
        // A trusted clock: the author's put stamped long ago.
        spec.clocks_synced = true;
        let mut ro = role(true, Seen::Present, None);
        ro.current = Some(Ok(SliceGet {
            selector: D.into(),
            complete: true,
            replies: vec![SliceReply {
                key: D.into(),
                delete: false,
                stamp: Some((
                    Stamp::parse(200, "a1").unwrap(),
                    SystemTime::now() - Duration::from_secs(60),
                )),
            }],
            member: vec![],
            at: SystemTime::now(),
        }));
        let t = target(Some(&converged), ro);
        let mut b = Behind::default();
        let r = judge(
            "",
            Some(&t.addr),
            &spec,
            &DesiredRead {
                began: t0,
                at: SystemTime::now(),
                cut_off: None,
                presence: None,
                targets: vec![t.clone()],
                asked: vec![],
            },
            &mut b,
            1,
        );
        assert_eq!(
            r.rows[0].behind.as_ref().unwrap().by,
            DesiredBehindBy::Clock
        );
        // The age is the slice GET's, never the reading's end (review of
        // #856): a GET answered a second after the put dates the version a
        // second old, however late the reading ends.
        let put = SystemTime::now() - Duration::from_secs(60);
        let mut ro = role(true, Seen::Present, None);
        ro.current = Some(Ok(SliceGet {
            selector: D.into(),
            complete: true,
            replies: vec![SliceReply {
                key: D.into(),
                delete: false,
                stamp: Some((Stamp::parse(200, "a1").unwrap(), put)),
            }],
            member: vec![],
            at: put + Duration::from_secs(1),
        }));
        let t = target(Some(&converged), ro);
        let mut b = Behind::default();
        let r = judge(
            "",
            Some(&t.addr),
            &spec,
            &DesiredRead {
                began: t0,
                at: SystemTime::now(),
                cut_off: None,
                presence: None,
                targets: vec![t.clone()],
                asked: vec![],
            },
            &mut b,
            1,
        );
        assert!(r.rows[0].behind.is_none(), "{:?}", r.rows[0].behind);
    }
}
