//! The fan-in query discipline (RFC 05 §2.1) — moved verbatim from
//! zenctl's `bus.rs`; this stays the single chokepoint for fleet GETs.

use std::time::Duration;

use crate::{Error, Result};
use zenoh::Session;
use zenoh::qos::Priority;
use zenoh::query::{ConsolidationMode, QueryTarget};

/// How a producer answered a procedure call.
///
/// `Clone` is a refcount bump on the payload, not a copy — which is what lets
/// a GUI hold an answer in widget state without paying for it.
#[derive(Debug, Clone)]
pub enum Answer {
    /// A value reply — RFC 05 §3: "a reply always indicates success".
    /// Carried as zenoh's refcounted buffer: cloning is a refcount bump,
    /// and consumers decode via `reader()`/`to_bytes()` (a `Cow` — it
    /// copies only when the payload arrived fragmented). Report §14's
    /// zero-copy discipline: the old `to_bytes().to_vec()` double copy per
    /// reply is retired.
    Value(zenoh::bytes::ZBytes),
    /// An error reply (`reply_err`), carrying the `{error, message}` envelope
    /// when it parses. RFC 05 §3: "an error always indicates failure".
    Error { name: String, message: String },
}

/// One answer, attributed by the key it was sent on.
#[derive(Debug, Clone)]
pub struct FleetAnswer {
    /// The reply's **own** key expression: the attribution, and the concrete
    /// key a follow-up must be addressed to (RFC 05 §2.1). A key that names
    /// no service a reader knows is still a fact about who answered (O1).
    ///
    /// Empty for an error reply, which zenoh gives no sample and therefore no
    /// key. v1's `origin`, the origin chunk the v1 grammar read out of this
    /// key, left with the v1 grammar (#612, FJ9).
    pub key: String,
    /// The reply's declared encoding, when it carried one.
    ///
    /// A caller that speaks a specific wire (`@blob`'s postcard replies, say)
    /// needs to tell "answered in a dialect we do not speak" from "did not
    /// answer": the first is an observation, the second is silence, and RFC 09
    /// §5.1 O4 forbids rendering them alike.
    pub encoding: Option<String>,
    /// The reply's attachment, when it carried one (refcounted, like the
    /// payload). `None` on an error reply is the only truth available:
    /// zenoh's `ReplyError` carries no attachment — a fact about the wire,
    /// not an unobserved field.
    pub attachment: Option<zenoh::bytes::ZBytes>,
    /// The reply sample's HLC, when it carried one (#215). A reply is a
    /// sample and is stamped like one — by the first timestamping node it
    /// passed, not necessarily the responder (RFC 09 §5.1 O7). `None` on an
    /// error reply, which is not a sample, and on a reply nothing stamped.
    pub timestamp: Option<zenoh::time::Timestamp>,
    pub answer: Answer,
}

/// What one GET may vary — everything RFC 05 §2.1 does **not** fix.
///
/// The §2.1 triple is not a knob and deliberately has no field here: it is
/// applied by `disciplined_get` to every GET this crate issues. What a
/// caller does choose is the timeout, the request body, an attachment riding
/// beside it (#126), the query's priority (RFC 04 §3, RFC 07 §2.6) and
/// whether replies from *outside* the selector are accepted.
///
/// A spec struct rather than named sibling functions: `fleet_get_at`
/// (priority) and `fleet_get_call` (attachment) used to be those siblings, and
/// `_at` had come to mean two things — this axis, and the injected-clock
/// convention (`ingest_at`, `ZrecWriter::new_at`). The greps the siblings
/// bought survive as setter greps: "who issues bulk GETs?" is
/// `grep '\.priority('`, "who sends attachments on queries?" is
/// `grep '\.attachment('`.
#[derive(Debug, Clone)]
pub struct GetOpts {
    timeout: Duration,
    payload: Option<Vec<u8>>,
    attachment: Option<Vec<u8>>,
    priority: Priority,
    accept_any: bool,
    max_replies: usize,
    /// What the bound cost, filled in by the GET (#339). Shared rather than
    /// returned — see [`GetOpts::elided`].
    elided: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// How many replies a GET keeps unless the caller says otherwise (#339).
///
/// Every fan-out here was unbounded: `collect_answers`, `fetch_timed` and
/// `admin_get` pushed every reply into a `Vec`, each holding a refcounted
/// payload, so a `**` sweep against a router with a large storage was
/// unbounded memory in a tool that bounds everything else it accumulates.
///
/// 4096 is chosen against what the fan-out *means*: a fleet GET is one reply
/// per producer per key, and a fleet with four thousand replying entities on
/// one selector is past what any of these renderers show anyway. A caller
/// that genuinely wants more says so, and hears what the last bound cost.
pub const DEFAULT_MAX_REPLIES: usize = 4096;

impl GetOpts {
    /// A plain GET, bounded by `timeout`.
    ///
    /// [`Priority::DEFAULT`] is `Priority::Data` — byte-identical to setting
    /// no priority at all, which is what every un-annotated GET did before
    /// this type existed.
    pub fn new(timeout: Duration) -> Self {
        GetOpts {
            timeout,
            payload: None,
            attachment: None,
            priority: Priority::DEFAULT,
            accept_any: false,
            max_replies: DEFAULT_MAX_REPLIES,
            elided: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// The request body, when there is one. `None` is the common case and
    /// costs nothing to say.
    pub fn payload(mut self, payload: Option<Vec<u8>>) -> Self {
        self.payload = payload;
        self
    }

    /// A query attachment (#126), verbatim — never schema-encoded. The encode
    /// ladder is for bodies; an attachment is outside the registry's
    /// vocabulary (#117), on a query exactly as on a publish.
    pub fn attachment(mut self, attachment: Option<Vec<u8>>) -> Self {
        self.attachment = attachment;
        self
    }

    /// State the query's priority (RFC 04 §3, RFC 07 §2.6).
    ///
    /// Replies inherit the *query's* QoS — a server-side setter is a no-op —
    /// so a bulk plane's priority can only be decided here. RFC 07 §2.6 makes
    /// that a caller obligation rather than a suggestion: `@blob` GETs MUST
    /// ride at [`Priority::DataLow`], or one operator fetching a debug bundle
    /// starves the telemetry and alerts sharing the link.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Accept replies on keys **outside** the selector
    /// ([`zenoh::query::ReplyKeyExpr::Any`]) — the querying-subscriber
    /// pattern.
    ///
    /// The `@adv` cache replies with the cached sample on the *sample's* own
    /// key, outside a `<key>/@adv/**` selector, and zenoh drops such replies
    /// unless the caller opts in. Harmless on a rung whose replies sit inside
    /// the selector anyway.
    pub fn accept_any(mut self) -> Self {
        self.accept_any = true;
        self
    }

    /// The bound this GET runs under.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Keep at most `max` replies (#339). Zero is clamped to one: a GET that
    /// kept nothing would report silence, and silence is never a verdict
    /// (RFC 05 §3.1).
    pub fn max_replies(mut self, max: usize) -> Self {
        self.max_replies = max.max(1);
        self
    }

    /// The reply bound in force.
    pub fn reply_bound(&self) -> usize {
        self.max_replies
    }

    /// **What the bound cost**: replies that arrived and were not kept,
    /// across every GET run under these options (RFC 13 §3 O6 — a bound that
    /// hides data must say so).
    ///
    /// It rides here, on the object that *states* the bound, rather than in
    /// the return type, for the reason every other bounded structure in this
    /// crate keeps its own ledger (`StatsTable::evicted`,
    /// `Retention::evicted`, `BoundedLru::admit`): the thing that owns the
    /// ceiling owns the count of what the ceiling refused. A caller reads it
    /// beside the answers it just got:
    ///
    /// ```ignore
    /// let opts = GetOpts::new(timeout);
    /// let answers = fleet_get(&fleet, key, &opts).await?;
    /// if opts.elided() > 0 { /* say so — never render this as "all of them" */ }
    /// ```
    ///
    /// The count is exact: past the bound the replies are still drained, they
    /// are simply not kept. Draining is what makes the number honest; *keeping*
    /// is what was unbounded.
    pub fn elided(&self) -> u64 {
        self.elided.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Forget what earlier GETs under these options cost — for a caller that
    /// reuses one `GetOpts` and reports per GET rather than per run.
    pub fn reset_elided(&self) {
        self.elided.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Add to the ledger — for the drains that live in another module
    /// ([`crate::admin_get_within`]) and keep their own reply shape.
    pub(crate) fn note_elided(&self, n: u64) {
        if n > 0 {
            self.elided
                .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// **The** `session.get` of this crate (RFC 05 §2.1) — no other module issues
/// one, which is what makes the discipline checkable by grep rather than by
/// review.
///
/// Two of the three things §2.1 requires are set here, once:
///
/// 1. **target = All.** The default `BestMatching` short-circuits to a single
///    queryable the moment any matching one is declared `complete` — "one
///    storage config away from silently collapsing the fleet to one reply".
/// 2. **consolidation = None.** Default consolidation keeps one reply *per
///    reply key*; belt-and-braces against a producer that wrongly echoes the
///    wildcard selector instead of replying on its own concrete key.
///
/// The third — **attribution by the reply's own key**, never by the key we
/// asked on — belongs to whoever drains the channel, and lives in
/// `answer_of` for the [`FleetAnswer`] path.
///
/// The error is the middleware's own, unwrapped: every caller has a better
/// sentence to wrap it in than this function does.
pub(crate) async fn disciplined_get(
    session: &Session,
    selector: &str,
    opts: &GetOpts,
) -> Result<zenoh::handlers::FifoChannelHandler<zenoh::query::Reply>> {
    let mut builder = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .priority(opts.priority)
        .timeout(opts.timeout);
    if let Some(body) = opts.payload.clone() {
        builder = builder.payload(body);
    }
    if let Some(att) = opts.attachment.clone() {
        builder = builder.attachment(att);
    }
    if opts.accept_any {
        builder = builder.accept_replies(zenoh::query::ReplyKeyExpr::Any);
    }
    builder.await.map_err(|e| Error::bus("get", "", e))
}

/// GET a selector and collect **every** reply, attributed by its own key.
///
/// The RFC 05 §2.1 fan-in, end to end: `disciplined_get` sets target `All`
/// and consolidation `None`, and `answer_of` attributes each reply by the
/// reply's *own* key — which is what makes a wildcard fan-out legible. The
/// session is the caller's: a raw `get` passes one in no namespace, and the
/// selector is the wire key.
///
/// Silence is deliberately *not* interpreted here (RFC 05 §3.1: "no reply" is
/// not one condition). Callers that need a verdict join this against a
/// presence read. Bounded at [`GetOpts::reply_bound`], and what the bound cost is on
/// [`GetOpts::elided`] (#339).
pub async fn fleet_get(session: &Session, key: &str, opts: &GetOpts) -> Result<Vec<FleetAnswer>> {
    let replies = disciplined_get(session, key, opts)
        .await
        .map_err(|e| Error::bus("query", key.to_string(), e))?;
    let (answers, elided) = collect_answers(replies, opts.max_replies).await;
    opts.note_elided(elided);
    Ok(answers)
}

/// Drain a reply channel into attributed answers — the shared back half of
/// [`fleet_get`] and [`RepeatingQuery`]: one implementation of reply-key
/// attribution and the RFC 05 §3 error envelope, however the query was issued.
///
/// Returns what it kept and **how many it did not** (#339). Past `max` the
/// replies are still drained — the channel is being emptied either way — they
/// are simply not retained, so the count is exact and the memory is bounded.
/// The two are different facts: draining is the fan-in finishing, keeping is
/// what used to be unbounded.
async fn collect_answers(
    replies: zenoh::handlers::FifoChannelHandler<zenoh::query::Reply>,
    max: usize,
) -> (Vec<FleetAnswer>, u64) {
    let mut out = Vec::new();
    let mut elided = 0u64;

    while let Ok(reply) = replies.recv_async().await {
        if out.len() >= max {
            elided += 1;
            continue;
        }
        out.push(answer_of(reply));
    }
    (out, elided)
}

/// One reply, attributed — the per-reply half of [`collect_answers`], shared
/// with the timed drain in [`RepeatingQuery::fetch_timed`] so attribution and
/// the RFC 05 §3 error envelope have exactly one implementation.
fn answer_of(reply: zenoh::query::Reply) -> FleetAnswer {
    match reply.result() {
        Ok(sample) => FleetAnswer {
            key: sample.key_expr().as_str().to_string(),
            encoding: Some(sample.encoding().to_string()),
            attachment: sample.attachment().cloned(),
            timestamp: sample.timestamp().copied(),
            answer: Answer::Value(sample.payload().clone()),
        },
        Err(err) => {
            // The error envelope is `{ "error": "<name>", "message": "…" }`
            // (RFC 05 §3), with reserved names like `error/not-found`. If it
            // does not parse we still surface the bytes — an unreadable
            // refusal is still a refusal.
            let bytes = err.payload().to_bytes();
            let (name, message) = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(v) => (
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("error/unparsed")
                        .to_string(),
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or_default()
                        .to_string(),
                ),
                Err(_) => (
                    "error/unparsed".to_string(),
                    String::from_utf8_lossy(&bytes).to_string(),
                ),
            };
            // An error reply has no sample, so no concrete key to attribute
            // by; zenoh does not surface the responder here.
            FleetAnswer {
                key: String::new(),
                encoding: None,
                attachment: None,
                timestamp: None,
                answer: Answer::Error { name, message },
            }
        }
    }
}

/// A **declared** querier carrying the same RFC 05 §2.1 discipline as
/// [`fleet_get`] (target `All`, consolidation `None`, attribution by reply
/// key), for fetches that re-ask the **same key expression** — watch loops
/// and periodic sweeps. Declaring once lets
/// the network keep routing state warm instead of rebuilding it per GET
/// (report §12's zenoh-1.9 adoption row).
///
/// When to use which:
/// - recurring, same keyexpr → declare a `RepeatingQuery` and `fetch` many
///   times (parameters and payload ride **per get**, never in the declared
///   keyexpr — a `?params` suffix in `key` is a bug here);
/// - genuinely one-shot, or an ad-hoc key → [`fleet_get`].
///
/// Liveliness reads are a different API
/// (`session.liveliness().get()`) with no querier equivalent and stay
/// undeclared; they have a chokepoint of their own,
/// [`crate::bus::presence::liveliness_read`], for spec §8.1's handler rule.
pub struct RepeatingQuery {
    querier: zenoh::query::Querier<'static>,
    /// Replies kept per fetch, and what the bound has cost across all of them
    /// (#339) — the same ledger [`GetOpts`] carries, for the declared path.
    max_replies: usize,
    elided: std::sync::atomic::AtomicU64,
}

/// Declare a repeating query on `key` (a full wire keyexpr, no `?params`).
///
/// The §2.1 discipline is fixed at declaration: target `All`, consolidation
/// `None`, `timeout` for every subsequent fetch.
pub async fn declare_repeating(
    session: &Session,
    key: &str,
    timeout: Duration,
) -> Result<RepeatingQuery> {
    declare(session, key, timeout, false).await
}

/// As [`declare_repeating`], additionally accepting replies **outside** the
/// declared keyexpr (`ReplyKeyExpr::Any`) — the querying-subscriber pattern
/// the `@adv` cache rung needs. A separate constructor because this axis is
/// part of the querier's identity: never reuse one querier across both modes.
pub async fn declare_repeating_any(
    session: &Session,
    key: &str,
    timeout: Duration,
) -> Result<RepeatingQuery> {
    declare(session, key, timeout, true).await
}

async fn declare(
    session: &Session,
    key: &str,
    timeout: Duration,
    accept_any: bool,
) -> Result<RepeatingQuery> {
    let mut builder = session
        .declare_querier(key.to_string())
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout);
    if accept_any {
        builder = builder.accept_replies(zenoh::query::ReplyKeyExpr::Any);
    }
    let querier = crate::bus::teardown::declared("declare querier", &key, builder).await?;
    Ok(RepeatingQuery {
        querier,
        max_replies: DEFAULT_MAX_REPLIES,
        elided: std::sync::atomic::AtomicU64::new(0),
    })
}

impl RepeatingQuery {
    /// The declared key expression.
    pub fn key(&self) -> &str {
        self.querier.key_expr().as_str()
    }

    /// One fetch on the declared keyexpr, every reply attributed by its own
    /// key — [`fleet_get`]'s contract, minus the per-call declaration.
    pub async fn fetch(&self) -> Result<Vec<FleetAnswer>> {
        self.fetch_with("", None).await
    }

    /// As [`fetch`](Self::fetch), with selector parameters and/or a request
    /// payload riding this one get.
    pub async fn fetch_with(
        &self,
        params: &str,
        payload: Option<Vec<u8>>,
    ) -> Result<Vec<FleetAnswer>> {
        let mut builder = self.querier.get();
        if !params.is_empty() {
            builder = builder.parameters(params);
        }
        if let Some(body) = payload {
            builder = builder.payload(body);
        }
        let replies = builder
            .await
            .map_err(|e| Error::bus("query", self.key(), e))?;
        let (answers, elided) = collect_answers(replies, self.max_replies).await;
        self.note_elided(elided);
        Ok(answers)
    }

    /// Keep at most `max` replies per fetch (#339). Zero is clamped to one.
    pub fn max_replies(mut self, max: usize) -> Self {
        self.max_replies = max.max(1);
        self
    }

    /// The reply bound in force.
    pub fn reply_bound(&self) -> usize {
        self.max_replies
    }

    /// Replies this querier's bound refused, across every fetch (RFC 13 §3
    /// O6). See [`GetOpts::elided`] for why the count lives with the bound.
    pub fn elided(&self) -> u64 {
        self.elided.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Forget what earlier fetches through this querier cost — for a caller
    /// that re-runs a sweep and reports per sweep rather than per querier
    /// ([`GetOpts::reset_elided`] is the same call on the one-shot path).
    ///
    /// Without it a per-sweep figure has to be read as a before/after
    /// subtraction, which is not safe when two sweeps overlap on one
    /// declared querier.
    pub fn reset_elided(&self) {
        self.elided.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn note_elided(&self, n: u64) {
        if n > 0 {
            self.elided
                .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// As [`fetch`](Self::fetch), stamping each reply with how long after the
    /// GET it arrived (issue #52).
    ///
    /// This exists because a fan-out call's *call* duration is the time until
    /// the slowest answer, so attributing it to every origin would report a
    /// fast responder's latency as the fleet's worst. Timing each reply where
    /// it is drained is the only place the distinction is available — and it
    /// keeps the RFC 05 §2.1 chokepoint intact rather than forking a second
    /// GET path to measure with.
    pub async fn fetch_timed(&self) -> Result<Vec<(FleetAnswer, Duration)>> {
        let started = std::time::Instant::now();
        let replies = self
            .querier
            .get()
            .await
            .map_err(|e| Error::bus("query", self.key(), e))?;
        let mut out = Vec::new();
        let mut elided = 0u64;
        while let Ok(reply) = replies.recv_async().await {
            let at = started.elapsed();
            if out.len() >= self.max_replies {
                elided += 1;
                continue;
            }
            out.push((answer_of(reply), at));
        }
        self.note_elided(elided);
        Ok(out)
    }

    /// Undeclare, telling the network to drop the routing state. The crate's
    /// idiom: teardown is explicit and awaited, never left to `Drop`.
    pub async fn undeclare(self) -> Result<()> {
        self.querier
            .undeclare()
            .await
            .map_err(|e| Error::bus("undeclare querier", "", e))
    }

    /// Whether any queryable currently matches **this querier** — "someone
    /// serves what *we* ask", a routing fact about the querier this process
    /// declared (RFC 12 §9's allowed half). `false` is not a fleet verdict:
    /// it never means "nobody serves this key" (RFC 05 §3.1).
    pub async fn matching_status(&self) -> Result<bool> {
        self.querier
            .matching_status()
            .await
            .map(|s| s.matching())
            .map_err(|e| Error::bus("matching status", "", e))
    }

    /// Event-driven matching changes for this querier — same honesty bounds
    /// as [`matching_status`](Self::matching_status).
    pub async fn matching_events(&self) -> Result<crate::bus::write::MatchingEvents> {
        crate::bus::write::MatchingEvents::for_querier(&self.querier).await
    }
}
