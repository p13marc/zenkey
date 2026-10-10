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
//! address needs a complete read behind it, as the projection's `down`
//! does. Whatever the projection cannot establish — no seed yet, a seed
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

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use zenkey_fleet::{
    AddressStatus, CondState, LivePresence, PresenceBasis, PresenceChange, PresenceSource,
    PresenceTransition, ResolvedTarget as Target,
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
    /// What the rule last said per identity: `true` held, `false` gone.
    known: BTreeMap<String, bool>,
    /// The instances (interface form) or epochs (member form) holding each
    /// identity's tokens.
    holders: BTreeMap<String, BTreeSet<InstanceId>>,
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
                    target,
                    form,
                    keyexpr,
                    known: BTreeMap::new(),
                    holders: BTreeMap::new(),
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
            presence.track_pattern(&r.target, at);
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
            let observer = observer_state(presence.basis());
            let held = r.holdings(presence, &namespace);
            let mut up = Vec::new();
            let mut down = Vec::new();
            for (identity, holds) in &held {
                r.known.insert(identity.clone(), *holds);
                let (address, _) = split_identity(identity);
                if *holds {
                    up.push(identity.clone());
                    out.push(r.notice(identity, CondState::Ok, None, false, at, None, address));
                } else {
                    down.push(identity.clone());
                    out.push(r.notice(identity, CondState::Firing, None, true, at, None, address));
                }
            }
            // The ledger's memory, against what the projection shows now.
            let id = rule_ids.get(&r.name).cloned().unwrap_or_default();
            for remembered in announced.get(&r.name).into_iter().flatten() {
                let Some(identity) = remembered
                    .strip_prefix(&id)
                    .and_then(|s| s.strip_prefix(':'))
                else {
                    continue;
                };
                if identity == OBSERVER || held.contains_key(identity) {
                    continue;
                }
                // Nothing of it held, and no record: gone if a complete read
                // stands behind it, unknowable otherwise.
                if presence.basis() == PresenceBasis::Complete {
                    r.known.insert(identity.to_string(), false);
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
        for r in &mut self.rules {
            for t in moved {
                if !r.target.matches(&t.address) {
                    continue;
                }
                if let Some((identity, holds)) = r.apply(t, &namespace) {
                    let prior = r.known.get(&identity).copied();
                    if prior == Some(holds) {
                        continue;
                    }
                    // Nothing can be gone that was never seen held — but an
                    // address the operator named exactly is watched from the
                    // start.
                    let named = r.form == Form::Address && r.target.concrete.is_some();
                    if prior.is_none() && !holds && !named {
                        continue;
                    }
                    r.known.insert(identity.clone(), holds);
                    let state = if holds {
                        CondState::Ok
                    } else {
                        CondState::Firing
                    };
                    let prior = prior.map(|h| if h { CondState::Ok } else { CondState::Firing });
                    let address = Some(t.address.to_string());
                    out.push(r.notice(&identity, state, prior, false, at, Some(t), address));
                }
            }
            let observer = observer_state(presence.basis());
            if r.observer != Some(observer) {
                let prior = r.observer;
                r.observer = Some(observer);
                out.push(r.observer_notice(observer, prior, presence.basis(), false, at));
            }
        }
        out
    }
}

/// The observer's state under a basis: only a complete read can see.
fn observer_state(basis: PresenceBasis) -> CondState {
    match basis {
        PresenceBasis::Complete => CondState::Ok,
        _ => CondState::Unobservable,
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

    /// What the projection holds now, per identity, for the addresses this
    /// rule's pattern matches: `true` held, `false` gone (an address down,
    /// a complete read behind it). Unobservable addresses are left out.
    fn holdings(&mut self, presence: &LivePresence, namespace: &str) -> BTreeMap<String, bool> {
        let mut out = BTreeMap::new();
        let target = self.target.clone();
        for (addr, rec) in presence.matching(&target) {
            match self.form {
                Form::Address => match rec.status {
                    AddressStatus::Up => {
                        out.insert(addr.to_string(), true);
                    }
                    // Down at the first look is news only for an address
                    // the operator named: under a wildcard, a record with
                    // nothing held is one another rule tracked, never one
                    // this rule saw.
                    AddressStatus::Down if self.target.concrete.is_some() => {
                        out.insert(addr.to_string(), false);
                    }
                    AddressStatus::Down | AddressStatus::Unobservable => {}
                },
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
        out
    }

    /// One move of the projection, as this rule reads it: the identity it
    /// moved and whether that identity is held now — or nothing, when the
    /// move is not this rule's (another form, a key the selector does not
    /// cover, an address going unobservable, which is the observer's).
    fn apply(&mut self, t: &PresenceTransition, namespace: &str) -> Option<(String, bool)> {
        let addr: &Addr = &t.address;
        match (&self.form, &t.change) {
            (Form::Address, PresenceChange::Status { to, .. }) => match to {
                AddressStatus::Up => Some((addr.to_string(), true)),
                AddressStatus::Down => Some((addr.to_string(), false)),
                AddressStatus::Unobservable => None,
            },
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
            Some(PresenceSource::Dropped) | None => "",
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
        let target = &self.target.address;
        let evidence = match basis {
            PresenceBasis::Complete => format!(
                "a complete presence read stands: every address under {target} is observable \
                 again"
            ),
            PresenceBasis::Lost { dropped } => format!(
                "the presence projection lost {dropped} liveliness event(s): every address \
                 under {target} is unobservable until a complete re-read — a token gone or \
                 back inside that span was not seen, and nothing here says it is fine (RFC 13 \
                 §3 O6)"
            ),
            PresenceBasis::Incomplete => format!(
                "the presence read ended at its timeout, possibly incomplete (spec §8.1): an \
                 address under {target} it did not show is unobservable, not down; re-reading"
            ),
            PresenceBasis::Unseeded => {
                format!("presence under {target} has not been read yet: nothing is established")
            }
        };
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
            PresenceBasis::Incomplete => {
                "a read that ended at its timeout: what it did not show is unobservable".into()
            }
            PresenceBasis::Lost { dropped } => {
                format!("{dropped} event(s) lost: unobservable until a complete re-read")
            }
            PresenceBasis::Unseeded => "no read yet".into(),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use zenkey_fleet::PresenceScope;

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
    /// notification at the rule's severity; an address up is a quiet `ok`;
    /// a pattern that matches nothing yet says so in an `info` baseline.
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
                ("tc-gone".into(), "h1/tc".into(), CondState::Ok, false),
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
        let h9 = &out[2];
        assert!(
            h9.passthrough && h9.severity.is_none(),
            "the rule's own severity"
        );
        assert!(
            h9.lines.iter().any(|l| l == "- gone: h9/tc"),
            "{:?}",
            h9.lines
        );
        let gui = &out[3];
        assert_eq!(gui.severity.as_deref(), Some("info"));
        assert!(
            gui.evidence.contains("no address matches */gui yet"),
            "{}",
            gui.evidence
        );
    }

    /// The live poles: an address going is firing with its prior and the
    /// move that decided it; coming back is ok; a second address appearing
    /// under a pattern is an identity of its own; a `Dropped(n)` is the
    /// observer going unobservable — never an `ok` — with no per-address
    /// notice, and the complete re-read restores it and finds what the loss
    /// hid.
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
                .len()
                == 1
        );

        // A second address appears under the pattern.
        let t = p.token(&instance("h2/tc", 2), true, at(2));
        let out = watch.fold(&t, &p, "t");
        assert_eq!(
            summary(&out),
            [("tc-gone".into(), "h2/tc".into(), CondState::Ok, false)]
        );
        assert_eq!(out[0].prior, None, "first sight");

        // It goes: firing, from ok, with the status move attached.
        let t = p.token(&instance("h2/tc", 2), false, at(3));
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
                from: AddressStatus::Up,
                to: AddressStatus::Down
            }
        );
        assert!(
            out[0].evidence.contains("heard as it happened"),
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
        assert_eq!(
            summary(&out),
            [
                (
                    "netif".into(),
                    "h1/tc/tc.netif.v1".into(),
                    CondState::Ok,
                    false
                ),
                (
                    "members".into(),
                    "h1/nav/nav.v2/a".into(),
                    CondState::Ok,
                    false
                ),
            ]
        );
        // One of two holders goes: still held. The other: gone.
        let t = p.token(&alive("h1/tc", &iface, 1), false, at(2));
        assert!(watch.fold(&t, &p, "t").is_empty());
        let t = p.token(&alive("h1/tc", &other, 1), false, at(3));
        assert!(watch.fold(&t, &p, "t").is_empty(), "not covered");
        let t = p.token(&alive("h1/tc", &iface, 2), false, at(4));
        assert_eq!(
            summary(&watch.fold(&t, &p, "t")),
            [(
                "netif".into(),
                "h1/tc/tc.netif.v1".into(),
                CondState::Firing,
                false
            )]
        );
        // Make-before-break: a handover, then the older epoch's delete —
        // no gap; withdrawn, gone.
        let t = p.token(&member("a", 11), true, at(5));
        assert!(watch.fold(&t, &p, "t").is_empty());
        let t = p.token(&member("a", 10), false, at(6));
        assert!(watch.fold(&t, &p, "t").is_empty(), "a handover is no gap");
        let t = p.token(&member("a", 11), false, at(7));
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
}
