//! Consuming through a role (spec §3.2): data reaches a component from the
//! providers its configuration binds the role to, never from a provider
//! named in code.
//!
//! - **R1:** a binding is a list of service addresses, exact
//!   (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, `*/tc`). It resolves
//!   at once: a subscription needs no presence. A service's
//!   `self.system/<service>` is spelled out when it starts (0.20); a tool,
//!   with no system of its own, is refused one.
//! - **R2:** a template parameter may be bound, to `self.system`,
//!   `self.service` or a value. Unbound parameters select every member.
//! - **R5:** a consumer MAY wait on presence for its providers.
//! - **R6:** a sample whose key expression is not concrete is discarded and
//!   counted: zenoh delivers a put on a wildcard key with the publisher's key.
//! - **R7:** where presence cannot be observed (a constrained face),
//!   liveness is judged from the freshness of what crosses, and silence is
//!   *unobservable*, never *down*.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Contract, Resource};
use zenkey_model::freshness::{
    ClockMeasure, ClockTrust, Horizon, Judged, Last, Observation, StampAge,
};
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey, parse};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Subscriber;
use zenoh::sample::Sample;

use crate::error::{Error, Result, zenoh};
use crate::state::{Current, StateGet};

/// One bound provider address: a position is `None` for `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    system: Option<String>,
    service: Option<String>,
}

impl Provider {
    /// Parses `<system>/<service>`, either position `*` (R1).
    pub fn parse(s: &str) -> Result<Self> {
        let bad = || {
            Error::Contract(format!(
                "binding {s:?} is not <system>/<service> (each a name or *)"
            ))
        };
        let (sys, svc) = s.split_once('/').ok_or_else(bad)?;
        if sys == crate::config::SELF_SYSTEM {
            // R1 (0.20): a service's runtime spells it out at start, so only
            // a tool, which has no system of its own, gets here.
            return Err(Error::Contract(format!(
                "binding {s:?} names the service's own system, which a tool has none of (R1)"
            )));
        }
        let pos = |p: &str| -> Result<Option<String>> {
            if p == "*" {
                Ok(None)
            } else if zenkey_model::chunk::is_plain_chunk(p) {
                Ok(Some(p.to_owned()))
            } else {
                Err(bad())
            }
        };
        Ok(Self {
            system: pos(sys)?,
            service: pos(svc)?,
        })
    }

    /// Whether `addr` is one of the services this binding names.
    #[must_use]
    pub fn matches(&self, addr: &Addr) -> bool {
        self.system
            .as_deref()
            .is_none_or(|s| s == addr.system.as_str())
            && self
                .service
                .as_deref()
                .is_none_or(|s| s == addr.service.as_str())
    }

    pub(crate) fn chunks(&self) -> (&str, &str) {
        (
            self.system.as_deref().unwrap_or("*"),
            self.service.as_deref().unwrap_or("*"),
        )
    }
}

/// Whether presence can be observed for a role's providers (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Tokens reach this consumer.
    Observable,
    /// They do not: the providers sit across a constrained face whose `@zk`
    /// traffic is denied (§8.5).
    Unavailable,
}

/// A provider's liveness as a consumer can judge it (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Presence is observable, and the provider holds its interface token.
    Present,
    /// Presence is observable, and the provider holds none.
    Absent,
    /// Presence is unavailable, and data arrived within the window.
    Fresh,
    /// Presence is unavailable, and nothing arrived within the window. Not
    /// *down*: nothing can tell.
    Unobservable,
}

/// One delivered sample, attributed (R3's edge, observed).
#[derive(Debug, Clone)]
pub struct Delivery {
    /// The provider whose key carried it.
    pub provider: Addr,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The template's parameters, unslugged.
    pub values: Bindings,
    pub sample: Sample,
}

/// A role's consumer, compiled against the required interface's contract
/// (R4: any revision of the major is fine).
#[derive(Clone)]
pub struct Consumer {
    session: zenoh::Session,
    role: String,
    contract: Arc<Contract>,
    providers: Vec<Provider>,
    params: BTreeMap<String, String>,
    presence: Presence,
}

/// A live subscription through a role. Dropping it undeclares it.
pub struct Subscription {
    _subs: Vec<Subscriber<()>>,
    seen: Arc<Seen>,
    /// When it was declared: how long it has listened (`freshness.v1` §2.5).
    since: Instant,
}

#[derive(Default)]
struct Seen {
    discarded: AtomicU64,
    unresolved: AtomicU64,
    last: Mutex<BTreeMap<Addr, Instant>>,
    /// Each member's last delivery, on this host's monotonic clock, and
    /// whether it was a delete (`freshness.v1` §2.5).
    members: Mutex<BTreeMap<String, (Instant, bool)>>,
    /// Per stamping clock, the delivery whose stamp came closest to this
    /// host's clock at receipt: what `freshness.v1` §2.6 measures a GET
    /// reader's clock from.
    clocks: Mutex<BTreeMap<String, ClockMeasure>>,
}

impl Consumer {
    pub(crate) fn new(
        session: &zenoh::Session,
        me: Option<&Addr>,
        role: &str,
        contract: Arc<Contract>,
        providers: &[String],
        params: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let providers = providers
            .iter()
            .map(|p| Provider::parse(p))
            .collect::<Result<Vec<_>>>()?;
        let params = params
            .iter()
            .map(|(k, v)| {
                let v = match (v.as_str(), me) {
                    ("self.system", Some(me)) => me.system.to_string(),
                    ("self.service", Some(me)) => me.service.to_string(),
                    ("self.system" | "self.service", None) => {
                        return Err(Error::Contract(format!(
                            "parameter {k:?} = {v:?} needs a service of its own (R2); a tool has none"
                        )));
                    }
                    (other, _) => other.to_owned(),
                };
                Ok((k.clone(), v))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            session: session.clone(),
            role: role.to_owned(),
            contract,
            providers,
            params,
            presence: Presence::Observable,
        })
    }

    /// A consumer for a tool, which has no service of its own: `contract`
    /// is typically built from a retrieved bundle
    /// ([`crate::Implementation::from_bundle`]), `providers` are service
    /// addresses, exact or wildcard (R1), and `params` binds template
    /// parameters to values (R2; `self.*` needs a service and is refused).
    /// It appears in no graph: a component that should is a service.
    pub fn for_tool(
        session: &zenoh::Session,
        contract: Arc<Contract>,
        providers: &[&str],
        params: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let providers: Vec<String> = providers.iter().map(|p| (*p).to_owned()).collect();
        Self::new(session, None, "tool", contract, &providers, params)
    }

    /// Marks presence unobservable for this role's providers (R7): set by a
    /// deployment whose providers sit across a constrained face.
    #[must_use]
    pub fn with_presence(mut self, presence: Presence) -> Self {
        self.presence = presence;
        self
    }

    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    #[must_use]
    pub fn interface(&self) -> &IfaceId {
        &self.contract.iface
    }

    /// The bound providers (R1).
    #[must_use]
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    /// The session it reads through: zenoh stays reachable.
    #[must_use]
    pub fn session(&self) -> &zenoh::Session {
        &self.session
    }

    /// The role's parameter bindings (R2), resolved.
    #[must_use]
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    pub(crate) fn resource(&self, name: &str) -> Result<&Resource> {
        name.split_once('/')
            .and_then(|(t, tpl)| self.contract.resource(t.parse().ok()?, tpl))
            .ok_or_else(|| Error::NoResource {
                iface: self.contract.iface.clone(),
                resource: name.to_owned(),
            })
    }

    /// The key expressions a subscription or GET on `resource` uses: one per
    /// bound provider, with R2's parameter bindings applied and every other
    /// parameter a wildcard. An event's selector ends in `*`, the ULID.
    pub fn selectors(&self, resource: &str) -> Result<Vec<OwnedKeyExpr>> {
        let r = self.resource(resource)?;
        let iface = self.contract.iface.to_string();
        let mut tail: Vec<String> = r
            .template
            .segments()
            .iter()
            .map(|seg| match seg {
                Segment::Literal(l) => l.clone(),
                Segment::Param(n) => self
                    .params
                    .get(n)
                    .map_or_else(|| "*".to_owned(), |v| chunk_slug(v)),
                Segment::Rest(n) => self
                    .params
                    .get(n)
                    .map_or_else(|| "**".to_owned(), |v| chunk_slug(v)),
            })
            .collect();
        if r.kind == Kind::Event {
            tail.push("*".to_owned());
        }
        self.providers
            .iter()
            .map(|p| {
                let (sys, svc) = p.chunks();
                let ke = format!("zk2/{sys}/{svc}/{iface}/{}/{}", r.token, tail.join("/"));
                OwnedKeyExpr::try_from(ke).map_err(zenoh)
            })
            .collect()
    }

    /// Subscribes to `resource` across the bound providers. It resolves at
    /// once, without presence (R1, R5). Samples on a non-concrete key are
    /// discarded and counted (R6). A sample on a concrete key that does not
    /// resolve to a member of the resource through a bound provider is
    /// dropped and counted apart ([`Subscription::unresolved`]); every other
    /// one is delivered attributed.
    pub async fn subscribe<C>(&self, resource: &str, callback: C) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
    {
        let r = self.resource(resource)?.clone();
        let name = format!("{}/{}", r.token, r.template);
        let seen = Arc::new(Seen::default());
        let since = Instant::now();
        let callback = Arc::new(callback);
        let mut subs = Vec::new();
        for ke in self.selectors(resource)? {
            let (seen, callback, r, name) = (
                Arc::clone(&seen),
                Arc::clone(&callback),
                r.clone(),
                name.clone(),
            );
            let iface = self.contract.iface.clone();
            let providers = self.providers.clone();
            subs.push(
                self.session
                    .declare_subscriber(ke)
                    .callback(move |sample: Sample| {
                        let key = sample.key_expr().as_str();
                        if key.contains('*') {
                            seen.discarded.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        let Ok(ZkKey::Data {
                            addr,
                            iface: got,
                            resource: mut chunks,
                            ..
                        }) = parse(key)
                        else {
                            seen.unresolved.fetch_add(1, Ordering::Relaxed);
                            return;
                        };
                        if got != iface || !providers.iter().any(|p| p.matches(&addr)) {
                            seen.unresolved.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        if r.kind == Kind::Event {
                            chunks.pop();
                        }
                        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                        let Some(values) = r.template.matches(&refs) else {
                            seen.unresolved.fetch_add(1, Ordering::Relaxed);
                            return;
                        };
                        let now = Instant::now();
                        seen.last
                            .lock()
                            .expect("not poisoned")
                            .insert(addr.clone(), now);
                        seen.members.lock().expect("not poisoned").insert(
                            key.to_owned(),
                            (now, sample.kind() == zenoh::sample::SampleKind::Delete),
                        );
                        if let Some(t) = sample.timestamp() {
                            let offset =
                                StampAge::between(t.get_time().to_system_time(), SystemTime::now());
                            seen.clocks
                                .lock()
                                .expect("not poisoned")
                                .entry(t.get_id().to_string())
                                .and_modify(|m| m.record(offset))
                                .or_insert(ClockMeasure::new(offset));
                        }
                        callback(Delivery {
                            provider: addr,
                            resource: name.clone(),
                            values,
                            sample,
                        });
                    })
                    .await
                    .map_err(zenoh)?,
            );
        }
        Ok(Subscription {
            _subs: subs,
            seen,
            since,
        })
    }

    /// Replays an event's occurrences within `retention` (§2.6): a GET on
    /// every bound provider's selector, bounded by `_time`, then filtered by
    /// each occurrence's ULID time on this side, because a storage backend
    /// may ignore `_time` (spike S5: the memory backend does).
    pub async fn replay(
        &self,
        resource: &str,
        retention: Duration,
        timeout: Duration,
    ) -> Result<Vec<Delivery>> {
        let r = self.resource(resource)?.clone();
        if r.kind != Kind::Event {
            return Err(Error::Contract(format!("{resource:?} is not an event")));
        }
        let name = format!("{}/{}", r.token, r.template);
        let since = std::time::SystemTime::now() - retention;
        let mut out = Vec::new();
        for ke in self.selectors(resource)? {
            let rx = self
                .session
                .get(format!("{ke}?_time=[now(-{}s)..]", retention.as_secs()))
                .target(zenoh::query::QueryTarget::All)
                .consolidation(zenoh::query::ConsolidationMode::None)
                .timeout(timeout)
                .with(flume::unbounded::<zenoh::query::Reply>())
                .await
                .map_err(zenoh)?;
            while let Ok(reply) = rx.recv_async().await {
                let Ok(sample) = reply.into_result() else {
                    continue;
                };
                let key = sample.key_expr().as_str();
                if key.contains('*') {
                    continue;
                }
                let Ok(ZkKey::Data {
                    addr,
                    resource: mut chunks,
                    ..
                }) = parse(key)
                else {
                    continue;
                };
                let Some(at) = chunks.pop().as_deref().and_then(crate::writer::ulid_time) else {
                    continue;
                };
                if at < since || !self.providers.iter().any(|p| p.matches(&addr)) {
                    continue;
                }
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                if let Some(values) = r.template.matches(&refs) {
                    out.push(Delivery {
                        provider: addr,
                        resource: name.clone(),
                        values,
                        sample,
                    });
                }
            }
        }
        Ok(out)
    }

    /// GETs current state from its owners (S4): every bound provider's keys
    /// for `resource`, one member when `values` binds its parameters (R2's
    /// bindings apply either way), with target `All` and consolidation
    /// `Latest` set explicitly. Silence is [`StateGet::Silent`], never a
    /// verdict (S6, O5).
    pub async fn get(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        timeout: Duration,
    ) -> Result<StateGet> {
        let r = self.resource(resource)?.clone();
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!("{resource:?} is not state")));
        }
        let selectors = match values {
            None => self.selectors(resource)?,
            Some(v) => {
                let mut merged: Bindings = self
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), vec![v.clone()]))
                    .collect();
                merged.extend(v.clone());
                let chunks = r
                    .template
                    .build(&merged)
                    .map_err(|e| Error::Contract(format!("{resource:?}: {e}")))?;
                self.providers
                    .iter()
                    .map(|p| {
                        let (sys, svc) = p.chunks();
                        let ke = format!(
                            "zk2/{sys}/{svc}/{}/{}/{}",
                            self.contract.iface,
                            r.token,
                            chunks.join("/")
                        );
                        OwnedKeyExpr::try_from(ke).map_err(zenoh)
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        let mut out = Vec::new();
        for ke in selectors {
            let rx = self
                .session
                .get(ke)
                .target(zenoh::query::QueryTarget::All)
                .consolidation(zenoh::query::ConsolidationMode::Latest)
                .timeout(timeout)
                .with(flume::unbounded::<zenoh::query::Reply>())
                .await
                .map_err(zenoh)?;
            while let Ok(reply) = rx.recv_async().await {
                let Ok(sample) = reply.into_result() else {
                    continue;
                };
                let key = sample.key_expr().as_str().to_owned();
                if key.contains('*') {
                    continue;
                }
                out.push(match sample.kind() {
                    zenoh::sample::SampleKind::Delete => Current::Deleted {
                        key,
                        timestamp: sample.timestamp().copied(),
                    },
                    zenoh::sample::SampleKind::Put => Current::Value {
                        key,
                        sample: Box::new(sample),
                    },
                });
            }
        }
        Ok(if out.is_empty() {
            StateGet::Silent
        } else {
            StateGet::Answered(out)
        })
    }

    /// The bound providers holding this interface's token now (§8.1).
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        let mut out = Vec::new();
        for p in &self.providers {
            let (sys, svc) = p.chunks();
            let sel = format!("zk2/{sys}/{svc}/@zk/alive/{}/**", self.contract.iface);
            for t in crate::presence::tokens(&self.session, &sel, timeout).await? {
                if let ZkKey::Alive { addr, .. } = t
                    && !out.contains(&addr)
                {
                    out.push(addr);
                }
            }
        }
        Ok(out)
    }

    /// Waits until a bound provider holds this interface's token, or
    /// `timeout` passes (R5). Returns the first one seen.
    pub async fn wait_for_provider(&self, timeout: Duration) -> Result<Option<Addr>> {
        let (tx, rx) = flume::unbounded::<Addr>();
        let mut subs = Vec::new();
        for p in &self.providers {
            let (sys, svc) = p.chunks();
            let sel = format!("zk2/{sys}/{svc}/@zk/alive/{}/**", self.contract.iface);
            let tx = tx.clone();
            subs.push(
                self.session
                    .liveliness()
                    .declare_subscriber(sel)
                    .history(true)
                    .callback(move |s: Sample| {
                        if s.kind() == zenoh::sample::SampleKind::Put
                            && let Ok(ZkKey::Alive { addr, .. }) = parse(s.key_expr().as_str())
                        {
                            let _ = tx.send(addr);
                        }
                    })
                    .await
                    .map_err(zenoh)?,
            );
        }
        drop(tx);
        Ok(tokio::time::timeout(timeout, rx.recv_async())
            .await
            .ok()
            .and_then(Result::ok))
    }

    /// The horizon `resource` declares (`freshness.v1` §2.1–§2.3), as the
    /// contract this consumer was compiled against states it.
    pub fn horizon(&self, resource: &str) -> Result<Horizon> {
        Ok(zenkey_model::freshness::horizon_of(
            self.resource(resource)?,
        ))
    }

    /// Whether one member of `resource` is fresh (`freshness.v1` §5), from
    /// the observations made of it: [`Subscription::freshness`], and
    /// [`crate::state::Current::freshness`] for a GET. Combined as §2.7
    /// says.
    pub fn freshness(&self, resource: &str, observations: &[Observation]) -> Result<Judged> {
        Ok(zenkey_model::freshness::judge_all(
            &self.horizon(resource)?,
            observations,
        ))
    }

    /// A provider's liveness (R7): from its token when presence is
    /// observable, from the freshness of `sub`'s deliveries when not.
    /// `window` is the horizon of what crosses (`freshness.v1` §2.9).
    pub async fn liveness(
        &self,
        sub: &Subscription,
        provider: &Addr,
        window: Duration,
        timeout: Duration,
    ) -> Result<Liveness> {
        Ok(match self.presence {
            Presence::Observable => {
                if self.present(timeout).await?.contains(provider) {
                    Liveness::Present
                } else {
                    Liveness::Absent
                }
            }
            Presence::Unavailable => match sub.last_seen(provider) {
                Some(t) if t.elapsed() <= window => Liveness::Fresh,
                _ => Liveness::Unobservable,
            },
        })
    }
}

impl Subscription {
    /// Samples discarded because their key was not concrete (R6).
    #[must_use]
    pub fn discarded(&self) -> u64 {
        self.seen.discarded.load(Ordering::Relaxed)
    }

    /// Samples dropped because their concrete key did not resolve: it is
    /// not a zk2 data key, names another interface or an unbound provider,
    /// or fits none of the resource's template positions (#671). These are
    /// a mismatch between the bus and the contract, not R6's rule, and a
    /// tool reports them apart from [`Subscription::discarded`] (the
    /// tooling guide's O6).
    #[must_use]
    pub fn unresolved(&self) -> u64 {
        self.seen.unresolved.load(Ordering::Relaxed)
    }

    /// When `provider` last delivered.
    #[must_use]
    pub fn last_seen(&self, provider: &Addr) -> Option<Instant> {
        self.seen
            .last
            .lock()
            .expect("not poisoned")
            .get(provider)
            .copied()
    }

    /// The members delivered so far, by key.
    #[must_use]
    pub fn members(&self) -> Vec<String> {
        self.seen
            .members
            .lock()
            .expect("not poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// How long it has listened: since it was declared.
    #[must_use]
    pub fn listened(&self) -> Duration {
        self.since.elapsed()
    }

    /// The member `key` as a `freshness.v1` observation now (§2.5): its last
    /// delivery's age on this host's monotonic clock, never against its
    /// stamp, and how long the subscription has listened. The callback
    /// receives every delivery, so the observation counts as complete.
    #[must_use]
    pub fn freshness(&self, key: &str) -> Observation {
        let last = self
            .seen
            .members
            .lock()
            .expect("not poisoned")
            .get(key)
            .map(|(at, deleted)| {
                if *deleted {
                    Last::Delete(at.elapsed())
                } else {
                    Last::Put(at.elapsed())
                }
            });
        Observation::Subscribed {
            last,
            listened: self.listened(),
            complete: true,
        }
    }

    /// Per stamping clock (a stamp's id), what this subscription measured
    /// of it, receipt minus stamp, over its lifetime, which is its reading
    /// (`freshness.v1` §2.6, ground 2, 0.2).
    #[must_use]
    pub fn clock_offsets(&self) -> BTreeMap<String, ClockMeasure> {
        self.seen.clocks.lock().expect("not poisoned").clone()
    }

    /// Whether this host's clock is trusted to `delta` against `clock`, a
    /// stamp's id (`freshness.v1` §2.6, ground 2, 0.2): a delivery stamped by
    /// it arrived within `delta` of this host's clock, and none arrived
    /// stamped further ahead than `delta`.
    #[must_use]
    pub fn clock_trust(&self, clock: &str, delta: Duration) -> ClockTrust {
        self.seen
            .clocks
            .lock()
            .expect("not poisoned")
            .get(clock)
            .map_or(ClockTrust::Untrusted, |m| m.trust(delta))
    }
}
