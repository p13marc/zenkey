//! Reading presence (spec §8.1): liveliness GETs, parsed tokens, and
//! descriptors.
//!
//! Every liveliness GET here runs on an **unbounded** handler. zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2), and the spec forbids it. The flume channel's
//! sender is dropped when the query finalizes, which is how a GET here knows
//! it is complete.

use std::time::Duration;

use zenkey_model::descriptor::{self, Descriptor};
use zenkey_model::grammar::{Addr, InstanceId, ZkKey, instance_key, parse};
use zenoh::query::{ConsolidationMode, Reply};

use crate::error::{Error, Result, zenoh};

/// The keys of every liveliness token matching `selector`, sorted.
pub async fn liveliness_keys(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
) -> Result<Vec<String>> {
    let rx = session
        .liveliness()
        .get(selector)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
        .map_err(zenoh)?;
    let mut keys = Vec::new();
    while let Ok(reply) = rx.recv_async().await {
        if let Ok(sample) = reply.result() {
            keys.push(sample.key_expr().as_str().to_owned());
        }
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
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
