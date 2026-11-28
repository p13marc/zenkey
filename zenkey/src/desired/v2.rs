//! `desired.v2`'s half of a target (`spec/profiles/desired/v2.md`, #986's
//! J3): what a role bound to an author whose revision uses `desired.v2`
//! does differently (§2.18). Everything a fixture checks is
//! [`zenkey_model::desired::v2`]'s: the order of versions
//! ([`model2::Target`]), the watch ([`model2::Watch`]), the packed report
//! and its tables.
//!
//! ```text
//! a seed, a period or the watch ─▶ one GET of the slice at a time, carrying the held stamp (§2.5)
//!   answered by that stamp   ─▶ model2::Target::unchanged: nothing judged, a last-known confirmed
//!   a snapshot's entry       ─▶ model2::Target::observe: a live entry a put, a tombstone a delete
//!   a snapshot with no entry ─▶ model2::Target::observe_no_entry(run, asof): a revert under the held run
//!   its timeout              ─▶ no verdict: made again at the next trigger, never sooner
//! the author's heartbeat ─▶ model2::Watch::heartbeat ─▶ read │ unmoved │ … (§2.5, "The watch")
//! an act's end ─▶ its refusal's place and line by the report resource's tables ─▶ the packed Report put
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.4 the version is the in-band stamp | each sample's and each snapshot entry's run and seq ([`super::Incoming`]); a sample with no envelope is `unstamped`, never acted on |
//! | §2.4 a revision above 4,294,967,295 is refused whole, `invalid` | [`model2::Act::bound`], before the handler: at [`REVISION_POINTER`] with [`REVISION_LINE`], by the tables |
//! | §2.5 each read conditional while a version is held | the GET's payload is [`model2::Target::held`]'s stamp (core §7.5), read at the GET |
//! | §2.5 one read at a time | every read, a seed, a re-read and the watch's, is one seed at a time, and the next waits until the last was judged; a heartbeat that would make the watch read while one is outstanding makes none (`outstanding`) |
//! | §2.5 a read that ends at its timeout is made again at the next trigger, never sooner | no retry after a GET that ran to its timeout |
//! | §2.5 a period below the timeout is refused at the start | [`super::Target::declare`] |
//! | §2.5 the watch | a subscriber on the author's heartbeat key, and on its owner heartbeat key of the desired collection's class where the service holds a catalog; each read into a [`model2::Beat`] |
//! | §2.5 with no heartbeat the period stands, and the target says so | [`Watching::silent`], logged once |
//! | §2.8 a snapshot with no entry reverts under the held run, never under another | [`model2::Target::observe_no_entry`], given the page's run and asof |
//! | §2.10 the packed report, before the tokens and after each change | [`record`], put on the report resource's writer |
//! | §2.10 a place and a line as indices | [`model2::Tables::place`] and [`model2::Tables::line`]; one the tables do not hold is `out_of_range`, its words logged |

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey_model::catalog::{Catalog, CatalogId};
use zenkey_model::contract::{Contract, Resource};
use zenkey_model::heartbeat::{Class, Heartbeat, Layout, OwnerHeartbeat};
use zenkey_model::wire;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::Sample;

use super::{
    APPLY, AuthorSeen, Document, Ended, Event, INVALID, Incoming, Refusal, ReportAt, SEED_RETRIES,
    Seeded, TargetInner, UNDECODABLE, Version, Via, Why, model, model2,
};
use crate::error::{Error, Result};
use crate::service::ServiceBuilder;

/// Where §2.4's bound refuses a document: its own revision.
pub const REVISION_POINTER: &str = "/revision";

/// The line of §2.4's bound's refusal, as `scenarios-v2.md` §4 writes it.
/// A report resource whose `desired.details` holds these words reports the
/// refusal by their index; any other reports `out_of_range`.
pub const REVISION_LINE: &str = "a revision above 4,294,967,295";

/// How long an owner's catalog that could not be read stands as none
/// before its descriptor is asked again: never a GET a heartbeat.
const CATALOG_RETRY: Duration = Duration::from_secs(30);

/// The version a `desired.v1` handler is given for a `desired.v2` act: no
/// Zenoh timestamp orders it ([`Document::inband`] is its version).
pub(super) static NO_STAMP: Version = Version {
    stamp: zenkey_model::seed::Stamp { time: 0, id: 0 },
    revision: None,
};

/// A report's packed record (§2.10, §3), as its writer puts it.
pub(super) fn record(report: &model2::Report) -> Result<Vec<u8>> {
    report.write().map(|r| r.bytes).map_err(|e| {
        Error::Contract(format!(
            "desired.v2: the report has no record ({e:?}) (§2.10)"
        ))
    })
}

/// The two tables of the report resource `at` (§2.10), refused unless the
/// resource is a report resource of this major that reports `role`.
pub(super) fn tables_of(b: &ServiceBuilder, role: &str, at: &ReportAt) -> Result<model2::Tables> {
    let imp = &b
        .impls()
        .iter()
        .find(|s| s.imp.iface() == &at.iface)
        .ok_or_else(|| Error::NotImplemented(at.iface.clone()))?
        .imp;
    let r = imp.resource(&at.resource)?;
    let (iface, name) = (&at.iface, &at.resource);
    match model2::report_declaration_of(imp.contract(), r) {
        model2::ReportDeclaration::Declared { names, tables } => {
            let reports = match &names {
                model2::Names::Role(n) => n == role,
                model2::Names::Param(p) => at
                    .values
                    .get(p)
                    .is_some_and(|v| v.len() == 1 && v[0] == role),
            };
            if reports {
                Ok(tables)
            } else {
                Err(Error::Contract(format!(
                    "{iface} {name:?}: its desired.report does not name the role {role:?} \
                     (desired.v2 §2.10)"
                )))
            }
        }
        model2::ReportDeclaration::Invalid(why) => Err(Error::Contract(format!(
            "{iface} {name:?}: its desired.report is invalid ({}) (desired.v2 §2.10)",
            why.iter()
                .map(|w| w.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        model2::ReportDeclaration::NotThisProfiles => Err(Error::Contract(format!(
            "{iface} {name:?} declares no desired.report: a role bound to a desired.v2 author \
             is reported in a packed Report, on a report resource of the target's contract \
             (desired.v2 §2.10, §2.18)"
        ))),
    }
}

/// What a target's watch of its author's heartbeat holds (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watching {
    /// Whether the target watches: its author's desired resource declares
    /// a horizon above 0, so it is a collection of the author's heartbeat.
    /// `false`: the deployment's period alone re-reads.
    pub watches: bool,
    /// The section it expects, a digest and a live count, or none.
    pub expects: Option<model2::Section>,
    /// The heartbeats read so far, in either form.
    pub read: u64,
    /// The heartbeats not read: of another revision, refused, or in a
    /// group whose catalog the target does not hold.
    pub unread: u64,
    /// The last heartbeat read: its key's stamp.
    pub last: Option<wire::Stamp>,
    /// The reads the watch asked for.
    pub asked: u64,
    /// It was to watch and reads no heartbeat: none within the horizon.
    /// The deployment's period stands, each re-read conditional.
    pub silent: bool,
}

/// One reply of a read of the slice (§2.5, "What a read draws").
#[derive(Debug)]
pub(super) enum Item {
    /// A live entry, a tombstone, or a per-key reply.
    Entry(Incoming),
    /// A snapshot with no entry: its run and its asof.
    NoEntry { run: wire::Run, asof: wire::Seq },
    /// The held stamp the GET carried: unchanged.
    Unchanged(wire::Stamp),
    /// A held stamp the GET did not carry: no answer (core §4.2, `held`).
    NotSent(wire::Stamp),
}

/// What one GET of the slice drew.
#[derive(Debug, Default)]
pub(super) struct Got {
    pub items: Vec<Item>,
    /// It ran to its timeout: no verdict, and not made again before the
    /// next trigger.
    pub timed_out: bool,
}

/// A read that ended, for the target's one task to judge in order.
pub(super) struct Read {
    got: Got,
    /// The watch asked for it: it is told how it ended.
    watch: bool,
    /// Told once it is judged: the next read waits for it.
    done: tokio::sync::oneshot::Sender<()>,
}

/// A heartbeat of the author's, as its subscriber heard it.
pub(super) enum Heard {
    /// The heartbeat covering the author's interface, read.
    Beat(model2::Beat),
    /// An owner heartbeat, to read through the catalog its owner names.
    Owner {
        key: String,
        stamp: wire::Stamp,
        tag: wire::Tag,
        body: Vec<u8>,
    },
}

/// The author's revision as its heartbeats are read: its layout, and the
/// desired resource's collection in it.
#[derive(Clone)]
pub(super) struct Covered {
    layout: Arc<Layout>,
    index: usize,
    class: Class,
    horizon: Duration,
}

pub(super) struct V2 {
    tables: model2::Tables,
    pub(super) core: tokio::sync::Mutex<Core>,
    covered: Option<Covered>,
    /// The catalogs the service holds: what an owner heartbeat's groups
    /// are read through (core §8.6).
    catalogs: Vec<Catalog>,
    pub(super) gets: AtomicU64,
}

pub(super) struct Core {
    pub(super) model: model2::Target,
    /// The running act's document, and the held one's.
    running: Option<Incoming>,
    waiting: Option<Incoming>,
    watch: Option<model2::Watch>,
    /// The watch asked for a read that has not started.
    watch_asked: bool,
    read: u64,
    unread: u64,
    last: Option<wire::Stamp>,
    last_at: Option<Instant>,
    asked: u64,
    said_silent: bool,
    /// The catalog the author names under each run, as its descriptor
    /// said, and when a read that failed may be made again.
    named: BTreeMap<String, (Option<CatalogId>, Option<Instant>)>,
}

impl V2 {
    /// A role's `desired.v2` half: nothing held, and a watch where the
    /// author's desired resource `r` is a collection of its heartbeat.
    pub(super) fn new(
        b: &ServiceBuilder,
        author: &Contract,
        r: &Resource,
        key: Option<&str>,
        tables: model2::Tables,
    ) -> Self {
        let layout = Layout::of(author);
        let covered = layout
            .index_of(&crate::implementation::resource_name(r))
            .map(|index| Covered {
                class: layout.collections[index].class,
                horizon: layout.collections[index].ttl,
                layout: Arc::new(layout.clone()),
                index,
            });
        // §2.5's MAY: the reference moves the digest it expects at a
        // version of its slice heard live.
        let watch = covered
            .as_ref()
            .and(key)
            .map(|k| model2::Watch::new(k, true));
        Self {
            tables,
            covered,
            catalogs: b.catalog_held().into_iter().cloned().collect(),
            gets: AtomicU64::new(0),
            core: tokio::sync::Mutex::new(Core {
                model: model2::Target::new(),
                running: None,
                waiting: None,
                watch,
                watch_asked: false,
                read: 0,
                unread: 0,
                last: None,
                last_at: None,
                asked: 0,
                said_silent: false,
                named: BTreeMap::new(),
            }),
        }
    }

    pub(super) async fn watching(&self) -> Watching {
        let c = self.core.lock().await;
        Watching {
            watches: c.watch.is_some(),
            expects: c.watch.as_ref().and_then(model2::Watch::expects),
            read: c.read,
            unread: c.unread,
            last: c.last,
            asked: c.asked,
            silent: c.watch.is_some() && self.silent(&c),
        }
    }

    /// No heartbeat read within the horizon: an owner sends one at most
    /// every half horizon (`freshness.v2` §2.4).
    fn silent(&self, c: &Core) -> bool {
        let horizon = self.covered.as_ref().map_or(Duration::ZERO, |c| c.horizon);
        c.last_at.is_none_or(|t| t.elapsed() > horizon)
    }
}

/// A heartbeat's section as the watch reads it (§2.5): its digest and its
/// live count. A desired resource has a target parameter, so its
/// collection is templated and its section holds a count.
fn section(s: &zenkey_model::heartbeat::Section) -> model2::Section {
    model2::Section {
        digest: s.digest,
        live: s.members.live().unwrap_or(1),
    }
}

/// One heartbeat on the key covering the author's interface, read with the
/// author's revision by `freshness.v2` §2.5's steps (§2.5, "What it
/// reads"): another form on the key and a refused body are not read, nor
/// is one of another revision.
fn beat_of(c: &Covered, sample: Sample) -> model2::Beat {
    let Ok((sample, stamp, tag)) = crate::inband::admit_heartbeat(crate::inband::read(sample))
    else {
        return model2::Beat::Refused;
    };
    if tag != c.layout.tag {
        return model2::Beat::Revision;
    }
    match Heartbeat::parse(&sample.payload().to_bytes(), &c.layout) {
        Ok(hb) => model2::Beat::Read {
            stamp,
            section: hb
                .sections
                .get(c.index)
                .and_then(Option::as_ref)
                .map(section),
        },
        Err(_) => model2::Beat::Refused,
    }
}

impl TargetInner {
    /// §2.5, "The watch": a subscriber on the heartbeat key covering the
    /// author's interface, and one on its owner heartbeat key of the
    /// desired collection's class where the service holds a catalog (a
    /// target that holds none could read no group, core §8.6). None for a
    /// `desired.v1` author, and for a desired resource with no horizon.
    pub(super) async fn watch_heartbeats(&self) -> Vec<zenoh::pubsub::Subscriber<()>> {
        let mut out = Vec::new();
        let (Some(v2), Some(author)) = (&self.v2, &self.author) else {
            return out;
        };
        let Some(covered) = &v2.covered else {
            return out;
        };
        let iface = covered.layout.iface.clone();
        let (system, service) = (author.system.to_string(), author.service.to_string());
        let key = zenkey_model::heartbeat::key(&system, &service, &iface);
        let (tx, c) = (self.tx.clone(), covered.clone());
        match self
            .session
            .declare_subscriber(key)
            .callback(move |s: Sample| {
                let _ = tx.send(Event::Beat(Heard::Beat(beat_of(&c, s))));
            })
            .await
        {
            Ok(s) => out.push(s),
            Err(e) => {
                tracing::warn!(role = %self.role, "desired.v2: the subscription to the author's heartbeat failed, so the period stands (§2.5): {e}");
            }
        }
        if v2.catalogs.is_empty() {
            return out;
        }
        let key = zenkey_model::heartbeat::owner_key(&system, &service, covered.class);
        let tx = self.tx.clone();
        match self
            .session
            .declare_subscriber(key)
            .callback(move |s: Sample| {
                let key = s.key_expr().as_str().to_owned();
                let heard = match crate::inband::admit_heartbeat(crate::inband::read(s)) {
                    Ok((sample, stamp, tag)) => Heard::Owner {
                        key,
                        stamp,
                        tag,
                        body: sample.payload().to_bytes().into_owned(),
                    },
                    Err(_) => Heard::Beat(model2::Beat::Refused),
                };
                let _ = tx.send(Event::Beat(heard));
            })
            .await
        {
            Ok(s) => out.push(s),
            Err(e) => {
                tracing::warn!(role = %self.role, "desired.v2: the subscription to the author's owner heartbeat failed (§2.5): {e}");
            }
        }
        out
    }

    /// Whether the deployment's period re-reads now (§2.5, "With no
    /// heartbeat"): always for a `desired.v1` author and a target that
    /// does not watch; for one that watches, only while it reads no
    /// heartbeat, which it says once.
    pub(super) async fn period_stands(&self) -> bool {
        let Some(v2) = &self.v2 else { return true };
        let mut c = v2.core.lock().await;
        if c.watch.is_none() {
            return true;
        }
        let silent = v2.silent(&c);
        if silent && !c.said_silent {
            c.said_silent = true;
            tracing::warn!(role = %self.role, "desired.v2: the target was to watch its author's heartbeat and reads none, so its period of re-reads stands (§2.5)");
        }
        if !silent {
            c.said_silent = false;
        }
        silent
    }

    /// One conditional read of the slice (§2.5; core §4.2): S4's GET of
    /// its one concrete key, carrying the stamp `held` where the target
    /// holds a version, and what each reply on that key says.
    async fn get_slice(&self, key: &str, held: Option<wire::Stamp>) -> Got {
        let started = Instant::now();
        let timeout = self.startup.timeout;
        let mut get = self
            .session
            .get(key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(timeout);
        if let Some(stamp) = held {
            get = get
                .payload(wire::HeldStamp::new(stamp).write())
                .encoding(zenoh::bytes::Encoding::ZENOH_BYTES);
        }
        let rx = match get.with(flume::unbounded::<Reply>()).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::warn!(role = %self.role, "desired.v2: the read's GET was refused: {e}");
                return Got::default();
            }
        };
        if let Some(v2) = &self.v2 {
            v2.gets.fetch_add(1, Ordering::Relaxed);
        }
        let mut got = Got::default();
        while let Ok(r) = rx.recv_async().await {
            let s = match r.into_result() {
                // R6: a reply on any key but the slice's is discarded.
                Ok(s) if s.key_expr().as_str() == key => s,
                _ => continue,
            };
            // A held stamp answers only the GET that carried the same one.
            if let Some(h) = crate::inband::held(&s) {
                got.items.push(if Some(h.stamp) == held {
                    Item::Unchanged(h.stamp)
                } else {
                    Item::NotSent(h.stamp)
                });
                continue;
            }
            match crate::inband::read(s.clone()) {
                crate::inband::Read::Snapshot {
                    sample,
                    page: Ok(page),
                } => {
                    let entries: Vec<Incoming> = crate::inband::replies(&sample, &page)
                        .iter()
                        .filter(|m| m.key_expr().as_str() == key)
                        .map(|m| Incoming::of(m, Via::Owner))
                        .collect();
                    if !entries.is_empty() {
                        got.items.extend(entries.into_iter().map(Item::Entry));
                    } else if page.index == 0 && !page.more {
                        // A member's snapshot is one page: with no entry,
                        // the author's word that it holds no document.
                        got.items.push(Item::NoEntry {
                            run: page.run,
                            asof: page.asof,
                        });
                    }
                }
                // A per-key reply (a `redundancy.v1` author's, §2.4), and
                // anything refused, which is never acted on.
                _ => got.items.push(Item::Entry(Incoming::of(&s, Via::Owner))),
            }
        }
        // It ran to its timeout: zenoh ends it there, with an error reply.
        got.timed_out = started.elapsed() + Duration::from_millis(50) >= timeout;
        got
    }

    /// One read of the slice under `desired.v2` (§2.5): a seed, a periodic
    /// re-read or the watch's, conditional while a version is held. As
    /// `desired.v1`'s seed, its GET is made again soon while the author is
    /// present and it drew nothing after an appearance, but never after a
    /// GET that ran to its timeout; and with the author absent by a
    /// complete read, the archive's last-known is read (§2.9).
    pub(super) async fn seed_v2(self: &Arc<Self>, why: Why) {
        let (Some(key), Some(v2)) = (self.key.clone(), self.v2.as_ref()) else {
            return;
        };
        self.seeds.fetch_add(1, Ordering::Relaxed);
        if why.reads_revisions() {
            self.read_revisions().await;
        }
        let mut attempt = 0;
        let mut watch = false;
        let (got, author) = loop {
            // The stamp of the latest version admitted, read at the GET.
            let held = {
                let mut c = v2.core.lock().await;
                watch |= std::mem::take(&mut c.watch_asked);
                c.model.held()
            };
            let got = self.get_slice(&key, held).await;
            if !got.items.is_empty() {
                break (got, AuthorSeen::NotRead);
            }
            let retry = matches!(why, Why::Appeared) && attempt < SEED_RETRIES && !got.timed_out;
            if self.startup.archive.is_none() && !retry {
                break (got, AuthorSeen::NotRead);
            }
            let author = self.author_presence().await;
            if author == AuthorSeen::Present && retry {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                continue;
            }
            break (got, author);
        };
        let answered = got.items.len();
        // One read at a time: the next is made once this one is judged.
        let (done, judged) = tokio::sync::oneshot::channel();
        if self.tx.send(Event::Read(Read { got, watch, done })).is_ok() {
            let _ = judged.await;
        }
        let mut archive = None;
        if answered == 0
            && author == AuthorSeen::Absent
            && let Some((at, timeout)) = &self.startup.archive
        {
            match crate::archive::last_known(&self.session, at, &key, *timeout).await {
                Ok(Some(lk)) => {
                    archive = Some(true);
                    let put = lk.value.is_some();
                    let _ = self.tx.send(Event::Sample(Incoming {
                        via: Via::Archive,
                        put,
                        stamp: lk.timestamp,
                        inband: lk.inband,
                        live: false,
                        payload: lk.value.unwrap_or_default(),
                        encoding: lk.encoding.map(|e| e.to_string()).unwrap_or_default(),
                        refused: None,
                        tag: lk.tag,
                    }));
                }
                Ok(None) => archive = Some(false),
                Err(e) => {
                    archive = Some(false);
                    tracing::warn!(role = %self.role, "desired.v2: the archive at {at} was not read (§2.9): {e}");
                }
            }
        }
        if let AuthorSeen::NotMade(why) = &author {
            tracing::info!(role = %self.role, "desired.v2: the author's presence was not read, so no absence is concluded and no archive read (core §8.1): {why}");
        }
        let revision = self.revision_now().await;
        *self.last_seed.lock().expect("not poisoned") = Some(Seeded {
            answered,
            author,
            archive,
            revision,
        });
    }

    /// The model's sample of one received (§2.4): its version its in-band
    /// stamp, none for a sample with no envelope or a refused one.
    fn sample_v2(&self, i: &Incoming) -> model2::Sample {
        let version = if i.refused.is_some() { None } else { i.inband };
        let (revision, content) = if i.put && i.refused.is_none() {
            (
                self.revision_of(i),
                Some(model::Content::of_payload(&i.payload, self.json)),
            )
        } else {
            (None, None)
        };
        model2::Sample {
            via: i.via,
            mutation: if i.put {
                model::Mutation::Put
            } else {
                model::Mutation::Delete
            },
            version,
            revision,
            content,
        }
    }

    async fn put_report_v2(&self, report: &model2::Report) {
        let Some(w) = &self.report else { return };
        let put = match record(report) {
            Ok(bytes) => w.put(bytes).await.map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = put {
            tracing::warn!(role = %self.role, "desired.v2: the report was not put (§2.10): {e}");
        }
    }

    /// Judges one version (§2.6), heard live or read.
    async fn judge_v2(self: &Arc<Self>, v2: &V2, i: Incoming) {
        let sample = self.sample_v2(&i);
        let mut c = v2.core.lock().await;
        let before = c.model.holds();
        let step = c.model.observe(sample);
        // §2.5, "Its own change costs nothing": a version of its slice
        // heard live and admitted moves the digest the watch expects.
        if i.live
            && matches!(
                step,
                model2::Step::Act(_) | model2::Step::Adopted | model2::Step::Held
            )
            && let Some(now) = c.model.holds()
            && let Some(w) = c.watch.as_mut()
        {
            w.heard(before, now);
        }
        match step {
            model2::Step::Act(act) => {
                c.running = Some(i);
                let doc = c.running.clone();
                drop(c);
                self.act_v2(act, doc);
            }
            model2::Step::Held => c.waiting = Some(i),
            model2::Step::Adopted => {
                let report = c.model.report();
                drop(c);
                self.put_report_v2(&report).await;
            }
            model2::Step::Confirmed => {
                // §2.9: put now unless an act runs, whose end puts it.
                if c.model.running().is_none() {
                    let report = c.model.report();
                    drop(c);
                    self.put_report_v2(&report).await;
                }
            }
            model2::Step::Unstamped => {
                tracing::warn!(role = %self.role, "desired.v2: a sample of the slice with no in-band stamp is never acted on (§2.6; its author broke core S1)");
            }
            model2::Step::SameVersion
            | model2::Step::Older
            | model2::Step::Done
            | model2::Step::NoVerdict => {}
        }
    }

    pub(super) async fn handle_v2(self: &Arc<Self>, ev: Event) {
        let Some(v2) = &self.v2 else { return };
        match ev {
            Event::Seed(why) => self.seed_soon(why),
            Event::HoldDue(run, id) => self.hold_due(&run, id).await,
            Event::Ended(_) => {}
            Event::Called => {
                let report = {
                    let mut c = v2.core.lock().await;
                    c.model.call();
                    c.model.report()
                };
                self.put_report_v2(&report).await;
            }
            Event::Sample(i) => {
                // §2.18: never a version under a run whose revision the
                // role's report cannot report.
                if self.admits(i.inband.map(|s| s.run), Some(&i)).await {
                    self.judge_v2(v2, i).await;
                }
            }
            Event::Read(read) => {
                let (mut whole, mut unchanged) = (false, false);
                for item in read.got.items {
                    // §2.18: nothing under a run whose revision the role's
                    // report cannot report is judged, nor confirmed.
                    // An entry under a run no descriptor states yet waits
                    // with that run's versions; the seed that follows its
                    // release reads the slice again for the rest.
                    let (run, held) = match &item {
                        Item::Entry(i) => (i.inband.map(|s| s.run), Some(i)),
                        Item::NoEntry { run, .. } => (Some(*run), None),
                        Item::Unchanged(s) | Item::NotSent(s) => (Some(s.run), None),
                    };
                    if !matches!(item, Item::NotSent(_)) && !self.admits(run, held).await {
                        continue;
                    }
                    match item {
                        Item::Entry(i) => {
                            whole = true;
                            self.judge_v2(v2, i).await;
                        }
                        Item::NoEntry { run, asof } => {
                            whole = true;
                            let mut c = v2.core.lock().await;
                            match c.model.observe_no_entry(run, asof) {
                                model2::Step::Act(act) => {
                                    c.running = None;
                                    drop(c);
                                    self.act_v2(act, None);
                                }
                                model2::Step::Held => c.waiting = None,
                                model2::Step::NoVerdict => {
                                    tracing::debug!(role = %self.role, %run, "desired.v2: a snapshot with no entry under a run the target holds no document's version of is no verdict (§2.8)");
                                }
                                _ => {}
                            }
                        }
                        Item::Unchanged(sent) => {
                            unchanged = true;
                            let mut c = v2.core.lock().await;
                            // §2.9: the author's own word of the version.
                            if c.model.unchanged(sent) == model2::Step::Confirmed
                                && c.model.running().is_none()
                            {
                                let report = c.model.report();
                                drop(c);
                                self.put_report_v2(&report).await;
                            }
                        }
                        Item::NotSent(stamp) => {
                            tracing::debug!(role = %self.role, stamp = %model2::version_text(stamp), "desired.v2: a held stamp the target did not send is no answer (core §4.2, held)");
                        }
                    }
                }
                if read.watch {
                    let answer = if whole {
                        model2::Answered::Whole
                    } else if unchanged {
                        model2::Answered::Unchanged
                    } else {
                        model2::Answered::NoAnswer
                    };
                    let mut c = v2.core.lock().await;
                    let holds = c.model.held();
                    if let Some(w) = c.watch.as_mut() {
                        w.answer(answer, holds);
                    }
                }
                let _ = read.done.send(());
            }
            Event::EndedV2(ended) => {
                let mut c = v2.core.lock().await;
                let step = c.model.finish(ended);
                c.running = None;
                let next = match step {
                    model2::Step::Act(act) => {
                        c.running = c.waiting.take();
                        Some((act, c.running.clone()))
                    }
                    _ => {
                        c.waiting = None;
                        None
                    }
                };
                let report = c.model.report();
                drop(c);
                self.put_report_v2(&report).await;
                if let Some((act, doc)) = next {
                    self.act_v2(act, doc);
                }
            }
            Event::Beat(heard) => {
                let beat = match heard {
                    Heard::Beat(b) => b,
                    Heard::Owner {
                        key,
                        stamp,
                        tag,
                        body,
                    } => self.owner_beat(v2, &key, stamp, tag, &body).await,
                };
                let mut c = v2.core.lock().await;
                let holds = c.model.held();
                let Some(w) = c.watch.as_mut() else { return };
                let decision = w.heartbeat(beat, holds);
                match (beat, decision) {
                    (_, model2::Decision::Revision | model2::Decision::Refused) => c.unread += 1,
                    (_, model2::Decision::Older) => {}
                    (model2::Beat::Read { stamp, .. }, _) => {
                        c.read += 1;
                        c.last = Some(stamp);
                        c.last_at = Some(Instant::now());
                    }
                    _ => {}
                }
                if let model2::Decision::Read { .. } = decision {
                    if self.seeding.load(Ordering::Acquire) {
                        // §2.5 step 6: a read of the slice is outstanding,
                        // a seed's or a period's, so it makes none. What it
                        // expects does not change, and the next heartbeat
                        // is its next trigger: the watch is told so as of a
                        // read that drew no answer.
                        if let Some(w) = c.watch.as_mut() {
                            w.answer(model2::Answered::NoAnswer, holds);
                        }
                    } else {
                        c.watch_asked = true;
                        c.asked += 1;
                        drop(c);
                        self.seed_soon(Why::Watch);
                    }
                }
            }
        }
    }

    /// One owner heartbeat as the watch reads it (§2.5, "What it reads"):
    /// the group for the author's interface, found through the catalog its
    /// owner names under the put's run, read as a heartbeat is. A group
    /// whose index is not read, or whose sections are not, is a heartbeat
    /// not read. One with no group for the author's interface has no
    /// section.
    async fn owner_beat(
        &self,
        v2: &V2,
        key: &str,
        stamp: wire::Stamp,
        tag: wire::Tag,
        body: &[u8],
    ) -> model2::Beat {
        let Some(covered) = &v2.covered else {
            return model2::Beat::Refused;
        };
        let Ok(hb) = OwnerHeartbeat::parse(tag, body) else {
            return model2::Beat::Refused;
        };
        // The catalog its owner names: its descriptor stating the run,
        // read once a run within the target's timeout, at least the face's
        // across one (`TargetStartup::across`; core §8.1, #1146); the
        // deployment's word, the catalog the service was given, where no
        // descriptor reaches the target (core §8.6).
        let run = stamp.run.to_string();
        let known = v2.core.lock().await.named.get(&run).copied();
        let named = match known {
            Some((named, retry)) if retry.is_none_or(|at| Instant::now() < at) => named,
            _ => {
                let (named, retry) = match crate::answer::owner_catalog_held(
                    &self.session,
                    key,
                    stamp.run,
                    &self.held,
                    self.startup.timeout,
                )
                .await
                {
                    crate::answer::OwnerCatalog::Named(c) => (c, None),
                    crate::answer::OwnerCatalog::Unreached => {
                        (v2.catalogs.last().map(Catalog::id), None)
                    }
                    crate::answer::OwnerCatalog::Unread => {
                        (None, Some(Instant::now() + CATALOG_RETRY))
                    }
                };
                v2.core.lock().await.named.insert(run, (named, retry));
                named
            }
        };
        let layout = &covered.layout;
        let mut unread = false;
        for g in &hb.groups {
            let entry = match g.entry(&v2.catalogs, named) {
                Ok(e) => e,
                Err(_) => {
                    // It may be the author's interface's.
                    unread = true;
                    continue;
                }
            };
            if entry.iface != layout.iface {
                continue;
            }
            if entry.contract != layout.fingerprint || !g.tag_is(layout) {
                return model2::Beat::Revision;
            }
            return match g.read(layout) {
                Ok(sections) => model2::Beat::Read {
                    stamp,
                    section: sections
                        .get(covered.index)
                        .and_then(Option::as_ref)
                        .map(section),
                },
                Err(_) => model2::Beat::Refused,
            };
        }
        if unread {
            model2::Beat::Refused
        } else {
            model2::Beat::Read {
                stamp,
                section: None,
            }
        }
    }

    /// Starts one act on a task of its own (§2.7); its end comes back as
    /// an event.
    fn act_v2(self: &Arc<Self>, act: model2::Act, doc: Option<Incoming>) {
        self.acts.fetch_add(1, Ordering::Relaxed);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let ended = me.perform_v2(act, doc).await;
            let _ = me.tx.send(Event::EndedV2(ended));
        });
    }

    /// A handler's refusal as this major's report carries it (§2.10): its
    /// code, and its place and its line as indices into the report
    /// resource's two tables. A place or a line the tables do not hold is
    /// `out_of_range`, and its words stay here, in the log.
    fn indexed(&self, r: &Refusal) -> model2::Refusal {
        let tables = self.v2.as_ref().map(|v| &v.tables);
        let pointer = r
            .pointer
            .as_deref()
            .map(|p| tables.map_or(model2::Index::OutOfRange, |t| t.place(p)));
        let detail = r
            .detail
            .as_deref()
            .map(|d| tables.map_or(model2::Index::OutOfRange, |t| t.line(d)));
        if pointer == Some(model2::Index::OutOfRange) || detail == Some(model2::Index::OutOfRange) {
            tracing::warn!(role = %self.role, code = %r.code, pointer = ?r.pointer, detail = ?r.detail, "desired.v2: a refusal's place or line is not in the report resource's tables, so the report says out_of_range and its words stay here (§2.10)");
        }
        model2::Refusal {
            code: model2::Code::of(&r.code),
            pointer,
            detail,
        }
    }

    fn ended_v2(&self, ended: Ended) -> model2::Ended {
        match ended {
            Ended::Done => model2::Ended::Done,
            Ended::Refused(r) => model2::Ended::Refused(self.indexed(&r)),
            Ended::Failed(r) => model2::Ended::Failed(self.indexed(&r)),
        }
    }

    async fn perform_v2(&self, act: model2::Act, doc: Option<Incoming>) -> model2::Ended {
        let version = act.version();
        let shown = version.map(model2::version_text).unwrap_or_default();
        let handler = Arc::clone(&self.handler);
        let lost = |e: tokio::task::JoinError| {
            let mut r = Refusal::new(APPLY);
            r.detail = Some(format!(
                "the handler's act did not complete, so what it applied is unknown: {e}"
            ));
            Ended::Failed(r)
        };
        let ended = match act.kind {
            model::ActKind::Revert => {
                tracing::info!(role = %self.role, version = %shown, "desired.v2: revert (§2.8)");
                tokio::spawn(async move { handler.revert_v2(version.as_ref()).await })
                    .await
                    .unwrap_or_else(lost)
            }
            model::ActKind::Apply => {
                let Some(i) = doc else {
                    return model2::Ended::Failed(model2::Refusal::new(model2::Code::Apply));
                };
                tracing::info!(role = %self.role, version = %shown, revision = ?act.revision, "desired.v2: act (§2.7)");
                // §2.4: a revision above the bound is refused whole,
                // `invalid`, before the handler sees the document.
                if act.bound().is_some() {
                    return model2::Ended::Refused(self.indexed(&Refusal {
                        code: INVALID.to_owned(),
                        pointer: Some(REVISION_POINTER.to_owned()),
                        detail: Some(REVISION_LINE.to_owned()),
                    }));
                }
                let value = match self.decoded(&i) {
                    Ok(v) => v,
                    Err(why) => {
                        let mut r = Refusal::new(UNDECODABLE);
                        r.detail = Some(why);
                        return model2::Ended::Refused(self.indexed(&r));
                    }
                };
                if let (Some(v), Some(bundle), Some(ty)) = (&value, &self.bundle, &self.ty)
                    && ty["kind"] == "jsonschema"
                    && let Err(violations) = zenkey_model::validate::validate(bundle, ty, v)
                {
                    let first = &violations[0];
                    let mut r = Refusal::new(INVALID);
                    r.pointer = Some(first.at.clone());
                    r.detail = Some(first.why.clone());
                    return model2::Ended::Refused(self.indexed(&r));
                }
                let doc = Document {
                    key: self.key.clone().unwrap_or_default(),
                    version: Version {
                        stamp: NO_STAMP.stamp,
                        revision: act.revision,
                    },
                    inband: version,
                    last_known: act.via == Via::Archive,
                    payload: i.payload,
                    encoding: i.encoding,
                    value,
                };
                tokio::spawn(async move { handler.apply(&doc).await })
                    .await
                    .unwrap_or_else(lost)
            }
        };
        self.ended_v2(ended)
    }
}
