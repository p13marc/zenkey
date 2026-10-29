//! `arbitration.v1` (#909, BC), the projection: what a tool reads of one
//! consumer's arbitrated role (`spec/profiles/arbitration/v1.md` §2.14), as
//! `zenkey_model::arbitration`'s [`Reading`], and the tool's own view of it
//! ([`ViewBook`]).
//!
//! Session-free: a descriptor, a contract and an event are values, so the
//! doctor's own reads, zengui's sweep and a test project the same way.
//! Nothing is decided here: every answer is the model's
//! ([`crate::judge::arbitration`]), and every switch the model's
//! [`Arbiter`]'s.
//!
//! - **The subject** ([`RoleSubject`]) is one role of one consumer address,
//!   as the descriptor of its lowest instance listing the role names it
//!   (core §3.3): `declared_by`, the revision that descriptor lists for it,
//!   the bindings in their order and the parameter bindings, a
//!   `self.system` or `self.service` resolved to the consumer's own address
//!   (§2.14). A role read from a contract in hand with no consumer's
//!   descriptor has no binding: `no_descriptor` (text 0.2, F-272).
//! - **The declaration** ([`read_role`]) is §2.2's over every revision of
//!   the required interface the tool holds, the one they all give
//!   ([`held_over`]); the declaring contract not held is `no_contract`, a
//!   manifest's role (`"declared_by": null`) not this profile's.
//! - **The view** ([`ViewBook`]) is the model's [`Arbiter`] over the tool's
//!   own events, on its own monotonic clock, instants never going back
//!   (§2.9), with a governed interface's [`Incumbents`] beside it (§2.5,
//!   outcome 6), and what the tool knows it lost (§2.14's `lost`).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::{Duration, Instant};

use zenkey_model::arbitration::{
    Arbiter, Arbitrated, Binding, Cause, Counts, Declaration, Discards, Event, Held, Holding,
    Incumbents, Outcome, Reading, Revision, Role, Sample, View, held_over,
};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::timing::Mono;

use crate::model::catalog::{ContractState, Contracts, Observed};

/// The most switches a view keeps; the rest are counted
/// ([`ViewBook::switches_dropped`]).
pub const SWITCHES_KEPT: usize = 64;

/// One role of one consumer, as the tool read it (§2.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSubject {
    /// The consumer's address; `None` for a role read from a contract in
    /// hand with no consumer's descriptor.
    pub consumer: Option<Addr>,
    /// The instance whose descriptor was read.
    pub instance: Option<InstanceId>,
    pub role: String,
    /// The required interface.
    pub required: IfaceId,
    /// The interface whose contract declares the role; `None` for a role a
    /// component's manifest declares (`"declared_by": null`).
    pub declared_by: Option<IfaceId>,
    /// The revision of `declared_by` the descriptor lists, or the one in
    /// hand read with no descriptor; `None` when the descriptor lists none.
    pub declaring: Option<Fingerprint>,
    /// The descriptor's bindings, in order, a `self.system` resolved;
    /// `None` with no consumer's descriptor.
    pub bindings: Option<Vec<String>>,
    /// The parameter bindings, `self.system` and `self.service` resolved
    /// to the consumer's own address (§2.14; core R2).
    pub params: BTreeMap<String, String>,
}

/// A binding entry with its `self.system` resolved to `me`'s (core R1,
/// 0.20): a runtime spells it out at start, so a descriptor should never
/// list one, and one listed is read so.
fn resolve_entry(entry: &str, me: &Addr) -> String {
    match entry.split_once('/') {
        Some(("self.system", svc)) => format!("{}/{svc}", me.system),
        _ => entry.to_owned(),
    }
}

/// The parameter bindings with `self.system` and `self.service` resolved
/// to `me`'s own address (core R2).
pub fn resolve_params(params: &BTreeMap<String, String>, me: &Addr) -> BTreeMap<String, String> {
    params
        .iter()
        .map(|(k, v)| {
            let v = match v.as_str() {
                "self.system" => me.system.to_string(),
                "self.service" => me.service.to_string(),
                other => other.to_owned(),
            };
            (k.clone(), v)
        })
        .collect()
}

/// Every role the descriptors list, or the one `role` names: one subject
/// per consumer address and role, from the descriptor of its lowest
/// instance that lists it (`served` in address then instance order, as a
/// presence read keeps its descriptors).
pub fn subjects<'d>(
    served: impl IntoIterator<Item = (&'d Addr, &'d InstanceId, &'d Descriptor)>,
    role: Option<&str>,
) -> Vec<RoleSubject> {
    let mut by: BTreeMap<(Addr, String), RoleSubject> = BTreeMap::new();
    for (addr, instance, d) in served {
        for r in &d.requires {
            if role.is_some_and(|w| w != r.role) {
                continue;
            }
            let Ok(required) = IfaceId::from_str(&r.interface) else {
                continue;
            };
            let declared_by = r
                .declared_by
                .as_deref()
                .and_then(|s| IfaceId::from_str(s).ok());
            let declaring = declared_by.as_ref().and_then(|by| {
                let want = by.to_string();
                d.interfaces
                    .iter()
                    .find(|e| e.iface == want)
                    .and_then(|e| Fingerprint::parse(&e.contract).ok())
            });
            by.entry((addr.clone(), r.role.clone()))
                .or_insert_with(|| RoleSubject {
                    consumer: Some(addr.clone()),
                    instance: Some(instance.clone()),
                    role: r.role.clone(),
                    required,
                    declared_by,
                    declaring,
                    bindings: Some(r.bindings.iter().map(|b| resolve_entry(b, addr)).collect()),
                    params: resolve_params(&r.params, addr),
                });
        }
    }
    by.into_values().collect()
}

/// A role read from a contract in hand with no consumer's descriptor
/// (§2.14, text 0.2, F-272): no binding, no parameter bindings, no view.
pub fn offline_subject(
    declaring: &Contract,
    fingerprint: &Fingerprint,
    role: &str,
) -> Option<RoleSubject> {
    let req = declaring.requires.get(role)?;
    Some(RoleSubject {
        consumer: None,
        instance: None,
        role: role.to_owned(),
        required: req.interface.clone(),
        declared_by: Some(declaring.iface.clone()),
        declaring: Some(fingerprint.clone()),
        bindings: None,
        params: BTreeMap::new(),
    })
}

/// The declaring contract of a subject, as the tool holds it.
#[derive(Debug, Clone, Copy)]
pub enum Declaring<'a> {
    /// The revision the descriptor lists, in hand.
    Held(&'a Contract),
    /// Not in hand, with why: `no_contract`.
    Missing(&'a str),
}

/// What [`read_role`] read: the model's reading with no view yet, the
/// declaration when it is arbitrated, and what the tool should know of it.
#[derive(Debug, Clone)]
pub struct RoleRead {
    pub reading: Reading,
    /// The valid declaration, when the revisions held give one.
    pub arbitrated: Option<Arbitrated>,
    /// The revisions of the required interface it was read over, by
    /// fingerprint.
    pub revisions: Vec<Fingerprint>,
    /// The required interface is governed (`redundancy.v1` §2.2) in every
    /// revision held: the view counts §2.5's `not_incumbent`.
    pub governed: bool,
    /// Whether the role carries `arbitration.policy` as far as the tool can
    /// see: `None` when its declaring contract is not in hand.
    pub carries_policy: Option<bool>,
    /// What could not be read, one line each.
    pub notes: Vec<String>,
}

/// §2.14's reading of `s`: its declaration over `required`, the revisions
/// of the required interface the tool holds (each with its fingerprint;
/// any other interface's set aside, one fingerprint read once), and its
/// binding. A manifest's role is not this profile's: no tool sees its
/// policy (§2.14).
pub fn read_role(
    s: &RoleSubject,
    declaring: Declaring<'_>,
    required: &[(Fingerprint, &Contract)],
) -> RoleRead {
    let mut notes = Vec::new();
    let mut seen = BTreeSet::new();
    let revs: Vec<(Fingerprint, &Contract)> = required
        .iter()
        .filter(|(fp, c)| c.iface == s.required && seen.insert(fp.clone()))
        .map(|(fp, c)| (fp.clone(), *c))
        .collect();
    let binding = match &s.bindings {
        None => None,
        Some(b) => match Binding::parse(b) {
            Ok(b) => Some(b),
            Err(e) => {
                notes.push(format!(
                    "its descriptor's binding is unreadable ({e}): the binding and the selection \
                     are read as with no descriptor"
                ));
                None
            }
        },
    };
    let mut carries_policy = Some(false);
    let declaration = match (&s.declared_by, declaring) {
        (None, _) => Held::Read(Declaration::NotArbitrated),
        (Some(_), Declaring::Missing(why)) => {
            carries_policy = None;
            notes.push(format!("its declaring contract is not in hand: {why}"));
            Held::NoContract
        }
        (Some(by), Declaring::Held(c)) => match c.requires.get(&s.role) {
            None => {
                carries_policy = None;
                notes.push(format!(
                    "{by}'s revision in hand declares no role {:?}: the descriptor names another",
                    s.role
                ));
                Held::NoContract
            }
            Some(req) => {
                carries_policy = Some(
                    req.annotations
                        .contains_key(zenkey_model::arbitration::POLICY),
                );
                let rs: Vec<Revision<'_>> = revs.iter().map(|(_, c)| Revision::of(c)).collect();
                held_over(&Role::of(c, req), &rs, &s.params)
            }
        },
    };
    let arbitrated = match &declaration {
        Held::Read(Declaration::Arbitrated(a)) => Some(a.clone()),
        _ => None,
    };
    let governed = !revs.is_empty()
        && revs
            .iter()
            .all(|(_, c)| zenkey_model::redundancy::is_governed(c));
    RoleRead {
        reading: Reading {
            declaration,
            binding,
            view: None,
        },
        arbitrated,
        revisions: revs.into_iter().map(|(fp, _)| fp).collect(),
        governed,
        carries_policy,
        notes,
    }
}

/// Whether a subject is the tool's to read without `--role`: its role
/// carries a policy, or might (its declaring contract not in hand, which is
/// `no_contract`, never "not this profile's"). A manifest's role is shown
/// only when asked by name.
pub fn in_scope(r: &RoleRead) -> bool {
    r.carries_policy != Some(false)
}

/// The descriptors a presence read served, in address then instance order.
pub fn served(o: &Observed) -> impl Iterator<Item = (&Addr, &InstanceId, &Descriptor)> {
    o.descriptors
        .iter()
        .flatten()
        .filter_map(|((a, i), read)| Some((a, i, read.descriptor()?)))
}

/// The instances holding `iface`'s interface token, per address, that
/// `tokens` show; `exact`'s address included, none held too, since a read
/// of one address says what it holds (§2.7).
pub fn held_by_address<'k>(
    tokens: impl IntoIterator<Item = &'k ZkKey>,
    iface: &IfaceId,
    exact: Option<&Addr>,
) -> BTreeMap<Addr, BTreeSet<InstanceId>> {
    let mut by: BTreeMap<Addr, BTreeSet<InstanceId>> = BTreeMap::new();
    for k in tokens {
        if let ZkKey::Alive {
            addr,
            iface: i,
            instance,
            ..
        } = k
            && i == iface
        {
            by.entry(addr.clone()).or_default().insert(instance.clone());
        }
    }
    if let Some(a) = exact {
        by.entry(a.clone()).or_default();
    }
    by
}

/// The roles a presence read already made shows, read with no view over
/// the revisions `contracts` holds: every role carrying a policy (or whose
/// declaring contract is not in hand), or the one `role` names. The
/// doctor's `arbitration-declaration` and zengui's Workbench read so.
pub fn roles_in(
    o: &Observed,
    contracts: &dyn Contracts,
    role: Option<&str>,
) -> Vec<(RoleSubject, RoleRead)> {
    let mut out = Vec::new();
    for s in subjects(served(o), role) {
        let declaring = match (&s.declared_by, &s.declaring) {
            (None, _) => Err("a manifest's role".to_owned()),
            (Some(by), Some(fp)) => match contracts.state(by, fp) {
                Some(ContractState::Held(r)) => Ok(r),
                Some(ContractState::Unavailable { .. }) => {
                    Err(format!("{by} {fp}: no holder served its bundle"))
                }
                Some(ContractState::Unreadable { reason }) => Err(format!("{by} {fp}: {reason}")),
                None => Err(format!("{by} {fp}: not retrieved")),
            },
            (Some(by), None) => Err(format!(
                "the descriptor lists no revision of {by}, which declares the role"
            )),
        };
        let required = contracts.held_of(&s.required);
        let pairs: Vec<(Fingerprint, &Contract)> = required
            .iter()
            .map(|r| (r.fingerprint().clone(), r.contract()))
            .collect();
        let rr = match &declaring {
            Ok(r) => read_role(&s, Declaring::Held(r.contract()), &pairs),
            Err(why) => read_role(&s, Declaring::Missing(why), &pairs),
        };
        if role.is_some() || in_scope(&rr) {
            out.push((s, rr));
        }
    }
    out
}

/// One switch of the tool's view, at its instants (§2.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSwitch {
    pub from: Option<Addr>,
    pub to: Option<Addr>,
    pub cause: Cause,
    pub at: Instant,
    /// `from`'s last arrival.
    pub last: Option<Instant>,
}

/// The tool's own view of one role (§2.14): the model's [`Arbiter`] over
/// the tool's own arrivals, watch and clock, every event at an instant
/// never before the last one's (§2.9), and a governed interface's
/// [`Incumbents`] beside it. It is never the consumer's.
#[derive(Debug, Clone)]
pub struct ViewBook {
    arbiter: Arbiter,
    iface: IfaceId,
    origin: Instant,
    last: Instant,
    incumbents: Option<Incumbents>,
    switches: Vec<ViewSwitch>,
    switches_dropped: u64,
    /// The last instant it knew it lost a sample: a queue's refusal, or a
    /// stall of its receive path (§2.14's `lost`).
    lost_at: Option<Instant>,
    losses: u64,
}

impl ViewBook {
    /// A view of a role declared `declared`, bound by `binding`, its
    /// parameter bindings `params` resolved, started at `origin`: the
    /// dead-man, cause `start` (§2.11). `governed`: §2.5's outcome 6 read.
    pub fn new(
        declared: &Arbitrated,
        binding: Binding,
        params: BTreeMap<String, String>,
        governed: bool,
        origin: Instant,
    ) -> ViewBook {
        let mut arbiter = Arbiter::new(declared, binding, params);
        let step = arbiter.start(Mono::ZERO);
        let mut book = ViewBook {
            arbiter,
            iface: declared.iface.clone(),
            origin,
            last: origin,
            incumbents: governed.then(Incumbents::default),
            switches: Vec::new(),
            switches_dropped: 0,
            lost_at: None,
            losses: 0,
        };
        book.record(origin, step.switches);
        book
    }

    fn mono(&self, at: Instant) -> Mono {
        Mono::after(at.saturating_duration_since(self.origin))
    }

    fn instant(&self, m: Mono) -> Instant {
        self.origin + m.since_origin()
    }

    /// The event's instant, never before the last one's (§2.9): events
    /// reach the tool on several threads, each reading the clock first.
    fn settle(&mut self, at: Instant) -> Instant {
        let at = at.max(self.last);
        self.last = at;
        at
    }

    fn record(&mut self, at: Instant, switches: Vec<zenkey_model::arbitration::Switch>) {
        for s in switches {
            if self.switches.len() >= SWITCHES_KEPT {
                self.switches_dropped += 1;
                continue;
            }
            let last = s.last.map(|m| self.instant(m));
            self.switches.push(ViewSwitch {
                from: s.from,
                to: s.to,
                cause: s.cause,
                at,
                last,
            });
        }
    }

    fn observe(&mut self, at: Instant, event: Event<'_>) -> (Option<Outcome>, Vec<Addr>) {
        let at = self.settle(at);
        let step = self.arbiter.observe(self.mono(at), event);
        self.record(at, step.switches);
        (step.outcome, step.reread)
    }

    /// A presence event, to the arbiter and to the incumbents, each kept
    /// through a move (§2.8). The addresses to read again.
    fn presence(&mut self, at: Instant, event: Event<'_>, held: Option<Holding>) -> Vec<Addr> {
        let moving = self.arbiter.moving();
        let stood = matches!(event, Event::Stood);
        let (_, reread) = self.observe(at, event);
        if let Some(inc) = self.incumbents.as_mut() {
            if let Some(h) = held {
                inc.hear(h, moving);
            }
            if stood && moving {
                inc.stood();
            }
        }
        reread
    }

    /// A sample of the role's subscription at its arrival `at`: its key,
    /// base-relative, whether it is a delete, and its stamp's clock id.
    /// Nothing is expired: a tool holds no ε of the consumer's binding
    /// (§2.5, outcome 5: with none stated, nothing is).
    pub fn sample(
        &mut self,
        at: Instant,
        key: &str,
        delete: bool,
        stamp: Option<u128>,
    ) -> Option<Outcome> {
        let not_incumbent = !delete
            && match (&self.incumbents, self.arbiter.identify(key)) {
                (Some(inc), Ok(addr)) => !inc.accepts(&addr, stamp),
                _ => false,
            };
        self.observe(
            at,
            Event::Sample(Sample {
                key,
                delete,
                expired: false,
                not_incumbent,
            }),
        )
        .0
    }

    /// An interface token its watch heard, declared (`held`) or withdrawn:
    /// the addresses to read again (§2.7).
    pub fn token(&mut self, at: Instant, key: &str, held: bool) -> Vec<Addr> {
        let h = Holding::of_token(key, held, &self.iface, self.arbiter.binding());
        let ev = if held {
            Event::Declared(key)
        } else {
            Event::Withdrawn(key)
        };
        self.presence(at, ev, h)
    }

    /// A read of `address`'s presence: whether it is complete, and the
    /// instances it shows holding the interface's token (§2.7).
    pub fn read(
        &mut self,
        at: Instant,
        address: Addr,
        complete: bool,
        held: BTreeSet<InstanceId>,
    ) -> Vec<Addr> {
        let h = Holding::Read(address.clone(), complete, held.clone());
        self.presence(
            at,
            Event::Read {
                address,
                complete,
                held,
            },
            Some(h),
        )
    }

    /// The watch session's transport moved, or a withdrawal began a move
    /// (§2.8, F-268).
    pub fn moved(&mut self, at: Instant) {
        self.presence(at, Event::Moved, None);
    }

    /// The round trip after the last move stands (§2.8).
    pub fn stood(&mut self, at: Instant) -> Vec<Addr> {
        self.presence(at, Event::Stood, None)
    }

    /// Its timer (§2.9).
    pub fn tick(&mut self, at: Instant) {
        self.observe(at, Event::Tick);
    }

    /// A holder's `meta.zid`, from its descriptor (`redundancy.v1` §2.10).
    pub fn described(&mut self, addr: Addr, instance: InstanceId, zid: u128) {
        if let Some(inc) = self.incumbents.as_mut() {
            inc.described(addr, instance, zid);
        }
    }

    /// The holders whose descriptor to get: none when not governed.
    pub fn undescribed(&self) -> Vec<(Addr, InstanceId)> {
        self.incumbents
            .as_ref()
            .map(Incumbents::undescribed)
            .unwrap_or_default()
    }

    /// Whether §2.5's outcome 6 is read.
    pub fn governed(&self) -> bool {
        self.incumbents.is_some()
    }

    /// The active provider's due instant, to arm the one-shot timer for.
    pub fn due(&self) -> Option<Instant> {
        self.arbiter.due().map(|m| self.instant(m))
    }

    /// The tool lost samples at `at` (§2.14): a refusal of its queue, or a
    /// stall of its receive path.
    pub fn lose(&mut self, at: Instant) {
        self.losses += 1;
        self.lost_at = Some(self.lost_at.map_or(at, |l| l.max(at)));
    }

    /// How many losses it was told of.
    pub fn losses(&self) -> u64 {
        self.losses
    }

    /// The view at `end`: how long it listened, whether it knows it lost a
    /// sample within the last deadline, and the provider it selects.
    pub fn view(&self, end: Instant) -> View {
        let deadline = self.arbiter.deadline();
        View {
            listened: end.saturating_duration_since(self.origin),
            lost: self
                .lost_at
                .is_some_and(|l| end.saturating_duration_since(l) <= deadline),
            active: self.arbiter.active().cloned(),
        }
    }

    /// When it started: the origin of every instant it reports.
    pub fn origin(&self) -> Instant {
        self.origin
    }

    /// Milliseconds from its start to `at`.
    pub fn ms(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.origin).as_secs_f64() * 1e3
    }

    /// The deadline it judges by (§2.3).
    pub fn deadline(&self) -> Duration {
        self.arbiter.deadline()
    }

    /// The switches it made, the first its start's.
    pub fn switches(&self) -> &[ViewSwitch] {
        &self.switches
    }

    /// How many switches past [`SWITCHES_KEPT`] were counted, not kept.
    pub fn switches_dropped(&self) -> u64 {
        self.switches_dropped
    }

    /// Whether a move is pending (§2.8).
    pub fn moving(&self) -> bool {
        self.arbiter.moving()
    }

    /// The providers known, in rank order (§2.4), each with its counts
    /// (§2.5) and whether it is withdrawn (§2.7).
    pub fn providers(&self) -> Vec<(Addr, Counts, bool)> {
        self.arbiter
            .providers()
            .map(|a| (a.clone(), self.arbiter.counts(a), self.arbiter.withdrawn(a)))
            .collect()
    }

    /// The samples with no provider (§2.5).
    pub fn discards(&self) -> Discards {
        self.arbiter.discards()
    }

    /// The binding it ranks by.
    pub fn binding(&self) -> &Binding {
        self.arbiter.binding()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::arbitration::Policy;
    use zenkey_model::grammar::KindToken;
    use zenkey_model::template::Template;

    fn declared() -> Arbitrated {
        Arbitrated {
            policy: Policy::Priority,
            iface: "steer.v1".parse().unwrap(),
            token: KindToken::Stream,
            template: Template::parse("cmd").unwrap(),
            deadline: Duration::from_millis(300),
            period: Some(Duration::from_millis(50)),
        }
    }

    const SAFETY: &str = "zk2/lab/safety/steer.v1/stream/cmd";
    const TELEOP: &str = "zk2/lab/teleop/steer.v1/stream/cmd";

    /// The view is the arbiter's: the dead-man at its start, a higher
    /// provider taking over at its first arrival, the time decided first
    /// at a later event, and an event that comes earlier than the last one
    /// decided at the last one's instant (§2.9).
    #[test]
    fn the_view_is_the_arbiters_over_the_tools_instants() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let binding = Binding::parse(&["lab/safety", "lab/teleop"]).unwrap();
        let mut v = ViewBook::new(&declared(), binding, BTreeMap::new(), false, t0);
        assert_eq!(v.switches().len(), 1);
        assert_eq!(v.switches()[0].cause, Cause::Start);
        assert_eq!(
            v.sample(ms(10), TELEOP, false, None),
            Some(Outcome::Delivered)
        );
        assert_eq!(
            v.sample(ms(20), SAFETY, false, None),
            Some(Outcome::Delivered)
        );
        assert_eq!(
            v.sample(ms(5), TELEOP, false, None),
            Some(Outcome::NotActive)
        );
        assert_eq!(v.due(), Some(ms(320)));
        let view = v.view(ms(100));
        assert_eq!(view.active, Some("lab/safety".parse().unwrap()));
        assert!(!view.lost);
        assert_eq!(view.listened, Duration::from_millis(100));
        v.tick(ms(400));
        assert_eq!(v.view(ms(400)).active, None);
        let causes: Vec<Cause> = v.switches().iter().map(|s| s.cause).collect();
        assert_eq!(
            causes,
            [Cause::Start, Cause::Sample, Cause::Sample, Cause::Deadline]
        );
    }

    /// §2.14's `lost`: a loss within the last deadline, never one before.
    #[test]
    fn a_loss_counts_within_the_last_deadline() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let binding = Binding::parse(&["lab/safety"]).unwrap();
        let mut v = ViewBook::new(&declared(), binding, BTreeMap::new(), false, t0);
        v.lose(ms(100));
        assert!(v.view(ms(400)).lost, "300 ms after it");
        assert!(!v.view(ms(401)).lost, "past the deadline");
    }

    /// A governed interface's view counts a sample not the incumbent's
    /// `not_incumbent` (§2.5, outcome 6): nothing before an incumbent is
    /// seen, its own stamp once its `meta.zid` is held.
    #[test]
    fn a_governed_view_counts_only_the_incumbents_samples() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let binding = Binding::parse(&["lab/safety"]).unwrap();
        let mut v = ViewBook::new(&declared(), binding, BTreeMap::new(), true, t0);
        assert_eq!(
            v.sample(ms(1), SAFETY, false, Some(0xa1)),
            Some(Outcome::NotIncumbent)
        );
        let lab_safety: Addr = "lab/safety".parse().unwrap();
        let i: InstanceId = "000000000000000a".parse().unwrap();
        v.read(ms(2), lab_safety.clone(), true, [i.clone()].into());
        assert_eq!(v.undescribed(), [(lab_safety.clone(), i.clone())]);
        v.described(lab_safety, i, 0xa1);
        assert_eq!(
            v.sample(ms(3), SAFETY, false, Some(0xb2)),
            Some(Outcome::NotIncumbent)
        );
        assert_eq!(
            v.sample(ms(4), SAFETY, false, Some(0xa1)),
            Some(Outcome::Delivered)
        );
    }

    /// The subjects of a descriptor: one per address and role, from its
    /// lowest instance, `self.system` and `self.service` resolved.
    #[test]
    fn a_descriptors_roles_are_its_subjects() {
        let d: Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/rudder-l",
            "instance": "000000000000000a",
            "interfaces": [{"iface": "rudder.v1", "contract": format!("sha256:{}", "a".repeat(64)), "minor": 0}],
            "requires": [{
                "role": "cmd", "interface": "steer.v1", "declared_by": "rudder.v1",
                "bindings": ["self.system/safety", "lab/teleop"],
                "params": {"who": "self.service"}
            }]
        }))
        .unwrap();
        let a: Addr = "lab/rudder-l".parse().unwrap();
        let i: InstanceId = "000000000000000a".parse().unwrap();
        let s = subjects([(&a, &i, &d)], None);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].bindings.as_deref(),
            Some(&["lab/safety".to_owned(), "lab/teleop".to_owned()][..])
        );
        assert_eq!(s[0].params["who"], "rudder-l");
        assert!(s[0].declaring.is_some());
        assert!(subjects([(&a, &i, &d)], Some("other")).is_empty());
    }
}
