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

use anyhow::{Result, anyhow};
use zenoh::Session;

use crate::sub::SampleView;

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

/// What each seed path contributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SeedCoverage {
    /// Replies from the `@adv` cache query (`None` = the history path was
    /// disabled; `Some(0)` = ran and no cache answered — an observation,
    /// not a verdict).
    pub history_replies: Option<usize>,
    /// Replies from the storage GET (same `None`/`Some(0)` reading).
    pub storage_replies: Option<usize>,
    /// Samples suppressed by the merge — not newer than what was already
    /// seen for their key. The honesty counter: a seed that arrived late
    /// and lost (or duplicated the other path) is counted, never silently
    /// absorbed.
    pub superseded: u64,
}

/// One delivery from a seeded subscription.
#[derive(Debug, Clone)]
pub enum SeedItem {
    /// A sample that survived the per-key LWW merge — seed and live alike.
    Sample(SampleView),
    /// Both seed paths have resolved; everything after this is live-only.
    /// Consumers that render "loading" state key off this boundary.
    SeedComplete(SeedCoverage),
}

/// A subscription whose first phase is a correctly-merged seed.
pub struct SeededSubscriber {
    rx: tokio::sync::mpsc::UnboundedReceiver<SeedItem>,
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
    /// `None` when the subscription ended.
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
    if let Ok(replies) = session
        .get(selector)
        .target(zenoh::query::QueryTarget::All)
        .consolidation(zenoh::query::ConsolidationMode::None)
        // Cache replies arrive on the sample's own key, outside an
        // `@adv`-suffixed selector — without Any they are dropped.
        .accept_replies(zenoh::query::ReplyKeyExpr::Any)
        .timeout(timeout)
        .await
    {
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
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SeedItem>();
    let merge = Arc::new(Merge::new());

    // 1) The subscriber, FIRST — anything published from here on is caught.
    let subscriber = session
        .declare_subscriber(selector.to_string())
        .callback({
            let tx = tx.clone();
            let merge = Arc::clone(&merge);
            move |sample| {
                let view = view_of(&sample);
                if merge.admit(&view) {
                    let _ = tx.send(SeedItem::Sample(view));
                }
            }
        })
        .await
        .map_err(|e| anyhow!("seeded subscribe {selector}: {e}"))?;

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
                            let _ = tx.send(SeedItem::Sample(view));
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
                            let _ = tx.send(SeedItem::Sample(view));
                        })
                        .await,
                    )
                } else {
                    None
                }
            };
            let (history_replies, storage_replies) = tokio::join!(history, storage);
            let _ = tx.send(SeedItem::SeedComplete(SeedCoverage {
                history_replies,
                storage_replies,
                superseded: merge.superseded(),
            }));
        })
    };

    Ok(SeededSubscriber {
        rx,
        _subscriber: subscriber,
        task,
    })
}
