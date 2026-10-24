//! `desired.v1` (#840, DD): §5's four questions of each target's role,
//! each answered in the profile's tokens (`spec/profiles/desired/v1.md`
//! §2.17, §5; `conformance/conclusions.json`).
//!
//! **Every verdict is the model's.** A role's reading
//! ([`crate::model::desired::RoleRead::reading`]) is handed to
//! `zenkey_model::desired`'s [`dm::converged`], [`dm::refused`],
//! [`dm::last_known`] and [`dm::in_force`], and their answer and reason are
//! the row's; this module adds the words, what was read beside them, and
//! the window's finding (text 0.4): a target "not yet" on one version of
//! its author's for at least the window ([`Behind`]), which leaves §5's
//! answer as it is.
//!
//! | Question | Here | Finding on |
//! |---|---|---|
//! | Has the target converged on its author's current document? | `answers.converged` | No |
//! | Is its last act refused or failed? | `answers.refused` | Yes |
//! | Is what it acted on last-known? | `answers.last_known` | Neither |
//! | Is a desired document in force? | `answers.in_force` | Neither |
//! | Has it been behind one version for the whole window? | `behind` | Yes |

use std::time::Instant;

use zenkey_model::desired::{self as dm, Answer, Current, Judged, Listing, Reason, Seen};

use crate::bus::desired::{DesiredSpec, presence_selector};
use crate::model::catalog::Catalog;
use crate::model::desired::{
    Behind, BehindBy, DesiredRead, RoleRead, RoleReading, TargetRead, current_time,
};
use crate::model::health::Trust;
use crate::report::{
    ClockGround, DesiredAnswerToken, DesiredAnswers, DesiredArchived, DesiredAuthorSeen,
    DesiredBehind, DesiredBehindBy, DesiredCurrentState, DesiredFreshness, DesiredListing,
    DesiredPresence, DesiredQuestion, DesiredReport, DesiredReportSeen, DesiredRow, FreshnessToken,
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

/// The words of one answer: what the reason means for this role, from
/// what was read.
fn says(j: Judged, r: &RoleRead, t: &TargetRead, rr: &RoleReading<'_>) -> String {
    let res = &r.resolution;
    let report = rr.report.and_then(|x| x.read.as_ref().ok());
    let observed = report
        .and_then(|d| d.observed)
        .map(|o| o.stamp.to_string())
        .unwrap_or_default();
    let current = match rr.author.current {
        Current::Put(s) => format!("its put {s}"),
        Current::Delete(s) => format!("its delete {s}"),
        _ => String::new(),
    };
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
        Reason::NoReport => rr
            .no_report
            .clone()
            .unwrap_or_else(|| "no report of the slice was read (§2.10)".to_owned()),
        Reason::NothingCrossed => {
            "across a face, no report of the slice crossed to this reader (§2.14)".to_owned()
        }
        Reason::ReportInvalid => match rr.report.map(|x| &x.read) {
            Some(Err(u)) => format!("its report does not read ({}): {u}", u.as_str()),
            Some(Ok(d)) => format!(
                "its report is malformed (§2.10): {}",
                d.check()
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => rr
                .no_report
                .clone()
                .unwrap_or_else(|| "its report does not read (§2.10)".to_owned()),
        },
        Reason::ReportStale => format!(
            "its report is stale: {} (freshness.v1)",
            rr.freshness.reason.says()
        ),
        Reason::ReportUnobservable => format!(
            "its report's age is unobservable: {}{}",
            rr.freshness.reason.says(),
            if rr.freshness.reason == zenkey_model::freshness::Reason::ClockUntrusted {
                " — a reply is aged only against a clock trusted to the HLC delta \
                 (--clocks-synced, freshness.v1 §2.6)"
            } else {
                ""
            }
        ),
        Reason::NotActed if current.is_empty() => {
            "the target has acted on nothing in this run (§2.7)".to_owned()
        }
        Reason::NotActed => format!(
            "the target has acted on nothing in this run, and {author} holds {current}: not yet"
        ),
        Reason::Converged | Reason::Reverted if j.answer == Answer::Yes && !current.is_empty() => {
            format!(
                "its report's observed {observed} is {author}'s current version, {}",
                j.reason
            )
        }
        Reason::Converged | Reason::Reverted => {
            format!("its last act, on {observed}, {}", j.reason)
        }
        Reason::Refused | Reason::Failed => {
            let refusal = report
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
                .unwrap_or_default();
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
        Reason::Current => format!("what it acted on, {observed}, is its author's own"),
        Reason::Desired => format!(
            "{} is wholly in force",
            report
                .and_then(|d| d.in_force)
                .map(|v| v.stamp.to_string())
                .unwrap_or_default()
        ),
        Reason::Partial => "a document applied in part after a failed act: no one version is \
             wholly in force (§2.7)"
            .to_owned(),
        Reason::Baseline => "its baseline is in force (§2.8, §2.10)".to_owned(),
        Reason::Call => "a change its own operation made is in force since (§2.7)".to_owned(),
        Reason::AuthorAbsent => format!(
            "{author} is absent by a complete presence read: whether the target converged on a \
             current document is not asked (§2.17 step 6); an archive's last-known is shown \
             beside, never current"
        ),
        Reason::AuthorPresenceIncomplete => format!(
            "{author}'s instance token was not seen{} (core §8.1)",
            why(&rr.author_why)
        ),
        Reason::AuthorNotRead => format!("no GET of the slice was made{}", why(&rr.author_why)),
        Reason::AuthorReadIncomplete => format!(
            "{author}'s GET of the slice could not answer{}",
            why(&rr.author_why)
        ),
        Reason::AuthorSilent => format!(
            "{author}'s GET drew nothing for the slice: no verdict (core O5); a paced author's \
             restart reads so until it has put the document again (§2.11, text 0.4)"
        ),
        Reason::Behind => format!(
            "its report's observed {observed} is earlier than {author}'s {current}, the same \
             id: not yet"
        ),
        Reason::Ahead => format!(
            "its report's observed {observed} is later than {author}'s answer, {current}: the \
             reads raced"
        ),
        Reason::OtherEpoch => format!(
            "{author}'s {current} is of another id than its observed {observed}: a new epoch \
             the target has not acted on yet (§2.6)"
        ),
    }
}

fn answer(j: Judged, r: &RoleRead, t: &TargetRead, rr: &RoleReading<'_>) -> DesiredQuestion {
    DesiredQuestion {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: says(j, r, t, rr),
    }
}

fn listing(r: &RoleRead, t: &TargetRead) -> DesiredListing {
    let l = match &t.word {
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

/// One role's row: the model's four answers, what was read, and the
/// window's finding.
#[allow(clippy::too_many_arguments)]
fn row(
    t: &TargetRead,
    r: &RoleRead,
    trust: &Trust<'_>,
    spec: &DesiredSpec,
    behind: &mut Behind,
    began: Instant,
    at: std::time::SystemTime,
) -> DesiredRow {
    let rr = r.reading(t, trust);
    let converged = dm::converged(&rr.target, &rr.author);
    let answers = DesiredAnswers {
        converged: answer(converged, r, t, &rr),
        refused: answer(dm::refused(&rr.target), r, t, &rr),
        last_known: answer(dm::last_known(&rr.target), r, t, &rr),
        in_force: answer(dm::in_force(&rr.target), r, t, &rr),
    };
    let target = t.addr.to_string();
    let role = r.resolution.role.clone();
    let version = match rr.author.current {
        Current::Put(s) | Current::Delete(s) => Some(s),
        _ => None,
    };
    behind.note(
        &target,
        &role,
        version.filter(|_| converged.answer == Answer::NotYet),
        began,
    );
    let slice = rr
        .report
        .and_then(|x| x.read.as_ref().ok())
        .and_then(|d| d.key.clone())
        .filter(|k| r.resolution.slice.contains(k))
        .or_else(|| r.resolution.slice.first().cloned());
    let found_behind = spec.window.and_then(|w| {
        let clock = spec
            .clocks_synced
            .then(|| {
                slice
                    .as_deref()
                    .and_then(|s| current_time(r.current.as_ref(), s))
            })
            .flatten()
            .map(|v| (v, at));
        behind.behind(&target, &role, w, began, clock)
    });
    let author = r.resolution.author.as_ref().map(|a| DesiredAuthorSeen {
        address: a.to_string(),
        presence: r
            .author_seen
            .as_ref()
            .map_or("incomplete", |(s, _)| seen_word(*s))
            .to_owned(),
        current: match rr.author.current {
            Current::NotRead => DesiredCurrentState::NotRead,
            Current::Incomplete => DesiredCurrentState::Incomplete,
            Current::Silent => DesiredCurrentState::Silent,
            Current::Put(_) => DesiredCurrentState::Put,
            Current::Delete(_) => DesiredCurrentState::Delete,
        },
        stamp: version.map(|s| s.to_string()),
        why: rr.author_why.clone(),
    });
    let report = rr.report.map(|x| DesiredReportSeen {
        key: x.key.clone(),
        document: x.read.as_ref().ok().cloned(),
        unreadable: x
            .read
            .as_ref()
            .err()
            .map(|u| format!("{}: {u}", u.as_str())),
        findings: x
            .read
            .as_ref()
            .map(|d| d.check().iter().map(|f| f.as_str().to_owned()).collect())
            .unwrap_or_default(),
        stamp: x.stamp.as_ref().map(crate::bus::health::Stamped::report),
        freshness: DesiredFreshness {
            verdict: match rr.freshness.verdict {
                zenkey_model::freshness::Verdict::Fresh => FreshnessToken::Fresh,
                zenkey_model::freshness::Verdict::Stale => FreshnessToken::Stale,
                zenkey_model::freshness::Verdict::Unobservable => FreshnessToken::Unobservable,
                zenkey_model::freshness::Verdict::NotAsked => FreshnessToken::NotAsked,
            },
            reason: rr.freshness.reason.as_str().to_owned(),
            says: rr.freshness.reason.says().to_owned(),
        },
    });
    DesiredRow {
        target,
        role,
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
        report,
        author,
        archived: match &r.archived {
            Some(Ok(a)) => a
                .iter()
                .map(|a| DesiredArchived {
                    archive: a.archive.to_string(),
                    deleted: a.deleted,
                    stamp: a.stamp.map(|s| s.to_string()),
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
                .map(|r| row(t, r, &trust, spec, behind, read.began, read.at))
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
        FaceWord, ReportReply, ReportsGet, RoleResolution, SliceFrom, SliceGet, SliceReply,
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
                at,
            })),
        }
    }

    fn role(bound: bool, author: Seen, current: Option<(bool, u64, &str)>) -> RoleRead {
        RoleRead {
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
                    resolution: res,
                    author_seen: Some((seen(&av["presence"]), None)),
                    current,
                    archived: None,
                }],
                reports: Some(Ok(ReportsGet {
                    selectors: vec![RP.into()],
                    complete: true,
                    replies,
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
    }
}
