//! `check probe` (#59; zk2's since #612, FJ8b): read a zk2 resource the way a
//! consumer does, and judge whether values arrive within a window.
//!
//! v1's probe was RFC 09 §6's consumer-shaped acceptance: "a probe MUST
//! build its keys the way the product builds them". A zk2 consumer binds a
//! role to providers (spec §3.2 R1) and reads through the runtime's
//! consumer, so this probe does exactly that — `Consumer::for_tool` with
//! the address as its binding, the template values as R2 bindings — and
//! for a state resource reads the owner's current state first (S4), as a
//! consumer of state does, before it follows the puts.
//!
//! The question is "do values arrive within the window?", named for the
//! sought answer, so the finding is the *no* (the tooling guide's §1
//! polarity):
//!
//! - a value arrived that decodes as its declared type: established clean,
//!   exit 0;
//! - values arrived and none conforms to its declared type: the finding —
//!   a consumer cannot use them;
//! - nothing arrived, and a complete presence read shows a provider
//!   holding the interface's token: attributable silence, the finding
//!   (§2: a bounded window, a named party, an independent check that it
//!   could speak);
//! - nothing arrived, and a complete presence read shows no token visible
//!   to this reader: the finding too, worded as what this reader could see;
//! - nothing arrived and the presence read ended at its timeout, or could
//!   not be made: unobservable, exit 2.

use std::collections::BTreeSet;
use std::time::Duration;

use zenkey_model::authoring::Kind;
use zenkey_model::contract::Resource;
use zenkey_model::template::Bindings;
use zenoh::Session;

use crate::Result;
use crate::judge::expect::ExpectAim;
use crate::model::catalog::Revision;
use crate::model::target::Target;
use crate::report::{
    Conformance, ExpectPresence, Judgement, ProbeCurrent, ProbeReport, WatchEvent, WatchSample,
};

/// Probe one resource for `window`, on a session in the deployment's
/// namespace. `Err` is a subscription that could not be declared: the
/// probe never stood up.
pub async fn run_probe(
    session: &Session,
    aim: ExpectAim<'_>,
    window: Duration,
    timeout: Duration,
) -> Result<ProbeReport> {
    let ExpectAim {
        revision,
        target,
        resource,
        values,
    } = aim;
    let Some(r) = resource else {
        return Err(crate::Error::unaskable(
            "check probe",
            "a probe reads a resource: name one",
        ));
    };
    // Subscribe first, then read: a put between the two is not lost.
    let mut watch = crate::bus::consume::watch(session, revision, target, r, values).await?;
    let current = if r.kind == Kind::State {
        Some(current_state(session, revision, target, r, values, timeout).await)
    } else {
        None
    };
    let opened = tokio::time::Instant::now();
    let (mut received, mut conforming, mut nonconforming) = (0u64, 0u64, 0u64);
    let mut first: Option<WatchSample> = None;
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let usable_now = current.as_ref().is_some_and(|c| c.conforming > 0);
    if !usable_now {
        let window_over = tokio::time::sleep_until(opened + window);
        tokio::pin!(window_over);
        loop {
            tokio::select! {
                next = watch.next() => {
                    let Some(s) = next else { break };
                    received += 1;
                    keys.insert(s.key.clone());
                    let usable = match &s.event {
                        WatchEvent::Put { conformance, .. } => usable(conformance),
                        // A deletion is a value of state's: the key is gone.
                        WatchEvent::Delete => true,
                    };
                    if usable {
                        conforming += 1;
                    } else {
                        nonconforming += 1;
                    }
                    if first.is_none() || (usable && conforming == 1) {
                        first = Some(s);
                    }
                    if usable {
                        break;
                    }
                }
                () = &mut window_over => break,
            }
        }
    }
    let (lagged, discarded, unresolved, selectors) = (
        watch.lagged(),
        watch.discarded(),
        watch.unresolved(),
        watch.selectors().to_vec(),
    );
    drop(watch);
    let arrived = conforming > 0 || usable_now;
    let any_value = arrived
        || nonconforming > 0
        || lagged > 0
        || current.as_ref().is_some_and(|c| c.answered > 0);
    // Silence is attributed only through presence (§2).
    let presence = if any_value {
        None
    } else {
        Some(presence(session, revision, target, timeout).await)
    };
    let verdict = if arrived {
        Judgement::NotEstablished {
            reason: "a value that conforms to its declared type arrived within the window".into(),
        }
    } else if lagged > 0 {
        Judgement::Unobservable {
            reason: format!(
                "{lagged} value(s) arrived past this tool's buffer and were not inspected: one \
                 may have conformed (O6)"
            ),
        }
    } else if any_value {
        Judgement::Established
    } else {
        match &presence {
            Some(p) if p.error.is_none() && p.complete => Judgement::Established,
            Some(ExpectPresence { error: Some(e), .. }) => Judgement::Unobservable {
                reason: format!("nothing arrived, and presence could not be read: {e}"),
            },
            _ => Judgement::Unobservable {
                reason: "nothing arrived, and the presence read ended at its timeout: a \
                         provider it did not see may be there (§8.1)"
                    .into(),
            },
        }
    };
    Ok(ProbeReport {
        address: target.address.clone(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: zk2::implementation::resource_name(r),
        selectors,
        window_s: window.as_secs_f64(),
        elapsed_s: opened.elapsed().as_secs_f64(),
        current,
        received,
        conforming,
        nonconforming,
        keys_seen: keys.len(),
        lagged,
        discarded,
        unresolved,
        first: first.map(Box::new),
        presence,
        verdict,
    })
}

/// Whether a consumer can use a payload: it decoded as its declared type
/// (and satisfies a JSON Schema type), or the type is raw, which a consumer
/// reads as bytes.
fn usable(c: &Conformance) -> bool {
    match c {
        Conformance::Valid => true,
        Conformance::NotChecked { reason } => reason.contains("raw type"),
        Conformance::Invalid { .. } | Conformance::Undecodable { .. } => false,
    }
}

/// The owner's current state (S4), as a consumer of state reads it first.
async fn current_state(
    session: &Session,
    revision: &Revision,
    target: &Target,
    r: &Resource,
    values: &Bindings,
    timeout: Duration,
) -> ProbeCurrent {
    let Some(owner) = &target.concrete else {
        return ProbeCurrent {
            answered: 0,
            conforming: 0,
            error: Some(
                "a pattern names no owner: current state is the owner's answer (S4), read \
                 per address"
                    .into(),
            ),
        };
    };
    let read = crate::bus::consume::StateRead {
        revision,
        owner,
        resource: r,
        values,
        timeout,
    };
    match crate::bus::consume::get_state(session, read).await {
        Ok(report) => ProbeCurrent {
            answered: report.rows.len(),
            conforming: report
                .rows
                .iter()
                .filter(|row| match &row.value {
                    crate::report::StateValue::Value { payload } => {
                        usable(&crate::model::lens::conformance(revision, payload))
                    }
                    crate::report::StateValue::Deleted => true,
                })
                .count(),
            error: None,
        },
        Err(e) => ProbeCurrent {
            answered: 0,
            conforming: 0,
            error: Some(crate::one_line(&e)),
        },
    }
}

/// Who holds the interface's token among the bound providers (§8.1).
async fn presence(
    session: &Session,
    revision: &Revision,
    target: &Target,
    timeout: Duration,
) -> ExpectPresence {
    let selector = format!("zk2/{}/@zk/alive/{}/**", target.address, revision.iface());
    match crate::bus::presence::liveliness_read(session, &selector, timeout).await {
        Ok(r) => ExpectPresence {
            holders: r
                .keys
                .iter()
                .filter_map(|k| match zenkey_model::grammar::parse(k) {
                    Ok(zenkey_model::grammar::ZkKey::Alive { addr, .. }) => Some(addr.to_string()),
                    _ => None,
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            complete: r.complete,
            error: None,
            selector,
        },
        Err(e) => ExpectPresence {
            holders: Vec::new(),
            complete: false,
            error: Some(crate::one_line(&e)),
            selector,
        },
    }
}
