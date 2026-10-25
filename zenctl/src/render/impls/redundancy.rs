//! `redundancy.v1` (#841, RC): `zenctl redundancy`'s report.

use zenkey_fleet::report::{
    DoctorSeverity, RedundancyAnswer, RedundancyAnswerToken, RedundancyReport, RedundancyRole,
};

use crate::render::{Cell, Grid, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

fn answer_word(a: RedundancyAnswerToken) -> &'static str {
    match a {
        RedundancyAnswerToken::Yes => "yes",
        RedundancyAnswerToken::No => "no",
        RedundancyAnswerToken::Unobservable => "unobservable",
        RedundancyAnswerToken::NotAsked => "not asked",
        RedundancyAnswerToken::NotThisProfiles => "not this profile's",
    }
}

fn says(a: &RedundancyAnswer) -> String {
    format!("{} ({}) — {}", answer_word(a.answer), a.reason, a.says)
}

/// `redundancy.v1` (#841, RC): §5's four answers per governed interface,
/// in the profile's tokens and in words. One row per address and interface
/// (`interface`), its instances as lines beneath it, each with its role. In
/// the table, "is it served by exactly one incumbent?" leads each row with
/// its own mark — `✓` one, `✗` a finding (a split, a vacancy in both reads,
/// a twin), `?` no verdict — and the run's judgement leads the envelope.
impl Render for RedundancyReport {
    const FAMILY: &'static str = "redundancy";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["interfaces"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.interfaces {
            out(Row::of("interface", r));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        t.line(format!(
            "redundancy.v1 in {}{}{} — {} interface(s), two reads {:.1}s apart{}",
            namespace_words(&self.namespace),
            self.service
                .as_ref()
                .map(|a| format!(", {a}"))
                .unwrap_or_default(),
            self.iface
                .as_ref()
                .map(|i| format!(", {i}"))
                .unwrap_or_default(),
            self.interfaces.len(),
            self.apart_s,
            if self.across_face {
                ", across a constrained face"
            } else {
                ""
            }
        ));
        for (n, r) in self.reads.iter().enumerate() {
            if !r.complete {
                t.line(format!(
                    "  the {} read is not complete: {}",
                    if n == 0 { "first" } else { "second" },
                    r.reason.as_deref().unwrap_or("possibly incomplete")
                ));
            }
        }
        let mut grid = Grid::unheaded(3);
        for r in &self.interfaces {
            let (mark, st) = if r.is_finding() {
                ("✗", style::severity(DoctorSeverity::Error))
            } else if r.is_clean() {
                ("✓", style::PASS)
            } else {
                ("?", style::UNPROVEN)
            };
            grid.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} {}", r.address, r.iface)),
                Cell::text(format!("exactly one incumbent: {}", says(&r.one_incumbent))),
            ]);
            let mut detail = vec![
                format!("    governance {} — {}", r.governance, r.governance_says),
                format!("    vacancy: {}", says(&r.vacancy)),
                format!(
                    "    standbys: {} ({}, {} {}) — {}",
                    answer_word(r.standbys.answer),
                    r.standbys.reason,
                    r.standbys.count,
                    if r.standbys.exact {
                        "exact"
                    } else {
                        "at least"
                    },
                    r.standbys.says
                ),
            ];
            for i in &r.instances {
                let role = match i.role {
                    RedundancyRole::Incumbent => "the incumbent",
                    RedundancyRole::Standby => "a standby",
                    RedundancyRole::Twin => "a TWIN",
                    RedundancyRole::NotACandidate => "no candidate",
                    RedundancyRole::Unknown => "unknown",
                };
                detail.push(format!(
                    "    {}{} {role}: {}",
                    i.instance,
                    i.zid
                        .as_ref()
                        .map(|z| format!(" (zid {z})"))
                        .unwrap_or_default(),
                    says(&i.twin)
                ));
            }
            grid.detail(detail);
        }
        t.grid(grid);
        if let Some(why) = &self.unobservable {
            t.line(format!("unobservable: {why}"));
        }
    }
}
