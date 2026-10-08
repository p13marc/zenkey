//! Last-known state (spec §4.4, S5): a minimal `archive.v1`, and the
//! consumer's explicit read of it.
//!
//! - **It records** owners' mutations under its own address, each with its
//!   stamp and type identity (§7.1), and keeps tombstones for its window.
//! - **Its key** is the archived key without its leading `zk2`, each chunk
//!   slugged as a rest parameter, under its own `@state` token.
//! - **It answers GETs only,** with a JSON attachment
//!   `{"iface", "contract", "type", "confirmed"}`.
//! - **It refuses a put older than what it holds**, a delete included: the
//!   storage manager of zenoh 1.10.1 does not, and resurrects keys.
//! - **Alignment:** when an owner becomes reachable, it re-reads the owner's
//!   recorded collection; while the owner is absent, it re-reads an archive
//!   on the owner's side. It drops a key only on a `reply_del`; a key the
//!   source neither reports nor tombstones is kept, `confirmed: false`.
//!
//! `archive.v1` is a profile (#613); this is what the core requires of it,
//! and the contract below is its minimal form until the profile is written.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zenkey_model::bundle::Bundle;
use zenkey_model::contract::{Contract, load_str};
use zenkey_model::decode::type_of;
use zenkey_model::grammar::{Addr, IfaceId, ZkKey, parse};
use zenkey_model::slug::{chunk_slug, chunk_unslug};
use zenkey_model::template::Bindings;
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

use crate::config::ServiceConfig;
use crate::error::{Error, Result, zenoh};
use crate::implementation::Implementation;
use crate::service::{Service, ServiceBuilder};
use crate::state::DEFAULT_WINDOW;

/// `archive.v1`'s minimal contract. Its population is the sum of what it
/// records, which no contract can fix in advance (§4.4); the cardinality
/// below is a placeholder ceiling until `archive.v1` is specified (#613).
pub const CONTRACT: &str = r#"[interface]
name = "archive"
major = 1
minor = 0
summary = "Last-known state, recorded from owners (spec §4.4); minimal until #613"

[resources."{origin...}"]
kind = "state"
explicit = true
type = { raw = "application/octet-stream" }
params = { origin = "path" }
cardinality = 4294967295
"#;

/// `archive.v1`'s contract.
#[must_use]
pub fn contract() -> Contract {
    let l = load_str(CONTRACT, Path::new("."), None);
    l.contract.expect("archive.v1's contract loads")
}

fn iface() -> IfaceId {
    "archive.v1".parse().expect("an interface id")
}

/// What an archive records: one owner's state under `selector`, typed by
/// the owner's contract.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub owner: Addr,
    /// A key expression over the owner's state keys, such as
    /// `zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*`.
    pub selector: String,
    pub implementation: Implementation,
}

/// An archive's configuration.
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    /// The archive's own address and deployment settings; its tombstone
    /// window is at least the owners' (S3).
    pub service: ServiceConfig,
    pub records: Vec<Recorded>,
    /// Archives on the owners' side, read while an owner is absent.
    pub peers: Vec<Addr>,
    /// An unconfirmed key may be dropped after this long; `None` keeps it.
    pub unconfirmed_horizon: Option<Duration>,
}

#[derive(Debug, Clone)]
struct Rec {
    value: Option<(Vec<u8>, Encoding)>,
    ts: Timestamp,
    confirmed: bool,
    identity: Value,
    at: Instant,
}

struct Source {
    rec: Recorded,
    bundle: Bundle,
    selector: OwnedKeyExpr,
}

struct Inner {
    session: zenoh::Session,
    me: Addr,
    sources: Vec<Source>,
    peers: Vec<Addr>,
    store: Mutex<HashMap<String, Rec>>,
    window: Duration,
    horizon: Option<Duration>,
    refuse: AtomicU32,
}

/// Where an alignment read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignedFrom {
    Owner(Addr),
    Peer(Addr),
    /// Neither answered: nothing confirmed, nothing dropped.
    Nothing,
}

/// What one alignment did, per recorded selector.
#[derive(Debug, Clone)]
pub struct Alignment {
    pub from: Vec<AlignedFrom>,
    pub confirmed: usize,
    pub dropped: usize,
    pub unconfirmed: usize,
}

/// A running archive.
pub struct Archive {
    inner: Arc<Inner>,
    service: Service,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
    _watch: Vec<zenoh::pubsub::Subscriber<()>>,
    _q: zenoh::query::Queryable<()>,
}

/// An archive key for `origin` (§4.4): `origin` without `zk2`, each chunk
/// slugged, under `<archive>`'s `@state`. Wildcard chunks stay wildcards.
#[must_use]
pub fn archive_key(archive: &Addr, origin: &str) -> String {
    let tail: Vec<String> = origin
        .split('/')
        .skip(1)
        .map(|c| {
            if c == "*" || c == "**" {
                c.to_owned()
            } else {
                chunk_slug(c)
            }
        })
        .collect();
    format!(
        "zk2/{}/{}/archive.v1/@state/{}",
        archive.system,
        archive.service,
        tail.join("/")
    )
}

fn origin_of(archive_key: &str) -> Option<String> {
    let (_, tail) = archive_key.split_once("/archive.v1/@state/")?;
    let chunks: Option<Vec<String>> = tail.split('/').map(chunk_unslug).collect();
    Some(format!("zk2/{}", chunks?.join("/")))
}

impl Inner {
    fn identity(&self, origin: &str) -> Option<Value> {
        let ZkKey::Data { kind, resource, .. } = parse(origin).ok()? else {
            return None;
        };
        let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
        self.sources.iter().find_map(|s| {
            let c = s.rec.implementation.contract();
            let r = c.resources.iter().find(|r| r.token == kind && r.template.matches(&refs).is_some())?;
            let ty = type_of(&s.bundle, &kind.to_string(), r.template.as_str(), "type")?;
            Some(json!({"iface": c.iface.to_string(), "contract": s.rec.implementation.fingerprint().to_string(), "type": ty}))
        })
    }

    /// Records a mutation if it is newer than what is held: an older put,
    /// even one arriving after a delete, is refused (§4.4).
    fn record(
        &self,
        origin: &str,
        value: Option<(Vec<u8>, Encoding)>,
        ts: Timestamp,
        confirmed: bool,
    ) -> bool {
        let Some(identity) = self.identity(origin) else {
            return false;
        };
        let mut store = self.store.lock().expect("not poisoned");
        if let Some(held) = store.get_mut(origin)
            && ts <= held.ts
        {
            if ts == held.ts {
                held.confirmed |= confirmed;
            }
            return false;
        }
        store.insert(
            origin.to_owned(),
            Rec {
                value,
                ts,
                confirmed,
                identity,
                at: Instant::now(),
            },
        );
        true
    }

    fn on_sample(&self, s: &Sample) {
        let Some(ts) = s.timestamp().copied() else {
            tracing::warn!(key = %s.key_expr(), "an unstamped state mutation is not recorded (S1)");
            return;
        };
        let value = match s.kind() {
            SampleKind::Put => Some((s.payload().to_bytes().into_owned(), s.encoding().clone())),
            SampleKind::Delete => None,
        };
        self.record(s.key_expr().as_str(), value, ts, true);
    }

    fn answer(&self, q: &zenoh::query::Query) {
        let mut store = self.store.lock().expect("not poisoned");
        store.retain(|_, r| r.value.is_some() || r.at.elapsed() <= self.window);
        for (origin, r) in store.iter() {
            let Ok(ke) = OwnedKeyExpr::try_from(archive_key(&self.me, origin)) else {
                continue;
            };
            if !q.key_expr().intersects(&ke) {
                continue;
            }
            let _ = match &r.value {
                Some((bytes, enc)) => {
                    let mut att = r.identity.clone();
                    att["confirmed"] = Value::Bool(r.confirmed);
                    q.reply(ke, bytes.clone())
                        .encoding(enc.clone())
                        .timestamp(r.ts)
                        .attachment(serde_json::to_vec(&att).expect("JSON"))
                        .wait()
                }
                None => q.reply_del(ke).timestamp(r.ts).wait(),
            };
        }
    }

    async fn read(&self, selector: &str, timeout: Duration) -> Result<Vec<Sample>> {
        if self
            .refuse
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Ok(Vec::new());
        }
        let rx = self
            .session
            .get(selector)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(timeout)
            .with(flume::unbounded::<Reply>())
            .await
            .map_err(zenoh)?;
        let mut out = Vec::new();
        while let Ok(r) = rx.recv_async().await {
            if let Ok(s) = r.into_result() {
                out.push(s);
            }
        }
        Ok(out)
    }

    async fn align(&self, timeout: Duration) -> Result<Alignment> {
        let mut a = Alignment {
            from: Vec::new(),
            confirmed: 0,
            dropped: 0,
            unconfirmed: 0,
        };
        for src in &self.sources {
            let owner = &src.rec.owner;
            let present = !crate::presence::liveliness_keys(
                &self.session,
                &format!("zk2/{}/{}/@zk/instance/*", owner.system, owner.service),
                timeout,
            )
            .await?
            .is_empty();
            let (from, replies) = if present {
                (
                    AlignedFrom::Owner(owner.clone()),
                    self.read(&src.rec.selector, timeout).await?,
                )
            } else {
                let mut found = (AlignedFrom::Nothing, Vec::new());
                for peer in &self.peers {
                    let got = self
                        .read(&archive_key(peer, &src.rec.selector), timeout)
                        .await?;
                    if !got.is_empty() {
                        found = (AlignedFrom::Peer(peer.clone()), got);
                        break;
                    }
                }
                found
            };
            let mut seen = Vec::new();
            for s in &replies {
                let key = s.key_expr().as_str();
                let (origin, confirmed) = match from {
                    AlignedFrom::Peer(_) => {
                        let Some(o) = origin_of(key) else { continue };
                        let c = s
                            .attachment()
                            .and_then(|a| serde_json::from_slice::<Value>(&a.to_bytes()).ok())
                            .and_then(|v| v["confirmed"].as_bool())
                            .unwrap_or(false);
                        (o, c || s.kind() == SampleKind::Delete)
                    }
                    _ => (key.to_owned(), true),
                };
                let Some(ts) = s.timestamp().copied() else {
                    continue;
                };
                let value = match s.kind() {
                    SampleKind::Put => {
                        Some((s.payload().to_bytes().into_owned(), s.encoding().clone()))
                    }
                    SampleKind::Delete => {
                        a.dropped += 1;
                        None
                    }
                };
                self.record(&origin, value, ts, confirmed);
                if confirmed {
                    a.confirmed += 1;
                }
                seen.push(origin);
            }
            // Keys the source neither reported nor tombstoned: kept,
            // unconfirmed (no positive evidence).
            let mut store = self.store.lock().expect("not poisoned");
            for (origin, r) in store.iter_mut() {
                let matches = OwnedKeyExpr::try_from(origin.clone())
                    .is_ok_and(|k| src.selector.intersects(&k));
                if matches && !seen.contains(origin) && r.value.is_some() {
                    r.confirmed = false;
                    a.unconfirmed += 1;
                }
            }
            if let Some(h) = self.horizon {
                store.retain(|_, r| r.confirmed || r.value.is_none() || r.at.elapsed() < h);
            }
            a.from.push(from);
        }
        Ok(a)
    }
}

impl Inner {
    /// Aligns, retrying a few times while keys stay unconfirmed: a token can
    /// arrive before the route to its owner's queryable does, and an empty
    /// read confirms nothing (§4.4). Bounded: an unreachable source leaves
    /// the keys unconfirmed, which is what they are.
    async fn align_settled(&self) {
        for attempt in 0..5u32 {
            match self.align(Duration::from_secs(1)).await {
                Ok(a) if a.unconfirmed == 0 => return,
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "alignment failed"),
            }
            tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt + 1))).await;
        }
    }
}

impl Archive {
    /// Starts an archive: records its sources, answers on its keys, and
    /// aligns whenever an owner's instance token appears (and once now).
    pub async fn start(session: &zenoh::Session, config: ArchiveConfig) -> Result<Self> {
        let window = config
            .service
            .tombstone_window_s
            .map_or(DEFAULT_WINDOW, Duration::from_secs);
        let me = config.service.address.clone();
        let sources = config
            .records
            .iter()
            .map(|r| {
                let bundle = Bundle::verify(r.implementation.bundle_bytes()).map_err(|e| {
                    Error::Contract(format!("an origin's bundle does not verify: {e}"))
                })?;
                let selector = OwnedKeyExpr::try_from(r.selector.clone()).map_err(zenoh)?;
                Ok(Source {
                    rec: r.clone(),
                    bundle,
                    selector,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let inner = Arc::new(Inner {
            session: session.clone(),
            me,
            sources,
            peers: config.peers.clone(),
            store: Mutex::new(HashMap::new()),
            window,
            horizon: config.unconfirmed_horizon,
            refuse: AtomicU32::new(0),
        });
        let mut subs = Vec::new();
        for src in &inner.sources {
            let i = Arc::clone(&inner);
            subs.push(
                session
                    .declare_subscriber(src.selector.clone())
                    .callback(move |s| i.on_sample(&s))
                    .await
                    .map_err(zenoh)?,
            );
        }
        let mut b = ServiceBuilder::new(session, config.service);
        b.implement(Implementation::new(contract()))?;
        let i = Arc::clone(&inner);
        let q = b
            .declare_queryable(
                &iface(),
                "@state/{origin...}",
                None::<&Bindings>,
                move |q| i.answer(&q),
            )
            .await?;
        let service = b.start().await?;
        // Align whenever an owner, or an archive on an owner's side, becomes
        // reachable: with the owner gone, a peer's arrival is the only
        // signal that the link healed.
        let mut watch = Vec::new();
        let watched: Vec<Addr> = inner
            .sources
            .iter()
            .map(|s| s.rec.owner.clone())
            .chain(inner.peers.iter().cloned())
            .collect();
        for o in &watched {
            let i = Arc::clone(&inner);
            let handle = tokio::runtime::Handle::current();
            watch.push(
                session
                    .liveliness()
                    .declare_subscriber(format!("zk2/{}/{}/@zk/instance/*", o.system, o.service))
                    .callback(move |s: Sample| {
                        if s.kind() == SampleKind::Put {
                            let i = Arc::clone(&i);
                            handle.spawn(async move { i.align_settled().await });
                        }
                    })
                    .await
                    .map_err(zenoh)?,
            );
        }
        Ok(Self {
            inner,
            service,
            _subs: subs,
            _watch: watch,
            _q: q,
        })
    }

    /// The archive's service (its address, descriptor, tokens).
    #[must_use]
    pub fn service(&self) -> &Service {
        &self.service
    }

    /// This archive's key for an origin key.
    #[must_use]
    pub fn key(&self, origin: &str) -> String {
        archive_key(&self.inner.me, origin)
    }

    /// Aligns now (§4.4), each GET waiting at most `timeout`.
    pub async fn align(&self, timeout: Duration) -> Result<Alignment> {
        self.inner.align(timeout).await
    }

    /// Whether the archive holds `origin` as confirmed, if it holds it.
    #[must_use]
    pub fn confirmed(&self, origin: &str) -> Option<bool> {
        self.inner
            .store
            .lock()
            .expect("not poisoned")
            .get(origin)
            .map(|r| r.confirmed)
    }

    /// Makes the next `n` reads of an alignment return an empty reply set,
    /// as an access-control refusal does (automatic alignment retries, so a
    /// refusal that should outlast it needs `n` above one). For tests of the positive-evidence
    /// rule (spec scenarios state.md §5.2); a deployment never calls it.
    pub fn refuse_next_reads(&self, n: u32) {
        self.inner.refuse.store(n, Ordering::Relaxed);
    }
}

/// A key's last-known state, read from an archive (S5). Never current
/// (S6): a consumer presents it as last-known.
#[derive(Debug, Clone)]
pub struct LastKnown {
    pub origin: String,
    /// The value, or `None` for a tombstone.
    pub value: Option<Vec<u8>>,
    pub encoding: Option<Encoding>,
    pub timestamp: Option<Timestamp>,
    /// `{"iface", "contract", "type"}`: the value's type identity (§7.1).
    pub identity: Value,
    /// `false` while alignment has not confirmed the key.
    pub confirmed: bool,
}

/// Reads `origin`'s last-known state from the archive at `archive`,
/// explicitly (S5). `Ok(None)` when it answered nothing.
pub async fn last_known(
    session: &zenoh::Session,
    archive: &Addr,
    origin: &str,
    timeout: Duration,
) -> Result<Option<LastKnown>> {
    let rx = session
        .get(archive_key(archive, origin))
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
        .map_err(zenoh)?;
    while let Ok(r) = rx.recv_async().await {
        let Ok(s) = r.into_result() else { continue };
        let att: Value = s
            .attachment()
            .and_then(|a| serde_json::from_slice(&a.to_bytes()).ok())
            .unwrap_or(Value::Null);
        let put = s.kind() == SampleKind::Put;
        return Ok(Some(LastKnown {
            origin: origin.to_owned(),
            value: put.then(|| s.payload().to_bytes().into_owned()),
            encoding: put.then(|| s.encoding().clone()),
            timestamp: s.timestamp().copied(),
            confirmed: att["confirmed"].as_bool().unwrap_or(false),
            identity: json!({"iface": att["iface"], "contract": att["contract"], "type": att["type"]}),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{archive_key, contract, origin_of};

    #[test]
    fn the_archive_key_slugs_the_origin_as_a_rest_parameter() {
        let a = "vehicle-01/archive".parse().unwrap();
        let k = archive_key(
            &a,
            "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01",
        );
        assert_eq!(
            k,
            "zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"
        );
        let x = archive_key(&a, "zk2/g/s/i.v1/@state/x");
        assert_eq!(
            x,
            "zk2/vehicle-01/archive/archive.v1/@state/g/s/i.v1/x-_x40state/x"
        );
        assert!(zenkey_model::grammar::parse(&k).is_ok());
        assert_eq!(origin_of(&x).as_deref(), Some("zk2/g/s/i.v1/@state/x"));
        assert_eq!(contract().iface.to_string(), "archive.v1");
    }
}
