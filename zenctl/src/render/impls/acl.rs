//! `Render` for the ACL family (#612, FJ7): the plan, the check, the
//! explanation.
//!
//! The plan is three lists and the table draws them as two: the principals
//! (who, bound by what, running what, with which rules) and the rules (what
//! each allows or denies, where, as which grant). The JSON5 fragment is
//! deliberately *not* here: it is zenoh's schema, emitted under `--json5`,
//! and a rendering of it would be a second copy of `to_json5` that drifts.

use zenkey_fleet::report::{
    AclCheck, AclDecision, AclDirection, AclExplain, AclPermission, AclPlan,
};

use crate::render::{Cell, Grid, Note, Render, Row, Table, envelope_without};

impl Render for AclPlan {
    const FAMILY: &'static str = "acl-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(
            self,
            &["rules", "subjects", "policies", "warnings", "refusals"],
        )
    }

    /// Five row kinds on one stream: the three lists zenoh needs, plus what
    /// the plan wants said about them.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rules {
            out(Row::of("rule", r));
        }
        for s in &self.subjects {
            out(Row::of("subject", s));
        }
        for p in &self.policies {
            out(Row::of("policy", p));
        }
        for w in &self.warnings {
            out(Row::of("warning", w));
        }
        for r in &self.refusals {
            out(Row::of("refusal", r));
        }
    }

    fn table(&self, t: &mut Table) {
        if !self.subjects.is_empty() {
            t.line(format!(
                "principals under namespace {:?} (default {}):",
                self.namespace,
                self.default_permission.as_str()
            ))
            .blank();
            let mut g = Grid::new(["  subject", "bound by", "runs", "rules"]);
            for s in &self.subjects {
                let mut bound = Vec::new();
                if !s.usernames.is_empty() {
                    bound.push(format!("user {}", s.usernames.join(", ")));
                }
                if !s.cert_common_names.is_empty() {
                    bound.push(format!("cn {}", s.cert_common_names.join(", ")));
                }
                let rules = self
                    .policies
                    .iter()
                    .filter(|p| p.subjects.contains(&s.id))
                    .flat_map(|p| p.rules.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(", ");
                g.row([
                    Cell::text(format!("  {}", s.id)),
                    Cell::text(bound.join("; ")),
                    Cell::text(s.runs.join(", ")),
                    Cell::text(rules),
                ]);
            }
            t.grid(g);
        }
        if !self.rules.is_empty() {
            t.blank().line("rules:").blank();
            let mut g = Grid::new(["  rule", "grant", "flows", "messages"]);
            for r in &self.rules {
                let mark = match r.permission {
                    AclPermission::Allow => "  ",
                    AclPermission::Deny => "✗ ",
                };
                g.row([
                    Cell::text(format!("{mark}{}", r.id)),
                    Cell::text(r.grant.as_str()),
                    Cell::text(
                        r.flows
                            .iter()
                            .map(|f| f.as_str())
                            .collect::<Vec<_>>()
                            .join("+"),
                    ),
                    Cell::text(
                        r.messages
                            .iter()
                            .map(|m| m.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ]);
                g.detail(r.key_exprs.iter().map(|k| format!("      {k}")));
            }
            t.grid(g);
        }
        if let (Some(face), Some(gw)) = (&self.face, &self.gateway) {
            t.blank().line(format!(
                "gateway.south (the far router {} in region {:?}):",
                face.far,
                face.region.as_deref().unwrap_or_default()
            ));
            for (i, s) in gw.south.iter().enumerate() {
                let filters: Vec<String> = s
                    .filters
                    .iter()
                    .map(|f| {
                        let mut parts = Vec::new();
                        if !f.modes.is_empty() {
                            parts.push(format!("modes {}", f.modes.join(", ")));
                        }
                        if !f.region_names.is_empty() {
                            parts.push(format!("region {}", f.region_names.join(", ")));
                        }
                        parts.join("; ")
                    })
                    .collect();
                t.line(format!("  subregion {i}: {}", filters.join(" | ")));
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        notes.extend(self.warnings.iter().map(crate::cmd::acl::warning_note));
        notes.extend(self.refusals.iter().map(crate::cmd::acl::refusal_note));
        if self.subjects.is_empty() {
            notes.push(Note::caveat(
                "no principal placed: under deny the block would deny everything to everybody",
            ));
        }
        if let Some(face) = &self.face {
            notes.push(Note::caveat(format!(
                "a constrained face to {} ({}): its policy carries the @zk and @stream denies, \
                 and no presence or contract grant; across it, liveliness is unobservable, \
                 never down (R7, §8.5)",
                face.far,
                face.attach.as_str()
            )));
        }
        notes.push(Note::summary(format!(
            "{} rule(s), {} subject(s), {} polic(y/ies), compiled from {} contract revision(s)",
            self.rules.len(),
            self.subjects.len(),
            self.policies.len(),
            self.contracts.len()
        )));
        notes.push(Note::next_step(
            "write the fragment: `zenctl acl gen --enrollment … --contracts … --json5 > \
             router-acl.json5`, merge it at the router config's top level, restart the \
             router, and regenerate on every contract revision (spec §11.2)",
        ));
        notes
    }
}

impl Render for AclCheck {
    const FAMILY: &'static str = "acl-check";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["findings"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.findings {
            out(Row::of("finding", f));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{}: {} rule(s) and {} subject(s) configured; {} and {} planned",
            self.against,
            self.observed_rules,
            self.observed_subjects,
            self.planned_rules,
            self.planned_subjects
        ));
        if !self.findings.is_empty() {
            let mut g = Grid::unheaded(3);
            for f in &self.findings {
                g.row([
                    Cell::text(format!("  ✗ {}", f.kind.as_str())),
                    Cell::text(&f.id),
                    Cell::text(match (&f.planned, &f.observed) {
                        (Some(p), Some(o)) => format!("planned {p}; configured {o}"),
                        (Some(p), None) => format!("planned {p}"),
                        (None, Some(o)) => format!("configured {o}"),
                        (None, None) => String::new(),
                    }),
                ]);
            }
            t.grid(g);
        }
        // The word is the carrier; the colour repeats it (#200).
        let (word, style) = match self.judgement.conclusive() {
            Some(false) => ("PASS", crate::render::style::PASS),
            Some(true) => ("FAIL", crate::render::style::ERROR),
            None => ("UNPROVEN", crate::render::style::UNPROVEN),
        };
        t.line_styled(word, style);
    }

    fn notes(&self) -> Vec<Note> {
        vec![
            Note::coverage(format!(
                "compared against the config file {} through zenoh's own loader: access \
                 control is enforced per hop and the running configuration is not observable \
                 on the bus, so a file that differs from what the router was started with is \
                 invisible here",
                self.against
            ))
            .cite("spec §11.3"),
        ]
    }
}

impl Render for AclExplain {
    const FAMILY: &'static str = "acl-explain";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["ingress", "egress"])
    }

    /// One row per direction, tagged: a script keys on `flow`.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for (flow, d) in [("ingress", &self.ingress), ("egress", &self.egress)] {
            let mut v = crate::render::envelope_of(d);
            v.insert("flow".into(), flow.into());
            out(Row::tagged("direction", serde_json::Value::Object(v)));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{}  {}  {}",
            self.principal,
            self.message.as_str(),
            self.key
        ));
        let mut g = Grid::unheaded(3);
        for (flow, d) in [("ingress", &self.ingress), ("egress", &self.egress)] {
            let word = match d.decision {
                AclDecision::Allowed => "ALLOWED",
                AclDecision::Denied => "DENIED",
                AclDecision::DeniedByDefault => "denied by default",
                AclDecision::AllowedByDefault => "allowed by default",
            };
            g.row([
                Cell::text(format!("  {flow}")),
                Cell::text(word),
                Cell::text(&d.reason),
            ]);
            g.detail(via_lines(d));
        }
        t.grid(g);
    }
}

fn via_lines(d: &AclDirection) -> Vec<String> {
    d.via
        .iter()
        .map(|g| {
            let mark = match g.permission {
                AclPermission::Allow => "✓",
                AclPermission::Deny => "✗",
            };
            format!(
                "      {mark} {}  {}  {}  ({})",
                g.rule,
                g.permission.as_str(),
                g.key_expr,
                g.grant.as_str()
            )
        })
        .collect()
}
