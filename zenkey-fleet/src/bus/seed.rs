//! A late joiner's seed — core §4.2 (0.26) as an engine helper (#614, FL4;
//! issue #42 before it, which carried v1's RFC 04 §3.2).
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
//!   and consolidation `Latest` set explicitly
//!   ([`crate::bus::query::state_get`]): addressed to the owners' keys,
//!   since each key it matches has one owner (P3), and answered for a
//!   foreign key by whatever holds it. A storage on an owner's state is a
//!   deployment fault (S4), never this seed's source;
//! - **the history leg** asks the `@adv` caches of the advanced publishers
//!   present (`<selector>/@adv/**`, spec §2.5), for resources that declare
//!   `history` — streams among them, which no state GET reaches. Under a
//!   verbatim chunk a late *publisher* is never found (§2.5), and a
//!   publisher that is gone took its cache with it;
//! - **replies and live samples merge per key by the owner's stamp**
//!   ([`Merge`]): the newer wins whichever path brought it, a `reply_del` is
//!   a deletion with its stamp, and one mutation both paths carried is kept
//!   once.
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Result;
use zenoh::Session;

use crate::bus::monitor::SampleView;
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
    /// than handed a silently thinned stream (RFC 09 §5.1 O6 — the mirror
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

/// The late joiner's merge (core §4.2, 0.26): one entry per key, and a
/// sample is admitted only when its stamp is newer than the one held,
/// whichever path — reply or live — brought either.
///
/// - Stamps compare as zenoh's `Latest` compares replies (§4.1): by time,
///   which within one timestamp id is the owner's order (§4.3). Two ids for
///   one key inside one seed are §4.3's residual risk, not resolved here.
/// - Equal stamps are one mutation: the second copy is suppressed.
/// - A deletion (a live delete, or a `reply_del` with the deletion's stamp,
///   S3) is a sample like any other, and merges the same way: never
///   dropped, never mistaken for an absence.
/// - Stamped beats unstamped; unstamped against unstamped passes through.
///   An owner stamps every mutation (S1), so an unstamped sample is no
///   conforming owner's: a foreign key's, on a deployment without
///   timestamping that has opted out of last-writer-wins, where
///   suppressing would be guessing.
///
/// Every suppression is counted ([`SeedCoverage::superseded`]).
///
/// `pub(crate)`: [`crate::Monitor::watch_seeded`] runs the same merge over
/// its seed phase (issue #92) — one discipline, not two.
pub(crate) struct Merge {
    latest: Mutex<HashMap<String, Option<zenoh::time::Timestamp>>>,
    superseded: std::sync::atomic::AtomicU64,
}

impl Merge {
    pub(crate) fn new() -> Merge {
        Merge {
            latest: Mutex::new(HashMap::new()),
            superseded: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(crate) fn superseded(&self) -> u64 {
        self.superseded.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn admit(&self, view: &SampleView) -> bool {
        let mut latest = self.latest.lock().expect("merge lock");
        let entry = latest.entry(view.key.clone()).or_insert(None);
        let admit = match (&entry, &view.timestamp) {
            (None, _) => true,
            (Some(_), None) => false, // stamped state beats an unstamped echo
            (Some(prev), Some(ts)) => ts > prev,
        };
        if admit {
            if view.timestamp.is_some() || entry.is_none() {
                *entry = view.timestamp;
            }
        } else {
            self.superseded
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        admit
    }
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
/// the replies and makes the leg incomplete.
async fn run_leg(
    session: &Session,
    selector: &str,
    leg: Leg,
    timeout: Duration,
    merge: &Merge,
    deliver: &(impl Fn(SampleView) + Sync),
) -> SeedLeg {
    let opts = crate::bus::query::GetOpts::new(timeout);
    let replies = match leg {
        // `accept_any`: cache replies arrive on the sample's own key, outside
        // an `@adv`-suffixed selector — without it they are dropped.
        Leg::History => {
            crate::bus::query::disciplined_get(
                session,
                &cache_selector(selector),
                &opts.accept_any(),
            )
            .await
        }
        Leg::State => crate::bus::query::state_get(session, selector, &opts).await,
    };
    let Ok(replies) = replies else {
        return SeedLeg {
            replies: 0,
            complete: false,
        };
    };
    let mut out = SeedLeg {
        replies: 0,
        complete: true,
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

/// The legs `policy` asks for, concurrently, through one merge; the
/// coverage once every one of them has resolved.
pub(crate) async fn run_legs(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
    merge: &Merge,
    deliver: impl Fn(SampleView) + Sync,
) -> SeedCoverage {
    let leg = |on: bool, leg: Leg| {
        let deliver = &deliver;
        async move {
            if on {
                Asked::Asked(run_leg(session, selector, leg, policy.timeout, merge, deliver).await)
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
/// per-key merge by the owner's stamp, so a mutation published in the seed
/// window lands exactly once and a stale reply cannot resurrect or regress
/// a key. Deletions ride through as tombstone samples
/// ([`zenoh::sample::SampleKind::Delete`]) — a live delete and a
/// `reply_del` alike — subject to the same merge, never dropped.
/// [`SeedItem::SeedComplete`] is sent only once **every** leg has resolved.
pub async fn seed_subscribe(
    session: &Session,
    selector: &str,
    policy: SeedPolicy,
) -> Result<SeededSubscriber> {
    let (tx, rx) = seed_channel(SEED_CAPACITY);

    let merge = Arc::new(Merge::new());

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
            let coverage = run_legs(&session, &selector, policy, &merge, |view| {
                tx.send_sample(view);
            })
            .await;
            tx.send_boundary(coverage).await;
        })
    };

    Ok(SeededSubscriber {
        rx,
        _subscriber: subscriber,
        task,
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
                }),
                state: Asked::Asked(SeedLeg {
                    replies: 0,
                    complete: true,
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
