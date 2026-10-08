//! Reading presence (spec §8.1): liveliness GETs, parsed tokens, and
//! descriptors.
//!
//! Every liveliness GET here runs on an **unbounded** handler. zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2), and the spec forbids it. The flume channel's
//! sender is dropped when the query finalizes, which is how a GET here knows
//! it has ended.
//!
//! **How it ended** is told by its error replies, which are counted and
//! reported, never dropped (#660). zenoh 1.10.1 ends a liveliness GET that
//! reaches its timeout with an error reply, `zenoh/string` `Timeout`, and
//! one that the routers finished with no reply at all; any error reply
//! leaves the read possibly incomplete (§8.1).

use std::time::Duration;

use zenkey_model::descriptor::{self, Descriptor};
use zenkey_model::grammar::{Addr, InstanceId, ZkKey, instance_key, parse};
use zenoh::query::{ConsolidationMode, Reply};

use crate::error::{Error, Result, zenoh};

/// A presence read, and whether it is complete (§8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceRead {
    /// The token keys, sorted.
    pub keys: Vec<String>,
    /// `false` when the GET ended at its timeout, or with any other error
    /// reply: it may have missed tokens, and a tool reports the result as
    /// possibly incomplete (§8.1), never as absence (O5).
    pub complete: bool,
    /// The GET's error replies, in arrival order, each
    /// `<encoding>: <payload>`: `zenoh/string: Timeout` for one that ended
    /// at its timeout.
    pub errors: Vec<String>,
}

/// The keys of every liveliness token matching `selector`, sorted.
pub async fn liveliness_keys(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
) -> Result<Vec<String>> {
    Ok(liveliness_read(session, selector, timeout).await?.keys)
}

/// [`liveliness_keys`], saying whether the GET completed before `timeout`,
/// with its error replies. A GET that runs to its timeout, or gets any
/// error reply, is read as possibly incomplete (§8.1).
pub async fn liveliness_read(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
) -> Result<PresenceRead> {
    let rx = session
        .liveliness()
        .get(selector)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
        .map_err(zenoh)?;
    let mut keys = Vec::new();
    let mut errors = Vec::new();
    while let Ok(reply) = rx.recv_async().await {
        match reply.result() {
            Ok(sample) => keys.push(sample.key_expr().as_str().to_owned()),
            Err(e) => errors.push(format!(
                "{}: {}",
                e.encoding(),
                String::from_utf8_lossy(&e.payload().to_bytes())
            )),
        }
    }
    keys.sort();
    keys.dedup();
    // The flume sender drops when the query finalizes, at the routers' final
    // reply or at the timeout; zenoh sends an error reply at the timeout.
    let complete = errors.is_empty();
    Ok(PresenceRead {
        keys,
        complete,
        errors,
    })
}

/// The zk2 tokens matching `selector`, parsed (§1.1). Keys that are not
/// zk2 keys are left out: the bus is shared (§1.7).
pub async fn tokens(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
) -> Result<Vec<ZkKey>> {
    Ok(liveliness_keys(session, selector, timeout)
        .await?
        .iter()
        .filter_map(|k| parse(k).ok())
        .collect())
}

/// What a descriptor GET found.
#[derive(Debug, Clone)]
pub enum Found {
    /// A descriptor that passes the syntax checks (the contracts are not
    /// at hand, so exposure is not checked: §3.3).
    Descriptor(Box<Descriptor>, Vec<u8>),
    /// A reply that fails the descriptor check, with its findings.
    Invalid(String),
    /// No reply. Silence is not a verdict: the instance may be gone, or the
    /// reply may not have crossed.
    Nothing,
}

/// GETs the descriptor of one instance (§3.3).
pub async fn descriptor(
    session: &zenoh::Session,
    addr: &Addr,
    instance: &InstanceId,
    timeout: Duration,
) -> Result<Found> {
    let key = instance_key(addr, instance)?;
    let rx = session
        .get(key.into_keyexpr())
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
        .map_err(zenoh)?;
    while let Ok(reply) = rx.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        let bytes = sample.payload().to_bytes().into_owned();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Ok(Found::Invalid("not UTF-8".to_owned()));
        };
        let (d, report) = descriptor::check(text, &[]);
        return Ok(match d {
            Some(d) if !report.has_errors() => Found::Descriptor(Box::new(d), bytes),
            _ => Found::Invalid(
                report
                    .errors()
                    .map(|e| format!("{} {}", e.code, e.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        });
    }
    Ok(Found::Nothing)
}

impl Found {
    /// The descriptor, or an error naming why there is none.
    pub fn into_descriptor(self) -> Result<Descriptor> {
        match self {
            Self::Descriptor(d, _) => Ok(*d),
            Self::Invalid(why) => Err(Error::Descriptor(why)),
            Self::Nothing => Err(Error::Zenoh("no descriptor reply".to_owned())),
        }
    }
}

/// An edge of the data-flow graph (R3): a consumer's role, bound to a
/// provider.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    /// The consumer's `<system>/<service>`.
    pub consumer: String,
    pub role: String,
    /// The provider's `<system>/<service>`.
    pub provider: String,
}

/// The graph, read from descriptors and interface tokens only, never
/// inferred from traffic (R3). A provider of an interface is a service with
/// its interface token, or whose descriptor lists it (the tokenless set,
/// §8.1). Each role's bindings, exact or wildcard, select among them.
#[must_use]
pub fn edges(descriptors: &[Descriptor], alive: &[ZkKey]) -> Vec<Edge> {
    let mut providers: Vec<(String, Addr)> = alive
        .iter()
        .filter_map(|k| match k {
            ZkKey::Alive { addr, iface, .. } => Some((iface.to_string(), addr.clone())),
            _ => None,
        })
        .collect();
    for d in descriptors {
        if let Ok(addr) = d.service.parse::<Addr>() {
            providers.extend(d.interfaces.iter().map(|e| (e.iface.clone(), addr.clone())));
        }
    }
    let mut out = Vec::new();
    for d in descriptors {
        for r in &d.requires {
            for b in &r.bindings {
                let Ok(pattern) = crate::consumer::Provider::parse(b) else {
                    continue;
                };
                for (iface, addr) in &providers {
                    if *iface == r.interface && pattern.matches(addr) {
                        out.push(Edge {
                            consumer: d.service.clone(),
                            role: r.role.clone(),
                            provider: addr.to_string(),
                        });
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
