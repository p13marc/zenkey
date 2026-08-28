//! The liveliness roster (RFC 04 §5).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::report::{Freshness, MediaStreamInfo, NodeInfo, ProducerInfo};
use crate::{Error, Result};
use zenkey::grammar::with_base;

/// The fleet-presence roster: who is up, and what they run.
///
/// RFC 04 §5 — a liveliness query on `<base>/v1/*/state/*/alive`. Zero
/// payload bytes: the token *key* is the record. `@catalog` is asked for by
/// name because `*` can never match a verbatim service origin (property D4).
pub async fn roster(
    fleet: &crate::Fleet<'_>,
    timeout: Duration,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let catalog_alive = zenkey::selector::service_alive(&zenkey::ServiceOrigin::catalog());

    // The builders are base-relative; this session is deliberately
    // un-namespaced, so it must spell the base itself.
    for expr in [
        fleet.wire(zenkey::selector::all_liveliness(
            zenkey::selector::Scope::fleet(),
        )),
        fleet.wire(catalog_alive),
    ] {
        let Ok(replies) = fleet
            .session()
            .liveliness()
            .get(&expr)
            .timeout(timeout)
            .await
        else {
            continue;
        };
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else { continue };
            let key = sample.key_expr().as_str();
            let Some((origin, producer)) = token_identity(fleet.base(), key) else {
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
    /// What [`next_change`](RosterWatch::next_change) has applied to `roster`
    /// but not yet reported — the accumulator, held here rather than in the
    /// poll's stack frame so a dropped poll cannot take it with it (#328).
    pending: RosterChange,
}

/// What one coalesced burst of liveliness events did to the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    pub async fn start(fleet: &crate::Fleet<'_>, timeout: Duration) -> Result<RosterWatch> {
        let liveliness = vec![
            fleet.wire(zenkey::selector::all_liveliness(
                zenkey::selector::Scope::fleet(),
            )),
            fleet.wire(zenkey::selector::service_alive(
                &zenkey::ServiceOrigin::catalog(),
            )),
        ];
        let monitor = crate::Monitor::start(
            fleet.session(),
            crate::MonitorSpec {
                selectors: vec![],
                liveliness,
                ..Default::default()
            },
        )
        .await?;
        let events = monitor.events();
        let roster = roster(fleet, timeout).await?;
        Ok(RosterWatch {
            monitor,
            events,
            roster,
            base: fleet.base().to_string(),
            pending: RosterChange::default(),
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
    ///
    /// **Cancel-safe** (#328): [`apply_token`] mutates `self.roster` the moment
    /// an event lands, and a burst is drained across an await — so a poll
    /// dropped in that await had already changed the roster. The accumulator
    /// therefore lives in `self.pending`, not on the stack: dropping this
    /// future loses the *wait*, never the change, and the next call reports it
    /// before it listens for anything further. A roster that moved while its
    /// caller was told nothing is a hole in the window with no later event
    /// bound to correct it — RFC 13 §3 O6, where an unreported gap converts
    /// "I missed it" into "it never happened".
    pub async fn next_change(&mut self) -> Option<RosterChange> {
        next_change_in(
            &mut self.events,
            &mut self.roster,
            &self.base,
            &mut self.pending,
        )
        .await
    }

    /// The same coalesced changes as a [`Stream`](futures_core::Stream)
    /// (#343).
    ///
    /// **Borrowing, deliberately**: [`stop`](Self::stop) is an acknowledged
    /// teardown that consumes `self` (#207/#336), and a stream that moved the
    /// watch in would leave a caller no way to reach it — a half-torn-down
    /// monitor is exactly what that teardown exists to prevent. Hold the
    /// watch, take the stream, drop the stream, then `stop`.
    ///
    /// Cancel-safety carries over unchanged, because the accumulator lives in
    /// `self.pending` rather than in a poll's stack frame (#328): a stream
    /// dropped mid-burst keeps the transitions it had already applied, and the
    /// next one reports them.
    pub fn changes(&mut self) -> impl futures_core::Stream<Item = RosterChange> + '_ {
        futures_util::stream::unfold(self, |watch| async move {
            watch.next_change().await.map(|change| (change, watch))
        })
    }

    /// Release the subscriptions, **acknowledged**.
    ///
    /// On every exit path, which the zenctl original managed only on Ctrl-C:
    /// its channel-closed arm returned before reaching `monitor.stop()`, so a
    /// closed stream leaked the liveliness subscribers (#207).
    ///
    /// And awaited, which it only looked like (#336): this was an `async fn`
    /// that awaited nothing, calling the `Drop` teardown and leaving the
    /// liveliness subscribers to undeclare in the background. It now goes
    /// through [`crate::Monitor::shutdown`], so the caller that waits for this
    /// gets what waiting was for.
    pub async fn stop(self) -> Result<()> {
        self.monitor.shutdown().await
    }
}

/// [`RosterWatch::next_change`]'s body over its four moving parts — the seam
/// that lets the cancellation contract be tested against a bare
/// [`crate::MonitorCore`], with no session and no bus.
async fn next_change_in(
    events: &mut crate::EventStream,
    roster: &mut BTreeMap<String, Vec<String>>,
    base: &str,
    pending: &mut RosterChange,
) -> Option<RosterChange> {
    loop {
        // First, whatever a previous — possibly cancelled — poll applied.
        if let Some(change) = take_change(pending) {
            return Some(change);
        }
        let mut item = events.recv().await;
        loop {
            match item {
                // Closed: report this burst's work, and say so on the next
                // call — the roster moved, and a closing stream is no reason
                // to drop the last thing it said.
                None => return take_change(pending),
                Some(crate::StreamItem::Dropped(_)) => {}
                Some(crate::StreamItem::Event(ev)) => {
                    let transition = match ev {
                        crate::FleetEvent::NodeUp(key) => Some((key, true)),
                        crate::FleetEvent::NodeDown(key) => Some((key, false)),
                        _ => None,
                    };
                    if let Some((key, up)) = transition
                        && apply_token(roster, base, &key, up)
                    {
                        if up {
                            pending.node_up = true;
                        } else {
                            pending.node_down = true;
                        }
                    }
                }
            }
            match tokio::time::timeout(BURST_QUIET, events.recv()).await {
                Ok(next) => item = next,
                Err(_) => break,
            }
        }
    }
}

/// Take the accumulated change, leaving nothing behind. `None` when the burst
/// moved nothing — a caller renders on every `Some` without checking.
fn take_change(pending: &mut RosterChange) -> Option<RosterChange> {
    if !pending.node_up && !pending.node_down {
        return None;
    }
    Some(std::mem::take(pending))
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
        .producer()
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
                .map_err(Error::from)
        } else {
            zenkey::origin::RemoteOrigin::parse(origin)
                .map(Node::Host)
                .map_err(|e| {
                    Error::unaskable(
                        "origin",
                        format!("{e} — a hostname is not an origin (RFC 06 §6)"),
                    )
                })
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
            Node::Host(o) => zenkey::selector::rpc(
                zenkey::selector::Scope::origin(o),
                zenkey::selector::Producers::all(),
                &["introspect"],
            )
            .to_string(),
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
    fleet: &crate::Fleet<'_>,
    origin: &str,
    timeout: Duration,
    with_freshness: bool,
) -> Result<NodeInfo> {
    let (session, base) = (fleet.session(), fleet.base());

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
                    .producer()
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
    let answers = crate::bus::query::fleet_get(
        fleet,
        &introspect,
        &crate::bus::query::GetOpts::new(timeout),
    )
    .await
    .unwrap_or_default();
    let served: Vec<zenkey::slice::RegistrySlice> = answers
        .into_iter()
        .filter(|a| a.origin == origin)
        .filter_map(|a| {
            let crate::bus::query::Answer::Value(bytes) = a.answer else {
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
                    .map(|s| s.blob.iter().map(|b| b.tier.token().to_string()).collect())
                    .unwrap_or_default(),
                media: slice
                    .map(|s| {
                        s.media
                            .iter()
                            .map(|m| MediaStreamInfo {
                                path: m.path.clone(),
                                encoding: m.encoding.as_encoding_str().to_string(),
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
        let samples = crate::bus::query::state_snapshot(session, &selector, timeout, None)
            .await
            .unwrap_or_default();
        let now = std::time::SystemTime::now();
        for slice in &mine {
            for subject in &slice.subjects {
                let Some(ttl) = subject.ttl_s else { continue };
                if !subject.class.is(&zenkey::Class::State) {
                    continue;
                }
                // Newest sample whose tail refines to this subject.
                let age = samples
                    .iter()
                    .filter_map(|s| {
                        let parsed = zenkey::grammar::parse_full(base, &s.key)?;
                        let p = parsed.producer()?.name().to_string();
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
    fleet: &crate::Fleet<'_>,
    producer: &str,
    label: &str,
    timeout: std::time::Duration,
) -> Result<(Vec<BridgeMatch>, usize)> {
    let relative =
        zenkey::selector::producer_state(zenkey::selector::Scope::fleet(), producer, &["health"])
            .to_string();
    let key = fleet.wire(relative);
    let answers =
        crate::bus::query::fleet_get(fleet, &key, &crate::bus::query::GetOpts::new(timeout))
            .await?;
    let mut matches = Vec::new();
    let seen = answers.len();
    for a in &answers {
        let crate::bus::query::Answer::Value(bytes) = &a.answer else {
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

    /// The cancellation contract (#328): a poll dropped mid-burst has already
    /// mutated the roster, so the change it accumulated must survive the drop.
    /// It used to live in the poll's stack frame and die with it — the roster
    /// moved, the caller was told nothing, and the display stayed stale until
    /// some unrelated later token happened to arrive.
    ///
    /// Time is paused, so the two windows below are exact rather than raced:
    /// the token is ready immediately, and the poll is then dropped inside
    /// [`BURST_QUIET`] while it waits for the rest of the burst.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_poll_keeps_the_change_it_already_applied() {
        let core = crate::MonitorCore::new(16);
        let mut events = core.events();
        let mut roster: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut pending = RosterChange::default();

        core.node_event("v1/h-3fa9c2d41b7e/state/sysinfo/alive".into(), true);

        let cancelled = tokio::time::timeout(
            BURST_QUIET / 2,
            next_change_in(&mut events, &mut roster, "", &mut pending),
        )
        .await;
        assert!(cancelled.is_err(), "the poll is still draining the burst");
        assert!(
            roster.contains_key("h-3fa9c2d41b7e"),
            "the token was applied before the drop"
        );

        // …and the next call reports it, without waiting on the bus for a
        // second event that may never come.
        let change = tokio::time::timeout(
            BURST_QUIET / 2,
            next_change_in(&mut events, &mut roster, "", &mut pending),
        )
        .await
        .expect("the applied change is reported, not waited on")
        .expect("a change, not a closed stream");
        assert_eq!(
            change,
            RosterChange {
                node_up: true,
                node_down: false
            }
        );
    }
}
