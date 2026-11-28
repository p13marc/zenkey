//! `alarms.v2` (#984's S3): the book's half for an owner whose descriptor
//! lists `alarms.v2` (`spec/profiles/alarms/v2.md`, text 0.1).
//!
//! **Values in hand → meaning**, as the book's v1 half: what is folded here
//! is a subscription's deliveries of `…/alarms.v2/state/{alarms,words}/<slot>`
//! ([`SlotSample`]), a collection read of `…/alarms.v2/state/**`
//! ([`SlotRead`]), each owner's table and epoch, and an archive's records.
//! Every verdict is `zenkey_model::alarms::v2`'s:
//!
//! - **a slot** is read by identity, through the model's `Track` (§2.6):
//!   each put against the record held there, `rebound`, `replaced` and
//!   `dropped` among the readings, its record held with the run of the put
//!   that carried it, merged by its in-band stamp (core §4.3);
//! - **a collection read** is the model's `Seed`, entry by entry, one
//!   owner's pages at a time: each live entry a put of its slot under its
//!   stamp, aged from its confirmation (`freshness.v2` §2.6), each
//!   tombstone a delete, the words' snapshot each slot's words;
//! - **"nothing fires"** is one complete snapshot (§2.15, `owner_set`): the
//!   latest read's, its run against the runs the owner's present
//!   descriptors state, and since it was asked no loss and no gap in the
//!   owner's presence;
//! - **an alarm's words** are its owner's table's (`Record::name`), read
//!   from the revisions its descriptor names (the bus reads them), or its
//!   slot's `words/<slot>` where they derive its key (`words_taken`), or its
//!   numbers: an unnamed alarm, answered by its slot, matching no reference;
//! - **confirmation** is the heartbeat's bit or an owner heartbeat's group,
//!   folded into the owner's views beside `alarms.v1`'s.
//!
//! **An alarm another identity displaced** from its slot is kept by its
//! identity ([`Detached`]): `rebound` answers no, `rebound` (§2.14 step 5),
//! and `replaced` or `dropped` holds nothing of it (`not_held` or
//! `not_read`), until its identity takes a slot again or it is forgotten.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use zenkey_model::alarms::v2 as m2;
use zenkey_model::alarms::{
    self as am, AlarmRef, Answer, Judged, Major, Reason, SetJudged, SetVerdict, Severity, State,
};
use zenkey_model::freshness::{self as fm, Last, Observation, StampAge, View, Views};
use zenkey_model::grammar::Addr;
use zenkey_model::seed::Held as Merge;
use zenkey_model::wire::{Run, Stamp, Tag};

use super::{
    AlarmBook, AlarmScope, ArchivedAlarm, FORGET_AFTER, MAX_KEYS, alarms_views, answer_of,
};
use crate::report::{
    AlarmChange, AlarmLatest, AlarmLoss, AlarmNamedBy, AlarmRow, AlarmSeen, AlarmTransition,
    AlarmVia, AlarmWords, LastKnownAlarm,
};

/// An `alarms.v2` revision as a reader reads an owner by it (§2.7, text
/// 0.2, F-613): the layout its heartbeat is read with (core §7.5: the
/// heartbeat's tag names the revision), and the horizon of
/// `state/alarms/{slot}`, the `freshness.ttl_s` its alarms age at. The
/// standard revision's is 900 s; a copy with another horizon is read at its
/// own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Revision {
    pub layout: Arc<zenkey_model::heartbeat::Layout>,
    pub horizon: fm::Horizon,
}

impl V2Revision {
    /// The revision `c` is.
    #[must_use]
    pub fn of(c: &zenkey_model::contract::Contract) -> V2Revision {
        V2Revision {
            layout: Arc::new(zenkey_model::heartbeat::Layout::of(c)),
            horizon: c
                .resource(zenkey_model::grammar::KindToken::State, m2::ALARMS)
                .map_or(fm::Horizon::Undeclared, fm::horizon_of),
        }
    }

    /// The standard revision's, embedded: what a reader holds of every
    /// owner it is told nothing else of.
    #[must_use]
    pub fn standard() -> &'static V2Revision {
        static STANDARD: std::sync::LazyLock<V2Revision> = std::sync::LazyLock::new(|| {
            V2Revision::of(zenkey::alarms::v2::implementation().contract())
        });
        &STANDARD
    }

    /// The one of `revisions` whose alarms age soonest: no reading of it
    /// holds an alarm firing that another of them reads stale. `None` for
    /// none.
    pub(crate) fn soonest(revisions: impl IntoIterator<Item = V2Revision>) -> Option<V2Revision> {
        revisions
            .into_iter()
            .min_by_key(|r| r.horizon.ttl().unwrap_or(std::time::Duration::MAX))
    }
}

/// alarms.v2's layout for its heartbeat: the embedded standard revision's
/// collection with a horizon, `state/alarms/{slot}` (§2.7). Its interface
/// is every revision's.
pub(super) fn layout() -> &'static zenkey_model::heartbeat::Layout {
    &V2Revision::standard().layout
}

/// The index of `state/alarms/{slot}` among a view's collections. `None`
/// for a revision that gives the records no horizon, whose heartbeat
/// confirms none of them.
fn index(view: &View<Instant>) -> Option<usize> {
    view.layout().index_of(zenkey::alarms::v2::ALARMS)
}

/// An owner's view of its `alarms.v2` records, read by `revision`.
pub(super) fn view(revision: &V2Revision) -> View<Instant> {
    let mut v = View::new((*revision.layout).clone());
    v.follow_resource(zenkey::alarms::v2::ALARMS);
    v
}

pub(super) fn view_of(views: &Views<Instant>) -> &View<Instant> {
    views
        .view(&layout().iface)
        .expect("an owner's views hold its alarms.v2 records'")
}

pub(super) fn view_of_mut(views: &mut Views<Instant>) -> &mut View<Instant> {
    views
        .view_mut(&layout().iface)
        .expect("an owner's views hold its alarms.v2 records'")
}

/// The owner a key of `alarms.v2`'s two templates names.
pub(super) fn owner_of(key: &str) -> Option<Addr> {
    let k = m2::slot_of(key).or_else(|| m2::words_slot_of(key))?;
    Addr::new(&k.system, &k.service).ok()
}

/// The owner a reply of a collection read is under: a page's key is its
/// collection's snapshot key, `zk2/<system>/<service>/alarms.v2/state/…`.
fn owner_of_reply(key: &str) -> Option<Addr> {
    let mut c = key.split('/');
    if c.next()? != "zk2" {
        return None;
    }
    let (system, service) = (c.next()?, c.next()?);
    (c.next()? == m2::IFACE).then_some(())?;
    Addr::new(system, service).ok()
}

fn slot_key(owner: &Addr, slot: u8) -> String {
    m2::alarm_key(owner.system.as_str(), owner.service.as_str(), slot)
        .map(|k| k.to_string())
        .unwrap_or_default()
}

// ─── the inputs ─────────────────────────────────────────────────────────────

/// One delivery of an `alarms.v2` key to the reader's subscription: a
/// slot's record, or its words (§2.2, §2.4).
#[derive(Debug, Clone)]
pub struct SlotSample {
    /// The key, base-relative.
    pub key: String,
    /// A delete. No owner sends one (§2.5), and it reads as v1's column.
    pub delete: bool,
    /// Its bytes, the zk2c envelope stripped where it carried one; a
    /// refused envelope's bytes as they came, which are never read.
    pub payload: Vec<u8>,
    /// CH's tag: what a record is read under (core §7.6, §3's 11).
    pub tag: Option<Tag>,
    /// What its envelope said: its in-band stamp, or why it was refused.
    pub envelope: crate::model::inband::Envelope,
    /// Its arrival, on this host's monotonic clock.
    pub at: Instant,
}

/// One collection read of the scope's `alarms.v2/state/**` (§2.12, §2.15;
/// core §4.2), as read: when it was asked, how it ended, and every reply as
/// it came, each owner's pages read by the model's `Seed`.
#[derive(Debug, Clone)]
pub struct SlotRead {
    pub asked_at: Instant,
    /// It ran to its final reply with no error reply.
    pub complete: bool,
    /// Each reply's key and payload, in the order they arrived.
    pub replies: Vec<(String, Vec<u8>)>,
}

/// What a reader holds of the owners' tables and epochs (§4, core §8.6):
/// each owner's table, read from the revisions its present descriptors name
/// (`None` where they name none that holds one, or two that differ), and
/// the deployment's word for an owner with no descriptor in hand: the
/// table of the revisions held offline, and the epoch.
///
/// **And each owner's `alarms.v2` revision** (§2.7, text 0.2, F-613): the
/// one its present descriptors name, which its alarms age at and its
/// heartbeat is read with, `None` where one they name is neither held nor
/// retrieved, whose horizon this reader does not have; across a face, or
/// with no descriptor in hand, the deployment's word, an `alarms.v2`
/// revision held offline (`word_revision`). With neither, the standard
/// revision's.
#[derive(Debug, Clone, Default)]
pub struct V2Words {
    pub tables: BTreeMap<Addr, Option<Vec<m2::Rule>>>,
    pub epochs: BTreeMap<Addr, Option<i64>>,
    pub word: Option<Vec<m2::Rule>>,
    pub default_epoch: Option<i64>,
    pub revisions: BTreeMap<Addr, Option<V2Revision>>,
    pub word_revision: Option<V2Revision>,
}

// ─── the state ──────────────────────────────────────────────────────────────

/// What the latest sample of a slot was.
#[derive(Debug, Clone)]
enum Got {
    Record(m2::Record),
    Deleted,
    /// A put that does not read (§2.4): the refusal's token and words.
    Undecodable(String),
}

#[derive(Debug, Clone)]
struct SlotLatest {
    got: Got,
    inband: Option<Stamp>,
    via: AlarmVia,
    /// A read's entry: its snapshot's run, which attributes it (§2.14).
    snapshot_run: Option<Run>,
}

/// The latest read's entry of a slot.
#[derive(Debug, Clone, Copy)]
struct SlotGot {
    asked_at: Instant,
    /// Its snapshot held every page.
    complete: bool,
    /// A live entry, with when its owner last vouched for it; or a
    /// tombstone.
    live: Option<Option<SystemTime>>,
}

#[derive(Debug, Clone)]
struct WordsHeld {
    merged: Merge,
    words: Result<m2::Words, m2::WordsInvalid>,
}

/// One slot of one owner, as this reader follows it.
#[derive(Debug, Clone)]
struct SlotEntry {
    track: m2::Track,
    merged: Merge,
    latest: Option<SlotLatest>,
    /// The last arrival of anything of it: a read asked after it that did
    /// not answer the slot left it out (§2.14 steps 4 and 5).
    arrived: Instant,
    sub_last: Option<(Instant, bool)>,
    got: Option<SlotGot>,
    words: Option<WordsHeld>,
    /// The last record of the slot that read: what names its alarm once
    /// it holds nothing of it, left out by an establishing read or deleted
    /// (§2.14 step 5's `not_held`).
    last_record: Option<m2::Record>,
    gone_at: Option<Instant>,
    reported: Option<Judged>,
}

/// An alarm's identity, as a displaced one is kept: its alert key, or, for
/// an unnamed one, the slot it was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Ident {
    Key(u64),
    Unnamed(u8),
}

impl Ident {
    fn of(held: &m2::Held, slot: u8) -> Ident {
        held.alert_key.map_or(Ident::Unnamed(slot), Ident::Key)
    }
}

/// An alarm another identity's record displaced from its slot (§2.6).
#[derive(Debug, Clone)]
struct Detached {
    slot: u8,
    held: m2::Held,
    how: m2::Displaced,
    at: Instant,
    reported: Option<Judged>,
}

/// An owner's latest collection read (§2.15).
#[derive(Debug, Clone)]
struct OwnerRead {
    asked_at: Instant,
    complete: bool,
    /// The alarms' snapshot it drew: whether it held every page, and its
    /// run. `None` where it drew no page.
    snapshot: Option<(bool, Run)>,
    /// Every slot its snapshot listed, live or as a tombstone.
    answered: BTreeSet<u8>,
}

/// The book's `alarms.v2` half.
#[derive(Debug, Clone)]
pub(super) struct Slots {
    slots: BTreeMap<(Addr, u8), SlotEntry>,
    detached: BTreeMap<(Addr, Ident), Detached>,
    reads: BTreeMap<Addr, OwnerRead>,
    words: V2Words,
    archived: BTreeMap<(Addr, u8), ArchivedAlarm>,
    refused: u64,
    /// The reader is across a face from the scope's owner (§2.11): only
    /// there does the deployment's word name the revision of an owner none
    /// of whose descriptors it read (§2.7).
    across_face: bool,
}

impl Default for Slots {
    fn default() -> Self {
        Slots {
            slots: BTreeMap::new(),
            detached: BTreeMap::new(),
            reads: BTreeMap::new(),
            words: V2Words {
                default_epoch: Some(zenkey_model::catalog::EPOCH_WITHOUT_CATALOG),
                ..V2Words::default()
            },
            archived: BTreeMap::new(),
            refused: 0,
            across_face: false,
        }
    }
}

impl Slots {
    /// The book's `alarms.v2` half, across a face from its owner or not.
    pub(super) fn across(across_face: bool) -> Slots {
        Slots {
            across_face,
            ..Slots::default()
        }
    }

    /// Whether a read answered for `addr` with a page of its records: what
    /// reads an owner with no descriptor in hand by `alarms.v2`.
    pub(super) fn answered(&self, addr: &Addr) -> bool {
        self.reads.get(addr).is_some_and(|r| r.snapshot.is_some())
            || self.slots.keys().any(|(a, _)| a == addr)
            || self.archived.keys().any(|(a, _)| a == addr)
    }

    /// Slots refused at [`MAX_KEYS`].
    pub(super) fn refused(&self) -> u64 {
        self.refused
    }

    /// The table this reader holds of `addr` (§4): the one its present, or
    /// its last, descriptors name; the deployment's word for an owner none
    /// of whose descriptors reached this reader. Never another revision's.
    fn table(&self, addr: &Addr) -> Option<&[m2::Rule]> {
        match self.words.tables.get(addr) {
            Some(t) => t.as_deref(),
            None => self.words.word.as_deref(),
        }
    }

    /// The `alarms.v2` revision this reader reads `addr` by (§2.7): the one
    /// its present, or its last, descriptors name, `None` where one they
    /// name is not in hand. For an owner none of whose descriptors this
    /// reader read: across a face, the deployment's word, the revision
    /// held offline else the standard one; on the near side `None`, a
    /// revision not in hand (text 0.2, "No descriptor read").
    pub(super) fn revision(&self, addr: &Addr) -> Option<&V2Revision> {
        match self.words.revisions.get(addr) {
            Some(named) => named.as_ref(),
            None if self.across_face => Some(
                self.words
                    .word_revision
                    .as_ref()
                    .unwrap_or_else(|| V2Revision::standard()),
            ),
            None => None,
        }
    }

    /// The revision `addr`'s heartbeat is read with: [`Slots::revision`],
    /// the standard one's layout where that is not in hand, under which a
    /// heartbeat of another revision is refused by its tag (core §7.5).
    pub(super) fn layout_of(&self, addr: &Addr) -> &V2Revision {
        self.revision(addr)
            .unwrap_or_else(|| V2Revision::standard())
    }

    fn epoch(&self, addr: &Addr) -> Option<i64> {
        self.words
            .epochs
            .get(addr)
            .copied()
            .unwrap_or(self.words.default_epoch)
    }

    /// An archive's last-known record of a slot in scope (core §4.4, S6):
    /// held, never current. `false` for a key of no `alarms.v2` slot.
    pub(super) fn archived(&mut self, scope: &AlarmScope, a: &ArchivedAlarm) -> bool {
        let Some(k) = m2::slot_of(&a.key) else {
            return false;
        };
        let Ok(owner) = Addr::new(&k.system, &k.service) else {
            return true;
        };
        if !scope.selects_slot(&owner, k.slot) {
            return true;
        }
        let at = (owner, k.slot);
        if self
            .archived
            .get(&at)
            .is_none_or(|have| a.stamp > have.stamp)
        {
            self.archived.insert(at, a.clone());
        }
        true
    }
}

/// A record's words as a report says them, with the reference they give,
/// from the table held and the slot's words (§2.4, §5).
fn naming(
    owner: &Addr,
    record: &m2::Record,
    table: Option<&[m2::Rule]>,
    words: Option<&Result<m2::Words, m2::WordsInvalid>>,
) -> (Option<AlarmRef>, AlarmWords) {
    let reference = |key: u64| AlarmRef {
        system: owner.system.as_str().to_owned(),
        service: owner.service.as_str().to_owned(),
        alert_key: am::key_hex(key),
    };
    let empty = |by| AlarmWords {
        by,
        rule: None,
        index: None,
        labels: BTreeMap::new(),
        codes: None,
        summary: None,
        category: None,
        argument: None,
        reason: None,
    };
    match record.name(table) {
        m2::Naming::Named(n) => (
            Some(reference(n.alert_key)),
            AlarmWords {
                rule: Some(n.rule.name.clone()),
                index: Some(n.index),
                labels: n.labels.iter().cloned().collect(),
                summary: Some(n.summary.clone()),
                category: n.rule.category.clone(),
                argument: n.argument.as_ref().map(ToString::to_string),
                ..empty(AlarmNamedBy::Table)
            },
        ),
        m2::Naming::ByWords { alert_key } => {
            let ok: Result<m2::Record, ()> = Ok(*record);
            match m2::words_taken(Some(&ok), words) {
                Ok(_) => {
                    let w = words
                        .and_then(|w| w.as_ref().ok())
                        .expect("words taken are words read");
                    (
                        Some(reference(alert_key)),
                        AlarmWords {
                            rule: Some(w.rule.clone()),
                            labels: w.labels.clone(),
                            summary: Some(w.summary.clone()),
                            category: w.category.clone(),
                            ..empty(AlarmNamedBy::Words)
                        },
                    )
                }
                Err(why) => (
                    Some(reference(alert_key)),
                    AlarmWords {
                        reason: Some(why.as_str().to_owned()),
                        ..empty(AlarmNamedBy::Key)
                    },
                ),
            }
        }
        m2::Naming::Unnamed(why) => (
            None,
            AlarmWords {
                index: match record.rule {
                    m2::RuleIndex::Table(i) => Some(i),
                    m2::RuleIndex::NotAvailable => None,
                },
                codes: record.labels,
                reason: Some(why.as_str().to_owned()),
                ..empty(AlarmNamedBy::Numbers)
            },
        ),
    }
}

/// §2.6's transition, as the report spells it.
fn change_of(t: am::Transition) -> AlarmChange {
    super::change_of(t)
}

/// One slot's or one displaced alarm's reading at an instant, owned: what
/// the model's judges take, and what a row shows.
#[derive(Debug, Clone)]
struct SlotReading {
    reading: m2::Reading,
    severity: Option<Severity>,
}

impl SlotReading {
    fn firing(&self) -> Judged {
        m2::firing(&self.reading)
    }

    fn stale(&self) -> Judged {
        m2::stale_firing(&self.reading)
    }
}

impl AlarmBook {
    // ─── folding ────────────────────────────────────────────────────────

    /// The owners' tables, epochs and revisions, as this reading holds them
    /// (§2.7, §4, core §8.6): each owner's replaces what was held of it; an
    /// owner the read does not name keeps its last, so a record of an owner
    /// gone is named by the table its last descriptor named.
    ///
    /// An owner's view read by another revision than the one it is read by
    /// now is replaced by an empty one of that revision, whose heartbeat it
    /// reads (core §7.5): the next complete read re-reads it, as a refresh
    /// does right after these words.
    pub fn v2_words(&mut self, words: V2Words) {
        let held = &mut self.v2.words;
        held.tables.extend(words.tables);
        held.epochs.extend(words.epochs);
        held.word = words.word;
        held.default_epoch = words.default_epoch;
        held.revisions.extend(words.revisions);
        held.word_revision = words.word_revision;
        for (owner, views) in &mut self.views {
            let revision = self.v2.layout_of(owner);
            if view_of(views).layout() != &*revision.layout {
                views.insert(view(revision));
            }
        }
    }

    /// One delivery of an `alarms.v2` key to the subscription (§2.6):
    /// a slot's record read against the record held there, or its words.
    /// Returns the transitions it made: a `rebound` alarm resolved first,
    /// then the slot's own.
    pub fn sample_v2(&mut self, s: SlotSample, wall: SystemTime) -> Vec<AlarmTransition> {
        let inband = match s.envelope {
            Some(Ok(stamp)) => Some(stamp),
            _ => None,
        };
        let refused = matches!(s.envelope, Some(Err(_)));
        if let Some(k) = m2::words_slot_of(&s.key) {
            let Ok(owner) = Addr::new(&k.system, &k.service) else {
                return Vec::new();
            };
            if !self.scope.selects_slot(&owner, k.slot) || s.delete {
                return Vec::new();
            }
            let words = if refused {
                Err(m2::WordsInvalid::Json("its envelope is refused".into()))
            } else {
                m2::Words::decode(&s.payload)
            };
            self.fold_words(&owner, k.slot, words, inband, s.at, AlarmVia::Subscription);
            return Vec::new();
        }
        let Some(k) = m2::slot_of(&s.key) else {
            return Vec::new();
        };
        let Ok(owner) = Addr::new(&k.system, &k.service) else {
            return Vec::new();
        };
        if !self.scope.selects_slot(&owner, k.slot) {
            return Vec::new();
        }
        let got = match (&s.envelope, s.delete) {
            (_, true) => Got::Deleted,
            (Some(Err(r)), _) => Got::Undecodable(format!("envelope: {}", r.reason())),
            _ => match (s.tag, self.v2.epoch(&owner)) {
                (None, _) => Got::Undecodable("tag: a record is put in the envelope".into()),
                (_, None) => Got::Undecodable(
                    "epoch: no deployment epoch can be had for its owner (core §8.6)".into(),
                ),
                (Some(tag), Some(epoch)) => match m2::Record::read(tag.get(), &s.payload, epoch) {
                    Ok(r) => Got::Record(r),
                    Err(e) => Got::Undecodable(format!("{}: {e}", e.token())),
                },
            },
        };
        self.fold_slot(
            &owner,
            k.slot,
            got,
            inband,
            refused,
            AlarmVia::Subscription,
            None,
            s.at,
            wall,
        )
    }

    fn fold_words(
        &mut self,
        owner: &Addr,
        slot: u8,
        words: Result<m2::Words, m2::WordsInvalid>,
        inband: Option<Stamp>,
        at: Instant,
        _via: AlarmVia,
    ) {
        let Some(entry) = self.slot_entry(owner, slot, at) else {
            return;
        };
        let incoming = crate::model::inband::mark_of(inband.map(Ok), None);
        let held = entry.words.as_ref().map_or(Merge::Nothing, |w| w.merged);
        if !zenkey_model::seed::admits_mark(held, incoming) {
            return;
        }
        entry.words = Some(WordsHeld {
            merged: held.after_mark(incoming),
            words,
        });
    }

    fn slot_entry(&mut self, owner: &Addr, slot: u8, at: Instant) -> Option<&mut SlotEntry> {
        let id = (owner.clone(), slot);
        if !self.v2.slots.contains_key(&id) && self.v2.slots.len() + self.keys.len() >= MAX_KEYS {
            self.v2.refused += 1;
            return None;
        }
        self.owner(owner)
            .known
            .insert(crate::report::KnownBy::Earlier);
        let e = self.v2.slots.entry(id).or_insert_with(|| SlotEntry {
            track: m2::Track::default(),
            merged: Merge::Nothing,
            latest: None,
            arrived: at,
            sub_last: None,
            got: None,
            words: None,
            last_record: None,
            gone_at: None,
            reported: None,
        });
        e.arrived = e.arrived.max(at);
        Some(e)
    }

    #[allow(clippy::too_many_arguments)]
    fn fold_slot(
        &mut self,
        owner: &Addr,
        slot: u8,
        got: Got,
        inband: Option<Stamp>,
        refused: bool,
        via: AlarmVia,
        snapshot_run: Option<Run>,
        at: Instant,
        wall: SystemTime,
    ) -> Vec<AlarmTransition> {
        let table = self.v2.table(owner).map(<[m2::Rule]>::to_vec);
        let key = slot_key(owner, slot);
        let Some(entry) = self.slot_entry(owner, slot, at) else {
            return Vec::new();
        };
        let incoming = crate::model::inband::mark_of(inband.map(Ok), None);
        if !refused && !zenkey_model::seed::admits_mark(entry.merged, incoming) {
            return Vec::new();
        }
        if via == AlarmVia::Subscription {
            entry.sub_last = Some((at, matches!(got, Got::Deleted)));
        }
        if !refused {
            entry.merged = entry.merged.after_mark(incoming);
        }
        let sample = match &got {
            Got::Deleted => m2::Sample::Delete,
            Got::Undecodable(_) => m2::Sample::Unreadable,
            Got::Record(r) => m2::Sample::Put(m2::Held {
                record: *r,
                run: inband.map_or(snapshot_run.unwrap_or(Run::U16(0)), |s| s.run),
                alert_key: r.name(table.as_deref()).alert_key(),
            }),
        };
        let applied = entry.track.apply(sample);
        entry.gone_at = matches!(got, Got::Deleted).then_some(at);
        if let Got::Record(r) = &got {
            entry.last_record = Some(*r);
        }
        entry.latest = Some(SlotLatest {
            got,
            inband,
            via,
            snapshot_run,
        });
        let new = entry.track.held;
        if !refused {
            let views = self
                .views
                .entry(owner.clone())
                .or_insert_with(|| alarms_views(&self.v2, owner));
            let view = view_of_mut(views);
            if matches!(sample, m2::Sample::Delete) {
                view.delete(&key, inband, at);
            } else {
                view.put(&key, inband, at);
            }
        }
        // The identity now on the slot is held there, not displaced.
        if let Some(h) = new {
            self.v2
                .detached
                .remove(&(owner.clone(), Ident::of(&h, slot)));
        }
        let mut out = Vec::new();
        if let Some((how, h)) = applied.displaced {
            let id = (owner.clone(), Ident::of(&h, slot));
            self.v2.detached.insert(
                id.clone(),
                Detached {
                    slot,
                    held: h,
                    how,
                    at,
                    reported: None,
                },
            );
            out.push(self.detached_transition(&id, AlarmChange::Resolved, Some(via), at, wall));
        }
        out.push(self.slot_transition(
            owner,
            slot,
            change_of(applied.reads),
            Some(via),
            None,
            at,
            wall,
        ));
        out
    }

    /// One collection read of the scope's `alarms.v2/state/**` folded whole
    /// (§2.15, core §4.2): each owner's pages read by the model's `Seed`
    /// under its epoch, each live entry its slot's put, each tombstone a
    /// delete, the words' entries each slot's words; every owner in scope
    /// it drew nothing for read as such (no page: `snapshot_incomplete`);
    /// a complete snapshot re-reads the owner's view (`freshness.v2` §2.5).
    pub fn seed_v2(
        &mut self,
        read: SlotRead,
        now: Instant,
        wall: SystemTime,
    ) -> Vec<AlarmTransition> {
        let mut by_owner: BTreeMap<Addr, Vec<(String, Vec<u8>)>> = BTreeMap::new();
        for (key, payload) in read.replies {
            let Some(owner) = owner_of_reply(&key) else {
                continue;
            };
            if self.scope.selects_owner(&owner) {
                by_owner.entry(owner).or_default().push((key, payload));
            }
        }
        for owner in self.owners.keys() {
            by_owner.entry(owner.clone()).or_default();
        }
        let mut out = Vec::new();
        for (owner, replies) in by_owner {
            let seed = match self.v2.epoch(&owner) {
                Some(epoch) => m2::Seed::of(
                    replies.iter().map(|(k, p)| (k.as_str(), p.as_slice())),
                    epoch,
                ),
                // No epoch can be had for it: its pages are not read, and
                // its alarms are not established.
                None => m2::Seed::default(),
            };
            let epoch = self.v2.epoch(&owner).unwrap_or_default();
            let mut answered = BTreeSet::new();
            let mut live_keys = Vec::new();
            if let Some(snap) = &seed.alarms {
                let complete = snap.heard.complete && !seed.mixed;
                for e in &snap.live {
                    let Some(slot) = e.slot else { continue };
                    if !self.scope.selects_slot(&owner, slot) {
                        continue;
                    }
                    answered.insert(slot);
                    live_keys.push((slot_key(&owner, slot), Some(e.stamp)));
                    let confirmed = e.confirmed_at.and_then(|t| {
                        let ms = u64::try_from(t.unix_ms(epoch)?).ok()?;
                        Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
                    });
                    let got = match &e.record {
                        Ok(r) => Got::Record(*r),
                        Err(u) => Got::Undecodable(format!("{}: {u}", u.token())),
                    };
                    if let Some(entry) = self.slot_entry(&owner, slot, read.asked_at) {
                        entry.got = Some(SlotGot {
                            asked_at: read.asked_at,
                            complete,
                            live: Some(confirmed),
                        });
                    }
                    out.extend(self.fold_slot(
                        &owner,
                        slot,
                        got,
                        Some(e.stamp),
                        false,
                        AlarmVia::Get,
                        Some(snap.heard.run),
                        read.asked_at,
                        wall,
                    ));
                }
                for (id, stamp) in &snap.retired {
                    let Some(slot) = m2::parse_slot(id) else {
                        continue;
                    };
                    if !self.scope.selects_slot(&owner, slot) {
                        continue;
                    }
                    answered.insert(slot);
                    if let Some(entry) = self.slot_entry(&owner, slot, read.asked_at) {
                        entry.got = Some(SlotGot {
                            asked_at: read.asked_at,
                            complete,
                            live: None,
                        });
                    }
                    out.extend(self.fold_slot(
                        &owner,
                        slot,
                        Got::Deleted,
                        Some(*stamp),
                        false,
                        AlarmVia::Get,
                        Some(snap.heard.run),
                        read.asked_at,
                        wall,
                    ));
                }
                if read.complete && complete {
                    let views = self
                        .views
                        .entry(owner.clone())
                        .or_insert_with(|| alarms_views(&self.v2, &owner));
                    let view = view_of_mut(views);
                    if let Some(i) = index(view) {
                        view.reread(
                            i,
                            live_keys.iter().map(|(k, s)| (k.as_str(), *s)),
                            read.asked_at,
                            now,
                        );
                    }
                }
            }
            if let Some(words) = &seed.words {
                for e in &words.live {
                    let Some(slot) = e.slot else { continue };
                    if self.scope.selects_slot(&owner, slot) {
                        self.fold_words(
                            &owner,
                            slot,
                            e.record.clone(),
                            Some(e.stamp),
                            read.asked_at,
                            AlarmVia::Get,
                        );
                    }
                }
            }
            self.v2.reads.insert(
                owner,
                OwnerRead {
                    asked_at: read.asked_at,
                    complete: read.complete,
                    snapshot: seed
                        .alarms
                        .as_ref()
                        .map(|s| (s.heard.complete && !seed.mixed, s.heard.run)),
                    answered,
                },
            );
        }
        out
    }

    // ─── reading ────────────────────────────────────────────────────────

    /// The runs `addr`'s present descriptors state (core §3.3).
    fn stated(&self, addr: &Addr) -> Vec<Run> {
        self.owners
            .get(addr)
            .into_iter()
            .flat_map(|o| o.instances.values())
            .filter_map(|s| s.as_ref()?.stated)
            .collect()
    }

    /// Whether `addr`'s `alarms.v2` alarms are known whole (§2.15): the
    /// model's `owner_set` over the presence as v2's reader asks it, its
    /// latest read, and, where a subscription listens, whether a loss or a
    /// gap in its presence came since that read was asked.
    pub(super) fn owner_set_v2(&self, addr: &Addr) -> SetJudged {
        let presence = self.presence_for(addr, Major::V2);
        let r = self.v2.reads.get(addr);
        let read = r.map(|r| {
            if r.complete {
                m2::Read::Complete(r.snapshot.map(|(complete, run)| m2::SnapshotRead {
                    complete,
                    run: m2::SnapshotRun::of(run, &self.stated(addr)),
                }))
            } else {
                m2::Read::Incomplete
            }
        });
        let since = match (self.declared, r) {
            (Some(_), Some(r)) => Some(m2::Since {
                lossless: self.loss.is_none_or(|l| l.at < r.asked_at),
                present_throughout: self
                    .owners
                    .get(addr)
                    .and_then(|o| o.gap)
                    .is_none_or(|g| g.at < r.asked_at),
            }),
            _ => None,
        };
        m2::owner_set(presence, read, since)
    }

    /// Whether the latest read of `addr` left `slot` out: it ran to its
    /// final reply and drew a complete snapshot listing the slot neither
    /// live nor as a tombstone, and nothing of the slot arrived after it
    /// was asked (§2.14 steps 4 and 5).
    fn left_out_v2(&self, addr: &Addr, slot: u8, e: &SlotEntry) -> bool {
        self.v2.reads.get(addr).is_some_and(|r| {
            r.complete
                && r.snapshot.is_some_and(|(c, _)| c)
                && !r.answered.contains(&slot)
                && e.arrived < r.asked_at
        })
    }

    fn slot_reading(
        &self,
        addr: &Addr,
        slot: u8,
        now: Instant,
        wall: SystemTime,
    ) -> Option<SlotReading> {
        let e = self.v2.slots.get(&(addr.clone(), slot))?;
        let key = slot_key(addr, slot);
        let presence = self.presence_for(addr, Major::V2);
        let mut observations = Vec::new();
        let healed_by = |asked: Instant| self.loss.is_none_or(|l| asked > l.at);
        if let Some(declared) = self.declared {
            let views = self.views.get(addr);
            let heartbeat = views
                .and_then(|vs| view_of(vs).confirmed(&key))
                .map(|c| now.saturating_duration_since(c));
            let lost = views.is_some_and(|vs| {
                index(view_of(vs)).is_some_and(|i| vs.is_lost(&layout().iface, i))
            });
            observations.push(Observation::Subscribed {
                last: fm::confirmed_last(
                    e.sub_last.map(|(t, delete)| {
                        let age = now.saturating_duration_since(t);
                        if delete {
                            Last::Delete(age)
                        } else {
                            Last::Put(age)
                        }
                    }),
                    heartbeat,
                ),
                listened: now.saturating_duration_since(declared),
                complete: self.loss.is_none() && !lost,
            });
        }
        if let Some(g) = e.got.filter(|g| healed_by(g.asked_at)) {
            let clock = self
                .trust()
                .of(&crate::bus::query::owner_clock(&key).to_string());
            let entry = match g.live {
                Some(confirmed) => fm::Entry::Live {
                    stamp_age: confirmed.map(|t| StampAge::between(t, wall)),
                },
                None => fm::Entry::Tombstone,
            };
            observations.push(Observation::of_entry(g.complete, entry, clock));
        }
        let lossy = observations.iter().any(|o| {
            matches!(
                o,
                Observation::Subscribed {
                    complete: false,
                    ..
                }
            )
        });
        let answered = observations
            .iter()
            .any(|o| matches!(o, Observation::Got { reply: Some(_), .. }));
        let latest = e.latest.as_ref().map(|l| match &l.got {
            Got::Record(r) if r.state == State::Firing => m2::Latest::Firing,
            Got::Record(_) => m2::Latest::Resolved,
            Got::Deleted => m2::Latest::Deleted,
            Got::Undecodable(_) => m2::Latest::Undecodable,
        });
        let stated = self.stated(addr);
        let previous_instance = e.latest.as_ref().is_some_and(|l| {
            l.via == AlarmVia::Get
                && l.snapshot_run
                    .is_some_and(|run| !stated.is_empty() && !stated.contains(&run))
        });
        let severity = e.latest.as_ref().and_then(|l| match &l.got {
            Got::Record(r) => r.severity,
            _ => None,
        });
        Some(SlotReading {
            reading: m2::Reading {
                presence,
                // At the horizon of the revision its owner names (§2.7,
                // §2.14 step 6, text 0.2): the standard's 900 s is one. A
                // revision not in hand gives no horizon: unobservable,
                // never 900 s, which would call a stale alarm of a shorter
                // horizon fresh.
                freshness: Some(match self.v2.revision(addr) {
                    Some(r) => fm::judge_all(&r.horizon, &observations),
                    None => fm::Judged {
                        verdict: fm::Verdict::Unobservable,
                        reason: fm::Reason::NotAHorizon,
                    },
                }),
                latest,
                lost: lossy && !answered,
                previous_instance,
                held_whole: self.owner_set_v2(addr).verdict == SetVerdict::Established,
                left_out: self.left_out_v2(addr, slot, e),
            },
            severity,
        })
    }

    fn detached_reading(&self, addr: &Addr, d: &Detached) -> SlotReading {
        SlotReading {
            reading: m2::Reading {
                presence: self.presence_for(addr, Major::V2),
                freshness: None,
                latest: (d.how == m2::Displaced::Rebound).then_some(m2::Latest::Rebound),
                lost: false,
                previous_instance: false,
                held_whole: self.owner_set_v2(addr).verdict == SetVerdict::Established,
                left_out: false,
            },
            severity: d.held.record.severity,
        }
    }

    /// Each alarm of `addr` this reader holds, its first answer and its
    /// severity, as `any_firing` takes them (§2.15): every slot's, an
    /// unnamed one's among them, and every displaced alarm's.
    pub(super) fn v2_answers(
        &self,
        addr: &Addr,
        now: Instant,
        wall: SystemTime,
    ) -> Vec<(Judged, Option<Severity>)> {
        let slots = self
            .v2
            .slots
            .keys()
            .filter(|(a, _)| a == addr)
            .filter_map(|(a, s)| self.slot_reading(a, *s, now, wall))
            .map(|r| (r.firing(), r.severity));
        let detached = self
            .v2
            .detached
            .iter()
            .filter(|((a, _), _)| a == addr)
            .map(|((a, _), d)| self.detached_reading(a, d))
            .map(|r| (r.firing(), r.severity));
        slots.chain(detached).collect()
    }

    /// How many alarms of `addr` this reader holds under `alarms.v2`: its
    /// slots and its displaced alarms.
    pub fn slots_of(&self, addr: &Addr) -> usize {
        self.v2.slots.keys().filter(|(a, _)| a == addr).count()
            + self.v2.detached.keys().filter(|(a, _)| a == addr).count()
    }

    /// §5's fourth question, "does this owner hold each identity on one
    /// slot?" (§2.2): the model's `taken` over the slots held, under the
    /// run its present descriptor states, and `one_slot_each`.
    pub fn one_slot_each(&self, addr: &Addr) -> Answer {
        let seen: Vec<m2::Seen> = self
            .v2
            .slots
            .iter()
            .filter(|((a, _), _)| a == addr)
            .filter_map(|((_, slot), e)| {
                let h = e.track.held?;
                let stamp = e.latest.as_ref()?.inband.unwrap_or(Stamp {
                    run: h.run,
                    seq: zenkey_model::wire::Seq::new(1)?,
                });
                Some(m2::Seen {
                    slot: *slot,
                    stamp,
                    alert_key: h.alert_key,
                })
            })
            .collect();
        let present = self.stated(addr).into_iter().next();
        m2::one_slot_each(self.owner_set_v2(addr).verdict, &m2::taken(&seen, present))
    }

    /// Every `alarms.v2` row this reader holds (§5): a row per slot held,
    /// per displaced alarm, and per archived record of a slot no reading
    /// holds.
    pub fn v2_rows(&self, now: Instant, wall: SystemTime) -> Vec<AlarmRow> {
        let mut rows = Vec::new();
        for ((owner, slot), e) in &self.v2.slots {
            let Some(r) = self.slot_reading(owner, *slot, now, wall) else {
                continue;
            };
            let table = self.v2.table(owner);
            let words = e.words.as_ref().map(|w| &w.words);
            // A slot that holds nothing of its alarm any more names it by
            // its last record.
            let record = e
                .latest
                .as_ref()
                .and_then(|l| match &l.got {
                    Got::Record(r) => Some(*r),
                    _ => None,
                })
                .or(e.last_record);
            let (alarm, named) = match record {
                Some(rec) => {
                    let (a, w) = naming(owner, &rec, table, words);
                    (a, Some(w))
                }
                None => (None, None),
            };
            let rederives = match &e.latest {
                None => m2::words_derive(Err(m2::NotTaken::NoRecord)),
                Some(l) => match &l.got {
                    Got::Record(rec) => {
                        let ok: Result<m2::Record, ()> = Ok(*rec);
                        m2::words_derive(m2::words_taken(Some(&ok), words))
                    }
                    Got::Undecodable(_) => m2::words_derive(Err(m2::NotTaken::Undecodable)),
                    Got::Deleted => m2::words_derive(Err(m2::NotTaken::NoRecord)),
                },
            };
            rows.push(AlarmRow {
                key: slot_key(owner, *slot),
                alarm,
                profile: Some(m2::PROFILE),
                slot: Some(*slot),
                words: named,
                displaced: None,
                firing: answer_of(r.firing()),
                stale: answer_of(r.stale()),
                rederives: answer_of(rederives),
                severity: r.severity,
                latest: e.latest.as_ref().map(seen_of),
                problems: Vec::new(),
                foreign_stamp: false,
                last_known: self
                    .v2
                    .archived
                    .get(&(owner.clone(), *slot))
                    .map(|a| self.last_known_v2(owner, a)),
            });
        }
        for ((owner, _), d) in &self.v2.detached {
            let r = self.detached_reading(owner, d);
            let (alarm, named) = naming(owner, &d.held.record, self.v2.table(owner), None);
            rows.push(AlarmRow {
                key: slot_key(owner, d.slot),
                alarm,
                profile: Some(m2::PROFILE),
                slot: None,
                words: Some(named),
                displaced: Some(d.how.as_str()),
                firing: answer_of(r.firing()),
                stale: answer_of(r.stale()),
                // A displaced alarm is held on no slot: no record (§5).
                rederives: answer_of(m2::words_derive(Err(m2::NotTaken::NoRecord))),
                severity: r.severity,
                latest: None,
                problems: Vec::new(),
                foreign_stamp: false,
                last_known: None,
            });
        }
        for ((owner, slot), a) in &self.v2.archived {
            if self.v2.slots.contains_key(&(owner.clone(), *slot)) {
                continue;
            }
            let premise = match self.presence_of(owner) {
                am::Presence::Absent => Reason::Absent,
                _ => Reason::LastKnown,
            };
            let not_asked = answer_of(Judged {
                answer: Answer::NotAsked,
                reason: premise,
            });
            let lk = self.last_known_v2(owner, a);
            let (alarm, words) = match self.archived_record(owner, a) {
                Some(rec) => {
                    let (r, w) = naming(owner, &rec, self.v2.table(owner), None);
                    (r, Some(w))
                }
                None => (None, None),
            };
            rows.push(AlarmRow {
                key: a.key.clone(),
                alarm,
                profile: Some(m2::PROFILE),
                slot: Some(*slot),
                words,
                displaced: None,
                firing: not_asked.clone(),
                stale: not_asked.clone(),
                rederives: not_asked,
                severity: None,
                latest: None,
                problems: Vec::new(),
                foreign_stamp: false,
                last_known: Some(lk),
            });
        }
        rows
    }

    /// An archive's record, read under its envelope's tag and its owner's
    /// epoch; `None` for a tombstone, or one that does not read.
    fn archived_record(&self, owner: &Addr, a: &ArchivedAlarm) -> Option<m2::Record> {
        let bytes = a.value.as_deref()?;
        m2::Record::read(a.tag?.get(), bytes, self.v2.epoch(owner)?).ok()
    }

    fn last_known_v2(&self, owner: &Addr, a: &ArchivedAlarm) -> LastKnownAlarm {
        LastKnownAlarm {
            archive: a.archive.to_string(),
            document: None,
            deleted: a.value.is_none(),
            stamp: a.stamp.as_ref().map(super::stamp_report),
            confirmed: a.confirmed,
            record: self.archived_record(owner, a).map(|r| r.to_json()),
        }
    }

    // ─── transitions ────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn slot_transition(
        &mut self,
        owner: &Addr,
        slot: u8,
        change: AlarmChange,
        via: Option<AlarmVia>,
        lost: Option<AlarmLoss>,
        now: Instant,
        wall: SystemTime,
    ) -> AlarmTransition {
        let r = self.slot_reading(owner, slot, now, wall);
        let firing = r.as_ref().map_or_else(super::unread, SlotReading::firing);
        let table = self.v2.table(owner).map(<[m2::Rule]>::to_vec);
        let e = self
            .v2
            .slots
            .get_mut(&(owner.clone(), slot))
            .expect("a slot the book holds");
        e.reported = Some(firing);
        let latest = e.latest.as_ref().and_then(|l| match &l.got {
            Got::Record(r) => Some(*r),
            _ => None,
        });
        let record = latest.or(e.last_record);
        let (alarm, words) = match &record {
            Some(rec) => {
                let (a, w) = naming(
                    owner,
                    rec,
                    table.as_deref(),
                    e.words.as_ref().map(|w| &w.words),
                );
                (a, Some(w))
            }
            None => (
                e.track.held.and_then(|h| h.alert_key).map(|k| AlarmRef {
                    system: owner.system.as_str().to_owned(),
                    service: owner.service.as_str().to_owned(),
                    alert_key: am::key_hex(k),
                }),
                None,
            ),
        };
        AlarmTransition {
            key: slot_key(owner, slot),
            alarm,
            change,
            firing: answer_of(firing),
            severity: latest.and_then(|r| r.severity),
            document: None,
            stamp: None,
            via,
            lost,
            profile: Some(m2::PROFILE),
            slot: Some(slot),
            words,
            record: if matches!(change, AlarmChange::Lost | AlarmChange::Judged) {
                None
            } else {
                latest.map(|r| r.to_json())
            },
            displaced: None,
        }
    }

    fn detached_transition(
        &mut self,
        id: &(Addr, Ident),
        change: AlarmChange,
        via: Option<AlarmVia>,
        _now: Instant,
        _wall: SystemTime,
    ) -> AlarmTransition {
        let d = self
            .v2
            .detached
            .get(id)
            .expect("a displaced alarm the book holds")
            .clone();
        let firing = self.detached_reading(&id.0, &d).firing();
        let (alarm, words) = naming(&id.0, &d.held.record, self.v2.table(&id.0), None);
        if let Some(d) = self.v2.detached.get_mut(id) {
            d.reported = Some(firing);
        }
        AlarmTransition {
            key: slot_key(&id.0, d.slot),
            alarm,
            // A record `replaced` or `dropped` is no change of its alarm to
            // report by itself (§2.16): it is judged, not resolved.
            change: if d.how == m2::Displaced::Rebound {
                change
            } else {
                AlarmChange::Judged
            },
            firing: answer_of(firing),
            severity: d.held.record.severity,
            document: None,
            stamp: None,
            via,
            lost: None,
            profile: Some(m2::PROFILE),
            slot: None,
            words: Some(words),
            record: Some(d.held.record.to_json()),
            displaced: Some(d.how.as_str()),
        }
    }

    /// A loss (core §2.7): every slot and every displaced alarm moves to
    /// unobservable, as a key does.
    pub(super) fn lose_v2(
        &mut self,
        loss: AlarmLoss,
        at: Instant,
        wall: SystemTime,
    ) -> Vec<AlarmTransition> {
        let slots: Vec<(Addr, u8)> = self.v2.slots.keys().cloned().collect();
        slots
            .into_iter()
            .map(|(a, s)| {
                self.slot_transition(&a, s, AlarmChange::Lost, None, Some(loss), at, wall)
            })
            .collect()
    }

    /// Every slot and every displaced alarm whose first answer moved since
    /// it was last reported, with no sample; a slot the read that
    /// established its owner left out holds nothing from now (§2.14 step 5),
    /// and it, a deleted one and a displaced alarm are forgotten
    /// [`FORGET_AFTER`] later.
    pub(super) fn sweep_v2(&mut self, now: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        let left: Vec<(Addr, u8)> = self
            .v2
            .slots
            .iter()
            .filter(|((a, s), e)| {
                e.latest.is_some()
                    && self.owner_set_v2(a).verdict == SetVerdict::Established
                    && self.left_out_v2(a, *s, e)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in left {
            if let Some(e) = self.v2.slots.get_mut(&k) {
                e.latest = None;
                e.track = m2::Track::default();
                e.merged = Merge::Nothing;
                e.gone_at.get_or_insert(now);
            }
        }
        self.v2.slots.retain(|_, e| {
            e.gone_at
                .is_none_or(|g| now.saturating_duration_since(g) < FORGET_AFTER)
        });
        self.v2
            .detached
            .retain(|_, d| now.saturating_duration_since(d.at) < FORGET_AFTER);
        let mut out = Vec::new();
        let slots: Vec<(Addr, u8)> = self.v2.slots.keys().cloned().collect();
        for (a, s) in slots {
            let j = self.slot_reading(&a, s, now, wall).map(|r| r.firing());
            if self
                .v2
                .slots
                .get(&(a.clone(), s))
                .is_some_and(|e| e.reported != j)
            {
                out.push(self.slot_transition(&a, s, AlarmChange::Judged, None, None, now, wall));
            }
        }
        let detached: Vec<(Addr, Ident)> = self.v2.detached.keys().cloned().collect();
        for id in detached {
            let d = &self.v2.detached[&id];
            let j = Some(self.detached_reading(&id.0, d).firing());
            if d.reported != j {
                out.push(self.detached_transition(&id, AlarmChange::Judged, None, now, wall));
            }
        }
        out
    }
}

/// A slot's latest sample as a report says it (§2.6).
fn seen_of(l: &SlotLatest) -> AlarmSeen {
    let (latest, record, invalid) = match &l.got {
        Got::Record(r) => (
            if r.state == State::Firing {
                AlarmLatest::Firing
            } else {
                AlarmLatest::Resolved
            },
            Some(*r),
            None,
        ),
        Got::Deleted => (AlarmLatest::Deleted, None, None),
        Got::Undecodable(why) => (AlarmLatest::Undecodable, None, Some(why.clone())),
    };
    AlarmSeen {
        latest,
        document: None,
        invalid,
        stamp: None,
        via: l.via,
        record: record.map(|r| r.to_json()),
        at_s: record.and_then(|r| r.at),
    }
}

#[cfg(test)]
mod tests {
    //! The `alarms.v2` half folded from values: a complete snapshot with no
    //! entry establishes "nothing fires", a slot is named by the table its
    //! owner's descriptor names or shown by its numbers, `rebound` answers
    //! the displaced alarm no, a gap or a loss since the read leaves the
    //! owner unobservable, and both majors make one answer.

    use super::*;
    use crate::model::alarms::{AlarmBook, BookSpec};
    use crate::model::catalog::Observed;
    use std::time::Duration;
    use zenkey_model::collection::{Confirmation, Entry as PageEntry, Page, Time};
    use zenkey_model::wire::Seq;

    const INSTANCE: &str = "000000000000000a";

    fn table() -> Vec<m2::Rule> {
        m2::rules(&serde_json::json!([
            {"name": "hot", "summary": "Temperature {argument} C",
             "argument": {"minimum": "-40", "resolution": "0.1"}},
            {"name": "link_down", "summary": "Link to {peer} on {port} is down",
             "labels": [{"name": "peer", "values": ["r1", "r2", "r3"]},
                        {"name": "port", "values": ["eth0", "eth1"]}]},
            {"name": "sensor_lost", "summary": "Sensor {sensor} lost",
             "labels": [{"name": "sensor", "minimum": 0, "maximum": 63}]},
            {"name": "door_open", "summary": "Enclosure door open"},
        ]))
        .expect("T1")
    }

    fn now_s() -> i64 {
        i64::try_from(
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    /// A of the scenarios: `link_down`, `r2`, `eth0`; B: `door_open`.
    fn a(state: State) -> m2::Record {
        m2::Record {
            state,
            severity: Some(Severity::Warning),
            rule: m2::RuleIndex::Table(1),
            at: Some(now_s()),
            labels: Some(2),
            argument: m2::Argument::NotAvailable,
            key: None,
        }
    }

    fn b(state: State) -> m2::Record {
        m2::Record {
            rule: m2::RuleIndex::Table(3),
            labels: None,
            ..a(state)
        }
    }

    /// One page of `owner`'s records under run `run`: each entry its slot,
    /// its seq and its record, vouched for at the snapshot's time.
    fn page(owner: &str, run: u16, entries: &[(u8, u32, m2::Record)]) -> (String, Vec<u8>) {
        let time = Time::from_unix_ms(i128::from(now_s()) * 1000, 0).unwrap();
        let p = Page {
            tag: Tag::new(m2::TAG).unwrap(),
            run: Run::U16(run),
            asof: Seq::new(entries.iter().map(|e| e.1).max().unwrap_or(1)).unwrap(),
            time,
            index: 0,
            more: false,
            coll: m2::ALARMS_COLL,
            live: entries
                .iter()
                .map(|(slot, seq, r)| PageEntry {
                    id: slot.to_string(),
                    run: None,
                    seq: Seq::new(*seq).unwrap(),
                    confirmation: Confirmation::AtTime,
                    record: r.write(0).unwrap().bytes,
                })
                .collect(),
            tombs: Vec::new(),
        };
        (
            format!("zk2/{owner}/alarms.v2/state/alarms"),
            p.write().unwrap(),
        )
    }

    /// A presence read of `owner` with one instance whose descriptor lists
    /// `iface`, names `tc.v1` beside it, and states run 7.
    fn observed(owner: &str, iface: &str) -> Observed {
        let keys = [format!("zk2/{owner}/@zk/instance/{INSTANCE}")];
        let mut o = Observed::from_keys("zk2/lab/*/@zk/**", &keys, true);
        let d: zenkey_model::descriptor::Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1", "service": owner, "instance": INSTANCE,
            "run": "0007",
            "interfaces": [
                {"iface": iface, "contract": format!("sha256:{}", "a".repeat(64)),
                 "minor": 0, "token": false},
            ],
            "meta": {},
        }))
        .unwrap();
        o.descriptors = Some(BTreeMap::from([(
            (owner.parse().unwrap(), INSTANCE.parse().unwrap()),
            crate::model::catalog::DescriptorRead::Served(Box::new(d)),
        )]));
        o
    }

    fn book(selector: &str) -> AlarmBook {
        let mut spec = BookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = true;
        AlarmBook::new(AlarmScope::parse(selector).unwrap(), spec)
    }

    fn read(at: Instant, replies: Vec<(String, Vec<u8>)>) -> SlotRead {
        SlotRead {
            asked_at: at,
            complete: true,
            replies,
        }
    }

    /// The words a refresh reads for `owner`, whose descriptor names the
    /// standard revision beside its table's.
    fn named(owner: &str, table: Option<Vec<m2::Rule>>) -> V2Words {
        V2Words {
            tables: BTreeMap::from([(owner.parse().unwrap(), table)]),
            default_epoch: Some(0),
            revisions: BTreeMap::from([(
                owner.parse().unwrap(),
                Some(V2Revision::standard().clone()),
            )]),
            ..V2Words::default()
        }
    }

    /// §2.15, §2.17: a selector of `alarms.v1` reaches an `alarms.v2`
    /// owner; one complete snapshot with no entry establishes it, and
    /// "nothing fires" is its answer; with A firing on slot 0 it is yes,
    /// and the row names A by the table: `link_down`, its reference v1's.
    #[test]
    fn one_snapshot_establishes_an_owner_and_its_table_names_a_slot() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let owner: Addr = "lab/svc".parse().unwrap();
        let mut bk = book("zk2/lab/*/alarms.v1/state/alarms/*");
        bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
        bk.v2_words(named("lab/svc", Some(table())));
        assert_eq!(bk.major_of(&owner), Some(Major::V2));
        bk.seed_v2(read(t0, vec![page("lab/svc", 7, &[])]), t0, wall);
        let set = bk.owner_set(&owner, t0);
        assert_eq!(
            (set.verdict, set.reason),
            (SetVerdict::Established, Reason::Snapshot)
        );
        let any = bk.any_firing(Severity::Warning, t0, wall);
        assert_eq!((any.answer, any.reason), (Answer::No, Reason::NoneFiring));

        let t1 = t0 + Duration::from_secs(1);
        bk.seed_v2(
            read(t1, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
            t1,
            wall,
        );
        let any = bk.any_firing(Severity::Warning, t1, wall);
        assert_eq!((any.answer, any.firing), (Answer::Yes, 1));
        let rows = bk.v2_rows(t1, wall);
        let row = &rows[0];
        assert_eq!(row.slot, Some(0));
        assert_eq!(row.profile, Some("alarms.v2"));
        assert_eq!(
            row.alarm.as_ref().map(|r| r.alert_key.as_str()),
            Some("a659f813308ad1da")
        );
        let w = row.words.as_ref().unwrap();
        assert_eq!(
            (w.by, w.rule.as_deref(), w.summary.as_deref()),
            (
                AlarmNamedBy::Table,
                Some("link_down"),
                Some("Link to r2 on eth0 is down")
            )
        );
        assert_eq!(row.firing.reason, "firing");
        assert_eq!(row.rederives.reason, "table");
        assert_eq!(bk.one_slot_each(&owner), Answer::Yes);
    }

    /// §2.4, §2.14: with no table of its owner a record is unnamed: its
    /// numbers shown, no reference, and its answer still counts.
    #[test]
    fn an_unnamed_alarm_is_shown_by_its_numbers_and_counts() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let mut bk = book("zk2/lab/*/alarms.v2/state/alarms/*");
        bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
        bk.v2_words(named("lab/svc", None));
        bk.seed_v2(
            read(t0, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
            t0,
            wall,
        );
        let rows = bk.v2_rows(t0, wall);
        assert!(rows[0].alarm.is_none(), "an unnamed alarm has no reference");
        let w = rows[0].words.as_ref().unwrap();
        assert_eq!(
            (w.by, w.index, w.codes, w.reason.as_deref()),
            (AlarmNamedBy::Numbers, Some(1), Some(2), Some("no_table"))
        );
        assert_eq!(rows[0].firing.reason, "firing");
        let any = bk.any_firing(Severity::Warning, t0, wall);
        assert_eq!((any.answer, any.firing), (Answer::Yes, 1));
    }

    /// §2.6, §2.14 (scenarios-v2.md §4, step 2): a reader holding A firing
    /// on slot 0 reads B there under the same run as A `rebound`: A is no,
    /// `rebound`, B raised; a resolve to report, then B's raise.
    #[test]
    fn another_identity_on_a_firing_slot_rebounds_the_held_alarm() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let mut bk = book("zk2/lab/*/alarms.v2/state/alarms/*");
        bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
        bk.v2_words(named("lab/svc", Some(table())));
        bk.seed_v2(
            read(t0, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
            t0,
            wall,
        );
        let t1 = t0 + Duration::from_secs(2);
        let ts = bk.seed_v2(
            read(t1, vec![page("lab/svc", 7, &[(0, 6, b(State::Firing))])]),
            t1,
            wall,
        );
        let changes: Vec<(Option<&str>, AlarmChange)> =
            ts.iter().map(|t| (t.displaced, t.change)).collect();
        assert_eq!(
            changes,
            [
                (Some("rebound"), AlarmChange::Resolved),
                (None, AlarmChange::Raised)
            ]
        );
        let rows = bk.v2_rows(t1, wall);
        let a_row = rows
            .iter()
            .find(|r| r.displaced.is_some())
            .expect("A, displaced");
        assert_eq!(
            (a_row.firing.answer, a_row.firing.reason.as_str()),
            (crate::report::AlarmAnswerToken::No, "rebound")
        );
        assert_eq!(a_row.slot, None, "A is on no slot");
        let b_row = rows.iter().find(|r| r.slot == Some(0)).unwrap();
        assert_eq!(
            b_row.alarm.as_ref().map(|r| r.alert_key.as_str()),
            Some("abb089906038056a")
        );
        assert_eq!(b_row.firing.reason, "firing");
        // A back on slot 1: no longer displaced.
        let t2 = t1 + Duration::from_secs(1);
        bk.seed_v2(
            read(
                t2,
                vec![page(
                    "lab/svc",
                    7,
                    &[(0, 6, b(State::Firing)), (1, 8, a(State::Firing))],
                )],
            ),
            t2,
            wall,
        );
        let rows = bk.v2_rows(t2, wall);
        assert!(rows.iter().all(|r| r.displaced.is_none()), "{rows:?}");
        assert_eq!(rows.len(), 2);
    }

    /// §2.15's order, steps 3 and 4: an instance's move since the read is
    /// `owner_gap`, a loss `window_lost`; a snapshot under a run no present
    /// descriptor states `snapshot_run`; a read asked after both
    /// establishes again, and a slot it left out reads `not_held`.
    #[test]
    fn a_gap_or_a_loss_since_the_read_leaves_the_owner_unobservable() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let owner: Addr = "lab/svc".parse().unwrap();
        let mut bk = book("zk2/lab/*/alarms.v2/state/alarms/*");
        bk.subscribed(t0);
        bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
        bk.v2_words(named("lab/svc", Some(table())));
        let t1 = t0 + Duration::from_secs(1);
        bk.seed_v2(
            read(t1, vec![page("lab/svc", 7, &[(0, 3, b(State::Firing))])]),
            t1,
            wall,
        );
        assert_eq!(bk.owner_set(&owner, t1).reason, Reason::Snapshot);
        let t2 = t1 + Duration::from_secs(1);
        bk.instance(&owner, &"000000000000000b".parse().unwrap(), true, t2);
        assert_eq!(bk.owner_set(&owner, t2).reason, Reason::OwnerGap);
        let any = bk.any_firing(Severity::Critical, t2, wall);
        assert_eq!(any.answer, Answer::Unobservable);
        let t3 = t2 + Duration::from_secs(1);
        bk.seed_v2(read(t3, vec![page("lab/svc", 9, &[])]), t3, wall);
        assert_eq!(
            bk.owner_set(&owner, t3).reason,
            Reason::SnapshotRun,
            "run 9 is no present descriptor's"
        );
        bk.seed_v2(read(t3, vec![page("lab/svc", 7, &[])]), t3, wall);
        assert_eq!(bk.owner_set(&owner, t3).reason, Reason::Snapshot);
        bk.sweep(t3, wall);
        let rows = bk.v2_rows(t3, wall);
        let b_row = rows.iter().find(|r| r.slot == Some(0)).unwrap();
        assert_eq!(b_row.firing.reason, "not_held", "left out: {b_row:?}");
        bk.lose(
            AlarmLoss {
                dropped: 1,
                stalled: false,
            },
            t3 + Duration::from_secs(1),
            wall,
        );
        assert_eq!(
            bk.owner_set(&owner, t3 + Duration::from_secs(1)).reason,
            Reason::WindowLost
        );
    }

    /// One page of `owner`'s words under run `run` (§2.4, collection 1).
    fn words_page(
        owner: &str,
        run: u16,
        entries: &[(u8, u32, serde_json::Value)],
    ) -> (String, Vec<u8>) {
        let time = Time::from_unix_ms(i128::from(now_s()) * 1000, 0).unwrap();
        let p = Page {
            tag: Tag::new(0).unwrap(),
            run: Run::U16(run),
            asof: Seq::new(entries.iter().map(|e| e.1).max().unwrap_or(1)).unwrap(),
            time,
            index: 0,
            more: false,
            coll: m2::WORDS_COLL,
            live: entries
                .iter()
                .map(|(slot, seq, w)| PageEntry {
                    id: slot.to_string(),
                    run: None,
                    seq: Seq::new(*seq).unwrap(),
                    confirmation: Confirmation::None,
                    record: serde_json::to_vec(w).unwrap(),
                })
                .collect(),
            tombs: Vec::new(),
        };
        (
            format!("zk2/{owner}/alarms.v2/state/words"),
            p.write().unwrap(),
        )
    }

    /// §2.4, §5's third question: an alarm by words is named by its slot's
    /// words where they derive its record's key (`strict`), and shown by
    /// its alert key where they do not (`mismatch`) or none were read
    /// (`no_words`).
    #[test]
    fn a_slots_words_name_its_alarm_only_where_they_derive_its_key() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let d = m2::Record {
            rule: m2::RuleIndex::NotAvailable,
            labels: None,
            key: Some(0x719d_1599_37e5_4eb8),
            ..a(State::Firing)
        };
        let words = |rule: &str| {
            serde_json::json!({"rule": rule, "labels": {"mount": "/var"},
                               "summary": "/var is full"})
        };
        let read_with = |w: Option<serde_json::Value>| {
            let mut bk = book("zk2/lab/*/alarms.v2/state/alarms/*");
            bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
            bk.v2_words(named("lab/svc", Some(table())));
            let mut replies = vec![page("lab/svc", 7, &[(0, 3, d)])];
            replies.extend(w.map(|w| words_page("lab/svc", 7, &[(0, 2, w)])));
            bk.seed_v2(read(t0, replies), t0, wall);
            bk.v2_rows(t0, wall).remove(0)
        };
        let row = read_with(Some(words("disk_full")));
        assert_eq!(row.rederives.reason, "strict");
        let w = row.words.unwrap();
        assert_eq!(
            (w.by, w.rule.as_deref()),
            (AlarmNamedBy::Words, Some("disk_full"))
        );
        let row = read_with(Some(words("disk_empty")));
        assert_eq!(
            (row.rederives.answer, row.rederives.reason.as_str()),
            (crate::report::AlarmAnswerToken::No, "mismatch")
        );
        assert_eq!(row.words.unwrap().by, AlarmNamedBy::Key);
        assert_eq!(
            row.alarm.map(|a| a.alert_key),
            Some("719d159937e54eb8".to_owned()),
            "shown by its alert key"
        );
        let row = read_with(None);
        assert_eq!(row.rederives.reason, "no_words");
    }

    /// §2.1, §2.17: two owners, one of each major, make one answer; an
    /// `alarms.v1` key of an owner whose descriptor lists `alarms.v2` alone
    /// is not asked there, `not_listed`.
    #[test]
    fn both_majors_make_one_answer() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let mut bk = book("zk2/lab/*/alarms.v1/state/alarms/*");
        let mut o = observed("lab/svc", m2::IFACE);
        let old = observed("lab/old", "alarms.v1");
        o.tokens.extend(old.tokens.iter().cloned());
        o.descriptors
            .as_mut()
            .unwrap()
            .extend(old.descriptors.clone().unwrap());
        bk.presence_read(&o, t0);
        bk.v2_words(named("lab/svc", Some(table())));
        assert_eq!(bk.major_of(&"lab/old".parse().unwrap()), Some(Major::V1));
        bk.seed_v2(
            read(t0, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
            t0,
            wall,
        );
        bk.seed(
            crate::model::alarms::SeedRead {
                asked_at: t0,
                complete: true,
                replies: Vec::new(),
                discarded: 0,
                envelopes: BTreeMap::new(),
            },
            t0,
            wall,
        );
        let any = bk.any_firing(Severity::Warning, t0, wall);
        assert_eq!(any.answer, Answer::Yes, "{any:?}");
        assert_eq!(any.firing, 1);
        assert_eq!(
            bk.presence_for(&"lab/svc".parse().unwrap(), Major::V1),
            am::Presence::Present(am::Listing::NotListed)
        );
    }

    /// §2.7 (text 0.2, F-613): an alarm ages at the horizon of the
    /// revision its owner names. On a copy of the standard contract at 4 s,
    /// an entry its owner vouched for 10 s before reads stale, unobservable,
    /// where the standard's 900 s read it firing; and the owner's view is
    /// read by the copy's layout, so its heartbeat, whose tag is the copy's,
    /// is read, never refused as another revision's.
    #[test]
    fn an_alarm_ages_at_the_horizon_of_its_owners_revision() {
        let mut contract = zenkey::alarms::v2::implementation().contract().clone();
        contract
            .resources
            .iter_mut()
            .find(|r| r.template.as_str() == m2::ALARMS)
            .unwrap()
            .annotations
            .insert("freshness.ttl_s".to_owned(), serde_json::json!(4));
        let copy = V2Revision::of(&contract);
        let standard = V2Revision::standard();
        assert_eq!(copy.horizon, fm::Horizon::Within(Duration::from_secs(4)));
        assert_eq!(standard.horizon, am::horizon());
        assert_ne!(copy.layout.tag, standard.layout.tag, "the copy's own tag");

        let owner: Addr = "lab/svc".parse().unwrap();
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let later = wall + Duration::from_secs(10);
        let mut bk = book("zk2/lab/*/alarms.v2/state/alarms/*");
        bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
        bk.v2_words(named("lab/svc", Some(table())));
        bk.seed_v2(
            read(t0, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
            t0,
            wall,
        );
        let tag = |bk: &AlarmBook| view_of(&bk.views[&owner]).layout().tag;
        let firing = |bk: &AlarmBook| {
            let j = bk.slot_reading(&owner, 0, t0, later).unwrap().firing();
            (j.answer, j.reason)
        };
        assert_eq!(tag(&bk), standard.layout.tag);
        assert_eq!(
            firing(&bk),
            (Answer::Yes, Reason::Firing),
            "at the standard's 900 s"
        );

        let mut words = named("lab/svc", Some(table()));
        words.revisions.insert(owner.clone(), Some(copy.clone()));
        bk.v2_words(words);
        assert_eq!(tag(&bk), copy.layout.tag, "read by the copy's layout");
        assert_eq!(
            firing(&bk),
            (Answer::Unobservable, Reason::Stale),
            "at the copy's 4 s"
        );
        // An owner none of whose descriptors this reader read: on the near
        // side a revision not in hand (text 0.2, "No descriptor read");
        // across a face the deployment's word, the revision held offline,
        // else the standard one.
        let gone: Addr = "lab/gone".parse().unwrap();
        assert_eq!(bk.v2.revision(&gone), None, "the near side");
        bk.v2_words(V2Words {
            word_revision: Some(copy.clone()),
            revisions: BTreeMap::new(),
            ..named("lab/svc", Some(table()))
        });
        assert_eq!(bk.v2.revision(&gone), None, "the word is a face's alone");
        assert_eq!(bk.v2.revision(&owner), Some(&copy), "its own kept");
        let mut far = Slots::across(true);
        assert_eq!(far.revision(&gone), Some(standard));
        far.words.word_revision = Some(copy.clone());
        assert_eq!(far.revision(&gone), Some(&copy));

        // A revision its owner names that is not in hand gives no horizon:
        // unobservable, never at the standard's 900 s, at once as 10 s on;
        // its heartbeat read with the standard's layout, which refuses
        // another revision's by its tag.
        let mut words = named("lab/svc", Some(table()));
        words.revisions.insert(owner.clone(), None);
        bk.v2_words(words);
        assert_eq!(bk.v2.revision(&owner), None);
        assert_eq!(tag(&bk), standard.layout.tag);
        assert_eq!(
            firing(&bk),
            (
                Answer::Unobservable,
                Reason::Freshness(fm::Reason::NotAHorizon)
            ),
            "no horizon in hand"
        );
        let j = bk.slot_reading(&owner, 0, t0, wall).unwrap().firing();
        assert_eq!(j.answer, Answer::Unobservable, "fresh or not, no verdict");
    }

    /// §2.7 (text 0.2, "No descriptor read", review-bot on #1176): on the
    /// near side, an owner whose descriptor names no revision this reader
    /// read is a revision not in hand, its firing alarm unobservable,
    /// `not_a_horizon`, never aged at 900 s; across a face, the deployment's
    /// word, the standard revision, ages it: yes, `firing`.
    #[test]
    fn no_descriptor_read_on_the_near_side_is_a_revision_not_in_hand() {
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let owner: Addr = "lab/svc".parse().unwrap();
        let firing = |face: Option<bool>| {
            let mut spec = BookSpec::new(Duration::from_secs(1));
            spec.clocks_synced = true;
            spec.face = face;
            let scope = AlarmScope::parse("zk2/lab/svc/alarms.v2/state/alarms/*").unwrap();
            let mut bk = AlarmBook::new(scope, spec);
            if face.is_none() {
                bk.presence_read(&observed("lab/svc", m2::IFACE), t0);
            }
            // No refresh's words name a revision of the owner's.
            bk.v2_words(V2Words {
                revisions: BTreeMap::new(),
                ..named("lab/svc", Some(table()))
            });
            bk.seed_v2(
                read(t0, vec![page("lab/svc", 7, &[(0, 3, a(State::Firing))])]),
                t0,
                wall,
            );
            let j = bk.slot_reading(&owner, 0, t0, wall).unwrap().firing();
            (j.answer, j.reason)
        };
        assert_eq!(
            firing(None),
            (
                Answer::Unobservable,
                Reason::Freshness(fm::Reason::NotAHorizon)
            ),
            "the near side"
        );
        assert_eq!(
            firing(Some(true)),
            (Answer::Yes, Reason::Firing),
            "across a face"
        );
    }
}
