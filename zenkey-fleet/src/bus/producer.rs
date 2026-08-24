//! Producer-side bring-up discipline (RFC 04 §5, RFC 05 §2.1/§3, RFC 08
//! §6.1) — the MUSTs every producer owes the bus, as an API that makes the
//! wrong shapes unrepresentable rather than a checklist that trusts review.
//!
//! The conformance review found these uniformly unenforced, with the
//! in-repo dev tools modeling the violations; this module is the
//! enforcement seam, and [`crate::bus::serve`]/[`crate::tape::generate`] now ride it
//! or its rules. It lives beside — not inside — `serve.rs`, deliberately:
//! `serve` is the *explorer's* mock (answer anything, wildcards welcome),
//! while this is the *producer's* posture (concrete keys, ordered
//! presence), and folding the strict API into the permissive one would
//! give every mock a way to claim it is a producer.
//!
//! Three rules, three shapes:
//!
//! - **Order** ([`BringUp`]): a producer declares its queryables
//!   (`introspect`, `describe`, its procedures) **first**, and its `alive`
//!   liveliness token **last** — "alive ⇒ callable" (RFC 04 §5): callers
//!   attribute RPC silence against the roster, so a token that precedes
//!   the queryables manufactures false negatives. RFC 08 §6.1's bounded
//!   grace exists to tolerate declarations spawned concurrently at
//!   startup; this API sequences them instead, so there is no race for a
//!   checker to be graceful about. The token is only mintable by
//!   consuming the bring-up: alive-before-queryable does not typecheck.
//! - **Vocabulary** ([`ReservedError`]): the RFC 05 §3 reserved error
//!   names as an enum, so a responder cannot misspell `error/unsupported`
//!   — and RFC 08 §6.1's conditional-procedure rule (declare always,
//!   answer `error/unsupported`/`error/gated` when gated) is writable
//!   without a string literal.
//! - **Reply key** ([`Responder::reply`]): a procedure replies on its
//!   **own concrete key**, never by echoing `query.key_expr()` (RFC 05
//!   §2.1: consolidation keeps one reply *per reply key*, so a fleet
//!   echoing a shared wildcard selector consolidates down to one
//!   survivor). The responder holds its declared key and replies on it;
//!   the query's key is never consulted.

use anyhow::{Result, anyhow};
use zenoh::Session;
use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::query::{Query, Queryable};

/// The RFC 05 §3 reserved error vocabulary — the names every conforming
/// caller understands, as a closed enum. Producer-specific names live under
/// `error/<producer>/…` and are registered like subjects; these six are the
/// convention's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReservedError {
    /// `error/invalid-args` — the request payload does not parse or fails
    /// validation.
    InvalidArgs,
    /// `error/unauthorized` — the caller may not invoke this procedure.
    Unauthorized,
    /// `error/not-found` — the named entity does not exist here.
    NotFound,
    /// `error/unsupported` — the capability is absent from this build
    /// (RFC 08 §6.1: a conditional procedure still declares, and answers
    /// with this — "producer present, capability not in this build →
    /// rebuild").
    Unsupported,
    /// `error/busy` — present and capable, but not now.
    Busy,
    /// `error/gated` — the capability is built in but disabled here by
    /// policy or configuration (RFC 05 §3: gated writes keep their gate at
    /// the server; RFC 08 §6.1: "capability built in, disabled here →
    /// reconfigure").
    Gated,
}

impl ReservedError {
    /// Every reserved name, for iteration (renderers, doctors).
    pub const ALL: [ReservedError; 6] = [
        ReservedError::InvalidArgs,
        ReservedError::Unauthorized,
        ReservedError::NotFound,
        ReservedError::Unsupported,
        ReservedError::Busy,
        ReservedError::Gated,
    ];

    /// The wire name, namespaced like a key (RFC 05 §3).
    pub fn name(self) -> &'static str {
        match self {
            ReservedError::InvalidArgs => "error/invalid-args",
            ReservedError::Unauthorized => "error/unauthorized",
            ReservedError::NotFound => "error/not-found",
            ReservedError::Unsupported => "error/unsupported",
            ReservedError::Busy => "error/busy",
            ReservedError::Gated => "error/gated",
        }
    }

    /// The RFC 05 §3 error envelope, `{ "error": <name>, "message": … }`,
    /// serialized as JSON (the RFC shows the envelope as JSON for
    /// readability; a deployment whose payload default differs encodes the
    /// same two fields itself).
    pub fn envelope(self, message: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "error": self.name(),
            "message": message,
        }))
        .expect("two string fields serialize")
    }
}

/// One declared `@rpc` queryable, bound to its own concrete key — the only
/// key it will ever reply on.
pub struct Responder {
    key: String,
    queryable: Queryable<FifoChannelHandler<Query>>,
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Responder")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Responder {
    /// The concrete key this responder was declared on — and replies on.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The next query, or `None` once the queryable is gone.
    pub async fn next(&self) -> Option<Query> {
        self.queryable.recv_async().await.ok()
    }

    /// Reply a value on this responder's **own concrete key** (RFC 05
    /// §2.1). `query.key_expr()` is deliberately never consulted: echoing
    /// the query's selector puts a fleet's replies on one shared wildcard
    /// key, and default consolidation keeps one survivor per reply key.
    pub async fn reply(
        &self,
        query: &Query,
        payload: Vec<u8>,
        encoding: Option<&str>,
    ) -> Result<()> {
        let reply = query.reply(self.key.clone(), payload);
        let reply = match encoding {
            Some(e) => reply.encoding(e),
            None => reply,
        };
        reply
            .await
            .map_err(|e| anyhow!("reply on {}: {e}", self.key))
    }

    /// Refuse on Zenoh's reply-error channel with a [`ReservedError`]
    /// envelope (RFC 05 §3: a value reply always means success; a failure
    /// always rides `reply_err` — never a success payload carrying
    /// `ok: false`).
    pub async fn reply_err(
        &self,
        query: &Query,
        error: ReservedError,
        message: &str,
    ) -> Result<()> {
        query
            .reply_err(error.envelope(message))
            .encoding("application/json")
            .await
            .map_err(|e| anyhow!("reply_err on {}: {e}", self.key))
    }

    /// Undeclare, acknowledged.
    pub async fn undeclare(self) -> Result<()> {
        self.queryable
            .undeclare()
            .await
            .map_err(|e| anyhow!("undeclare {}: {e}", self.key))
    }
}

/// The ordered bring-up (RFC 04 §5): queryables first, `alive` last — as
/// one API in which the wrong order is unrepresentable.
///
/// ```no_run
/// # async fn demo(session: zenoh::Session) -> anyhow::Result<()> {
/// let mut up = zenkey_fleet::bus::producer::BringUp::new(&session);
/// up.serve("v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect").await?;
/// up.serve("v1/h-3fa9c2d41b7e/@rpc/sysinfo/describe").await?;
/// // The token is mintable only by consuming the bring-up: every
/// // queryable above is declared (awaited, not spawned) before it.
/// let live = up.alive("v1/h-3fa9c2d41b7e/state/sysinfo/alive").await?;
/// # let _ = live; Ok(()) }
/// ```
///
/// Keys are written base-relative here because a *producer's* session is
/// namespaced (the deployment base is session config, RFC 03 §1.1); an
/// un-namespaced explorer passes full wire keys, which is equally fine —
/// the API is a string seam either way, concreteness enforced.
#[derive(Debug)]
pub struct BringUp<'a> {
    session: &'a Session,
    responders: Vec<Responder>,
}

impl<'a> BringUp<'a> {
    /// Start a bring-up: nothing is declared yet, and no `alive` token can
    /// exist before [`alive`](Self::alive) consumes this value.
    pub fn new(session: &'a Session) -> Self {
        BringUp {
            session,
            responders: Vec::new(),
        }
    }

    /// Declare one `@rpc` queryable on the producer's **own concrete key**,
    /// awaited before return (no spawn race, so RFC 08 §6.1's bounded
    /// grace has nothing to tolerate). Refused:
    ///
    /// - a wildcard key — a producer serves its own concrete keys, and a
    ///   concrete declared key is what makes [`Responder::reply`]'s
    ///   reply-key discipline meaningful (RFC 05 §2.1);
    /// - `complete` is not a parameter at all: `@rpc` queryables are
    ///   **never** declared complete (RFC 05 §2.1 — one complete queryable
    ///   short-circuits `BestMatching` callers to a single reply).
    pub async fn serve(&mut self, key: &str) -> Result<&Responder> {
        let parsed = zenoh::key_expr::KeyExpr::try_from(key.to_string())
            .map_err(|e| anyhow!("declare queryable {key}: {e}"))?;
        if parsed.is_wild() {
            return Err(anyhow!(
                "declare queryable {key}: a producer serves its own concrete \
                 key, never a wildcard (RFC 05 §2.1 — replies are attributed \
                 by their concrete reply key)"
            ));
        }
        let queryable = self
            .session
            .declare_queryable(parsed)
            .complete(false)
            .await
            .map_err(|e| anyhow!("declare queryable {key}: {e}"))?;
        self.responders.push(Responder {
            key: key.to_string(),
            queryable,
        });
        Ok(self.responders.last().expect("just pushed"))
    }

    /// Declare the `alive` liveliness token — consuming the bring-up, so
    /// the token structurally cannot precede the queryables ("alive ⇒
    /// callable", RFC 04 §5).
    pub async fn alive(self, alive_key: &str) -> Result<LiveProducer> {
        let token = self
            .session
            .liveliness()
            .declare_token(alive_key.to_string())
            .await
            .map_err(|e| anyhow!("declare alive token {alive_key}: {e}"))?;
        Ok(LiveProducer {
            token: Some(token),
            responders: self.responders,
        })
    }

    /// Finish **without** presence: hand back the declared responders and
    /// never mint a token. This is the mock/synthetic path ([`crate::
    /// generate`]'s impersonated producers, RFC 13 §5) — a tool that
    /// answers for a producer must not also claim its presence. A real
    /// producer wants [`alive`](Self::alive).
    pub fn without_alive(self) -> Vec<Responder> {
        self.responders
    }
}

/// A producer that is up: queryables declared, then `alive` — held while
/// serving.
#[derive(Debug)]
pub struct LiveProducer {
    token: Option<LivelinessToken>,
    /// The declared responders, in bring-up order — drive them
    /// ([`Responder::next`]) to serve.
    pub responders: Vec<Responder>,
}

impl LiveProducer {
    /// Retire in the reverse of bring-up: retract `alive` **first** (the
    /// roster must stop attributing silence to a producer that is going
    /// away), then undeclare the queryables, acknowledged.
    ///
    /// A token that will not retract still bails before the queryables, and
    /// deliberately: "alive ⇒ callable" (RFC 04 §5) is a claim that outlives
    /// this call, and stripping callability while presence stands would
    /// manufacture exactly the false negative the ordering exists to prevent.
    /// The queryables themselves drain (#346): one that will not undeclare
    /// must not leave the rest declared, and the failures are reported
    /// together — the `bus::teardown` shape, shared with the monitor and the
    /// replayer.
    pub async fn retire(mut self) -> Result<()> {
        if let Some(token) = self.token.take() {
            token
                .undeclare()
                .await
                .map_err(|e| anyhow!("retract alive token: {e}"))?;
        }
        let declared: Vec<(String, Responder)> = self
            .responders
            .drain(..)
            .map(|r| (r.key.clone(), r))
            .collect();
        crate::bus::teardown::drain_undeclare(declared, Responder::undeclare).await
    }
}
