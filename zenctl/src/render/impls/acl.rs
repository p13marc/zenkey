//! `Render` for the ACL family (#392): the plan, the check, the explanation.
//!
//! The plan is three lists and the table draws them as two: the principals
//! (who, as what, with which rules) and the rules (what each allows or
//! denies, where). The JSON5 block is deliberately *not* here — it is
//! zenoh's schema, emitted under `--json5`, and a rendering of it would be a
//! second copy of `to_json5` that drifts.

use zenkey_fleet::report::{
    AclCheck, AclDecision, AclDirection, AclExplain, AclPermission, AclPlan,
};

use crate::render::{Cell, Grid, Note, Render, Row, Table, envelope_without};

fn flows_of(flows: &Option<Vec<zenkey_fleet::report::AclFlow>>) -> String {
    match flows {
        None => "both".to_string(),
        Some(f) => f.iter().map(|x| x.as_str()).collect::<Vec<_>>().join("+"),
    }
}

impl Render for AclPlan {
    const FAMILY: &'static str = "acl-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(
            self,
            &["rules", "subjects", "policies", "warnings", "refusals"],
        )
    }

    /// Five row kinds on one stream — the three lists zenoh needs, plus
    /// what the plan wants said about them.
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
                "principals under base {:?} (default deny):",
                self.base
            ))
            .blank();
            let mut g = Grid::new(["  subject", "role", "bound by", "rules"]);
            for s in &self.subjects {
                let bound = if !s.cert_common_names.is_empty() {
                    format!("cn {}", s.cert_common_names.join(", "))
                } else {
                    format!("zid {}", s.zids.join(", "))
                };
                let rules = self
                    .policies
                    .iter()
                    .filter(|p| p.subjects.contains(&s.id))
                    .flat_map(|p| p.rules.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(", ");
                g.row([
                    Cell::text(format!("  {}", s.id)),
                    Cell::text(s.role.as_str()),
                    Cell::text(bound),
                    Cell::text(rules),
                ]);
            }
            t.grid(g);
        }
        if !self.rules.is_empty() {
            t.blank().line("rules:").blank();
            let mut g = Grid::new(["  rule", "permission", "flows", "messages"]);
            for r in &self.rules {
                let mark = match r.permission {
                    AclPermission::Allow => "  ",
                    AclPermission::Deny => "✗ ",
                };
                g.row([
                    Cell::text(format!("{mark}{}", r.id)),
                    Cell::text(r.permission.as_str()),
                    Cell::text(flows_of(&r.flows)),
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
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        notes.extend(self.warnings.iter().map(crate::cmd::acl::warning_note));
        notes.extend(self.refusals.iter().map(crate::cmd::acl::refusal_note));
        if self.subjects.is_empty() {
            notes.push(Note::caveat(
                "no principal enrolled: the block would deny everything to everybody",
            ));
        }
        notes.push(Note::summary(format!(
            "{} rule(s), {} subject(s), {} polic(y/ies)",
            self.rules.len(),
            self.subjects.len(),
            self.policies.len()
        )));
        notes.push(Note::next_step(
            "write the block: `zenctl acl gen --enrollment … --json5 > router-acl.json5`, \
             merge it at the router config's top level, restart the router — ACL config \
             is not runtime-reloadable (RFC 03 §4 D6)",
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
                "compared against the config file {} through zenoh's own loader — zenoh 1.10 \
                 serves no GET on `@/<zid>/router/config/**`, so the running block is not \
                 observable from the bus and a file that differs from what the router was \
                 started with is invisible here",
                self.against
            ))
            .cite("RFC 13 §3 O5"),
            Note::coverage(
                "interest propagation not asked: whether a consumer's declared interest \
                 reaches the publishers' faces is observable only from a publisher's \
                 matching listener, which a config-file check has no way to hold",
            )
            .cite("RFC 13 §3 O4"),
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
                "      {mark} {}  {}  {}",
                g.rule,
                g.permission.as_str(),
                g.key_expr
            )
        })
        .collect()
}
