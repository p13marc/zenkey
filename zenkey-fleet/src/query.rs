//! The fan-in query discipline (RFC 05 §2.1) — moved verbatim from
//! zenctl's `bus.rs`; this stays the single chokepoint for fleet GETs.

use std::time::Duration;

use anyhow::{Context, Result};
use zenkey::grammar::with_base;
use zenkey::{RegistrySlice, parse_slice};
use zenoh::Session;
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

/// One host's answer, attributed to the origin that actually replied.
#[derive(Debug, Clone)]
pub struct FleetAnswer {
    pub origin: String,
    pub answer: Answer,
}

/// Call a procedure and collect **every** reply, attributed by origin.
///
/// The three things RFC 05 §2.1 requires, in the one place they cannot be
/// forgotten:
///
/// 1. **target = All.** The default `BestMatching` short-circuits to a single
///    queryable the moment any matching one is declared `complete` — "one
///    storage config away from silently collapsing the fleet to one reply".
/// 2. **consolidation = None.** Default consolidation keeps one reply *per
///    reply key*; belt-and-braces against a producer that wrongly echoes the
///    wildcard selector instead of replying on its own concrete key.
/// 3. **Attribution by the reply's own key**, never by the key we asked on —
///    that is what makes `*`-origin fan-out legible.
///
/// Silence is deliberately *not* interpreted here (RFC 05 §3.1: "no reply" is
/// not one condition). Callers that need a verdict join this against the
/// liveliness roster; see `cmd::doctor`.
pub async fn fleet_get(
    session: &Session,
    base: &str,
    key: &str,
    payload: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Vec<FleetAnswer>> {
    let mut builder = session
        .get(key)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout);
    if let Some(body) = payload {
        builder = builder.payload(body);
    }
    let replies = builder
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("query failed: {key}"))?;
    Ok(collect_answers(base, replies).await)
}

/// Drain a reply channel into attributed answers — the shared back half of
/// [`fleet_get`] and [`RepeatingQuery`]: one implementation of reply-key
/// attribution and the RFC 05 §3 error envelope, however the query was issued.
async fn collect_answers(
    base: &str,
    replies: zenoh::handlers::FifoChannelHandler<zenoh::query::Reply>,
) -> Vec<FleetAnswer> {
    let mut out = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) => {
                let origin = origin_of(base, sample.key_expr().as_str());
                out.push(FleetAnswer {
                    origin,
                    answer: Answer::Value(sample.payload().clone()),
                });
            }
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
                out.push(FleetAnswer {
                    origin: "?".to_string(),
                    answer: Answer::Error { name, message },
                });
            }
        }
    }
    out
}

/// A **declared** querier carrying the same RFC 05 §2.1 discipline as
/// [`fleet_get`] (target `All`, consolidation `None`, attribution by reply
/// key), for fetches that re-ask the **same key expression** — watch loops,
/// the schema cache's re-asks, registry sweeps, doctor. Declaring once lets
/// the network keep routing state warm instead of rebuilding it per GET
/// (report §12's zenoh-1.9 adoption row).
///
/// When to use which:
/// - recurring, same keyexpr → declare a `RepeatingQuery` and `fetch` many
///   times (parameters and payload ride **per get**, never in the declared
///   keyexpr — a `?params` suffix in `key` is a bug here);
/// - genuinely one-shot, or an ad-hoc key → [`fleet_get`].
///
/// Liveliness sweeps ([`crate::roster`]) are a different API
/// (`session.liveliness().get()`) with no querier equivalent and stay
/// undeclared.
pub struct RepeatingQuery {
    querier: zenoh::query::Querier<'static>,
    base: String,
}

/// Declare a repeating query on `key` (a full wire keyexpr, no `?params`).
///
/// The §2.1 discipline is fixed at declaration: target `All`, consolidation
/// `None`, `timeout` for every subsequent fetch.
pub async fn declare_repeating(
    session: &Session,
    base: &str,
    key: &str,
    timeout: Duration,
) -> Result<RepeatingQuery> {
    declare(session, base, key, timeout, false).await
}

/// As [`declare_repeating`], additionally accepting replies **outside** the
/// declared keyexpr (`ReplyKeyExpr::Any`) — the querying-subscriber pattern
/// the `@adv` cache rung needs. A separate constructor because this axis is
/// part of the querier's identity: never reuse one querier across both modes.
pub async fn declare_repeating_any(
    session: &Session,
    base: &str,
    key: &str,
    timeout: Duration,
) -> Result<RepeatingQuery> {
    declare(session, base, key, timeout, true).await
}

async fn declare(
    session: &Session,
    base: &str,
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
    let querier = builder
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("declare querier failed: {key}"))?;
    Ok(RepeatingQuery {
        querier,
        base: base.to_string(),
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
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("repeating query failed: {}", self.key()))?;
        Ok(collect_answers(&self.base, replies).await)
    }

    /// Undeclare, telling the network to drop the routing state. The crate's
    /// idiom: teardown is explicit and awaited, never left to `Drop`.
    pub async fn undeclare(self) -> Result<()> {
        self.querier
            .undeclare()
            .await
            .map_err(|e| anyhow::anyhow!("undeclare querier: {e}"))
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
            .map_err(|e| anyhow::anyhow!("matching status: {e}"))
    }

    /// Event-driven matching changes for this querier — same honesty bounds
    /// as [`matching_status`](Self::matching_status).
    pub async fn matching_events(&self) -> Result<crate::write::MatchingEvents> {
        crate::write::MatchingEvents::for_querier(&self.querier).await
    }
}

/// The origin chunk of a wire key, via the grammar (never by index — RFC 03
/// §1.1: positions are relative to the configured base).
fn origin_of(base: &str, key: &str) -> String {
    zenkey::grammar::parse_full(base, key)
        .map(|k| k.origin.chunk().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Discover every live producer's registry slice **from the bus**, with nothing
/// compiled in (RFC 08 §6: "generic explorer tooling … needs no compiled-in
/// registry").
///
/// Every producer MUST serve its registry slice as TOML on
/// `@rpc/<producer>/introspect`. This fans one wildcard-producer `introspect`
/// GET across the fleet — `<base>/v1/*/@rpc/*/introspect` — and parses each
/// reply. It is the same introspect+`parse_slice` path `doctor` walks, minus
/// the compiled-in diff: here the served slice *is* the answer.
///
/// A reply that does not parse is reported to stderr and skipped, never fatal:
/// one malformed producer must not blind the tool to every other producer's
/// slice. The tuple's first element is the producer (or service) base name the
/// slice declares (`slice.name`), matching the compiled path's producer column.
///
/// A verbatim service origin is unmatchable by the `*` of a fleet selector
/// (grammar property D4), so the wildcard sweep cannot enumerate services.
/// The well-known `@catalog` identity service (RFC 06 §5) is therefore asked
/// by name, exactly as [`crate::roster()`] does for its alive token; other
/// service origins remain reachable only via local registry files
/// (`doctor --registry` asks each declared `service_origin` by name).
pub async fn fleet_registry(
    session: &Session,
    base: &str,
    timeout: Duration,
) -> Result<Vec<(String, RegistrySlice)>> {
    Ok(fleet_registry_raw(session, base, timeout)
        .await?
        .into_iter()
        .map(|(slice, _)| (slice.name.clone(), slice))
        .collect())
}

/// As [`fleet_registry`], additionally yielding each reply's raw TOML text
/// (the artifact the slice cache persists).
pub async fn fleet_registry_raw(
    session: &Session,
    base: &str,
    timeout: Duration,
) -> Result<Vec<(RegistrySlice, String)>> {
    let repeating = RepeatingRegistry::declare(session, base, timeout).await?;
    let slices = repeating.fetch().await?;
    repeating.undeclare().await?;
    Ok(slices)
}

/// The registry sweep as a **declared** pair of queriers (#37) — for callers
/// that re-run the sweep (`--watch topic list`, doctor's second pass, a GUI
/// refresh). One-shot callers keep [`fleet_registry`].
///
/// Two queriers, not one: the wildcard-producer fan-out plus `@catalog` by
/// name (a `*` never matches a verbatim origin, D4 — the two cannot
/// double-count; same reasoning as [`fleet_registry`]).
pub struct RepeatingRegistry {
    wildcard: RepeatingQuery,
    catalog: RepeatingQuery,
}

impl RepeatingRegistry {
    pub async fn declare(session: &Session, base: &str, timeout: Duration) -> Result<Self> {
        // This session is un-namespaced on purpose (RFC 09 §5), so it must
        // spell the base itself — exactly as `service call` composes its key.
        let wildcard = with_base(base, zenkey::selector::fleet_rpc("*", &["introspect"]));
        let catalog = with_base(
            base,
            zenkey::selector::service_rpc(&zenkey::ServiceOrigin::catalog(), &["introspect"]),
        );
        Ok(RepeatingRegistry {
            wildcard: declare_repeating(session, base, &wildcard, timeout).await?,
            catalog: declare_repeating(session, base, &catalog, timeout).await?,
        })
    }

    /// One sweep: every parsed slice with its raw TOML. A reply that does not
    /// parse is logged and skipped, never fatal — one malformed producer must
    /// not blind the tool to every other producer's slice.
    pub async fn fetch(&self) -> Result<Vec<(RegistrySlice, String)>> {
        let mut slices = Vec::new();
        for q in [&self.wildcard, &self.catalog] {
            for answer in q.fetch().await? {
                let Answer::Value(bytes) = answer.answer else {
                    continue;
                };
                let served_toml = String::from_utf8_lossy(&bytes.to_bytes()).to_string();
                match parse_slice(&served_toml) {
                    Ok(slice) => slices.push((slice, served_toml)),
                    Err(e) => tracing::warn!(
                        origin = %answer.origin,
                        "introspect reply did not parse, skipping: {e}"
                    ),
                }
            }
        }
        Ok(slices)
    }

    pub async fn undeclare(self) -> Result<()> {
        self.wildcard.undeclare().await?;
        self.catalog.undeclare().await
    }
}

/// One state sample from a snapshot GET.
#[derive(Debug, Clone)]
pub struct StateSample {
    /// Full wire key.
    pub key: String,
    /// HLC timestamp, when the deployment stamps samples (RFC 04 §4
    /// requires it for LWW to be meaningful — its absence is itself a
    /// doctor-grade observation).
    pub timestamp: Option<zenoh::time::Timestamp>,
    pub payload_len: usize,
}

/// GET the current state under a selector with the fan-in discipline
/// (target All, consolidation None) — the doctor's freshness check
/// (RFC 04 §1.2) consumes the timestamps. Same chokepoint posture as
/// [`fleet_get`]: no subcommand issues a raw `session.get`.
///
/// `max` bounds the samples **drained** (`doctor --sample N`): the loop
/// stops reading at the cap, so a bounded sweep is cheaper, not merely
/// quieter. `None` drains every reply.
pub async fn state_snapshot(
    session: &Session,
    selector: &str,
    timeout: Duration,
    max: Option<usize>,
) -> Result<Vec<StateSample>> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("state snapshot failed: {selector}"))?;
    let mut out = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        if max.is_some_and(|m| out.len() >= m) {
            break;
        }
        let Ok(sample) = reply.result() else { continue };
        out.push(StateSample {
            key: sample.key_expr().as_str().to_string(),
            timestamp: sample.timestamp().copied(),
            payload_len: sample.payload().len(),
        });
    }
    Ok(out)
}

/// Which rung of the fetch ladder produced a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSource {
    /// A GET on the concrete key answered — a router storage (or any plain
    /// queryable standing at that key).
    Storage,
    /// The publisher's AdvancedPublisher cache answered on `<key>/@adv/**`.
    Cache,
    /// A brief bounded subscription caught a live sample.
    Window,
}

/// One fetched value with its provenance.
#[derive(Debug, Clone)]
pub struct FetchedValue {
    /// The concrete key the value arrived on.
    pub key: String,
    pub payload: zenoh::bytes::ZBytes,
    pub encoding: String,
    pub timestamp: Option<zenoh::time::Timestamp>,
    pub source: ValueSource,
}

/// The outcome: a value, or an attributed nothing.
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Value(FetchedValue),
    /// Every rung was tried and none answered — a non-verdict, stated with
    /// exactly what was asked (RFC 05 §3.1: silence never becomes a claim
    /// that no value exists).
    None {
        attempted: [&'static str; 3],
    },
}

/// Fetch ladder bounds.
#[derive(Debug, Clone, Copy)]
pub struct FetchSpec {
    /// Per-GET timeout (two GETs happen: concrete key, then `@adv` cache).
    pub get_timeout: Duration,
    /// The final subscribe-window rung's duration.
    pub window: Duration,
}

impl Default for FetchSpec {
    fn default() -> Self {
        FetchSpec {
            get_timeout: Duration::from_secs(2),
            window: Duration::from_millis(1500),
        }
    }
}

/// Fetch one concrete key's current value **on demand** — the value half of
/// the lazy-observation contract (issue #84): a selection retrieves one
/// value; nothing is prefetched, nothing stays subscribed.
///
/// The ladder, each rung bounded:
/// 1. GET the concrete key (storages answer; RFC 04 §3.2's "a plain GET does
///    not reach publisher caches" is exactly why rung 2 exists);
/// 2. GET `<key>/@adv/**?_max=1` — zenoh-ext's AdvancedPublisher cache
///    declares its queryable there and replies with the cached sample on its
///    own concrete key (`@adv` is verbatim, so no data selector ever collides
///    with it — RFC 03 §4 D2 working in our favor);
/// 3. a brief callback subscription on the key, first sample wins.
///
/// Several answers on a rung (multiple storages) resolve by latest HLC
/// timestamp; unstamped answers lose to stamped ones (RFC 04 §1.2's LWW).
pub async fn fetch_value(session: &Session, key: &str, spec: FetchSpec) -> Result<FetchOutcome> {
    // Rung 1 + 2: bounded GETs.
    for (selector, source) in [
        (key.to_string(), ValueSource::Storage),
        (format!("{key}/@adv/**?_max=1"), ValueSource::Cache),
    ] {
        if let Some(v) = get_latest(session, &selector, source, spec.get_timeout).await? {
            return Ok(FetchOutcome::Value(v));
        }
    }

    // Rung 3: a window. The subscriber is explicitly undeclared afterwards —
    // the window closes, provably.
    let (tx, rx) = tokio::sync::oneshot::channel::<FetchedValue>();
    let tx = std::sync::Mutex::new(Some(tx));
    let subscriber = session
        .declare_subscriber(key)
        .callback(move |sample| {
            if let Some(tx) = tx.lock().expect("fetch window lock").take() {
                let _ = tx.send(FetchedValue {
                    key: sample.key_expr().as_str().to_string(),
                    payload: sample.payload().clone(),
                    encoding: sample.encoding().to_string(),
                    timestamp: sample.timestamp().copied(),
                    source: ValueSource::Window,
                });
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("window subscribe {key}: {e}"))?;
    let caught = tokio::time::timeout(spec.window, rx).await;
    subscriber
        .undeclare()
        .await
        .map_err(|e| anyhow::anyhow!("window undeclare {key}: {e}"))?;
    if let Ok(Ok(v)) = caught {
        return Ok(FetchOutcome::Value(v));
    }

    Ok(FetchOutcome::None {
        attempted: ["get", "@adv cache", "subscribe window"],
    })
}

async fn get_latest(
    session: &Session,
    selector: &str,
    source: ValueSource,
    timeout: Duration,
) -> Result<Option<FetchedValue>> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        // The @adv cache replies with the cached sample on the sample's OWN
        // key — outside the `<key>/@adv/**` selector — and zenoh drops such
        // replies unless the caller opts in. This is the querying-subscriber
        // pattern; harmless for the storage rung, whose replies sit inside
        // the selector anyway.
        .accept_replies(zenoh::query::ReplyKeyExpr::Any)
        .timeout(timeout)
        .await
        .map_err(|e| anyhow::anyhow!("get {selector}: {e}"))?;
    let mut best: Option<FetchedValue> = None;
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        let candidate = FetchedValue {
            key: sample.key_expr().as_str().to_string(),
            payload: sample.payload().clone(),
            encoding: sample.encoding().to_string(),
            timestamp: sample.timestamp().copied(),
            source,
        };
        best = Some(match best.take() {
            None => candidate,
            // Latest HLC wins; stamped beats unstamped (RFC 04 §1.2 LWW).
            Some(cur) => match (cur.timestamp, candidate.timestamp) {
                (Some(a), Some(b)) if b > a => candidate,
                (None, Some(_)) => candidate,
                _ => cur,
            },
        });
    }
    Ok(best)
}
