//! `telemetry.v1` (#819, TC): `zenctl telemetry`'s report.

use zenkey_fleet::report::{
    ContinuityToken, DeclarationToken, DoctorSeverity, EpochToken, PairPresenceToken,
    TelemetryMember, TelemetryPair, TelemetryReport,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// A member's kind and unit, as §2.5 has them: a unit absent is unknown,
/// never dimensionless.
fn kind_unit(m: &TelemetryMember) -> String {
    let kind = match (&m.kind, &m.kind_from) {
        (Some(k), Some(f)) if f == "point" => format!("{k} (the Point's)"),
        (Some(k), _) => k.clone(),
        (None, _) => "kind unobservable".to_owned(),
    };
    let unit = match (&m.unit, &m.unit_from) {
        (Some(u), Some(f)) if f == "point" => format!("unit {u} (the Point's)"),
        (Some(u), _) => format!("unit {u}"),
        (None, _) => "unit unknown".to_owned(),
    };
    format!("{kind}, {unit}")
}

/// One pair in words: its values, its continuity and how its epoch was had,
/// and its rate or why it has none.
fn pair_words(p: &TelemetryPair) -> String {
    let continuity = match p.continuity {
        ContinuityToken::SameEpoch => "same_epoch",
        ContinuityToken::Reset => "reset",
        ContinuityToken::Decreased => "decreased",
        ContinuityToken::Unobservable => "unobservable (incomparable)",
        ContinuityToken::NotAsked => "not asked",
    };
    let epoch = match (p.epoch, p.via.as_deref()) {
        (Some(EpochToken::Established), Some("presence")) => ", established by presence".to_owned(),
        (Some(EpochToken::Established), Some(v)) => {
            format!(", established {}", v.replace('_', "-"))
        }
        (Some(EpochToken::Assumed), _) => match p.presence {
            PairPresenceToken::SpansLoss => ", assumed (a loss between them)".to_owned(),
            _ => ", assumed".to_owned(),
        },
        _ => String::new(),
    };
    let rate = match (p.rate, &p.no_rate) {
        (Some(r), _) => format!(
            "; {r} {} by {} over {:.3}s",
            p.rate_unit.as_deref().unwrap_or("/s (unit unknown)"),
            p.base,
            p.elapsed_s
        ),
        (None, Some(why)) if why != "kind" => format!("; no rate ({why})"),
        _ => String::new(),
    };
    format!("{} → {}  {continuity}{epoch}{rate}", p.earlier, p.later)
}

/// `telemetry.v1` (#819, TC): §5's answers in the profile's tokens. One row
/// per metric resource (`resource`, with its declaration) and per member
/// (`member`, with its kind and unit, its samples counted by conformance,
/// its pairs and its latest rate); each its own mark in the table — `✗` a
/// finding (a sample that does not conform, a decrease within an
/// established epoch), `·` answered clean, `?` unobservable — and every
/// pair's epoch said as established in-band, by presence, or assumed. The
/// run's judgement leads the envelope.
impl Render for TelemetryReport {
    const FAMILY: &'static str = "telemetry";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["resources", "members"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.resources {
            out(Row::of("resource", r));
        }
        for m in &self.members {
            out(Row::of("member", m));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        let x = &self.totals;
        t.line(format!(
            "telemetry under {} in {} — {} metric resource(s), {} member(s): {} sample(s), {} \
             conform, {} do not; {} pair(s), {} decreased ({} established, {} assumed)",
            self.selector.as_deref().unwrap_or("every metric resource"),
            namespace_words(&self.namespace),
            x.resources,
            x.members,
            x.samples,
            x.conforms,
            x.does_not,
            x.pairs,
            x.decreased,
            x.established,
            x.assumed
        ));
        let mut resources = Grid::unheaded(3);
        for r in &self.resources {
            let (mark, st) = match r.declaration.answer {
                DeclarationToken::Declared => ("✓", style::PASS),
                DeclarationToken::Invalid => ("!", style::severity(DoctorSeverity::Warning)),
            };
            resources.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} {} {}", r.address, r.iface, r.resource)),
                Cell::text(format!(
                    "{}; {} member(s) heard",
                    r.declaration.says, r.members
                )),
            ]);
        }
        t.grid(resources);
        let mut members = Grid::unheaded(3);
        for m in &self.members {
            let s = &m.samples;
            let c = &m.continuity;
            let (mark, st) = if s.does_not > 0 || c.decreased > 0 {
                ("✗", style::severity(DoctorSeverity::Error))
            } else if s.conforms == 0 {
                ("?", style::UNPROVEN)
            } else {
                ("·", style::PASS)
            };
            let mut what = format!(
                "{}; {} sample(s), {} conform",
                kind_unit(m),
                s.received,
                s.conforms
            );
            if s.does_not > 0 {
                what.push_str(&format!(
                    ", {} do not ({})",
                    s.does_not,
                    s.problems
                        .iter()
                        .map(|(p, n)| format!("{p} ×{n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if s.unobservable > 0 {
                what.push_str(&format!(
                    ", {} unobservable ({})",
                    s.unobservable,
                    s.unobservable_reason.as_deref().unwrap_or("unobservable")
                ));
            }
            if s.not_kept > 0 {
                what.push_str(&format!(
                    ", {} not kept by the late joiner's merge",
                    s.not_kept
                ));
            }
            if let Some(v) = &m.latest {
                what.push_str(&format!("; latest {v}"));
            }
            if let Some(r) = &m.rate {
                what.push_str(&format!(
                    "; rate {} {}",
                    r.per_s,
                    r.unit.as_deref().unwrap_or("/s (unit unknown)")
                ));
            }
            members.row([Cell::styled(mark, st), Cell::text(&m.key), Cell::text(what)]);
            let mut detail = Vec::new();
            if c.pairs > 0 {
                detail.push(format!(
                    "    {} pair(s): {} same_epoch, {} reset, {} decreased, {} incomparable, {} \
                     not asked — epochs {} in-band, {} by presence, {} assumed",
                    c.pairs,
                    c.same_epoch,
                    c.reset,
                    c.decreased,
                    c.unobservable,
                    c.not_asked,
                    c.in_band,
                    c.by_presence,
                    c.assumed
                ));
            }
            for p in &m.pairs {
                detail.push(format!("    {}", pair_words(p)));
            }
            if !s.latest_does_not.is_empty() {
                detail.push(format!(
                    "    the latest that did not conform: {}",
                    s.latest_does_not.join(", ")
                ));
            }
            members.detail(detail);
        }
        t.grid(members);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        match (&self.presence, self.across_face) {
            (_, true) => notes.push(
                Note::coverage(
                    "across a constrained face: presence and descriptors do not cross, so the \
                     metric resources are the contracts in hand, on the deployment's word, and \
                     only an in-band epoch establishes one",
                )
                .cite("telemetry.v1 §2.14"),
            ),
            (Some(p), false) => {
                notes.push(
                    Note::coverage(format!(
                        "read {ns} through `{}`: {} service(s) in scope holding an instance \
                         token; presence {} at the window's end",
                        p.selector, p.services, p.basis
                    ))
                    .cite("telemetry.v1 §2.9"),
                );
                if !p.complete {
                    notes.push(
                        Note::coverage(
                            "the presence seed ran to its timeout, so it is possibly \
                             incomplete: no pair has presence, and a counter's epoch is \
                             established in-band or assumed",
                        )
                        .cite("spec §8.1"),
                    );
                }
            }
            (None, false) => {}
        }
        notes.push(
            Note::coverage(format!(
                "a {:.1}s window: samples and token moves folded in the one order received; a \
                 state member's pairs in the order the late joiner's merge keeps them",
                self.window_s
            ))
            .cite("telemetry.v1 §2.10"),
        );
        if self.members.iter().any(|m| {
            m.pairs.iter().any(|p| {
                p.continuity == ContinuityToken::Decreased && p.via.as_deref() == Some("presence")
            })
        }) {
            notes.push(
                Note::caveat(
                    "a decrease judged by presence: a sample in transit across a re-mint can \
                     arrive after the old instance's tokens are gone and read as the new one's; \
                     an owner that writes an in-band epoch closes the risk",
                )
                .cite("telemetry.v1 §2.9"),
            );
        }
        if self.totals.assumed > 0 {
            notes.push(
                Note::caveat(format!(
                    "{} pair(s) had their epoch assumed: a decrease there reads as a reset, never \
                     a finding or a negative rate",
                    self.totals.assumed
                ))
                .cite("telemetry.v1 §2.10"),
            );
        }
        if self.lost > 0 {
            notes.push(Note::caveat(format!(
                "this reader lost {} delivery(ies) or stalled: every pair across a loss has no \
                 presence",
                self.lost
            )));
        }
        if self.unresolved > 0 {
            notes.push(Note::caveat(format!(
                "{} sample(s) on a key that resolved to no metric member, or on a wildcard key, \
                 were not read",
                self.unresolved
            )));
        }
        for s in &self.skipped {
            notes.push(Note::caveat(format!("not read: {s}")));
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let x = &self.totals;
        notes.push(Note::summary(format!(
            "{} member(s) of {} metric resource(s); {} sample(s) do not conform, {} pair(s) \
             decreased within an established epoch.",
            x.members, x.resources, x.does_not, x.decreased
        )));
        notes
    }

    /// What the reading put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: Some(self.window_s),
        })
    }
}
