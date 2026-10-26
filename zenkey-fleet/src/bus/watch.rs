//! A tool's **watch session** (#830; #875, core §8.1, 0.42): the session a
//! live reader holds its liveliness subscribers on, in **no** namespace,
//! watching a deployment's keys as the wire spells them.
//!
//! **Why no namespace.** A watch session's subscribers have no history:
//! the reader seeds by a read on its other session, and the round trip
//! that orders the subscribers before that read follows their history, so
//! with history it waits for the population (core §8.1, 0.41: 23 s beside
//! 10,000 tokens in a debug build, where zenctl's `--timeout` is 5 s).
//! Without history a token declared before the subscriber and withdrawn
//! later is still heard: the router tells its withdrawal by its key. In a
//! namespace zenoh 1.10.1 does not strip the namespace from that key
//! (`routing/namespace.rs`), and the withdrawal is never heard, which is
//! why #872 gave a namespaced watch session's subscribers history. A
//! session in no namespace hears it on the wire key, so here the
//! subscribers have no history and the round trip is one round trip at any
//! population, in a namespace too (`docs/zk2/upstream/namespace-drops-token-withdrawal-by-key.md`),
//! until the session's transport moves: the transport that opens again
//! tells the subscribers every token they match, and the round trip after
//! it waits for them (core §8.1, 0.44, F-239; `zenkey::presence::Barrier`).
//!
//! **One place knows the namespace.** A [`WatchSession`] carries the
//! session and the deployment's namespace together, and is the only way the
//! fleet and the tools declare a liveliness subscriber
//! (`scripts/check-liveliness-seam.sh`):
//! - every selector it declares is the base-relative selector with the
//!   namespace in front ([`WatchSession::wire`]);
//! - every key it hears reaches the reader stripped of it, base-relative,
//!   as a [`TokenHeard`], never as a zenoh `Sample`, so no reader can fold a
//!   wire key as if it were a foreign one;
//! - a key outside the namespace is refused and counted
//!   ([`WatchSession::outside`]), never delivered;
//! - its barrier asks `<namespace>/` + `barrier_key(scope)`
//!   ([`WatchSession::barrier`]), inside every reader's presence grant,
//!   which `acl gen` writes on the wire keys (§11.1).
//!
//! **It makes no read.** Its subscribers mark the session a watch session
//! (`zenkey::presence::declare_subscriber`), so a liveliness read on it is
//! refused; and it hands out no `zenoh::Session`, so no resolved read — a
//! presence read, a descriptor GET, S4's GET — can go through a session in
//! no namespace that would spell the deployment's keys without it. A reader
//! makes every read on the deployment's session.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use zenoh::Session;
use zenoh::sample::{Sample, SampleKind};

use crate::{Error, Result};

/// One token move a [`WatchSession`]'s subscriber heard: the token's key,
/// **relative to the deployment's namespace** (base-relative, `zk2/…`, for
/// a zk2 token), and whether it was put or withdrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHeard {
    pub key: String,
    /// `true` for a put, `false` for a withdrawal.
    pub held: bool,
}

/// A watch session: a session in no namespace and the deployment's
/// namespace it watches on the wire (see the module doc). Cloning shares
/// the session and the count of keys refused.
#[derive(Clone)]
pub struct WatchSession {
    session: Session,
    namespace: String,
    outside: Arc<AtomicU64>,
}

impl std::fmt::Debug for WatchSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchSession")
            .field("zid", &self.session.zid())
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl WatchSession {
    /// `session` as a watch session for the deployment in `namespace`,
    /// empty for the bus root (whose keys the wire spells as they are).
    /// A tool opens one with [`crate::open_watch`].
    ///
    /// Refused, [`Error::Unaskable`]: a session that is itself in a
    /// namespace, whose subscribers without history would never hear an
    /// earlier token withdrawn (zenoh 1.10.1, #872); and a namespace that
    /// is not a concrete key-expression prefix
    /// ([`crate::bus::session::check_namespace`]).
    pub fn new(session: &Session, namespace: &str) -> Result<WatchSession> {
        if zenkey::presence::subscriber_needs_history(session) {
            return Err(Error::unaskable(
                format!("a watch session for namespace {namespace:?}"),
                "its session is in a namespace of its own: a watch session runs in no namespace \
                 and watches the wire keys, since a namespaced subscriber without history never \
                 hears an earlier token withdrawn (#875, core §8.1)",
            ));
        }
        if !namespace.is_empty() {
            crate::bus::session::check_namespace(namespace)?;
        }
        Ok(WatchSession {
            session: session.clone(),
            namespace: namespace.to_owned(),
            outside: Arc::default(),
        })
    }

    /// The deployment's namespace; empty for the bus root.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The session's zid: a reader refuses to read on a session with the
    /// same one (#830).
    pub fn zid(&self) -> zenoh::session::ZenohId {
        self.session.zid()
    }

    /// `relative` as the wire spells it: the namespace in front.
    pub fn wire(&self, relative: &str) -> String {
        crate::model::namespace::join(&self.namespace, relative)
    }

    /// `wire` relative to the namespace, or `None` when it is outside it.
    pub fn relative<'k>(&self, wire: &'k str) -> Option<&'k str> {
        crate::model::namespace::strip(&self.namespace, wire)
    }

    /// How many keys its subscribers heard outside the namespace, each
    /// refused and never delivered. A selector carries the namespace, so
    /// zenoh delivers none: a count above zero is a fault worth showing.
    pub fn outside(&self) -> u64 {
        self.outside.load(Ordering::Relaxed)
    }

    /// What a subscriber hands its reader for a sample at `wire`: the key
    /// stripped of the namespace, or nothing, counted, when it is outside.
    #[cfg(test)]
    fn hear(&self, wire: &str, held: bool) -> Option<TokenHeard> {
        hear(&self.namespace, &self.outside, wire, held)
    }

    /// Declares a liveliness subscriber to `selector`, base-relative, on
    /// the wire ([`WatchSession::wire`]), **without history** (see the
    /// module doc), calling `on` with each token move it hears, its key
    /// relative to the namespace ([`TokenHeard`]). A callback, as a subscriber
    /// beside a liveliness GET must be (core §8.1, zenoh#2678), on zenoh's
    /// thread: `on` must not block. Bounded by
    /// [`crate::DECLARE_TIMEOUT`]; `what` names the subscription in an
    /// error.
    ///
    /// Declared through `zenkey::presence::declare_subscriber`, which marks
    /// the session a watch session: no liveliness read is made on it from
    /// then on (#830).
    pub async fn subscribe<F>(
        &self,
        what: &'static str,
        selector: &str,
        on: F,
    ) -> Result<zenoh::pubsub::Subscriber<()>>
    where
        F: Fn(TokenHeard) + Send + Sync + 'static,
    {
        let wire = self.wire(selector);
        // The namespace and the count, never the session: a session held by
        // its own subscriber's callback would outlive every handle to it.
        let (namespace, outside) = (self.namespace.clone(), Arc::clone(&self.outside));
        crate::bus::teardown::declared(
            what,
            &wire,
            zenkey::presence::declare_subscriber(&self.session, wire.clone())
                .history(false)
                .callback(move |s: Sample| {
                    let held = s.kind() == SampleKind::Put;
                    if let Some(h) = hear(&namespace, &outside, s.key_expr().as_str(), held) {
                        on(h);
                    }
                }),
        )
        .await
    }

    /// The barrier of its subscribers to `scope`, base-relative (#851,
    /// core §8.1, 0.41; #875): its round trip asks
    /// `zenkey::presence::barrier_key(scope)` on the wire, a key no token
    /// holds, so it carries nothing and is one round trip at any
    /// population, until the session's transport moves (core §8.1, 0.44,
    /// F-239). A reader's [`zenkey::presence::Barrier::establish`] waits
    /// `timeout` for a round trip, which runs on past it (#882).
    pub async fn barrier(
        &self,
        scope: &str,
        timeout: Duration,
    ) -> Result<zenkey::presence::Barrier> {
        let key = self.wire(&zenkey::presence::barrier_key(scope));
        zenkey::presence::Barrier::at(&self.session, &key, timeout)
            .await
            .map_err(|e| Error::bus("liveliness barrier", key, e))
    }

    /// Why the session reaches no other zenoh node, when it does not (core
    /// §8.1, 0.37): its subscribers then hear nothing, and a reader that
    /// follows presence through it follows nothing (0.42, F-228).
    pub async fn cut_off(&self) -> Option<String> {
        crate::bus::presence::cut_off(&self.session).await
    }

    /// [`WatchSession::new`], for this crate's unit tests, which may make
    /// one by hand (`scripts/check-liveliness-seam.sh` keeps the call to
    /// this file and the opener elsewhere).
    #[cfg(test)]
    pub(crate) fn by_hand(session: &Session, namespace: &str) -> WatchSession {
        WatchSession::new(session, namespace).expect("a watch session")
    }

    /// The session, for the fleet's own wiring (a monitor's data watches):
    /// never handed out, so that no read goes through it.
    pub(crate) fn session(&self) -> &Session {
        &self.session
    }
}

/// [`WatchSession::hear`], from what a subscriber's callback holds.
fn hear(namespace: &str, outside: &AtomicU64, wire: &str, held: bool) -> Option<TokenHeard> {
    match crate::model::namespace::strip(namespace, wire) {
        Some(key) => Some(TokenHeard {
            key: key.to_owned(),
            held,
        }),
        None => {
            outside.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip (#875): a key heard under the namespace reaches the reader
    /// base-relative; one outside it is refused and counted, never handed
    /// on; the bus root strips nothing; and a selector and a barrier key
    /// carry the namespace on the wire.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_key_heard_reads_base_relative_and_one_outside_is_refused() {
        let session = crate::bus::session::standalone().await;
        let watch = WatchSession::new(&session, "site/a").expect("a watch session");
        assert_eq!(
            watch.wire("zk2/*/*/@zk/instance/*"),
            "site/a/zk2/*/*/@zk/instance/*"
        );
        assert_eq!(
            watch.hear("site/a/zk2/h1/tc/@zk/instance/000000000000000a", false),
            Some(TokenHeard {
                key: "zk2/h1/tc/@zk/instance/000000000000000a".to_owned(),
                held: false
            })
        );
        assert_eq!(watch.outside(), 0);
        for foreign in [
            "zk2/h1/tc/@zk/instance/000000000000000a",
            "site/b/zk2/h1/tc/@zk/instance/000000000000000a",
            "site/ab/zk2/h1/tc/@zk/instance/000000000000000a",
            "site/a",
        ] {
            assert_eq!(watch.hear(foreign, true), None, "{foreign}");
        }
        assert_eq!(watch.outside(), 4, "each one counted");
        assert_eq!(watch.clone().outside(), 4, "a clone shares the count");

        let barrier = watch
            .barrier("zk2/*/*/@zk/instance/*", Duration::from_secs(1))
            .await
            .expect("a barrier");
        assert_eq!(barrier.key(), "site/a/zk2/*/*/@zk/settle");

        let root = WatchSession::new(&session, "").expect("the bus root");
        assert_eq!(root.wire("zk2/**"), "zk2/**");
        assert_eq!(
            root.hear("site/a/zk2/x", true).map(|h| h.key),
            Some("site/a/zk2/x".to_owned()),
            "the bus root's keys are the wire's"
        );
        session.close().await.expect("close");
    }

    /// Refused: a session in a namespace of its own, and a namespace that
    /// is not a concrete prefix.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_namespaced_session_or_a_wildcard_namespace_is_refused() {
        let mut c = zenoh::Config::default();
        for (k, v) in [
            ("mode", r#""peer""#),
            ("listen/endpoints", "[]"),
            ("scouting/multicast/enabled", "false"),
            ("scouting/gossip/enabled", "false"),
            ("namespace", r#""ns""#),
        ] {
            c.insert_json5(k, v).expect("a fixed key");
        }
        let namespaced = zenoh::open(c).await.expect("a peer in ns");
        let refused = WatchSession::new(&namespaced, "ns").expect_err("refused");
        assert!(refused.is_unaskable(), "{refused}");
        assert!(refused.to_string().contains("no namespace"), "{refused}");
        namespaced.close().await.expect("close");

        let session = crate::bus::session::standalone().await;
        for bad in ["fleet/*", "a//b", "**"] {
            let refused = WatchSession::new(&session, bad).expect_err(bad);
            assert!(refused.is_unaskable(), "{bad}: {refused}");
        }
        session.close().await.expect("close");
    }
}
