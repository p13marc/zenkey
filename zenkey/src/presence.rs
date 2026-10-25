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
//! **A read cut off while it runs** (core §8.1, 0.38, F-185) was made, and
//! is possibly incomplete. Asked while a transport was held, and in flight
//! when the session's last one closed, it comes back at the cut with the
//! final reply alone, which the check before it could not see. So the
//! session is checked again when the read ends, and a read that finds
//! nothing there is [`PresenceRead::cut_off_during`]: `complete` false, its
//! tokens seen, its absences establishing nothing.
//!
//! **What "cut off" reads** ([`cut_off`]): a session with no link to
//! another zenoh node. zenoh 1.10.1 closes a transport in this order: its
//! links, then the routing face, which answers every read in flight, then
//! the transport's entry, which `Session::info().transports()` lists until
//! the end. A check of `transports()` made the moment a read is answered at
//! the cut still finds the closing transport; `Session::info().links()` no
//! longer does.
//!
//! [`settle`] is the one liveliness GET a watch session makes, and its
//! answer is discarded: a barrier, so a reader knows its router has read the
//! subscribers it declared before it reads presence on its other session.
//! It asks [`barrier_key`], a key no token holds, so it carries nothing
//! (#851, core §8.1, 0.41): its final reply follows the subscribers'
//! registration whatever it matches, and their history too, which is why a
//! watch session's subscribers are declared without history when their
//! reader seeds by a read, unless the session is in a namespace
//! ([`subscriber_needs_history`]). [`Barrier`] is the barrier a reader
//! keeps: made once, lost when the watch session's transport moves, made
//! again.
//!
//! **A GET that runs to the session's interests timeout** (#851, core §8.1,
//! 0.41) is possibly incomplete. zenoh 1.10.1's client and peer runtimes end
//! a liveliness GET their routers have not finished at
//! `routing/interests/timeout` ([`interests_timeout`], 10 s by default) with
//! the final reply alone and no error reply, as if the routers had finished
//! it: measured, a read with a 110 s timeout came back at 10 s holding 6,121
//! of 10,000 tokens. So a read whose time reaches it is not complete
//! ([`PresenceRead::at_interests_timeout`]), and a barrier that ends there is
//! not established.
//!
//! **Every liveliness GET of the runtime, the fleet and the tools goes
//! through this module** (`scripts/check-liveliness-seam.sh`), and each is
//! counted per session ([`liveliness_gets`]): what a scenario observes when
//! it asks which session a reader made its reads on
//! (`spec/scenarios/presence.md` §9, 0.38).

use std::collections::{HashMap, HashSet};
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
    /// reply, or with its session cut off ([`PresenceRead::cut_off_during`]):
    /// it may have missed tokens, and a tool reports the result as possibly
    /// incomplete (§8.1), never as absence (O5).
    pub complete: bool,
    /// The GET's error replies, in arrival order, each
    /// `<encoding>: <payload>`: `zenoh/string: Timeout` for one that ended
    /// at its timeout. Only what zenoh replied: a read cut off while it ran
    /// has none, and says so in [`PresenceRead::cut_off_during`].
    pub errors: Vec<String>,
    /// The session held no link to another zenoh node when the read ended
    /// (core §8.1, 0.38, F-185): its last transport closed while the read
    /// ran, and zenoh answered it at the cut, with the final reply alone.
    /// The read was made, so it is possibly incomplete (`complete` is
    /// false), never a read not made: the keys it returned were seen, and
    /// what it shows absent establishes nothing.
    pub cut_off_during: bool,
    /// The read ran until the session's interests timeout
    /// ([`interests_timeout`]; #851, core §8.1, 0.41), where zenoh 1.10.1
    /// ends a GET with the final reply alone and no error reply, whether its
    /// routers finished it or not: possibly incomplete (`complete` is false).
    pub at_interests_timeout: bool,
}

impl PresenceRead {
    /// Why the read is possibly incomplete, in a phrase a tool can put
    /// after "the presence read …"; `None` when it is complete.
    #[must_use]
    pub fn incomplete_reason(&self) -> Option<&'static str> {
        if self.complete {
            None
        } else if self.cut_off_during {
            Some(CUT_OFF_DURING)
        } else if self.errors.is_empty() && self.at_interests_timeout {
            Some(AT_INTERESTS_TIMEOUT)
        } else if self.errors.iter().any(|e| e.ends_with(": Timeout")) {
            Some("ended at its timeout")
        } else {
            Some("ended with an error reply")
        }
    }
}

/// [`PresenceRead::incomplete_reason`] for a read cut off while it ran.
pub const CUT_OFF_DURING: &str =
    "was cut off while it ran: zenoh answered it as the session's last link closed (core §8.1)";

/// [`PresenceRead::incomplete_reason`] for a read that ran to the session's
/// interests timeout (#851, core §8.1, 0.41).
pub const AT_INTERESTS_TIMEOUT: &str = "ran to the session's interests timeout \
     (routing/interests/timeout), where zenoh ends it with no error reply (core §8.1)";

/// What a reader says of a read made while its watch session's barrier
/// does not stand (#851, core §8.1, 0.41), after "the read …".
pub const BARRIER_NOT_ESTABLISHED: &str = "was made while the watch session's barrier was not \
     established: a token withdrawn after it may not be heard (core §8.1)";

/// zenoh's own `routing/interests/timeout` when a session's config states
/// none: 10 s (zenoh 1.10.1's `DEFAULT_CONFIG.json5`).
pub const DEFAULT_INTERESTS_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `session`'s runtime waits for its routers to finish a
/// liveliness GET before it ends the GET itself, with no error reply
/// (#851, core §8.1, 0.41): `routing/interests/timeout`, read from the
/// session's config, else [`DEFAULT_INTERESTS_TIMEOUT`].
#[must_use]
pub fn interests_timeout(session: &zenoh::Session) -> Duration {
    session
        .config()
        .get_typed::<Option<u64>>("routing/interests/timeout")
        .ok()
        .flatten()
        .map_or(DEFAULT_INTERESTS_TIMEOUT, Duration::from_millis)
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
/// with its error replies. A GET that runs to its timeout, gets any error
/// reply, or ends with its session cut off ([`PresenceRead::cut_off_during`],
/// core §8.1, 0.38) is read as possibly incomplete (§8.1).
///
/// Refused before anything is asked (core §8.1, 0.37; see the module doc):
/// on a session that has ever held a liveliness subscriber ([`watching`]),
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
    get(session, selector, timeout, Kind::Read).await
}

/// Which of the module's two GETs one is, for [`liveliness_gets`].
#[derive(Clone, Copy)]
enum Kind {
    Read,
    Settle,
}

/// The GET itself, on the unbounded handler: [`liveliness_read`]'s, and the
/// barrier's ([`settle`]). Counted once it is asked ([`liveliness_gets`]);
/// the session is checked again when it ends (core §8.1, 0.38).
async fn get(
    session: &zenoh::Session,
    selector: &str,
    timeout: Duration,
    kind: Kind,
) -> Result<PresenceRead> {
    {
        let mut counts = get_counts();
        let c = counts.entry(session.zid()).or_default();
        match kind {
            Kind::Read => c.reads += 1,
            Kind::Settle => c.settles += 1,
        }
    }
    let ends_at = interests_timeout(session);
    let started = std::time::Instant::now();
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
    // reply or at the timeout; zenoh sends an error reply at the timeout. A
    // session whose last link closed while the read ran had it finalized at
    // the cut, with the final reply alone (core §8.1, 0.38, F-185). One that
    // ran to the session's interests timeout was ended by the session's own
    // runtime, with no error reply, whatever its routers had sent (#851,
    // core §8.1, 0.41).
    let at_interests_timeout = started.elapsed() >= ends_at;
    let cut_off_during = cut_off(session).await;
    let complete = errors.is_empty() && !cut_off_during && !at_interests_timeout;
    Ok(PresenceRead {
        keys,
        complete,
        errors,
        cut_off_during,
        at_interests_timeout,
    })
}

/// The liveliness GETs this module made through one session (#849): what
/// `spec/scenarios/presence.md` §9 observes of a reader (0.38, F-189).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LivelinessGets {
    /// Reads asked through [`liveliness_read`] and the functions over it;
    /// one refused before anything was asked is not counted.
    pub reads: u64,
    /// Barriers asked through [`settle`].
    pub settles: u64,
}

fn get_counts() -> std::sync::MutexGuard<'static, HashMap<zenoh::session::ZenohId, LivelinessGets>>
{
    static COUNTS: OnceLock<Mutex<HashMap<zenoh::session::ZenohId, LivelinessGets>>> =
        OnceLock::new();
    COUNTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How many liveliness GETs this process has made through `session`, each
/// counted when it was asked. Every liveliness GET of the runtime, the
/// fleet and the tools goes through this module
/// (`scripts/check-liveliness-seam.sh`), so a watch session's count is the
/// evidence that it made its round trip and no read (core §8.1, 0.37;
/// `spec/scenarios/presence.md` §9). A count is kept for the life of the
/// process, as a watch session's mark is.
#[must_use]
pub fn liveliness_gets(session: &zenoh::Session) -> LivelinessGets {
    get_counts()
        .get(&session.zid())
        .copied()
        .unwrap_or_default()
}

// ─── watch sessions (#830) ──────────────────────────────────────────────────

/// The zids of the sessions of this process that have declared a liveliness
/// subscriber through [`declare_subscriber`]. A mark is never removed: the
/// router keeps what it declared to a session for a subscriber after the
/// subscriber is undeclared (zenoh 1.10.1's `unregister_interest` leaves
/// the face's `local_tokens` as they are), so the hazard lasts as long as
/// the session. A zid names one session, so one process's sessions never
/// share a mark.
///
/// A panic elsewhere while the lock is held leaves the set as it was (an
/// insert or a lookup), so a poisoned lock is read through, never fatal.
fn watch_sessions() -> std::sync::MutexGuard<'static, HashSet<zenoh::session::ZenohId>> {
    static MARKED: OnceLock<Mutex<HashSet<zenoh::session::ZenohId>>> = OnceLock::new();
    MARKED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Declares a liveliness subscriber on `session`, **marking it a watch
/// session** for the rest of its life (#830, core §8.1, 0.37): every
/// liveliness read on it is refused from now on
/// ([`Error::ReadBesideSubscriber`]). The builder is zenoh's own, so a
/// caller sets history, a callback or a handler as before.
///
/// The mark is made here, before the builder is awaited, so a declaration
/// that fails or is never awaited marks the session too: conservative,
/// since a read there is then refused and goes on a session of its own.
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
    watch_sessions().insert(session.zid());
    session.liveliness().declare_subscriber(key_expr)
}

/// Whether `session` is a watch session: it has declared a liveliness
/// subscriber through [`declare_subscriber`], and makes no liveliness read
/// (#830).
#[must_use]
pub fn watching(session: &zenoh::Session) -> bool {
    watch_sessions().contains(&session.zid())
}

/// Whether `session` reaches no other zenoh node (core §8.1, 0.37): it holds
/// no transport — a client that has lost its router, a peer or a router
/// alone. zenoh answers a read through it at once, complete and empty,
/// though nothing was asked of anyone (measured, #800 AD), so such a read
/// is not complete, whatever its replies say.
///
/// Read from the session's **links** (`Session::info().links()`), not its
/// transports: zenoh 1.10.1 drops a closing transport's links first, then
/// answers the reads in flight, and removes the transport from
/// `transports()` last (the module doc). A read answered at the cut finds
/// no link here; it could still find the transport (0.38, F-185).
pub async fn cut_off(session: &zenoh::Session) -> bool {
    session.info().links().await.next().is_none()
}

/// Whether a liveliness subscriber on `session` must have history to hear
/// a token withdrawn that was declared before it (#851, core §8.1, 0.41):
/// when the session is in a namespace. zenoh 1.10.1's router tells a
/// subscriber without history such a withdrawal by key, and a namespaced
/// session does not strip its namespace from that key
/// (`routing/namespace.rs`, `ENamespace`: an `UndeclareToken` passes
/// through as it came), so it matches none of the session's subscribers
/// and is never heard (measured, `tests/presence.rs` §10 step 7). A
/// session in no namespace hears it, a namespaced key included.
///
/// A watch session's subscribers have history exactly when this says so:
/// without, the round trip is one round trip at any population; with, it
/// waits for the history, and a reader that saw it run out makes it again
/// before its next seed, when the history is in.
#[must_use]
pub fn subscriber_needs_history(session: &zenoh::Session) -> bool {
    session
        .config()
        .get_typed::<Option<String>>("namespace")
        .ok()
        .flatten()
        .is_some_and(|ns| !ns.is_empty())
}

/// The key a barrier asks for a watch session whose subscribers read
/// `scope` (#851, core §8.1, 0.41): the scope's chunks before its first
/// `@zk` chunk, then `@zk/settle`; `zk2/@zk/settle` for a scope with no
/// `@zk` chunk (a foreign one).
///
/// - **No token holds it.** Every zk2 token's `@zk` chunk is followed by
///   its kind and at least one chunk more (§1.1: `instance/<id>`,
///   `alive/<iface>/<id>/<fp16>`, `member/<iface>/<member>/<epoch>`); here
///   one chunk follows it, whatever comes before, a namespace included.
/// - **The reader may ask it.** Every presence grant `acl gen` writes is a
///   subtree `zk2/<address>/@zk/**` (§11.1), so one that lets the reader
///   subscribe to `<prefix>/@zk/…` includes `<prefix>/@zk/settle`. A
///   foreign scope's grants are the deployment's own; a barrier they
///   refuse is still ordered, since zenoh 1.10.1 answers a refused
///   interest with its final reply at ingress, after every earlier message
///   of the session (measured, `tests/presence.rs`).
#[must_use]
pub fn barrier_key(scope: &str) -> String {
    let chunks: Vec<&str> = scope.split('/').collect();
    match chunks
        .iter()
        .position(|c| *c == zenkey_model::grammar::CONTROL)
    {
        Some(at) => {
            let mut key: Vec<&str> = chunks[..=at].to_vec();
            key.push(BARRIER_CHUNK);
            key.join("/")
        }
        None => FOREIGN_BARRIER.to_owned(),
    }
}

/// The chunk a barrier key ends with ([`barrier_key`]).
pub const BARRIER_CHUNK: &str = "settle";

/// A foreign scope's barrier key ([`barrier_key`]).
pub const FOREIGN_BARRIER: &str = "zk2/@zk/settle";

/// A barrier for a watch session (#830; #851, core §8.1, 0.41): one
/// liveliness GET of [`barrier_key`]`(scope)` whose answer is discarded,
/// made after the session declared its liveliness subscribers. zenoh orders
/// one link's messages, and its router handles a session's interests in
/// turn, so the final reply comes after the router registered every
/// subscriber the session declared before it, and sent their history: a
/// token withdrawn from then on is told to the subscribers. A reader that
/// seeds presence on **another** session settles the watch session first,
/// since nothing orders two sessions' messages at their router.
///
/// The GET matches no token, so it carries none: beside a subscriber
/// without history it is one round trip at any population (measured, 6 ms
/// beside 10,000 tokens in a debug build), and beside one with history it
/// waits for the history (23 s there).
///
/// On a session that holds no subscriber it orders the session's token
/// declarations the same way (`redundancy.v1`'s claim, F-204).
///
/// Returns whether the routers' final reply came: `false` when the GET ran
/// to its timeout, ran to the session's interests timeout
/// ([`interests_timeout`]: zenoh's runtime ended it, and nothing says the
/// routers did), when the session is cut off, or when it was cut off while
/// the GET ran (zenoh's reply at the cut follows no declaration). The
/// barrier is then not established.
pub async fn settle(session: &zenoh::Session, scope: &str, timeout: Duration) -> Result<bool> {
    settle_at(session, &barrier_key(scope), timeout).await
}

/// [`settle`], asking `key` itself: a watch session in no namespace that
/// watches a deployment's wire keys asks its barrier key on the wire,
/// `<namespace>/` + [`barrier_key`]`(scope)` (#875, core §8.1, 0.42).
/// The checks are [`settle`]'s: no round trip through a session cut off,
/// and none stands that was cut off while it ran (0.42, F-228).
pub async fn settle_at(session: &zenoh::Session, key: &str, timeout: Duration) -> Result<bool> {
    if cut_off(session).await {
        return Ok(false);
    }
    Ok(get(session, key, timeout, Kind::Settle).await?.complete)
}

/// A watch session's barrier, kept (#851, core §8.1, 0.41): [`settle`]'s
/// round trip made once and remembered, until the session's transport
/// moves. A transport that closes or opens re-declares the session's
/// subscribers at a router that has to read them again, and nothing orders
/// them before a read on another session, so the barrier is lost and the
/// next [`Barrier::establish`] makes it again. A reader asks for it before
/// each seed and reads a seed made without it as possibly incomplete
/// ([`BARRIER_NOT_ESTABLISHED`]), never as one that stands.
///
/// **A lost barrier is a reason to read again** ([`Barrier::moved_since`]). A
/// subscriber without history hears no withdrawal of a token declared
/// before it when its own link drops, since the session was never told of
/// the token; and a token withdrawn while the link was down is in no
/// history the router sends on reconnection. So a reader that follows
/// presence reads again when its watch session's transport moves, as it
/// reads again after a withdrawal.
///
/// Cloning shares it.
#[derive(Clone)]
pub struct Barrier {
    inner: std::sync::Arc<BarrierInner>,
}

struct BarrierInner {
    session: zenoh::Session,
    /// The key its round trip asks: [`barrier_key`] of the scope, on the
    /// wire when the session watches a deployment's wire keys (#875).
    key: String,
    timeout: Duration,
    /// Bumped by every transport event of the session.
    moved: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The `moved` count a barrier was established at; `u64::MAX` for none.
    at: std::sync::atomic::AtomicU64,
    /// One round trip at a time.
    making: tokio::sync::Mutex<()>,
    /// Woken at every transport event of the session.
    moves: std::sync::Arc<tokio::sync::Notify>,
    _events: zenoh::session::TransportEventsListener<()>,
}

impl std::fmt::Debug for Barrier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Barrier")
            .field("key", &self.inner.key)
            .field("established", &self.established())
            .finish_non_exhaustive()
    }
}

impl Barrier {
    /// A barrier for `session`'s subscribers to `scope`, each round trip
    /// bounded by `timeout`: it listens to the session's transport events,
    /// and makes no round trip yet ([`Barrier::establish`]). Its round trip
    /// asks [`barrier_key`]`(scope)`.
    pub async fn new(session: &zenoh::Session, scope: &str, timeout: Duration) -> Result<Barrier> {
        Barrier::at(session, &barrier_key(scope), timeout).await
    }

    /// [`Barrier::new`], its round trip asking `key` itself: a key no token
    /// holds, as [`barrier_key`] spells one. A watch session in no
    /// namespace that watches a deployment's wire keys asks
    /// `<namespace>/` + [`barrier_key`]`(scope)` (#875, core §8.1, 0.42).
    pub async fn at(session: &zenoh::Session, key: &str, timeout: Duration) -> Result<Barrier> {
        use std::sync::atomic::{AtomicU64, Ordering};
        let moved = std::sync::Arc::new(AtomicU64::new(0));
        let moves = std::sync::Arc::new(tokio::sync::Notify::new());
        let (m, n) = (std::sync::Arc::clone(&moved), std::sync::Arc::clone(&moves));
        let events = session
            .info()
            .transport_events_listener()
            .callback(move |_| {
                m.fetch_add(1, Ordering::SeqCst);
                n.notify_waiters();
            })
            .await
            .map_err(zenoh)?;
        Ok(Barrier {
            inner: std::sync::Arc::new(BarrierInner {
                session: session.clone(),
                key: key.to_owned(),
                timeout,
                moved,
                at: AtomicU64::new(u64::MAX),
                making: tokio::sync::Mutex::new(()),
                moves,
                _events: events,
            }),
        })
    }

    /// Whether the barrier stands: a round trip came back, and the
    /// session's transport has not moved since it was asked.
    #[must_use]
    pub fn established(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.inner.at.load(Ordering::SeqCst) == self.inner.moved.load(Ordering::SeqCst)
    }

    /// The barrier, made now unless it stands: one [`settle`], counted
    /// against the transport events seen before it was asked. Returns
    /// whether it stands.
    pub async fn establish(&self) -> bool {
        use std::sync::atomic::Ordering;
        if self.established() {
            return true;
        }
        let _one = self.inner.making.lock().await;
        if self.established() {
            return true;
        }
        let before = self.inner.moved.load(Ordering::SeqCst);
        let came = settle_at(&self.inner.session, &self.inner.key, self.inner.timeout)
            .await
            .unwrap_or(false);
        if came && self.inner.moved.load(Ordering::SeqCst) == before {
            self.inner.at.store(before, Ordering::SeqCst);
        }
        self.established()
    }

    /// How many transport events the session has had since this barrier
    /// was created: a reader keeps the count it last read, and waits on
    /// [`Barrier::moved_since`] for the next.
    #[must_use]
    pub fn moves(&self) -> u64 {
        self.inner.moved.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Resolves once the session has had a transport event, a close or an
    /// open, since [`Barrier::moves`] read `seen`: at once when it has. The
    /// barrier is lost then, if it stood. Cancel-safe.
    pub async fn moved_since(&self, seen: u64) {
        loop {
            let moved = self.inner.moves.notified();
            tokio::pin!(moved);
            moved.as_mut().enable();
            if self.moves() != seen {
                return;
            }
            moved.await;
        }
    }

    /// The key its round trip asks.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.inner.key
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The barrier key (#851): the scope's chunks to its first `@zk`, then
    /// `settle`; a foreign scope's is fixed. None is a token key, under any
    /// namespace.
    #[test]
    fn a_barrier_key_is_the_scopes_control_subtree_and_no_token() {
        for (scope, key) in [
            ("zk2/*/*/@zk/instance/*", "zk2/*/*/@zk/settle"),
            ("zk2/*/*/@zk/alive/**", "zk2/*/*/@zk/settle"),
            ("zk2/h1/tc/@zk/**", "zk2/h1/tc/@zk/settle"),
            (
                "zk2/h1/tc/@zk/instance/000000000000000a",
                "zk2/h1/tc/@zk/settle",
            ),
            ("site-a/zk2/*/*/@zk/member/**", "site-a/zk2/*/*/@zk/settle"),
            ("group/**", FOREIGN_BARRIER),
            ("robot/*/alive", FOREIGN_BARRIER),
        ] {
            assert_eq!(barrier_key(scope), key, "{scope}");
            let ke = zenoh::key_expr::KeyExpr::try_from(key).expect("a key expression");
            assert_eq!(ke.as_str(), key, "canonical");
            for token in [
                "zk2/h1/tc/@zk/instance/000000000000000a",
                "zk2/h1/tc/@zk/alive/nav.v2/000000000000000a/3fa9c2d41b7e9a01",
                "zk2/h1/tc/@zk/member/nav.v2/settle/0001",
                "site-a/zk2/h1/tc/@zk/instance/settle",
            ] {
                let t = zenoh::key_expr::KeyExpr::try_from(token).unwrap();
                assert!(!ke.intersects(&t), "{key} matches the token {token}");
            }
        }
    }
}
