//! Presence-backed rules (#614, FL2): `instance-gone <SYSTEM/SERVICE>`,
//! and `liveliness-gone` over the deployment's own zk2 presence tokens,
//! read from the fleet's live projection ([`LivePresence`], fed by a
//! [`zenkey_fleet::PresenceFeed`]).
//!
//! **One state per thing that persists.** An instance token names one run
//! of a service, and a restart mints another: a rule keyed on instance
//! tokens would fire forever on every restart. So a rule's identities are
//! what a token form stands for ([`Form`]): an **address** for instance
//! tokens (`instance-gone`, and `liveliness-gone …/@zk/instance/*`) — up
//! while any instance token is held; an **address and interface** for
//! interface tokens (`…/@zk/alive/…`) — held while any instance holds one;
//! an **address, interface and member** for member tokens
//! (`…/@zk/member/…`) — a make-before-break handover is no gap (§8.1).
//!
//! **Three states.** An identity is `firing` (gone) or `ok` (held); a gone
//! identity needs a complete read behind it, as the projection's `down`
//! does.
//!
//! **A withdrawal heard live is not gone** (core §8.1, 0.37). A reader whose
//! router restarts hears every token withdrawn at once, and their owners
//! never left. An address heard withdrawn is unobservable in the projection
//! until the read the withdrawal owes says down, and only then gone here;
//! an interface or a member heard going with nothing left held is pending
//! until the next complete read ([`LivePresence::complete_reads`]): gone if
//! that read shows it still unheld, nothing at all if it came back first,
//! since it was never said gone. A crash still pages, a settle and one
//! read's round trip after its lease runs out. Whatever the projection cannot establish — no seed yet, a seed
//! that ended at its timeout, events lost to a `Dropped(n)` — is the
//! rule's **observer** going `unobservable` (identity `<rule_id>:observer`),
//! never an `ok`: one notification for the rule, not one per address, and
//! `observable_again` when a complete read restores the basis. An address
//! that was firing stays firing across the loss until a re-read says
//! otherwise; one whose stop was hidden in the loss is found by that
//! re-read and fires then.
//!
//! **The baseline** is the first seed, as the scheduled doctor's first run
//! is: an address already down is entered into the ledger *assumed*
//! ([`crate::discipline::NoticeMeta::assume`]) — remembered, never paged
//! one by one — and said once, in one `baseline` notification per rule
//! that has something to say (an address down, an observer that cannot
//! see, a pattern that matches nothing yet). After a restart the state
//! file's memory is reconciled against it: still down is a duplicate, back
//! up is a `resolved`.
//!
//! **Patterns.** `*/tc` and `host-a/*` are tracked patterns
//! ([`LivePresence::track_pattern`]): every address they match is a record
//! of its own, and a second address appearing under one is one more
//! identity. A wildcard names no address until a token does: nothing can
//! be gone that was never seen. An exact address is tracked from the start,
//! so a complete seed that shows none of it says it is down.
//!
//! **One reading, two tools** (#747). The address form — `instance-gone`,
//! and `liveliness-gone …/@zk/instance/*` — is the fleet's
//! [`AddressWatch`]: which addresses are held and gone, the first look, and
//! what a pattern can call gone are decided there, and `zenctl watchdog`'s
//! `instance-gone` folds the same reading into one state per rule. What is
//! this daemon's is what it does with a move: the notification, its ledger
//! and its baseline. The observer's sight is the fleet's too
//! ([`presence_observer`]).

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use zenkey_fleet::{
    AddressWatch, CondState, LivePresence, PresenceBasis, PresenceChange, PresenceSource,
    PresenceTransition, ResolvedTarget as Target, presence_observer,
};
use zenkey_model::grammar::{Addr, InstanceId, alive_key, member_key};
use zenoh::key_expr::KeyExpr;

use crate::engine::RuleNotice;
use crate::rules::{Rule, RuleKind};
use crate::sinks::NoticeKind;

/// What a presence rule's identities are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// One per address: held while an instance token is (`instance-gone`;
    /// `liveliness-gone …/@zk/instance/*` and `…/@zk/**`).
    Address,
    /// One per address and interface: held while any instance holds an
    /// interface token for it (`liveliness-gone …/@zk/alive/…`).
    Interface,
    /// One per address, interface and member: held while any epoch of the
    /// member holds a token (`liveliness-gone …/@zk/member/…`).
    Member,
}

/// The identity of the rule's own sight.
pub const OBSERVER: &str = "observer";
/// The identity of a presence rule's baseline notification (passthrough).
pub const BASELINE: &str = "baseline";

/// The presence form of a `liveliness-gone` selector, when it names the
/// deployment's own zk2 presence tokens: under `namespace`, then
/// `zk2/<system>/<service>/@zk/<form>…`, each of system and service a name
/// or `*`. `None` for anything else — another namespace's tokens, a
/// foreign key, a shape this reading does not cover — which the daemon's
/// own token ledger keeps ([`crate::liveliness`]).
pub fn presence_form(selector: &str, namespace: &str) -> Option<(Target, Form)> {
    let relative = zenkey_fleet::strip_namespace(namespace, selector)?;
    let c: Vec<&str> = relative.split('/').collect();
    if c.len() < 5 || c[0] != zenkey_model::grammar::GRAMMAR || c[3] != "@zk" {
        return None;
    }
    let position = |p: &str| p == "*" || zenkey_model::chunk::is_plain_chunk(p);
    if !position(c[1]) || !position(c[2]) {
        return None;
    }
    let target = Target::parse(&format!("{}/{}", c[1], c[2])).ok()?;
    let form = match &c[4..] {
        ["**"] | ["instance", "*"] | ["instance", "**"] => Form::Address,
        ["alive", ..] => Form::Interface,
        ["member", ..] => Form::Member,
        _ => return None,
    };
    Some((target, form))
}

/// One presence-backed rule and what it last said.
#[derive(Debug)]
struct PresenceRule {
    name: String,
    target: Target,
    form: Form,
    /// The rule's wire selector (`liveliness-gone`), matched against each
    /// token's wire key; `None` for `instance-gone`, which reads the
    /// address alone.
    keyexpr: Option<KeyExpr<'static>>,
    /// The address form's reading: the fleet's, which `zenctl watchdog`'s
    /// `instance-gone` folds too (#747).
    addresses: AddressWatch,
    /// What the rule last said per identity of the interface and member
    /// forms: `true` held, `false` gone.
    known: BTreeMap<String, bool>,
    /// The instances (interface form) or epochs (member form) holding each
    /// identity's tokens.
    holders: BTreeMap<String, BTreeSet<InstanceId>>,
    /// The interface and member identities heard going with nothing left
    /// held, each with the complete reads folded when it was heard and the
    /// withdrawal itself: said gone only once a later complete read shows
    /// it still unheld (core §8.1, 0.37).
    pending: BTreeMap<String, (u64, PresenceTransition)>,
    /// What the rule last said of its own sight.
    observer: Option<CondState>,
}

/// Every presence-backed rule of a config, folded together over one
/// projection.
#[derive(Debug)]
pub struct PresenceRules {
    namespace: String,
    rules: Vec<PresenceRule>,
}

impl PresenceRules {
    /// The presence-backed rules among `rules`: every `instance-gone`, and
    /// every `liveliness-gone` whose selector names the deployment's own zk2
    /// presence tokens in `namespace`.
    pub fn new(rules: &[Rule], namespace: &str) -> PresenceRules {
        let rules = rules
            .iter()
            .filter_map(|r| {
                let (target, form, keyexpr) = match &r.kind {
                    RuleKind::InstanceGone { target } => (target.clone(), Form::Address, None),
                    RuleKind::LivelinessGone { selector } => {
                        let (target, form) = presence_form(selector, namespace)?;
                        (target, form, r.keyexpr.clone())
                    }
                    _ => return None,
                };
                Some(PresenceRule {
                    name: r.name.clone(),
                    addresses: AddressWatch::new(target.clone()),
                    target,
                    form,
                    keyexpr,
                    known: BTreeMap::new(),
                    holders: BTreeMap::new(),
                    pending: BTreeMap::new(),
                    observer: None,
                })
            })
            .collect();
        PresenceRules {
            namespace: namespace.to_string(),
            rules,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules' names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| r.name.as_str())
    }

    /// Track every rule's pattern on the projection, before it is seeded.
    pub fn track(&self, presence: &mut LivePresence, at: SystemTime) {
        for r in &self.rules {
            r.addresses.track(presence, at);
        }
    }

    /// The first look (see the module doc): per rule, what the seeded
    /// projection holds, assumed or observed, reconciled against what the
    /// ledger remembers announced (`announced`, by rule name: identities),
    /// and a baseline notification when there is something to say.
    pub fn baseline(
        &mut self,
        presence: &LivePresence,
        announced: &BTreeMap<String, Vec<String>>,
        rule_ids: &BTreeMap<String, String>,
        at: &str,
    ) -> Vec<RuleNotice> {
        let namespace = self.namespace.clone();
        let mut out = Vec::new();
        for r in &mut self.rules {
            let observer = presence_observer(presence.basis(), &r.target.address).state;
            let held = r.holdings(presence, &namespace);
            let id = rule_ids.get(&r.name).cloned().unwrap_or_default();
            // What the ledger remembers announced under the rule, by
            // identity (the observer apart).
            let remembered: BTreeSet<&str> = announced
                .get(&r.name)
                .into_iter()
                .flatten()
                .filter_map(|m| m.strip_prefix(&id).and_then(|s| s.strip_prefix(':')))
                .filter(|identity| *identity != OBSERVER)
                .collect();
            let mut up = Vec::new();
            let mut down = Vec::new();
            for (identity, holds) in &held {
                let (address, _) = split_identity(identity);
                if *holds {
                    up.push(identity.clone());
                    // Held at the first look is not news — unless the ledger
                    // remembers it gone, when it is the resolve. Nothing else
                    // enters the ledger: an `ok` there would only crowd out
                    // what it must remember.
                    if remembered.contains(identity.as_str()) {
                        out.push(r.notice(identity, CondState::Ok, None, false, at, None, address));
                    }
                } else {
                    down.push(identity.clone());
                    out.push(r.notice(identity, CondState::Firing, None, true, at, None, address));
                }
            }
            // The rest of the ledger's memory, against what the projection
            // shows now: nothing of it held and no record — gone if a
            // complete read stands behind it, and remembered gone either
            // way, so that its return is the resolve it owes.
            for identity in remembered {
                if held.contains_key(identity) {
                    continue;
                }
                r.remember_gone(identity);
                if presence.basis() == PresenceBasis::Complete {
                    down.push(identity.to_string());
                    let (address, _) = split_identity(identity);
                    out.push(r.notice(identity, CondState::Firing, None, true, at, None, address));
                }
            }
            r.observer = Some(observer);
            if observer == CondState::Unobservable {
                out.push(r.observer_notice(observer, None, presence.basis(), true, at));
            }
            let empty_pattern = r.target.concrete.is_none() && held.is_empty();
            if !down.is_empty() || observer == CondState::Unobservable || empty_pattern {
                out.push(r.baseline_notice(&up, &down, presence.basis(), at));
            }
        }
        out
    }

    /// Fold one batch of the projection's moves into notices, the rules'
    /// sight read off `presence` after it.
    pub fn fold(
        &mut self,
        moved: &[PresenceTransition],
        presence: &LivePresence,
        at: &str,
    ) -> Vec<RuleNotice> {
        let namespace = self.namespace.clone();
        let mut out = Vec::new();
        let state_of = |held: bool| {
            if held {
                CondState::Ok
            } else {
                CondState::Firing
            }
        };
        for r in &mut self.rules {
            if r.form == Form::Address {
                // The fleet's reading decides each address (#747).
                for m in r.addresses.fold(moved) {
                    let identity = m.address.to_string();
                    let address = Some(identity.clone());
                    let prior = m.prior.map(state_of);
                    let state = state_of(m.held);
                    out.push(r.notice(&identity, state, prior, false, at, Some(&m.by), address));
                }
            }
            for t in moved {
                if r.form == Form::Address || !r.target.matches(&t.address) {
                    continue;
                }
                if let Some((identity, holds)) = r.apply(t, &namespace) {
                    // Heard again before a read said it went: it never did.
                    if holds {
                        r.pending.remove(&identity);
                    }
                    let prior = r.known.get(&identity).copied();
                    if prior == Some(holds) {
                        continue;
                    }
                    // Heard going, live: a reason to read again, never gone
                    // by itself (core §8.1, 0.37). The next complete read
                    // decides it, below.
                    if !holds && prior == Some(true) && t.source == PresenceSource::Event {
                        r.pending
                            .entry(identity)
                            .or_insert_with(|| (presence.complete_reads(), t.clone()));
                        continue;
                    }
                    // First sight: held is not news, only remembered, so its
                    // going can say it went. Nothing can be gone that was
                    // never seen held.
                    if prior.is_none() {
                        if holds {
                            r.known.insert(identity, true);
                        }
                        continue;
                    }
                    r.known.insert(identity.clone(), holds);
                    let address = Some(t.address.to_string());
                    let (state, prior) = (state_of(holds), prior.map(state_of));
                    out.push(r.notice(&identity, state, prior, false, at, Some(t), address));
                }
            }
            // The identities heard going that a complete read since shows
            // still unheld: gone now.
            let reads = presence.complete_reads();
            let decided: Vec<String> = r
                .pending
                .iter()
                .filter(|(_, (heard, _))| reads > *heard)
                .map(|(identity, _)| identity.clone())
                .collect();
            for identity in decided {
                let (_, t) = r.pending.remove(&identity).expect("listed");
                if r.holders.get(&identity).is_some_and(|h| !h.is_empty()) {
                    continue;
                }
                let prior = r.known.insert(identity.clone(), false).map(state_of);
                let address = Some(t.address.to_string());
                let mut n = r.notice(
                    &identity,
                    CondState::Firing,
                    prior,
                    false,
                    at,
                    Some(&t),
                    address,
                );
                n.evidence.push_str(
                    "; heard withdrawn, and a complete read since shows it unheld (core §8.1)",
                );
                out.push(n);
            }
            let observer = presence_observer(presence.basis(), &r.target.address).state;
            if r.observer != Some(observer) {
                let prior = r.observer;
                r.observer = Some(observer);
                out.push(r.observer_notice(observer, prior, presence.basis(), false, at));
            }
        }
        out
    }
}

/// An identity's address, and what follows it (an interface, a member).
fn split_identity(identity: &str) -> (Option<String>, &str) {
    let mut parts = identity.splitn(3, '/');
    match (parts.next(), parts.next()) {
        (Some(system), Some(service)) => (
            Some(format!("{system}/{service}")),
            identity
                .get(system.len() + service.len() + 1..)
                .unwrap_or("")
                .trim_start_matches('/'),
        ),
        _ => (None, identity),
    }
}

impl PresenceRule {
    fn head(&self) -> &'static str {
        if self.keyexpr.is_some() {
            "liveliness-gone"
        } else {
            "instance-gone"
        }
    }

    /// Whether a token's base-relative key is one this rule's selector
    /// covers (always, for `instance-gone`).
    fn covers(&self, relative: &str, namespace: &str) -> bool {
        let Some(k) = &self.keyexpr else {
            return true;
        };
        let wire = zenkey_fleet::with_namespace(namespace, relative);
        KeyExpr::try_from(wire.as_str()).is_ok_and(|w| k.intersects(&w))
    }

    /// The first look, per identity, for the addresses this rule's pattern
    /// matches: `true` held, `false` gone, each said from here. The address
    /// form is the fleet's ([`AddressWatch::first_look`]: an address down
    /// is gone only when the operator named it exactly); the interface and
    /// member forms are held by any holder. Unobservable addresses are left
    /// out.
    fn holdings(&mut self, presence: &LivePresence, namespace: &str) -> BTreeMap<String, bool> {
        if self.form == Form::Address {
            return self
                .addresses
                .first_look(presence)
                .into_iter()
                .map(|(addr, held)| (addr.to_string(), held))
                .collect();
        }
        let mut out = BTreeMap::new();
        let target = self.target.clone();
        for (addr, rec) in presence.matching(&target) {
            match self.form {
                Form::Address => {}
                Form::Interface => {
                    for (instance, i) in &rec.instances {
                        for (iface, fps) in &i.interfaces {
                            for fp in fps {
                                let Ok(key) = alive_key(addr, iface, instance, fp) else {
                                    continue;
                                };
                                if !self.covers(key.as_str(), namespace) {
                                    continue;
                                }
                                let identity = format!("{addr}/{iface}");
                                self.holders
                                    .entry(identity.clone())
                                    .or_default()
                                    .insert(instance.clone());
                                out.insert(identity, true);
                            }
                        }
                    }
                }
                Form::Member => {
                    for (member, epochs) in &rec.members {
                        for epoch in epochs {
                            let Ok(key) = member_key(addr, &member.iface, &member.member, epoch)
                            else {
                                continue;
                            };
                            if !self.covers(key.as_str(), namespace) {
                                continue;
                            }
                            let identity = format!("{addr}/{}/{}", member.iface, member.member);
                            self.holders
                                .entry(identity.clone())
                                .or_default()
                                .insert(epoch.clone());
                            out.insert(identity, true);
                        }
                    }
                }
            }
        }
        self.known.extend(out.iter().map(|(i, h)| (i.clone(), *h)));
        out
    }

    /// Remember `identity` gone with no move to show for it: the ledger's
    /// memory after a restart, so that its return is the resolve it owes.
    fn remember_gone(&mut self, identity: &str) {
        match (self.form, identity.parse::<Addr>()) {
            (Form::Address, Ok(addr)) => self.addresses.remember_gone(addr),
            (Form::Address, Err(_)) => {}
            _ => {
                self.known.insert(identity.to_string(), false);
            }
        }
    }

    /// One move of the projection, as an interface or member rule reads
    /// it: the identity it moved and whether that identity is held now — or
    /// nothing, when the move is not this rule's (another form, a key the
    /// selector does not cover). The address form is the fleet's
    /// ([`AddressWatch::fold`]).
    fn apply(&mut self, t: &PresenceTransition, namespace: &str) -> Option<(String, bool)> {
        let addr: &Addr = &t.address;
        match (&self.form, &t.change) {
            (
                Form::Interface,
                PresenceChange::InterfaceAppeared {
                    instance,
                    iface,
                    fp,
                },
            ) => {
                let key = alive_key(addr, iface, instance, fp).ok()?;
                if !self.covers(key.as_str(), namespace) {
                    return None;
                }
                let identity = format!("{addr}/{iface}");
                let holders = self.holders.entry(identity.clone()).or_default();
                holders.insert(instance.clone());
                Some((identity, true))
            }
            (
                Form::Interface,
                PresenceChange::InterfaceGone {
                    instance,
                    iface,
                    fp,
                },
            ) => {
                let key = alive_key(addr, iface, instance, fp).ok()?;
                if !self.covers(key.as_str(), namespace) {
                    return None;
                }
                let identity = format!("{addr}/{iface}");
                let holders = self.holders.entry(identity.clone()).or_default();
                holders.remove(instance);
                Some((identity, !holders.is_empty()))
            }
            (Form::Member, PresenceChange::MemberAppeared { member, epoch }) => {
                let key = member_key(addr, &member.iface, &member.member, epoch).ok()?;
                if !self.covers(key.as_str(), namespace) {
                    return None;
                }
                let identity = format!("{addr}/{}/{}", member.iface, member.member);
                self.holders
                    .entry(identity.clone())
                    .or_default()
                    .insert(epoch.clone());
                Some((identity, true))
            }
            (Form::Member, PresenceChange::MemberGone { member, epoch }) => {
                let key = member_key(addr, &member.iface, &member.member, epoch).ok()?;
                if !self.covers(key.as_str(), namespace) {
                    return None;
                }
                let identity = format!("{addr}/{}/{}", member.iface, member.member);
                self.holders.remove(&identity);
                Some((identity, false))
            }
            _ => None,
        }
    }

    /// What an identity's state is, in a sentence.
    fn evidence(&self, identity: &str, holds: bool, t: Option<&PresenceTransition>) -> String {
        let (address, rest) = split_identity(identity);
        let address = address.unwrap_or_else(|| identity.to_string());
        let how = match t.map(|t| t.source) {
            Some(PresenceSource::Event) => " — heard as it happened",
            Some(PresenceSource::Seed) => " — found by a re-read: it moved at or before then",
            Some(PresenceSource::Dropped)
            | Some(PresenceSource::Withdrawn)
            | Some(PresenceSource::CutOff)
            | Some(PresenceSource::Moved)
            | None => "",
        };
        match (self.form, holds) {
            (Form::Address, false) => format!(
                "no instance token of {address} visible to this reader, a complete read \
                 behind it (spec §8.1; a refused read is empty too){how}"
            ),
            (Form::Address, true) => {
                format!("{address} holds an instance token (spec §8.1){how}")
            }
            (Form::Interface, false) => format!(
                "no instance of {address} holds an interface token for {rest} (spec §8.1){how}"
            ),
            (Form::Interface, true) => {
                format!("{address} holds an interface token for {rest} (spec §8.1){how}")
            }
            (Form::Member, false) => {
                format!("member {rest} of {address} holds no token (spec §8.1){how}")
            }
            (Form::Member, true) => format!(
                "member {rest} of {address} holds a token (spec §8.1; a handover is no gap){how}"
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn notice(
        &self,
        identity: &str,
        state: CondState,
        prior: Option<CondState>,
        assume: bool,
        at: &str,
        t: Option<&PresenceTransition>,
        address: Option<String>,
    ) -> RuleNotice {
        RuleNotice {
            rule: self.name.clone(),
            what: identity.to_string(),
            kind: match self.form {
                Form::Address if self.keyexpr.is_none() => NoticeKind::Presence,
                _ => NoticeKind::Liveliness,
            },
            state,
            prior,
            severity: None,
            evidence: self.evidence(identity, state == CondState::Ok, t),
            lines: Vec::new(),
            key: None,
            address,
            at: at.to_string(),
            presence: t.cloned(),
            assume,
            passthrough: false,
            reason: "a presence token carries no payload",
            alert: None,
        }
    }

    fn observer_notice(
        &self,
        state: CondState,
        prior: Option<CondState>,
        basis: PresenceBasis,
        assume: bool,
        at: &str,
    ) -> RuleNotice {
        // The observer's words are the fleet's (#747), the watchdog's
        // `instance-gone` fold says the same.
        let evidence = presence_observer(basis, &self.target.address).evidence;
        RuleNotice {
            rule: self.name.clone(),
            what: OBSERVER.to_string(),
            kind: NoticeKind::Unobservable,
            state,
            prior,
            severity: None,
            evidence,
            lines: Vec::new(),
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume,
            passthrough: false,
            reason: "nothing was observed",
            alert: None,
        }
    }

    fn baseline_notice(
        &self,
        up: &[String],
        down: &[String],
        basis: PresenceBasis,
        at: &str,
    ) -> RuleNotice {
        let what = match self.form {
            Form::Address => "address(es)",
            Form::Interface => "interface(s)",
            Form::Member => "member(s)",
        };
        let sight = match basis {
            PresenceBasis::Complete => "a complete read".to_string(),
            PresenceBasis::Incomplete { cut_off_during } => format!(
                "a read that {}: what it did not show is unobservable",
                zenkey_fleet::model::catalog::incomplete_reason(
                    false,
                    cut_off_during,
                    "ended at its timeout"
                )
                .unwrap_or_default()
            ),
            PresenceBasis::Lost { dropped } => {
                format!("{dropped} event(s) lost: unobservable until a complete re-read")
            }
            PresenceBasis::Unseeded => "no read yet".into(),
            PresenceBasis::CutOff => {
                "this daemon's session reaches no router and no peer: no read is made, and \
                 everything is unobservable until a complete read"
                    .into()
            }
            PresenceBasis::Moved => {
                "the watch session's transport moved: what it held is unconfirmed until a \
                 complete re-read"
                    .into()
            }
        };
        let mut evidence = format!(
            "{} {}: {} {what} held, {} gone — {sight}",
            self.head(),
            self.keyexpr
                .as_ref()
                .map(|k| k.as_str().to_string())
                .unwrap_or_else(|| self.target.address.clone()),
            up.len(),
            down.len()
        );
        if up.is_empty() && down.is_empty() && self.target.concrete.is_none() {
            evidence.push_str(&format!(
                "; no address matches {} yet — nothing can be gone that was never seen, so \
                 the first one to appear is the first one watched",
                self.target.address
            ));
        }
        let mut lines = vec![
            "the first look: what is down now is remembered, not paged one by one; only what \
             changes from here is news"
                .to_string(),
        ];
        lines.extend(down.iter().map(|d| format!("- gone: {d}")));
        lines.extend(up.iter().map(|u| format!("- held: {u}")));
        let state = if !down.is_empty() {
            CondState::Firing
        } else if basis != PresenceBasis::Complete {
            CondState::Unobservable
        } else {
            CondState::Ok
        };
        RuleNotice {
            rule: self.name.clone(),
            what: BASELINE.to_string(),
            kind: NoticeKind::Baseline,
            state,
            prior: None,
            severity: down.is_empty().then(|| "info".to_string()),
            evidence,
            lines,
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume: false,
            passthrough: true,
            reason: "a baseline is a count, not a payload",
            alert: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use zenkey_fleet::{AddressStatus, PresenceScope};

    fn rule(name: &str, text: &str) -> Rule {
        Rule::from_config(&crate::config::RuleConfig {
            name: name.into(),
            rule: text.into(),
            severity: Some("error".into()),
            labels: Default::default(),
            sinks: vec!["ops".into()],
            for_s: None,
        })
        .unwrap()
    }

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    }

    fn addr(s: &str) -> Addr {
        s.parse().unwrap()
    }

    fn instance(a: &str, i: u64) -> String {
        zenkey_model::grammar::instance_key(&addr(a), &InstanceId::from_u64(i))
            .unwrap()
            .as_str()
            .to_string()
    }

    fn seed(keys: &[String], complete: bool) -> zenkey_fleet::ObservedPresence {
        zenkey_fleet::ObservedPresence::from_keys(PresenceScope::all().selector(), keys, complete)
    }

    fn ids(rules: &[Rule]) -> BTreeMap<String, String> {
        rules
            .iter()
            .map(|r| (r.name.clone(), r.id.clone()))
            .collect()
    }

    fn summary(out: &[RuleNotice]) -> Vec<(String, String, CondState, bool)> {
        out.iter()
            .map(|n| (n.rule.clone(), n.what.clone(), n.state, n.assume))
            .collect()
    }

    /// Which `liveliness-gone` selectors ride the projection: the
    /// deployment's own zk2 presence tokens, by form; another namespace's,
    /// a foreign key and a shape outside the reading do not.
    #[test]
    fn a_liveliness_selector_rides_the_projection_by_its_form() {
        let form = |sel: &str, ns: &str| presence_form(sel, ns).map(|(t, f)| (t.address, f));
        assert_eq!(
            form("prod/zk2/*/tc/@zk/instance/*", "prod"),
            Some(("*/tc".into(), Form::Address))
        );
        assert_eq!(
            form("zk2/host-a/*/@zk/**", ""),
            Some(("host-a/*".into(), Form::Address))
        );
        assert_eq!(
            form("prod/zk2/*/*/@zk/alive/tc.netif.v1/**", "prod"),
            Some(("*/*".into(), Form::Interface))
        );
        assert_eq!(
            form("zk2/h1/nav/@zk/member/nav.v2/*/*", ""),
            Some(("h1/nav".into(), Form::Member))
        );
        for (sel, ns) in [
            ("staging/zk2/*/tc/@zk/instance/*", "prod"),
            ("demo/held/*/alive", ""),
            ("zk2/**", ""),
            ("zk2/h1/tc/@zk/instance/0123456789abcdef", ""),
            ("zk2/h*/tc/@zk/instance/*", ""),
        ] {
            assert_eq!(form(sel, ns), None, "{sel} in {ns:?}");
        }
    }

    /// The baseline (#614, FL2): an exact address absent from a complete
    /// seed is down — assumed, not paged — and said in one baseline
    /// notification at the rule's severity; an address up is not news, and
    /// no notice at all (nothing to crowd the ledger with); a wildcard does
    /// not claim an address another rule tracks; a pattern that matches
    /// nothing yet says so in an `info` baseline.
    #[test]
    fn the_first_look_assumes_what_is_down_and_says_it_once() {
        let rules = vec![
            rule("tc-gone", "instance-gone */tc"),
            rule("h9-gone", "instance-gone h9/tc"),
            rule("gui-gone", "instance-gone */gui"),
        ];
        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        let out = watch.baseline(&p, &BTreeMap::new(), &ids(&rules), "t");
        assert_eq!(
            summary(&out),
            [
                ("h9-gone".into(), "h9/tc".into(), CondState::Firing, true),
                (
                    "h9-gone".into(),
                    "baseline".into(),
                    CondState::Firing,
                    false
                ),
                ("gui-gone".into(), "baseline".into(), CondState::Ok, false),
            ]
        );
        // h1/tc is remembered held: its going is news, once the read its
        // withdrawal owes says so (core §8.1, 0.37).
        let t = p.token(&instance("h1/tc", 1), false, at(2));
        assert!(watch.fold(&t, &p, "t").is_empty(), "heard withdrawn");
        let t = p.seed(&seed(&[], true), at(2));
        let gone = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&gone),
            [("tc-gone".into(), "h1/tc".into(), CondState::Firing, false)]
        );
        let h9 = &out[1];
        assert!(
            h9.passthrough && h9.severity.is_none(),
            "the rule's own severity"
        );
        assert!(
            h9.lines.iter().any(|l| l == "- gone: h9/tc"),
            "{:?}",
            h9.lines
        );
        let gui = &out[2];
        assert_eq!(gui.severity.as_deref(), Some("info"));
        assert!(
            gui.evidence.contains("no address matches */gui yet"),
            "{}",
            gui.evidence
        );
    }

    /// The live poles: a second address appearing under a pattern is an
    /// identity of its own — not news, remembered — so its going is firing,
    /// with its prior and the move that decided it; coming back is ok; a
    /// `Dropped(n)` is the observer going unobservable — never an `ok` —
    /// with no per-address notice, and the complete re-read restores it and
    /// finds what the loss hid.
    #[test]
    fn instance_gone_fires_resolves_and_goes_unobservable_on_a_loss() {
        let rules = vec![rule("tc-gone", "instance-gone */tc")];
        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        assert!(
            watch
                .baseline(&p, &BTreeMap::new(), &ids(&rules), "t")
                .is_empty(),
            "all up: nothing to say"
        );

        // A second address appears under the pattern: first sight.
        let t = p.token(&instance("h2/tc", 2), true, at(2));
        assert!(watch.fold(&t, &p, "t").is_empty(), "held is not news");

        // It goes: heard withdrawn, nothing yet (core §8.1, 0.37); the read
        // that shows none is firing, from ok, with its status move attached.
        let t = p.token(&instance("h2/tc", 2), false, at(3));
        assert!(watch.fold(&t, &p, "t").is_empty(), "heard withdrawn");
        assert!(p.awaits_read());
        let t = p.seed(&seed(&[instance("h1/tc", 1)], true), at(3));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [("tc-gone".into(), "h2/tc".into(), CondState::Firing, false)]
        );
        assert_eq!(out[0].prior, Some(CondState::Ok));
        assert_eq!(out[0].kind, NoticeKind::Presence);
        assert_eq!(out[0].address.as_deref(), Some("h2/tc"));
        let moved = out[0].presence.as_ref().expect("the deciding move");
        assert_eq!(
            moved.change,
            PresenceChange::Status {
                from: AddressStatus::Unobservable,
                to: AddressStatus::Down
            }
        );
        assert!(
            out[0].evidence.contains("found by a re-read"),
            "{}",
            out[0].evidence
        );

        // Back: ok, from firing.
        let t = p.token(&instance("h2/tc", 3), true, at(4));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [("tc-gone".into(), "h2/tc".into(), CondState::Ok, false)]
        );
        assert_eq!(out[0].prior, Some(CondState::Firing));

        // A loss: the observer goes unobservable, no address says ok.
        let t = p.dropped(5, at(5));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [(
                "tc-gone".into(),
                "observer".into(),
                CondState::Unobservable,
                false
            )]
        );
        assert!(out[0].evidence.contains("lost 5"), "{}", out[0].evidence);
        // h1/tc stopped inside the loss: the complete re-read finds it, and
        // the observer is observable again.
        let t = p.seed(&seed(&[instance("h2/tc", 3)], true), at(6));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [
                ("tc-gone".into(), "h1/tc".into(), CondState::Firing, false),
                ("tc-gone".into(), "observer".into(), CondState::Ok, false),
            ]
        );
        assert_eq!(out[0].prior, Some(CondState::Ok));
        assert!(
            out[0].evidence.contains("found by a re-read"),
            "{}",
            out[0].evidence
        );
        assert_eq!(out[1].prior, Some(CondState::Unobservable));
    }

    /// The baseline against the ledger after a restart: an address the
    /// state file remembers firing that is up now is an observed `ok` (a
    /// resolve); one still down is assumed (a duplicate); and with no
    /// complete read, nothing remembered is decided.
    #[test]
    fn the_first_look_reconciles_the_ledger() {
        let rules = vec![rule("tc-gone", "instance-gone */tc")];
        let announced = BTreeMap::from([(
            "tc-gone".to_string(),
            vec!["tc-gone:h1/tc".to_string(), "tc-gone:h5/tc".to_string()],
        )]);
        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        let out = watch.baseline(&p, &announced, &ids(&rules), "t");
        assert_eq!(
            summary(&out),
            [
                ("tc-gone".into(), "h1/tc".into(), CondState::Ok, false),
                ("tc-gone".into(), "h5/tc".into(), CondState::Firing, true),
                (
                    "tc-gone".into(),
                    "baseline".into(),
                    CondState::Firing,
                    false
                ),
            ]
        );

        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(&seed(&[], false), at(1));
        let out = watch.baseline(&p, &announced, &ids(&rules), "t");
        assert_eq!(
            summary(&out),
            [
                (
                    "tc-gone".into(),
                    "observer".into(),
                    CondState::Unobservable,
                    true
                ),
                (
                    "tc-gone".into(),
                    "baseline".into(),
                    CondState::Unobservable,
                    false
                ),
            ],
            "possibly incomplete: nothing remembered is decided"
        );
        // …and remembered gone all the same: the complete read that shows
        // h1/tc up is its resolve, and the observer's.
        let t = p.seed(&seed(&[instance("h1/tc", 1)], true), at(2));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [
                ("tc-gone".into(), "h1/tc".into(), CondState::Ok, false),
                ("tc-gone".into(), "observer".into(), CondState::Ok, false),
            ]
        );
        assert_eq!(out[0].prior, Some(CondState::Firing));
    }

    /// `liveliness-gone` on interface and member tokens: an interface is
    /// gone when its last holder goes, not before; a member's handover is
    /// no gap; a token the selector does not cover moves nothing.
    #[test]
    fn interface_and_member_tokens_are_held_by_any_holder() {
        let rules = vec![
            rule("netif", "liveliness-gone zk2/*/tc/@zk/alive/tc.netif.v1/**"),
            rule(
                "members",
                "liveliness-gone zk2/h1/nav/@zk/member/nav.v2/a/*",
            ),
        ];
        let iface: zenkey_model::grammar::IfaceId = "tc.netif.v1".parse().unwrap();
        let other: zenkey_model::grammar::IfaceId = "tc.netem.v1".parse().unwrap();
        let nav: zenkey_model::grammar::IfaceId = "nav.v2".parse().unwrap();
        let fp = zenkey_model::grammar::Fp16::new("abababababababab").unwrap();
        let alive = |a: &str, f: &zenkey_model::grammar::IfaceId, i: u64| {
            alive_key(&addr(a), f, &InstanceId::from_u64(i), &fp)
                .unwrap()
                .as_str()
                .to_string()
        };
        let member = |m: &str, e: u64| {
            member_key(&addr("h1/nav"), &nav, m, &InstanceId::from_u64(e))
                .unwrap()
                .as_str()
                .to_string()
        };
        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(
            &seed(
                &[
                    instance("h1/tc", 1),
                    alive("h1/tc", &iface, 1),
                    alive("h1/tc", &other, 1),
                    instance("h1/tc", 2),
                    alive("h1/tc", &iface, 2),
                    member("a", 10),
                ],
                true,
            ),
            at(1),
        );
        let out = watch.baseline(&p, &BTreeMap::new(), &ids(&rules), "t");
        assert!(out.is_empty(), "held at the first look: {out:?}");
        // One of two holders goes: still held. The other: heard withdrawn,
        // and gone once a complete read since shows it unheld (core §8.1,
        // 0.37).
        let t = p.token(&alive("h1/tc", &iface, 1), false, at(2));
        assert!(watch.fold(&t, &p, "t").is_empty());
        let t = p.token(&alive("h1/tc", &other, 1), false, at(3));
        assert!(watch.fold(&t, &p, "t").is_empty(), "not covered");
        let t = p.token(&alive("h1/tc", &iface, 2), false, at(4));
        assert!(watch.fold(&t, &p, "t").is_empty(), "heard withdrawn");
        assert!(p.awaits_read(), "the interface has no token left");
        let read = [instance("h1/tc", 1), instance("h1/tc", 2), member("a", 10)];
        let t = p.seed(&seed(&read, true), at(4));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [(
                "netif".into(),
                "h1/tc/tc.netif.v1".into(),
                CondState::Firing,
                false
            )]
        );
        assert!(
            out[0]
                .evidence
                .contains("a complete read since shows it unheld"),
            "{}",
            out[0].evidence
        );
        // Make-before-break: a handover, then the older epoch's delete —
        // no gap; withdrawn, gone once a read says so.
        let t = p.token(&member("a", 11), true, at(5));
        assert!(watch.fold(&t, &p, "t").is_empty());
        let t = p.token(&member("a", 10), false, at(6));
        assert!(watch.fold(&t, &p, "t").is_empty(), "a handover is no gap");
        let t = p.token(&member("a", 11), false, at(7));
        assert!(watch.fold(&t, &p, "t").is_empty(), "heard withdrawn");
        // Heard again before the read: it never went, and nothing is said.
        let t = p.token(&member("a", 11), true, at(8));
        assert!(watch.fold(&t, &p, "t").is_empty(), "back before a read");
        let t = p.seed(&seed(&[instance("h1/tc", 1), member("a", 11)], true), at(8));
        assert!(watch.fold(&t, &p, "t").is_empty(), "held throughout");
        let t = p.token(&member("a", 11), false, at(9));
        assert!(watch.fold(&t, &p, "t").is_empty(), "heard withdrawn");
        let t = p.seed(&seed(&[instance("h1/tc", 1)], true), at(9));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [(
                "members".into(),
                "h1/nav/nav.v2/a".into(),
                CondState::Firing,
                false
            )]
        );
        assert_eq!(out[0].kind, NoticeKind::Liveliness);
    }

    /// A router's restart pages nothing (core §8.1, 0.37): every token is
    /// heard withdrawn at once, and the read the withdrawals owe shows them
    /// all, so no rule says gone, and nothing resolves either.
    #[test]
    fn a_router_restart_pages_nothing() {
        let rules = vec![
            rule("tc-gone", "instance-gone */tc"),
            rule("netif", "liveliness-gone zk2/*/tc/@zk/alive/tc.netif.v1/**"),
        ];
        let iface: zenkey_model::grammar::IfaceId = "tc.netif.v1".parse().unwrap();
        let fp = zenkey_model::grammar::Fp16::new("abababababababab").unwrap();
        let alive = alive_key(&addr("h1/tc"), &iface, &InstanceId::from_u64(1), &fp)
            .unwrap()
            .as_str()
            .to_string();
        let held = [instance("h1/tc", 1), alive.clone()];
        let mut watch = PresenceRules::new(&rules, "");
        let mut p = LivePresence::new(PresenceScope::all());
        watch.track(&mut p, at(0));
        p.seed(&seed(&held, true), at(1));
        assert!(
            watch
                .baseline(&p, &BTreeMap::new(), &ids(&rules), "t")
                .is_empty()
        );
        let mut t = p.token(&alive, false, at(2));
        t.extend(p.token(&instance("h1/tc", 1), false, at(2)));
        assert!(watch.fold(&t, &p, "t").is_empty(), "{t:?}");
        let t = p.seed(&seed(&held, true), at(3));
        assert!(watch.fold(&t, &p, "t").is_empty(), "{t:?}");
        assert_eq!(p.status(&addr("h1/tc")), AddressStatus::Up);
    }

    /// One scenario, both tools' readings of it (#747): zenwatch's rules
    /// over the projection, and `zenctl watchdog`'s path — the fleet's
    /// `RuleSet`, judged per tick from the same moves.
    struct Both {
        watch: PresenceRules,
        set: zenkey_fleet::RuleSet,
        ids: BTreeMap<String, String>,
        /// zenwatch's last word per (rule, address), and per rule observer.
        addresses: BTreeMap<(String, String), CondState>,
        observers: BTreeMap<String, CondState>,
        /// The watchdog's state per rule, from its transitions.
        watchdog: BTreeMap<String, CondState>,
    }

    impl Both {
        fn new(rules: &[(&str, &str)]) -> Both {
            let configured: Vec<Rule> = rules.iter().map(|(n, r)| rule(n, r)).collect();
            let conditions: Vec<zenkey_fleet::Condition> = rules
                .iter()
                .map(|(_, r)| zenkey_fleet::Condition::parse(r).expect("a rule"))
                .collect();
            Both {
                watch: PresenceRules::new(&configured, ""),
                set: zenkey_fleet::RuleSet::new(&conditions).expect("compiles"),
                ids: ids(&configured),
                addresses: BTreeMap::new(),
                observers: BTreeMap::new(),
                watchdog: BTreeMap::new(),
            }
        }

        fn track(&self, p: &mut LivePresence) {
            self.watch.track(p, at(0));
            self.set.track_presence(p, at(0));
        }

        fn absorb(&mut self, out: Vec<RuleNotice>) {
            for n in out {
                match n.what.as_str() {
                    OBSERVER => {
                        self.observers.insert(n.rule, n.state);
                    }
                    BASELINE => {}
                    what => {
                        self.addresses.insert((n.rule, what.to_string()), n.state);
                    }
                }
            }
        }

        fn tick(&mut self, p: &LivePresence) {
            let sweep = zenkey_fleet::SweepOutcome {
                doctor: None,
                presence: Some(p),
                ..zenkey_fleet::SweepOutcome::default()
            };
            for t in self.set.evaluate(tokio::time::Instant::now(), "t", sweep) {
                self.watchdog.insert(t.rule, t.to);
            }
        }

        fn first_look(&mut self, p: &LivePresence) {
            let out = self.watch.baseline(p, &BTreeMap::new(), &self.ids, "t");
            self.absorb(out);
            self.set.first_look(p);
            self.tick(p);
        }

        fn moved(&mut self, moved: &[PresenceTransition], p: &LivePresence) {
            let out = self.watch.fold(moved, p, "t");
            self.absorb(out);
            self.set.observe_presence(moved);
            self.tick(p);
        }

        /// zenwatch's per-address states, folded the watchdog's way: firing
        /// if any address is, else unobservable if the observer is, else ok.
        fn zenwatch(&self, name: &str) -> CondState {
            if self
                .addresses
                .iter()
                .any(|((r, _), s)| r == name && *s == CondState::Firing)
            {
                CondState::Firing
            } else if self.observers.get(name) == Some(&CondState::Unobservable) {
                CondState::Unobservable
            } else {
                CondState::Ok
            }
        }

        /// Both agree on every rule, and say `want`.
        fn agree(&self, step: &str, want: &[(&str, &str, CondState)]) {
            for (name, text, state) in want {
                assert_eq!(
                    (self.zenwatch(name), self.watchdog.get(*text).copied()),
                    (*state, Some(*state)),
                    "{step}: {name} — zenwatch {:?}, the watchdog {:?}",
                    self.addresses,
                    self.watchdog
                );
            }
        }
    }

    /// #747: the same scenario judged by `zenctl watchdog`'s path and by
    /// zenwatch's rule gives the same states, step by step — a first look
    /// that cannot see, a complete read with an exact address absent, a
    /// wildcard's address gone while another holds, a loss, the re-read
    /// that finds what it hid, and a wildcard that never matched.
    #[test]
    fn instance_gone_reads_the_same_in_zenwatch_and_the_watchdog() {
        use CondState::{Firing, Ok, Unobservable};
        let tc = ("tc-gone", "instance-gone */tc");
        let h9 = ("h9-gone", "instance-gone h9/tc");
        let gui = ("gui-gone", "instance-gone */gui");
        let mut both = Both::new(&[tc, h9, gui]);
        let mut p = LivePresence::new(PresenceScope::all());
        both.track(&mut p);
        let states = |a, b, c| [(tc.0, tc.1, a), (h9.0, h9.1, b), (gui.0, gui.1, c)];

        p.seed(&seed(&[instance("h1/tc", 1)], false), at(1));
        both.first_look(&p);
        both.agree(
            "a first read possibly incomplete",
            &states(Unobservable, Unobservable, Unobservable),
        );

        let t = p.seed(
            &seed(&[instance("h1/tc", 1), instance("h2/tc", 2)], true),
            at(2),
        );
        both.moved(&t, &p);
        both.agree("a complete read", &states(Ok, Firing, Ok));

        let t = p.token(&instance("h2/tc", 2), false, at(3));
        both.moved(&t, &p);
        both.agree("h2/tc heard withdrawn", &states(Ok, Firing, Ok));
        let t = p.seed(&seed(&[instance("h1/tc", 1)], true), at(3));
        both.moved(&t, &p);
        both.agree("h2/tc goes, a read says", &states(Firing, Firing, Ok));

        let t = p.dropped(3, at(4));
        both.moved(&t, &p);
        both.agree("a loss", &states(Firing, Firing, Unobservable));

        // h1/tc went inside the loss; h2/tc came back, and h9/tc started.
        let t = p.seed(
            &seed(&[instance("h2/tc", 3), instance("h9/tc", 1)], true),
            at(5),
        );
        both.moved(&t, &p);
        both.agree("the re-read", &states(Firing, Ok, Ok));

        let t = p.token(&instance("h1/tc", 4), true, at(6));
        both.moved(&t, &p);
        both.agree("h1/tc back", &states(Ok, Ok, Ok));
    }
}
