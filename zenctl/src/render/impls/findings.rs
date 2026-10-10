//! The families whose rows are *findings*: doctor, why, check conform and
//! health.
//!
//! What this seam is for: an empty result means something, and it never
//! means "nothing is wrong" — `doctor` with no findings has to say what it
//! checked.

use zenkey_fleet::report::{CheckReport, DoctorReport, DoctorSeverity, Judgement};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

/// zk2's doctor (#612, FJ6): one row per check, its verdict in the
/// judgement shape. Each pole has its own word and its own mark in the
/// table — `✗`/`⚠`/`·` a finding by its worst severity, `✓ clean`,
/// `? unobservable`, `— not asked` — and its own `answer` in a row, so a
/// script branches on the answer and never on the prose. A finding is the
/// yes of every check's question (tooling guide §1).
impl Render for DoctorReport {
    const FAMILY: &'static str = "doctor";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The scope and the empty-scope reason, so a document whose checks
        // were cut off still says what it read.
        envelope_without(self, &["checks"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for c in &self.checks {
            out(Row::of("check", c));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3);
        for c in &self.checks {
            let (mark, style, word) = verdict_cell(c);
            // The lists ride the detail lines below; the line itself says
            // the pole and how many subjects, or the reason when there are
            // none to list.
            let unjudged = match c.unjudged.len() {
                0 => String::new(),
                n => format!(", {n} unjudged"),
            };
            let what = match &c.verdict {
                Judgement::Established => {
                    format!("{word} — {} subject(s){unjudged}", c.findings.len())
                }
                Judgement::NotEstablished { reason } => format!("{word} — {reason}"),
                Judgement::Unobservable { .. } if !c.unjudged.is_empty() => {
                    format!("{word} — {} subject(s) unjudged", c.unjudged.len())
                }
                Judgement::Unobservable { reason } => format!("{word} — {reason}"),
                Judgement::NotAsked => word.to_owned(),
            };
            grid.row([
                // The mark and the word both say the pole; colour only
                // repeats it, so stripping the escape loses nothing (#200).
                Cell::styled(mark, style),
                Cell::text(format!("{} ({})", c.check, c.section)),
                Cell::text(what),
            ]);
            let findings = c.findings.iter().map(|f| {
                format!(
                    "    {} {}: {} — {}",
                    crate::render::style::mark(f.severity),
                    f.severity.as_str(),
                    f.subject,
                    f.evidence
                )
            });
            let unjudged = c
                .unjudged
                .iter()
                .map(|u| format!("    ? unjudged {}: {}", u.subject, u.reason));
            let detail: Vec<String> = findings.chain(unjudged).collect();
            if !detail.is_empty() {
                grid.detail(detail);
            }
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = if self.scope.namespace.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", self.scope.namespace)
        };
        if let Some(p) = self.scope.presence.as_option() {
            notes.push(
                Note::coverage(format!(
                    "read {ns} through `{}`, twice, {:.1}s apart: {} service(s), {} \
                     instance(s), {} token(s), {} instance(s) with no readable descriptor; \
                     {} of {} revision(s) named retrieved and verified",
                    p.selector,
                    p.grace_s,
                    p.services,
                    p.instances,
                    p.tokens,
                    p.undescribed,
                    p.held,
                    p.revisions
                ))
                .cite("spec §8.1"),
            );
            if !p.complete {
                notes.push(
                    Note::coverage(
                        "a presence read ran to its timeout, so it is possibly incomplete: \
                         an absence it would claim is left unjudged — raise --timeout to \
                         ask again",
                    )
                    .cite("spec §8.1"),
                );
            }
        }
        if let Some(n) = self.scope.routers.get() {
            notes.push(Note::coverage(format!(
                "{n} router(s) answered the admin space, read in no namespace"
            )));
        }
        if let Some(h) = self.scope.health.as_option() {
            notes.push(
                Note::coverage(format!(
                    "read health.v1 through `{}`, with each presence read: {} service(s) list \
                     it; {}",
                    h.selector,
                    h.services,
                    if h.clocks_synced {
                        "a status's age is read against this host's clock, on the deployment's \
                         word (--clocks-synced)"
                    } else {
                        "no clock is trusted, so a status's age is unobservable — pass \
                         --clocks-synced on the deployment's word"
                    }
                ))
                .cite("freshness.v1 §2.6"),
            );
        }
        let not_asked: Vec<String> = self
            .checks
            .iter()
            .filter(|c| c.verdict.is_not_asked())
            .map(|c| match c.check {
                zenkey_fleet::report::CheckId::StateStampForeign => {
                    format!("{} (pass --deep)", c.check)
                }
                _ => c.check.to_string(),
            })
            .collect();
        if !not_asked.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "not asked: {} — not asked is neither clean nor a finding",
                    not_asked.join(", ")
                ))
                .cite("tooling guide O4"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let count = |s| self.count(s);
        let unobservable = self
            .checks
            .iter()
            .filter(|c| c.verdict.is_unobservable())
            .count();
        let clean = self
            .checks
            .iter()
            .filter(|c| matches!(c.verdict, Judgement::NotEstablished { .. }))
            .count();
        notes.push(Note::summary(if self.unobservable.is_some() {
            "nothing in scope — no verdict on the deployment.".to_owned()
        } else {
            format!(
                "{} finding(s): {} error(s), {} warning(s), {} info; {clean} check(s) clean, \
                 {unobservable} unobservable.",
                self.findings().count(),
                count(DoctorSeverity::Error),
                count(DoctorSeverity::Warning),
                count(DoctorSeverity::Info),
            )
        }));
        notes
    }

    /// What the run put to the bus: the presence selector in the namespace,
    /// and the admin space in none.
    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = Vec::new();
        if let Some(p) = self.scope.presence.as_option() {
            asked.push(p.selector.clone());
        }
        if self.scope.routers.is_asked() {
            asked.push("@/*/router".to_owned());
        }
        if let Some(h) = self.scope.health.as_option() {
            asked.push(h.selector.clone());
        }
        Some(ObservedScope {
            asked,
            window_s: None,
        })
    }
}

/// zk2's `why` (#702): one row per rung, in ladder order, each pole its
/// own mark and word — `✗ cause`, `✓ healthy`, `? unobservable`, `— not
/// asked` — and its own `answer` in a row, so a script branches on the
/// rung and the answer, never on the prose. A cause is the yes of every
/// rung's question (tooling guide §1), and the finding of the run.
impl Render for zenkey_fleet::WhyReport {
    const FAMILY: &'static str = "why";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The verdict, the stop and what was asked lead, so a document cut
        // short still says what the ladder established.
        envelope_without(self, &["rungs"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rungs {
            out(Row::of("rung", r));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        let headline = match (&self.verdict, self.stopped_at) {
            (Judgement::Established, Some(at)) => format!("a cause, at {at}"),
            (Judgement::NotEstablished { .. }, _) => "every rung healthy".to_owned(),
            (Judgement::Unobservable { .. }, Some(at)) => format!("no verdict, at {at}"),
            (Judgement::NotAsked, _) => "no verdict: the answer was not asked".to_owned(),
            _ => "no verdict".to_owned(),
        };
        t.line(format!("why {} — {headline}", self.target));
        let mut grid = Grid::unheaded(3);
        for r in &self.rungs {
            let (mark, st, word) = match &r.verdict {
                Judgement::Established => ("✗", style::severity(DoctorSeverity::Error), "cause"),
                Judgement::NotEstablished { .. } => ("✓", style::PASS, "healthy"),
                Judgement::Unobservable { .. } => ("?", style::UNPROVEN, "unobservable"),
                Judgement::NotAsked => ("—", style::UNPROVEN, "not asked"),
            };
            let what = match (&r.verdict, &r.cause) {
                (Judgement::Established, Some(c)) => format!("{word} — {c}"),
                (Judgement::NotEstablished { reason }, _)
                | (Judgement::Unobservable { reason }, _) => format!("{word} — {reason}"),
                _ => word.to_owned(),
            };
            grid.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} ({})", r.rung, r.section)),
                Cell::text(what),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if let Some(lk) = &self.last_known {
            notes.push(
                Note::caveat(format!(
                    "{} still holds a last-known value of {} ({}): last-known, never current — \
                     the owner's silence is not explained by it",
                    lk.archive,
                    lk.key,
                    if lk.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    }
                ))
                .cite("spec §4.2 S6"),
            );
        }
        if self.verdict.is_not_asked() {
            notes.push(Note::coverage(
                "an operation's answer is not asked: why calls nothing — `zenctl call` asks it, \
                 and its silence is attributed through presence",
            ));
        }
        let not_asked = self
            .rungs
            .iter()
            .filter(|r| r.verdict.is_not_asked())
            .count();
        notes.push(Note::summary(match (&self.verdict, self.cause()) {
            (Judgement::Established, Some((rung, _))) => format!(
                "a cause at {rung}: the ladder stopped there, {not_asked} rung(s) not asked"
            ),
            (Judgement::NotEstablished { .. }, _) => {
                "every rung is healthy, and the key answers.".to_owned()
            }
            _ => {
                "no verdict — a rung could not be observed, or the answer was not asked.".to_owned()
            }
        }));
        notes
    }

    /// What the ladder put to the bus: presence, the key, the archives —
    /// and over what window, when it listened to a stream.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: self.window_s,
        })
    }
}

/// zk2's `check conform` (#703): one row per case and subject, each pole
/// its own mark and word — `✗ violation`, `✓ passed`, `? unobservable`,
/// `— not asked` — and its own `answer` in a `case` row. A violation is
/// the yes of every case's question (tooling guide §1).
impl Render for zenkey_fleet::ConformReport {
    const FAMILY: &'static str = "conform";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["cases"]);
        // The run's own judgement leads, so a document cut short still
        // says whether the service conformed.
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for c in &self.cases {
            out(Row::of("case", c));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        t.line(format!(
            "conform {} {} at {}",
            self.address,
            self.iface,
            self.fingerprint.as_deref().unwrap_or("no revision")
        ));
        let mut grid = Grid::unheaded(4);
        for c in &self.cases {
            let (mark, st, word) = match &c.verdict {
                Judgement::Established => {
                    ("✗", style::severity(DoctorSeverity::Error), "violation")
                }
                Judgement::NotEstablished { .. } => ("✓", style::PASS, "passed"),
                Judgement::Unobservable { .. } => ("?", style::UNPROVEN, "unobservable"),
                Judgement::NotAsked => ("—", style::UNPROVEN, "not asked"),
            };
            let what = match (&c.verdict, &c.detail) {
                (Judgement::NotEstablished { reason }, _)
                | (Judgement::Unobservable { reason }, _) => format!("{word} — {reason}"),
                (_, Some(d)) => format!("{word} — {d}"),
                _ => word.to_owned(),
            };
            grid.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} ({})", c.case, c.section)),
                Cell::text(&c.subject),
                Cell::text(what),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let not_called = self
            .cases
            .iter()
            .filter(|c| {
                c.verdict.is_not_asked()
                    && matches!(
                        c.case,
                        zenkey_fleet::report::CaseId::Operation
                            | zenkey_fleet::report::CaseId::FanoutRefused
                    )
            })
            .count();
        if not_called > 0 {
            notes.push(
                Note::coverage(format!(
                    "{not_called} operation case(s) not asked: their operations are not \
                     idempotent, and a call is a write — pass --i-know to call every one"
                ))
                .cite("tooling guide O4"),
            );
        }
        // §2.7 (0.24): a window shows a stream's or an event's population
        // within its bound only once it is complete — a whole liveness span,
        // lossless, the owner present throughout.
        let windowed = self
            .cases
            .iter()
            .filter(|c| {
                c.case == zenkey_fleet::report::CaseId::BudgetWindow && c.verdict.is_unobservable()
            })
            .count();
        if windowed > 0 {
            notes.push(
                Note::coverage(format!(
                    "{windowed} budget-window case(s) unobservable: a window shows a stream's or \
                     an event's population within its bound only over a whole liveness span (an \
                     hour, or the retention: --for), nothing lost, the owner present throughout \
                     — more than the bound would be a violation; --skip budget-window leaves the \
                     case unasked and the state's budget asked"
                ))
                .cite("spec §2.7"),
            );
        }
        let count = |p: fn(&Judgement) -> bool| self.cases.iter().filter(|c| p(&c.verdict)).count();
        notes.push(Note::summary(format!(
            "{} case(s): {} violation(s), {} passed, {} unobservable, {} not asked.",
            self.cases.len(),
            count(|j| *j == Judgement::Established),
            count(|j| matches!(j, Judgement::NotEstablished { .. })),
            count(Judgement::is_unobservable),
            count(Judgement::is_not_asked),
        )));
        notes
    }

    /// What the suite put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: Some(self.window_s),
        })
    }
}

/// `health.v1` (#721, PF): one row per service, each verdict its own mark
/// and word — `✓ healthy`, `✗ unhealthy`, `⚠ stale`, `? unobservable`, `—
/// not asked` — and its own `verdict` token in a `service` row, so a script
/// branches on the profile's token and never on the prose. §5's other
/// answers, the status and checks as read, and an archive's last-known
/// status ride the detail lines; stale is never drawn as a level, and a
/// last-known status never as current. The run's judgement leads the
/// envelope; the finding is the yes of "is a service unhealthy, stale,
/// breaking §2.2 or running its clock ahead?".
impl Render for zenkey_fleet::HealthReport {
    const FAMILY: &'static str = "health";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["services"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.services {
            out(Row::of("service", s));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        use zenkey_fleet::report::{HealthAnswerToken, HealthVerdict, StatusVia};
        let r = &self.rollup;
        t.line(format!(
            "health in {} — worst {}: {} healthy, {} unhealthy, {} stale, {} unobservable, {} \
             not asked",
            namespace_words(&self.namespace),
            r.worst
                .map_or("none established".to_owned(), |l| level_word(l.as_str())),
            r.healthy,
            r.unhealthy,
            r.stale,
            r.unobservable,
            r.not_asked
        ));
        let mut grid = Grid::unheaded(3);
        for s in &self.services {
            let (mark, st) = match s.verdict {
                HealthVerdict::Healthy => ("✓", style::PASS),
                HealthVerdict::Unhealthy => ("✗", style::severity(DoctorSeverity::Error)),
                HealthVerdict::Stale => ("⚠", style::severity(DoctorSeverity::Warning)),
                HealthVerdict::Unobservable | HealthVerdict::NotAsked => ("?", style::UNPROVEN),
            };
            let mark = if s.verdict == HealthVerdict::NotAsked {
                "—"
            } else {
                mark
            };
            let level = s
                .level
                .map(|l| format!(", {}", level_word(l.as_str())))
                .unwrap_or_default();
            grid.row([
                Cell::styled(mark, st),
                Cell::text(&s.address),
                Cell::text(format!(
                    "{} ({}{level}) — {}",
                    s.verdict.as_str().replace('_', " "),
                    s.reason,
                    s.says
                )),
            ]);
            let mut detail = Vec::new();
            if let Some(st) = &s.status {
                detail.push(format!(
                    "    status {} {:?} — {}{}",
                    level_read_word(&st.level),
                    st.reason,
                    match st.via {
                        StatusVia::Get => "its owner's reply",
                        StatusVia::Subscription => "the last put heard",
                    },
                    st.stamp
                        .as_ref()
                        .map(|s| format!(", stamped {} by {}", s.time, s.clock))
                        .unwrap_or_default()
                ));
            }
            if !s.checks.is_empty() {
                let checks: Vec<String> = s
                    .checks
                    .iter()
                    .map(|c| {
                        if c.detail.is_empty() {
                            format!("{} {}", c.check, level_read_word(&c.level))
                        } else {
                            format!("{} {} {:?}", c.check, level_read_word(&c.level), c.detail)
                        }
                    })
                    .collect();
                detail.push(format!("    checks {}", checks.join(", ")));
            }
            let answer = |a: &zenkey_fleet::report::HealthAnswer| {
                let word = match a.answer {
                    HealthAnswerToken::Yes => "yes",
                    HealthAnswerToken::No => "no",
                    HealthAnswerToken::Unobservable => "unobservable",
                    HealthAnswerToken::NotAsked => "not asked",
                };
                format!("{word} — {}", a.says)
            };
            detail.push(format!("    agrees with its checks: {}", answer(&s.agrees)));
            detail.push(format!("    clock ahead: {}", answer(&s.clock_ahead)));
            if s.faults > 0 {
                detail.push(format!(
                    "    {} fault(s) heard{}",
                    s.faults,
                    s.last_fault
                        .as_ref()
                        .map(|f| format!(
                            ", the last {} ({}) {}{}",
                            f.code,
                            f.class,
                            level_read_word(&f.level),
                            if f.detail.is_empty() {
                                String::new()
                            } else {
                                format!(" {:?}", f.detail)
                            }
                        ))
                        .unwrap_or_default()
                ));
            }
            if let Some(lk) = &s.last_known {
                detail.push(format!(
                    "    last-known at {} ({}): {} {:?} — last-known, never current",
                    lk.archive,
                    if lk.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    },
                    level_read_word(&lk.level),
                    lk.reason
                ));
            }
            grid.detail(detail);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        use zenkey_fleet::report::ClockGround;
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        match (&self.presence, &self.face) {
            (_, Some(f)) => notes.push(
                Note::coverage(format!(
                    "across a constrained face: presence and descriptors do not cross, and the \
                     deployment's word stands for them — the face {} its status cross",
                    if f.status_crosses {
                        "lets"
                    } else {
                        "does not let"
                    }
                ))
                .cite("health.v1 §2.8"),
            ),
            (Some(p), None) => {
                notes.push(
                    Note::coverage(format!(
                        "read {ns} through `{}`: {} service(s) holding an instance token; a \
                         service implements health.v1 when its descriptor lists it, never by its \
                         interface token",
                        p.selector, p.services
                    ))
                    .cite("health.v1 §2.7"),
                );
                if !p.complete {
                    notes.push(
                        Note::coverage(
                            "the presence read ran to its timeout, so it is possibly incomplete: \
                             a service unseen is unobservable, never absent",
                        )
                        .cite("spec §8.1"),
                    );
                }
            }
            (None, None) => {}
        }
        notes.push(Note::coverage(format!(
            "two readings {:.1}s apart, each one GET that answers a status and its checks \
             together{}",
            self.apart_s,
            self.window_s
                .map(|w| format!(", after a {w:.1}s subscription to every status and the faults"))
                .unwrap_or_default()
        )));
        notes.push(
            Note::caveat(match self.clock {
                ClockGround::DeploymentWord => {
                    "a status reply's stamp is aged against this host's clock, on the \
                     deployment's word that it and the owners' agree (--clocks-synced)"
                }
                ClockGround::Measured => {
                    "a status reply's stamp is aged only against a clock this reading measured on \
                     a live put of the same clock"
                }
                ClockGround::None => {
                    "no clock was trusted — no window, no --clocks-synced — so a status reply's \
                     age is unobservable"
                }
            })
            .cite("freshness.v1 §2.6"),
        );
        if self.rollup.stale > 0 {
            notes.push(
                Note::caveat(
                    "stale is its own finding, never a level: never FAILED, never DEGRADED, and \
                     never the last level as though current — and never down",
                )
                .cite("health.v1 §2.4"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let r = &self.rollup;
        notes.push(Note::summary(format!(
            "{} service(s): {} healthy, {} unhealthy, {} stale, {} unobservable, {} not asked; \
             worst established level {}.",
            self.services.len(),
            r.healthy,
            r.unhealthy,
            r.stale,
            r.unobservable,
            r.not_asked,
            r.worst
                .map_or("none".to_owned(), |l| level_word(l.as_str()))
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

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// A level as the profile spells it: `OK`, `DEGRADED`, `FAILED`.
fn level_word(token: &str) -> String {
    token.to_ascii_uppercase()
}

/// A level as a payload carried it: a level, `UNSPECIFIED`, `undecodable`,
/// or a number the enum does not list.
fn level_read_word(l: &zenkey_fleet::report::LevelRead) -> String {
    match l {
        zenkey_fleet::report::LevelRead::Token("undecodable") => "undecodable".to_owned(),
        zenkey_fleet::report::LevelRead::Token(t) => level_word(t),
        zenkey_fleet::report::LevelRead::Unlisted(n) => format!("level {n} (unknown)"),
    }
}

/// A check's pole as the table spells it: the mark, its style, and the word.
fn verdict_cell(c: &CheckReport) -> (&'static str, anstyle::Style, &'static str) {
    use crate::render::style;
    match &c.verdict {
        Judgement::Established => {
            let worst = c
                .findings
                .iter()
                .map(|f| f.severity)
                .find(|s| *s == DoctorSeverity::Error)
                .or_else(|| {
                    c.findings
                        .iter()
                        .map(|f| f.severity)
                        .find(|s| *s == DoctorSeverity::Warning)
                })
                .unwrap_or(DoctorSeverity::Info);
            (style::mark(worst), style::severity(worst), "finding")
        }
        Judgement::NotEstablished { .. } => ("✓", style::PASS, "clean"),
        Judgement::Unobservable { .. } => ("?", style::UNPROVEN, "unobservable"),
        Judgement::NotAsked => ("—", style::UNPROVEN, "not asked"),
    }
}
