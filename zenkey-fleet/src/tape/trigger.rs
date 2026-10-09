//! Trigger capture (#218; RFC 13 §4.1 version 2, §4.3's pre-roll bullet):
//! leave a recorder armed with a small pre-roll and a condition, and get a
//! file only when something happens — with **the thirty seconds before it
//! fired** in it.
//!
//! The pre-roll is the monitor's retained window ([`crate::model::retain`],
//! #217), re-budgeted to `--pre` — a bounded ring on the ingest path, so
//! nothing is written while the rules are armed. The condition is the
//! watchdog's own vocabulary ([`Condition`]), judged tick by tick by a
//! [`RuleSet`] over **the same event stream the ring is fed from**: one
//! subscription, one drop ledger. That is the seam's whole reason for
//! existing — had this run a watchdog beside a recorder, the drops the
//! judge saw and the drops in the file would have been two different facts
//! about two different observers, and a `{"dropped": n}` in the file would
//! say nothing about whether the rule that fired was judged over a clean
//! window.
//!
//! The obstacle nobody else would notice: a pre-roll of a `state` fleet is
//! uninterpretable. Last-writer-wins keys arrive as deltas with no base —
//! the value that explains the incident was published an hour before the
//! window. So the capture carries a **state preamble**: a bounded fan-in
//! GET on the state-class projection of the watched selectors, taken at
//! trigger time, written as rows marked `"preamble": true` at `t: 0` ahead
//! of the pre-roll, each keeping the fetched value's HLC as provenance. The
//! header states what the preamble is a snapshot *of*
//! ([`PreambleSemantics`]): the values at the moment the ring began are
//! not recoverable, and the honest substitute has to be named rather than
//! implied.
//!
//! The file, in order: header → preamble rows → pre-roll rows (their real
//! `t`, the epoch being the ring's oldest arrival) → the trigger record →
//! the samples that arrived while the preamble was fetched and the file
//! opened → the post-roll, drained live through [`record`]. The ring keeps
//! moving under all of it; the snapshot is taken the instant the rule
//! fires, before the fetch, because the fetch takes as long as the fleet
//! takes to answer.
//!
//! **zk2** (#612, FJ8a, FJ8b). The capture is a `.zrec` version 3 (the
//! tooling guide's §5): the header's base is the namespace the operator
//! stated, and it says what the watched selectors exclude (O5). The rules
//! are the watchdog's zk2 vocabulary whole — `invalid-payload`,
//! `qos-mismatch` and `instance-gone` read the deployment through a lens
//! and a session in its namespace ([`TriggerSpec::deployment`]), refreshed
//! beside the drain like every other sweep. The preamble is the owners' own
//! answer (S4: target `All`, consolidation `Latest`, R6's discard) on the
//! plain-`state` projection of the watched selectors
//! ([`state_projection`]), never a storage's.

use std::collections::{BTreeSet, HashSet};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::SampleKind;

use crate::bus::monitor::{FleetEvent, SampleView, StreamItem};
use crate::judge::condition::{Condition, RuleSet, SweepOutcome};
use crate::model::catalog::{Catalog, ContractSet};
use crate::model::lens::Lens;
use crate::model::retain::RetentionBudget;
use crate::report::{
    CondState, PreRollInfo, PreambleInfo, PreambleSemantics, RecordReport, Transition, ZrecHeader,
};
use crate::tape::record::{RecordBounds, ZrecSink, record};
use crate::{Error, Result};

/// What a trigger capture watches, judges, and keeps.
#[derive(Debug, Clone)]
pub struct TriggerSpec {
    /// Full wire selectors to retain and record. The rules' own selectors
    /// are watched too; the header names the union (O5).
    pub selectors: Vec<String>,
    /// How far back the retained window reaches: the pre-roll asked for.
    pub pre: Duration,
    /// How long to keep recording after the trigger.
    pub post: Duration,
    /// The rules; the first transition **to** `firing` on any of them fires
    /// the capture.
    pub rules: Vec<Condition>,
    /// Evaluation cadence — the watchdog's `--every`.
    pub tick: Duration,
    /// Per-ask timeout: the presence, contract and doctor sweeps, and the
    /// preamble GET.
    pub timeout: Duration,
    /// The deployment's sessions (#612, FJ6, FJ8b): where a `doctor` rule's
    /// doctor reads, an `instance-gone` rule's presence, and the lens
    /// `invalid-payload` and `qos-mismatch` judge through. `None` makes
    /// each of those rules unobservable, saying why.
    pub deployment: Option<crate::judge::doctor::DoctorBus>,
    /// Revisions known offline (`--contracts`): never retrieved.
    pub contracts: ContractSet,
    /// Stop waiting after this long with nothing fired; `None` waits until
    /// the caller stops the future.
    pub give_up: Option<Duration>,
    /// What the preamble is a snapshot of; `None` writes no preamble at all.
    pub preamble: Option<PreambleSemantics>,
    /// Stop the post-roll after this many observed samples (the pre-roll
    /// and the preamble never count towards it).
    pub max_samples: Option<u64>,
    /// Replies kept per preamble GET (#339); what the bound cost rides the
    /// header's `incomplete`.
    pub max_replies: usize,
}

/// What a trigger capture reports as it runs — the frontend renders these;
/// the [`RecordReport`] at the end carries the counts.
#[derive(Debug, Clone)]
pub enum TriggerEvent<'a> {
    /// The subscriptions are declared and the ring is filling: the watch
    /// set, and the pre-roll it is budgeted to.
    Armed {
        watched: &'a [String],
        pre: Duration,
    },
    /// A rule changed state (every genuine change, `firing` or not).
    Transition(&'a Transition),
    /// The capture fired on this transition: the ring is snapshotted and
    /// the preamble is being fetched.
    Fired(&'a Transition),
    /// The preamble is written, as the header states it.
    Preamble(&'a PreambleInfo),
    /// Post-roll progress: observed samples and drops queued so far.
    Progress { samples: u64, dropped: u64 },
    /// Nothing fired within `give_up`; nothing was written.
    GaveUp { after: Duration },
}

/// How many samples the post-fire buffer holds while the preamble is
/// fetched and the file opened — the broadcast's default capacity, four
/// times over, as the sink's own queue is. Past it the buffer records a
/// drop where the loss happened rather than growing without bound.
const FIRE_BUFFER: usize = 4096;

/// The state projection of a wire selector under `namespace` (#612, FJ8a):
/// the selector narrowed to zk2's state keys (spec §1.1, §1.3), or `None`
/// when it reaches none.
///
/// Positions 1–5 of a zk2 key have fixed arity, so the kind sits at
/// position 5: `zk2/**`, `zk2/<system>/**`, `zk2/<system>/<service>/**` and
/// `zk2/<s>/<v>/<iface>/**` widen to `…/state/**` with each unnamed
/// position `*` (a `**` that spans the kind spans plain `state`, and never
/// `@state`, which no wildcard matches); a selector whose kind is `state`,
/// `@state` or `*` keeps its tail, `*` becoming `state`. A selector naming
/// another kind, a control key (`@zk`), one too short to reach a kind, or
/// one with `**` before the kind yields `None` — which the caller records
/// under `failed`, because "no preamble for this watch" is a fact about the
/// watch, not silence. A selector outside `namespace` is another
/// deployment's and yields `None` too.
pub fn state_projection(namespace: &str, selector: &str) -> Option<String> {
    let rel = zenkey::grammar::strip_base(namespace, selector)?;
    let chunks: Vec<&str> = rel.split('/').collect();
    let named = |c: &str| c != "**" && !c.starts_with('@');
    let projected: Vec<String> = match chunks.as_slice() {
        ["**"] | ["zk2", "**"] => vec!["zk2", "*", "*", "*", "state", "**"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ["zk2", head @ .., "**"] if head.len() <= 3 && head.iter().all(|c| named(c)) => {
            let mut v = vec!["zk2".to_owned()];
            v.extend(head.iter().map(|c| (*c).to_owned()));
            while v.len() < 4 {
                v.push("*".to_owned());
            }
            v.extend(["state".to_owned(), "**".to_owned()]);
            v
        }
        ["zk2", system, service, iface, kind, rest @ ..]
            if [system, service, iface].iter().all(|c| named(c))
                && matches!(*kind, "state" | "@state" | "*") =>
        {
            let kind = if *kind == "*" { "state" } else { kind };
            let mut v: Vec<String> = ["zk2", system, service, iface, kind]
                .iter()
                .map(|c| (*c).to_owned())
                .collect();
            v.extend(rest.iter().map(|c| (*c).to_owned()));
            if rest.is_empty() {
                v.push("**".to_owned());
            }
            v
        }
        _ => return None,
    };
    Some(zenkey::grammar::with_base(namespace, projected.join("/")))
}

/// The owners' current state under `selector` (S4): target `All`,
/// consolidation `Latest`, each reply on a concrete key (R6). A deletion
/// within an owner's window is not a value, and is left out. Returns the
/// values and the error replies counted.
async fn state_get(
    session: &Session,
    selector: &str,
    timeout: Duration,
    max_replies: usize,
) -> Result<(Vec<SampleView>, u64)> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .await
        .map_err(|e| Error::bus("state preamble", selector, e))?;
    let (mut values, mut errors) = (Vec::new(), 0u64);
    while let Ok(reply) = replies.recv_async().await {
        match reply.into_result() {
            Ok(s) if s.kind() == SampleKind::Put && !s.key_expr().is_wild() => {
                if values.len() < max_replies {
                    values.push(SampleView::of(&s));
                } else {
                    errors += 1;
                }
            }
            Ok(_) => {}
            Err(_) => errors += 1,
        }
    }
    Ok((values, errors))
}

/// The watch set: every selector asked for, minus any that another one in
/// the set already includes.
///
/// A union would be wrong here, not merely wasteful. Each watched selector
/// is its own subscriber feeding one ring, and a sample matching two of
/// them is delivered twice — so `--on 'silent-for v1/x/state/p/health 30'`
/// under a `v1/**` watch would put every `health` sample into the file
/// twice and count it twice for every rate rule. The narrower expression is
/// dropped; the wider one still reaches every key the rule judges. Order is
/// first-seen, so the header's coverage statement (O5) reads as the operator
/// wrote it.
pub fn watch_cover<'s>(selectors: impl IntoIterator<Item = &'s String>) -> Vec<String> {
    let mut distinct: Vec<String> = Vec::new();
    for sel in selectors {
        if !distinct.contains(sel) {
            distinct.push(sel.clone());
        }
    }
    let parsed: Vec<Option<zenoh::key_expr::KeyExpr<'static>>> = distinct
        .iter()
        .map(|s| zenoh::key_expr::KeyExpr::try_from(s.clone()).ok())
        .collect();
    distinct
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let Some(mine) = &parsed[*i] else { return true };
            !parsed
                .iter()
                .enumerate()
                .any(|(j, other)| j != *i && other.as_ref().is_some_and(|o| o.includes(mine)))
        })
        .map(|(_, s)| s.clone())
        .collect()
}

/// Arm the rules over the retained window and, when one fires, write the
/// capture: preamble, pre-roll, trigger, post-roll.
///
/// `open` is called **only when something fired** — a run that gives up
/// leaves no file behind, and the report says so (`out: None`,
/// `trigger: None`; a rule not firing is not a finding). The header names
/// the watch set and both version-2 blocks; `pre_roll.covered_s` is what
/// the ring actually held, which is less than `spec.pre` while the ring is
/// still filling or when its byte budget bit, and both are said (O6).
///
/// Every await inside the drain is the watchdog's (#338): the sweep and the
/// preamble fetch run *beside* the drain, never instead of it, so the drops
/// in the file are the bus's and not this loop's own.
///
/// `session` is the capture's own, in no namespace (the selectors are wire
/// keys, and rows keep them whole); `namespace` is the one the operator
/// stated, which the header carries as its base (#612, FJ8a).
pub async fn record_on<W, F>(
    session: &Session,
    namespace: &str,
    spec: &TriggerSpec,
    open: impl FnOnce() -> F,
    mut on_event: impl FnMut(TriggerEvent<'_>),
) -> Result<RecordReport>
where
    W: Write + Send + 'static,
    F: std::future::Future<Output = Result<W>>,
{
    if spec.rules.is_empty() {
        return Err(Error::unaskable(
            "--on",
            "a trigger capture needs at least one rule to fire on",
        ));
    }

    // Compiled before the monitor exists, so the `?` has nothing to tear
    // down (#336).
    let mut rules = RuleSet::new(&spec.rules)?;
    let watched = watch_cover(spec.selectors.iter().chain(rules.watched()));
    let (wants_doctor, wants_lens) = (rules.wants_doctor(), rules.wants_lens());
    let addresses = rules.instance_addresses();
    // The doctor rules' doctor (#612, FJ6): only the checks they name, one
    // contract store for the capture, seeded with what `--contracts` holds.
    let doctor_spec = rules.doctor_spec(spec.timeout);
    let bundles = crate::bus::contracts::BundleStore::new(spec.timeout);
    bundles.seed(&spec.contracts);
    let deployment = spec.deployment.as_ref();
    // The lens before the first window (FJ8b): a check inside the drain
    // never waits on a presence read or a retrieval.
    let mut catalog: Option<Catalog> = match (wants_lens, deployment) {
        (true, Some(bus)) => crate::bus::lens::read(&bus.session, &bundles, spec.timeout)
            .await
            .ok(),
        _ => None,
    };

    // The ring is budgeted to the pre-roll **before** anything is watched:
    // `set_budget` applies from the next push, and the first push must
    // already be under the window this capture claims.
    let monitor = crate::Monitor::start(session, crate::MonitorSpec::default()).await?;
    monitor.core().set_retention_budget(RetentionBudget {
        max_age: spec.pre,
        ..RetentionBudget::default()
    });
    let mut events = monitor.events();
    let monitor = monitor.watching(&watched).await?;
    let core = Arc::clone(monitor.core());
    on_event(TriggerEvent::Armed {
        watched: &watched,
        pre: spec.pre,
    });

    let armed = tokio::time::Instant::now();
    let give_up_at = spec.give_up.map(|d| armed + d);

    // ── armed: judge tick by tick, write nothing ──────────────────────────
    let fired: Option<Transition> = 'armed: loop {
        let deadline = rules.last_eval() + spec.tick;
        // The sweep asks what the rules need, beside the drain: the doctor,
        // the instance tokens, a fresh lens.
        let sweep = async {
            let doctor = if wants_doctor {
                Some(crate::judge::condition::tick_doctor(deployment, &bundles, &doctor_spec).await)
            } else {
                None
            };
            let instances = if addresses.is_empty() {
                None
            } else {
                Some(
                    crate::judge::condition::tick_instances(deployment, &addresses, spec.timeout)
                        .await,
                )
            };
            let lens = match (wants_lens, deployment) {
                (true, Some(bus)) => crate::bus::lens::read(&bus.session, &bundles, spec.timeout)
                    .await
                    .ok(),
                _ => None,
            };
            (doctor, instances, lens)
        };
        let mut sweep = std::pin::pin!(sweep);
        let mut swept = None;
        let tick_over = tokio::time::sleep_until(deadline);
        tokio::pin!(tick_over);
        let give_up = async {
            match give_up_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(give_up);
        let mut closed = false;
        let mut gave_up = false;
        while !closed {
            let item = tokio::select! {
                item = events.recv() => item,
                (doctor, instances, lens) = &mut sweep, if swept.is_none() => {
                    if let Some(c) = lens {
                        catalog = Some(c);
                    }
                    swept = Some((doctor, instances));
                    continue;
                }
                () = &mut tick_over, if swept.is_some() => break,
                () = &mut give_up => {
                    gave_up = true;
                    break;
                }
            };
            match item {
                Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                    let lens =
                        Lens::new(namespace, catalog.as_ref(), &bundles).offline(&spec.contracts);
                    rules.observe_sample(&s, &lens);
                }
                Some(StreamItem::Dropped(n)) => rules.observe_drop(n),
                Some(_) => {}
                None => closed = true,
            }
        }
        if gave_up {
            break 'armed None;
        }
        if closed {
            // The stream closed under the rules: nothing can fire on a
            // stream that is gone, and nothing was written.
            break 'armed None;
        }
        let (doctor_outcome, instance_asks) = match swept {
            Some(outcome) => outcome,
            None => {
                let (doctor, instances, lens) = sweep.await;
                if let Some(c) = lens {
                    catalog = Some(c);
                }
                (doctor, instances)
            }
        };
        let now = tokio::time::Instant::now();
        let at = crate::tape::record::rfc3339_now();
        let transitions = rules.evaluate(
            now,
            &at,
            SweepOutcome {
                doctor: doctor_outcome
                    .as_ref()
                    .map(|o| o.as_ref().map_err(String::as_str)),
                instances: instance_asks.as_deref(),
            },
        );
        for t in transitions {
            on_event(TriggerEvent::Transition(&t));
            if t.to == CondState::Firing {
                break 'armed Some(t);
            }
        }
    };

    let Some(trigger) = fired else {
        monitor.shutdown().await?;
        let after = armed.elapsed();
        on_event(TriggerEvent::GaveUp { after });
        return Ok(RecordReport {
            header: ZrecHeader::capture(watched, namespace),
            out: None,
            samples: 0,
            dropped: 0,
            duration_ms: u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
            trigger: None,
            preamble: None,
            pre_roll: None,
            preamble_rows: 0,
        });
    };

    // ── fired: the ring is the pre-roll, taken now ────────────────────────
    on_event(TriggerEvent::Fired(&trigger));
    let fired_at = Instant::now();
    let captured_at = crate::tape::record::rfc3339_now();
    let ring = core.retained();
    let stats = core.retention();
    let epoch = ring.first().map_or(fired_at, |v| v.received);
    let pre_roll = PreRollInfo {
        asked_s: spec.pre.as_secs_f64(),
        covered_s: stats.span.as_secs_f64(),
        watched: watched.clone(),
        evicted: stats.evicted,
        expired: stats.expired,
    };

    // The broadcast runs behind the ring (the ring is on the ingest path),
    // so the first items drained from here on are samples the ring already
    // holds. They are the same `Arc`s: the newest broadcast-capacity-worth
    // of ring pointers is the exact set to skip.
    // Addresses, not pointers: the set crosses an `.await`, and a raw
    // pointer would make this future `!Send` for no reason — nothing is
    // ever dereferenced through it.
    let already: HashSet<usize> = ring
        .iter()
        .rev()
        .take(crate::MonitorSpec::default().capacity)
        .map(|v| Arc::as_ptr(v) as usize)
        .collect();
    let mut buffered: Vec<StreamItem> = Vec::new();
    let mut buffer_dropped = 0u64;
    let drain_into =
        |item: Option<StreamItem>, buffered: &mut Vec<StreamItem>, dropped: &mut u64| match item {
            Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                if already.contains(&(Arc::as_ptr(&s) as usize)) {
                    return;
                }
                if buffered.len() < FIRE_BUFFER {
                    buffered.push(StreamItem::Event(FleetEvent::Sample(s)));
                } else {
                    *dropped += 1;
                }
            }
            Some(StreamItem::Dropped(n)) => buffered.push(StreamItem::Dropped(n)),
            _ => {}
        };

    // The preamble fetch runs beside the drain, like a sweep. Nothing about
    // it stops sampling: the samples that arrive meanwhile are buffered and
    // land in the file after the trigger record, where they happened.
    let preamble = async {
        let semantics = spec.preamble?;
        let started = Instant::now();
        let mut selectors = Vec::new();
        let mut failed = Vec::new();
        for sel in &watched {
            match state_projection(namespace, sel) {
                Some(p) if !selectors.contains(&p) => selectors.push(p),
                Some(_) => {}
                None => failed.push(sel.clone()),
            }
        }
        // The owners' answer (S4), never a storage's: one GET per projected
        // selector, together.
        let gets = futures_util::future::join_all(selectors.iter().map(|selector| async move {
            (
                selector.clone(),
                state_get(session, selector, spec.timeout, spec.max_replies).await,
            )
        }))
        .await;
        let mut values: Vec<SampleView> = Vec::new();
        let mut errors = 0u64;
        for (selector, replies) in gets {
            match replies {
                Ok((v, e)) => {
                    errors += e;
                    values.extend(v);
                }
                Err(e) => {
                    tracing::warn!(selector, error = %e, "preamble GET could not be issued");
                    failed.push(selector);
                }
            }
        }
        // What the ring cannot tell you, fetched at trigger time: a key the
        // ring holds already has its story in the pre-roll rows, and a
        // preamble that repeated it would put a newer value *before* the
        // deltas that led to it. `Latest` kept one value per key; two
        // overlapping selectors could still both hold one, kept once.
        let in_ring: BTreeSet<&str> = ring.iter().map(|v| v.key.as_str()).collect();
        let mut seen = BTreeSet::new();
        let rows: Vec<Arc<SampleView>> = values
            .into_iter()
            .filter(|view| seen.insert(view.key.clone()))
            .filter(|view| match semantics {
                PreambleSemantics::AbsentFromWindow => !in_ring.contains(view.key.as_str()),
                PreambleSemantics::Full => true,
            })
            .map(Arc::new)
            .collect();
        Some((
            PreambleInfo {
                count: rows.len() as u64,
                collected_over_s: started.elapsed().as_secs_f64(),
                selectors,
                semantics,
                incomplete: errors,
                failed,
            },
            rows,
        ))
    };
    let mut preamble = std::pin::pin!(preamble);
    let fetched = loop {
        tokio::select! {
            item = events.recv() => drain_into(item, &mut buffered, &mut buffer_dropped),
            fetched = &mut preamble => break fetched,
        }
    };
    let (preamble_info, preamble_rows) = match fetched {
        Some((info, rows)) => (Some(info), rows),
        None => (None, Vec::new()),
    };
    if let Some(info) = &preamble_info {
        on_event(TriggerEvent::Preamble(info));
    }

    // ── the file ──────────────────────────────────────────────────────────
    let header = ZrecHeader {
        captured_at,
        preamble: preamble_info.clone(),
        pre_roll: Some(pre_roll.clone()),
        ..ZrecHeader::capture(watched.clone(), namespace)
    };
    // Opened only now: a run that never fires leaves nothing behind. The
    // open itself is the caller's (it names the path) and async, so a
    // `create` can go through `tokio::fs` (#332); the broadcast's own
    // capacity covers its length, and the drain below picks the backlog up.
    let out = match open().await {
        Ok(out) => out,
        Err(e) => {
            if let Err(teardown) = monitor.shutdown().await {
                tracing::warn!("after a failed open: {teardown}");
            }
            return Err(e);
        }
    };
    let sink = ZrecSink::spawn_at(out, &header, epoch).await?;
    for row in &preamble_rows {
        sink.write_preamble(Arc::clone(row)).await?;
    }
    for view in ring.iter() {
        sink.write_sample(Arc::clone(view)).await?;
    }
    sink.write_trigger(trigger.clone()).await?;
    // Whatever the broadcast holds *now* is still pre-open backlog: drain it
    // without waiting, so the post-roll starts from a current stream.
    while let Ok(item) = tokio::time::timeout(Duration::ZERO, events.recv()).await {
        drain_into(item, &mut buffered, &mut buffer_dropped);
    }
    for item in buffered.drain(..) {
        match item {
            StreamItem::Event(FleetEvent::Sample(s)) => sink.write_sample(s).await?,
            StreamItem::Dropped(n) => sink.write_dropped(n).await?,
            StreamItem::Event(_) => {}
        }
    }
    if buffer_dropped > 0 {
        sink.write_dropped(buffer_dropped).await?;
    }

    // ── the post-roll, live ───────────────────────────────────────────────
    let pre_written = sink.counts().samples;
    let bounds = RecordBounds {
        max_samples: spec.max_samples.map(|n| n + pre_written),
        max_duration: Some(spec.post),
    };
    let mut last_line = Instant::now();
    let recorded = record(&mut events, &sink, bounds, |samples, dropped| {
        if last_line.elapsed() >= Duration::from_secs(1) {
            last_line = Instant::now();
            on_event(TriggerEvent::Progress { samples, dropped });
        }
    })
    .await;
    // Teardown before the report, on every path out.
    let closed = monitor.shutdown().await;
    recorded?;
    let counts = sink.finish().await?;
    closed?;
    Ok(RecordReport {
        header,
        out: None,
        samples: counts.samples,
        dropped: counts.dropped,
        duration_ms: u64::try_from((stats.span + fired_at.elapsed()).as_millis())
            .unwrap_or(u64::MAX),
        trigger: Some(trigger),
        preamble: preamble_info,
        pre_roll: Some(pre_roll),
        preamble_rows: counts.preamble,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule selector the watch already includes is not declared twice —
    /// that would deliver every matching sample twice into one ring — and
    /// a wider rule selector supersedes the narrower watch the same way.
    #[test]
    fn the_watch_cover_declares_no_included_selector_twice() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            watch_cover(&s(&["v1/**", "v1/h-aaaaaaaaaaaa/state/p/health"])),
            s(&["v1/**"])
        );
        assert_eq!(
            watch_cover(&s(&["v1/h-aaaaaaaaaaaa/state/p/health", "v1/**"])),
            s(&["v1/**"])
        );
        assert_eq!(
            watch_cover(&s(&["v1/a/state/**", "v1/b/state/**", "v1/a/state/**"])),
            s(&["v1/a/state/**", "v1/b/state/**"])
        );
        // Two spellings of one set: the first stays, the identical second
        // is a duplicate, and an equal-but-distinct expression that includes
        // the first is kept (it includes; it is not included).
        assert_eq!(watch_cover(&s(&["v1/x/**", "v1/x/**"])), s(&["v1/x/**"]));
    }

    /// zk2's projection (FJ8a): a `**` before the kind widens to plain
    /// `state` with every unnamed position `*`; a kind of `state`, `@state`
    /// or `*` keeps its tail; another kind, a control key, a `**` among the
    /// named positions or a selector too short to reach a kind reaches no
    /// state — never a guess.
    #[test]
    fn the_state_projection_narrows_widens_or_declines() {
        let p = |s| state_projection("", s);
        assert_eq!(p("zk2/**").as_deref(), Some("zk2/*/*/*/state/**"));
        assert_eq!(p("**").as_deref(), Some("zk2/*/*/*/state/**"));
        assert_eq!(
            p("zk2/host-a/**").as_deref(),
            Some("zk2/host-a/*/*/state/**")
        );
        assert_eq!(
            p("zk2/host-a/tc/**").as_deref(),
            Some("zk2/host-a/tc/*/state/**")
        );
        assert_eq!(
            p("zk2/*/tc/tc.netif.v1/**").as_deref(),
            Some("zk2/*/tc/tc.netif.v1/state/**")
        );
        assert_eq!(
            p("zk2/host-a/tc/tc.netif.v1/*/interfaces/**").as_deref(),
            Some("zk2/host-a/tc/tc.netif.v1/state/interfaces/**")
        );
        assert_eq!(
            p("zk2/host-a/tc/tc.netif.v1/state/namespaces").as_deref(),
            Some("zk2/host-a/tc/tc.netif.v1/state/namespaces")
        );
        assert_eq!(
            p("zk2/host-a/cam/camera.v1/@state").as_deref(),
            Some("zk2/host-a/cam/camera.v1/@state/**")
        );
        assert_eq!(p("zk2/host-a/tc/tc.netif.v1/stream/**"), None);
        assert_eq!(p("zk2/host-a/tc/tc.netif.v1/@op/diagnostics"), None);
        assert_eq!(
            p("zk2/host-a/tc/@zk/**"),
            None,
            "control keys hold no state"
        );
        assert_eq!(p("zk2/**/state/x"), None);
        assert_eq!(p("zk2/host-a/tc"), None);
        assert_eq!(p("v1/**"), None, "a v1 key is not zk2's");
        // Under a namespace, the namespace rides back out — and another
        // deployment's selector is not projected at all.
        assert_eq!(
            state_projection("prod", "prod/zk2/**").as_deref(),
            Some("prod/zk2/*/*/*/state/**")
        );
        assert_eq!(state_projection("prod", "other/zk2/**"), None);
    }

    /// Every zk2 rule arms a capture now (FJ8b): the three FJ8a refused —
    /// `invalid-payload`, `qos-mismatch` and v1's `origin-down` as zk2's
    /// `instance-gone` — parse and compile into one rule set; v1's
    /// `alert-firing` is dark (#613) and refused at the parse.
    #[test]
    fn every_zk2_condition_arms_a_capture() {
        let rules: Vec<Condition> = [
            "rate-above zk2/** 10",
            "silent-for zk2/** 5",
            "dropped",
            "doctor split-brain",
            "invalid-payload zk2/**",
            "qos-mismatch zk2/**",
            "instance-gone host-a/tc",
        ]
        .iter()
        .map(|s| Condition::parse(s).expect(s))
        .collect();
        let set = RuleSet::new(&rules).expect("they compile");
        assert!(set.wants_lens());
        assert_eq!(set.instance_addresses(), ["host-a/tc"]);
        let e = Condition::parse("alert-firing zk2/**").unwrap_err();
        assert!(e.is_unaskable() && e.to_string().contains("#613"), "{e}");
    }
}
