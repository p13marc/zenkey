//! Correct state seeding — RFC 04 §3.2 as an engine helper (issue #42; the
//! repo's old #20).
//!
//! The discipline is normative and subtle, and every state-showing pane
//! would otherwise reimplement (or skip) it:
//!
//! - the subscriber is declared **before** any seed GET — "GET-then-subscribe
//!   is forbidden: a transition published in the gap is silently dropped, and
//!   a dropped delete is a resurrected key";
//! - seed replies and live samples **merge per key by HLC timestamp** (LWW,
//!   RFC 04 §1.2) — a stale seed must never overwrite a newer live sample;
//! - the two seed paths differ in **coverage** and both are needed: the
//!   history GET (`<selector>/@adv/**`) reaches *live* publishers' caches
//!   (dies with the publisher), the plain GET on the selector is answered by
//!   *router storages* — the crashed-producer case a UI must include. "What
//!   no consumer may do: assume a plain GET reaches publisher caches, or
//!   that a history query reaches storages."
//!
//! Both seed paths are queries **this module issues itself** rather than
//! zenoh-ext's `history()` replay: the boundary ([`SeedItem::SeedComplete`])
//! must not fire until every seed path has resolved, and only a query we own
//! has an awaitable end. Coverage is **reported, never assumed**:
//! [`SeedCoverage`] says which paths ran and what each yielded, and its
//! zeros are observations, not verdicts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Result;
use zenoh::Session;

use crate::bus::monitor::SampleView;
use crate::report::SeedCoverage;

/// Which seed paths to run. Default: both — per-path opt-out exists because
/// a deployment may *know* it has no storages (or no advanced publishers),
/// not because skipping is free.
#[derive(Debug, Clone, Copy)]
pub struct SeedPolicy {
    /// Query live publishers' `@adv` caches (`<selector>/@adv/**`) — the
    /// same rung `fetch_value` uses; reaches only publishers that are alive.
    pub history: bool,
    /// GET the selector itself (answered by router storages — the only path
    /// that still has state from *crashed* producers).
    pub storage: bool,
    /// Bound on each seed GET (they run concurrently, so this bounds the
    /// whole seed phase too).
    pub timeout: Duration,
}

impl Default for SeedPolicy {
    fn default() -> Self {
        SeedPolicy {
            history: true,
            storage: true,
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

/// The shared LWW merge: one entry per key, latest HLC wins; stamped beats
/// unstamped; unstamped-vs-unstamped passes through (nothing to compare — a
/// deployment without timestamping has opted out of LWW, RFC 04 §4, and
/// suppressing would be guessing).
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

/// Run one seed GET; every reply passes the merge; admitted samples go to
/// `deliver`; returns the reply count.
pub(crate) async fn seed_get(
    session: &Session,
    selector: &str,
    timeout: Duration,
    merge: &Merge,
    mut deliver: impl FnMut(SampleView),
) -> usize {
    let mut n = 0usize;

    // `accept_any`: cache replies arrive on the sample's own key, outside an
    // `@adv`-suffixed selector — without it they are dropped.
    let opts = crate::bus::query::GetOpts::new(timeout).accept_any();
    if let Ok(replies) = crate::bus::query::disciplined_get(session, selector, &opts).await {
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            n += 1;
            let view = view_of(sample);
            if merge.admit(&view) {
                deliver(view);
            }
        }
    }
    n
}

/// The history-path selector for a data selector (the `fetch_value` cache
/// rung, applied to a whole subtree).
pub(crate) fn cache_selector(selector: &str) -> String {
    format!("{selector}/@adv/**")
}

/// Subscribe with a correct seed phase (RFC 04 §3.2).
///
/// Order of operations is the contract: the subscriber is declared first;
/// the seed GETs (history `@adv` + storage) run after, concurrently; every
/// delivery — cached, stored, or live — passes one per-key LWW merge, so a
/// transition published in the seed window lands exactly once and a stale
/// seed cannot resurrect or regress a key. Deletes ride through as tombstone
/// samples ([`zenoh::sample::SampleKind::Delete`]) subject to the same merge — never
/// dropped. [`SeedItem::SeedComplete`] is sent only once **both** paths have
/// resolved.
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

    // 2) The seed GETs, AFTER — and the completion boundary once both
    //    (or their opt-outs) resolve.
    let task = {
        let session = session.clone();
        let selector = selector.to_string();
        let merge = Arc::clone(&merge);
        tokio::spawn(async move {
            let history = async {
                if policy.history {
                    let sel = cache_selector(&selector);
                    Some(
                        seed_get(&session, &sel, policy.timeout, &merge, |view| {
                            tx.send_sample(view);
                        })
                        .await,
                    )
                } else {
                    None
                }
            };
            let storage = async {
                if policy.storage {
                    Some(
                        seed_get(&session, &selector, policy.timeout, &merge, |view| {
                            tx.send_sample(view);
                        })
                        .await,
                    )
                } else {
                    None
                }
            };
            let (history_replies, storage_replies) = tokio::join!(history, storage);
            tx.send_boundary(SeedCoverage {
                history_replies,
                storage_replies,
                superseded: merge.superseded(),
            })
            .await;
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
                history_replies: Some(0),
                storage_replies: Some(0),
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
