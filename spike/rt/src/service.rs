//! A mock zk2 service: every resource of its contracts served with mock
//! values, brought up in r3 §3.10's order.
//!
//! 1. **Resources:** publishers for streams and state; one `complete`
//!    queryable per concrete operation key (O1); one non-`complete`
//!    queryable per interface over `state/**` (and `@state/**`), answering
//!    the latest value or a tombstone (`reply_del`) with the producer's
//!    timestamp (S1–S3).
//! 2. Required exposure would be validated here (nothing is required of a mock).
//! 3. **The descriptor** queryable (on the instance key) and **the contract**
//!    queryables (on the location-free `zk2/@zk/contract/…` key, `complete`).
//! 4. **The instance token, then the interface tokens.** Alive ⇒ callable.
//!
//! Templated resources are instantiated for a few members (`m0`, `m1`, …).
//! The mock holds every capability its contracts' gates name, so every
//! resource is exposed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use zenkey_model::authoring::{Kind, ParamType};
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Congestion, Contract, Priority as P, Reliability as R, Resource};
use zenkey_model::grammar::{Addr, InstanceId, KindToken, alive_key, contract_key, data_key, instance_key};
use zenkey_model::template::Bindings;
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Publisher;
use zenoh::qos::{CongestionControl, Priority, Reliability};
use zenoh::query::Queryable;
use zenoh::time::Timestamp;

use crate::mock::{Mocker, Payload};

/// How long a tombstone answers GETs after a delete (S3).
pub const TOMBSTONE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
enum Entry {
    Value { bytes: Vec<u8>, encoding: Encoding, ts: Timestamp },
    Tombstone { ts: Timestamp, at: Instant },
}

type Store = Arc<Mutex<std::collections::HashMap<OwnedKeyExpr, Entry>>>;

/// Options for a mock service.
#[derive(Debug, Clone)]
pub struct Options {
    /// Members instantiated per templated resource (capped by its cardinality).
    pub members: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { members: 3 }
    }
}

/// A running mock service. Dropping it undeclares everything.
pub struct Service {
    pub addr: Addr,
    pub instance: InstanceId,
    pub descriptor: Value,
    /// Concrete keys served, by kind token, for the measuring clients.
    pub served: Vec<(String, KindToken, String)>,
    store: Store,
    publishers: BTreeMap<String, Publisher<'static>>,
    // Tokens first: they are undeclared first on drop.
    _tokens: Vec<LivelinessToken>,
    _queryables: Vec<Queryable<()>>,
    session: zenoh::Session,
}

fn qos_reliability(r: R) -> Reliability {
    match r {
        R::BestEffort => Reliability::BestEffort,
        R::Reliable => Reliability::Reliable,
    }
}

fn qos_congestion(c: Congestion) -> CongestionControl {
    match c {
        Congestion::Drop => CongestionControl::Drop,
        Congestion::Block => CongestionControl::Block,
    }
}

fn qos_priority(p: P) -> Priority {
    match p {
        P::RealTime => Priority::RealTime,
        P::InteractiveHigh => Priority::InteractiveHigh,
        P::InteractiveLow => Priority::InteractiveLow,
        P::DataHigh => Priority::DataHigh,
        P::Data => Priority::Data,
        P::DataLow => Priority::DataLow,
        P::Background => Priority::Background,
    }
}

/// Resource chunks for up to `n` members of a resource.
fn members(r: &Resource, n: u64) -> Vec<Vec<String>> {
    if !r.template.has_params() {
        return vec![r.template.build(&Bindings::new()).expect("no parameters")];
    }
    let n = n.min(r.cardinality.unwrap_or(1)).max(1);
    (0..n)
        .filter_map(|i| {
            let mut b = Bindings::new();
            for (name, ty) in &r.params {
                let v = match ty {
                    ParamType::String => vec![format!("m{i}")],
                    ParamType::Uint => vec![i.to_string()],
                    ParamType::Path => vec![format!("m{i}"), "x".to_owned()],
                };
                b.insert(name.clone(), v);
            }
            r.template.build(&b).ok()
        })
        .collect()
}

impl Service {
    /// Brings a mock service up on `session`, implementing `contracts`.
    pub async fn start(
        session: zenoh::Session,
        addr: Addr,
        instance: InstanceId,
        contracts: &[Arc<Contract>],
        opts: &Options,
    ) -> Result<Self> {
        let store: Store = Arc::default();
        let mut queryables = Vec::new();
        let mut publishers = BTreeMap::new();
        let mut served = Vec::new();
        let mut interfaces = Vec::new();
        let mut capabilities = std::collections::BTreeSet::new();

        // 1. Resources.
        for c in contracts {
            let mut mocker = Mocker::new(c);
            let mut has_state = false;
            let mut has_xstate = false;
            for r in &c.resources {
                for g in &r.gate {
                    if let Some(cap) = g.strip_prefix("capability:") {
                        capabilities.insert(cap.to_owned());
                    }
                }
                for chunks in members(r, opts.members) {
                    let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                    let key = data_key(&addr, &c.iface, r.token, &refs)?;
                    let ke = OwnedKeyExpr::try_from(key.as_str().to_owned()).map_err(|e| anyhow!("{e}"))?;
                    served.push((c.iface.to_string(), r.token, key.as_str().to_owned()));
                    match &r.body {
                        Body::Data(d) => {
                            if r.kind == Kind::Event {
                                // An event key ends in a fresh ULID per
                                // occurrence: nothing to declare up front.
                                continue;
                            }
                            let p = session
                                .declare_publisher(ke.clone())
                                .reliability(qos_reliability(d.reliability))
                                .congestion_control(qos_congestion(d.congestion))
                                .priority(qos_priority(d.priority))
                                .express(d.express)
                                .await
                                .map_err(|e| anyhow!("publisher {key}: {e}"))?;
                            if r.kind == Kind::State {
                                if r.token == KindToken::ExplicitState {
                                    has_xstate = true;
                                } else {
                                    has_state = true;
                                }
                                let Payload { bytes, encoding } = mocker.payload(&d.type_, d.encoding)?;
                                let ts = session.new_timestamp();
                                p.put(bytes.clone())
                                    .encoding(encoding.clone())
                                    .timestamp(ts)
                                    .await
                                    .map_err(|e| anyhow!("put {key}: {e}"))?;
                                store.lock().unwrap().insert(ke, Entry::Value { bytes, encoding, ts });
                            }
                            publishers.insert(key.as_str().to_owned(), p);
                        }
                        Body::Operation(o) => {
                            let reply = mocker.payload(&o.response, o.encoding)?;
                            let ke2 = ke.clone();
                            let q = session
                                .declare_queryable(ke)
                                .complete(true)
                                .callback(move |query| {
                                    let _ = query
                                        .reply(ke2.clone(), reply.bytes.clone())
                                        .encoding(reply.encoding.clone())
                                        .wait();
                                })
                                .await
                                .map_err(|e| anyhow!("op queryable {key}: {e}"))?;
                            queryables.push(q);
                        }
                    }
                }
            }
            for (has, token) in [(has_state, KindToken::State), (has_xstate, KindToken::ExplicitState)] {
                if !has {
                    continue;
                }
                let sel = format!("{}/{}/{}/{}/**", zenkey_model::grammar::GRAMMAR, addr, c.iface, token.as_str());
                let st = store.clone();
                let q = session
                    .declare_queryable(sel.clone())
                    .complete(false)
                    .callback(move |query| {
                        let entries: Vec<(OwnedKeyExpr, Entry)> = {
                            let mut s = st.lock().unwrap();
                            s.retain(|_, e| !matches!(e, Entry::Tombstone { at, .. } if at.elapsed() > TOMBSTONE_WINDOW));
                            s.iter()
                                .filter(|(k, _)| query.key_expr().intersects(k))
                                .map(|(k, e)| (k.clone(), e.clone()))
                                .collect()
                        };
                        for (k, e) in entries {
                            let _ = match e {
                                Entry::Value { bytes, encoding, ts } => {
                                    query.reply(k, bytes).encoding(encoding).timestamp(ts).wait()
                                }
                                Entry::Tombstone { ts, .. } => query.reply_del(k).timestamp(ts).wait(),
                            };
                        }
                    })
                    .await
                    .map_err(|e| anyhow!("state queryable {sel}: {e}"))?;
                queryables.push(q);
            }
            interfaces.push(json!({
                "iface": c.iface.to_string(),
                "contract": Fingerprint::of(c).to_string(),
                "minor": c.minor,
                "unavailable": [],
            }));
        }

        // 3. The descriptor and the contracts.
        let inst_key = instance_key(&addr, &instance)?;
        let descriptor = json!({
            "service": addr.to_string(),
            "instance": instance.as_str(),
            "interfaces": interfaces,
            "capabilities": capabilities,
            "requires": contracts.iter().flat_map(|c| c.requires.iter().map(|(role, r)| json!({
                "iface": c.iface.to_string(), "role": role, "interface": r.interface.to_string(), "bindings": [],
            }))).collect::<Vec<_>>(),
            "meta": {"zid": session.zid().to_string(), "build": "zk2-spike"},
        });
        let desc_bytes = serde_json::to_vec(&descriptor)?;
        let dk = inst_key.as_str().to_owned();
        let q = session
            .declare_queryable(dk.clone())
            .complete(true)
            .callback(move |query| {
                let _ = query.reply(dk.clone(), desc_bytes.clone()).encoding(Encoding::APPLICATION_JSON).wait();
            })
            .await
            .map_err(|e| anyhow!("descriptor queryable: {e}"))?;
        queryables.push(q);
        for c in contracts {
            let fp = Fingerprint::of(c);
            let ck = contract_key(&c.iface, fp.hex())?.as_str().to_owned();
            let bytes = Bundle::build(c).to_bytes();
            let ck2 = ck.clone();
            let q = session
                .declare_queryable(ck.clone())
                .complete(true)
                .callback(move |query| {
                    let _ = query.reply(ck2.clone(), bytes.clone()).encoding(Encoding::APPLICATION_JSON).wait();
                })
                .await
                .map_err(|e| anyhow!("contract queryable {ck}: {e}"))?;
            queryables.push(q);
        }

        // 4. The instance token, then the interface tokens.
        let mut tokens = vec![
            session
                .liveliness()
                .declare_token(inst_key.as_str().to_owned())
                .await
                .map_err(|e| anyhow!("instance token: {e}"))?,
        ];
        for c in contracts {
            let k = alive_key(&addr, &c.iface, &instance, &Fingerprint::of(c).hex().fp16())?;
            tokens.push(
                session
                    .liveliness()
                    .declare_token(k.as_str().to_owned())
                    .await
                    .map_err(|e| anyhow!("interface token {k}: {e}"))?,
            );
        }

        Ok(Self {
            addr,
            instance,
            descriptor,
            served,
            store,
            publishers,
            _tokens: tokens,
            _queryables: queryables,
            session,
        })
    }

    /// Deletes a state value: a delete on the wire and a tombstone in the
    /// store, both stamped (S2, S3).
    pub async fn delete_state(&self, key: &str) -> Result<()> {
        let ts = self.session.new_timestamp();
        let p = self.publishers.get(key).ok_or_else(|| anyhow!("{key} is not served here"))?;
        p.delete().timestamp(ts).await.map_err(|e| anyhow!("delete {key}: {e}"))?;
        let ke = OwnedKeyExpr::try_from(key.to_owned()).map_err(|e| anyhow!("{e}"))?;
        self.store.lock().unwrap().insert(ke, Entry::Tombstone { ts, at: Instant::now() });
        Ok(())
    }

    /// The session, for measurements that need it.
    #[must_use]
    pub fn session(&self) -> &zenoh::Session {
        &self.session
    }
}
