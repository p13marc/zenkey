//! The `@blob` plane: probe, tree inspection, and a download whose progress
//! and outcome arrive on one stream.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::blob::BlobMsg;

/// Who holds this blob (RFC 07 §2).
pub fn blob_probe(
    session: zenoh::Session,
    base: String,
    target: zenkey_fleet_v1::BlobTarget,
    slices: Vec<zenkey_v1::RegistrySlice>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
            let out = zenkey_fleet_v1::blob_probe(&fleet, &target, &slices, timeout)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of);
            (base, out)
        },
        |(ran, out)| Message::Pane(PaneMsg::Blob(BlobMsg::ProbeDone(ran, out))),
    )
}

/// A tree target is inspected, not downloaded (RFC 07 §2.3, v1.17): the
/// descriptor and index chunks, validated against the root — which the key
/// already is. No file, no progress stream, no cancel token.
pub fn blob_tree(
    session: zenoh::Session,
    base: String,
    origin: String,
    root: zenkey_v1::ContentHash,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
            let out = zenkey_fleet_v1::blob_tree_index(&fleet, &origin, &root, timeout)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of);
            (base, out)
        },
        |(ran, out)| Message::Pane(PaneMsg::Blob(BlobMsg::InspectDone(ran, out))),
    )
}

/// What one blob download needs. The origin comes off the chosen holder, which
/// came off a reply key — there is no other way for one to enter here.
pub struct BlobFetch {
    pub session: zenoh::Session,
    pub base: String,
    pub origin: String,
    pub target: zenkey_fleet_v1::BlobTarget,
    pub dest: std::path::PathBuf,
    pub spec: zenkey_fleet_v1::BlobFetchSpec,
}

/// How many progress events may queue ahead of the pane (#344).
///
/// The producer runs at network speed — one event per verified chunk — and the
/// consumer at frame rate. Unbounded, this queue grew without limit and kept
/// the bar ticking long after the file was on disk. Bounded, a burst that
/// outruns a frame is **coalesced**: the events that do not fit are dropped
/// and counted, never queued and never waited on.
///
/// Nothing final is lost by that. Every event the pane reads carries absolute
/// `received`/`total`/`bytes_received`, so the next one that fits states the
/// whole truth again, and the transfer's own report supersedes all of them
/// when it lands. What coalescing *does* cost is the assumption that the bar
/// ticked once per chunk — so the count is carried to the pane and the bar
/// says it (RFC 13 §3 O6, the *coalesced* kind).
///
/// 32 is a frame's worth of headroom at any plausible chunk rate: deep enough
/// that an ordinary transfer never coalesces at all, shallow enough that the
/// queue cannot become a backlog in its own right.
const PROGRESS_QUEUE: usize = 32;

/// Download, reporting as it goes.
///
/// Progress arrives on a channel rather than through the return value: a
/// transfer that only reported at the end would leave the pane unable to say
/// anything true while it ran.
pub fn blob_fetch(f: BlobFetch) -> Task<Message> {
    let (tx, rx) = tokio::sync::mpsc::channel(PROGRESS_QUEUE);
    let coalesced = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = Arc::clone(&coalesced);
    // The sink and everything it borrows live *inside* this future, so they
    // are gone the moment it resolves — which is what closes the channel and
    // lets `ordered` prove it has drained.
    let transfer = async move {
        let BlobFetch {
            session,
            base,
            origin,
            target,
            dest,
            spec,
        } = f;
        // `try_send`, not `send`: a full queue must never make a transfer wait
        // on the frame rate. What does not fit is counted, not queued.
        let sink = move |p| {
            if tx.try_send(p).is_err() {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        };
        let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
        let out = zenkey_fleet_v1::blob_fetch(&fleet, &origin, &target, &dest, &spec, &sink)
            .await
            .map(Arc::new)
            .map_err(ServiceError::of);
        (base, out)
    };
    Task::run(
        ordered(rx, transfer, coalesced, |(base, out)| {
            Message::Pane(PaneMsg::Blob(BlobMsg::FetchDone(base, out)))
        }),
        |m| m,
    )
}

/// Progress and outcome on **one** stream, in that order (#344).
///
/// The ordering is by construction, not by luck: `transfer` owns the only
/// sender, so the channel closes exactly when the transfer resolves, and the
/// drain that follows yields every event it queued *before* `finish` maps the
/// outcome. Batched as two tasks the order was whatever the executor felt
/// like, and a backlogged `Progress` landing after `FetchDone` was harmless
/// only because the handler happens to guard on `Fetch::InFlight`.
fn ordered<T>(
    mut rx: tokio::sync::mpsc::Receiver<zenkey_fleet_v1::report::BlobProgress>,
    transfer: impl Future<Output = T>,
    coalesced: Arc<std::sync::atomic::AtomicU64>,
    finish: impl FnOnce(T) -> Message,
) -> impl iced::futures::Stream<Item = Message> {
    async_stream::stream! {
        let out = {
            let mut transfer = std::pin::pin!(transfer);
            loop {
                // `yield` cannot live inside a `select!` branch, so the branch
                // hands its step out and the match does the yielding.
                let step = tokio::select! {
                    // Biased on the drain: a queued update is never held back
                    // behind the outcome that would make it stale.
                    biased;
                    p = rx.recv() => Ok(p),
                    out = &mut transfer => Err(out),
                };
                match step {
                    Ok(Some(p)) => yield progress(p, &coalesced),
                    // The sender lives in `transfer`, so the channel cannot
                    // close before it resolves; waiting for the outcome is
                    // the honest answer if it somehow did.
                    Ok(None) => break (&mut transfer).await,
                    Err(out) => break out,
                }
            }
        };
        // The sender died with the transfer, so this terminates — and it
        // yields everything still queued before the outcome goes out.
        while let Some(p) = rx.recv().await {
            yield progress(p, &coalesced);
        }
        yield finish(out);
    }
}

/// One progress event plus what the queue has coalesced away so far.
fn progress(
    p: zenkey_fleet_v1::report::BlobProgress,
    coalesced: &std::sync::atomic::AtomicU64,
) -> Message {
    Message::Pane(PaneMsg::Blob(BlobMsg::Progress(
        p,
        coalesced.load(std::sync::atomic::Ordering::Relaxed),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt as _;
    use zenkey_fleet_v1::report::BlobProgress;

    fn chunk(i: u32) -> BlobProgress {
        BlobProgress::Chunk {
            index: i,
            received: i + 1,
            total: 4096,
            bytes_received: u64::from(i + 1) * 64,
        }
    }

    /// #344, both halves at once: a producer running far ahead of the frame
    /// rate coalesces instead of queueing without limit, the drops are
    /// counted, and no `Progress` can land after the `FetchDone` that ends the
    /// fetch. The old shape batched two tasks and let the executor decide the
    /// order; the guard in `update::pane::blob` that made that survivable was
    /// incidental.
    #[tokio::test]
    async fn a_burst_coalesces_and_no_progress_lands_after_the_outcome() {
        let (tx, rx) = tokio::sync::mpsc::channel(PROGRESS_QUEUE);
        let coalesced = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter = Arc::clone(&coalesced);
        // Four queues' worth in one go with nothing draining: the pathological
        // form of "the producer runs at network speed".
        let burst = PROGRESS_QUEUE as u32 * 4;
        let transfer = async move {
            let sink = move |p| {
                if tx.try_send(p).is_err() {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            };
            for i in 0..burst {
                sink(chunk(i));
            }
            "the outcome"
        };

        let out: Vec<Message> = ordered(rx, transfer, Arc::clone(&coalesced), |t| {
            assert_eq!(t, "the outcome");
            Message::Pane(PaneMsg::Blob(BlobMsg::FetchDone(
                "base".into(),
                Err("outcome".into()),
            )))
        })
        .collect()
        .await;

        // Bounded, not unbounded: the queue held its cap and no more.
        assert_eq!(
            out.len(),
            PROGRESS_QUEUE + 1,
            "{burst} events queued behind a bound of {PROGRESS_QUEUE}, plus the outcome"
        );
        assert_eq!(
            coalesced.load(std::sync::atomic::Ordering::Relaxed),
            u64::from(burst) - PROGRESS_QUEUE as u64,
            "every event that did not fit is counted, not silently gone"
        );

        // The ordering, by construction: the outcome is last, and every
        // progress before it carries the running coalesced count.
        let (last, progress) = out.split_last().expect("a fetch always ends");
        assert!(
            matches!(last, Message::Pane(PaneMsg::Blob(BlobMsg::FetchDone(..)))),
            "the outcome is the last thing this stream ever yields"
        );
        for m in progress {
            let Message::Pane(PaneMsg::Blob(BlobMsg::Progress(_, n))) = m else {
                panic!("nothing but progress precedes the outcome");
            };
            assert_eq!(*n, u64::from(burst) - PROGRESS_QUEUE as u64);
        }
    }

    /// A transfer whose events all fit coalesces nothing — the bound is not a
    /// tax on the ordinary case, and the bar says nothing about it.
    #[tokio::test]
    async fn a_transfer_inside_the_bound_coalesces_nothing() {
        let (tx, rx) = tokio::sync::mpsc::channel(PROGRESS_QUEUE);
        let coalesced = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter = Arc::clone(&coalesced);
        let transfer = async move {
            let sink = move |p| {
                if tx.try_send(p).is_err() {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            };
            for i in 0..4 {
                sink(chunk(i));
            }
        };
        let out: Vec<Message> = ordered(rx, transfer, Arc::clone(&coalesced), |()| {
            Message::Pane(PaneMsg::Blob(BlobMsg::FetchDone(
                "base".into(),
                Err("outcome".into()),
            )))
        })
        .collect()
        .await;
        assert_eq!(out.len(), 5, "four updates and the outcome");
        assert_eq!(coalesced.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}
