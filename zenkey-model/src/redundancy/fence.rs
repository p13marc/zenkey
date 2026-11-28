//! The follower's fence (v2 §2.10, §2.17; v1 §2.10), over either major:
//! the holders of a governed interface's token at each address, what each
//! instance's descriptors stated, and the samples it cannot attribute yet,
//! held for one read of the descriptors. Moved from
//! `zenkey_model::arbitration::Incumbents` (#909's BC), which is this
//! type, so that a follower of `redundancy.v2`, arbitration's outcome 6
//! and a tool's view fence alike (#987's N1).
//!
//! ```text
//! hear(holding) / read_ended(descriptors) ─▶ the holders, the lowest the incumbent        v1 §2.3, §2.10
//! describe(instance, major, run, zid)     ─▶ its runs, kept while the address is followed v2 §2.10
//! judge(sample)                           ─▶ accepted │ discarded {no_incumbent, no_run,
//!                                              not_incumbent, unattributed, no_stamp} │ pending
//! offer(sample, member, item)             ─▶ the item decided, or held for one read       v2 §2.10
//! ```
//!
//! A follower of a `redundancy.v1` incumbent reads its samples' Zenoh
//! timestamp ids against its `meta.zid`, by value (v1 §2.10); of a
//! `redundancy.v2` one, or one whose major it does not know, their runs
//! against the runs its descriptors stated (v2 §2.10). It follows its
//! incumbent's revision, and changes as its incumbent does (v2 §2.17).
//!
//! There is no I/O here. The runtime hands the fence each presence event
//! ([`Fence::hear`]), each descriptor it reads or hears put
//! ([`Fence::describe`]), each sample ([`Fence::offer`]), and, when an
//! [`Outcome`] says `asks_read`, makes one read of the address's
//! descriptors and hands its end back ([`Fence::read_ended`]).
//!
//! **A read never undoes a token heard since it was asked** (observer
//! §3.6, 0.38). The read and the watch ride two sessions, which no router
//! orders. So the runtime asks the fence when it asks a presence read
//! ([`Fence::ask`]), and the read carries that [`Asked`] back
//! ([`Holding::Read`]): it applies to no instance whose token the watch
//! heard declared or withdrawn since.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::Major;
use crate::grammar::{Addr, InstanceId};
use crate::wire::{Run, Stamp};

/// A presence event as the fence follows it (v1 §2.10, v2 §2.10): kept
/// through a move of the watch session, as arbitration's
/// [`crate::arbitration::Arbiter`] keeps its own (`arbitration.v1` §2.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holding {
    /// The watch heard an interface token declared.
    Declared(Addr, InstanceId),
    /// The watch heard one withdrawn.
    Withdrawn(Addr, InstanceId),
    /// A read: when it was asked ([`Fence::ask`]), whether it is
    /// complete, and the instances it shows holding the interface's token
    /// at the address.
    Read(Addr, Asked, bool, BTreeSet<InstanceId>),
}

/// When a presence read was asked, among the token events a reader heard
/// ([`Fence::ask`], [`crate::arbitration::Arbiter::ask`]). The read
/// establishes nothing of a token the watch heard since (observer §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Asked(pub(crate) u64);

/// How many instances no longer held a reader remembers hearing, per
/// address, beyond those held: a read asked before the oldest of them
/// ended long since.
const HEARD_KEPT: usize = 64;

/// When the watch last heard each instance's token at one address,
/// declared or withdrawn, numbered among the reader's token events
/// (observer §3.6). Every instance held is kept, and the last
/// [`HEARD_KEPT`] others.
#[derive(Debug, Clone, Default)]
pub(crate) struct Heard(BTreeMap<InstanceId, u64>);

impl Heard {
    /// The watch heard `instance`'s token declared or withdrawn, the
    /// reader's event `n`; `held` the instances held now.
    pub(crate) fn note(&mut self, instance: InstanceId, n: u64, held: &BTreeSet<InstanceId>) {
        self.0.insert(instance, n);
        while self.0.len() > HEARD_KEPT + held.len() {
            let oldest = self
                .0
                .iter()
                .filter(|(i, _)| !held.contains(*i))
                .min_by_key(|(_, n)| **n)
                .map(|(i, _)| i.clone());
            match oldest {
                Some(i) => self.0.remove(&i),
                None => break,
            };
        }
    }

    fn since(&self, instance: &InstanceId, asked: Asked) -> bool {
        self.0.get(instance).is_some_and(|n| *n > asked.0)
    }

    /// A read asked at `asked` into `known` (observer §3.6): a complete
    /// read removes each instance it does not show, one not complete none,
    /// and either adds each it shows. Neither touches an instance whose
    /// token the watch heard since the read was asked: the read
    /// establishes nothing of it.
    pub(crate) fn read_into(
        &self,
        known: &mut BTreeSet<InstanceId>,
        asked: Asked,
        complete: bool,
        held: BTreeSet<InstanceId>,
    ) {
        if complete {
            known.retain(|i| held.contains(i) || self.since(i, asked));
        }
        known.extend(held.into_iter().filter(|i| !self.since(i, asked)));
    }
}

/// What one descriptor of an instance says, as the fence reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Described {
    /// The major of the revision its entry names, `None` where its
    /// contract was not read: then read by run, as `redundancy.v2`'s.
    pub major: Option<Major>,
    /// The run it states ([`super::read_descriptor`]), `None` for none.
    pub run: Option<Run>,
    /// The `meta.zid` it states, which a `redundancy.v1` incumbent is
    /// followed by.
    pub zid: Option<u128>,
}

/// One sample as the fence reads it (v2 §2.10's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// The in-band stamp its form carries: a put's envelope's, a delete's
    /// attachment's, a snapshot's header's for every entry it lists, an
    /// event's key's occurrence chunk's, a call's reply's as-of mark. `None`
    /// for a sample with no envelope, which has no run.
    pub stamp: Option<Stamp>,
    /// Its Zenoh timestamp's id, where it carries one: what a
    /// `redundancy.v1` incumbent is followed by.
    pub timestamp_id: Option<u128>,
}

impl Sample {
    /// Its run, `None` with no envelope.
    #[must_use]
    pub fn run(&self) -> Option<Run> {
        self.stamp.map(|s| s.run)
    }
}

/// What a sample is to the fence while it is held (v2 §2.10): a state
/// member keeps one held value, the one core §4.3's consumer rule would
/// keep; anything else is held alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    /// A state member, by its key: one held value.
    Key(String),
    /// A stream sample, an event, a heartbeat or a reply: each held alone.
    /// A follower MAY drop a stream's instead.
    Alone,
}

/// Why the fence discards a sample (v2 §2.10, v1 §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Discard {
    /// No incumbent seen: what the follower holds is the last incumbent's
    /// last word.
    NoIncumbent,
    /// A `redundancy.v2` incumbent, and a sample with no envelope.
    NoRun,
    /// Another instance's: its run is one another instance's descriptor
    /// stated, or, under a `redundancy.v1` incumbent, its stamp's id is
    /// not the incumbent's `meta.zid`.
    NotIncumbent,
    /// No descriptor the follower holds states its run, once a read asked
    /// after it arrived has ended; or a `redundancy.v1` incumbent whose
    /// `meta.zid` is not held.
    Unattributed,
    /// A `redundancy.v1` incumbent, and a sample with no Zenoh timestamp.
    NoStamp,
}

impl Discard {
    /// The fixture's token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Discard::NoIncumbent => "no_incumbent",
            Discard::NoRun => "no_run",
            Discard::NotIncumbent => "not_incumbent",
            Discard::Unattributed => "unattributed",
            Discard::NoStamp => "no_stamp",
        }
    }
}

impl fmt::Display for Discard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the rule decides of one sample (v2 §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fenced {
    /// The incumbent's.
    Accepted,
    Discarded(Discard),
    /// Unattributed, while no read of the address's descriptors asked
    /// after it arrived has ended: the follower MUST NOT discard it yet.
    Pending,
}

impl Fenced {
    /// The fixture's decision: `accept`, `discard` or `pending`.
    #[must_use]
    pub const fn decision(self) -> &'static str {
        match self {
            Fenced::Accepted => "accept",
            Fenced::Discarded(_) => "discard",
            Fenced::Pending => "pending",
        }
    }

    /// The fixture's reason: `incumbent` for an accepted one,
    /// `unattributed` for a pending one, else the discard's.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Fenced::Accepted => "incumbent",
            Fenced::Discarded(d) => d.as_str(),
            Fenced::Pending => "unattributed",
        }
    }
}

/// What one call to the fence decided, of the item it was given and of
/// those it held (v2 §2.10; `v2/fences.json`'s sequences).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome<T> {
    /// Accepted, in arrival order.
    pub accepted: Vec<T>,
    /// Discarded, in arrival order, each with its reason.
    pub discarded: Vec<(T, Discard)>,
    /// The item [`Fence::offer`] was given is held.
    pub held: bool,
    /// A state member's held value replaced by a newer one, or a new one
    /// not kept beside the held: within one run the higher seq is kept,
    /// across two the later arrival.
    pub superseded: Vec<T>,
    /// Make one read of the address's descriptors, and hand its end to
    /// [`Fence::read_ended`].
    pub asks_read: bool,
}

impl<T> Default for Outcome<T> {
    fn default() -> Self {
        Outcome {
            accepted: Vec::new(),
            discarded: Vec::new(),
            held: false,
            superseded: Vec::new(),
            asks_read: false,
        }
    }
}

impl<T> Outcome<T> {
    fn decide(&mut self, item: T, fenced: Fenced) {
        match fenced {
            Fenced::Accepted => self.accepted.push(item),
            Fenced::Discarded(d) => self.discarded.push((item, d)),
            Fenced::Pending => unreachable!("a pending sample is held, never decided"),
        }
    }
}

/// What the descriptors of one instance stated, as the follower read them:
/// each run, kept for as long as it follows the address (v2 §2.10).
#[derive(Debug, Clone, Default)]
struct Known {
    major: Option<Major>,
    runs: Vec<Run>,
    zid: Option<u128>,
}

#[derive(Debug, Clone)]
struct Held<T> {
    /// When it arrived, among every sample offered.
    arrival: u64,
    member: Member,
    sample: Sample,
    item: T,
}

/// One address as the fence follows it.
#[derive(Debug, Clone)]
struct Followed<T> {
    holders: BTreeSet<InstanceId>,
    /// When the watch last heard each instance's token.
    heard: Heard,
    known: BTreeMap<InstanceId, Known>,
    /// In arrival order.
    held: Vec<Held<T>>,
    /// The outstanding read, asked once the samples before this arrival
    /// had arrived.
    asked: Option<u64>,
}

impl<T> Default for Followed<T> {
    fn default() -> Self {
        Followed {
            holders: BTreeSet::new(),
            heard: Heard::default(),
            known: BTreeMap::new(),
            held: Vec::new(),
            asked: None,
        }
    }
}

impl<T> Followed<T> {
    fn learn(&mut self, instance: InstanceId, d: Described) {
        let k = self.known.entry(instance).or_default();
        if d.major.is_some() {
            k.major = d.major;
        }
        if d.zid.is_some() {
            k.zid = d.zid;
        }
        if let Some(run) = d.run
            && !k.runs.contains(&run)
        {
            k.runs.push(run);
        }
    }

    /// v2 §2.10's rule, the first step that holds: no incumbent; a
    /// `redundancy.v1` incumbent, by `meta.zid`; no run; the incumbent's
    /// run; another instance's; unattributed, discarded once `ended`.
    fn judge(&self, s: &Sample, ended: bool) -> Fenced {
        use Discard::{NoIncumbent, NoRun, NoStamp, NotIncumbent, Unattributed};
        let Some(incumbent) = self.holders.iter().next() else {
            return Fenced::Discarded(NoIncumbent);
        };
        let known = self.known.get(incumbent);
        if let Some(k) = known.filter(|k| k.major == Some(Major::V1)) {
            let Some(id) = s.timestamp_id else {
                return Fenced::Discarded(NoStamp);
            };
            let Some(zid) = k.zid else {
                return Fenced::Discarded(Unattributed);
            };
            return if id == zid {
                Fenced::Accepted
            } else {
                Fenced::Discarded(NotIncumbent)
            };
        }
        let Some(run) = s.run() else {
            return Fenced::Discarded(NoRun);
        };
        if known.is_some_and(|k| k.runs.contains(&run)) {
            return Fenced::Accepted;
        }
        if self
            .known
            .iter()
            .any(|(i, k)| i != incumbent && k.runs.contains(&run))
        {
            return Fenced::Discarded(NotIncumbent);
        }
        if ended {
            Fenced::Discarded(Unattributed)
        } else {
            Fenced::Pending
        }
    }
}

/// The incumbent of each address a follower follows, and what it fences
/// by (v2 §2.10, §2.17; v1 §2.10): the lowest instance holding the
/// interface's token, as the watch and the reads show it, the major, the
/// runs and the `meta.zid` each instance's descriptors stated, and the
/// samples held for one read. `T` is what the runtime holds of a sample;
/// `arbitration::Incumbents` is `Fence<()>`.
///
/// **The hold** (v2 §2.10). A sample whose run no descriptor states is
/// held, never discarded, until a read of the address's descriptors asked
/// after it arrived has ended. One read runs at a time: the fence asks one
/// when it holds a sample and none is outstanding, and asks the next as
/// that one ends while any is still held. A held sample is decided as soon
/// as a descriptor read or heard states its run, and every held one when
/// the read ends, only those that arrived before that read was asked being
/// discarded `unattributed`. A presence event decides none.
///
/// **A read never undoes a token heard since it was asked** (observer
/// §3.6). Each token the watch hears is numbered as it is heard, kept
/// through a move or not, and a read applies to no instance heard since
/// its [`Asked`].
#[derive(Debug, Clone)]
pub struct Fence<T = ()> {
    addrs: BTreeMap<Addr, Followed<T>>,
    kept: Vec<Holding>,
    arrivals: u64,
    /// The token events heard, counted.
    heard: u64,
}

impl<T> Default for Fence<T> {
    fn default() -> Self {
        Fence {
            addrs: BTreeMap::new(),
            kept: Vec::new(),
            arrivals: 0,
            heard: 0,
        }
    }
}

impl<T> Fence<T> {
    /// A presence read is asked now: what it carries back in
    /// [`Holding::Read`], so it applies to no token heard since.
    #[must_use]
    pub fn ask(&self) -> Asked {
        Asked(self.heard)
    }

    /// A presence event: kept while a move is pending (`moving`, the
    /// arbiter's [`crate::arbitration::Arbiter::moving`] before the event),
    /// else applied. A token declared or withdrawn counts as heard now,
    /// kept or not.
    pub fn hear(&mut self, h: Holding, moving: bool) {
        if let Holding::Declared(a, i) | Holding::Withdrawn(a, i) = &h {
            self.heard += 1;
            let f = self.addrs.entry(a.clone()).or_default();
            f.heard.note(i.clone(), self.heard, &f.holders);
        }
        if moving {
            self.kept.push(h);
        } else {
            self.apply(h);
        }
    }

    /// The round trip after a move stands: what was kept, applied in the
    /// order heard (`arbitration.v1` §2.8).
    pub fn stood(&mut self) {
        for h in std::mem::take(&mut self.kept) {
            self.apply(h);
        }
    }

    fn apply(&mut self, h: Holding) {
        match h {
            Holding::Declared(a, i) => {
                self.addrs.entry(a).or_default().holders.insert(i);
            }
            Holding::Withdrawn(a, i) => {
                self.addrs.entry(a).or_default().holders.remove(&i);
            }
            Holding::Read(a, asked, complete, held) => {
                let f = self.addrs.entry(a).or_default();
                f.heard.read_into(&mut f.holders, asked, complete, held);
            }
        }
    }

    /// The incumbent the fence follows at `addr`: the lowest holder (v1
    /// §2.3), `None` with none seen.
    #[must_use]
    pub fn incumbent(&self, addr: &Addr) -> Option<&InstanceId> {
        self.addrs.get(addr)?.holders.iter().next()
    }

    /// A descriptor of `instance` read or heard put (v2 §2.10): its major,
    /// its run, kept beside those read before, and its `meta.zid`. Decides
    /// each held sample whose run it states.
    pub fn describe(&mut self, addr: &Addr, instance: InstanceId, d: Described) -> Outcome<T> {
        let f = self.addrs.entry(addr.clone()).or_default();
        f.learn(instance, d);
        let mut out = Outcome::default();
        let Some(run) = d.run else {
            return out;
        };
        for h in std::mem::take(&mut f.held) {
            if h.sample.run() != Some(run) {
                f.held.push(h);
                continue;
            }
            match f.judge(&h.sample, false) {
                Fenced::Pending => f.held.push(h),
                v => out.decide(h.item, v),
            }
        }
        out
    }

    /// The `meta.zid` a `redundancy.v1` instance's descriptor states (v1
    /// §2.10): [`Fence::describe`] with that major and no run, which
    /// decides no held sample.
    pub fn described(&mut self, addr: Addr, instance: InstanceId, zid: u128) {
        self.describe(
            &addr,
            instance,
            Described {
                major: Some(Major::V1),
                run: None,
                zid: Some(zid),
            },
        );
    }

    /// What v2 §2.10's rule decides of `sample` at `addr`, a read asked
    /// after it arrived having `ended` or not. Holds nothing.
    #[must_use]
    pub fn judge(&self, addr: &Addr, sample: &Sample, ended: bool) -> Fenced {
        match self.addrs.get(addr) {
            Some(f) => f.judge(sample, ended),
            None => Fenced::Discarded(Discard::NoIncumbent),
        }
    }

    /// Whether a sample of `addr` stamped by the clock `id` is a
    /// `redundancy.v1` incumbent's (v1 §2.10): `id` the lowest holder's
    /// `meta.zid`, by value. Nothing is accepted with no incumbent seen, no
    /// `meta.zid` held of it, or no stamp.
    #[must_use]
    pub fn accepts(&self, addr: &Addr, id: Option<u128>) -> bool {
        let Some(f) = self.addrs.get(addr) else {
            return false;
        };
        let Some(lowest) = f.holders.iter().next() else {
            return false;
        };
        let Some(zid) = f.known.get(lowest).and_then(|k| k.zid) else {
            return false;
        };
        id == Some(zid)
    }

    /// The holders of whom no descriptor is held: whose descriptor to get.
    #[must_use]
    pub fn undescribed(&self) -> Vec<(Addr, InstanceId)> {
        self.addrs
            .iter()
            .flat_map(|(a, f)| {
                f.holders
                    .iter()
                    .filter(|i| !f.known.contains_key(*i))
                    .map(move |i| (a.clone(), i.clone()))
            })
            .collect()
    }

    /// Whether a read of `addr`'s descriptors the fence asked for is
    /// outstanding.
    #[must_use]
    pub fn reading(&self, addr: &Addr) -> bool {
        self.addrs.get(addr).is_some_and(|f| f.asked.is_some())
    }

    /// One sample of `addr`, `item` being what the runtime holds of it
    /// (v2 §2.10): decided at once, or held, a state member's one held
    /// value replacing another's by core §4.3's consumer rule. Asks a read
    /// when it holds the item and none is outstanding.
    pub fn offer(&mut self, addr: &Addr, sample: Sample, member: Member, item: T) -> Outcome<T> {
        let arrival = self.arrivals;
        self.arrivals += 1;
        let next = self.arrivals;
        let f = self.addrs.entry(addr.clone()).or_default();
        let mut out = Outcome::default();
        match f.judge(&sample, false) {
            Fenced::Pending => {}
            v => {
                out.decide(item, v);
                return out;
            }
        }
        if let Member::Key(key) = &member
            && let Some(at) = f
                .held
                .iter()
                .position(|h| matches!(&h.member, Member::Key(k) if k == key))
        {
            let newer = match (f.held[at].sample.stamp, sample.stamp) {
                (Some(old), Some(new)) if old.run == new.run => new.seq > old.seq,
                _ => true,
            };
            if !newer {
                out.superseded.push(item);
                return out;
            }
            out.superseded.push(f.held.remove(at).item);
        }
        f.held.push(Held {
            arrival,
            member,
            sample,
            item,
        });
        out.held = true;
        if f.asked.is_none() {
            f.asked = Some(next);
            out.asks_read = true;
        }
        out
    }

    /// The end of the read [`Fence::offer`] or a read's end asked for, and
    /// the descriptors it read (v2 §2.10): each learnt, then every held
    /// sample decided, those that arrived before the read was asked and
    /// are still unattributed discarded so. Asks the next read while any
    /// is still held.
    pub fn read_ended(
        &mut self,
        addr: &Addr,
        descriptors: impl IntoIterator<Item = (InstanceId, Described)>,
    ) -> Outcome<T> {
        let next = self.arrivals;
        let f = self.addrs.entry(addr.clone()).or_default();
        for (i, d) in descriptors {
            f.learn(i, d);
        }
        let asked = f.asked.take();
        let mut out = Outcome::default();
        for h in std::mem::take(&mut f.held) {
            let before = asked.is_some_and(|a| h.arrival < a);
            match f.judge(&h.sample, before) {
                Fenced::Pending => f.held.push(h),
                v => out.decide(h.item, v),
            }
        }
        if !f.held.is_empty() {
            f.asked = Some(next);
            out.asks_read = true;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Seq;

    fn id(n: u64) -> InstanceId {
        format!("{n:016x}").parse().unwrap()
    }

    fn addr() -> Addr {
        "lab/ledger".parse().unwrap()
    }

    fn put(run: u32, seq: u32, zid: Option<u128>) -> Sample {
        Sample {
            stamp: Some(Stamp {
                run: Run::U32(run),
                seq: Seq::new(seq).unwrap(),
            }),
            timestamp_id: zid,
        }
    }

    /// `arbitration::Incumbents`' reading, kept (v1 §2.10): the lowest
    /// holder's `meta.zid`, by value; nothing without one held.
    #[test]
    fn a_v1_incumbent_is_followed_by_its_zid() {
        let mut f: Fence = Fence::default();
        let a = addr();
        f.hear(Holding::Declared(a.clone(), id(0xb)), false);
        f.hear(Holding::Declared(a.clone(), id(0xa)), false);
        assert_eq!(
            f.undescribed(),
            [(a.clone(), id(0xa)), (a.clone(), id(0xb))]
        );
        assert!(!f.accepts(&a, Some(0xa1)), "no meta.zid held");
        f.described(a.clone(), id(0xa), 0xa1);
        f.described(a.clone(), id(0xb), 0xb1);
        assert!(f.undescribed().is_empty());
        assert!(f.accepts(&a, Some(0xa1)));
        assert!(!f.accepts(&a, Some(0xb1)), "a split: the lowest holder's");
        assert!(!f.accepts(&a, None));
        assert_eq!(f.judge(&a, &put(7, 0, Some(0xa1)), false), Fenced::Accepted);
        f.hear(Holding::Withdrawn(a.clone(), id(0xa)), false);
        assert!(f.accepts(&a, Some(0xb1)), "it changes with its incumbent");
    }

    /// What a move keeps is applied when it stands (`arbitration.v1` §2.8).
    #[test]
    fn presence_heard_during_a_move_waits_for_it_to_stand() {
        let mut f: Fence = Fence::default();
        let a = addr();
        f.hear(
            Holding::Read(a.clone(), f.ask(), true, [id(1)].into()),
            true,
        );
        assert_eq!(f.incumbent(&a), None);
        f.stood();
        assert_eq!(f.incumbent(&a), Some(&id(1)));
    }

    /// A read never undoes a token heard since it was asked (observer §3.6,
    /// 0.38; round 128's F-629). A's withdrawal leaves none, and the read
    /// asked after it comes back complete without B, whose token the watch
    /// heard declared meanwhile: B stays the incumbent. A token heard
    /// withdrawn meanwhile is not brought back either. A read asked after
    /// both decides each.
    #[test]
    fn a_read_never_undoes_a_token_heard_since_it_was_asked() {
        let mut f: Fence = Fence::default();
        let a = addr();
        f.hear(Holding::Declared(a.clone(), id(0xa)), false);
        f.hear(Holding::Withdrawn(a.clone(), id(0xa)), false);
        let asked = f.ask();
        f.hear(Holding::Declared(a.clone(), id(0xb)), false);
        f.hear(
            Holding::Read(a.clone(), asked, true, BTreeSet::new()),
            false,
        );
        assert_eq!(
            f.incumbent(&a),
            Some(&id(0xb)),
            "the read says nothing of B"
        );
        let asked = f.ask();
        f.hear(Holding::Withdrawn(a.clone(), id(0xb)), false);
        f.hear(
            Holding::Read(a.clone(), asked, true, [id(0xb)].into()),
            false,
        );
        assert_eq!(f.incumbent(&a), None, "nor brings B back");
        f.hear(Holding::Declared(a.clone(), id(0xc)), false);
        let asked = f.ask();
        f.hear(
            Holding::Read(a.clone(), asked, true, [id(0xd)].into()),
            false,
        );
        assert_eq!(
            f.incumbent(&a),
            Some(&id(0xd)),
            "asked after C: C read gone, D read held"
        );
        // Not complete: it adds, and removes nothing.
        let asked = f.ask();
        f.hear(
            Holding::Read(a.clone(), asked, false, [id(0x9)].into()),
            false,
        );
        assert_eq!(f.incumbent(&a), Some(&id(0x9)));
    }

    /// Through a move (`arbitration.v1` §2.8), in either order kept: a
    /// token heard declared after the read was asked stands.
    #[test]
    fn a_read_kept_through_a_move_never_undoes_a_token_heard_since() {
        for read_first in [true, false] {
            let mut f: Fence = Fence::default();
            let a = addr();
            f.hear(Holding::Declared(a.clone(), id(0xa)), false);
            f.hear(Holding::Withdrawn(a.clone(), id(0xa)), true);
            let asked = f.ask();
            let read = Holding::Read(a.clone(), asked, true, BTreeSet::new());
            if read_first {
                f.hear(read, true);
                f.hear(Holding::Declared(a.clone(), id(0xb)), true);
            } else {
                f.hear(Holding::Declared(a.clone(), id(0xb)), true);
                f.hear(read, true);
            }
            f.stood();
            assert_eq!(f.incumbent(&a), Some(&id(0xb)), "read first: {read_first}");
        }
    }

    /// What the fence remembers hearing is bounded, every holder's kept:
    /// a holder heard since a read was asked stands however many others
    /// came and went meanwhile.
    #[test]
    fn a_holder_heard_since_stands_past_the_bound() {
        let mut f: Fence = Fence::default();
        let a = addr();
        let asked = f.ask();
        f.hear(Holding::Declared(a.clone(), id(0xffff)), false);
        for n in 1..=(3 * HEARD_KEPT as u64) {
            f.hear(Holding::Declared(a.clone(), id(n)), false);
            f.hear(Holding::Withdrawn(a.clone(), id(n)), false);
        }
        assert!(f.addrs[&a].heard.0.len() <= HEARD_KEPT + 1);
        f.hear(
            Holding::Read(a.clone(), asked, true, BTreeSet::new()),
            false,
        );
        assert_eq!(f.incumbent(&a), Some(&id(0xffff)));
    }

    /// A follower reads its incumbent's revision (v2 §2.17): a v1 one by
    /// zid, whatever the run, then a v2 one by run, whatever the stamp.
    #[test]
    fn a_follower_follows_each_incumbent_by_its_own_major() {
        let mut f: Fence<&str> = Fence::default();
        let a = addr();
        f.hear(
            Holding::Read(a.clone(), f.ask(), true, [id(1)].into()),
            false,
        );
        let v1 = Described {
            major: Some(Major::V1),
            run: Some(Run::U32(0x11)),
            zid: Some(0x1),
        };
        let v2 = Described {
            major: Some(Major::V2),
            run: Some(Run::U32(0x22)),
            zid: Some(0x2),
        };
        f.describe(&a, id(1), v1);
        f.describe(&a, id(2), v2);
        let out = f.offer(&a, put(0x22, 0, Some(0x1)), Member::Alone, "v1 stamp");
        assert_eq!(out.accepted, ["v1 stamp"], "the run is not read");
        f.hear(Holding::Declared(a.clone(), id(2)), false);
        f.hear(Holding::Withdrawn(a.clone(), id(1)), false);
        let out = f.offer(&a, put(0x22, 1, Some(0x1)), Member::Alone, "v2 run");
        assert_eq!(out.accepted, ["v2 run"], "the stamp is not read");
        let out = f.offer(&a, put(0x11, 2, None), Member::Alone, "v1 run");
        assert_eq!(out.discarded, [("v1 run", Discard::NotIncumbent)]);
    }

    /// A held sample waits for the read; a sample that is decided at once
    /// asks none (v2 §2.10).
    #[test]
    fn only_a_held_sample_asks_a_read() {
        let mut f: Fence<u8> = Fence::default();
        let a = addr();
        f.hear(
            Holding::Read(a.clone(), f.ask(), true, [id(1)].into()),
            false,
        );
        f.describe(
            &a,
            id(1),
            Described {
                major: Some(Major::V2),
                run: Some(Run::U32(1)),
                zid: None,
            },
        );
        let out = f.offer(&a, put(1, 0, None), Member::Key("balance".into()), 0);
        assert_eq!((out.accepted, out.asks_read), (vec![0], false));
        assert!(!f.reading(&a));
        let out = f.offer(&a, put(9, 0, None), Member::Key("balance".into()), 1);
        assert!(out.held && out.asks_read);
        assert!(f.reading(&a));
        let out = f.read_ended(&a, []);
        assert_eq!(out.discarded, [(1, Discard::Unattributed)]);
        assert!(!out.asks_read && !f.reading(&a));
    }
}
