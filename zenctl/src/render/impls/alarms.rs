//! `alarms.v1` (#800, AC): `zenctl alarms`'s report.

use zenkey_fleet::report::{
    AlarmAnswer, AlarmAnswerToken, AlarmLatest, AlarmSetVerdict, AlarmVia, AlarmsReport,
    ClockGround, DoctorSeverity, KnownBy,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// An answer's word, as the table spells it.
fn answer_word(a: AlarmAnswerToken) -> &'static str {
    match a {
        AlarmAnswerToken::Yes => "yes",
        AlarmAnswerToken::No => "no",
        AlarmAnswerToken::Unobservable => "unobservable",
        AlarmAnswerToken::NotAsked => "not asked",
    }
}

fn says(a: &AlarmAnswer) -> String {
    format!("{} ({}) — {}", answer_word(a.answer), a.reason, a.says)
}

/// `alarms.v1` (#800, AC): §5's answers, each key's and the scope's, in the
/// profile's tokens and in words. One row per owner in scope (`owner`,
/// with how it was known and whether its alarms are known whole) and per
/// alarm key (`alarm`, with "is this alarm firing?", "is it past its
/// horizon?" and "does its key re-derive?"); each answer its own mark and
/// word in the table — `✗ firing`, `· no`, `? unobservable`, `— not
/// asked` — and its own token in a row. An archive's document rides as
/// last-known, never current. The run's judgement, §2.15's answer at the
/// floor, leads the envelope.
impl Render for AlarmsReport {
    const FAMILY: &'static str = "alarms";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["owners", "alarms"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for o in &self.owners {
            out(Row::of("owner", o));
        }
        for a in &self.alarms {
            out(Row::of("alarm", a));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        let a = &self.any_firing;
        t.line(format!(
            "alarms under {} in {} — anything at or above {}: {} ({}), {} firing; owners {} \
             established, {} unobservable, {} not asked",
            self.selector,
            namespace_words(&self.namespace),
            a.at_least,
            answer_word(a.answer),
            a.reason,
            a.firing,
            a.established,
            a.unobservable,
            a.not_asked
        ));
        let mut owners = Grid::unheaded(3);
        for o in &self.owners {
            let (mark, st) = match o.verdict {
                AlarmSetVerdict::Established => ("✓", style::PASS),
                AlarmSetVerdict::Unobservable => ("?", style::UNPROVEN),
                AlarmSetVerdict::NotAsked => ("—", style::UNPROVEN),
            };
            let known: Vec<&str> = o
                .known_by
                .iter()
                .map(|k| match k {
                    KnownBy::Presence => "presence",
                    KnownBy::Earlier => "an earlier reading",
                    KnownBy::Archive => "an archive's keys",
                    KnownBy::Word => "the deployment's word",
                })
                .collect();
            owners.row([
                Cell::styled(mark, st),
                Cell::text(&o.address),
                Cell::text(format!(
                    "{} ({}) — {}; {} alarm(s); known by {}{}",
                    o.verdict.as_str().replace('_', " "),
                    o.reason,
                    o.says,
                    o.alarms,
                    known.join(", "),
                    o.window_s
                        .map(|w| format!("; window {w:.1}s"))
                        .unwrap_or_default()
                )),
            ]);
        }
        t.grid(owners);
        let mut alarms = Grid::unheaded(3);
        for r in &self.alarms {
            let (mark, st) = match r.firing.answer {
                AlarmAnswerToken::Yes => ("✗", style::severity(DoctorSeverity::Error)),
                AlarmAnswerToken::No => ("·", style::PASS),
                AlarmAnswerToken::Unobservable => ("?", style::UNPROVEN),
                AlarmAnswerToken::NotAsked => ("—", style::UNPROVEN),
            };
            let severity = r.severity.map(|s| format!(", {s}")).unwrap_or_default();
            alarms.row([
                Cell::styled(mark, st),
                Cell::text(format!(
                    "{}/{} {}",
                    r.alarm.system, r.alarm.service, r.alarm.alert_key
                )),
                Cell::text(format!(
                    "firing: {} ({}{severity}) — {}",
                    answer_word(r.firing.answer),
                    r.firing.reason,
                    r.firing.says
                )),
            ]);
            let mut detail = Vec::new();
            if let Some(l) = &r.latest {
                let what = match l.latest {
                    AlarmLatest::Firing => "firing",
                    AlarmLatest::Resolved => "resolved",
                    AlarmLatest::Deleted => "deleted",
                    AlarmLatest::Undecodable => "undecodable",
                };
                let doc = l
                    .document
                    .as_ref()
                    .map(|d| format!(" {} {:?}", d.rule, d.summary))
                    .unwrap_or_default();
                let invalid = l
                    .invalid
                    .as_ref()
                    .map(|i| format!(" — {i}"))
                    .unwrap_or_default();
                detail.push(format!(
                    "    latest {what}{doc}{invalid} — {}{}",
                    match l.via {
                        AlarmVia::Get => "its owner's reply",
                        AlarmVia::Subscription => "the last put heard",
                    },
                    l.stamp
                        .as_ref()
                        .map(|s| format!(", stamped {} by {}", s.time, s.clock))
                        .unwrap_or_default()
                ));
            }
            detail.push(format!("    past its horizon: {}", says(&r.stale)));
            detail.push(format!("    key re-derives: {}", says(&r.rederives)));
            if !r.problems.is_empty() {
                detail.push(format!("    its document: {}", r.problems.join(", ")));
            }
            if r.foreign_stamp {
                detail.push(
                    "    its latest put's stamp is not its owner's: a foreign stamp (spec §4.2)"
                        .to_owned(),
                );
            }
            if let Some(lk) = &r.last_known {
                let doc = match (&lk.document, lk.deleted) {
                    (Some(d), _) => format!("{} {} {:?}", d.state.as_str(), d.rule, d.summary),
                    (None, true) => "deleted".to_owned(),
                    (None, false) => "undecodable".to_owned(),
                };
                detail.push(format!(
                    "    last-known at {} ({}): {doc} — last-known, never current",
                    lk.archive,
                    if lk.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    },
                ));
            }
            alarms.detail(detail);
        }
        t.grid(alarms);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        match (&self.presence, &self.face) {
            (_, Some(f)) => notes.push(
                Note::coverage(format!(
                    "across a constrained face: presence and descriptors do not cross, and the \
                     deployment's word stands for them — the face {} this reader GET the alarms",
                    if f.reachable { "lets" } else { "does not let" }
                ))
                .cite("alarms.v1 §2.11"),
            ),
            (Some(p), None) => {
                notes.push(
                    Note::coverage(format!(
                        "read {ns} through `{}`: {} service(s) in scope holding an instance \
                         token, {} of them listing alarms.v1 — found by descriptor, never by \
                         interface token",
                        p.selector, p.services, p.listing
                    ))
                    .cite("alarms.v1 §2.13"),
                );
                if !p.complete {
                    notes.push(
                        Note::coverage(
                            "the presence read ran to its timeout, so it is possibly \
                             incomplete: an owner unseen is unobservable, never absent",
                        )
                        .cite("spec §8.1"),
                    );
                }
            }
            (None, None) => {}
        }
        notes.push(
            Note::coverage(format!(
                "one GET of the owners' alarms (S4){}; the owners in scope are those presence \
                 finds, those an earlier reading knew, and those an archive's keys name",
                self.window_s
                    .map(|w| format!(
                        ", after a {w:.1}s subscription and the lease past it, which the owners \
                         were watched for"
                    ))
                    .unwrap_or_default()
            ))
            .cite("alarms.v1 §2.15"),
        );
        notes.push(
            Note::caveat(match self.clock {
                ClockGround::DeploymentWord => {
                    "a reply's stamp is aged against this host's clock, on the deployment's word \
                     that it and the owners' agree (--clocks-synced)"
                }
                ClockGround::Measured => {
                    "a reply's stamp is aged only against a clock this reading measured on a live \
                     put of the same clock"
                }
                ClockGround::None => {
                    "no clock was trusted — no window, no --clocks-synced — so a reply's age is \
                     unobservable"
                }
            })
            .cite("freshness.v1 §2.6"),
        );
        if self.lost > 0 {
            notes.push(Note::caveat(format!(
                "this reader lost {} delivery(ies) or stalled: each key is unobservable until a \
                 GET answers it",
                self.lost
            )));
        }
        if self
            .alarms
            .iter()
            .any(|a| a.stale.answer == AlarmAnswerToken::Yes)
        {
            notes.push(
                Note::caveat(
                    "a stale alarm is neither firing now nor resolved: its owner, present, has \
                     not confirmed it within 900 s",
                )
                .cite("alarms.v1 §2.7"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let a = &self.any_firing;
        notes.push(Note::summary(format!(
            "{} alarm key(s) of {} owner(s); anything firing at or above {}: {} ({}) — {}.",
            self.alarms.len(),
            self.owners.len(),
            a.at_least,
            answer_word(a.answer),
            a.reason,
            a.says
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
