//! Asking the fleet a whole question at once (#175).
//!
//! What separates these from [`super::value`] is not the transport — it is all
//! `fleet_get` underneath — but what an answer is *about*: a sweep's answer
//! describes a deployment, so every one of them carries the base it ran
//! against back to the pane. That is the staleness guard's other half (#109),
//! and it is the reason so many of these return a tuple rather than a plain
//! `Result`: the pane cannot tell whether an answer is still true without
//! knowing what it was asked of.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use zenkey_fleet::{Skeleton, SliceSet};

use crate::message::{BusMsg, Message, PaneMsg};
use anyhow::Context as _;

use crate::services::ServiceError;
use crate::view::admin::AdminMsg;
use crate::view::blob::BlobMsg;
use crate::view::doctor::DoctorMsg;
use crate::view::nodes::NodesMsg;

/// The registry slices, from the bus alone.
pub fn slices(session: zenoh::Session, base: String, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_bus(&zenkey_fleet::Fleet::new(&session, &base), timeout)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::SlicesLoaded(r)),
    )
}

/// The §6.1 union (issue #43): served wins, dirs fill, and the disagreement
/// count reaches the status strip as data.
pub fn slices_union(
    session: zenoh::Session,
    base: String,
    dirs: Vec<std::path::PathBuf>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_union(&zenkey_fleet::Fleet::new(&session, &base), &dirs, timeout)
                .await
                .map(|out| {
                    let counts = crate::view::status::UnionCounts {
                        from_bus: out.from_bus.len(),
                        dirs_only: out.dirs_only.len(),
                        disagreements: out.disagreements.len(),
                        // Read before the set moves into the `Arc` (#399).
                        self_disagreements: self_disagreements(&out.set),
                    };
                    (Arc::new(out.set), counts)
                })
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::SlicesUnionLoaded(r)),
    )
}

/// (Re)build the skeleton. Slices are already loaded; roster and admin are
/// gathered inside the task (both metadata-only).
pub fn skeleton(
    session: zenoh::Session,
    base: String,
    slices: Arc<SliceSet>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let roster = zenkey_fleet::roster(&fleet, timeout)
                .await
                .unwrap_or_default();
            let admin = zenkey_fleet::declared_entities(&session, timeout)
                .await
                .unwrap_or(None);
            let skeleton = Arc::new(Skeleton::build(&base, &slices, &roster, admin.as_ref()));
            Ok((skeleton, Arc::new(roster)))
        },
        |r| Message::Bus(BusMsg::SkeletonBuilt(r)),
    )
}

/// One node's detail — the nodes pane's one data-plane cost, paid on selection
/// (the laziness ground rule, #84/#85).
pub fn node_info(
    session: zenoh::Session,
    base: String,
    origin: String,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let out = zenkey_fleet::node_info(&fleet, &origin, timeout, true)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of);
            (origin, base, out)
        },
        |(origin, ran, out)| Message::Pane(PaneMsg::Nodes(NodesMsg::InfoLoaded(origin, ran, out))),
    )
}

/// The doctor run (#109).
///
/// `dirs` rather than the app's resolved slice set, deliberately: the doctor
/// diffs local declarations against served ones, and handing it the union
/// would diff the bus against itself.
pub fn doctor(
    session: zenoh::Session,
    base: String,
    dirs: Vec<std::path::PathBuf>,
    spec: zenkey_fleet::DoctorSpec,
) -> Task<Message> {
    Task::perform(
        async move {
            // `None` when no dirs were given: "no registry loaded" is a
            // different claim from "a registry that declares nothing", and
            // only the engine's `Option` can say which happened.
            let locals = match dirs.is_empty() {
                true => None,
                false => match SliceSet::from_dirs(&dirs) {
                    Ok(set) => Some(set),
                    Err(e) => {
                        return Err(ServiceError::of(
                            anyhow::Error::new(e).context("failed to read the registry dirs"),
                        ));
                    }
                },
            };
            zenkey_fleet::run_doctor(
                &zenkey_fleet::Fleet::new(&session, &base),
                locals.as_ref(),
                &spec,
            )
            .await
            .map(|r| crate::doctor::DoctorRun {
                report: Arc::new(r),
                base,
            })
            .map_err(ServiceError::of)
        },
        |out| Message::Pane(PaneMsg::Doctor(DoctorMsg::Done(out))),
    )
}

/// The admin sweep (#131): routers, storages, declared entities, the coverage
/// join, the topology and the origin attachments, as one act the user asked
/// for.
///
/// `slices` here is the app's *resolved* set — bus, dirs or the union — and
/// deliberately not the doctor's dirs-only one: the coverage join asks "what
/// does the registry say exists", and a dirs-only answer would empty the table
/// on every bus-registry-only deployment. An invisible, plausible-looking
/// wrong answer is worse than no table.
pub fn admin(
    session: zenoh::Session,
    base: String,
    slices: Option<Arc<SliceSet>>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let routers = zenkey_fleet::routers(&session, timeout)
                .await
                .context("failed to list the routers")
                .map_err(ServiceError::of)?;
            let storages = zenkey_fleet::storages(&session, timeout)
                .await
                .context("failed to list the storages")
                .map_err(ServiceError::of)?;
            let declared = zenkey_fleet::declared_entities(&session, timeout)
                .await
                .context("failed to list the declared entities")
                .map_err(ServiceError::of)?;
            let (coverage, coverage_note) = match slices.as_deref() {
                Some(set) => (zenkey_fleet::state_coverage(set, &base, &storages), None),
                None => (
                    Vec::new(),
                    Some(
                        "no registry slices resolved, so the declared state \
                         families are unknown — this is \"not asked\", not \
                         \"uncovered\" (RFC 09 §5.1 O4). Pass --registry, \
                         or wait for the bus registry (RFC 08 §6)."
                            .to_string(),
                    ),
                ),
            };
            let topology = zenkey_fleet::topology(&session, timeout)
                .await
                .context("failed to read the topology")
                .map_err(ServiceError::of)?;
            let origins = zenkey_fleet::origin_attachments(
                &zenkey_fleet::Fleet::new(&session, &base),
                timeout,
            )
            .await
            .context("failed to read the origin attachments")
            .map_err(ServiceError::of)?;
            Ok(Arc::new(crate::admin::AdminSweep {
                routers,
                storage: zenkey_fleet::report::StorageList { storages, coverage },
                declared,
                coverage_note,
                topology,
                origins,
                base,
            }))
        },
        |out| Message::Pane(PaneMsg::Admin(AdminMsg::Done(out))),
    )
}

/// The key-population budget join (#221): declared `cardinality` against the
/// observed tree, per family and per origin.
///
/// The one entry in this module that costs the bus **nothing** — it joins
/// data already in hand — but it is O(observed keys × refinement) and runs on
/// a throttled cadence, so it is a `Task` like the sweeps rather than work on
/// the update thread. `zenkey_fleet::judge::budget` does the judging; the landing
/// carries the badge map the tree looks rows up in.
pub fn budget(
    base: String,
    slices: Arc<SliceSet>,
    observed: Arc<zenkey_fleet::KeyTreeSnapshot>,
) -> Task<Message> {
    Task::perform(
        async move { Arc::new(crate::budget::badges(&base, &slices, &observed)) },
        |badges| Message::Bus(BusMsg::BudgetJoined(badges)),
    )
}

/// Who holds this blob (RFC 07 §2).
pub fn blob_probe(
    session: zenoh::Session,
    base: String,
    target: zenkey_fleet::BlobTarget,
    slices: Vec<zenkey::RegistrySlice>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let out = zenkey_fleet::blob_probe(&fleet, &target, &slices, timeout)
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
    root: zenkey::ContentHash,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet::Fleet::new(&session, &base);
            let out = zenkey_fleet::blob_tree_index(&fleet, &origin, &root, timeout)
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
    pub target: zenkey_fleet::BlobTarget,
    pub dest: std::path::PathBuf,
    pub spec: zenkey_fleet::BlobFetchSpec,
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
        let fleet = zenkey_fleet::Fleet::new(&session, &base);
        let out = zenkey_fleet::blob_fetch(&fleet, &origin, &target, &dest, &spec, &sink)
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
    mut rx: tokio::sync::mpsc::Receiver<zenkey_fleet::report::BlobProgress>,
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
    p: zenkey_fleet::report::BlobProgress,
    coalesced: &std::sync::atomic::AtomicU64,
) -> Message {
    Message::Pane(PaneMsg::Blob(BlobMsg::Progress(
        p,
        coalesced.load(std::sync::atomic::Ordering::Relaxed),
    )))
}

/// Producers the fleet does not agree with itself about (#399).
///
/// Zero for a set that never asked — files carry no origin — which is why the
/// status strip's `Dirs` variant has no such field at all rather than a zero
/// this would hand it (RFC 13 §3 O4).
pub fn self_disagreements(set: &SliceSet) -> usize {
    set.collapsed()
        .as_option()
        .copied()
        .unwrap_or(&[])
        .iter()
        .filter(|c| !c.agreed)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt as _;
    use zenkey_fleet::report::BlobProgress;

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
