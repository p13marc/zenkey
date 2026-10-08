//! zk2's families (#612, FJ4): presence, interfaces, contracts and the
//! binding graph, as the fleet's zk2 core reports them.
//!
//! Three honesty rules run through all of them, and each has a place in the
//! rendering rather than in prose somebody may forget:
//!
//! * **completeness rides every presence read** (spec §8.1): a liveliness
//!   GET that ran to its timeout is possibly incomplete, and a service or an
//!   interface missing from it may still be up — a coverage note, in every
//!   format, never a quiet empty table;
//! * **a token and a descriptor are two sources** (§3.3): a row shows the
//!   token's fingerprint prefix and the descriptor's fingerprint side by
//!   side, and "tokenless" (U22) is a different cell from "no token";
//! * **not asked is not absent** (O4): a descriptor that was not read, a
//!   contract that was not looked for, an exposure that could not be
//!   computed render as `—`, never as nothing.

use std::collections::BTreeSet;

use zenkey_fleet::report::{
    Asked, BindingGraph, CompatClass, CompatReport, ContractAnswer, ContractView, DescriptorAnswer,
    IfaceListing, IfaceSighting, IfaceView, InstanceRef, InstanceSighting, NamespaceListing,
    ResourceBody, ResourceView, SchemaView, ServiceListing, ServiceView, TypeView,
};
use zenkey_model::descriptor::Descriptor;

use crate::render::style;
use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

// ── shared pieces ──────────────────────────────────────────────────────────

/// The note every presence read owes when it ran to its timeout.
fn incomplete() -> Note {
    Note::coverage(
        "the presence read ran to its timeout, so it is possibly incomplete: \
         anything missing here may still be up — raise --timeout to ask again",
    )
    .cite("spec §8.1")
}

/// The note for keys under `@zk` that are not zk2 tokens.
fn unparsed_note(keys: &[String]) -> Option<Note> {
    (!keys.is_empty()).then(|| {
        Note::caveat(format!(
            "{} key(s) under @zk are not zk2 tokens, shown as found: {}",
            keys.len(),
            keys.join(", ")
        ))
    })
}

/// The note for instances whose descriptor did not read.
fn undescribed_note(undescribed: &[InstanceRef]) -> Option<Note> {
    (!undescribed.is_empty()).then(|| {
        Note::coverage(format!(
            "{} instance(s) did not serve a readable descriptor ({}): their roles, and \
             any interface they provide without a token, are missing here",
            undescribed.len(),
            undescribed
                .iter()
                .map(|i| format!("{}@{} {}", i.address, i.instance, i.descriptor))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .cite("spec §8.1")
    })
}

/// A fingerprint short enough for a table: `sha256:` and its first 16 hex
/// digits — the token's prefix, so the two line up. The document carries
/// the whole of it.
pub(super) fn short_fp(fp: &str) -> String {
    let hex = fp.strip_prefix("sha256:").unwrap_or(fp);
    if hex.len() > 16 {
        format!("sha256:{}…", &hex[..16])
    } else {
        fp.to_owned()
    }
}

/// The token cell: its prefix, or why there is none.
fn token_cell(token: Option<&str>, tokenless: bool) -> Cell {
    match (token, tokenless) {
        (Some(t), _) => Cell::text(t),
        (None, true) => Cell::text("tokenless"),
        (None, false) => Cell::styled("no token", style::WARNING),
    }
}

/// A descriptor answer as one word, `—` when it was not asked.
fn descriptor_cell(d: &Asked<DescriptorAnswer>) -> Cell {
    match d {
        Asked::NotAsked => Cell::Unknown,
        Asked::Asked(DescriptorAnswer::Served { .. }) => Cell::text("descriptor served"),
        Asked::Asked(DescriptorAnswer::Silent) => {
            Cell::styled("descriptor silent", style::UNPROVEN)
        }
        Asked::Asked(DescriptorAnswer::Invalid { .. }) => {
            Cell::styled("descriptor invalid", style::ERROR)
        }
        Asked::Asked(DescriptorAnswer::Failed { .. }) => {
            Cell::styled("descriptor not asked: GET failed", style::UNPROVEN)
        }
    }
}

fn type_text(t: &TypeView) -> String {
    match t.kind.as_str() {
        "jsonschema" => format!("json:{}", t.name),
        "raw" => format!("raw {}", t.name),
        _ => t.name.clone(),
    }
}

fn spelled<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// One instance's interface rows, indented under it.
fn iface_rows(grid: &mut Grid, rows: &[IfaceSighting], descriptor_served: bool) {
    for r in rows {
        let contract = match &r.contract {
            Some(c) => Cell::text(short_fp(c)),
            // Served and silent about it is a fact; never read is not.
            None if descriptor_served => Cell::styled("not in descriptor", style::WARNING),
            None => Cell::Unknown,
        };
        grid.row([
            Cell::text(format!("    {}", r.iface)),
            token_cell(r.token.as_deref(), r.tokenless),
            contract,
            match r.minor {
                Some(m) => Cell::text(format!("minor {m}")),
                None => Cell::text(""),
            },
        ]);
    }
}

fn served(d: &Asked<DescriptorAnswer>) -> Option<&Descriptor> {
    match d {
        Asked::Asked(DescriptorAnswer::Served { descriptor }) => Some(descriptor),
        _ => None,
    }
}

/// An instance's rows: the instance, then its interfaces.
fn instance_rows(grid: &mut Grid, i: &InstanceSighting) {
    grid.row([
        Cell::text(format!("  instance {}", i.instance)),
        if i.instance_token {
            Cell::text("")
        } else {
            Cell::styled("no instance token", style::WARNING)
        },
        descriptor_cell(&i.descriptor),
        Cell::text(""),
    ]);
    iface_rows(grid, &i.interfaces, served(&i.descriptor).is_some());
}

/// An instance as an ndjson row, tagged with its address.
fn instance_row(address: &str, i: &InstanceSighting) -> Row {
    let mut v = serde_json::to_value(i).expect("an instance serializes");
    if let serde_json::Value::Object(m) = &mut v {
        m.insert("address".into(), address.into());
    }
    Row::tagged("instance", v)
}

fn presence_scope(selector: &str) -> Option<ObservedScope> {
    Some(ObservedScope {
        asked: vec![selector.to_owned()],
        window_s: None,
    })
}

// ── service list ───────────────────────────────────────────────────────────

impl Render for ServiceListing {
    const FAMILY: &'static str = "service-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["services"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.services {
            for i in &s.instances {
                out(instance_row(&s.address, i));
            }
            for m in &s.members {
                let mut v = serde_json::to_value(m).expect("a member serializes");
                if let serde_json::Value::Object(o) = &mut v {
                    o.insert("address".into(), s.address.clone().into());
                }
                out(Row::tagged("member", v));
            }
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(4).max(0, 48);
        for s in &self.services {
            grid.group(&s.address);
            for i in &s.instances {
                instance_rows(&mut grid, i);
            }
            if !s.members.is_empty() {
                grid.detail(
                    s.members
                        .iter()
                        .map(|m| format!("  member {} {} epoch {}", m.iface, m.member, m.epoch)),
                );
            }
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.services.is_empty() {
            notes.push(Note::silence(
                "no zk2 service holds a token here. An empty read is not a verdict: \
                 check --namespace (`zenctl namespace list` shows the namespaces in \
                 use) and --connect",
            ));
        } else {
            let instances: usize = self.services.iter().map(|s| s.instances.len()).sum();
            notes.push(Note::summary(format!(
                "{} service(s), {instances} instance(s).",
                self.services.len()
            )));
            notes.push(Note::next_step(
                "one service in full: zenctl service show <system>/<service>",
            ));
        }
        notes.extend(unparsed_note(&self.unparsed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        presence_scope(&self.selector)
    }
}

// ── service show ───────────────────────────────────────────────────────────

impl Render for ServiceView {
    const FAMILY: &'static str = "service-show";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["instances", "members"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for i in &self.instances {
            out(instance_row(&self.address, i));
        }
        for m in &self.members {
            out(Row::of("member", m));
        }
    }

    fn table(&self, t: &mut Table) {
        for i in &self.instances {
            t.blank();
            let mut head = format!("{}  instance {}", self.address, i.instance);
            if !i.instance_token {
                head.push_str("  (no instance token)");
            }
            t.line(head);
            match &i.descriptor {
                Asked::NotAsked => {
                    t.line("  descriptor: not asked");
                }
                Asked::Asked(DescriptorAnswer::Served { descriptor }) => {
                    descriptor_lines(t, descriptor, &i.interfaces);
                }
                Asked::Asked(DescriptorAnswer::Silent) => {
                    t.line_styled(
                        "  descriptor: silent — no reply within the timeout",
                        style::UNPROVEN,
                    );
                    let mut g = Grid::unheaded(4);
                    iface_rows(&mut g, &i.interfaces, false);
                    t.grid(g);
                }
                Asked::Asked(DescriptorAnswer::Invalid { findings }) => {
                    t.line_styled(format!("  descriptor: invalid — {findings}"), style::ERROR);
                }
                Asked::Asked(DescriptorAnswer::Failed { reason }) => {
                    t.line_styled(
                        format!("  descriptor: the GET failed — {reason}"),
                        style::UNPROVEN,
                    );
                }
            }
        }
        if !self.members.is_empty() {
            t.blank();
            t.line("members");
            let mut g = Grid::unheaded(3);
            for m in &self.members {
                g.row([
                    Cell::text(format!("  {}", m.iface)),
                    Cell::text(&m.member),
                    Cell::text(format!("epoch {}", m.epoch)),
                ]);
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.instances.is_empty() {
            notes.push(Note::silence(format!(
                "presence shows no instance of {}: it may be down, in another namespace, \
                 or not yet up — silence is not a verdict, and this exits 2",
                self.address
            )));
        }
        notes.extend(unparsed_note(&self.unparsed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        presence_scope(&self.selector)
    }
}

/// A served descriptor, laid out: interfaces (with what the token says
/// beside them), capabilities, roles.
fn descriptor_lines(t: &mut Table, d: &Descriptor, rows: &[IfaceSighting]) {
    t.line(format!("  descriptor: served ({})", d.format));
    if !rows.is_empty() {
        t.line("  interfaces");
        let mut g = Grid::unheaded(4);
        for r in rows {
            g.row([
                Cell::text(format!("    {}", r.iface)),
                token_cell(r.token.as_deref(), r.tokenless),
                match &r.contract {
                    Some(c) => Cell::text(c),
                    None => Cell::styled("not in descriptor", style::WARNING),
                },
                match r.minor {
                    Some(m) => Cell::text(format!("minor {m}")),
                    None => Cell::text(""),
                },
            ]);
            if let Some(e) = d.interfaces.iter().find(|e| e.iface == r.iface) {
                let mut detail = Vec::new();
                for u in &e.unavailable {
                    detail.push(format!(
                        "      unavailable {} ({}){}",
                        u.resource,
                        spelled(&u.cause),
                        u.reason
                            .as_ref()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ));
                }
                for (resource, n) in &e.cardinality {
                    detail.push(format!("      cardinality {resource} ≤ {n}"));
                }
                if !detail.is_empty() {
                    g.detail(detail);
                }
            }
        }
        t.grid(g);
    }
    if !d.capabilities.is_empty() {
        t.line(format!("  capabilities: {}", d.capabilities.join(", ")));
    }
    if !d.profiles.is_empty() {
        t.line(format!("  profiles: {}", d.profiles.join(", ")));
    }
    if !d.requires.is_empty() {
        t.line("  requires");
        let mut g = Grid::unheaded(3);
        for r in &d.requires {
            g.row([
                Cell::text(format!("    {}", r.role)),
                Cell::text(&r.interface),
                Cell::text(format!("← {}", r.bindings.join(", "))),
            ]);
            let mut detail = Vec::new();
            if !r.params.is_empty() {
                detail.push(format!(
                    "      params {}",
                    r.params
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(by) = &r.declared_by {
                detail.push(format!("      declared by {by}"));
            }
            if !detail.is_empty() {
                g.detail(detail);
            }
        }
        t.grid(g);
    }
    if !d.meta.is_empty() {
        t.line(format!(
            "  meta: {}",
            serde_json::to_string(&d.meta).unwrap_or_default()
        ));
    }
}

// ── iface list ─────────────────────────────────────────────────────────────

impl Render for IfaceListing {
    const FAMILY: &'static str = "iface-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["interfaces", "undescribed"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for i in &self.interfaces {
            out(Row::of("iface", i));
        }
        for u in &self.undescribed {
            out(Row::of("undescribed", u));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::new(["INTERFACE", "PROVIDERS", "CONSUMERS", "REVISIONS"]).max(1, 48);
        for i in &self.interfaces {
            let mut providers = i.providers.join(", ");
            if i.tokenless {
                providers.push_str("  [tokenless]");
            }
            g.row([
                Cell::text(&i.iface),
                Cell::text(providers),
                Cell::text(i.consumers.join(", ")),
                Cell::text(
                    i.revisions
                        .iter()
                        .map(|r| short_fp(r))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.interfaces.is_empty() {
            notes.push(Note::silence(
                "no interface is provided or required here. An empty read is not a \
                 verdict: check --namespace and --connect",
            ));
        } else {
            notes.push(Note::summary(format!(
                "{} interface(s).",
                self.interfaces.len()
            )));
            if self.interfaces.iter().any(|i| i.revisions.len() > 1) {
                notes.push(Note::caveat(
                    "an interface with more than one revision is a rolling upgrade or a \
                     split brain; telling them apart is `doctor`'s",
                ));
            }
            notes.push(Note::next_step(
                "one interface in full: zenctl iface show <iface>[@<fingerprint>]",
            ));
        }
        notes.extend(undescribed_note(&self.undescribed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        presence_scope(&self.selector)
    }
}

// ── iface show ─────────────────────────────────────────────────────────────

impl Render for IfaceView {
    const FAMILY: &'static str = "iface-show";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(
            self,
            &["providers", "consumers", "revisions", "undescribed"],
        )
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for p in &self.providers {
            out(Row::of("provider", p));
        }
        for c in &self.consumers {
            out(Row::of("consumer", c));
        }
        for r in &self.revisions {
            out(Row::of("revision", r));
        }
        for u in &self.undescribed {
            out(Row::of("undescribed", u));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(&self.iface);
        if !self.providers.is_empty() {
            t.blank();
            t.line("providers");
            let mut g = Grid::unheaded(4);
            for p in &self.providers {
                g.row([
                    Cell::text(format!("  {}@{}", p.address, p.instance)),
                    token_cell(p.token.as_deref(), p.tokenless),
                    Cell::asked(p.contract.as_deref().map(short_fp)),
                    match p.minor {
                        Some(m) => Cell::text(format!("minor {m}")),
                        None => Cell::text(""),
                    },
                ]);
                let mut detail = Vec::new();
                match &p.exposes {
                    Asked::Asked(e) => detail.push(format!("    exposes {}", e.join(", "))),
                    Asked::NotAsked => detail.push(
                        "    exposes — (needs its descriptor and its revision in hand)".into(),
                    ),
                }
                for u in &p.unavailable {
                    detail.push(format!(
                        "    unavailable {} ({}){}",
                        u.resource,
                        spelled(&u.cause),
                        u.reason
                            .as_ref()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ));
                }
                for (resource, n) in &p.cardinality {
                    detail.push(format!("    cardinality {resource} ≤ {n}"));
                }
                g.detail(detail);
            }
            t.grid(g);
        }
        if !self.consumers.is_empty() {
            t.blank();
            t.line("consumers");
            let mut g = Grid::unheaded(3);
            for c in &self.consumers {
                g.row([
                    Cell::text(format!("  {}@{}", c.address, c.instance)),
                    Cell::text(format!("role {}", c.role)),
                    Cell::text(format!("← {}", c.bindings.join(", "))),
                ]);
                let mut detail = Vec::new();
                if !c.params.is_empty() {
                    detail.push(format!(
                        "    params {}",
                        c.params
                            .iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if let Some(by) = &c.declared_by {
                    detail.push(format!("    declared by {by}"));
                }
                if !detail.is_empty() {
                    g.detail(detail);
                }
            }
            t.grid(g);
        }
        for r in &self.revisions {
            t.blank();
            t.line(format!("revision {}", r.fingerprint));
            if !r.providers.is_empty() {
                t.line(format!("  named by {}", r.providers.join(", ")));
            }
            match &r.contract {
                Asked::NotAsked => {
                    t.line("  contract: not asked");
                }
                Asked::Asked(ContractAnswer::Held { source, contract }) => {
                    t.line(format!("  contract: held ({})", spelled(source)));
                    contract_lines(t, contract);
                }
                Asked::Asked(ContractAnswer::Unavailable { refused }) => {
                    let why = if refused.is_empty() {
                        "no holder answered".to_owned()
                    } else {
                        format!("refused replies: {}", refused.join(", "))
                    };
                    t.line_styled(format!("  contract: unavailable — {why}"), style::WARNING);
                }
                Asked::Asked(ContractAnswer::Unreadable { reason }) => {
                    t.line_styled(format!("  contract: unreadable — {reason}"), style::ERROR);
                }
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.providers.is_empty() && self.consumers.is_empty() {
            notes.push(Note::silence(format!(
                "presence shows nobody providing or requiring {}. Silence is not a \
                 verdict: check --namespace, and `zenctl iface list`",
                self.iface
            )));
        }
        notes.extend(undescribed_note(&self.undescribed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        // The view keeps no selector of its own: it is always the whole
        // namespace's presence (a tokenless provider has no interface token
        // to scope by, spec §8.1).
        presence_scope("zk2/*/*/@zk/**")
    }
}

/// One contract revision, laid out: resources with their kinds, types and
/// QoS, roles, schema artifacts.
fn contract_lines(t: &mut Table, c: &ContractView) {
    if let Some(s) = &c.summary {
        t.line(format!("  {s}"));
    }
    if !c.uses.is_empty() {
        t.line(format!("  uses {}", c.uses.join(", ")));
    }
    t.line("  resources");
    let mut g = Grid::unheaded(4);
    for r in &c.resources {
        g.row([
            Cell::text(format!("    {}", r.name)),
            Cell::text(spelled(&r.kind)),
            Cell::text(resource_types(r)),
            Cell::text(resource_qos(r)),
        ]);
        let mut detail = Vec::new();
        let mut flags = Vec::new();
        if r.optional {
            flags.push("optional".to_owned());
        }
        if !r.gate.is_empty() {
            flags.push(format!("gate {}", r.gate.join(" & ")));
        }
        if let Some(n) = r.cardinality {
            flags.push(format!("cardinality ≤ {n}"));
        }
        if let Some(e) = &r.epoch {
            flags.push(format!("epoch {{{e}}}"));
        }
        if !flags.is_empty() {
            detail.push(format!("      {}", flags.join(", ")));
        }
        if let Some(d) = &r.deprecated {
            detail.push(format!(
                "      deprecated: {}",
                serde_json::to_string(d).unwrap_or_default()
            ));
        }
        if let Some(doc) = &r.doc {
            detail.push(format!("      {doc}"));
        }
        if !detail.is_empty() {
            g.detail(detail);
        }
    }
    t.grid(g);
    if !c.requires.is_empty() {
        t.line("  requires");
        let mut g = Grid::unheaded(3);
        for r in &c.requires {
            g.row([
                Cell::text(format!("    {}", r.role)),
                Cell::text(&r.interface),
                Cell::text(format!(
                    "{}{}",
                    spelled(&r.cardinality),
                    if r.optional { ", optional" } else { "" }
                )),
            ]);
        }
        t.grid(g);
    }
    if !c.schemas.is_empty() {
        t.line("  schemas");
        let mut g = Grid::unheaded(3);
        for s in &c.schemas {
            g.row([
                Cell::text(format!("    {}", s.name)),
                Cell::text(&s.kind),
                Cell::text(&s.id),
            ]);
        }
        t.grid(g);
    }
}

fn resource_types(r: &ResourceView) -> String {
    match &r.body {
        ResourceBody::Data {
            type_, attachment, ..
        } => match attachment {
            Some(a) => format!("{} + {}", type_text(type_), type_text(a)),
            None => type_text(type_),
        },
        ResourceBody::Operation {
            request,
            response,
            error,
            ..
        } => {
            let mut s = format!("{} → {}", type_text(request), type_text(response));
            if let Some(e) = error {
                s.push_str(&format!(" | {}", type_text(e)));
            }
            s
        }
    }
}

fn resource_qos(r: &ResourceView) -> String {
    match &r.body {
        ResourceBody::Data {
            reliability,
            congestion,
            priority,
            express,
            history,
            rate,
            retention_s,
            ..
        } => {
            let mut parts = vec![spelled(reliability), spelled(congestion), spelled(priority)];
            if *express {
                parts.push("express".into());
            }
            if let Some(h) = history {
                parts.push(format!(
                    "history {}",
                    serde_json::to_string(h).unwrap_or_default()
                ));
            }
            if let Some(rate) = rate {
                parts.push(format!("rate {rate}"));
            }
            if let Some(s) = retention_s {
                parts.push(format!("retention {s}s"));
            }
            parts.join(" ")
        }
        ResourceBody::Operation {
            idempotent,
            fanout,
            serving,
            replies,
            timeout_ms,
            priority,
            ..
        } => {
            let mut parts = vec![
                format!("fanout {}", spelled(fanout)),
                format!("serving {}", spelled(serving)),
                format!("replies {}", spelled(replies)),
                spelled(priority),
            ];
            if *idempotent {
                parts.push("idempotent".into());
            }
            if let Some(ms) = timeout_ms {
                parts.push(format!("timeout {ms}ms"));
            }
            parts.join(" ")
        }
    }
}

// ── schema show ────────────────────────────────────────────────────────────

impl Render for SchemaView {
    const FAMILY: &'static str = "schema-show";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["members", "artifacts"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for m in &self.members {
            out(Row::of("member", m));
        }
        for a in &self.artifacts {
            out(Row::of("artifact", a));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "{} {}  ({})",
            self.iface,
            self.fingerprint,
            spelled(&self.source)
        ));
        let mut g = Grid::unheaded(4);
        for m in &self.members {
            g.row([
                Cell::text(format!("  {}", m.resource)),
                Cell::text(&m.member),
                Cell::text(type_text(&m.type_)),
                // A raw type names no schema: asked, and there is none —
                // an empty cell, never the not-asked dash.
                Cell::text(m.type_.schema.as_deref().map(short_fp).unwrap_or_default()),
            ]);
        }
        t.grid(g);
        for a in &self.artifacts {
            t.blank();
            t.line(format!("{} {}  {}", a.kind, a.name, a.id));
            if let Asked::Asked(doc) = &a.document {
                let lines = if a.kind == "protobuf" {
                    proto_lines(doc)
                } else {
                    serde_json::to_string_pretty(doc)
                        .unwrap_or_default()
                        .lines()
                        .map(str::to_owned)
                        .collect()
                };
                for l in lines {
                    t.line(format!("  {l}"));
                }
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.members.iter().any(|m| m.type_.kind == "raw") {
            notes.push(Note::caveat(
                "a raw type names a media type and no schema: its bytes are not a \
                 tool's to read (spec §7.2)",
            ));
        }
        if self.artifacts.iter().all(|a| a.document.is_not_asked()) && !self.artifacts.is_empty() {
            notes.push(Note::next_step(
                "the documents themselves: add --full, or name a resource",
            ));
        }
        notes
    }
}

/// A protobuf artifact's summary, spelled like the `.proto` it came from.
fn proto_lines(doc: &serde_json::Value) -> Vec<String> {
    if let Some(e) = doc.get("unreadable").and_then(|e| e.as_str()) {
        return vec![format!("// the descriptor set does not decode: {e}")];
    }
    let mut out = Vec::new();
    for f in doc["files"].as_array().into_iter().flatten() {
        out.push(format!(
            "// {} (package {})",
            f["name"].as_str().unwrap_or("?"),
            f["package"].as_str().unwrap_or("")
        ));
        for m in f["messages"].as_array().into_iter().flatten() {
            let name = m["name"].as_str().unwrap_or("?");
            if let Some(values) = m["values"].as_array() {
                out.push(format!("enum {name} {{"));
                for v in values {
                    out.push(format!(
                        "  {} = {};",
                        v["name"].as_str().unwrap_or("?"),
                        v["number"]
                    ));
                }
                out.push("}".into());
                continue;
            }
            out.push(format!("message {name} {{"));
            for fld in m["fields"].as_array().into_iter().flatten() {
                let label = fld["label"]
                    .as_str()
                    .map(|l| format!("{l} "))
                    .unwrap_or_default();
                let oneof = fld["oneof"]
                    .as_str()
                    .map(|o| format!("  // oneof {o}"))
                    .unwrap_or_default();
                out.push(format!(
                    "  {label}{} {} = {};{oneof}",
                    fld["type"].as_str().unwrap_or("?"),
                    fld["name"].as_str().unwrap_or("?"),
                    fld["number"]
                ));
            }
            out.push("}".into());
        }
        for e in f["enums"].as_array().into_iter().flatten() {
            out.push(format!("enum {} {{", e["name"].as_str().unwrap_or("?")));
            for v in e["values"].as_array().into_iter().flatten() {
                out.push(format!(
                    "  {} = {};",
                    v["name"].as_str().unwrap_or("?"),
                    v["number"]
                ));
            }
            out.push("}".into());
        }
    }
    out
}

// ── namespace list ─────────────────────────────────────────────────────────

/// How the bus-root deployment is spelled in a table: `(empty)`, as v1's
/// base listing spelled it, never a blank cell that reads as "unknown".
fn namespace_word(ns: &str) -> String {
    if ns.is_empty() {
        "(empty)".to_owned()
    } else {
        ns.to_owned()
    }
}

impl Render for NamespaceListing {
    const FAMILY: &'static str = "namespace-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["namespaces"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for n in &self.namespaces {
            out(Row::of("namespace", n));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(4).max(3, 60);
        for n in &self.namespaces {
            g.row([
                Cell::text(namespace_word(&n.namespace)),
                Cell::text(format!("{} service(s)", n.services.len())),
                Cell::text(format!("{} instance(s)", n.instances)),
                Cell::text(n.services.join(", ")),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.namespaces.is_empty() {
            notes.push(Note::silence(
                "no zk2 instance token anywhere this session reaches. An empty read is \
                 not a verdict: check --connect",
            ));
        } else {
            notes.push(Note::next_step(
                "read one with --namespace <namespace>; the bus root (empty) is --namespace ''",
            ));
        }
        notes.extend(unparsed_note(&self.unparsed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        presence_scope(&self.selector)
    }
}

// ── graph ──────────────────────────────────────────────────────────────────

/// The roles with no edge: `(consumer, role, interface, bindings)`.
fn unbound(g: &BindingGraph) -> Vec<(&str, &str, &str, String)> {
    let bound: BTreeSet<(&str, &str)> = g
        .edges
        .iter()
        .map(|e| (e.consumer.as_str(), e.role.as_str()))
        .collect();
    g.nodes
        .iter()
        .flat_map(|n| {
            n.requires.iter().map(move |(role, r)| {
                (
                    n.address.as_str(),
                    role.as_str(),
                    r.interface.as_str(),
                    r.bindings.join(", "),
                )
            })
        })
        .filter(|(c, r, ..)| !bound.contains(&(*c, *r)))
        .collect()
}

impl Render for BindingGraph {
    const FAMILY: &'static str = "graph";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["nodes", "edges", "undescribed"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for n in &self.nodes {
            out(Row::of("node", n));
        }
        for e in &self.edges {
            out(Row::of("edge", e));
        }
        for u in &self.undescribed {
            out(Row::of("undescribed", u));
        }
    }

    fn table(&self, t: &mut Table) {
        if !self.edges.is_empty() {
            t.line("bindings");
            let mut g = Grid::unheaded(4);
            for e in &self.edges {
                g.row([
                    Cell::text(format!("  {}", e.consumer)),
                    Cell::text(&e.role),
                    Cell::text(&e.interface),
                    Cell::text(format!("→ {}", e.provider)),
                ]);
            }
            t.grid(g);
        }
        let unbound = unbound(self);
        if !unbound.is_empty() {
            t.blank();
            t.line("roles that select no provider present now");
            let mut g = Grid::unheaded(4);
            for (consumer, role, iface, bindings) in &unbound {
                g.row([
                    Cell::text(format!("  {consumer}")),
                    Cell::text(*role),
                    Cell::text(*iface),
                    Cell::styled(format!("← {bindings}"), style::UNPROVEN),
                ]);
            }
            t.grid(g);
        }
        if !self.nodes.is_empty() {
            t.blank();
            t.line("services");
            let mut g = Grid::unheaded(3);
            for n in &self.nodes {
                g.row([
                    Cell::text(format!("  {}", n.address)),
                    Cell::text(format!("{} instance(s)", n.instances)),
                    Cell::text(if n.provides.is_empty() {
                        "provides nothing".to_owned()
                    } else {
                        format!("provides {}", n.provides.join(", "))
                    }),
                ]);
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if !self.complete {
            notes.push(incomplete());
        }
        if self.nodes.is_empty() {
            notes.push(Note::silence(
                "no zk2 service holds a token here, so there is no graph. An empty read \
                 is not a verdict: check --namespace and --connect",
            ));
        } else {
            notes.push(Note::summary(format!(
                "{} service(s), {} binding(s).",
                self.nodes.len(),
                self.edges.len()
            )));
            notes.push(Note::caveat(
                "edges come from declared bindings and present providers, never from \
                 traffic; a role with no edge is shown, and judging it is `doctor`'s",
            ));
        }
        notes.extend(undescribed_note(&self.undescribed));
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        presence_scope("zk2/*/*/@zk/**")
    }
}

/// The graph as Graphviz: a box per service, an edge per binding drawn the
/// way the data flows (provider → consumer, labelled with the consumer's
/// role and the interface), and each role that selects nothing as a dashed
/// edge from a point standing for its unmatched bindings.
pub fn graph_dot(g: &BindingGraph) -> String {
    fn q(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
    let mut out = String::from("digraph zk2 {\n  rankdir=LR;\n  node [shape=box];\n");
    for n in &g.nodes {
        let mut label = n.address.clone();
        if n.instances != 1 {
            label.push_str(&format!("\\n{} instances", n.instances));
        }
        if !n.provides.is_empty() {
            label.push_str(&format!("\\n{}", n.provides.join(", ")));
        }
        out.push_str(&format!(
            "  {} [label=\"{}\"];\n",
            q(&n.address),
            label.replace('"', "\\\"")
        ));
    }
    for e in &g.edges {
        out.push_str(&format!(
            "  {} -> {} [label={}];\n",
            q(&e.provider),
            q(&e.consumer),
            q(&format!("{} ({})", e.role, e.interface))
        ));
    }
    for (i, (consumer, role, iface, bindings)) in unbound(g).into_iter().enumerate() {
        let id = format!("unbound{i}");
        out.push_str(&format!(
            "  {id} [shape=point];\n  {id} -> {} [style=dashed, label={}];\n",
            q(consumer),
            q(&format!("{role} ({iface}) ← {bindings}: no provider"))
        ));
    }
    if !g.complete {
        out.push_str("  label=\"possibly incomplete: the presence read ran to its timeout\";\n");
    }
    out.push_str("}\n");
    out
}

// ── compat ─────────────────────────────────────────────────────────────────

fn class_style(c: CompatClass) -> anstyle::Style {
    match c {
        CompatClass::Compatible => style::PASS,
        CompatClass::Review => style::WARNING,
        CompatClass::Breaking => style::ERROR,
    }
}

impl Render for CompatReport {
    const FAMILY: &'static str = "compat";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["findings", "warnings"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.findings {
            out(Row::of("finding", f));
        }
        for w in &self.warnings {
            out(Row::of("warning", w));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(5);
        for (word, s) in [("old", &self.old), ("new", &self.new)] {
            g.row([
                Cell::text(word),
                Cell::text(&s.iface),
                Cell::text(short_fp(&s.fingerprint)),
                Cell::text(spelled(&s.source)),
                Cell::text(&s.input),
            ]);
        }
        t.grid(g);
        if !self.findings.is_empty() || !self.warnings.is_empty() {
            t.blank();
            let mut g = Grid::unheaded(3);
            for f in &self.findings {
                g.row([
                    Cell::styled(f.class.as_str(), class_style(f.class)),
                    Cell::text(&f.rule),
                    Cell::text(&f.at),
                ]);
                g.detail([format!("  {}", f.detail)]);
            }
            for w in &self.warnings {
                g.row([
                    Cell::styled("warning", style::WARNING),
                    Cell::text(&w.rule),
                    Cell::text(&w.at),
                ]);
                g.detail([format!("  {}", w.detail)]);
            }
            t.grid(g);
        }
        t.blank();
        t.line_styled(self.class.as_str(), class_style(self.class));
    }

    fn notes(&self) -> Vec<Note> {
        let exit = match self.class {
            CompatClass::Compatible => "exit 0",
            CompatClass::Review => "exit 1: a human accepts a review change",
            CompatClass::Breaking => "exit 1: a breaking change needs a new major",
        };
        let mut notes = vec![Note::summary(format!(
            "{} finding(s), {} warning(s); {exit}.",
            self.findings.len(),
            self.warnings.len()
        ))];
        if self.old.iface != self.new.iface {
            notes.push(Note::caveat(format!(
                "the two revisions are of different interfaces ({} and {}): that alone \
                 is breaking",
                self.old.iface, self.new.iface
            )));
        }
        notes
    }
}
