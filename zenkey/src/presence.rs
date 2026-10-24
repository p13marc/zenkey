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
//!
//! **Two reads are not made** (core §8.1, 0.37), and each is refused with
//! a typed error before anything is asked:
//! - **beside a liveliness subscriber** (#830): on zenoh 1.10.1, and on
//!   zenoh's main branch at 766b455, a liveliness GET from a session that
//!   holds a liveliness subscriber, history or not, can complete without
//!   tokens that are live. The session's router drops its answer for every
//!   token already declared to the session for the subscriber, and the
//!   session's own answer races it (Appendix B;
//!   `docs/zk2/upstream/liveliness-get-misses-beside-liveliness-subscriber.md`).
//!   So every liveliness subscriber the runtime and the fleet declare goes
//!   through [`declare_subscriber`], which marks its session as a **watch
//!   session** for the rest of its life (the router keeps those
//!   declarations after the subscriber is undeclared), and [`liveliness_read`]
//!   refuses a marked session: [`Error::ReadBesideSubscriber`]. A reader
//!   holds its subscribers on a session of its own, and reads on another;
//! - **through a session cut off** from every other zenoh node: zenoh
//!   answers it at once, complete and empty, though nothing was asked of
//!   anyone, so every owner would read absent. [`Error::CutOff`].
//!
//! [`settle`] is the one liveliness GET a watch session makes, and its
//! answer is discarded: a barrier, so a reader knows its router has read the
//! subscribers it declared before it reads presence on its other session.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
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
///
/// Refused before anything is asked (core §8.1, 0.37; see the module doc):
/// on a session that holds a liveliness subscriber ([`watching`]),
/// [`Error::ReadBesideSubscriber`]; through a session cut off from every
/// other zenoh node ([`cut_off`]), [`Error::CutOff`].
pub async fn liveliness_read(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
) -> Result<PresenceRead> {
    if watching(session) {
        return Err(Error::ReadBesideSubscriber {
            selector: selector.to_owned(),
            zid: session.zid().to_string(),
        });
    }
    if cut_off(session).await {
        return Err(Error::CutOff {
            selector: selector.to_owned(),
        });
    }
    get(session, selector, timeout).await
}

/// The GET itself, on the unbounded handler: [`liveliness_read`]'s, and the
/// barrier's ([`settle`]).
async fn get(session: &zenoh::Session, selector: &str, timeout: Duration) -> Result<PresenceRead> {
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

// ─── watch sessions (#830) ──────────────────────────────────────────────────

/// The zids of the sessions of this process that have declared a liveliness
/// subscriber through [`declare_subscriber`]. A mark is never removed: the
/// router keeps what it declared to a session for a subscriber after the
/// subscriber is undeclared (zenoh 1.10.1's `unregister_interest` leaves
/// the face's `local_tokens` as they are), so the hazard lasts as long as
/// the session. A zid names one session, so one process's sessions never
/// share a mark.
fn watch_sessions() -> &'static Mutex<HashSet<zenoh::session::ZenohId>> {
    static MARKED: OnceLock<Mutex<HashSet<zenoh::session::ZenohId>>> = OnceLock::new();
    MARKED.get_or_init(Mutex::default)
}

/// Declares a liveliness subscriber on `session`, **marking it a watch
/// session** for the rest of its life (#830, core §8.1, 0.37): every
/// liveliness read on it is refused from now on
/// ([`Error::ReadBesideSubscriber`]). The builder is zenoh's own, so a
/// caller sets history, a callback or a handler as before.
///
/// The one way the runtime and `zenkey-fleet` declare a liveliness
/// subscriber: a declaration made around it would leave its session
/// unmarked, and a read beside it unguarded.
pub fn declare_subscriber<'a, 'b, K>(
    session: &'a zenoh::Session,
    key_expr: K,
) -> zenoh::liveliness::LivelinessSubscriberBuilder<'a, 'b, zenoh::handlers::DefaultHandler>
where
    K: TryInto<zenoh::key_expr::KeyExpr<'b>>,
    <K as TryInto<zenoh::key_expr::KeyExpr<'b>>>::Error: Into<zenoh::Error>,
{
    watch_sessions()
        .lock()
        .expect("the watch-session registry is never poisoned")
        .insert(session.zid());
    session.liveliness().declare_subscriber(key_expr)
}

/// Whether `session` is a watch session: it has declared a liveliness
/// subscriber through [`declare_subscriber`], and makes no liveliness read
/// (#830).
#[must_use]
pub fn watching(session: &zenoh::Session) -> bool {
    watch_sessions()
        .lock()
        .expect("the watch-session registry is never poisoned")
        .contains(&session.zid())
}

/// Whether `session` reaches no other zenoh node (core §8.1, 0.37): it holds
/// no transport — a client that has lost its router, a peer or a router
/// alone. zenoh answers a read through it at once, complete and empty,
/// though nothing was asked of anyone (measured, #800 AD), so such a read
/// is not complete, whatever its replies say.
pub async fn cut_off(session: &zenoh::Session) -> bool {
    session.info().transports().await.next().is_none()
}

/// A barrier for a watch session (#830): one liveliness GET of `selector`
/// whose answer is discarded, made after the session declared its
/// liveliness subscribers. zenoh orders one link's messages, so its routers'
/// final reply comes after they read every declaration the session made
/// before it: a token withdrawn from then on is told to the subscribers. A
/// reader that seeds presence on **another** session settles the watch
/// session first, since nothing orders two sessions' messages at their
/// router. `selector` is one the reader may read, so access control does
/// not refuse it.
///
/// Returns whether the routers' final reply came before `timeout`: `false`
/// when the GET ran to its timeout, or when the session is cut off, and the
/// barrier is then not established.
pub async fn settle(session: &zenoh::Session, selector: &str, timeout: Duration) -> Result<bool> {
    if cut_off(session).await {
        return Ok(false);
    }
    Ok(get(session, selector, timeout).await?.complete)
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
