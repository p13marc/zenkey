//! Reading zk2 data through a contract, as a tool (spec §3.2, §4): the
//! runtime's `Consumer::for_tool` for current state (S4) and for a
//! subscription (R1, R6), and its `archive::last_known` for last-known
//! state (S5).
//!
//! **Current and last-known are different questions** (S6). [`get_state`]
//! asks the owner, with target `All` and consolidation `Latest` (the
//! runtime's S4 GET), and silence is no rows — never "no value".
//! [`last_known`] asks one archive, explicitly, and its report says
//! `last_known` in its first field: a tool never presents it as current.
//!
//! **A watch counts R6's discards apart.** A sample put on a wildcard key
//! is dropped by rule and counted by the runtime's subscription; samples
//! this tool could not keep up with are counted separately as lagged
//! (the tooling guide's O6). Neither is folded into the other.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use zenkey_model::contract::Resource;
use zenkey_model::grammar::{Addr, data_key};
use zenkey_model::template::Bindings;
use zenoh::Session;
use zenoh::sample::{Sample, SampleKind};

use crate::model::catalog::Revision;
use crate::model::render::{Member, render_with};
use crate::model::target::Target;
use crate::report::{
    Stamp, StateReading, StateReport, StateRow, StateValue, WatchEvent, WatchSample,
};
use crate::{Error, Result};

/// One state read's aim: the owner, the resource, the values given.
pub struct StateRead<'a> {
    pub revision: &'a Revision,
    /// The owner: one address.
    pub owner: &'a Addr,
    pub resource: &'a Resource,
    /// The template values given, unslugged; a parameter left out is a
    /// wildcard.
    pub values: &'a Bindings,
    pub timeout: Duration,
}

/// A timestamp and whose clock issued it (§4.1).
pub fn stamp(t: &zenoh::time::Timestamp) -> Stamp {
    Stamp {
        time: format!("{:#}", t.get_time()),
        clock: t.get_id().to_string(),
    }
}

fn runtime(op: &'static str, target: &str, e: zk2::Error) -> Error {
    match e {
        zk2::Error::Contract(_) | zk2::Error::NoResource { .. } | zk2::Error::Key(_) => {
            Error::unaskable(target, e.to_string())
        }
        other => Error::bus(op, target, other),
    }
}

/// The values a tool's consumer binds (R2): one per parameter. A rest
/// parameter's several chunks name one member, which only a fully bound
/// read can address.
fn r2_params(values: &Bindings) -> Result<BTreeMap<String, String>> {
    values
        .iter()
        .map(|(k, vs)| match vs.as_slice() {
            [v] => Ok((k.clone(), v.clone())),
            _ => Err(Error::unaskable(
                format!("--param {k}"),
                "several chunks name one member: give every other parameter too",
            )),
        })
        .collect()
}

/// Whether every template parameter of `r` is given: one member.
fn all_bound(r: &Resource, values: &Bindings) -> bool {
    r.template.params().all(|(n, _)| values.contains_key(n))
}

/// The owner's current state (S4): a GET on its keys for the resource,
/// target `All`, consolidation `Latest`, through the runtime's consumer.
/// No reply within the timeout is no rows: silence, never "no value" (S6).
pub async fn get_state(session: &Session, read: StateRead<'_>) -> Result<StateReport> {
    let StateRead {
        revision,
        owner,
        resource: r,
        values,
        timeout,
    } = read;
    let name = zk2::implementation::resource_name(r);
    let address = owner.to_string();
    let one = all_bound(r, values);
    let params = if one {
        BTreeMap::new()
    } else {
        r2_params(values)?
    };
    let consumer = zk2::consumer::Consumer::for_tool(
        session,
        revision.shared_contract(),
        &[&address],
        &params,
    )
    .map_err(|e| runtime("get", &address, e))?;
    let selectors = if one {
        vec![member_key(owner, revision, r, values)?]
    } else {
        consumer
            .selectors(&name)
            .map_err(|e| runtime("get", &address, e))?
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect()
    };
    let got = consumer
        .get(&name, one.then_some(values), timeout)
        .await
        .map_err(|e| runtime("get", &address, e))?;
    let mut rows: Vec<StateRow> = match got {
        zk2::state::StateGet::Silent => Vec::new(),
        zk2::state::StateGet::Answered(current) => current
            .into_iter()
            .map(|c| match c {
                zk2::state::Current::Value { key, sample } => StateRow {
                    timestamp: sample.timestamp().map(stamp),
                    value: StateValue::Value {
                        payload: Box::new(payload(revision, &key, &sample, Member::Type)),
                    },
                    key,
                    confirmed: None,
                    identity: None,
                },
                zk2::state::Current::Deleted { key, timestamp } => StateRow {
                    key,
                    value: StateValue::Deleted,
                    timestamp: timestamp.as_ref().map(stamp),
                    confirmed: None,
                    identity: None,
                },
            })
            .collect(),
    };
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(StateReport {
        reading: StateReading::Current,
        address,
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: name,
        values: values.clone(),
        selectors,
        archive: None,
        timeout_s: timeout.as_secs_f64(),
        rows,
    })
}

/// One member's key, every parameter given: `zk2/<owner>/<iface>/<token>/…`.
fn member_key(
    owner: &Addr,
    revision: &Revision,
    r: &Resource,
    values: &Bindings,
) -> Result<String> {
    let chunks = r
        .template
        .build(values)
        .map_err(|e| Error::unaskable(r.template.as_str(), e))?;
    let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
    data_key(owner, revision.iface(), r.token, &refs)
        .map(|k| k.as_str().to_owned())
        .map_err(|e| Error::unaskable_from(r.template.as_str(), e))
}

/// A key's last-known state, from the archive at `archive` (S5): one GET of
/// the archive's key for it, through the runtime's explicit read. Every
/// parameter must be given: an archive is read one key at a time. The
/// report says `last_known` before it says anything else (S6).
pub async fn last_known(
    session: &Session,
    read: StateRead<'_>,
    archive: &Addr,
) -> Result<StateReport> {
    let StateRead {
        revision,
        owner,
        resource: r,
        values,
        timeout,
    } = read;
    let name = zk2::implementation::resource_name(r);
    if !all_bound(r, values) {
        let missing: Vec<String> = r
            .template
            .params()
            .filter(|(n, _)| !values.contains_key(*n))
            .map(|(n, _)| format!("--param {n}=…"))
            .collect();
        return Err(Error::unaskable(
            "--last-known",
            format!(
                "an archive is read one key at a time (spec §4.4): give {}",
                missing.join(", ")
            ),
        ));
    }
    let origin = member_key(owner, revision, r, values)?;
    let selector = zk2::archive::archive_key(archive, &origin);
    let got = zk2::archive::last_known(session, archive, &origin, timeout)
        .await
        .map_err(|e| runtime("get", &selector, e))?;
    let rows = got
        .map(|lk| {
            let value = match &lk.value {
                Some(bytes) => {
                    let encoding = lk.encoding.as_ref().map(ToString::to_string);
                    StateValue::Value {
                        payload: Box::new(render_with(
                            revision,
                            &origin,
                            Member::Type,
                            encoding.as_deref(),
                            bytes,
                        )),
                    }
                }
                None => StateValue::Deleted,
            };
            StateRow {
                key: origin.clone(),
                value,
                timestamp: lk.timestamp.as_ref().map(stamp),
                confirmed: Some(lk.confirmed),
                identity: Some(lk.identity),
            }
        })
        .into_iter()
        .collect();
    Ok(StateReport {
        reading: StateReading::LastKnown,
        address: owner.to_string(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: name,
        values: values.clone(),
        selectors: vec![selector],
        archive: Some(archive.to_string()),
        timeout_s: timeout.as_secs_f64(),
        rows,
    })
}

/// A sample's payload, rendered through `member`'s type.
fn payload(
    revision: &Revision,
    key: &str,
    sample: &Sample,
    member: Member,
) -> crate::report::PayloadRendering {
    let encoding = sample.encoding().to_string();
    render_with(
        revision,
        key,
        member,
        Some(&encoding),
        &sample.payload().to_bytes(),
    )
}

/// How many delivered samples a [`Watch`] holds for its reader before it
/// counts the rest as lagged.
pub const WATCH_BUFFER: usize = 4096;

/// A subscription through a tool's consumer, delivering rendered samples.
/// Dropping it undeclares it.
pub struct Watch {
    sub: zk2::consumer::Subscription,
    rx: tokio::sync::mpsc::Receiver<WatchSample>,
    lagged: Arc<AtomicU64>,
    selectors: Vec<String>,
}

/// Subscribes to `r` across `target` (R1: at once, without presence), the
/// values given bound as R2 bindings and every other parameter a wildcard.
/// Each delivered sample is rendered through `revision` (R4: any revision
/// of the major reads the interface).
pub async fn watch(
    session: &Session,
    revision: &Revision,
    target: &Target,
    r: &Resource,
    values: &Bindings,
) -> Result<Watch> {
    let name = zk2::implementation::resource_name(r);
    let address = target.address.as_str();
    let params = r2_params(values)?;
    let consumer =
        zk2::consumer::Consumer::for_tool(session, revision.shared_contract(), &[address], &params)
            .map_err(|e| runtime("subscribe", address, e))?;
    let selectors = consumer
        .selectors(&name)
        .map_err(|e| runtime("subscribe", address, e))?
        .iter()
        .map(|k| k.as_str().to_owned())
        .collect();
    let (tx, rx) = tokio::sync::mpsc::channel(WATCH_BUFFER);
    let lagged = Arc::new(AtomicU64::new(0));
    let (rev, lag) = (revision.clone(), Arc::clone(&lagged));
    let sub = consumer
        .subscribe(&name, move |d: zk2::consumer::Delivery| {
            let key = d.sample.key_expr().as_str().to_owned();
            let event = match d.sample.kind() {
                SampleKind::Delete => WatchEvent::Delete,
                SampleKind::Put => WatchEvent::Put {
                    payload: Box::new(payload(&rev, &key, &d.sample, Member::Type)),
                    attachment: d.sample.attachment().map(|a| {
                        Box::new(render_with(
                            &rev,
                            &key,
                            Member::Attachment,
                            None,
                            &a.to_bytes(),
                        ))
                    }),
                },
            };
            let sample = WatchSample {
                provider: d.provider.to_string(),
                timestamp: d.sample.timestamp().map(stamp),
                key,
                values: d.values,
                event,
            };
            if tx.try_send(sample).is_err() {
                lag.fetch_add(1, Ordering::Relaxed);
            }
        })
        .await
        .map_err(|e| runtime("subscribe", address, e))?;
    Ok(Watch {
        sub,
        rx,
        lagged,
        selectors,
    })
}

impl Watch {
    /// The next delivered sample; `None` once the subscription is gone.
    pub async fn next(&mut self) -> Option<WatchSample> {
        self.rx.recv().await
    }

    /// Samples put on a wildcard key and discarded by rule (R6).
    pub fn discarded(&self) -> u64 {
        self.sub.discarded()
    }

    /// Samples dropped because the reader fell [`WATCH_BUFFER`] behind.
    pub fn lagged(&self) -> u64 {
        self.lagged.load(Ordering::Relaxed)
    }

    /// The key expressions subscribed, base-relative.
    pub fn selectors(&self) -> &[String] {
        &self.selectors
    }
}
