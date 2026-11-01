//! `Render` for the ACL family (#612, FJ7): the plan, the check, the
//! explanation; and `check face`, a router's block against its constrained
//! faces (`link.v1` §2.15, #769 LB).
//!
//! The plan is three lists and the table draws them as two: the principals
//! (who, bound by what, running what, with which rules) and the rules (what
//! each allows or denies, where, as which grant). A face policy adds a
//! section per face: its far principal and attachment, its downsampling
//! rules, its window floors and its liveness evidence; a `link.v2` policy
//! adds the router's transport, and `check face` each key of it against
//! the router's configuration and the far end's (#988, L2). The JSON5 fragment
//! is deliberately *not* here: it is zenoh's schema, emitted under
//! `--json5`, and a rendering of it would be a second copy of `to_json5`
//! that drifts.

use zenkey_fleet::report::{
    AclCheck, AclDecision, AclDirection, AclExplain, AclPermission, AclPlan, FaceCheck, FacePlan,
};
use zenkey_model::link::AttachPlan;

use crate::render::{Cell, Grid, Note, Render, Row, Table, envelope_without};

impl Render for AclPlan {
    const FAMILY: &'static str = "acl-plan";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(
            self,
            &[
                "rules",
                "subjects",
                "policies",
                "warnings",
                "refusals",
                "faces",
                "downsampling",
            ],
        )
    }

    /// Seven row kinds on one stream: the three lists zenoh needs, each
    /// face's plan and the downsampling items, plus what the plan wants said
    /// about them.
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
        for f in &self.faces {
            out(Row::of("face", f));
        }
        for d in &self.downsampling {
            out(Row::of("downsampling", d));
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
                // Narrowed off a face, or a face's own subject (link.v1
                // §2.4).
                if !s.link_protocols.is_empty() {
                    bound.push(format!("over {}", s.link_protocols.join(", ")));
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
        for f in &self.faces {
            face_table(t, f);
        }
        if let Some(tr) = &self.transport {
            transport_table(t, tr, self.binding, self.catalog.as_deref());
        }
        if let Some(gw) = &self.gateway {
            let regions: Vec<String> = self
                .faces
                .iter()
                .filter_map(|f| match &f.attach {
                    AttachPlan::SouthRegion { region, .. } => {
                        Some(format!("the far router {} in region {region:?}", f.far))
                    }
                    AttachPlan::Client => None,
                })
                .collect();
            t.blank()
                .line(format!("gateway.south ({}):", regions.join("; ")));
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
        notes.extend(
            self.faces
                .iter()
                .flat_map(|f| &f.findings)
                .map(crate::cmd::acl::face_finding_note),
        );
        for f in &self.faces {
            let unobservable: Vec<&str> = f
                .liveness
                .iter()
                .filter(|l| l.evidence.is_empty())
                .map(|l| l.service.as_str())
                .collect();
            notes.push(Note::caveat(format!(
                "face {} to {} ({}): its policy carries the face's denies and no presence, \
                 history, admin-space or contract grant; every other principal's subject is kept \
                 off the face's link protocols{}; requests cross by grant, never by exposure \
                 (link.v1 §2.4, §2.9){}",
                f.name,
                f.far,
                attach_word(&f.attach),
                match self.default_permission {
                    AclPermission::Deny => "",
                    AclPermission::Allow => ", and the face's own subject rides that policy",
                },
                if unobservable.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; across it the liveness of {} is unobservable, never down: nothing \
                         with a horizon crosses (§2.12, R7)",
                        unobservable.join(", ")
                    )
                }
            )));
        }
        if !self.faces.is_empty() {
            notes.push(Note::coverage(
                "what a router runs is not on the bus: `zenctl check face <router.json5> \
                 --face-policy …` judges the file it is said to run (link.v1 §2.14, §2.15); \
                 each listed owner of state must run with a tombstone window of at least the \
                 face's outage (§2.11), which no tool can observe",
            ));
        }
        notes.push(Note::summary(format!(
            "{} rule(s), {} subject(s), {} polic(y/ies){}, compiled from {} contract revision(s)",
            self.rules.len(),
            self.subjects.len(),
            self.policies.len(),
            if self.faces.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} face(s), {} downsampling rule(s)",
                    self.faces.len(),
                    self.downsampling
                        .iter()
                        .map(|d| d.rules.len())
                        .sum::<usize>()
                )
            },
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

/// A `link.v2` policy's transport (§2.16), as the generator writes it
/// (§2.18), each key beside zenoh's.
fn transport_table(
    t: &mut Table,
    tr: &zenkey_model::link::Transport,
    binding: Option<zenkey_model::link::Binding>,
    catalog: Option<&str>,
) {
    t.blank().line(format!(
        "transport (link.v2 §2.16){}: a keep-alive every {} ms when idle",
        binding.map_or(String::new(), |b| format!(", the router running {b}")),
        tr.keep_alive_interval_ms()
    ));
    let mut g = Grid::unheaded(3);
    for k in zenkey_model::link::TransportKey::ALL {
        g.row([
            Cell::text(format!("  {}", k.key())),
            Cell::text(tr.value(k).to_string()),
            Cell::text(k.path()),
        ]);
    }
    t.grid(g);
    if let Some(c) = catalog {
        t.line(format!("  catalog {c}: the far side decodes with it"));
    }
}

fn attach_word(a: &AttachPlan) -> String {
    match a {
        AttachPlan::Client => "client".to_owned(),
        AttachPlan::SouthRegion { region, .. } => format!("south-region {region:?}"),
    }
}

/// One face of a plan (`link.v1` §2.3): who it guards against, what it
/// downsamples, the windows it floors and the liveness that crosses. Its
/// denies are rules above, `face:<face>:<id>`.
fn face_table(t: &mut Table, f: &FacePlan) {
    t.blank().line(format!(
        "face {}: far {} ({}), {} deny rule(s) in its policy",
        f.name,
        f.far,
        attach_word(&f.attach),
        f.deny.len()
    ));
    match &f.downsampling {
        None => {
            t.line("  downsampling: none (no stream crosses)");
        }
        Some(d) => {
            t.line(format!(
                "  downsampling on {} ({}, {}):",
                d.link_protocols.join(", "),
                d.flows.join("+"),
                d.messages.join(", ")
            ));
            let mut g = Grid::unheaded(2);
            for r in &d.rules {
                g.row([
                    Cell::text(format!("    {}", r.key_expr)),
                    Cell::text(format!("{} Hz", r.freq)),
                ]);
            }
            t.grid(g);
        }
    }
    if !f.windows.is_empty() {
        t.line("  tombstone window floors (§2.11):");
        let mut g = Grid::unheaded(2);
        for w in &f.windows {
            g.row([
                Cell::text(format!("    {}", w.service)),
                Cell::text(format!("at least {} s", w.min_s)),
            ]);
        }
        t.grid(g);
    }
    if !f.liveness.is_empty() {
        t.line("  liveness across the face (§2.12):");
        let mut g = Grid::unheaded(2);
        for l in &f.liveness {
            g.row([
                Cell::text(format!("    {}", l.service)),
                Cell::text(if l.evidence.is_empty() {
                    "unobservable: no evidence crosses".to_owned()
                } else {
                    l.evidence.join(", ")
                }),
            ]);
        }
        t.grid(g);
    }
}

impl Render for FaceCheck {
    const FAMILY: &'static str = "face-check";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(
            self,
            &["faces", "findings", "windows", "transport", "comparison"],
        );
        // The comparison's verdict rides the envelope, its findings the rows.
        e.insert(
            "comparison".into(),
            serde_json::to_value(&self.comparison.judgement).unwrap_or_default(),
        );
        e
    }

    /// Five row kinds: each face's block as read, each finding of §2.15,
    /// §2.11 and `link.v2` §2.19, each window when windows were asked, each
    /// transport key of a `link.v2` policy, and each way the block differs
    /// from the plan.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.faces {
            out(Row::of("face", f));
        }
        for f in &self.findings {
            out(Row::of("finding", f));
        }
        for w in self.windows.as_option().into_iter().flatten() {
            out(Row::of("window", w));
        }
        for k in &self.transport {
            out(Row::of("transport", k));
        }
        for f in &self.comparison.findings {
            out(Row::of("difference", f));
        }
    }

    fn table(&self, t: &mut Table) {
        for f in &self.faces {
            t.line(format!(
                "{}: face {} (far {}): {} deny rule(s) across subject(s) {}; {} downsampling \
                 rule(s) on its transports",
                self.against,
                f.face,
                f.far,
                f.denies,
                if f.subjects.is_empty() {
                    "none".to_owned()
                } else {
                    f.subjects.join(", ")
                },
                f.rules
            ));
        }
        if !self.findings.is_empty() {
            let mut g = Grid::unheaded(3);
            for f in &self.findings {
                g.row([
                    Cell::text(format!("  ✗ {}", f.code)),
                    Cell::text(format!("face {}", f.face.as_deref().unwrap_or("-"))),
                    Cell::text(&f.at),
                ]);
            }
            t.grid(g);
        }
        if let Some(rows) = self.windows.as_option() {
            let mut g = Grid::unheaded(3);
            for w in rows {
                let mark = if w.window_s < w.min_s { "✗" } else { " " };
                g.row([
                    Cell::text(format!("  {mark} window {}", w.service)),
                    Cell::text(format!(
                        "{} s{}",
                        w.window_s,
                        if w.given { "" } else { " (the default)" }
                    )),
                    Cell::text(format!("face {}: at least {} s", w.face, w.min_s)),
                ]);
            }
            t.grid(g);
        }
        if !self.transport.is_empty() {
            t.line(format!(
                "transport (link.v2 §2.19){}:",
                self.binding.map_or(String::new(), |b| format!(
                    ", an omitted key read at {b}'s default"
                ))
            ));
            let mut g = Grid::unheaded(4);
            // The far end is judged on its lease and keep_alive alone; its
            // other keys are shown as it runs them.
            let judged = ["transport.lease_ms", "transport.keep_alive"];
            for k in &self.transport {
                let differs = !zenkey_model::link::same_value(&k.planned, &k.observed)
                    || (judged.contains(&k.key.as_str())
                        && k.peer
                            .as_ref()
                            .is_some_and(|p| !zenkey_model::link::same_value(p, &k.observed)));
                g.row([
                    Cell::text(format!("  {} {}", if differs { "✗" } else { " " }, k.key)),
                    Cell::text(format!("planned {}", k.planned)),
                    Cell::text(format!(
                        "runs {}{}",
                        k.observed,
                        if k.stated { "" } else { " (the default)" }
                    )),
                    Cell::text(match &k.peer {
                        Some(p) => format!("far end {p}"),
                        None => String::new(),
                    }),
                ]);
            }
            t.grid(g);
        }
        if let Some(c) = &self.catalog {
            t.line(format!(
                "catalog: the far side holds {}, the policy names {}",
                c.held,
                c.policy.as_deref().unwrap_or("none")
            ));
        }
        if !self.comparison.findings.is_empty() {
            t.line(format!(
                "against the plan: {} difference(s)",
                self.comparison.findings.len()
            ));
            let mut g = Grid::unheaded(3);
            for f in &self.comparison.findings {
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
        let mut notes: Vec<Note> = self
            .findings
            .iter()
            .map(|f| {
                let says = zenkey_fleet::link_says(f.code);
                // A finding of the whole block, as `narrowed_too_far` is,
                // names no face (link.v1 §2.15, 0.4).
                Note::caveat(match &f.face {
                    Some(face) => format!("{} [face {face}] {}: {says}", f.code, f.at),
                    None => format!("{} {}: {says}", f.code, f.at),
                })
            })
            .collect();
        notes.push(
            Note::coverage(format!(
                "judged against the config file {} through zenoh's own loader: what a router \
                 runs is not observable on the bus, so a file that differs from what it was \
                 started with is invisible here",
                self.against
            ))
            .cite("core §11.3, link.v1 §2.14"),
        );
        notes.push(match self.windows.as_option() {
            None => Note::coverage(
                "tombstone windows not asked: pass --window ADDRESS=SECS for each listed owner \
                 of state (link.v1 §2.11)",
            ),
            Some(_) => Note::coverage(
                "the windows are the ones given: whether each owner runs with its window is its \
                 configuration, not on the bus (link.v1 §2.11, §5)",
            ),
        });
        if !self.transport.is_empty() {
            if self.transport.iter().all(|k| k.peer.is_none()) {
                notes.push(Note::coverage(
                    "the far end not asked: pass --peer-config <FAR-CONFIG>, the far router's or \
                     the gateway client's config, to compare its lease and keep_alive, which are \
                     not negotiated (link.v2 §2.16, §2.19)",
                ));
            }
            if self.catalog.is_none() {
                notes.push(Note::coverage(
                    "the catalog not asked: pass --catalog <FILE>, the catalog the far side holds \
                     (link.v2 §2.19)",
                ));
            }
        }
        notes
    }
}
