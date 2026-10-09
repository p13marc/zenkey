//! zk2's `why` (#702), the reads: what the ladder of
//! [`crate::judge::why`] decides from, gathered through a session **in**
//! the deployment's namespace, as its own consumers read it.
//!
//! **Only what the ladder reaches is read.** Each read's outcome goes into
//! a [`WhyObservation`], and the judge is consulted on what is in hand
//! before the next one: a key outside the namespace asks the bus nothing, an
//! address with no visible token is not GET, and only an owner's silence
//! sends the reader on to an archive (S6). A read that cannot be put on the
//! bus is recorded with why, never dropped.
//!
//! **The answer** is read the way the key's kind is read (§1.3): a state
//! key by the owner's S4 GET (target `All`, consolidation `Latest`); a
//! stream key by a subscription over the window; an event key by a GET a
//! union storage may answer (§2.6). An operation is not called: `why` acts
//! on nothing (tooling guide §4), so that answer is not asked.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::SampleKind;

use crate::bus::contracts::BundleStore;
use crate::bus::presence::Scope;
use crate::model::catalog::{ContractState, DescriptorRead, Observed};
use crate::report::{RungId, Stamp, WhyReport};
use crate::{Error, Result};

/// The selector the `archive.v1` providers are read through: their
/// interface tokens (§8.1), base-relative.
pub const ARCHIVES: &str = "zk2/*/*/@zk/alive/archive.v1/**";

/// What a `why` run waits for.
#[derive(Debug, Clone, Copy)]
pub struct WhySpec {
    /// Each read's timeout: the liveliness GET, a descriptor GET, a
    /// retrieval attempt, the state or event GET, an archive's.
    pub timeout: Duration,
    /// How long a stream key is listened to for a sample.
    pub window: Duration,
}

/// What `why` explains: one wire key, or one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhyTarget {
    /// A wire key, the namespace included: the whole ladder.
    Key(String),
    /// A service address, `<system>/<service>`.
    Service(Addr),
}

impl WhyTarget {
    /// `<system>/<service>` — two plain chunks — is a service; anything
    /// else is a wire key. A key expression with a wildcard is refused:
    /// `why` explains one key, and a selector names many.
    pub fn parse(s: &str) -> Result<WhyTarget> {
        if s.contains('*') || s.contains('$') {
            return Err(Error::unaskable(
                s,
                "why explains one key, and a key expression with a wildcard names many: give a \
                 concrete key, or the service's address",
            ));
        }
        if let Ok(addr) = s.parse::<Addr>() {
            return Ok(WhyTarget::Service(addr));
        }
        if zenoh::key_expr::keyexpr::new(s).is_err() {
            return Err(Error::unaskable(s, "not a key expression"));
        }
        Ok(WhyTarget::Key(s.to_owned()))
    }

    /// As given.
    pub fn text(&self) -> String {
        match self {
            WhyTarget::Key(k) => k.clone(),
            WhyTarget::Service(a) => a.to_string(),
        }
    }
}

/// One reply or sample on the key: its value's bytes, or a deletion.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyReply {
    /// The key it came on, base-relative.
    pub key: String,
    /// `None` for a deletion (`reply_del`, or a delete sample).
    pub bytes: Option<Vec<u8>>,
    pub encoding: Option<String>,
    pub stamp: Option<Stamp>,
}

/// What the key answered, by how its kind is read.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAnswer {
    /// The owner's state GET (S4): its replies on the key, and whether the
    /// GET ended at the final reply.
    State {
        replies: Vec<KeyReply>,
        complete: bool,
    },
    /// The samples a subscription delivered on the key in the window: how
    /// many, and the first.
    Stream {
        samples: u64,
        first: Option<KeyReply>,
    },
    /// An event GET (§2.6): what a union storage answered for the
    /// occurrence, and whether the GET ended at the final reply.
    Event {
        replies: Vec<KeyReply>,
        complete: bool,
    },
}

/// What the archives held, read after the owner's silence (S6).
#[derive(Debug, Clone, Default)]
pub struct ArchiveRead {
    /// The `archive.v1` providers presence showed, by interface token.
    pub providers: Vec<Addr>,
    /// Whether that presence read ended at the routers' final reply.
    pub complete: bool,
    /// The first archive that held the key, and what it held.
    pub found: Option<(Addr, zenkey::archive::LastKnown)>,
    /// Archives whose read could not be made, with why.
    pub failed: Vec<(Addr, String)>,
}

/// Everything one `why` run read, as values: what
/// [`crate::judge::why::judge`] decides from. `None` is "not read": the
/// ladder stopped before it.
#[derive(Debug, Clone)]
pub struct WhyObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub target: WhyTarget,
    /// How long a stream was listened to, and each read waited.
    pub spec: WhySpec,
    /// The address's presence: tokens and every instance's descriptor.
    pub presence: Option<std::result::Result<Observed, String>>,
    /// What retrieving each revision the descriptors name found (§8.4).
    pub contracts: BTreeMap<(IfaceId, Fingerprint), std::result::Result<ContractState, String>>,
    /// The key's answer.
    pub answer: Option<std::result::Result<KeyAnswer, String>>,
    /// The archives, after the owner's silence (S6).
    pub archives: Option<std::result::Result<ArchiveRead, String>>,
}

impl WhyObservation {
    /// An observation of `target` with nothing read yet.
    pub fn new(namespace: &str, target: WhyTarget, spec: WhySpec) -> WhyObservation {
        WhyObservation {
            namespace: namespace.to_owned(),
            target,
            spec,
            presence: None,
            contracts: BTreeMap::new(),
            answer: None,
            archives: None,
        }
    }
}

/// One `why` run: [`observe`], then [`crate::judge::why::judge`].
///
/// `session` is in `namespace` (decided 2026-10-08); `store` is seeded with
/// the revisions known offline.
pub async fn run_why(
    session: &Session,
    store: &BundleStore,
    namespace: &str,
    target: WhyTarget,
    spec: WhySpec,
) -> WhyReport {
    let obs = observe(session, store, namespace, target, spec).await;
    crate::judge::why::judge(&obs)
}

/// Reads what the ladder reaches, judging what is in hand before each
/// read (see the module doc).
pub async fn observe(
    session: &Session,
    store: &BundleStore,
    namespace: &str,
    target: WhyTarget,
    spec: WhySpec,
) -> WhyObservation {
    let mut obs = WhyObservation::new(namespace, target, spec);
    let reaches = |obs: &WhyObservation, rung: RungId| {
        crate::judge::why::judge(obs)
            .stopped_at
            .is_none_or(|s| s >= rung)
    };
    // The address the ladder reads presence for, when the key names one.
    let addr = match crate::judge::why::subject_of(&obs) {
        Some(s) => s.addr,
        None => return obs,
    };
    let t = spec.timeout;
    obs.presence = Some(
        crate::bus::presence::observe(session, &Scope::service(&addr), t)
            .await
            .map_err(|e| crate::one_line(&e)),
    );
    if !reaches(&obs, RungId::Contract) {
        return obs;
    }
    // Every revision a served descriptor names: the interface the key is
    // of, or every one for a service.
    let iface = crate::judge::why::subject_of(&obs).and_then(|s| s.iface);
    let wanted: Vec<(IfaceId, Fingerprint)> = match &obs.presence {
        Some(Ok(o)) => o
            .descriptors
            .iter()
            .flatten()
            .filter_map(|(_, r)| r.descriptor())
            .flat_map(|d| d.interfaces.iter())
            .filter(|e| iface.as_ref().is_none_or(|i| e.iface == i.to_string()))
            .filter_map(|e| {
                Some((
                    IfaceId::from_str(&e.iface).ok()?,
                    Fingerprint::parse(&e.contract).ok()?,
                ))
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    let answers = store.fetch_all(session, &wanted).await;
    obs.contracts = wanted
        .into_iter()
        .zip(
            answers
                .into_iter()
                .map(|a| a.map_err(|e| crate::one_line(&e))),
        )
        .collect();
    if !reaches(&obs, RungId::Answer) {
        return obs;
    }
    let Some(subject) = crate::judge::why::subject_of(&obs) else {
        return obs;
    };
    let (Some(key), Some(kind)) = (subject.key, subject.kind) else {
        return obs;
    };
    obs.answer = match kind {
        KindToken::State | KindToken::ExplicitState => Some(state_answer(session, &key, t).await),
        KindToken::Stream | KindToken::ExplicitStream => {
            Some(stream_answer(session, &key, spec.window).await)
        }
        KindToken::Events => Some(event_answer(session, &key, t).await),
        KindToken::Op => None,
    };
    let silent = matches!(
        &obs.answer,
        Some(Ok(KeyAnswer::State { replies, .. })) if replies.is_empty()
    );
    if silent {
        obs.archives = Some(archives(session, &key, t).await);
    }
    obs
}

/// One reply's sample as a [`KeyReply`].
fn reply_of(sample: &zenoh::sample::Sample) -> KeyReply {
    KeyReply {
        key: sample.key_expr().as_str().to_owned(),
        bytes: (sample.kind() == SampleKind::Put).then(|| sample.payload().to_bytes().to_vec()),
        encoding: (sample.kind() == SampleKind::Put).then(|| sample.encoding().to_string()),
        stamp: sample.timestamp().map(crate::bus::consume::stamp),
    }
}

/// S4's GET on the owner's key: target `All`, consolidation `Latest`, both
/// set explicitly.
async fn state_answer(
    session: &Session,
    key: &str,
    timeout: Duration,
) -> std::result::Result<KeyAnswer, String> {
    let replies = session
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {key}: {e}"))?;
    let mut out = Vec::new();
    let mut complete = true;
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) if sample.key_expr().as_str() == key => out.push(reply_of(sample)),
            // R6: a reply on a key this GET did not name is discarded.
            Ok(_) => {}
            Err(_) => complete = false,
        }
    }
    Ok(KeyAnswer::State {
        replies: out,
        complete,
    })
}

/// A subscription on the stream key over `window`: how many samples came
/// on the key itself (R6 discards a put on a wildcard key), and the first.
async fn stream_answer(
    session: &Session,
    key: &str,
    window: Duration,
) -> std::result::Result<KeyAnswer, String> {
    let sub = session
        .declare_subscriber(key)
        .await
        .map_err(|e| format!("subscribe {key}: {e}"))?;
    let mut samples = 0u64;
    let mut first = None;
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, sub.recv_async()).await {
            Ok(Ok(sample)) if sample.key_expr().as_str() == key => {
                samples += 1;
                if first.is_none() {
                    first = Some(reply_of(&sample));
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }
    Ok(KeyAnswer::Stream { samples, first })
}

/// A GET on the occurrence's key (§2.6): target `All`, consolidation
/// `None`, what a union storage answers.
async fn event_answer(
    session: &Session,
    key: &str,
    timeout: Duration,
) -> std::result::Result<KeyAnswer, String> {
    let replies = session
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {key}: {e}"))?;
    let mut out = Vec::new();
    let mut complete = true;
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) if sample.key_expr().as_str() == key => out.push(reply_of(sample)),
            Ok(_) => {}
            Err(_) => complete = false,
        }
    }
    Ok(KeyAnswer::Event {
        replies: out,
        complete,
    })
}

/// The `archive.v1` providers presence shows, each asked for the key's
/// last-known value in turn (S5, S6), until one holds it.
async fn archives(
    session: &Session,
    key: &str,
    timeout: Duration,
) -> std::result::Result<ArchiveRead, String> {
    let read = crate::bus::presence::liveliness_read(session, ARCHIVES, timeout)
        .await
        .map_err(|e| crate::one_line(&e))?;
    let mut providers: Vec<Addr> = read
        .keys
        .iter()
        .filter_map(|k| match zenkey_model::grammar::parse(k) {
            Ok(ZkKey::Alive { addr, .. }) => Some(addr),
            _ => None,
        })
        .collect();
    providers.sort();
    providers.dedup();
    let mut out = ArchiveRead {
        providers: providers.clone(),
        complete: read.complete,
        ..ArchiveRead::default()
    };
    for archive in providers {
        match zenkey::archive::last_known(session, &archive, key, timeout).await {
            Ok(Some(lk)) => {
                out.found = Some((archive, lk));
                break;
            }
            Ok(None) => {}
            Err(e) => out.failed.push((archive, e.to_string())),
        }
    }
    Ok(out)
}

/// A descriptor read as a sentence.
pub(crate) fn descriptor_words(read: &DescriptorRead) -> String {
    match read {
        DescriptorRead::Served(_) => "served".into(),
        DescriptorRead::Invalid(why) => format!("invalid: {why}"),
        DescriptorRead::Silent => "no reply within the timeout".into(),
        DescriptorRead::Failed(why) => format!("the GET failed: {why}"),
    }
}
