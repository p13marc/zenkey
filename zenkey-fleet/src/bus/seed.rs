//! A late joiner's seed — core §4.2 (0.26) as an engine helper (#614, FL4;
//! issue #42 before it, which carried v1's seed).
//!
//! A pane that shows state wants what is there now and every change after
//! it. The rule is normative and subtle, and every state-showing pane would
//! otherwise reimplement (or skip) it:
//!
//! - **the subscription is declared first**, and the seed GETs after it: in
//!   the other order a mutation the owner makes after it answers and before
//!   the subscription reaches it is lost, and a lost delete leaves a
//!   deleted key standing;
//! - **the state leg is S4's GET** on the watched selector, target `All`
//!   and consolidation `None` set explicitly
//!   (`bus::query::state_get`, the crate's one state GET): addressed to the owners' keys,
//!   since each key it matches has one owner (P3), and answered for a
//!   foreign key by whatever holds it. A storage on an owner's state is a
//!   deployment fault (S4), never this seed's source. A snapshot's page
//!   (core §7.5's form 7) is its entries, each the per-key reply it stands
//!   for, merged as replies are, and the leg says which snapshots it heard
//!   and whether each is complete ([`SeedLeg::snapshots`]). A page heard
//!   live is no owner's answer: delivered undecodable, it moves nothing;
//! - **the omission rule** (core §4.2; #977's E4) runs where the tool holds
//!   contracts ([`seed_subscribe_with`], [`crate::Monitor::watch_seeded_with`]:
//!   those given offline and those retrieved by fingerprint, a
//!   [`BundleStore`]). A complete snapshot maps its index with the revision
//!   its owner's descriptor stating the snapshot's run names (F-497),
//!   retrieved where it is not held; with no descriptor reaching the tool,
//!   as across a constrained face, the one revision of the interface the
//!   tool holds is the deployment's word; a descriptor read that failed,
//!   or no such revision, applies none. A key at or under the snapshot's
//!   key that it lists neither live nor as a tombstone is dropped from the
//!   merge and delivered as a deletion with no stamp, unless what the merge
//!   holds is newer: under the snapshot's run above its asof, or under
//!   another run heard after the GET was asked, by either path, a mutation
//!   heard again included (`Merge::ask`). An incomplete snapshot drops
//!   nothing, and the leg reads its key again once, the SHOULD, when its
//!   GET ran to its final reply. A `reply_del` on a member of a collection
//!   the owner's revision answers by snapshot is no owner's answer, and is
//!   neither merged nor delivered (core S2, S3). Without contracts the seed
//!   maps no index and applies no omission rule (observer §4.1, "Without
//!   the contract"): a key a complete snapshot left out stays as it was
//!   held, possibly stale;
//! - **the history leg** asks the `@adv` caches of the advanced publishers
//!   present (`<selector>/@adv/**`, spec §2.5), for resources that declare
//!   `history` — streams among them, which no state GET reaches. Under a
//!   verbatim chunk a late *publisher* is never found (§2.5), and a
//!   publisher that is gone took its cache with it;
//! - **replies and live samples merge per key by the owner's in-band
//!   stamp** (core §4.3; one merge for both phases, kept for as long as
//!   the subscription follows: observer §4.1): the newer wins whichever
//!   path brought it, the later arrival across two runs, a
//!   `reply_del` is a deletion with its stamp, and one mutation both paths
//!   carried is kept once. The merge holds one entry per key, bounded at
//!   [`crate::DEFAULT_MAX_KEYS`] by last admission; a key it forgets holds
//!   nothing, so its next sample is kept whatever its stamp, and every key
//!   forgotten is counted ([`SeededSubscriber::evicted`]), never silent
//!   (tooling guide O6).
//!
//! Both legs are queries **this module issues itself**, rather than
//! zenoh-ext's `history()` replay: the boundary ([`SeedItem::SeedComplete`])
//! must not fire until every leg has resolved, and only a query we own has
//! an awaitable end. Coverage is **reported, never assumed**:
//! [`SeedCoverage`] says which legs ran, what each yielded and whether it
//! ran to its final reply, and its zeros are observations, not verdicts.
//! An owner absent at the GET seeds nothing for its keys (O5's silence,
//! never "no state"); its keys appear as they change.
//!
//! No events leg: an occurrence is replayed from union storages, bounded by
//! its retention (§2.6), and a seed of state does not reach it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Result;
use zenoh::Session;

use crate::bus::contracts::BundleStore;
use crate::bus::monitor::SampleView;
use crate::model::bounded::{BoundedLru, DEFAULT_MAX_KEYS};
use crate::report::{Asked, SeedCoverage, SeedLeg};

/// Which seed legs to run. Default: both — a per-leg opt-out exists because
/// a pane may *know* its selector holds no state (or no advanced
/// publishers), not because skipping is free.
#[derive(Debug, Clone, Copy)]
pub struct SeedPolicy {
    /// Ask the `@adv` caches of the advanced publishers present
    /// (`<selector>/@adv/**`, spec §2.5): what a resource declaring
    /// `history` keeps, streams included. Reaches only publishers that are
    /// alive.
    pub history: bool,
    /// S4's GET on the selector (core §4.2): every owner's current state
    /// under it, its deletions within the window as `reply_del`s, and
    /// whatever answers a foreign key.
    pub state: bool,
    /// Bound on each seed GET (they run concurrently, so this bounds the
    /// whole seed phase too). A GET that reaches it seeds possibly
    /// incompletely, and its leg says so ([`SeedLeg::complete`]).
    pub timeout: Duration,
}

impl Default for SeedPolicy {
    fn default() -> Self {
        SeedPolicy {
            history: true,
            state: true,
            timeout: Duration::from_secs(3),
        }
    }
}

/// One delivery from a seeded subscription.
#[derive(Debug, Clone)]
pub enum SeedItem {
    /// A sample that survived the per-key LWW merge — seed and live alike.
    Sample(SampleView),
    /// How many samples this consumer just missed: the delivery channel is
    /// bounded, and a receiver that fell behind is told the count rather
    /// than handed a silently thinned stream (tooling guide O6 — the mirror
    /// of [`crate::StreamItem::Dropped`]). Merge suppressions are *not* in
    /// this number; they ride [`SeedCoverage::superseded`].
    Dropped(u64),
    /// Both seed paths have resolved; everything after this is live-only.
    /// Consumers that render "loading" state key off this boundary. Never
    /// dropped: the boundary is sent with backpressure, not best-effort.
    SeedComplete(SeedCoverage),
}

/// The delivery channel's bound — the same figure as the monitor's default
/// broadcast capacity ([`crate::MonitorSpec::default`]), for the same
/// reason: bound it to what a consumer can drain, and surface the lag.
const SEED_CAPACITY: usize = 1024;

/// The sending half of the bounded seed channel: samples are best-effort
/// (`try_send`) with every refusal counted, so a slow consumer costs a
/// stated drop, never unbounded memory (deep-review D5).
#[derive(Clone)]
struct SeedSender {
    tx: tokio::sync::mpsc::Sender<SeedItem>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl SeedSender {
    fn send_sample(&self, view: SampleView) {
        use tokio::sync::mpsc::error::TrySendError;
        match self.tx.try_send(SeedItem::Sample(view)) {
            Ok(()) => {}
            // The bound refused it: count the drop (O6).
            Err(TrySendError::Full(_)) => {
                self.dropped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // No receiver any more — nothing is observing, nothing to count.
            Err(TrySendError::Closed(_)) => {}
        }
    }

    /// The boundary, with backpressure: waits for room rather than dropping
    /// — a lost `SeedComplete` would leave every consumer "loading" forever.
    async fn send_boundary(&self, coverage: SeedCoverage) {
        let _ = self.tx.send(SeedItem::SeedComplete(coverage)).await;
    }
}

/// The receiving half: surfaces the accumulated drop count as a
/// [`SeedItem::Dropped`] before the next item, like the monitor's lagging
/// broadcast receiver does.
struct SeedReceiver {
    rx: tokio::sync::mpsc::Receiver<SeedItem>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl SeedReceiver {
    async fn recv(&mut self) -> Option<SeedItem> {
        let missed = self.dropped.swap(0, std::sync::atomic::Ordering::Relaxed);
        if missed > 0 {
            return Some(SeedItem::Dropped(missed));
        }
        self.rx.recv().await
    }
}

/// The bounded seed channel, drop-accounted on both halves.
fn seed_channel(capacity: usize) -> (SeedSender, SeedReceiver) {
    let (tx, rx) = tokio::sync::mpsc::channel::<SeedItem>(capacity);
    let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
    (
        SeedSender {
            tx,
            dropped: Arc::clone(&dropped),
        },
        SeedReceiver { rx, dropped },
    )
}

/// A subscription whose first phase is a correctly-merged seed.
pub struct SeededSubscriber {
    rx: SeedReceiver,
    // Held for lifetime: dropping undeclares.
    _subscriber: zenoh::pubsub::Subscriber<()>,
    task: tokio::task::JoinHandle<()>,
    merge: Arc<Merge>,
}

impl std::fmt::Debug for SeededSubscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeededSubscriber").finish_non_exhaustive()
    }
}

impl Drop for SeededSubscriber {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SeededSubscriber {
    /// `None` when the subscription ended. A consumer that fell behind the
    /// bounded channel is handed [`SeedItem::Dropped`] with the count of
    /// samples it missed before the stream resumes (O6).
    pub async fn recv(&mut self) -> Option<SeedItem> {
        self.rx.recv().await
    }

    /// Samples the merge did not keep, over the subscription's whole life:
    /// the seed's, which the boundary's [`SeedCoverage::superseded`] counts,
    /// and every live one after it older than what its key held.
    pub fn superseded(&self) -> u64 {
        self.merge.superseded()
    }

    /// Times the merge forgot a key to stay within its bound
    /// ([`crate::DEFAULT_MAX_KEYS`]), each one's next sample then kept
    /// whatever its stamp (observer §4.1, tooling guide O6).
    pub fn evicted(&self) -> u64 {
        self.merge.evicted()
    }
}

/// The subscription **is** a stream (#343) — the seed phase and the live
/// phase are one sequence, which is the whole point of the type.
///
/// A direct impl rather than an adapter, because the channel underneath is an
/// `mpsc::Receiver` with a real `poll_recv`: no boxing, no self-reference, and
/// the [`SeedItem::Dropped`] preamble is the same one [`recv`](
/// SeededSubscriber::recv) applies, so a consumer that switched from `recv`
/// to `next` sees identical items in an identical order.
impl futures_core::Stream for SeededSubscriber {
    type Item = SeedItem;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<SeedItem>> {
        // Every field is `Unpin`, so the projection needs no unsafe — worth
        // stating because this type has a `Drop` impl, which is what stops
        // the derive from being available.
        let this = self.get_mut();
        // What the bounded channel refused, before what it kept (O6).
        let missed = this
            .rx
            .dropped
            .swap(0, std::sync::atomic::Ordering::Relaxed);
        if missed > 0 {
            return std::task::Poll::Ready(Some(SeedItem::Dropped(missed)));
        }
        this.rx.rx.poll_recv(cx)
    }
}

/// The late joiner's merge (core §4.2, §4.3): one entry per key, and a
/// sample is admitted only when its stamp is newer than the one held,
/// whichever path — reply or live — brought either. The decision is
/// `zenkey_model::seed`'s pure functions, which
/// `spec/conformance/seed/inband.json` and `merge.json` pin:
///
/// - **In band first.** An owner's sample carries a zk2c envelope (core
///   §7.5), ordered by its run and seq (below).
/// - **A sample with no envelope** (an `@stream` sample, a foreign key's,
///   or one that broke §4.3) is ordered by its Zenoh timestamp, as zenoh's
///   `Latest` compares replies (§4.1): by time; on equal times by the id's
///   bytes, little-endian, as zenoh's `Timestamp` compares them
///   ([`crate::model::inband::stamp_of`]), across two ids too, as a tool
///   keeps it (observer §4.1).
/// - Equal stamps are one mutation: the second copy is suppressed.
/// - A deletion (a live delete, or a `reply_del` with the deletion's stamp,
///   S3) is a sample like any other, and merges the same way: never
///   dropped, never mistaken for an absence.
/// - Stamped beats unstamped; unstamped against unstamped passes through,
///   the later arrival replacing the earlier. An owner stamps every
///   mutation in band (S1), so a sample with neither stamp is no
///   conforming owner's: an `@stream` sample's, or a foreign key's, from a
///   session with no HLC through no router that stamps, ranked below every
///   stamp as `Latest` ranks an unstamped reply.
///
/// - A sample carrying a zk2c envelope (core §7.5) is ordered by its run
///   and seq ([`SampleView::mark`], `zenkey_model::seed::admits_mark_in`):
///   within one run the newer seq is kept and an equal seq is the same
///   value, kept once, a state's seqs compared as integers and a stream
///   key's by serial order, across its wrap (core §4.3); across two runs,
///   or beside a Zenoh-stamped value, nothing orders them and the later
///   arrival is kept. A refused envelope is delivered undecodable and
///   moves nothing.
///
/// Every suppression is counted ([`SeedCoverage::superseded`] at the
/// boundary, [`Merge::superseded`] for the merge's life).
///
/// **Kept for as long as the reading follows** (observer §4.1): a tool's
/// merge spans the seed and every live sample after it, so a live sample
/// older than what its key holds within its run is not kept after the
/// boundary either. A value under another run, which nothing orders
/// against the held one, is kept as the later arrival (observer §4.1), an
/// old run's value still in transit included (core §4.3's residual risk).
///
/// **Bounded** (tooling guide O6): one entry per key, at most `max_keys`,
/// the least recently admitted forgotten first, in batches
/// ([`BoundedLru`]). A forgotten key holds nothing, so its next sample is
/// kept whatever its stamp, and each time it forgets a key is counted
/// ([`Merge::evicted`]), apart from `superseded` and from a channel's
/// `Dropped(n)`.
///
/// `pub(crate)`: [`crate::Monitor::watch_seeded`] runs the same merge
/// (issue #92) — one discipline, not two.
pub(crate) struct Merge {
    latest: Mutex<MergeTable>,
    superseded: std::sync::atomic::AtomicU64,
    evicted: std::sync::atomic::AtomicU64,
    /// A monitor's totals across its seeded watches, bumped beside this
    /// merge's own.
    ledger: Option<Arc<MergeLedger>>,
}

/// One merge's keys, the count of its admissions, which orders them for
/// the bound, and its ticks: one at each hearing and at each GET asked,
/// what "heard after it asked the GET" compares (core §4.2).
struct MergeTable {
    keys: BoundedLru<String, Entry>,
    admitted: u64,
    ticks: u64,
}

/// What one key holds, when it was last admitted, when what it holds was
/// last heard (the same mutation heard again included), and whether it
/// is a deletion.
#[derive(Debug, Clone, Copy)]
struct Entry {
    held: zenkey_model::seed::Held,
    admitted: u64,
    heard: u64,
    deleted: bool,
}

/// What every merge of a monitor's seeded watches did not keep and forgot
/// ([`crate::Monitor::merge_superseded`], [`crate::Monitor::merge_evicted`]).
#[derive(Debug, Default)]
pub(crate) struct MergeLedger {
    pub(crate) superseded: std::sync::atomic::AtomicU64,
    pub(crate) evicted: std::sync::atomic::AtomicU64,
}

impl Merge {
    /// A merge bounded at `max_keys` keys, its counts also added to
    /// `ledger` when one is given.
    pub(crate) fn new(max_keys: usize, ledger: Option<Arc<MergeLedger>>) -> Merge {
        Merge {
            latest: Mutex::new(MergeTable {
                keys: BoundedLru::with_capacity(max_keys),
                admitted: 0,
                ticks: 0,
            }),
            superseded: std::sync::atomic::AtomicU64::new(0),
            evicted: std::sync::atomic::AtomicU64::new(0),
            ledger,
        }
    }

    pub(crate) fn superseded(&self) -> u64 {
        self.superseded.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn evicted(&self) -> u64 {
        self.evicted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether `view` is kept over what its key holds, by its mark: an
    /// in-band stamp within one run by seq, plain for a state's and serial
    /// for a stream key's (core §4.3, `zenkey_model::seed::Order::of_key`),
    /// across runs (or two wires) the later arrival, a Zenoh timestamp by
    /// zenoh's order (`zenkey_model::seed::admits_mark_in`, core §7.5;
    /// observer §4.1). A refused envelope
    /// is never ordered: it is delivered, so a reader shows it
    /// undecodable, and moves nothing its key holds.
    pub(crate) fn admit(&self, view: &SampleView) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        if view.refusal().is_some() {
            return true;
        }
        let incoming = view.mark();
        let mut table = self.latest.lock().expect("merge lock");
        table.ticks += 1;
        let tick = table.ticks;
        let held = table.keys.get(view.key.as_str()).map(|e| e.held);
        let order = zenkey_model::seed::Order::of_key(&view.key);
        let admit = zenkey_model::seed::admits_mark_in(order, held.unwrap_or_default(), incoming);
        if !admit {
            // The same mutation heard again: heard now (core §4.2).
            if let (Some(h), Some(i)) = (held.and_then(zenkey_model::seed::Held::mark), incoming)
                && zenkey_model::seed::versus_in(order, h, i) == zenkey_model::seed::Versus::Same
                && let Some(e) = table.keys.get_mut(view.key.as_str())
            {
                e.heard = tick;
            }
            self.superseded.fetch_add(1, Relaxed);
            if let Some(l) = &self.ledger {
                l.superseded.fetch_add(1, Relaxed);
            }
            return false;
        }
        if held.is_none() {
            let forgot = table.keys.admit(|e| e.admitted) as u64;
            if forgot > 0 {
                self.evicted.fetch_add(forgot, Relaxed);
                if let Some(l) = &self.ledger {
                    l.evicted.fetch_add(forgot, Relaxed);
                }
            }
        }
        table.admitted += 1;
        let entry = Entry {
            held: held.unwrap_or_default().after_mark(incoming),
            admitted: table.admitted,
            heard: tick,
            deleted: view.kind == zenoh::sample::SampleKind::Delete,
        };
        table.keys.insert(view.key.clone(), entry);
        true
    }

    /// A tick for a GET asked now: what the merge hears after it holds a
    /// later one (core §4.2's guard, "heard after it asked the GET").
    pub(crate) fn ask(&self) -> u64 {
        let mut table = self.latest.lock().expect("merge lock");
        table.ticks += 1;
        table.ticks
    }

    /// What the merge holds at `key` or under it, for the omission rule:
    /// each key with the mark of what it holds and whether that was heard
    /// after the GET asked at `asked`.
    fn held_under(
        &self,
        key: &str,
        asked: u64,
    ) -> Vec<(String, Option<zenkey_model::seed::Mark>, bool)> {
        let table = self.latest.lock().expect("merge lock");
        table
            .keys
            .iter()
            .filter(|(k, _)| at_or_under(k, key))
            .map(|(k, e)| (k.clone(), e.held.mark(), e.heard > asked))
            .collect()
    }

    /// The omission rule over what the merge holds at `key` or under it
    /// (core §4.2): `omit` names the keys to drop from each key's mark and
    /// whether it was heard after the GET asked at `asked`, and they are
    /// forgotten, under one lock, so that nothing heard meanwhile is lost.
    /// Returns those that held a value, whose reader is then told it is
    /// gone; a deletion forgotten tells nothing.
    fn drop_omitted(
        &self,
        key: &str,
        asked: u64,
        omit: impl FnOnce(&[(String, Option<zenkey_model::seed::Mark>, bool)]) -> Vec<String>,
    ) -> Vec<String> {
        let mut table = self.latest.lock().expect("merge lock");
        let held: Vec<_> = table
            .keys
            .iter()
            .filter(|(k, _)| at_or_under(k, key))
            .map(|(k, e)| (k.clone(), e.held.mark(), e.heard > asked))
            .collect();
        omit(&held)
            .into_iter()
            .filter(|k| table.keys.remove(k.as_str()).is_some_and(|e| !e.deleted))
            .collect()
    }
}

/// Whether `k` is `key` or a key under it.
fn at_or_under(k: &str, key: &str) -> bool {
    k == key
        || k.strip_prefix(key)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(crate) fn view_of(sample: &zenoh::sample::Sample) -> SampleView {
    SampleView::of(sample)
}

/// Which GET a leg makes.
#[derive(Debug, Clone, Copy)]
enum Leg {
    History,
    State,
}

/// Run one leg's GET: every reply passes the merge, and admitted samples go
/// to `deliver`. A GET that could not be put on the bus seeded nothing and
/// is not complete; an error reply (a timeout's included) is counted out of
/// the replies and makes the leg incomplete. The state leg reads each
/// snapshot's page as its entries, applies the omission rule over a
/// complete snapshot where `contracts` are held, and reads a snapshot it
/// holds incomplete again once ([`StateRead`]).
async fn run_leg(
    session: &Session,
    selector: &str,
    leg: Leg,
    timeout: Duration,
    merge: &Merge,
    contracts: Option<&BundleStore>,
    deliver: &(impl Fn(SampleView) + Sync),
) -> SeedLeg {
    let opts = crate::bus::query::GetOpts::new(timeout);
    match leg {
        // `accept_any`: cache replies arrive on the sample's own key, outside
        // an `@adv`-suffixed selector — without it they are dropped.
        Leg::History => {
            let replies = crate::bus::query::disciplined_get(
                session,
                &cache_selector(selector),
                &opts.accept_any(),
            )
            .await;
            let Ok(replies) = replies else {
                return SeedLeg {
                    replies: 0,
                    complete: false,
                    snapshots: Vec::new(),
                };
            };
            let mut out = SeedLeg {
                replies: 0,
                complete: true,
                snapshots: Vec::new(),
            };
            while let Ok(reply) = replies.recv_async().await {
                let Ok(sample) = reply.result() else {
                    out.complete = false;
                    continue;
                };
                out.replies += 1;
                let view = view_of(sample);
                if merge.admit(&view) {
                    deliver(view);
                }
            }
            out
        }
        Leg::State => {
            let mut read = StateRead {
                session,
                timeout,
                merge,
                revisions: contracts.map(Revisions::new),
                heard: Vec::new(),
                pages: Vec::new(),
                replies: 0,
                complete: true,
            };
            let asked = merge.ask();
            read.get(selector, asked, deliver).await;
            // An incomplete snapshot drops nothing, and a late joiner
            // SHOULD read the collection again (core §4.2): once, where
            // the GET ran to its final reply, since a GET that reached its
            // timeout would likely reach it again. A collection read is
            // read again whole, a member read on its member's key.
            if read.complete {
                let again: std::collections::BTreeSet<String> = read
                    .heard
                    .iter()
                    .filter(|h| !read.settled(h))
                    .map(|h| h.key.clone())
                    .collect();
                for key in again {
                    let whole = format!("{key}/**");
                    let includes = zenoh::key_expr::KeyExpr::try_from(selector).is_ok_and(|s| {
                        zenoh::key_expr::KeyExpr::try_from(whole.as_str())
                            .is_ok_and(|w| s.includes(&w))
                    });
                    let again = if includes { whole } else { key };
                    let asked = merge.ask();
                    read.get(&again, asked, deliver).await;
                }
            }
            let settled = read.heard.iter().all(|h| read.settled(h));
            SeedLeg {
                replies: read.replies,
                complete: read.complete && settled,
                snapshots: read
                    .heard
                    .iter()
                    .map(crate::report::CollectionSnapshot::from)
                    .collect(),
            }
        }
    }
}

/// The state leg's reading of its GETs (core §4.2; #977's E3, E4): the
/// snapshots it heard and each one's pages, what it read with, and how it
/// went.
struct StateRead<'a> {
    session: &'a Session,
    timeout: Duration,
    merge: &'a Merge,
    /// The contracts held, where the tool holds them: the omission rule
    /// and a stray `reply_del` read with them.
    revisions: Option<Revisions<'a>>,
    heard: Vec<zenkey_model::collection::Heard>,
    pages: Vec<Vec<zenkey_model::collection::Page>>,
    replies: usize,
    complete: bool,
}

impl StateRead<'_> {
    /// Whether `h` is complete, or a later snapshot on its key is: an
    /// incomplete one read again stands settled by the next.
    fn settled(&self, h: &zenkey_model::collection::Heard) -> bool {
        h.complete()
            || self
                .heard
                .iter()
                .skip_while(|o| !std::ptr::eq(*o, h))
                .skip(1)
                .any(|o| o.key == h.key && o.complete())
    }

    /// One state GET of `selector`, asked at `asked`, its replies merged
    /// and delivered: a snapshot's page as its entries, each the per-key
    /// reply it stands for on its member's key, the page held toward its
    /// snapshot, and the omission rule over the snapshot it completes; a
    /// page whose body is refused, as every other reply, as it came
    /// (delivered undecodable, as a mutation's reader reads one), but a
    /// stray `reply_del`, which is dropped.
    async fn get(&mut self, selector: &str, asked: u64, deliver: &(impl Fn(SampleView) + Sync)) {
        let opts = crate::bus::query::GetOpts::new(self.timeout);
        let Ok(replies) = crate::bus::query::state_get(self.session, selector, &opts).await else {
            self.complete = false;
            return;
        };
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else {
                self.complete = false;
                continue;
            };
            self.replies += 1;
            let view = view_of(sample);
            if view.kind == zenoh::sample::SampleKind::Delete {
                if self.stray(&view).await {
                    tracing::debug!(key = %view.key, "a reply_del where its owner answers by snapshot is no answer (#977's E4)");
                    continue;
                }
                if self.merge.admit(&view) {
                    deliver(view);
                }
                continue;
            }
            let page = {
                let bytes = view.payload.to_bytes();
                crate::model::inband::page(&view.key, Some(&view.encoding), &bytes)
            };
            let Some(Ok(page)) = page else {
                if self.merge.admit(&view) {
                    deliver(view);
                }
                continue;
            };
            let (i, done) = zenkey_model::collection::hear(&mut self.heard, &view.key, &page);
            if i == self.pages.len() {
                self.pages.push(Vec::new());
            }
            for v in crate::model::inband::views_of(&view, &page) {
                if self.merge.admit(&v) {
                    deliver(v);
                }
            }
            if !self.pages[i].iter().any(|p| p.index == page.index) {
                self.pages[i].push(page);
            }
            if done {
                self.omit(i, asked, &view, deliver).await;
            }
        }
    }

    /// Whether `view`, a `reply_del`, is a stray one: on a member of a
    /// collection its owner's revision under the deletion's run answers by
    /// snapshot (`zenkey_model::collection::stray_reply_del`). Never
    /// without contracts, nor where the revision is not had.
    async fn stray(&mut self, view: &SampleView) -> bool {
        let (Some(r), Some(stamp)) = (self.revisions.as_mut(), view.inband()) else {
            return false;
        };
        r.of(self.session, self.timeout, &view.key, stamp.run)
            .await
            .is_some_and(|c| zenkey_model::collection::stray_reply_del(&c, &view.key))
    }

    /// The omission rule over the snapshot `heard[i]` that the page `view`
    /// carried completed (core §4.2): with the owner's revision, each key
    /// at or under its key that it lists neither live nor as a tombstone,
    /// and that the merge holds no newer than it, is forgotten, and a
    /// deletion with no stamp is delivered for each that held a value.
    async fn omit(
        &mut self,
        i: usize,
        asked: u64,
        view: &SampleView,
        deliver: &(impl Fn(SampleView) + Sync),
    ) {
        let Some(r) = self.revisions.as_mut() else {
            return;
        };
        let (key, run) = (self.heard[i].key.clone(), self.heard[i].run);
        let Some(snapshot) = zenkey_model::collection::Snapshot::complete(&self.pages[i]) else {
            return;
        };
        // Nothing to drop asks no descriptor.
        let listed: std::collections::BTreeSet<String> = snapshot
            .live
            .iter()
            .map(|e| e.id.as_str())
            .chain(snapshot.tombs.iter().map(|t| t.id.as_str()))
            .map(|id| zenkey_model::collection::member_key(&key, id))
            .collect();
        if self
            .merge
            .held_under(&key, asked)
            .iter()
            .all(|(k, _, _)| listed.contains(k))
        {
            return;
        }
        let Some(contract) = r.of(self.session, self.timeout, &key, run).await else {
            return;
        };
        let pages = &self.pages[i];
        for k in self.merge.drop_omitted(&key, asked, |held| {
            zenkey_model::collection::omit(
                &contract,
                &key,
                pages,
                held.iter()
                    .map(|(k, mark, after)| zenkey_model::seed::HeldMember {
                        key: k,
                        mark: *mark,
                        heard_after: *after,
                    }),
            )
            .into_iter()
            .map(str::to_owned)
            .collect()
        }) {
            deliver(omitted(view, k));
        }
    }
}

/// The deletion the omission rule delivers for `key`: a delete with no
/// envelope and no stamp, the completing page's QoS and arrival kept.
fn omitted(page: &SampleView, key: String) -> SampleView {
    SampleView {
        key,
        payload: zenoh::bytes::ZBytes::default(),
        encoding: zenkey_model::wire::ENCODING.to_owned(),
        kind: zenoh::sample::SampleKind::Delete,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        ..page.clone()
    }
}

/// The revisions a seed with contracts reads a snapshot's index and a
/// `reply_del` with (core §4.2, "The owner's revision maps the index"):
/// per owner, interface and run, the one its descriptor stating the run
/// names (`zenkey::answer::owner_revision`), from the store or retrieved
/// by fingerprint; where no descriptor reaches the tool, the one revision
/// of the interface the store holds, the deployment's word; else none.
/// Each asked once.
struct Revisions<'a> {
    store: &'a BundleStore,
    held: std::collections::BTreeMap<
        (String, String),
        Option<std::sync::Arc<zenkey_model::contract::Contract>>,
    >,
}

impl<'a> Revisions<'a> {
    fn new(store: &'a BundleStore) -> Self {
        Revisions {
            store,
            held: std::collections::BTreeMap::new(),
        }
    }

    /// The revision `key`'s owner runs its interface under `run`.
    async fn of(
        &mut self,
        session: &Session,
        timeout: Duration,
        key: &str,
        run: zenkey_model::wire::Run,
    ) -> Option<std::sync::Arc<zenkey_model::contract::Contract>> {
        use zenkey::answer::OwnerRevision;
        let (prefix, iface) = owner_interface(key)?;
        let at = (prefix, run.to_string());
        if let Some(c) = self.held.get(&at) {
            return c.clone();
        }
        let c = match zenkey::answer::owner_revision(session, key, run, &iface, timeout).await {
            OwnerRevision::Named(fp) => match zenkey_model::canonical::Fingerprint::parse(&fp) {
                Ok(fp) => self
                    .store
                    .contract(session, &iface, &fp)
                    .await
                    .ok()
                    .flatten(),
                Err(_) => None,
            },
            OwnerRevision::Unreached => {
                let held: Vec<_> = self
                    .store
                    .held()
                    .into_iter()
                    .filter(|r| r.iface() == &iface)
                    .collect();
                match held.as_slice() {
                    [one] => Some(one.shared_contract()),
                    _ => None,
                }
            }
            OwnerRevision::Unread => None,
        };
        self.held.insert(at, c.clone());
        c
    }
}

/// A zk2 data key's text up to its interface, a namespace in front kept,
/// and its interface.
fn owner_interface(key: &str) -> Option<(String, zenkey_model::grammar::IfaceId)> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(zenkey_model::grammar::ZkKey::Data { addr, iface, .. }) =
                zenkey_model::grammar::parse(k)
        {
            let prefix = &key[..key.len() - k.len()];
            return Some((format!("{prefix}zk2/{addr}/{iface}"), iface));
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}

/// The legs `policy` asks for, concurrently, through one merge; the
/// coverage once every one of them has resolved. The state leg reads with
/// `contracts` where the tool holds them.
pub(crate) async fn run_legs(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
    merge: &Merge,
    contracts: Option<&BundleStore>,
    deliver: impl Fn(SampleView) + Sync,
) -> SeedCoverage {
    let leg = |on: bool, leg: Leg| {
        let deliver = &deliver;
        async move {
            if on {
                Asked::Asked(
                    run_leg(
                        session,
                        selector,
                        leg,
                        policy.timeout,
                        merge,
                        contracts,
                        deliver,
                    )
                    .await,
                )
            } else {
                Asked::NotAsked
            }
        }
    };
    let (history, state) = tokio::join!(
        leg(policy.history, Leg::History),
        leg(policy.state, Leg::State)
    );
    SeedCoverage {
        history,
        state,
        superseded: merge.superseded(),
    }
}

/// The history leg's selector for a data selector: every advanced
/// publisher's cache queryable under it (spec §2.5).
fn cache_selector(selector: &str) -> String {
    format!("{selector}/@adv/**")
}

/// Subscribe as a late joiner (core §4.2, 0.26).
///
/// Order of operations is the contract: the subscriber is declared first;
/// the seed legs (S4's state GET, and the `@adv` history) run after,
/// concurrently; every delivery — a reply or a live sample — passes one
/// per-key merge by the owner's in-band stamp (core §4.3; a Zenoh timestamp
/// for a sample with no envelope), so a mutation published in the seed
/// window lands exactly once and a stale reply cannot resurrect or regress
/// a key within a run. Deletions ride through as tombstone samples
/// ([`zenoh::sample::SampleKind::Delete`]) — a live delete and a
/// `reply_del` alike — subject to the same merge, never dropped.
/// [`SeedItem::SeedComplete`] is sent only once **every** leg has resolved.
/// It holds no contract, so it applies no omission rule
/// ([`seed_subscribe_with`] does).
pub async fn seed_subscribe(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
) -> Result<SeededSubscriber> {
    seed_subscribe_in(session, selector, policy, None).await
}

/// [`seed_subscribe`] for a tool that holds contracts (#977's E4):
/// `contracts`, those given offline and those retrieved by fingerprint
/// (core §8.4). Its state leg applies the omission rule over each complete
/// snapshot with the revision its owner runs, drops a stray `reply_del`,
/// and reads a snapshot it holds incomplete again once (the module doc).
pub async fn seed_subscribe_with(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
    contracts: &BundleStore,
) -> Result<SeededSubscriber> {
    seed_subscribe_in(session, selector, policy, Some(contracts.clone())).await
}

async fn seed_subscribe_in(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
    contracts: Option<BundleStore>,
) -> Result<SeededSubscriber> {
    let (tx, rx) = seed_channel(SEED_CAPACITY);

    let merge = Arc::new(Merge::new(DEFAULT_MAX_KEYS, None));

    // 1) The subscriber, FIRST — anything published from here on is caught.
    let subscriber = crate::bus::teardown::declared(
        "seeded subscribe",
        selector,
        session.declare_subscriber(selector.to_string()).callback({
            let tx = tx.clone();
            let merge = Arc::clone(&merge);
            move |sample| {
                let view = view_of(&sample);
                if merge.admit(&view) {
                    tx.send_sample(view);
                }
            }
        }),
    )
    .await?;

    // 2) The seed legs, AFTER — and the completion boundary once every one
    //    (or its opt-out) resolves.
    let task = {
        let session = session.clone();
        let selector = selector.to_string();
        let merge = Arc::clone(&merge);
        tokio::spawn(async move {
            let coverage = run_legs(
                &session,
                &selector,
                policy,
                &merge,
                contracts.as_ref(),
                |view| {
                    tx.send_sample(view);
                },
            )
            .await;
            tx.send_boundary(coverage).await;
        })
    };

    Ok(SeededSubscriber {
        rx,
        _subscriber: subscriber,
        task,
        merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(key: &str) -> SampleView {
        SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(vec![0u8; 1]),
            encoding: "zenoh/bytes".to_string(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        }
    }

    /// The merge's order is zenoh's (core §4.2, 0.27, #752): for every pair
    /// of stamps, `zenkey_model::seed::Stamp` compares as zenoh's
    /// `Timestamp` does — time, then the id's little-endian bytes — and
    /// spells itself as zenoh does. Ids whose integer order and byte order
    /// disagree are among them (`2` and `100`, two real zids).
    #[test]
    fn the_merge_orders_stamps_as_zenoh_does() {
        use crate::model::inband::stamp_of;
        use zenoh::time::{NTP64, Timestamp, TimestampId};
        let ids = [
            "1",
            "2",
            "ff",
            "100",
            "1ff",
            "2fe",
            "a1b2",
            "a2b2",
            "7db42403f70edc0d9e28614c0eeeb1b1",
            "4baa50f80d37a5dda951691aac616ef2",
            "10000000000000000000000000000000",
        ];
        let stamps: Vec<Timestamp> = [5u64, 6, 7_386_690_599_959_157_260]
            .into_iter()
            .flat_map(|t| {
                ids.iter().map(move |id| {
                    Timestamp::new(NTP64(t), id.parse::<TimestampId>().expect("an id"))
                })
            })
            .collect();
        for a in &stamps {
            assert_eq!(stamp_of(a).to_string(), a.to_string());
            for b in &stamps {
                assert_eq!(stamp_of(a).cmp(&stamp_of(b)), a.cmp(b), "{a} against {b}");
            }
        }
        let low = |id: &str| Timestamp::new(NTP64(5), id.parse::<TimestampId>().unwrap());
        assert!(low("100") < low("2"), "zenoh: the low byte decides");
    }

    /// The merge is bounded and says what the bound cost (observer §4.1,
    /// tooling guide O6): past `max_keys`, the least recently admitted key
    /// is forgotten and counted, apart from `superseded`, and a forgotten
    /// key holds nothing, so its next sample is kept whatever its stamp. A
    /// key the bound kept still refuses an older stamp, after any number of
    /// admissions: the merge has no end.
    #[test]
    fn the_merge_forgets_past_its_bound_and_counts_it() {
        use zenoh::time::{NTP64, Timestamp, TimestampId};
        let ledger = Arc::new(MergeLedger::default());
        let merge = Merge::new(2, Some(Arc::clone(&ledger)));
        let at = |key: &str, t: u64| {
            let mut v = view(key);
            v.timestamp = Some(Timestamp::new(
                NTP64(t),
                "a1".parse::<TimestampId>().expect("an id"),
            ));
            v
        };
        assert!(merge.admit(&at("k/a", 5)));
        assert!(merge.admit(&at("k/b", 5)));
        assert!(!merge.admit(&at("k/a", 4)), "a is held, and 4 is older");
        assert_eq!((merge.superseded(), merge.evicted()), (1, 0));
        // A third key: `a`, admitted least recently, is forgotten.
        assert!(merge.admit(&at("k/c", 5)));
        assert_eq!(merge.evicted(), 1, "one key forgotten, and counted");
        assert!(!merge.admit(&at("k/c", 4)), "c is held");
        assert!(
            merge.admit(&at("k/a", 1)),
            "a forgotten holds nothing: kept whatever its stamp"
        );
        assert_eq!((merge.superseded(), merge.evicted()), (2, 2));
        assert_eq!(
            (
                ledger.superseded.load(std::sync::atomic::Ordering::Relaxed),
                ledger.evicted.load(std::sync::atomic::Ordering::Relaxed)
            ),
            (2, 2),
            "a monitor's totals move with the merge's own"
        );
    }

    /// Deep-review D5: the seed channel is bounded, and what the bound
    /// refuses is counted and surfaced as [`SeedItem::Dropped`] before the
    /// stream resumes — the O6 honesty every other delivery surface in this
    /// crate already has. The boundary rides with backpressure and is never
    /// among the dropped.
    #[tokio::test]
    async fn a_slow_seed_consumer_is_told_what_it_missed() {
        let (tx, mut rx) = seed_channel(4);
        for i in 0..10 {
            tx.send_sample(view(&format!("k/{i}")));
        }
        // 4 fit; 6 were refused by the bound.
        let Some(SeedItem::Dropped(n)) = rx.recv().await else {
            panic!("expected the dropped count first");
        };
        assert_eq!(n, 6, "every refusal is counted, exactly once");
        for i in 0..4 {
            let Some(SeedItem::Sample(v)) = rx.recv().await else {
                panic!("expected the retained samples");
            };
            assert_eq!(v.key, format!("k/{i}"), "the retained head is in order");
        }
        // The count was handed over, not double-reported.
        tx.send_sample(view("k/late"));
        let Some(SeedItem::Sample(v)) = rx.recv().await else {
            panic!("the stream resumes");
        };
        assert_eq!(v.key, "k/late");

        // The boundary waits for room instead of dropping (a lost boundary
        // is a consumer stuck on "loading" forever).
        for i in 0..4 {
            tx.send_sample(view(&format!("b/{i}")));
        }
        let boundary = tokio::spawn(async move {
            tx.send_boundary(SeedCoverage {
                history: Asked::Asked(SeedLeg {
                    replies: 0,
                    complete: true,
                    snapshots: Vec::new(),
                }),
                state: Asked::Asked(SeedLeg {
                    replies: 0,
                    complete: true,
                    snapshots: Vec::new(),
                }),
                superseded: 0,
            })
            .await;
        });
        let mut seen_boundary = false;
        while let Some(item) = rx.recv().await {
            if let SeedItem::SeedComplete(c) = item {
                assert_eq!(c.superseded, 0);
                seen_boundary = true;
                break;
            }
        }
        assert!(seen_boundary, "the boundary is never among the dropped");
        boundary.await.expect("boundary task");
    }
}
