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
use crate::view::admin::AdminMsg;
use crate::view::blob::BlobMsg;
use crate::view::doctor::DoctorMsg;
use crate::view::nodes::NodesMsg;

/// The registry slices, from the bus alone.
pub fn slices(session: zenoh::Session, base: String, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_bus(&session, &base, timeout)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string())
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
            SliceSet::from_union(&session, &base, &dirs, timeout)
                .await
                .map(|out| {
                    (
                        Arc::new(out.set),
                        out.from_bus.len(),
                        out.dirs_only.len(),
                        out.disagreements.len(),
                    )
                })
                .map_err(|e| e.to_string())
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
            let roster = zenkey_fleet::roster(&session, &base, timeout)
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
            let out = zenkey_fleet::node_info(&session, &base, &origin, timeout, true)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string());
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
            let locals = match SliceSet::from_dirs(&dirs) {
                Ok(set) => set.slices().to_vec(),
                Err(e) => return Err(format!("registry dirs: {e}")),
            };
            zenkey_fleet::run_doctor(&session, &base, &locals, &spec)
                .await
                .map(|r| crate::doctor::DoctorRun {
                    report: Arc::new(r),
                    base,
                })
                .map_err(|e| e.to_string())
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
                .map_err(|e| e.to_string())?;
            let storages = zenkey_fleet::storages(&session, timeout)
                .await
                .map_err(|e| e.to_string())?;
            let declared = zenkey_fleet::declared_entities(&session, timeout)
                .await
                .map_err(|e| e.to_string())?;
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
                .map_err(|e| e.to_string())?;
            let origins = zenkey_fleet::origin_attachments(&session, &base, timeout)
                .await
                .map_err(|e| e.to_string())?;
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
            let out = zenkey_fleet::blob_probe(&session, &base, &target, &slices, timeout)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string());
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
            let out = zenkey_fleet::blob_tree_index(&session, &base, &origin, &root, timeout)
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string());
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

/// Download, reporting as it goes.
///
/// Progress arrives on a channel rather than through the return value: a
/// transfer that only reported at the end would leave the pane unable to say
/// anything true while it ran. The two halves are batched so they start
/// together.
pub fn blob_fetch(f: BlobFetch) -> Task<Message> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = Task::run(
        async_stream::stream! {
            let mut rx = rx;
            while let Some(p) = rx.recv().await {
                yield p;
            }
        },
        |p| Message::Pane(PaneMsg::Blob(BlobMsg::Progress(p))),
    );
    let run = Task::perform(
        async move {
            let BlobFetch {
                session,
                base,
                origin,
                target,
                dest,
                spec,
            } = f;
            let sink = move |p| {
                let _ = tx.send(p);
            };
            let out =
                zenkey_fleet::blob_fetch(&session, &base, &origin, &target, &dest, &spec, &sink)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string());
            (base, out)
        },
        |(ran, out)| Message::Pane(PaneMsg::Blob(BlobMsg::FetchDone(ran, out))),
    );
    Task::batch([progress, run])
}
