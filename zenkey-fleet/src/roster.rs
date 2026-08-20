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
            let Some((origin, producer)) = token_identity(base, key) else {
                continue;
            };
            out.entry(origin).or_default().push(producer);
        }
    }
    for producers in out.values_mut() {
        producers.sort();
        producers.dedup();
    }
    Ok(out)
}

/// A live roster, driven by liveliness events rather than polled (#56).
///
/// The roster is *pushed* by the bus, so a `--watch` on it has no business
/// running a timer. Both explorers had the same loop — seed with one GET,
/// subscribe with history, coalesce a burst, re-render only on a real change
/// — and zenctl's copy had drifted into `cmd/node.rs` alongside a duplicate of
/// the polling driver's cycle body (issue #207). This is that loop, once.
///
/// Zero data-plane subscribers by construction: the monitor is started with an
/// empty selector list and only liveliness selectors, so watching the roster
/// costs nothing on the data plane (the lazy contract, #85).
pub struct RosterWatch {
    monitor: crate::Monitor,
    events: crate::EventStream,
    roster: BTreeMap<String, Vec<String>>,
    base: String,
}

/// What one coalesced burst of liveliness events did to the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosterChange {
    /// At least one producer appeared. The caller may want to re-read the
    /// registry — a new producer can serve a slice nothing has asked for yet —
    /// and this says so once per burst rather than once per event.
    pub node_up: bool,
    /// At least one producer went away.
    pub node_down: bool,
}

/// How long a burst is drained before rendering. Liveliness storms arrive in
/// clumps (a host booting declares every producer at once); one frame per
/// clump beats one frame per token.
const BURST_QUIET: Duration = Duration::from_millis(50);

impl RosterWatch {
    /// Subscribe, then seed.
    ///
    /// Both, and in that order: the monitor's history-backed events can land
    /// in the broadcast before this task drains it, so history alone races,
    /// and a GET alone would miss everything after it. Duplicates from a late
    /// history event are absorbed by [`apply_token`]'s idempotence.
    pub async fn start(session: &Session, base: &str, timeout: Duration) -> Result<RosterWatch> {
        let liveliness = vec![
            with_base(
                base,
                zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet()),
            ),
            with_base(
                base,
                zenkey::selector::service_alive(&zenkey::ServiceOrigin::catalog()),
            ),
        ];
        let monitor = crate::Monitor::start(
            session,
            crate::MonitorSpec {
                selectors: vec![],
                liveliness,
                ..Default::default()
            },
        )
        .await?;
        let events = monitor.events();
        let roster = roster(session, base, timeout).await?;
        Ok(RosterWatch {
            monitor,
            events,
            roster,
            base: base.to_string(),
        })
    }

    /// The roster as it stands. Valid immediately after [`start`](Self::start)
    /// — the first frame is the seed, and "0 producers" is a statement rather
    /// than silence (RFC 05 §3.1).
    pub fn roster(&self) -> &BTreeMap<String, Vec<String>> {
        &self.roster
    }

    /// Wait for the roster to actually change, coalescing a burst into one
    /// answer. `None` = the event stream closed.
    ///
    /// Never returns for a burst that changed nothing, so a caller can render
    /// on every `Some` without checking.
    pub async fn next_change(&mut self) -> Option<RosterChange> {
        loop {
            let mut change = RosterChange {
                node_up: false,
                node_down: false,
            };
            let mut pending = self.events.recv().await;
            loop {
                match pending {
                    None => return None,
                    Some(crate::StreamItem::Dropped(_)) => {}
                    Some(crate::StreamItem::Event(ev)) => {
                        let transition = match ev {
                            crate::FleetEvent::NodeUp(key) => Some((key, true)),
                            crate::FleetEvent::NodeDown(key) => Some((key, false)),
                            _ => None,
                        };
                        if let Some((key, up)) = transition
                            && apply_token(&mut self.roster, &self.base, &key, up)
                        {
                            if up {
                                change.node_up = true;
                            } else {
                                change.node_down = true;
                            }
                        }
                    }
                }
                match tokio::time::timeout(BURST_QUIET, self.events.recv()).await {
                    Ok(next) => pending = next,
                    Err(_) => break,
                }
            }
            if change.node_up || change.node_down {
                return Some(change);
            }
        }
    }

    /// Release the subscriptions.
    ///
    /// On every exit path, which the zenctl original managed only on Ctrl-C:
    /// its channel-closed arm returned before reaching `monitor.stop()`, so a
    /// closed stream leaked the liveliness subscribers (#207).
    pub async fn stop(self) {
        self.monitor.stop();
    }
}

/// Who a liveliness token names: `(origin, producer)`, or `None` when the key
/// is not a token under this base.
///
/// One home for a rule that had two (issue #207): `roster()` read it off a GET
/// reply and `zenctl node list --watch` re-derived it, character for
/// character, from a `NodeUp`/`NodeDown` event. `@catalog`'s token has no
/// producer chunk — the service *is* the producer — and everything else names
/// its producer in position 5.
pub fn token_identity(base: &str, key: &str) -> Option<(String, String)> {
    let parsed = zenkey::grammar::parse_full(base, key)?;
    let origin = parsed.origin.chunk().to_string();
    let producer = parsed
        .producer
        .as_ref()
        .map(|p| p.chunk())
        .unwrap_or_else(|| origin.trim_start_matches('@').to_string());
    Some((origin, producer))
}

/// Apply one liveliness transition to a roster. Returns whether it changed
/// anything — a burst of no-op events must not force a re-render.
///
/// Idempotent on the way up, which is what makes a seeded roster safe: the
/// history-backed events a monitor replays can land after the one-shot GET
/// that seeded it, and a duplicate `NodeUp` returns `false` rather than
/// double-listing the producer.
pub fn apply_token(
    roster: &mut BTreeMap<String, Vec<String>>,
    base: &str,
    key: &str,
    up: bool,
) -> bool {
    let Some((origin, producer)) = token_identity(base, key) else {
        return false;
    };
    if up {
        let entry = roster.entry(origin).or_default();
        if entry.contains(&producer) {
            return false;
        }
        entry.push(producer);
        entry.sort();
        return true;
    }
    let Some(entry) = roster.get_mut(&origin) else {
        return false;
    };
    let before = entry.len();
    entry.retain(|p| p != &producer);
    let changed = entry.len() != before;
    if entry.is_empty() {
        roster.remove(&origin);
    }
    changed
}

/// Roster → typed rows, joining the slice facts when given (`--verbose`).
/// Absent slice = `None` fields, never a default (RFC 09 §5.1 O4).
pub fn node_rows(
    roster: &BTreeMap<String, Vec<String>>,
    slices: Option<&crate::SliceSet>,
) -> crate::report::NodeList {
    let mut nodes = Vec::new();
    for (origin, producers) in roster {
        for producer in producers {
            let joined = slices.and_then(|s| {
                // Instance suffixes share the base slice (RFC 03 §1.5).
                let base_name = zenkey::grammar::Producer::parse_chunk(producer)
                    .map(|pr| pr.name().to_string())
                    .unwrap_or_else(|_| producer.clone());
                s.get(&base_name)
            });
            nodes.push(crate::report::NodeRow {
                origin: origin.clone(),
                producer: producer.clone(),
                app: joined.map(|s| s.app.clone()),
                registry_version: joined.map(|s| s.version.clone()),
            });
        }
    }
    crate::report::NodeList {
        nodes,
        slices_joined: slices.is_some(),
    }
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
    /// Declared `@media` streams (RFC 08 §2/§6, v1.16 — the slice finally
    /// carries what §6 claimed through v1.7): discoverable off the bus, so
    /// a viewer can enumerate streams without a compiled-in registry.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub media: Vec<MediaStreamInfo>,
    /// Deprecated subjects this build still serves — RFC 08 §6's headline
    /// buy ("which hosts still serve a deprecated subject").
    pub deprecated_served: usize,
}

/// One declared media stream, as the slice states it (RFC 08 §2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaStreamInfo {
    /// The stream pattern after `@media/<producer>/`.
    pub path: String,
    /// The declared wire encoding (`image/jpeg`, `video/*`).
    pub encoding: String,
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
                media: slice
                    .map(|s| {
                        s.media
                            .iter()
                            .map(|m| MediaStreamInfo {
                                path: m.path.clone(),
                                encoding: m.encoding.clone(),
                            })
                            .collect()
                    })
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

/// One origin claiming a human identity label, through the health-document
/// bridge (RFC 06 §6.2 bridge 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMatch {
    /// The origin id — the payload `host_id`, which IS the origin (§6.1).
    pub host_id: zenkey::origin::HostId,
    /// The display label the document carried (`source`).
    pub source: String,
    /// The key the claim arrived on — self-certifying, because the doc is
    /// origin-scoped and carries `host_id` beside `source`.
    pub key: String,
}

/// Resolve a human identity (hostname, `source` label) to the origin(s)
/// claiming it — the consumer identity bridge, run the sanctioned way
/// (RFC 06 §6.2): GET the fleet's `state/<producer>/health` documents and
/// read `host_id` beside `source`. Every match is returned; the *caller*
/// prices zero (the bridge yielded nothing — a probe MUST fail there,
/// RFC 09 §6) and more-than-one (a hostname collision is exactly the
/// misrouting hazard §6.2 names).
///
/// A document without both fields is skipped silently here — it is not a
/// claim about this label either way — but the total documents seen ride
/// back so the caller can tell "no claims" from "nobody answered".
pub async fn bridge_resolve(
    session: &zenoh::Session,
    base: &str,
    producer: &str,
    label: &str,
    timeout: std::time::Duration,
) -> Result<(Vec<BridgeMatch>, usize)> {
    let relative =
        zenkey::selector::producer_state(zenkey::selector::Scope::fleet(), producer, &["health"])
            .to_string();
    let key = zenkey::grammar::with_base(base, relative);
    let answers = crate::query::fleet_get(session, base, &key, None, timeout).await?;
    let mut matches = Vec::new();
    let seen = answers.len();
    for a in &answers {
        let crate::query::Answer::Value(bytes) = &a.answer else {
            continue;
        };
        let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&bytes.to_bytes()) else {
            continue;
        };
        let (Some(host_id), Some(source)) = (
            doc.get("host_id").and_then(|v| v.as_str()),
            doc.get("source").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if source == label
            && let Ok(id) = zenkey::origin::HostId::parse(host_id)
        {
            matches.push(BridgeMatch {
                host_id: id,
                source: source.to_string(),
                key: a.key.clone(),
            });
        }
    }
    matches.dedup_by(|a, b| a.host_id == b.host_id);
    Ok((matches, seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule that had two homes (#207): `roster()` read it off a GET reply
    /// and the CLI's watch loop re-derived it from a liveliness event.
    #[test]
    fn a_token_names_its_origin_and_producer() {
        assert_eq!(
            token_identity("acme", "acme/v1/h-3fa9c2d41b7e/state/sysinfo/alive"),
            Some(("h-3fa9c2d41b7e".into(), "sysinfo".into()))
        );
        // A service origin's token carries no producer chunk — the service is
        // the producer (RFC 06 §5).
        assert_eq!(
            token_identity("acme", "acme/v1/@catalog/state/alive"),
            Some(("@catalog".into(), "catalog".into()))
        );
        // The base-less deployment is a real one (RFC v1.6).
        assert_eq!(
            token_identity("", "v1/h-3fa9c2d41b7e/state/sysinfo/alive"),
            Some(("h-3fa9c2d41b7e".into(), "sysinfo".into()))
        );
        assert_eq!(token_identity("acme", "demo/not/a/token"), None);
    }

    /// Idempotent up, subtractive down — what makes seeding safe. The monitor
    /// replays history-backed tokens that the seeding GET may already have
    /// returned, and a double `NodeUp` must not double-list the producer or
    /// force a redundant frame.
    #[test]
    fn applying_a_token_reports_only_real_changes() {
        let mut roster: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let key = "acme/v1/h-3fa9c2d41b7e/state/sysinfo/alive";

        assert!(apply_token(&mut roster, "acme", key, true));
        assert!(
            !apply_token(&mut roster, "acme", key, true),
            "a replayed history token is not a change"
        );
        assert_eq!(roster["h-3fa9c2d41b7e"], ["sysinfo"]);

        // A second producer on the same origin sorts in.
        let other = "acme/v1/h-3fa9c2d41b7e/state/alerts/alive";
        assert!(apply_token(&mut roster, "acme", other, true));
        assert_eq!(roster["h-3fa9c2d41b7e"], ["alerts", "sysinfo"]);

        assert!(apply_token(&mut roster, "acme", key, false));
        assert!(
            !apply_token(&mut roster, "acme", key, false),
            "retracting what is already gone is not a change"
        );
        assert_eq!(roster["h-3fa9c2d41b7e"], ["alerts"]);

        // The last producer leaving takes the origin with it: an origin with
        // no producers is not a fact worth rendering.
        assert!(apply_token(&mut roster, "acme", other, false));
        assert!(roster.is_empty());

        // An unparseable key changes nothing and does not panic (O1).
        assert!(!apply_token(&mut roster, "acme", "demo/foreign", true));
    }

    /// The join is `None`-on-absence, never a default (O4), and an instance
    /// suffix shares its base slice (RFC 03 §1.5).
    #[test]
    fn rows_say_whether_a_slice_was_even_asked_for() {
        let mut roster: BTreeMap<String, Vec<String>> = BTreeMap::new();
        roster.insert(
            "h-3fa9c2d41b7e".into(),
            vec!["sysinfo".into(), "sysinfo-2".into()],
        );

        let unasked = node_rows(&roster, None);
        assert!(!unasked.slices_joined, "no join was attempted");
        assert!(unasked.nodes.iter().all(|n| n.app.is_none()));

        let slice = zenkey::parse_slice(
            "[registry]\nversion = \"1.0\"\napp = \"demo\"\nconvention = 1\n\
             [producer]\nname = \"sysinfo\"\n",
        )
        .expect("fixture slice parses");
        let joined = node_rows(&roster, Some(&crate::SliceSet::from_slices(vec![slice])));
        assert!(joined.slices_joined);
        assert_eq!(joined.nodes.len(), 2);
        for row in &joined.nodes {
            assert_eq!(
                row.app.as_deref(),
                Some("demo"),
                "an instance suffix shares the base producer's slice: {}",
                row.producer
            );
        }
        assert_eq!(
            joined.nodes[1].producer, "sysinfo-2",
            "the row keeps the suffix"
        );
    }
}
