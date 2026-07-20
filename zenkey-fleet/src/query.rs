//! The fan-in query discipline (RFC 05 §2.1) — moved verbatim from
//! zenctl's `bus.rs`; this stays the single chokepoint for fleet GETs.

use std::time::Duration;

use anyhow::{Context, Result};
use zenkey::grammar::with_base;
use zenkey::{RegistrySlice, parse_slice};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};

/// How a producer answered a procedure call.
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
    Ok(out)
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
/// by name, exactly as [`crate::roster`] does for its alive token; other
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
    // This session is un-namespaced on purpose (RFC 09 §5), so it must
    // spell the base itself — exactly as `service call` composes its full key.
    // Two GETs: the wildcard-producer fan-out, plus `@catalog` by name (a `*`
    // never matches a verbatim origin, D4 — the two cannot double-count).
    let keys = [
        with_base(base, zenkey::selector::fleet_rpc("*", &["introspect"])),
        with_base(
            base,
            zenkey::selector::service_rpc(&zenkey::ServiceOrigin::catalog(), &["introspect"]),
        ),
    ];
    let mut slices = Vec::new();
    for key in keys {
        let answers = fleet_get(session, base, &key, None, timeout).await?;
        for answer in answers {
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
pub async fn state_snapshot(
    session: &Session,
    selector: &str,
    timeout: Duration,
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
        let Ok(sample) = reply.result() else { continue };
        out.push(StateSample {
            key: sample.key_expr().as_str().to_string(),
            timestamp: sample.timestamp().copied(),
            payload_len: sample.payload().len(),
        });
    }
    Ok(out)
}
