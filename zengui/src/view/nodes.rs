//! The node dashboard (#61; zk2's since #614, FL9): one card per address,
//! fed by presence (free — the presence link's own monitor) and the
//! watched-only freshness join; health loads on selection, through one
//! `health.v1` reading, and so does who serves each governed interface,
//! through one `redundancy.v1` reading (#841, RC) — the pane's only
//! data-plane costs (the laziness ground rule).
//!
//! v1's roster keyed alive tokens by origin and producer. zk2 has neither:
//! a service is an **address**, `<system>/<service>`, run as instances that
//! hold an instance token (core §8.1), and what each instance is comes from
//! its descriptor (§3.3), never from the shape of its name.

use std::sync::Arc;
use std::time::SystemTime;

use iced::widget::{Column, column, row, scrollable};
use iced::{Element, Length};
use zenkey_fleet::report::{
    ClockGround, HealthAnswer, HealthAnswerToken, HealthReport, HealthRow, HealthVerdict,
    LevelRead, RedundancyAnswer, RedundancyAnswerToken, RedundancyInstance, RedundancyReport,
    RedundancyRole,
};
use zenkey_fleet::{AddressStatus, LiveAddress, PresenceBasis};
use zenkey_model::grammar::Addr;

use crate::message::{Message, PaneMsg, Subject, SubjectMsg};
use crate::nodes::{Described, HealthState, NodesState, RedundancyState};
use crate::presence_link::PresenceUpdate;
use crate::view::kit;
use crate::view::theme::{PresenceTone, Tone};
use crate::view::tokens::{Spacing, face, font};

/// The pane's interactions, nested per the `CallMsg` precedent.
#[derive(Debug, Clone)]
pub enum NodesMsg {
    /// The selected address's `health.v1` reading landed: the address, the
    /// namespace it ran in (the staleness guard, #109), and the engine's
    /// report.
    HealthRead(Addr, String, Arc<HealthReport>),
    /// Read the selected address's health again — one more reading, asked.
    AskHealth,
    /// The selected address's `redundancy.v1` reading landed (#841, RC):
    /// the address, the namespace it ran in, and the engine's report.
    RedundancyRead(Addr, String, Arc<RedundancyReport>),
    /// Read who serves the selected address again — one more reading.
    AskRedundancy,
    /// The selection settled: the address, and the selection's count
    /// ([`NodesState::due_redundancy`]). Only the latest asks the reading.
    RedundancySettled(Addr, u64),
    /// The "clocks synced" toggle (D11): the operator's word that this
    /// host's clock and the owners' agree. Re-asks the selected address.
    ClocksSynced(bool),
    /// Click-through: land on the address's subtree in the key tree.
    ShowInTree(Addr),
}

/// Everything the pane renders (args struct per the `DetailData` precedent).
pub struct NodesData<'a> {
    /// The presence link's last landing; `None` = not read (O4).
    pub presence: Option<&'a PresenceUpdate>,
    /// Descriptors, freshness and health.
    pub nodes: &'a NodesState,
    /// The address the workspace points at, when it points at one.
    pub selected: Option<&'a Addr>,
    /// The dock's resolved spacing grid (#192).
    pub sp: Spacing,
}

/// Wrap one of this pane's messages for the app (#176).
fn msg(m: NodesMsg) -> Message {
    Message::Pane(PaneMsg::Nodes(m))
}

pub fn pane(d: NodesData<'_>) -> Element<'_, Message> {
    let sp = d.sp;
    let mut col = column![kit::section_header("Nodes", None)].spacing(sp.sm);

    let Some(update) = d.presence else {
        col = col.push(kit::empty(
            kit::EmptyKind::NotAsked,
            "presence not read",
            "the nodes pane reads zk2's instance tokens on the session in the \
             namespace, and no read has landed — not asked is not an empty \
             deployment (O4)",
        ));
        return col.into();
    };
    let presence = &update.presence;
    // The strip's own sentence (one wording): counts, and the basis when it
    // is not complete — a loss as a caution, never as everything down.
    let (said, caution) = crate::view::status::presence_text(Some(update));
    col = col.push(if caution {
        kit::status_chip(Tone::Caution, said)
    } else {
        kit::muted(said)
    });

    if presence.addresses().next().is_none() {
        col = col.push(match presence.basis() {
            PresenceBasis::Complete => kit::empty(
                kit::EmptyKind::Empty,
                "no zk2 service holds a token here",
                "a complete presence read of this namespace found no instance \
                 token — a reading of this namespace, so check it is the right \
                 one before reading a dead deployment",
            ),
            _ => kit::empty(
                kit::EmptyKind::Silent,
                "no token observed",
                "the read was not complete, so nothing here is down: silence is \
                 not a verdict (§8.1)",
            ),
        });
        return col.into();
    }

    // MD between cards (#192): the rule, not a taste — SM is for inside one.
    let mut cards = column![].spacing(sp.md);
    for (addr, live) in presence.addresses() {
        let selected = d.selected == Some(addr);
        cards = cards.push(address_card(addr, live, presence.basis(), selected, &d));
    }
    col = col.push(scrollable(cards).height(Length::Fill));
    col.into()
}

fn address_card<'a>(
    addr: &'a Addr,
    live: &'a LiveAddress,
    basis: PresenceBasis,
    selected: bool,
    d: &NodesData<'a>,
) -> Element<'a, Message> {
    let sp = d.sp;
    let mut header = row![
        kit::link(kit::emphasis(addr.to_string()).font(face::MONO))
            .padding(iced::Padding::ZERO)
            // Straight to the workspace's subject (#181): a card is one of
            // several ways to point the window at an address. Re-click
            // deselects.
            .on_press(Message::Subject(SubjectMsg::Select(if selected {
                Subject::None
            } else {
                Subject::Address(addr.clone())
            }))),
        iced::widget::space::horizontal(),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if let Some(configure) = configure_button(addr, d.nodes, sp) {
        header = header.push(configure);
    }
    let header = header.push(
        kit::secondary(kit::caption("show in tree"))
            .padding([0.0, sp.xs])
            .on_press(msg(NodesMsg::ShowInTree(addr.clone()))),
    );
    let mut body = column![header, presence_row(addr, live, basis, d.nodes, sp)].spacing(sp.xs);
    if selected {
        body = body.push(detail_section(addr, d.presence, d.nodes, sp));
    }
    kit::card(body)
}

/// "configure", where a served descriptor of the address lists `config.v1`
/// (§2.1: the interface is the gate) — a link to a form nothing serves
/// would be a promise this window cannot keep (#768, CD). Shared by the
/// card and the Inspector's address view.
fn configure_button<'a>(
    addr: &Addr,
    nodes: &NodesState,
    sp: Spacing,
) -> Option<Element<'a, Message>> {
    nodes.lists_config(addr).then(|| {
        kit::secondary(kit::labelled(kit::Icon::Config, "configure"))
            .padding([0.0, sp.xs])
            .on_press(Message::Pane(PaneMsg::Config(
                crate::view::configure::ConfigMsg::Open(addr.clone()),
            )))
            .into()
    })
}

/// One address's presence in a row: its status — up, down or unobservable,
/// never folded — since when, how many instances, and how fresh its state
/// is when a watch covers it.
///
/// Shared between the dashboard card and the Inspector's address section
/// (#182): one claim, one wording.
fn presence_row<'a>(
    addr: &Addr,
    live: &LiveAddress,
    basis: PresenceBasis,
    nodes: &NodesState,
    sp: Spacing,
) -> Element<'a, Message> {
    let tone = match live.status {
        AddressStatus::Up => PresenceTone::Up,
        AddressStatus::Down => PresenceTone::Down,
        AddressStatus::Unobservable => PresenceTone::Unobservable,
    };
    let since = ago(live.since);
    let status = match live.status {
        AddressStatus::Up => format!("since {since}"),
        AddressStatus::Down => format!("no instance token since {since}"),
        // Why it is not down, said: a loss, a read not made, a withdrawal
        // heard and the read it owes not yet made, or no complete seed
        // behind it (§8.1, 0.37).
        AddressStatus::Unobservable => match basis {
            PresenceBasis::Lost { .. } => {
                "events lost since the last complete read — not down".to_string()
            }
            PresenceBasis::CutOff => {
                "no read made: this window reaches no router — not down (§8.1)".to_string()
            }
            PresenceBasis::Moved => {
                "the watch session's link moved, awaiting a read — not down (§8.1)".to_string()
            }
            _ if live.withdrawn => "heard withdrawn, awaiting a read — not down (§8.1)".to_string(),
            _ => "no complete read behind it — not down (§8.1)".to_string(),
        },
    };
    let held = live.instance_tokens().count();
    let instances = match (held, live.instances.len()) {
        (0, 0) => None,
        (0, n) => Some(format!(
            "{} known from interface tokens alone",
            kit::plural(n, "instance")
        )),
        (1, _) => Some("1 instance".to_string()),
        // A re-mint's overlap, a redundant pair, or a split brain: telling
        // them apart is the doctor's and the redundancy reading's (#841,
        // RC), not this row's.
        (n, _) => Some(format!(
            "{} — an overlap, a pair or a split brain (the doctor and redundancy.v1 tell \
             them apart)",
            kit::plural(n, "instance")
        )),
    };
    let fresh = nodes.freshness(addr);
    let freshness = match (fresh.watched, fresh.last_state_age) {
        (true, Some(age)) => format!("last state sample {}s ago", age.as_secs()),
        (true, None) => "watched — no state sample seen".to_string(),
        (false, _) => "state not watched — freshness unknown".to_string(),
    };
    let mut r = row![
        kit::badge_presence(tone, format!("{addr}: {}", live.status.as_str())),
        kit::muted(status),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center);
    if let Some(instances) = instances {
        r = r.push(kit::muted(instances));
    }
    r.push(iced::widget::space::horizontal())
        .push(kit::muted(freshness))
        .into()
}

/// The Inspector's presence sections for one address (#182).
///
/// An address presence holds no record of is a *fact* and says so on the
/// basis it stands on — never an empty card reading as "no instances".
pub fn address_sections<'a>(
    addr: &'a Addr,
    presence: Option<&'a PresenceUpdate>,
    nodes: &'a NodesState,
    sp: Spacing,
) -> Column<'a, Message> {
    let mut col = column![].spacing(sp.xs);
    if let Some(configure) = configure_button(addr, nodes, sp) {
        col = col.push(configure);
    }
    let Some(update) = presence else {
        // Health reads presence itself, so it stands without the link's.
        return col
            .push(kit::empty(
                kit::EmptyKind::NotAsked,
                "presence not read",
                "no presence read has landed — whether this address is up is not \
                 asked yet (O4)",
            ))
            .push(health_section(addr, nodes, sp));
    };
    match update.presence.address(addr) {
        Some(live) => {
            col = col.push(presence_row(addr, live, update.presence.basis(), nodes, sp));
        }
        None => {
            let status = update.presence.status(addr);
            col = col.push(kit::badge_presence(
                match status {
                    AddressStatus::Down => PresenceTone::Down,
                    AddressStatus::Up => PresenceTone::Up,
                    AddressStatus::Unobservable => PresenceTone::Unobservable,
                },
                format!("{addr}: {}", status.as_str()),
            ));
            col = col.push(kit::muted(match status {
                AddressStatus::Down => {
                    "no token of it in a complete presence read of this namespace"
                }
                _ => "no token of it observed, and no complete read behind that — not down",
            }));
        }
    }
    col.push(detail_section(addr, presence, nodes, sp))
}

/// The selected address's detail: each instance as presence and its
/// descriptor describe it, and its `health.v1`, read on selection.
pub fn detail_section<'a>(
    addr: &Addr,
    presence: Option<&'a PresenceUpdate>,
    nodes: &'a NodesState,
    sp: Spacing,
) -> Column<'a, Message> {
    let mut col = column![kit::eyebrow("INSTANCES")].spacing(sp.xs);
    let live = presence.and_then(|p| p.presence.address(addr));
    let described = nodes.instances(addr);
    // Every instance either source names: presence's tokens and the
    // catalog's descriptors are read apart, and neither is folded into the
    // other.
    let mut ids: Vec<String> = live
        .map(|l| l.instances.keys().map(|i| i.to_string()).collect())
        .unwrap_or_default();
    for i in described.into_iter().flatten() {
        if !ids.contains(&i.instance) {
            ids.push(i.instance.clone());
        }
    }
    ids.sort();
    if ids.is_empty() {
        col = col.push(kit::muted(
            "no instance of it in presence or in the lens's catalog",
        ));
    }
    for id in &ids {
        let token = live.and_then(|l| {
            l.instances
                .iter()
                .find(|(i, _)| i.as_str() == id.as_str())
                .map(|(_, t)| t)
        });
        let held = match token {
            Some(t) if t.instance_token => match t.since {
                Some(since) => format!("instance token held since {}", ago(since)),
                None => "instance token held".to_string(),
            },
            Some(_) => "no instance token — interface tokens only (§8.2's order caught \
                        mid-way, or broken)"
                .to_string(),
            None => "not in the presence projection".to_string(),
        };
        col = col.push(
            row![kit::data_chip(id.clone()), kit::muted(held)]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center),
        );
        let facts = described
            .and_then(|all| all.iter().find(|i| &i.instance == id))
            .map(|i| &i.described);
        col = col.push(descriptor_lines(facts, sp));
        if let Some(roles) = role_chips(addr, id, nodes, sp) {
            col = col.push(roles);
        }
    }
    if let Some(l) = live
        && !l.members.is_empty()
    {
        col = col.push(kit::muted(format!(
            "{} held by this address",
            kit::plural(l.members.len(), "member token")
        )));
    }
    col.push(redundancy_section(addr, nodes, sp))
        .push(health_section(addr, nodes, sp))
}

/// One instance's role on each governed interface of the address, read
/// off the fleet's judge (`redundancy.v1` §2.15's fourth question, #841,
/// RC): serving, a standby, a twin — or why it cannot be told. `None` when
/// no reading of the address has landed, or it names the instance on no
/// governed interface.
fn role_chips<'a>(
    addr: &Addr,
    instance: &str,
    nodes: &'a NodesState,
    sp: Spacing,
) -> Option<Element<'a, Message>> {
    let roles = nodes.redundancy.roles(addr, instance);
    if roles.is_empty() {
        return None;
    }
    let mut r = row![].spacing(sp.xs).align_y(iced::Alignment::Center);
    for (row, i) in roles {
        if let Some(chip) = role_chip(&row.iface, i) {
            r = r.push(chip);
        }
    }
    Some(r.into())
}

/// One role, in the judge's words: the role, then the reason token.
fn role_chip<'a>(iface: &str, i: &RedundancyInstance) -> Option<Element<'a, Message>> {
    let reason = &i.twin.reason;
    Some(match i.role {
        RedundancyRole::Incumbent => {
            kit::status_chip(Tone::Positive, format!("serving {iface} — the incumbent"))
        }
        RedundancyRole::Standby => kit::status_chip(
            Tone::Neutral,
            format!("standby on {iface} — marked, holding no token"),
        ),
        RedundancyRole::Twin => kit::status_chip(
            Tone::Negative,
            format!("TWIN on {iface} ({reason}) — {}", i.twin.says),
        ),
        RedundancyRole::Unknown => kit::status_chip(
            Tone::Neutral,
            format!("role on {iface} unobservable ({reason})"),
        ),
        RedundancyRole::NotACandidate => return None,
    })
}

/// `redundancy.v1` for this address (#841, RC): asked on selection, never
/// polled — §5's questions of each governed interface, in the engine's
/// tokens and words.
fn redundancy_section<'a>(addr: &Addr, nodes: &'a NodesState, sp: Spacing) -> Column<'a, Message> {
    let mut ask = kit::secondary(kit::caption("read again")).padding([0.0, sp.xs]);
    let mine = nodes.redundancy.address() == Some(addr);
    if mine
        && !matches!(
            nodes.redundancy,
            RedundancyState::Asking(_) | RedundancyState::Due(_)
        )
    {
        ask = ask.on_press(msg(NodesMsg::AskRedundancy));
    }
    let mut col = column![
        row![
            kit::eyebrow("REDUNDANCY (REDUNDANCY.V1)"),
            iced::widget::space::horizontal(),
            ask
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(sp.xs);
    let Some(report) = nodes.redundancy.report(addr) else {
        return col.push(kit::muted(match &nodes.redundancy {
            RedundancyState::Due(a) if a == addr => {
                "who serves it is read once the selection settles"
            }
            RedundancyState::Asking(a) if a == addr => {
                "reading redundancy.v1 — two presence reads of the address a grace apart"
            }
            _ => "who serves it not asked — it is read on selection, never polled",
        }));
    };
    if let Some(why) = &report.unobservable {
        return col.push(kit::muted(format!("unobservable — {why}")));
    }
    if report.interfaces.is_empty() {
        return col.push(kit::muted(
            "no interface of it is governed by redundancy.v1: one instance or many, \
             the profile does not say who serves it",
        ));
    }
    for row in &report.interfaces {
        let (tone, word) = match (row.one_incumbent.answer, row.is_finding()) {
            (_, true) => (Tone::Negative, "a finding"),
            (RedundancyAnswerToken::Yes, false) => (Tone::Positive, "one incumbent"),
            _ => (Tone::Neutral, "not established"),
        };
        col = col.push(kit::status_chip(
            tone,
            format!("{} ({}): {word}", row.iface, row.governance),
        ));
        col = col.push(kit::muted(answer_words(
            "served by exactly one incumbent",
            &row.one_incumbent,
        )));
        col = col.push(kit::muted(answer_words("a vacancy", &row.vacancy)));
        col = col.push(kit::muted(format!(
            "stand by: {} ({}, {} {}) — {}",
            token_word(row.standbys.answer),
            row.standbys.reason,
            row.standbys.count,
            if row.standbys.exact {
                "exact"
            } else {
                "at least"
            },
            row.standbys.says
        )));
    }
    col
}

/// One of §5's questions, in the profile's tokens and the judge's words.
fn answer_words(question: &str, a: &RedundancyAnswer) -> String {
    format!(
        "{question}: {} ({}) — {}",
        token_word(a.answer),
        a.reason,
        a.says
    )
}

fn token_word(t: RedundancyAnswerToken) -> &'static str {
    match t {
        RedundancyAnswerToken::Yes => "yes",
        RedundancyAnswerToken::No => "no",
        RedundancyAnswerToken::Unobservable => "unobservable",
        RedundancyAnswerToken::NotAsked => "not asked",
        RedundancyAnswerToken::NotThisProfiles => "not this profile's",
    }
}

/// What one instance's descriptor says (§3.3) — host, process, build and
/// session zid as written, its profiles, what it implements — or why there
/// is nothing to say, in each case's own words.
fn descriptor_lines<'a>(facts: Option<&Described>, sp: Spacing) -> Element<'a, Message> {
    let f = match facts {
        None => return kit::muted("descriptor: no catalog read names this instance"),
        Some(Described::NotAsked) => return kit::muted("descriptor: not asked"),
        Some(Described::Silent) => {
            return kit::muted(
                "descriptor: no reply within the timeout — the instance may be gone, or \
                 the reply did not cross; not a verdict",
            );
        }
        Some(Described::Invalid(why)) => {
            return kit::status_chip(Tone::Caution, format!("descriptor invalid: {why}"));
        }
        Some(Described::Failed(why)) => {
            return kit::muted(format!("descriptor not read: {why}"));
        }
        Some(Described::Served(f)) => f,
    };
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".to_string());
    let mut col = column![kit::muted(format!(
        "host {} · process {} · build {} · zid {}",
        dash(&f.host),
        dash(&f.process),
        dash(&f.build),
        dash(&f.zid),
    ))]
    .spacing(sp.xs);
    let mut chips = row![].spacing(sp.xs).align_y(iced::Alignment::Center);
    // Minted only by the descriptor's own word (hostid.v1 §2.8): a system
    // *named* like `h-…` says nothing (§2.11).
    if f.lists_hostid() {
        chips = chips.push(kit::data_chip("hostid.v1 — listed in its profiles"));
    }
    for (iface, fp) in &f.interfaces {
        chips = chips.push(kit::data_chip(format!("{iface} @ {fp}")));
    }
    // The core's mark (§3.3, 0.39), as the descriptor writes it: whether
    // the instance stands by is the redundancy reading's, below.
    for iface in &f.standby {
        chips = chips.push(kit::data_chip(format!("{iface} marked standby")));
    }
    col = col.push(chips);
    let others: Vec<&str> = f
        .profiles
        .iter()
        .map(String::as_str)
        .filter(|p| *p != crate::nodes::HOSTID)
        .collect();
    if !others.is_empty() {
        col = col.push(kit::muted(format!("profiles: {}", others.join(", "))));
    }
    col.into()
}

/// `health.v1` for this address (#614, FL9; D11): asked on selection,
/// never polled, its clock ground said.
fn health_section<'a>(addr: &Addr, nodes: &'a NodesState, sp: Spacing) -> Column<'a, Message> {
    let clocks = kit::check(nodes.clocks_synced)
        .label("clocks synced — take my word that this host's clock agrees with the owners'")
        .size(font::CAPTION)
        .text_size(font::CAPTION)
        .on_toggle(|b| msg(NodesMsg::ClocksSynced(b)));
    let mut ask = kit::secondary(kit::caption("read again")).padding([0.0, sp.xs]);
    let mine = nodes.health.address() == Some(addr);
    if mine && !matches!(nodes.health, HealthState::Asking(_)) {
        ask = ask.on_press(msg(NodesMsg::AskHealth));
    }
    let mut col = column![
        kit::eyebrow("HEALTH (HEALTH.V1)"),
        row![clocks, iced::widget::space::horizontal(), ask]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
    ]
    .spacing(sp.xs);
    match &nodes.health {
        HealthState::Read(a, report) if a == addr => {
            col = col.push(health_lines(addr, report, sp));
        }
        HealthState::Asking(a) if a == addr => {
            col = col.push(kit::muted(
                "reading health.v1 — two readings of the owner's status, a grace apart",
            ));
        }
        // Read for another address, or never — no session in the namespace
        // to ask through included: this one was not asked.
        _ => {
            col = col.push(kit::muted(
                "health not asked for this address — it is read on selection, never polled",
            ));
        }
    }
    col
}

/// The engine's health report for one address, in its own tokens: the
/// verdict, its level and reason, the clock it was aged against, the
/// status and checks as read, and §5's other questions.
fn health_lines<'a>(addr: &Addr, report: &HealthReport, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![].spacing(sp.xs);
    if let Some(why) = &report.unobservable {
        col = col.push(kit::callout(
            Tone::Neutral,
            kit::caption(format!("unobservable — {why}")),
        ));
    }
    let want = addr.to_string();
    let Some(row) = report.services.iter().find(|r| r.address == want) else {
        return col
            .push(kit::muted("the reading named no row for this address"))
            .into();
    };
    let (tone, said) = verdict_words(row, report.clock);
    col = col.push(kit::status_chip(tone, said));
    // A question the reading did not put to this service has no clock, no
    // agreement and no clock-ahead to report: the verdict said why.
    if row.verdict == HealthVerdict::NotAsked {
        return col.into();
    }
    col = col.push(kit::muted(match report.clock {
        ClockGround::DeploymentWord => "clock: the deployment's word (clocks synced)",
        ClockGround::Measured => "clock: measured on the owners' live puts",
        ClockGround::None => {
            "clock: no word and no window — a reply's age is unobservable (freshness.v1 §2.6)"
        }
    }));
    if let Some(s) = &row.status {
        let mut line = format!("status {}", level(&s.level));
        if !s.reason.is_empty() {
            line.push_str(&format!(" — {}", s.reason));
        }
        line.push_str(match s.via {
            zenkey_fleet::report::StatusVia::Get => " (the owner's reply, S4)",
            zenkey_fleet::report::StatusVia::Subscription => " (heard on the subscription)",
        });
        col = col.push(kit::muted(line));
    }
    for c in &row.checks {
        col = col.push(kit::muted(format!(
            "check {}: {}{}",
            c.check,
            level(&c.level),
            if c.detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", c.detail)
            }
        )));
    }
    col = col.push(kit::muted(answer("agrees with its checks", &row.agrees)));
    col = col.push(kit::muted(answer("clock ahead", &row.clock_ahead)));
    if let Some(lk) = &row.last_known {
        col = col.push(kit::muted(format!(
            "last-known from {}: {} — an archive's, never current (S6)",
            lk.archive,
            level(&lk.level)
        )));
    }
    col.into()
}

/// "Is this service healthy?" in words, with the tone that groups it. An
/// age the reader could not judge says *age unobservable*, and how to give
/// it a ground — never a level, never stale.
pub fn verdict_words(row: &HealthRow, clock: ClockGround) -> (Tone, String) {
    let level = row
        .level
        .map(|l| format!(" ({})", l.as_str()))
        .unwrap_or_default();
    match row.verdict {
        HealthVerdict::Healthy => (Tone::Positive, format!("healthy{level} — {}", row.says)),
        HealthVerdict::Unhealthy => {
            let tone = match row.level {
                Some(zenkey_fleet::report::HealthLevel::Failed) => Tone::Negative,
                _ => Tone::Caution,
            };
            (tone, format!("unhealthy{level} — {}", row.says))
        }
        HealthVerdict::Stale => (Tone::Caution, format!("stale — {}", row.says)),
        HealthVerdict::Unobservable if row.reason == "clock_untrusted" => (
            Tone::Neutral,
            format!(
                "age unobservable — {}{}",
                row.says,
                if clock == ClockGround::None {
                    "; tick \"clocks synced\" if they agree"
                } else {
                    ""
                }
            ),
        ),
        HealthVerdict::Unobservable => (Tone::Neutral, format!("unobservable — {}", row.says)),
        HealthVerdict::NotAsked => (Tone::Neutral, format!("not asked — {}", row.says)),
    }
}

/// One of §5's yes-or-no questions, in the three states.
fn answer(question: &str, a: &HealthAnswer) -> String {
    let word = match a.answer {
        HealthAnswerToken::Yes => "yes",
        HealthAnswerToken::No => "no",
        HealthAnswerToken::Unobservable => "unobservable",
        HealthAnswerToken::NotAsked => "not asked",
    };
    // The engine's words point a CLI at `--for`; a reading on selection
    // opens no window, and says so in this window's terms.
    let says = match a.reason.as_str() {
        "no_window" => {
            "a reading on selection opens no subscription, and the question rests \
                        on what a subscriber hears"
        }
        _ => a.says.as_str(),
    };
    format!("{question}: {word} — {says}")
}

/// A level as the payload carried it (§2.6).
fn level(l: &LevelRead) -> String {
    match l {
        LevelRead::Token(t) => (*t).to_string(),
        LevelRead::Unlisted(n) => format!("unlisted level {n} — unknown, never ok"),
    }
}

/// How long ago `since` was, in the age words the tree uses: "just now",
/// "12s ago", "4m ago". A presence `since` is the fold that learned it — a
/// seed's is a lower bound on how long the token was held (FL4).
fn ago(since: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(since)
        .unwrap_or_default()
        .as_secs_f32();
    match kit::age_word(secs).as_str() {
        "now" => "just now".to_string(),
        w => format!("{w} ago"),
    }
}
