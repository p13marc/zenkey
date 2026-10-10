//! Live presence, per address (#614, FL1): which instances of each
//! `<system>/<service>` hold their tokens now, and whether the address is
//! **up**, **down** or **unobservable**, folded from one liveliness read and
//! the liveliness events that follow it (spec §8.1).
//!
//! [`crate::bus::presence::read_tokens`] answers "who is here" once, as an
//! [`Observed`] with its `complete` flag. A notifier asleep beside a fleet
//! and a roster that stays open for hours need the same answer kept
//! current, and need the *moves* as values: an address up or down, an
//! instance appeared or gone, an interface token appeared or gone, a member
//! token appeared, handed over or gone. [`LivePresence`] is that fold. It
//! takes a seed ([`LivePresence::seed`]), then each token put or delete
//! ([`LivePresence::token`]), then any `Dropped(n)`
//! ([`LivePresence::dropped`]), and hands back [`PresenceTransition`]s, each
//! stamped with the time its caller passed in.
//!
//! **Three states, and the down pole is the one that needs evidence.**
//! - *Up*: at least one instance token of the address is held, and nothing
//!   was lost since it was heard.
//! - *Down*: none is held, and a complete seed stands behind the claim — the
//!   seed showed none, or showed some and every one was heard going since.
//! - *Unobservable*: neither could be established. No seed, a seed that
//!   ended at its timeout (possibly incomplete, §8.1), a `Dropped(n)` since
//!   the last complete seed, or a subscription's history alone: liveliness
//!   history has no end-of-history mark, so a token not heard yet is not
//!   absent. Silence is never a verdict (O5, S6), and *unobservable* is its
//!   own state, never folded into either pole (R7).
//!
//! An address going down is presence's word for "no instance token visible
//! to this reader". It is not a level, and it is not a verdict about the
//! service's health (that is `health.v1`'s); a refused read is complete and
//! empty too (§8.1, 0.8), which a consumer that reports absence says.
//!
//! **What the transitions carry.** Each one names its address, its change,
//! when it was folded, and how it was learned ([`PresenceSource`]): heard as
//! it happened, found by a seed's comparison with what was held before
//! (it happened at or before the time it carries), or the basis lost to a
//! `Dropped(n)`. Within one fold the token changes come first and the
//! address's status last, so a consumer reads cause before effect.
//!
//! **Make-before-break is a handover, never a gap** (§8.1). A member's
//! newer epoch declared while an older one is held is a
//! [`PresenceChange::MemberHandover`], and the older epoch's delete after it
//! is no transition at all; the member was never without a token. A
//! re-minted instance (`Service::new_epoch`) is an instance appeared, then
//! an instance gone, and the address stays up between them. A gap the owner
//! did leave (the old token deleted before the new one was declared) is
//! shown as the gap it was: gone, then appeared.
//!
//! **What is not presence** is counted, never dropped silently: a key
//! outside the namespace, a key that is not zk2 (the bus is shared, §1.7),
//! a zk2 key that is not a token (a data key, a contract key), and a token
//! of an address outside the [`Scope`] ([`PresenceIgnored`]).
//!
//! **Bounded.** Held tokens are bounded by the bus itself (the presence
//! budget, §8.3). What is not is the memory of addresses that went away:
//! records with nothing held and no consumer [tracking](LivePresence::track)
//! them are kept up to a bound ([`DEFAULT_MAX_GONE`]) and the oldest beyond
//! it forgotten, and counted ([`LivePresence::forgotten`]). Forgetting one
//! changes no answer [`LivePresence::status`] gives: a record with nothing
//! held reads exactly as an address never heard of does.
//!
//! **Not serialized.** Nothing here derives `Serialize`: no wire shape reads
//! a transition yet. When zenwatch's notices or a `.zrec` carry one (FL2),
//! the shape moves to `report/` with its pinned test, by the placement rule.
//!
//! Session-free: [`crate::bus::presence::PresenceFeed`] seeds it from a
//! liveliness read and feeds it from a [`crate::Monitor`], and a replay can
//! feed it the same values from a file.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::SystemTime;

use zenkey_model::grammar::{Addr, Fp16, IfaceId, InstanceId, ZkKey};

use crate::bus::presence::Scope;
use crate::model::catalog::Observed;

/// How many records with nothing held a [`LivePresence`] remembers before it
/// forgets the oldest (tracked addresses are never forgotten).
pub const DEFAULT_MAX_GONE: usize = 4_096;

/// An address's presence, in the three states (spec §8.1, O5, R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AddressStatus {
    /// At least one instance token held, heard since the last loss.
    Up,
    /// No instance token held, with a complete seed behind the claim.
    Down,
    /// Neither established: see the module doc.
    Unobservable,
}

impl AddressStatus {
    /// The status as a person reads it: `up`, `down`, `unobservable`.
    pub fn as_str(self) -> &'static str {
        match self {
            AddressStatus::Up => "up",
            AddressStatus::Down => "down",
            AddressStatus::Unobservable => "unobservable",
        }
    }
}

impl fmt::Display for AddressStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the projection's answers stand on: the reason an address with
/// nothing held is down, or only unobservable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceBasis {
    /// No seed yet: what is held came from the subscription alone, whose
    /// history has no end mark. Nothing can be down.
    Unseeded,
    /// The last seed ended at its timeout, or with an error reply, and no
    /// complete seed stands: an address it did not show may be up (§8.1).
    Incomplete,
    /// A complete seed stands, and nothing was lost since.
    Complete,
    /// Events were lost (`Dropped`) since the last complete seed: every
    /// address is unobservable until a complete seed restores the basis.
    /// What is held is the last known, kept for display.
    Lost {
        /// Events lost since the last complete seed.
        dropped: u64,
    },
}

/// Keys a [`LivePresence`] was shown and did not fold, by why (O6: one
/// counter per fact, never folded).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PresenceIgnored {
    /// Keys outside the namespace live keys arrive under: another
    /// deployment's, or none.
    pub outside_namespace: u64,
    /// Keys that are not zk2 keys: another application's on the shared bus
    /// (§1.7), or a key under `@zk` that fits none of the five forms.
    pub not_zk2: u64,
    /// zk2 keys that are not presence tokens: a data key, a contract key.
    pub not_presence: u64,
    /// Presence tokens of an address outside the projection's scope.
    pub out_of_scope: u64,
}

impl PresenceIgnored {
    /// Every key ignored, whatever the reason.
    pub fn total(&self) -> u64 {
        self.outside_namespace + self.not_zk2 + self.not_presence + self.out_of_scope
    }
}

/// One member of an interface's `epoch` template (§8.1): the interface and
/// the member's chunk. Its epoch is not part of its identity: a new epoch
/// is the same member, its continuity broken.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId {
    pub iface: IfaceId,
    /// The slugged value of the template's `epoch` parameter, as the key
    /// carries it.
    pub member: String,
}

/// One instance of an address, as its tokens show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveInstance {
    /// Whether its instance token is held. `false` with interface tokens
    /// held is a shape §8.2's order passes through briefly (and a doctor's
    /// to judge if it lasts): shown as found.
    pub instance_token: bool,
    /// Its interface tokens: the fingerprint prefix of each. One per
    /// interface on a conforming instance; a set, so a second is shown
    /// rather than overwritten.
    pub interfaces: BTreeMap<IfaceId, BTreeSet<Fp16>>,
}

impl LiveInstance {
    fn is_empty(&self) -> bool {
        !self.instance_token && self.interfaces.is_empty()
    }
}

/// One address, as the projection holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAddress {
    /// Its status now.
    pub status: AddressStatus,
    /// When its status last moved, or when the projection first held a
    /// record of it.
    pub since: SystemTime,
    /// Whether a consumer asked to track it ([`LivePresence::track`]).
    pub tracked: bool,
    /// Its instances, by id: every instance a token names.
    pub instances: BTreeMap<InstanceId, LiveInstance>,
    /// Its members' epochs, oldest declaration first; the last is current.
    /// A member token's epoch is not an instance id (§1.2), so members
    /// belong to the address: the key does not say which instance holds
    /// one.
    pub members: BTreeMap<MemberId, Vec<InstanceId>>,
}

impl LiveAddress {
    fn new(status: AddressStatus, since: SystemTime) -> LiveAddress {
        LiveAddress {
            status,
            since,
            tracked: false,
            instances: BTreeMap::new(),
            members: BTreeMap::new(),
        }
    }

    /// The instances holding their instance token.
    pub fn instance_tokens(&self) -> impl Iterator<Item = &InstanceId> {
        self.instances
            .iter()
            .filter(|(_, i)| i.instance_token)
            .map(|(id, _)| id)
    }

    /// A member's current epoch: the newest declaration still held.
    pub fn member_epoch(&self, member: &MemberId) -> Option<&InstanceId> {
        self.members.get(member).and_then(|e| e.last())
    }

    /// Whether any token of the address is held.
    pub fn holds_any(&self) -> bool {
        !self.instances.is_empty() || !self.members.is_empty()
    }

    fn holds_instance_token(&self) -> bool {
        self.instances.values().any(|i| i.instance_token)
    }
}

/// How a transition was learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceSource {
    /// Heard on the subscription, as it happened.
    Event,
    /// Found by a seed, against what was held before it: it happened at or
    /// before the time the transition carries.
    Seed,
    /// The basis was lost to a `Dropped(n)`: what follows is unobservable.
    Dropped,
}

/// One move of one address's presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceChange {
    /// The address's status moved, in the three states. `to: Down` is
    /// presence's word, never a verdict about health.
    Status {
        from: AddressStatus,
        to: AddressStatus,
    },
    /// An instance token appeared (§8.1).
    InstanceAppeared { instance: InstanceId },
    /// An instance token went.
    InstanceGone { instance: InstanceId },
    /// An interface token appeared.
    InterfaceAppeared {
        instance: InstanceId,
        iface: IfaceId,
        fp: Fp16,
    },
    /// An interface token went.
    InterfaceGone {
        instance: InstanceId,
        iface: IfaceId,
        fp: Fp16,
    },
    /// A member's first token appeared: the member exists from here (§8.1).
    MemberAppeared { member: MemberId, epoch: InstanceId },
    /// A member's current epoch moved while it held a token throughout:
    /// make-before-break, never a gap (§8.1).
    MemberHandover {
        member: MemberId,
        from: InstanceId,
        to: InstanceId,
    },
    /// A member's last token went.
    MemberGone { member: MemberId, epoch: InstanceId },
}

/// One transition: what moved, where, when it was folded and how it was
/// learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceTransition {
    /// The time the caller folded it with.
    pub at: SystemTime,
    pub source: PresenceSource,
    pub address: Addr,
    pub change: PresenceChange,
}

/// A presence token, its address taken off.
#[derive(Debug, Clone)]
enum Token {
    Instance(InstanceId),
    Alive(InstanceId, IfaceId, Fp16),
    Member(MemberId, InstanceId),
}

/// The live presence projection: see the module doc.
#[derive(Debug, Clone)]
pub struct LivePresence {
    scope: Scope,
    namespace: String,
    basis: PresenceBasis,
    addresses: BTreeMap<Addr, LiveAddress>,
    ignored: PresenceIgnored,
    max_gone: usize,
    forgotten: u64,
}

impl LivePresence {
    /// An empty projection over `scope`: unseeded, every address
    /// unobservable. Live keys are base-relative until
    /// [`LivePresence::wire_namespace`] says otherwise.
    pub fn new(scope: Scope) -> LivePresence {
        LivePresence {
            scope,
            namespace: String::new(),
            basis: PresenceBasis::Unseeded,
            addresses: BTreeMap::new(),
            ignored: PresenceIgnored::default(),
            max_gone: DEFAULT_MAX_GONE,
            forgotten: 0,
        }
    }

    /// The namespace live keys ([`LivePresence::token`]) arrive under: the
    /// deployment's when the subscription runs on a session in no
    /// namespace, empty when it runs in the namespace. A seed is always
    /// base-relative: it is read in the namespace.
    pub fn wire_namespace(mut self, namespace: impl Into<String>) -> LivePresence {
        self.namespace = namespace.into();
        self
    }

    /// How many records with nothing held to keep before forgetting the
    /// oldest ([`DEFAULT_MAX_GONE`]); zero is clamped to one.
    pub fn max_gone(mut self, n: usize) -> LivePresence {
        self.max_gone = n.max(1);
        self
    }

    /// The scope this projection answers for.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// What its answers stand on.
    pub fn basis(&self) -> PresenceBasis {
        self.basis
    }

    /// The keys it was shown and did not fold, by why.
    pub fn ignored(&self) -> PresenceIgnored {
        self.ignored
    }

    /// Records with nothing held forgotten under the bound.
    pub fn forgotten(&self) -> u64 {
        self.forgotten
    }

    /// An address's status, whether or not the projection holds a record of
    /// it: one never heard of is down under a complete seed of a scope that
    /// covers it, and unobservable otherwise.
    pub fn status(&self, address: &Addr) -> AddressStatus {
        match self.addresses.get(address) {
            Some(a) => a.status,
            None => self.unheard(self.basis, address),
        }
    }

    /// The record of one address, when the projection holds one.
    pub fn address(&self, address: &Addr) -> Option<&LiveAddress> {
        self.addresses.get(address)
    }

    /// Every address it holds a record of, in order.
    pub fn addresses(&self) -> impl Iterator<Item = (&Addr, &LiveAddress)> {
        self.addresses.iter()
    }

    /// Keep a record of `address` whatever it holds, and never forget it: a
    /// consumer that watches one address (a dead-man's switch) gets every
    /// move of its status as a transition, the ones a seed or a loss make
    /// included. Tracking moves nothing, so it returns no transition.
    pub fn track(&mut self, address: &Addr, at: SystemTime) {
        let status = self.unheard(self.basis, address);
        self.addresses
            .entry(address.clone())
            .or_insert_with(|| LiveAddress::new(status, at))
            .tracked = true;
    }

    /// Stop tracking `address`; its record is then kept or forgotten like
    /// any other.
    pub fn untrack(&mut self, address: &Addr) {
        if let Some(a) = self.addresses.get_mut(address) {
            a.tracked = false;
        }
        self.bound();
    }

    /// Fold a seed: one liveliness read of the scope (base-relative keys).
    /// Equivalent to [`LivePresence::seed_then`] with nothing heard since.
    pub fn seed(&mut self, seed: &Observed, at: SystemTime) -> Vec<PresenceTransition> {
        self.seed_then(seed, std::iter::empty::<(&str, bool)>(), at)
    }

    /// Fold a seed, then the token events the subscription delivered while
    /// it was read (wire keys, as [`LivePresence::token`] takes them), and
    /// report only the net moves against what was held before.
    ///
    /// A complete seed replaces what is held and restores the basis; one
    /// that is possibly incomplete adds what it read and removes nothing,
    /// since absence from it is not evidence (§8.1). The events after it
    /// are folded in order, which is right whichever side of the read each
    /// one landed on: a token's last event decides it, and a token with
    /// none since the subscription began is the seed's. Folding them here
    /// rather than one by one keeps a delete the seed already reflects from
    /// reading as a token gone and back.
    pub fn seed_then<'k>(
        &mut self,
        seed: &Observed,
        then: impl IntoIterator<Item = (&'k str, bool)>,
        at: SystemTime,
    ) -> Vec<PresenceTransition> {
        let prior = self.basis;
        let mut next: BTreeMap<Addr, LiveAddress> = BTreeMap::new();
        if !seed.complete {
            for (addr, a) in &self.addresses {
                if a.holds_any() {
                    next.insert(addr.clone(), holdings_of(a));
                }
            }
        }
        self.ignored.not_zk2 += seed.unparsed.len() as u64;
        for key in &seed.tokens {
            if let Some((addr, token)) = self.presence_token(key.clone()) {
                apply(entry(&mut next, &addr, at), &token, true);
            }
        }
        for (wire, held) in then {
            if let Some((addr, token)) = self.classify_wire(wire) {
                apply(entry(&mut next, &addr, at), &token, held);
            }
        }
        next.retain(|_, a| a.holds_any());
        self.basis = match (seed.complete, prior) {
            (true, _) => PresenceBasis::Complete,
            (false, PresenceBasis::Lost { .. } | PresenceBasis::Complete) => prior,
            (false, _) => PresenceBasis::Incomplete,
        };

        let mut out = Vec::new();
        let every: BTreeSet<Addr> = self.addresses.keys().chain(next.keys()).cloned().collect();
        for addr in every {
            let unheard = self.unheard(prior, &addr);
            let record = self
                .addresses
                .entry(addr.clone())
                .or_insert_with(|| LiveAddress::new(unheard, at));
            let new = next.remove(&addr);
            let (instances, members) = match new {
                Some(n) => (n.instances, n.members),
                None => (BTreeMap::new(), BTreeMap::new()),
            };
            for change in diff(record, &instances, &members) {
                out.push(PresenceTransition {
                    at,
                    source: PresenceSource::Seed,
                    address: addr.clone(),
                    change,
                });
            }
            record.instances = instances;
            record.members = members;
            self.restatus(&addr, at, PresenceSource::Seed, &mut out);
        }
        self.bound();
        out
    }

    /// Fold one token event: a put (`held`) or a delete of `key`, as the
    /// subscription delivered it (under [`LivePresence::wire_namespace`]).
    /// A key that is not a presence token of the scope is counted and
    /// changes nothing.
    pub fn token(&mut self, key: &str, held: bool, at: SystemTime) -> Vec<PresenceTransition> {
        let Some((addr, token)) = self.classify_wire(key) else {
            return Vec::new();
        };
        let unheard = self.unheard(self.basis, &addr);
        let record = self
            .addresses
            .entry(addr.clone())
            .or_insert_with(|| LiveAddress::new(unheard, at));
        let mut out: Vec<PresenceTransition> = apply(record, &token, held)
            .into_iter()
            .map(|change| PresenceTransition {
                at,
                source: PresenceSource::Event,
                address: addr.clone(),
                change,
            })
            .collect();
        self.restatus(&addr, at, PresenceSource::Event, &mut out);
        self.bound();
        out
    }

    /// Fold a loss: `n` events the subscription did not deliver. Every
    /// address is unobservable from here until a complete seed; what is held
    /// stays, as the last known. `n == 0` lost nothing, and moves nothing.
    pub fn dropped(&mut self, n: u64, at: SystemTime) -> Vec<PresenceTransition> {
        if n == 0 {
            return Vec::new();
        }
        self.basis = match self.basis {
            PresenceBasis::Lost { dropped } => PresenceBasis::Lost {
                dropped: dropped.saturating_add(n),
            },
            _ => PresenceBasis::Lost { dropped: n },
        };
        let mut out = Vec::new();
        let every: Vec<Addr> = self.addresses.keys().cloned().collect();
        for addr in every {
            self.restatus(&addr, at, PresenceSource::Dropped, &mut out);
        }
        out
    }

    /// The status of an address with nothing held, under `basis`.
    fn unheard(&self, basis: PresenceBasis, address: &Addr) -> AddressStatus {
        if basis == PresenceBasis::Complete && self.scope.contains(address) {
            AddressStatus::Down
        } else {
            AddressStatus::Unobservable
        }
    }

    /// Recompute one record's status; a move is pushed as a transition.
    fn restatus(
        &mut self,
        address: &Addr,
        at: SystemTime,
        source: PresenceSource,
        out: &mut Vec<PresenceTransition>,
    ) {
        let basis = self.basis;
        let in_scope = self.scope.contains(address);
        let Some(record) = self.addresses.get_mut(address) else {
            return;
        };
        let now = match basis {
            _ if !in_scope => AddressStatus::Unobservable,
            PresenceBasis::Lost { .. } => AddressStatus::Unobservable,
            _ if record.holds_instance_token() => AddressStatus::Up,
            PresenceBasis::Complete => AddressStatus::Down,
            _ => AddressStatus::Unobservable,
        };
        if now != record.status {
            out.push(PresenceTransition {
                at,
                source,
                address: address.clone(),
                change: PresenceChange::Status {
                    from: record.status,
                    to: now,
                },
            });
            record.status = now;
            record.since = at;
        }
    }

    /// A wire key, stripped of the namespace and classified.
    fn classify_wire(&mut self, wire: &str) -> Option<(Addr, Token)> {
        let Some(relative) = crate::model::namespace::strip(&self.namespace, wire) else {
            self.ignored.outside_namespace += 1;
            return None;
        };
        match zenkey_model::grammar::parse(relative) {
            Ok(key) => self.presence_token(key),
            Err(_) => {
                self.ignored.not_zk2 += 1;
                None
            }
        }
    }

    /// A parsed key as a presence token of the scope, or counted.
    fn presence_token(&mut self, key: ZkKey) -> Option<(Addr, Token)> {
        let (addr, token) = match key {
            ZkKey::Instance { addr, instance } => (addr, Token::Instance(instance)),
            ZkKey::Alive {
                addr,
                iface,
                instance,
                fp,
            } => (addr, Token::Alive(instance, iface, fp)),
            ZkKey::Member {
                addr,
                iface,
                member,
                epoch,
            } => (addr, Token::Member(MemberId { iface, member }, epoch)),
            ZkKey::Data { .. } | ZkKey::Contract { .. } => {
                self.ignored.not_presence += 1;
                return None;
            }
        };
        if !self.scope.contains(&addr) {
            self.ignored.out_of_scope += 1;
            return None;
        }
        Some((addr, token))
    }

    /// Forget the oldest records with nothing held beyond the bound.
    fn bound(&mut self) {
        if self.addresses.len() <= self.max_gone {
            return;
        }
        let mut gone: Vec<(SystemTime, Addr)> = self
            .addresses
            .iter()
            .filter(|(_, a)| !a.tracked && !a.holds_any())
            .map(|(addr, a)| (a.since, addr.clone()))
            .collect();
        if gone.len() <= self.max_gone {
            return;
        }
        // In batches, as `bounded` does: one scan pays for a sixteenth of
        // the bound's worth of later departures.
        let keep = self.max_gone - self.max_gone / 16;
        gone.sort();
        let excess = gone.len() - keep;
        for (_, addr) in gone.into_iter().take(excess) {
            self.addresses.remove(&addr);
            self.forgotten += 1;
        }
    }
}

/// A record holding `a`'s tokens and nothing else.
fn holdings_of(a: &LiveAddress) -> LiveAddress {
    LiveAddress {
        status: a.status,
        since: a.since,
        tracked: false,
        instances: a.instances.clone(),
        members: a.members.clone(),
    }
}

fn entry<'m>(
    map: &'m mut BTreeMap<Addr, LiveAddress>,
    addr: &Addr,
    at: SystemTime,
) -> &'m mut LiveAddress {
    map.entry(addr.clone())
        .or_insert_with(|| LiveAddress::new(AddressStatus::Unobservable, at))
}

/// Fold one token into an address's holdings, returning what moved (the
/// status aside: that is the caller's, from the basis).
fn apply(record: &mut LiveAddress, token: &Token, held: bool) -> Vec<PresenceChange> {
    let mut out = Vec::new();
    match token {
        Token::Instance(instance) => {
            if held {
                let i = record.instances.entry(instance.clone()).or_default();
                if !i.instance_token {
                    i.instance_token = true;
                    out.push(PresenceChange::InstanceAppeared {
                        instance: instance.clone(),
                    });
                }
            } else if let Some(i) = record.instances.get_mut(instance) {
                if i.instance_token {
                    i.instance_token = false;
                    out.push(PresenceChange::InstanceGone {
                        instance: instance.clone(),
                    });
                }
                if i.is_empty() {
                    record.instances.remove(instance);
                }
            }
        }
        Token::Alive(instance, iface, fp) => {
            if held {
                let fresh = record
                    .instances
                    .entry(instance.clone())
                    .or_default()
                    .interfaces
                    .entry(iface.clone())
                    .or_default()
                    .insert(fp.clone());
                if fresh {
                    out.push(PresenceChange::InterfaceAppeared {
                        instance: instance.clone(),
                        iface: iface.clone(),
                        fp: fp.clone(),
                    });
                }
            } else if let Some(i) = record.instances.get_mut(instance) {
                if let Some(fps) = i.interfaces.get_mut(iface) {
                    if fps.remove(fp) {
                        out.push(PresenceChange::InterfaceGone {
                            instance: instance.clone(),
                            iface: iface.clone(),
                            fp: fp.clone(),
                        });
                    }
                    if fps.is_empty() {
                        i.interfaces.remove(iface);
                    }
                }
                if i.is_empty() {
                    record.instances.remove(instance);
                }
            }
        }
        Token::Member(member, epoch) => {
            let before = record.member_epoch(member).cloned();
            if held {
                let epochs = record.members.entry(member.clone()).or_default();
                if !epochs.contains(epoch) {
                    epochs.push(epoch.clone());
                }
            } else if let Some(epochs) = record.members.get_mut(member) {
                epochs.retain(|e| e != epoch);
                if epochs.is_empty() {
                    record.members.remove(member);
                }
            }
            let after = record.member_epoch(member).cloned();
            out.extend(member_change(member, before, after));
        }
    }
    out
}

/// A member's move, from its current epoch before to its current epoch
/// after: nothing, appeared, handed over, or gone.
fn member_change(
    member: &MemberId,
    before: Option<InstanceId>,
    after: Option<InstanceId>,
) -> Option<PresenceChange> {
    match (before, after) {
        (None, Some(epoch)) => Some(PresenceChange::MemberAppeared {
            member: member.clone(),
            epoch,
        }),
        (Some(epoch), None) => Some(PresenceChange::MemberGone {
            member: member.clone(),
            epoch,
        }),
        (Some(from), Some(to)) if from != to => Some(PresenceChange::MemberHandover {
            member: member.clone(),
            from,
            to,
        }),
        _ => None,
    }
}

/// Every move from `old`'s holdings to `instances` and `members`: per
/// instance its interface tokens gone, its instance token, its interface
/// tokens appeared (§8.2's order, and teardown's); then the members.
fn diff(
    old: &LiveAddress,
    instances: &BTreeMap<InstanceId, LiveInstance>,
    members: &BTreeMap<MemberId, Vec<InstanceId>>,
) -> Vec<PresenceChange> {
    let mut out = Vec::new();
    let empty = LiveInstance::default();
    let ids: BTreeSet<&InstanceId> = old.instances.keys().chain(instances.keys()).collect();
    for id in ids {
        let was = old.instances.get(id).unwrap_or(&empty);
        let now = instances.get(id).unwrap_or(&empty);
        let fps = |i: &LiveInstance| -> BTreeSet<(IfaceId, Fp16)> {
            i.interfaces
                .iter()
                .flat_map(|(iface, fps)| fps.iter().map(|fp| (iface.clone(), fp.clone())))
                .collect()
        };
        let (was_fps, now_fps) = (fps(was), fps(now));
        for (iface, fp) in was_fps.difference(&now_fps) {
            out.push(PresenceChange::InterfaceGone {
                instance: id.clone(),
                iface: iface.clone(),
                fp: fp.clone(),
            });
        }
        match (was.instance_token, now.instance_token) {
            (false, true) => out.push(PresenceChange::InstanceAppeared {
                instance: id.clone(),
            }),
            (true, false) => out.push(PresenceChange::InstanceGone {
                instance: id.clone(),
            }),
            _ => {}
        }
        for (iface, fp) in now_fps.difference(&was_fps) {
            out.push(PresenceChange::InterfaceAppeared {
                instance: id.clone(),
                iface: iface.clone(),
                fp: fp.clone(),
            });
        }
    }
    let ids: BTreeSet<&MemberId> = old.members.keys().chain(members.keys()).collect();
    for member in ids {
        let before = old.members.get(member).and_then(|e| e.last()).cloned();
        let after = members.get(member).and_then(|e| e.last()).cloned();
        out.extend(member_change(member, before, after));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use zenkey_model::grammar::{alive_key, contract_key, data_key, instance_key, member_key};

    fn addr(s: &str) -> Addr {
        s.parse().expect("an address")
    }

    fn inst(n: u64) -> InstanceId {
        InstanceId::from_u64(n)
    }

    fn iface(s: &str) -> IfaceId {
        s.parse().expect("an interface")
    }

    fn fp(n: u64) -> Fp16 {
        Fp16::new(&format!("{n:016x}")).expect("a prefix")
    }

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    }

    /// The keys, built through the grammar's own builders.
    fn instance(a: &str, i: u64) -> String {
        instance_key(&addr(a), &inst(i))
            .expect("a key")
            .as_str()
            .to_owned()
    }

    fn alive(a: &str, f: &str, i: u64, p: u64) -> String {
        alive_key(&addr(a), &iface(f), &inst(i), &fp(p))
            .expect("a key")
            .as_str()
            .to_owned()
    }

    fn member(a: &str, f: &str, m: &str, e: u64) -> String {
        member_key(&addr(a), &iface(f), m, &inst(e))
            .expect("a key")
            .as_str()
            .to_owned()
    }

    fn seed(keys: &[String], complete: bool) -> Observed {
        Observed::from_keys(Scope::all().selector(), keys, complete)
    }

    fn statuses(t: &[PresenceTransition]) -> Vec<(String, AddressStatus, AddressStatus)> {
        t.iter()
            .filter_map(|t| match t.change {
                PresenceChange::Status { from, to } => Some((t.address.to_string(), from, to)),
                _ => None,
            })
            .collect()
    }

    use AddressStatus::{Down, Unobservable, Up};

    /// The seed's three poles: present in a complete read is up, absent from
    /// a complete read is down, absent from a read possibly incomplete is
    /// unobservable — and present in one is up, since a token read is held.
    #[test]
    fn a_seed_establishes_each_pole_by_its_completeness() {
        let mut p = LivePresence::new(Scope::all());
        let t = p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        assert_eq!(
            t,
            [
                PresenceTransition {
                    at: at(1),
                    source: PresenceSource::Seed,
                    address: addr("h1/tc"),
                    change: PresenceChange::InstanceAppeared { instance: inst(1) },
                },
                PresenceTransition {
                    at: at(1),
                    source: PresenceSource::Seed,
                    address: addr("h1/tc"),
                    change: PresenceChange::Status {
                        from: Unobservable,
                        to: Up
                    },
                },
            ]
        );
        assert_eq!(p.status(&addr("h1/tc")), Up);
        assert_eq!(p.basis(), PresenceBasis::Complete);

        // Complete and empty: down, for an address never heard of too.
        let mut p = LivePresence::new(Scope::all());
        p.track(&addr("h1/tc"), at(0));
        let t = p.seed(&seed(&[], true), at(1));
        assert_eq!(statuses(&t), [("h1/tc".into(), Unobservable, Down)]);
        assert_eq!(p.status(&addr("h1/tc")), Down);
        assert_eq!(p.status(&addr("h9/never")), Down);

        // Possibly incomplete: what it read is up, what it did not is
        // unobservable.
        let mut p = LivePresence::new(Scope::all());
        p.track(&addr("h2/tc"), at(0));
        let t = p.seed(&seed(&[instance("h1/tc", 1)], false), at(1));
        assert_eq!(statuses(&t), [("h1/tc".into(), Unobservable, Up)]);
        assert_eq!(p.basis(), PresenceBasis::Incomplete);
        assert_eq!(p.status(&addr("h1/tc")), Up);
        assert_eq!(p.status(&addr("h2/tc")), Unobservable);
        assert_eq!(p.status(&addr("h9/never")), Unobservable);
    }

    /// The last instance token gone is down; a re-declare is up again.
    #[test]
    fn the_last_instance_gone_is_down_and_a_redeclare_is_up() {
        let mut p = LivePresence::new(Scope::all());
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        let t = p.token(&instance("h1/tc", 1), false, at(2));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [
                &PresenceChange::InstanceGone { instance: inst(1) },
                &PresenceChange::Status { from: Up, to: Down },
            ],
            "the token first, the status last"
        );
        assert!(t.iter().all(|t| t.source == PresenceSource::Event));
        let a = p.address(&addr("h1/tc")).expect("a record");
        assert_eq!((a.status, a.since), (Down, at(2)));

        let t = p.token(&instance("h1/tc", 2), true, at(3));
        assert_eq!(statuses(&t), [("h1/tc".into(), Down, Up)]);
        // A service that starts after the seed was down before it, not
        // unobservable: the complete seed covered it.
        let t = p.token(&instance("h2/new", 1), true, at(4));
        assert_eq!(statuses(&t), [("h2/new".into(), Down, Up)]);
    }

    /// A `Dropped(n)` makes every address unobservable, up and down alike,
    /// and keeps them so until a complete seed: a token heard meanwhile, or
    /// a seed possibly incomplete, cannot rule out a delete that was lost.
    #[test]
    fn a_loss_is_unobservable_until_a_complete_reseed() {
        let mut p = LivePresence::new(Scope::all());
        p.track(&addr("h2/tc"), at(0));
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        assert_eq!(p.status(&addr("h2/tc")), Down);

        let t = p.dropped(7, at(2));
        assert_eq!(
            statuses(&t),
            [
                ("h1/tc".into(), Up, Unobservable),
                ("h2/tc".into(), Down, Unobservable),
            ]
        );
        assert!(t.iter().all(|t| t.source == PresenceSource::Dropped));
        assert_eq!(p.basis(), PresenceBasis::Lost { dropped: 7 });
        assert_eq!(p.status(&addr("h9/never")), Unobservable);
        // The last known is kept, for display.
        assert_eq!(
            p.address(&addr("h1/tc"))
                .expect("kept")
                .instance_tokens()
                .count(),
            1
        );
        assert!(p.dropped(0, at(2)).is_empty(), "nothing lost moves nothing");

        // Heard meanwhile: folded, still unobservable.
        let t = p.token(&instance("h2/tc", 5), true, at(3));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [&PresenceChange::InstanceAppeared { instance: inst(5) }]
        );
        assert_eq!(p.status(&addr("h2/tc")), Unobservable);

        // A possibly incomplete re-seed keeps the loss.
        p.dropped(3, at(4));
        let t = p.seed(&seed(&[instance("h2/tc", 5)], false), at(5));
        assert!(statuses(&t).is_empty(), "{t:?}");
        assert_eq!(p.basis(), PresenceBasis::Lost { dropped: 10 });

        // A complete one restores it, and a token lost meanwhile goes now,
        // learned from the seed.
        let t = p.seed(&seed(&[instance("h2/tc", 5)], true), at(6));
        assert_eq!(
            t,
            [
                PresenceTransition {
                    at: at(6),
                    source: PresenceSource::Seed,
                    address: addr("h1/tc"),
                    change: PresenceChange::InstanceGone { instance: inst(1) },
                },
                PresenceTransition {
                    at: at(6),
                    source: PresenceSource::Seed,
                    address: addr("h1/tc"),
                    change: PresenceChange::Status {
                        from: Unobservable,
                        to: Down
                    },
                },
                PresenceTransition {
                    at: at(6),
                    source: PresenceSource::Seed,
                    address: addr("h2/tc"),
                    change: PresenceChange::Status {
                        from: Unobservable,
                        to: Up
                    },
                },
            ]
        );
        assert_eq!(p.basis(), PresenceBasis::Complete);
    }

    /// The subscription's history alone is never down: liveliness history
    /// has no end mark, so an address not heard yet, or heard and then
    /// heard going, may still hold a token not delivered yet.
    #[test]
    fn history_alone_is_never_down() {
        let mut p = LivePresence::new(Scope::all());
        p.track(&addr("h2/tc"), at(0));
        let t = p.token(&instance("h1/tc", 1), true, at(1));
        assert_eq!(statuses(&t), [("h1/tc".into(), Unobservable, Up)]);
        assert_eq!(p.basis(), PresenceBasis::Unseeded);
        assert_eq!(p.status(&addr("h2/tc")), Unobservable);
        assert_eq!(p.status(&addr("h9/never")), Unobservable);

        let t = p.token(&instance("h1/tc", 1), false, at(2));
        assert_eq!(statuses(&t), [("h1/tc".into(), Up, Unobservable)]);
        assert_eq!(p.status(&addr("h1/tc")), Unobservable);
    }

    /// Two instances of one address, one leaving: still up. A re-mint
    /// (make-before-break, `new_epoch`) is the same shape, and the address
    /// never reads down between the two.
    #[test]
    fn one_of_two_instances_leaving_stays_up() {
        let mut p = LivePresence::new(Scope::all());
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        let t = p.token(&instance("h1/tc", 2), true, at(2));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [&PresenceChange::InstanceAppeared { instance: inst(2) }]
        );
        let t = p.token(&instance("h1/tc", 1), false, at(3));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [&PresenceChange::InstanceGone { instance: inst(1) }]
        );
        assert_eq!(p.status(&addr("h1/tc")), Up);
        let a = p.address(&addr("h1/tc")).expect("a record");
        assert_eq!(a.since, at(1), "the status never moved");
        assert_eq!(a.instance_tokens().collect::<Vec<_>>(), [&inst(2)]);
    }

    /// Interface tokens are folded per instance; an instance token alone
    /// decides the status, so an interface token without one (§8.2's order
    /// passing) is shown and is not up.
    #[test]
    fn interface_tokens_are_folded_and_do_not_make_an_address_up() {
        let mut p = LivePresence::new(Scope::all());
        p.seed(&seed(&[], true), at(1));
        let t = p.token(&alive("h1/tc", "tc.v1", 1, 0xab), true, at(2));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [&PresenceChange::InterfaceAppeared {
                instance: inst(1),
                iface: iface("tc.v1"),
                fp: fp(0xab),
            }]
        );
        assert_eq!(p.status(&addr("h1/tc")), Down);
        let a = p.address(&addr("h1/tc")).expect("a record");
        assert!(!a.instances[&inst(1)].instance_token);
        let t = p.token(&alive("h1/tc", "tc.v1", 1, 0xab), false, at(3));
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [&PresenceChange::InterfaceGone {
                instance: inst(1),
                iface: iface("tc.v1"),
                fp: fp(0xab),
            }]
        );
        assert!(!p.address(&addr("h1/tc")).expect("kept").holds_any());
    }

    /// A member's newer epoch declared before the older one goes is a
    /// handover, and the older one's delete is no transition: the member
    /// was never without a token. A gap the owner did leave is shown as
    /// one.
    #[test]
    fn a_member_epoch_handover_is_no_gap() {
        let mut p = LivePresence::new(Scope::all());
        p.seed(&seed(&[instance("h1/nav", 1)], true), at(1));
        let a = MemberId {
            iface: iface("nav.v2"),
            member: "a".into(),
        };
        let changes = |t: Vec<PresenceTransition>| -> Vec<PresenceChange> {
            t.into_iter().map(|t| t.change).collect()
        };

        let t = p.token(&member("h1/nav", "nav.v2", "a", 10), true, at(2));
        assert_eq!(
            changes(t),
            [PresenceChange::MemberAppeared {
                member: a.clone(),
                epoch: inst(10),
            }]
        );
        let t = p.token(&member("h1/nav", "nav.v2", "a", 11), true, at(3));
        assert_eq!(
            changes(t),
            [PresenceChange::MemberHandover {
                member: a.clone(),
                from: inst(10),
                to: inst(11),
            }]
        );
        let t = p.token(&member("h1/nav", "nav.v2", "a", 10), false, at(4));
        assert!(t.is_empty(), "the overlap ending is no gap: {t:?}");
        let rec = p.address(&addr("h1/nav")).expect("a record");
        assert_eq!(rec.member_epoch(&a), Some(&inst(11)));

        // Break before make: gone, then appeared.
        let t = p.token(&member("h1/nav", "nav.v2", "a", 11), false, at(5));
        assert_eq!(
            changes(t),
            [PresenceChange::MemberGone {
                member: a.clone(),
                epoch: inst(11),
            }]
        );
        let t = p.token(&member("h1/nav", "nav.v2", "a", 12), true, at(6));
        assert_eq!(
            changes(t),
            [PresenceChange::MemberAppeared {
                member: a.clone(),
                epoch: inst(12),
            }]
        );
        // The newer epoch going first hands back to the older one held.
        p.token(&member("h1/nav", "nav.v2", "a", 13), true, at(7));
        let t = p.token(&member("h1/nav", "nav.v2", "a", 13), false, at(8));
        assert_eq!(
            changes(t),
            [PresenceChange::MemberHandover {
                member: a,
                from: inst(13),
                to: inst(12),
            }]
        );
        assert_eq!(p.status(&addr("h1/nav")), Up, "members move no status");
    }

    /// Keys that are not presence tokens of the scope are counted, each by
    /// why, and change nothing.
    #[test]
    fn keys_that_are_not_presence_are_ignored_and_counted() {
        let mut p =
            LivePresence::new(Scope::system("h1").expect("a system")).wire_namespace("prod");
        p.seed(&seed(&[], true), at(1));
        let data = data_key(
            &addr("h1/tc"),
            &iface("tc.v1"),
            zenkey_model::grammar::KindToken::State,
            &["status"],
        )
        .expect("a key");
        let sha = zenkey_model::grammar::Sha256Hex::new(&"ab".repeat(32)).expect("a sha");
        let contract = contract_key(&iface("tc.v1"), &sha).expect("a key");
        let wire = |k: &str| crate::model::namespace::join("prod", k);
        for (key, held) in [
            ("demo/held/p1/alive".to_owned(), true),
            ("prod/demo/held/p1/alive".to_owned(), true),
            ("prod/zk2/h1/tc/@zk/instance/not-hex".to_owned(), true),
            (wire(data.as_str()), true),
            (wire(contract.as_str()), false),
            (wire(&instance("h2/tc", 1)), true),
            (instance("h1/tc", 1), true),
        ] {
            assert!(p.token(&key, held, at(2)).is_empty(), "{key}");
        }
        assert_eq!(
            p.ignored(),
            PresenceIgnored {
                outside_namespace: 2,
                not_zk2: 2,
                not_presence: 2,
                out_of_scope: 1,
            }
        );
        assert_eq!(p.ignored().total(), 7);
        assert_eq!(p.addresses().count(), 0);
        // Outside the scope is never established, under a complete seed too.
        assert_eq!(p.status(&addr("h2/tc")), Unobservable);
        assert_eq!(p.status(&addr("h1/tc")), Down);
        // In the namespace and the scope, it folds.
        let t = p.token(&wire(&instance("h1/tc", 1)), true, at(3));
        assert_eq!(statuses(&t), [("h1/tc".into(), Down, Up)]);
    }

    /// The events the subscription delivered while the seed was read are
    /// folded into it, and only the net move is reported: a token the seed
    /// shows, deleted and re-declared in the meantime, is no transition.
    #[test]
    fn a_seed_folds_what_was_heard_while_it_was_read() {
        let mut p = LivePresence::new(Scope::all());
        let key = alive("h1/tc", "tc.v1", 1, 0xab);
        let passing = instance("h2/tc", 2);
        let then = [
            (key.as_str(), false),
            (key.as_str(), true),
            (passing.as_str(), true),
            (passing.as_str(), false),
        ];
        let t = p.seed_then(
            &seed(&[instance("h1/tc", 1), key.clone()], true),
            then,
            at(1),
        );
        assert_eq!(
            t.iter().map(|t| &t.change).collect::<Vec<_>>(),
            [
                &PresenceChange::InstanceAppeared { instance: inst(1) },
                &PresenceChange::InterfaceAppeared {
                    instance: inst(1),
                    iface: iface("tc.v1"),
                    fp: fp(0xab),
                },
                &PresenceChange::Status {
                    from: Unobservable,
                    to: Up
                },
            ],
            "h2/tc came and went inside the read: no record, no move"
        );
        assert!(p.address(&addr("h2/tc")).is_none());
        assert_eq!(p.status(&addr("h2/tc")), Down);
    }

    /// A seed possibly incomplete adds what it read and removes nothing,
    /// and a complete one replaces what is held.
    #[test]
    fn an_incomplete_seed_adds_and_a_complete_one_replaces() {
        let mut p = LivePresence::new(Scope::all());
        p.token(&instance("h1/tc", 1), true, at(1));
        let t = p.seed(&seed(&[instance("h2/tc", 2)], false), at(2));
        assert_eq!(statuses(&t), [("h2/tc".into(), Unobservable, Up)]);
        assert_eq!(
            p.status(&addr("h1/tc")),
            Up,
            "absence from it is not evidence"
        );
        let t = p.seed(&seed(&[instance("h2/tc", 2)], true), at(3));
        assert_eq!(statuses(&t), [("h1/tc".into(), Up, Down)]);
        assert_eq!(
            t[0].change,
            PresenceChange::InstanceGone { instance: inst(1) }
        );
    }

    /// Records with nothing held are bounded, the oldest forgotten and
    /// counted; a forgotten address reads as it did, and a tracked one is
    /// never forgotten.
    #[test]
    fn records_with_nothing_held_are_bounded() {
        let mut p = LivePresence::new(Scope::all()).max_gone(16);
        p.track(&addr("h0/kept"), at(0));
        p.seed(&seed(&[], true), at(0));
        for i in 0..40u64 {
            let a = format!("h{i}/tc");
            p.token(&instance(&a, i), true, at(i));
            p.token(&instance(&a, i), false, at(i));
        }
        assert!(p.forgotten() > 0);
        assert_eq!(p.addresses().count() as u64 + p.forgotten(), 41);
        assert!(p.address(&addr("h0/kept")).is_some(), "tracked: kept");
        assert!(p.address(&addr("h1/tc")).is_none(), "the oldest: forgotten");
        assert_eq!(p.status(&addr("h1/tc")), Down, "and still down");
        assert!(p.address(&addr("h39/tc")).is_some(), "the newest: kept");
    }
}
