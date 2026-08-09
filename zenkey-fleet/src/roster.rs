//! The liveliness roster (RFC 04 §5).

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Result;
use zenkey::grammar::with_base;
use zenoh::Session;

/// The fleet-presence roster: who is up, and what they run.
///
/// RFC 04 §5 — a liveliness query on `<base>/v1/*/state/*/alive`. Zero
/// payload bytes: the token *key* is the record. `@catalog` is asked for by
/// name because `*` can never match a verbatim service origin (property D4).
pub async fn roster(
    session: &Session,
    base: &str,
    timeout: Duration,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let catalog_alive = zenkey::selector::service_alive(&zenkey::ServiceOrigin::catalog());
    // The builders are base-relative; this session is deliberately
    // un-namespaced, so it must spell the base itself.
    for expr in [
        with_base(
            base,
            zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet()),
        ),
        with_base(base, catalog_alive),
    ] {
        let Ok(replies) = session.liveliness().get(&expr).timeout(timeout).await else {
            continue;
        };
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            let key = sample.key_expr().as_str();
            let Some(parsed) = zenkey::grammar::parse_full(base, key) else {
                continue;
            };
            let origin = parsed.origin.chunk().to_string();
            // `@catalog`'s token has no producer chunk — the service *is* the
            // producer. Everything else names its producer in position 5.
            let producer = parsed
                .producer
                .as_ref()
                .map(|p| p.chunk())
                .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
            out.entry(origin).or_default().push(producer);
        }
    }
    for producers in out.values_mut() {
        producers.sort();
        producers.dedup();
    }
    Ok(out)
}

/// One producer's story on one node — the enrichment §6.3 promised
/// (issue #40). Every field is honest about its provenance: absent
/// introspection is `None`, never a default (RFC 09 §5.1 O4).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProducerInfo {
    pub name: String,
    /// A liveliness token stands (RFC 04 §5 — the only presence signal).
    pub alive: bool,
    /// From this origin's served introspect slice, when it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
    pub subjects: usize,
    pub procedures: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blob_tiers: Vec<String>,
    /// Deprecated subjects this build still serves — RFC 08 §6's headline
    /// buy ("which hosts still serve a deprecated subject").
    pub deprecated_served: usize,
}

/// Freshness of one declared state subject on this node (RFC 04 §1.2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Freshness {
    pub producer: String,
    pub path: String,
    pub ttl_s: i64,
    /// Seconds since the newest matching sample's HLC stamp; `None` when no
    /// sample answered — which is "not seen", not "fresh" (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_s: Option<i64>,
    /// `age > ttl`, or no sample at all for a declared live subject.
    pub stale: bool,
}

/// One node, joined: liveliness × introspect × state freshness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub origin: String,
    pub producers: Vec<ProducerInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub freshness: Vec<Freshness>,
}

/// How one origin string spells its two framework keys. A host and a service
/// differ in both (`v1/<h>/state/*/alive` + a producer chunk in `@rpc`, versus
/// `v1/@svc/state/alive` + no producer chunk), and a `*` can reach neither
/// other's shape — so the split is made once, up front, rather than guessed
/// per key (D4).
enum Node {
    Host(zenkey::origin::RemoteOrigin),
    Service(zenkey::ServiceOrigin),
}

impl Node {
    fn parse(origin: &str) -> Result<Node> {
        if origin.starts_with('@') {
            zenkey::ServiceOrigin::new(origin)
                .map(Node::Service)
                .map_err(|e| anyhow::anyhow!("{e}"))
        } else {
            zenkey::origin::RemoteOrigin::parse(origin)
                .map(Node::Host)
                .map_err(|e| anyhow::anyhow!("{e} — a hostname is not an origin (RFC 06 §6)"))
        }
    }

    /// This node's liveliness tokens, and nothing else's.
    fn alive_selector(&self) -> String {
        match self {
            Node::Host(o) => {
                zenkey::selector::all_liveliness(zenkey::selector::Scope::origin(o)).to_string()
            }
            Node::Service(o) => zenkey::selector::service_alive(o).to_string(),
        }
    }

    /// This node's producers' `introspect`, and nothing else's.
    fn introspect_selector(&self) -> String {
        match self {
            Node::Host(o) => {
                zenkey::selector::rpc(zenkey::selector::Scope::origin(o), "*", &["introspect"])
                    .to_string()
            }
            Node::Service(o) => zenkey::selector::service_rpc(o, &["introspect"]).to_string(),
        }
    }

    /// This node's state subtree. `**` cannot cross an `@` chunk (D2), so this
    /// cannot pull a plane however deep the subject tail runs.
    fn state_selector(&self) -> String {
        let scope = match self {
            Node::Host(o) => zenkey::selector::Scope::origin(o),
            Node::Service(o) => zenkey::selector::Scope::origin(o),
        };
        zenkey::selector::all_state(scope).to_string()
    }
}

/// Assemble one node's full story (issue #40; feeds `zenctl node info` and
/// the zengui dashboard).
///
/// Three bounded sweeps, **all three scoped to the asked origin** (issue #96 —
/// before it, this re-ran the whole fleet roster and a fleet-wide introspect
/// fan-in per call and then filtered, which the zengui node dashboard pays for
/// on every card click): this origin's liveliness tokens, this origin's
/// producers' introspect replies (per-origin truth, not the fleet-deduped
/// `SliceSet`), and — when `with_freshness` — one state GET on this origin
/// only (D2 guarantees it cannot pull planes).
///
/// Narrower is also *more* honest: the answers can no longer be diluted by a
/// deduplication across origins that never applied to this one.
pub async fn node_info(
    session: &Session,
    base: &str,
    origin: &str,
    timeout: Duration,
    with_freshness: bool,
) -> Result<NodeInfo> {
    let node = Node::parse(origin)?;

    // Liveliness, this origin only. A producer chunk is position 5 for a host;
    // `@catalog`'s token has none — the service *is* the producer.
    let mut alive: Vec<String> = Vec::new();
    let alive_expr = with_base(base, node.alive_selector());
    if let Ok(replies) = session.liveliness().get(&alive_expr).timeout(timeout).await {
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            let Some(parsed) = zenkey::grammar::parse_full(base, sample.key_expr().as_str()) else {
                continue;
            };
            alive.push(
                parsed
                    .producer
                    .as_ref()
                    .map(|p| p.chunk())
                    .unwrap_or_else(|| parsed.origin.chunk().trim_start_matches('@').to_string()),
            );
        }
    }
    alive.sort();
    alive.dedup();

    // Per-origin capabilities: one origin-scoped introspect GET. Replies are
    // still attributed by reply key, so a router that answered for somebody
    // else could not smuggle a slice in.
    let introspect = with_base(base, node.introspect_selector());
    let answers = crate::query::fleet_get(session, base, &introspect, None, timeout)
        .await
        .unwrap_or_default();
    let served: Vec<zenkey::slice::RegistrySlice> = answers
        .into_iter()
        .filter(|a| a.origin == origin)
        .filter_map(|a| {
            let crate::query::Answer::Value(bytes) = a.answer else {
                return None;
            };
            let toml = String::from_utf8_lossy(&bytes.to_bytes()).to_string();
            match zenkey::parse_slice(&toml) {
                Ok(slice) => Some(slice),
                Err(e) => {
                    tracing::warn!(origin, "introspect reply did not parse, skipping: {e}");
                    None
                }
            }
        })
        .collect();
    let mine: Vec<&zenkey::slice::RegistrySlice> = served.iter().collect();

    let mut names: Vec<String> = alive.clone();
    names.extend(mine.iter().map(|s| s.name.clone()));
    names.sort();
    names.dedup();

    let producers: Vec<ProducerInfo> = names
        .iter()
        .map(|name| {
            let slice = mine.iter().find(|s| &s.name == name);
            ProducerInfo {
                name: name.clone(),
                alive: alive.iter().any(|a| a == name),
                app: slice.map(|s| s.app.clone()),
                registry_version: slice.map(|s| s.version.clone()),
                subjects: slice.map(|s| s.subjects.len()).unwrap_or(0),
                procedures: slice.map(|s| s.procedures.len()).unwrap_or(0),
                blob_tiers: slice
                    .map(|s| s.blob.iter().map(|b| b.tier.clone()).collect())
                    .unwrap_or_default(),
                deprecated_served: slice.map(|s| s.deprecated.len()).unwrap_or(0),
            }
        })
        .collect();

    let mut freshness = Vec::new();
    if with_freshness && !mine.is_empty() {
        // One origin-scoped state sweep; join against declared ttl_s.
        let selector = with_base(base, node.state_selector());
        let samples = crate::query::state_snapshot(session, &selector, timeout, None)
            .await
            .unwrap_or_default();
        let now = std::time::SystemTime::now();
        for slice in &mine {
            for subject in &slice.subjects {
                let Some(ttl) = subject.ttl_s else { continue };
                if subject.class != "state" {
                    continue;
                }
                // Newest sample whose tail refines to this subject.
                let age = samples
                    .iter()
                    .filter_map(|s| {
                        let parsed = zenkey::grammar::parse_full(base, &s.key)?;
                        let p = parsed.producer.as_ref()?.name().to_string();
                        if p != slice.name {
                            return None;
                        }
                        let tail: Vec<&str> = parsed.subject.clone();
                        let pattern = zenkey::pattern::SubjectPattern::parse(&subject.path).ok()?;
                        pattern.matches(&tail)?;
                        s.timestamp.map(|t| {
                            now.duration_since(t.get_time().to_system_time())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0)
                        })
                    })
                    .min();
                freshness.push(Freshness {
                    producer: slice.name.clone(),
                    path: subject.path.clone(),
                    ttl_s: ttl,
                    age_s: age,
                    stale: match age {
                        Some(a) => a > ttl,
                        // Declared live state with no sample anywhere: stale
                        // in the sense that matters — but the age stays None.
                        None => true,
                    },
                });
            }
        }
    }

    Ok(NodeInfo {
        origin: origin.to_string(),
        producers,
        freshness,
    })
}
