//! The measuring side: presence, descriptors, contract retrieval by hash
//! (r3 §3.10), state GETs (S4: target `All`, consolidation `Latest`) and
//! calls (O-rules: consolidation `None`).

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::Value;
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{IfaceId, contract_key};
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::SampleKind;
use zenoh::time::Timestamp;

/// The keys of the liveliness tokens matching `sel`.
pub async fn tokens(session: &zenoh::Session, sel: &str, timeout: Duration) -> Result<Vec<String>> {
    let replies = session
        .liveliness()
        .get(sel)
        .timeout(timeout)
        .await
        .map_err(|e| anyhow!("liveliness get {sel}: {e}"))?;
    let mut out = Vec::new();
    while let Ok(r) = replies.recv_async().await {
        if let Ok(s) = r.result() {
            out.push(s.key_expr().as_str().to_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// One GET reply, flattened.
#[derive(Debug, Clone)]
pub struct Got {
    pub key: String,
    /// `None` for a delete (a tombstone, `reply_del`).
    pub value: Option<Vec<u8>>,
    pub encoding: Option<Encoding>,
    pub timestamp: Option<Timestamp>,
}

/// A GET with explicit target and consolidation. Error replies are returned
/// as `Err` entries, never dropped.
pub async fn get(
    session: &zenoh::Session,
    sel: &str,
    target: QueryTarget,
    consolidation: ConsolidationMode,
    payload: Option<(Vec<u8>, Encoding)>,
    timeout: Duration,
) -> Result<Vec<Result<Got, String>>> {
    let mut b = session.get(sel).target(target).consolidation(consolidation).timeout(timeout);
    if let Some((bytes, enc)) = payload {
        b = b.payload(bytes).encoding(enc);
    }
    let replies = b.await.map_err(|e| anyhow!("get {sel}: {e}"))?;
    let mut out = Vec::new();
    while let Ok(r) = replies.recv_async().await {
        out.push(match r.result() {
            Ok(s) => Ok(Got {
                key: s.key_expr().as_str().to_owned(),
                value: (s.kind() == SampleKind::Put).then(|| s.payload().to_bytes().into_owned()),
                encoding: Some(s.encoding().clone()),
                timestamp: s.timestamp().copied(),
            }),
            Err(e) => Err(String::from_utf8_lossy(&e.payload().to_bytes()).into_owned()),
        });
    }
    Ok(out)
}

/// The state rule's GET: every replier, the latest per key (S4).
pub async fn state_get(session: &zenoh::Session, sel: &str, timeout: Duration) -> Result<Vec<Result<Got, String>>> {
    get(session, sel, QueryTarget::All, ConsolidationMode::Latest, None, timeout).await
}

/// A call: the nearest complete queryable, every reply kept (O-rules).
pub async fn call(
    session: &zenoh::Session,
    key: &str,
    request: (Vec<u8>, Encoding),
    timeout: Duration,
) -> Result<Vec<Result<Got, String>>> {
    get(session, key, QueryTarget::BestMatching, ConsolidationMode::None, Some(request), timeout).await
}

/// The outcome of a contract retrieval.
#[derive(Debug)]
pub struct Fetched {
    pub bundle: Bundle,
    pub bytes: usize,
    /// 1 when the `BestMatching` GET gave a valid bundle, 2 when the retry
    /// with target `All` was needed.
    pub attempts: u32,
    pub elapsed: Duration,
}

/// Contract retrieval by hash (r3 §3.10): GET with `BestMatching`, verify
/// each reply, accept the first valid one; on none, retry once with `All`.
pub async fn fetch_contract(
    session: &zenoh::Session,
    iface: &IfaceId,
    fp: &Fingerprint,
    timeout: Duration,
) -> Result<Fetched> {
    let key = contract_key(iface, fp.hex())?;
    let start = Instant::now();
    for (attempt, target) in [(1, QueryTarget::BestMatching), (2, QueryTarget::All)] {
        let replies = get(session, key.as_str(), target, ConsolidationMode::None, None, timeout).await?;
        for g in replies.into_iter().flatten() {
            if let Some(v) = g.value {
                if let Ok(bundle) = Bundle::verify_expecting(&v, fp) {
                    return Ok(Fetched { bundle, bytes: v.len(), attempts: attempt, elapsed: start.elapsed() });
                }
            }
        }
    }
    Err(anyhow!("contract {iface} {fp} is unavailable"))
}

/// A descriptor, by GET on the instance key.
pub async fn descriptor(session: &zenoh::Session, instance_key: &str, timeout: Duration) -> Result<Value> {
    let replies =
        get(session, instance_key, QueryTarget::BestMatching, ConsolidationMode::None, None, timeout).await?;
    for g in replies.into_iter().flatten() {
        if let Some(v) = g.value {
            return Ok(serde_json::from_slice(&v)?);
        }
    }
    Err(anyhow!("no descriptor at {instance_key}"))
}
