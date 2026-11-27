//! `desired.v1` (#840, DD) and `desired.v2` (#986's J4): `zenctl desired`'s
//! report, each row by the major it was read by.

use zenkey_fleet::report::{
    ClockGround, DesiredAnswerToken, DesiredBehindBy, DesiredCurrentState, DesiredDocument,
    DesiredListing, DesiredQuestion, DesiredReport, DesiredReportSeen,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

/// `desired.v2`'s id, as a row's `profile` names it.
const V2: &str = "desired.v2";

/// The majors a report's rows were read by, as its head line names them.
fn profiles(r: &DesiredReport) -> &'static str {
    let v2 = r.rows.iter().filter(|x| x.profile == V2).count();
    if v2 == 0 {
        "desired.v1"
    } else if v2 == r.rows.len() {
        V2
    } else {
        "desired.v1 and desired.v2"
    }
}

/// What a row shows of a report: the document as its profile writes it; a
/// `desired.v2` refusal's place and line as the contract's words beside
/// its indices, an index no table in hand names left as its number (v2
/// §2.10); a malformed `desired.v2` one as its record's fields.
fn report_words(rep: &DesiredReportSeen) -> String {
    match (&rep.document, &rep.record, &rep.unreadable) {
        (Some(d), _, _) => {
            let mut s = match d {
                DesiredDocument::V1(d) => json(d),
                DesiredDocument::V2(d) => reading_words(d),
            };
            if let Some(p) = &rep.place {
                s.push_str(&format!(" — at {p:?}"));
            }
            if let Some(l) = &rep.line {
                s.push_str(&format!(
                    "{}{l:?}",
                    if rep.place.is_some() { ": " } else { " — " }
                ));
            }
            s
        }
        (None, Some(r), _) => format!("record {}", json(r)),
        (None, None, Some(u)) => format!("unreadable: {u}"),
        (None, None, None) => "nothing".to_owned(),
    }
}

/// A `desired.v2` report's reading on one line, its members in the
/// record's order (v2 §3), a version's and a refusal's in theirs: JSON
/// all the same, and what `--format json` carries.
fn reading_words(d: &serde_json::Value) -> String {
    const ORDER: [&str; 11] = [
        "source",
        "outcome",
        "last_known",
        "observed",
        "in_force",
        "refusal",
        "version",
        "revision",
        "code",
        "pointer",
        "detail",
    ];
    let Some(map) = d.as_object() else {
        return json(d);
    };
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()));
    let members: Vec<String> = keys
        .into_iter()
        .map(|k| format!("{}:{}", json(k), reading_words(&map[k])))
        .collect();
    format!("{{{}}}", members.join(","))
}

/// One line of JSON of anything that serializes.
fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// An answer's word, as the table spells it.
fn answer_word(a: DesiredAnswerToken) -> &'static str {
    match a {
        DesiredAnswerToken::Yes => "yes",
        DesiredAnswerToken::No => "no",
        DesiredAnswerToken::NotYet => "not yet",
        DesiredAnswerToken::Unobservable => "unobservable",
        DesiredAnswerToken::NotAsked => "not asked",
    }
}

fn says(q: &DesiredQuestion) -> String {
    format!("{} ({}) — {}", answer_word(q.answer), q.reason, q.says)
}

/// `desired.v1` (#840, DD) and `desired.v2` (#986's J4): §5's four answers
/// per role, in the profile's tokens and in words. A `desired.v2` row says
/// so, shows each version as `<run>/<seq>`, never a time (v2 §5), and a
/// refusal's place and line as its target's contract's words. One row per target's role (`desired`), with what
/// was read beside the answers: the slice, the report as the target put
/// it, the author's version, an archive's last-known (never current), and
/// a target behind past the window. In the table, "has it converged?"
/// leads each row with its own mark — `✓ yes`, `✗ no` or behind, `… not
/// yet`, `? unobservable`, `— not asked` — and the other three follow as
/// lines. The run's judgement leads the envelope.
impl Render for DesiredReport {
    const FAMILY: &'static str = "desired";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["rows"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            out(Row::of("desired", r));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        t.line(format!(
            "{} in {}{}{} — {} role(s), {} reading(s)",
            profiles(self),
            namespace_words(&self.namespace),
            self.target
                .as_ref()
                .map(|a| format!(", target {a}"))
                .unwrap_or_default(),
            self.role
                .as_ref()
                .map(|r| format!(", role {r}"))
                .unwrap_or_default(),
            self.rows.len(),
            self.readings
        ));
        let mut rows = Grid::unheaded(3);
        for r in &self.rows {
            let c = &r.answers.converged;
            let (mark, st) = match (r.finding().is_some(), c.answer) {
                (true, _) => (
                    "✗",
                    style::severity(zenkey_fleet::report::DoctorSeverity::Error),
                ),
                (false, DesiredAnswerToken::Yes) => ("✓", style::PASS),
                (false, DesiredAnswerToken::No) => (
                    "✗",
                    style::severity(zenkey_fleet::report::DoctorSeverity::Error),
                ),
                (false, DesiredAnswerToken::NotYet) => ("…", style::UNPROVEN),
                (false, DesiredAnswerToken::Unobservable) => ("?", style::UNPROVEN),
                (false, DesiredAnswerToken::NotAsked) => ("—", style::UNPROVEN),
            };
            rows.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} {}", r.target, r.role)),
                Cell::text(format!("converged: {}", says(c))),
            ]);
            let mut detail = Vec::new();
            let listing = match r.listing {
                DesiredListing::Bound => "bound",
                DesiredListing::Unbound => "unbound: the kill switch",
                DesiredListing::NotListed => "not listed on a desired resource",
                DesiredListing::Unread => "its descriptor not read",
            };
            detail.push(format!(
                "    role {}{}{}",
                listing,
                r.interface
                    .as_ref()
                    .map(|i| format!(
                        ", requiring {i}{}",
                        if r.profile == V2 { " (desired.v2)" } else { "" }
                    ))
                    .unwrap_or_default(),
                r.author
                    .as_ref()
                    .map(|a| format!(", its author {}", a.address))
                    .unwrap_or_default()
            ));
            if let Some(s) = &r.slice {
                detail.push(format!(
                    "    slice {s}{}",
                    r.slice_from
                        .as_ref()
                        .map(|f| format!(" (from the {})", f.replace('_', " ")))
                        .unwrap_or_default()
                ));
            }
            if !r.declaration_invalid.is_empty() {
                detail.push(format!(
                    "    the author's declaration is invalid: {}",
                    r.declaration_invalid.join(", ")
                ));
            }
            if !r.binding_invalid.is_empty() {
                detail.push(format!(
                    "    its binding is invalid: {}",
                    r.binding_invalid.join(", ")
                ));
            }
            if let Some(by) = &r.requirement_unread {
                detail.push(format!(
                    "    its requirement is unread: no revision of {by}, which declares the role, \
                     is in hand (text 0.8)"
                ));
            }
            if let Some(rep) = &r.report {
                let what = report_words(rep);
                detail.push(format!(
                    "    report {} {what}{}",
                    if rep.differing.is_empty() {
                        rep.key.clone()
                    } else {
                        format!("{} and {}", rep.key, rep.differing.join(", "))
                    },
                    if rep.findings.is_empty() {
                        String::new()
                    } else {
                        format!(" — malformed: {}", rep.findings.join(", "))
                    }
                ));
            }
            if let Some(a) = &r.author
                && let Some(s) = &a.stamp
            {
                detail.push(format!(
                    "    its author's current version: {} {s}",
                    match a.current {
                        DesiredCurrentState::Delete | DesiredCurrentState::Tombstone => "delete",
                        _ => "put",
                    }
                ));
            }
            if let Some(a) = &r.author
                && let Some(run) = &a.run
            {
                detail.push(format!(
                    "    its author's answer: a snapshot with no entry, under run {run}"
                ));
            }
            for a in &r.archived {
                detail.push(format!(
                    "    last-known at {}: {}{} ({}) — last-known, never current",
                    a.archive,
                    if a.deleted { "deleted " } else { "" },
                    a.stamp.as_deref().unwrap_or("unstamped"),
                    if a.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    }
                ));
            }
            detail.push(format!("    refused: {}", says(&r.answers.refused)));
            detail.push(format!("    last-known: {}", says(&r.answers.last_known)));
            detail.push(format!("    in force: {}", says(&r.answers.in_force)));
            if let Some(b) = &r.behind {
                detail.push(format!(
                    "    behind {} for at least {:.1}s, by {} (text 0.4)",
                    b.version,
                    b.since_s,
                    match b.by {
                        DesiredBehindBy::Window => "readings the window apart",
                        DesiredBehindBy::Clock => "a clock trusted to the HLC delta",
                    }
                ));
            }
            rows.detail(detail);
        }
        t.grid(rows);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        match (&self.presence, self.across_face) {
            (_, true) => notes.push(
                Note::coverage(
                    "across a constrained face: presence and the descriptor do not cross, and \
                     the deployment's word stands for the target's descriptor; the contracts \
                     are the ones held offline",
                )
                .cite("desired.v1 §2.14, §2.17"),
            ),
            (Some(p), false) => {
                notes.push(
                    Note::coverage(format!(
                        "read {ns} through `{}`: {} service(s) holding an instance token; a \
                         target is one whose descriptor lists a role on a desired resource, its \
                         report found by type identity",
                        p.selector, p.services
                    ))
                    .cite("desired.v1 §2.3, §2.10"),
                );
                if let Some(why) = zenkey_fleet::model::catalog::incomplete_reason(
                    p.complete,
                    p.cut_off_during,
                    zenkey_fleet::model::catalog::RAN_OUT,
                ) {
                    notes.push(
                        Note::coverage(format!(
                            "the presence read {why}, so it is possibly incomplete: a target or \
                             an author unseen is unobservable, never absent"
                        ))
                        .cite("spec §8.1"),
                    );
                }
            }
            (None, false) => {}
        }
        notes.push(
            Note::coverage(match self.window_s {
                Some(w) => format!(
                    "{} reading(s) over a {w:.1}s window, each S4's GET of the targets' reports \
                     and of each slice from its author: a target \"not yet\" on one version of \
                     its author's for the whole window is behind",
                    self.readings
                ),
                None => "one reading, S4's GET of the targets' reports and of each slice from \
                         its author: \"not yet\" is no verdict, and no target is found behind \
                         without a window (--for)"
                    .to_owned(),
            })
            .cite("desired.v1 §2.17"),
        );
        if self.rows.iter().any(|r| r.profile == V2) {
            notes.push(
                Note::coverage(
                    "a role whose author's revision uses desired.v2 is read by it: its report \
                     found by `desired.report`, a refusal's place and line read with the target's \
                     contract, and each version its put's in-band stamp, shown <run>/<seq>; a \
                     version has no time, so only the window finds a target behind, never a clock",
                )
                .cite("desired.v2 §2.10, §2.17, §2.18"),
            );
        }
        notes.push(
            Note::caveat(match self.clock {
                ClockGround::DeploymentWord => {
                    "a report's stamp is aged against this host's clock, on the deployment's \
                     word that it and the owners' agree (--clocks-synced)"
                }
                _ => {
                    "no clock was trusted, so a report whose resource declares a horizon has an \
                     unobservable age (--clocks-synced)"
                }
            })
            .cite("freshness.v1 §2.6"),
        );
        if self.rows.iter().any(|r| !r.archived.is_empty()) {
            notes.push(
                Note::caveat(
                    "an archive's last-known of an absent author's slice is shown beside, never \
                     as its author's current document",
                )
                .cite("desired.v1 §2.17, core S6"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let count = |a: DesiredAnswerToken| {
            self.rows
                .iter()
                .filter(|r| r.answers.converged.answer == a)
                .count()
        };
        notes.push(Note::summary(format!(
            "{} role(s): converged {} yes, {} no, {} not yet, {} unobservable, {} not asked; {} \
             finding(s).",
            self.rows.len(),
            count(DesiredAnswerToken::Yes),
            count(DesiredAnswerToken::No),
            count(DesiredAnswerToken::NotYet),
            count(DesiredAnswerToken::Unobservable),
            count(DesiredAnswerToken::NotAsked),
            self.rows.iter().filter(|r| r.finding().is_some()).count()
        )));
        notes
    }

    /// What the reading put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: self.window_s,
        })
    }
}
