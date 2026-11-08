//! The window's own `freshness.v2` reading (#990 V6's Inspector half;
//! freshness.v2 §2.5, §5): one [`View`] per owner interface whose contract
//! uses the profile, fed by the live ticks' samples in the order they
//! arrived — each member's put and delete of a collection, each heartbeat
//! of the interface — as the runtime's subscription folds them for the
//! CLI (`Consumer::subscribe_heartbeats`).
//!
//! An owner's view is made at its first sample the book can read: a
//! member the lens resolves to a revision that uses the profile, or a
//! heartbeat, through the revision the lens reads its body with for the
//! interface it covers (`Lens::heartbeat_revision`; #1113), so a quiet
//! collection, whose owner only heartbeats, is read too. A member heard
//! before its owner's view stands (its key not resolved yet: a seed that
//! outran the presence read) is held, bounded, and taken into the view
//! when it is made. A heartbeat no revision in hand reads is counted not
//! read, with the lens's word on why, never dropped.
//!
//! What it says is the reader's word, never a finding about the owner
//! (§5): a heartbeat whose digest is not this window's view **diverged**
//! (the window missed a put, or never heard the members); one after a gap
//! in its key's seqs came after one **lost**; a section in step names the
//! members its owner **vouched** for. A window that subscribed after the
//! members were put holds none of them, so its first heartbeats diverge:
//! said so, not judged.
//!
//! Session-free, and bounded: [`MAX_OWNERS`] interfaces and [`MAX_HELD`]
//! members held before their view, the rest counted.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use zenkey_fleet::SampleView;
use zenkey_model::freshness::{Applied, SectionRead, View};
use zenkey_model::wire::Stamp;

/// How many owner interfaces the book follows; past it a new one is
/// counted in [`Book::refused`], never silently dropped (O6).
pub const MAX_OWNERS: usize = 256;

/// How many members the book holds for owners whose view does not stand
/// yet; past it the earliest held is forgotten, counted in
/// [`Book::forgotten`] (O6).
pub const MAX_HELD: usize = 4_096;

/// What one heartbeat's section did to this window's view of its
/// collection (freshness.v2 §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// Its digest was the view's: the members it vouched for, confirmed.
    Vouched(usize),
    /// Its digest or live count was not the view's: nothing confirmed.
    Diverged,
    /// Its map bit was 0: the collection not available.
    Absent,
    /// A collection the view does not follow.
    NotFollowed,
}

/// The latest heartbeat the view read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatRead {
    /// Its key's run and seq.
    pub stamp: Stamp,
    /// Its key's seqs skipped since the one before: a heartbeat lost.
    pub lost: bool,
    /// Each collection, `<kind token>/<template>`, in the layout's order,
    /// and what its section did.
    pub sections: Vec<(String, Section)>,
}

/// One owner interface's view and what its heartbeats did to it.
#[derive(Debug, Clone, Default)]
pub struct Owner {
    /// `None` while no revision in hand reads its heartbeats.
    view: Option<View<Instant>>,
    /// While no view stands, why its heartbeats are not read: the lens's
    /// word on the covered interface's revision.
    pub waiting: Option<String>,
    /// The latest heartbeat read, `None` before the first.
    pub last: Option<HeartbeatRead>,
    /// Heartbeats read.
    pub heartbeats: u64,
    /// Of them, those read after a gap in their key's seqs.
    pub lost: u64,
    /// Of them, those with a section that diverged from the view.
    pub diverged: u64,
    /// Heartbeats not read: no revision in hand to read them with, refused,
    /// another revision's tag, or not a heartbeat's body.
    pub unread: u64,
}

/// A member heard before its owner's view stood.
#[derive(Debug, Clone)]
struct Held {
    owner: String,
    stamp: Option<Stamp>,
    deleted: bool,
    at: Instant,
    /// Its place in the order the book heard members in.
    order: u64,
}

/// The members held before their owners' views stood (#1120): by key,
/// in the order heard, and by owner, so the earliest goes in O(log n) and
/// an owner's are found without a scan.
#[derive(Debug, Clone, Default)]
struct HeldSet {
    /// By base-relative key.
    by_key: HashMap<String, Held>,
    /// Each held key by its place in the order heard.
    by_order: BTreeMap<u64, String>,
    /// Each owner's held keys.
    by_owner: HashMap<String, BTreeSet<String>>,
}

impl HeldSet {
    fn len(&self) -> usize {
        self.by_key.len()
    }

    fn get(&self, key: &str) -> Option<&Held> {
        self.by_key.get(key)
    }

    fn insert(&mut self, key: String, held: Held) {
        self.remove(&key);
        self.by_order.insert(held.order, key.clone());
        self.by_owner
            .entry(held.owner.clone())
            .or_default()
            .insert(key.clone());
        self.by_key.insert(key, held);
    }

    fn remove(&mut self, key: &str) -> Option<Held> {
        let held = self.by_key.remove(key)?;
        self.by_order.remove(&held.order);
        if let Some(keys) = self.by_owner.get_mut(&held.owner) {
            keys.remove(key);
            if keys.is_empty() {
                self.by_owner.remove(&held.owner);
            }
        }
        Some(held)
    }

    /// The earliest held, taken out.
    fn pop_earliest(&mut self) -> Option<(String, Held)> {
        let key = self.by_order.first_key_value()?.1.clone();
        let held = self.remove(&key)?;
        Some((key, held))
    }

    /// Every member held of `owner`, taken out in the order heard.
    fn take_owner(&mut self, owner: &str) -> Vec<(String, Held)> {
        let keys = self.by_owner.remove(owner).unwrap_or_default();
        let mut taken: Vec<(String, Held)> = keys
            .into_iter()
            .filter_map(|k| {
                let held = self.by_key.remove(&k)?;
                self.by_order.remove(&held.order);
                Some((k, held))
            })
            .collect();
        taken.sort_by_key(|(_, h)| h.order);
        taken
    }
}

/// The window's views, one per owner interface (`<system>/<service>/<iface>`).
#[derive(Debug, Clone, Default)]
pub struct Book {
    owners: BTreeMap<String, Owner>,
    held: HeldSet,
    heard: u64,
    forgotten: u64,
    refused: u64,
}

impl Book {
    /// One live sample, `resolved` its key's resolution through `lens`
    /// where one stands: a member's put or delete into its owner's view
    /// (made at its first member that resolved to a revision that uses the
    /// profile, following every collection; held until then), a heartbeat
    /// applied to the view of the interface it covers (made through the
    /// revision `lens` reads it with, when the owner has none). Anything
    /// else is no business of the book's.
    pub fn observe(
        &mut self,
        sample: &SampleView,
        resolved: Option<&zenkey_fleet::Resolved>,
        lens: &zenkey_fleet::Lens<'_>,
        at: Instant,
    ) {
        if let Some((addr, iface)) = zenkey_model::heartbeat::covered(&sample.key) {
            self.heartbeat(format!("{addr}/{iface}"), sample, lens, at);
            return;
        }
        let Some(key) = zenkey_model::heartbeat::base_relative(&sample.key) else {
            return;
        };
        // A collection is a state's (freshness.v2 §2.2).
        let Ok(zenkey_model::grammar::ZkKey::Data {
            addr,
            iface,
            kind:
                zenkey_model::grammar::KindToken::State
                | zenkey_model::grammar::KindToken::ExplicitState,
            ..
        }) = zenkey_model::grammar::parse(key)
        else {
            return;
        };
        let name = format!("{addr}/{iface}");
        let deleted = sample.kind == zenoh::sample::SampleKind::Delete;
        let view = if self.owners.get(&name).is_some_and(|o| o.view.is_some()) {
            self.owners.get_mut(&name).and_then(|o| o.view.as_mut())
        } else {
            match resolved {
                Some(r) if zenkey_model::freshness::uses_v2(r.revision.contract()) => {
                    self.make_view(&name, r.revision.contract())
                }
                // Its revision uses no freshness.v2: no view will be made,
                // so what was held of it goes (#1120).
                Some(_) => {
                    self.held.take_owner(&name);
                    None
                }
                // Not resolved yet: held for the view its owner's first
                // readable sample makes.
                None => {
                    self.hold(key, name, sample.inband(), deleted, at);
                    None
                }
            }
        };
        let Some(view) = view else {
            return;
        };
        if deleted {
            view.delete(key, sample.inband(), at);
        } else {
            view.put(key, sample.inband(), at);
        }
    }

    /// A heartbeat of the interface `name`: its owner's view made first
    /// where none stands, through the revision `lens` reads it with; where
    /// none reads it, counted not read, with why.
    fn heartbeat(
        &mut self,
        name: String,
        sample: &SampleView,
        lens: &zenkey_fleet::Lens<'_>,
        at: Instant,
    ) {
        if !self.owners.get(&name).is_some_and(|o| o.view.is_some()) {
            let bytes = sample.payload.to_bytes();
            let why = match zenkey_fleet::model::inband::payload(
                &sample.key,
                Some(&sample.encoding),
                &bytes,
            ) {
                zenkey_fleet::model::inband::Payload::Heartbeat { tag, .. } => {
                    match lens.heartbeat_revision(&sample.key, tag) {
                        Some(Ok(revision)) => {
                            let contract = revision.contract();
                            if zenkey_model::heartbeat::Layout::of(contract)
                                .collections
                                .is_empty()
                            {
                                Some("its revision holds no collection".to_owned())
                            } else if self.make_view(&name, contract).is_some() {
                                None
                            } else {
                                // Past the bound: counted in `refused`.
                                return;
                            }
                        }
                        Some(Err(why)) => Some(why.words()),
                        None => Some("no heartbeat key".to_owned()),
                    }
                }
                _ => Some("not a heartbeat's body".to_owned()),
            };
            if let Some(why) = why {
                if !self.owners.contains_key(&name) && self.owners.len() >= MAX_OWNERS {
                    self.refused += 1;
                    return;
                }
                let owner = self.owners.entry(name).or_default();
                owner.unread += 1;
                owner.waiting = Some(why);
                return;
            }
        }
        if let Some(owner) = self.owners.get_mut(&name) {
            owner.heartbeat(sample, at);
        }
    }

    /// Makes `name`'s view of `contract`'s collections, following each,
    /// and takes in what the book held of it, in the order heard; `None`
    /// when the contract has none, or past [`MAX_OWNERS`], when what was
    /// held of it goes too.
    fn make_view(
        &mut self,
        name: &str,
        contract: &zenkey_model::contract::Contract,
    ) -> Option<&mut View<Instant>> {
        let layout = zenkey_model::heartbeat::Layout::of(contract);
        if layout.collections.is_empty() {
            self.held.take_owner(name);
            return None;
        }
        if !self.owners.contains_key(name) && self.owners.len() >= MAX_OWNERS {
            self.refused += 1;
            self.held.take_owner(name);
            return None;
        }
        let n = layout.collections.len();
        let mut view = View::new(layout);
        for i in 0..n {
            view.follow(i);
        }
        for (key, h) in self.held.take_owner(name) {
            if h.deleted {
                view.delete(&key, h.stamp, h.at);
            } else {
                view.put(&key, h.stamp, h.at);
            }
        }
        let owner = self.owners.entry(name.to_owned()).or_default();
        owner.view = Some(view);
        owner.waiting = None;
        owner.view.as_mut()
    }

    /// Holds a member of `owner` heard before its view stood, as the view
    /// would: a put older than the one held, within one run, not taken.
    fn hold(&mut self, key: &str, owner: String, stamp: Option<Stamp>, deleted: bool, at: Instant) {
        if let Some(h) = self.held.get(key)
            && let (Some(held), Some(s)) = (h.stamp, stamp)
            && held.run == s.run
            && s.seq <= held.seq
        {
            return;
        }
        if self.held.get(key).is_none()
            && self.held.len() >= MAX_HELD
            && self.held.pop_earliest().is_some()
        {
            self.forgotten += 1;
        }
        self.heard += 1;
        self.held.insert(
            key.to_owned(),
            Held {
                owner,
                stamp,
                deleted,
                at,
                order: self.heard,
            },
        );
    }

    /// The owner a key belongs to: a heartbeat key's covered interface, or
    /// a data key's own; `None` where the book follows none.
    #[must_use]
    pub fn of_key(&self, key: &str) -> Option<&Owner> {
        if let Some((addr, iface)) = zenkey_model::heartbeat::covered(key) {
            return self.owners.get(&format!("{addr}/{iface}"));
        }
        let rel = zenkey_model::heartbeat::base_relative(key)?;
        match zenkey_model::grammar::parse(rel) {
            Ok(zenkey_model::grammar::ZkKey::Data { addr, iface, .. }) => {
                self.owners.get(&format!("{addr}/{iface}"))
            }
            _ => None,
        }
    }

    /// Owner interfaces past [`MAX_OWNERS`], not followed.
    #[must_use]
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Members held before their view stood, forgotten past [`MAX_HELD`].
    #[must_use]
    pub fn forgotten(&self) -> u64 {
        self.forgotten
    }
}

impl Owner {
    fn heartbeat(&mut self, sample: &SampleView, at: Instant) {
        let bytes = sample.payload.to_bytes();
        let (Some(view), zenkey_fleet::model::inband::Payload::Heartbeat { body, stamp, tag }) = (
            self.view.as_mut(),
            zenkey_fleet::model::inband::payload(&sample.key, Some(&sample.encoding), &bytes),
        ) else {
            self.unread += 1;
            return;
        };
        match view.apply(tag, stamp, body, at) {
            Applied::Read { gap, sections } => {
                self.heartbeats += 1;
                if gap {
                    self.lost += 1;
                }
                let sections: Vec<(String, Section)> = view
                    .layout()
                    .collections
                    .iter()
                    .zip(sections)
                    .map(|(c, s)| {
                        (
                            c.resource(),
                            match s {
                                SectionRead::Confirmed(keys) => Section::Vouched(keys.len()),
                                SectionRead::Diverged => Section::Diverged,
                                SectionRead::Absent => Section::Absent,
                                SectionRead::NotFollowed => Section::NotFollowed,
                            },
                        )
                    })
                    .collect();
                if sections.iter().any(|(_, s)| *s == Section::Diverged) {
                    self.diverged += 1;
                }
                self.last = Some(HeartbeatRead {
                    stamp,
                    lost: gap,
                    sections,
                });
            }
            // An older heartbeat within the run: no news.
            Applied::Older => {}
            Applied::Revision | Applied::Refused(_) => self.unread += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::heartbeat::{Digest, Heartbeat, Layout, Members, Section as Body, Time};
    use zenkey_model::wire::{Envelope, Form, Run, Seq, Tag};

    fn modem() -> zenkey_fleet::ContractSet {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2/zenoh-modem/modem.v3.toml");
        let (set, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        set
    }

    fn view(key: &str, payload: Vec<u8>) -> SampleView {
        SampleView::of(
            &zenoh::sample::SampleBuilder::put(
                zenoh::key_expr::KeyExpr::try_from(key.to_owned()).unwrap(),
                payload,
            )
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .into(),
        )
    }

    fn stamp(run: u16, seq: u32) -> Stamp {
        Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        }
    }

    /// A put of `state/oper` under `stamp(3, seq)`.
    fn oper_put(seq: u32) -> Vec<u8> {
        Envelope {
            form: Form::Mutation,
            tag: Tag::NONE,
            stamp: stamp(3, seq),
            record: b"{}",
        }
        .write()
    }

    /// A heartbeat of modem.v3 at `seq`, its `state/oper` section of
    /// `digest`, vouched; every other collection absent.
    fn modem_heartbeat(layout: &Layout, seq: u32, digest: Digest) -> Vec<u8> {
        let sections = layout
            .collections
            .iter()
            .map(|c| {
                (c.resource() == "state/oper").then_some(Body {
                    digest,
                    members: Members::One { vouched: true },
                })
            })
            .collect();
        Heartbeat {
            time: Time::Unknown,
            sections,
        }
        .envelope(layout, stamp(9, seq))
        .unwrap()
    }

    const OPER: &str = "zk2/site-a/rf0/modem.v3/state/oper";
    const BEAT: &str = "zk2/site-a/rf0/freshness.v2/stream/heartbeat/modem.v3";

    /// #1113: a quiet collection. The window heard `state/oper` before its
    /// key resolved (a seed that outran the presence read); from then on its
    /// owner only heartbeats. The first heartbeat makes the view through the
    /// revision the lens reads it with, the member held taken in, and its
    /// section reads vouched.
    #[test]
    fn a_quiet_collections_owner_reads_vouched_from_its_heartbeats() {
        let set = modem();
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let layout = Layout::of(set.iter().next().expect("modem.v3").contract());
        let mut book = Book::default();
        let at = Instant::now();
        book.observe(&view(OPER, oper_put(5)), None, &lens, at);
        assert!(book.of_key(OPER).is_none(), "held, no view yet");
        let held = Digest::of([(OPER, stamp(3, 5))]);
        for seq in [1, 2] {
            book.observe(
                &view(BEAT, modem_heartbeat(&layout, seq, held)),
                None,
                &lens,
                at,
            );
        }
        let owner = book.of_key(BEAT).expect("made at its first heartbeat");
        assert_eq!(
            (owner.heartbeats, owner.lost, owner.diverged, owner.unread),
            (2, 0, 0, 0)
        );
        assert!(owner.waiting.is_none());
        let last = owner.last.as_ref().expect("read");
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r == "state/oper" && *s == Section::Vouched(1)),
            "{last:?}"
        );
    }

    /// #1113: a heartbeat no revision in hand reads is counted not read,
    /// with the lens's word on why, never dropped; once a revision is in
    /// hand, the next one makes the view and is read.
    #[test]
    fn a_heartbeat_no_revision_reads_is_counted_unread() {
        let set = modem();
        let none = zenkey_fleet::ContractSet::default();
        let blind = zenkey_fleet::Lens::new("", None, &none).offline(&none);
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let layout = Layout::of(set.iter().next().expect("modem.v3").contract());
        let mut book = Book::default();
        let at = Instant::now();
        let empty = Digest::EMPTY;
        book.observe(
            &view(BEAT, modem_heartbeat(&layout, 1, empty)),
            None,
            &blind,
            at,
        );
        let owner = book.of_key(BEAT).expect("counted, not dropped");
        assert_eq!((owner.heartbeats, owner.unread), (0, 1));
        assert!(owner.last.is_none());
        assert!(owner.waiting.is_some(), "the lens's word on why");
        book.observe(
            &view(BEAT, modem_heartbeat(&layout, 2, empty)),
            None,
            &lens,
            at,
        );
        let owner = book.of_key(BEAT).expect("its view");
        assert_eq!((owner.heartbeats, owner.unread), (1, 1));
        assert!(owner.waiting.is_none());
        assert!(owner.last.is_some());
    }

    /// #1120: a member held before its key resolved goes once its owner's
    /// revision resolves to one that uses no freshness.v2 (`radio.v1`, on
    /// freshness.v1), since no view will be made of it.
    #[test]
    fn a_non_v2_owners_held_members_go_once_its_revision_resolves() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2/link/radio.v1.toml");
        let (set, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let oper = "zk2/lab/radio-a/radio.v1/state/oper";
        let resolution = lens.resolve(oper);
        let resolved = resolution.resolved.as_ref().expect("radio.v1's oper");
        assert!(!zenkey_model::freshness::uses_v2(
            resolved.revision.contract()
        ));
        let mut book = Book::default();
        let at = Instant::now();
        book.observe(&view(oper, oper_put(5)), None, &lens, at);
        assert_eq!(book.held.len(), 1, "held while unresolved");
        book.observe(&view(oper, oper_put(6)), Some(resolved), &lens, at);
        assert_eq!(book.held.len(), 0, "gone once its revision says no view");
        assert!(book.of_key(oper).is_none());
    }

    /// #1120: past [`MAX_HELD`] the earliest held goes first, in the order
    /// heard, and a member heard again moves to the back.
    #[test]
    fn the_held_set_forgets_its_earliest_first() {
        let set = modem();
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let key = |i: usize| format!("zk2/site-{i}/rf0/modem.v3/state/oper");
        let mut book = Book::default();
        let at = Instant::now();
        for i in 0..MAX_HELD {
            book.observe(&view(&key(i), oper_put(1)), None, &lens, at);
        }
        assert_eq!((book.held.len(), book.forgotten()), (MAX_HELD, 0));
        // The first heard again, a later seq: now the last in order.
        book.observe(&view(&key(0), oper_put(2)), None, &lens, at);
        assert_eq!((book.held.len(), book.forgotten()), (MAX_HELD, 0));
        // One more: the earliest, now the second heard, goes.
        book.observe(&view(&key(MAX_HELD), oper_put(1)), None, &lens, at);
        assert_eq!((book.held.len(), book.forgotten()), (MAX_HELD, 1));
        assert!(book.held.get(&key(0)).is_some(), "heard again, kept");
        assert!(book.held.get(&key(1)).is_none(), "the earliest, forgotten");
        assert!(book.held.get(&key(MAX_HELD)).is_some());
        let orders: Vec<u64> = book.held.by_order.keys().copied().collect();
        assert!(orders.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(book.held.by_order.len(), MAX_HELD, "one place per key");
    }

    /// #1120: a resolved member of a new owner past [`MAX_OWNERS`] is
    /// refused and counted, makes no view and leaves nothing held.
    #[test]
    fn a_member_past_the_owners_bound_is_refused_and_counted() {
        let set = modem();
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let key = |i: usize| format!("zk2/site-{i}/rf0/modem.v3/state/oper");
        let mut book = Book::default();
        let at = Instant::now();
        for i in 0..=MAX_OWNERS {
            let k = key(i);
            if i == MAX_OWNERS {
                // Held first, as before its key resolved.
                book.observe(&view(&k, oper_put(1)), None, &lens, at);
                assert_eq!(book.held.len(), 1);
            }
            let r = lens.resolve(&k);
            book.observe(&view(&k, oper_put(2)), r.resolved.as_ref(), &lens, at);
        }
        assert_eq!(book.refused(), 1);
        assert!(book.of_key(&key(MAX_OWNERS - 1)).is_some());
        assert!(book.of_key(&key(MAX_OWNERS)).is_none(), "no view");
        assert_eq!(book.held.len(), 0, "nothing held for it");
    }

    /// A member heard, then a heartbeat whose `state/oper` section is in
    /// step (vouched), then one after a gap whose digest the view does not
    /// hold (lost, diverged): the reader's word, counted per heartbeat.
    #[test]
    fn a_heartbeat_is_vouched_lost_or_diverged_by_this_windows_view() {
        let set = modem();
        let rev = set.iter().next().expect("modem.v3");
        let layout = Layout::of(rev.contract());
        let oper = "zk2/site-a/rf0/modem.v3/state/oper";
        let stamp = |seq| Stamp {
            run: Run::U16(3),
            seq: Seq::new(seq).unwrap(),
        };
        let mut book = Book::default();
        let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
        let resolution = lens.resolve(oper);
        let put = Envelope {
            form: Form::Mutation,
            tag: Tag::NONE,
            stamp: stamp(5),
            record: b"{}",
        }
        .write();
        let at = Instant::now();
        book.observe(&view(oper, put), resolution.resolved.as_ref(), &lens, at);

        let heartbeat = |seq: u32, digest: Digest| {
            let sections = layout
                .collections
                .iter()
                .map(|c| {
                    (c.resource() == "state/oper").then_some(Body {
                        digest,
                        members: Members::One { vouched: true },
                    })
                })
                .collect();
            Heartbeat {
                time: Time::Unknown,
                sections,
            }
            .envelope(
                &layout,
                Stamp {
                    run: Run::U16(9),
                    seq: Seq::new(seq).unwrap(),
                },
            )
            .unwrap()
        };
        let key = "zk2/site-a/rf0/freshness.v2/stream/heartbeat/modem.v3";
        let held = Digest::of([(oper, stamp(5))]);
        book.observe(&view(key, heartbeat(1, held)), None, &lens, at);
        let owner = book.of_key(key).expect("its owner's view");
        let last = owner.last.as_ref().expect("read");
        assert!(!last.lost);
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r == "state/oper" && *s == Section::Vouched(1)),
            "{last:?}"
        );
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r != "state/oper" && *s == Section::Absent)
        );

        let missed = Digest::of([(oper, stamp(6))]);
        book.observe(&view(key, heartbeat(3, missed)), None, &lens, at);
        let owner = book.of_key(oper).expect("a member's key finds its owner");
        let last = owner.last.as_ref().expect("read");
        assert!(last.lost, "seq 1 then 3: one lost");
        assert!(
            last.sections
                .iter()
                .any(|(r, s)| r == "state/oper" && *s == Section::Diverged)
        );
        assert_eq!(
            (owner.heartbeats, owner.lost, owner.diverged, owner.unread),
            (2, 1, 1, 0)
        );
    }
}
