//! Consuming through a role (spec §3.2): data reaches a component from the
//! providers its configuration binds the role to, never from a provider
//! named in code.
//!
//! - **R1:** a binding is a list of service addresses, exact
//!   (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, `*/tc`). It resolves
//!   at once: a subscription needs no presence.
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
use std::time::{Duration, Instant};

use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Contract, Resource};
use zenkey_model::grammar::{Addr, IfaceId, ZkKey, parse};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Subscriber;
use zenoh::sample::Sample;

use crate::error::{Error, Result, zenoh};

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

    fn chunks(&self) -> (&str, &str) {
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
}

#[derive(Default)]
struct Seen {
    discarded: AtomicU64,
    last: Mutex<BTreeMap<Addr, Instant>>,
}

impl Consumer {
    pub(crate) fn new(
        session: &zenoh::Session,
        me: &Addr,
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
                let v = match v.as_str() {
                    "self.system" => me.system.to_string(),
                    "self.service" => me.service.to_string(),
                    other => other.to_owned(),
                };
                (k.clone(), v)
            })
            .collect();
        Ok(Self {
            session: session.clone(),
            role: role.to_owned(),
            contract,
            providers,
            params,
            presence: Presence::Observable,
        })
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

    fn resource(&self, name: &str) -> Result<&Resource> {
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
    /// discarded and counted (R6); every other one is delivered attributed.
    pub async fn subscribe<C>(&self, resource: &str, callback: C) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
    {
        let r = self.resource(resource)?.clone();
        let name = format!("{}/{}", r.token, r.template);
        let seen = Arc::new(Seen::default());
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
                            return;
                        };
                        if got != iface || !providers.iter().any(|p| p.matches(&addr)) {
                            return;
                        }
                        if r.kind == Kind::Event {
                            chunks.pop();
                        }
                        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                        let Some(values) = r.template.matches(&refs) else {
                            return;
                        };
                        seen.last
                            .lock()
                            .expect("not poisoned")
                            .insert(addr.clone(), Instant::now());
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
        Ok(Subscription { _subs: subs, seen })
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

    /// A provider's liveness (R7): from its token when presence is
    /// observable, from the freshness of `sub`'s deliveries when not.
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
}
