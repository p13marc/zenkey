//! Contract retrieval (spec §8.4): bundles by `(iface, fingerprint)`,
//! verified, cached, and handed out as revisions.
//!
//! A [`BundleStore`] retrieves through the runtime's own
//! [`zk2::retrieval::fetch_bundle`] — target `BestMatching`, then `All`,
//! consolidation `None`, each reply verified as it arrives and the first
//! valid one taken (§8.4, §9.6) — so a tool and a service follow one
//! implementation of the rule. What it adds is what a long-running tool
//! needs on top:
//!
//! - **One retrieval per revision.** A bundle is content-addressed, so a
//!   revision retrieved once is held for the life of the store, and
//!   concurrent asks for one revision share one retrieval.
//! - **Honest absences.** A revision that was never asked about is
//!   "not asked" ([`Contracts::state`] answers `None`); one asked about and
//!   not found is *unavailable*, with the refused replies' tags, and is
//!   re-asked only after [`UNAVAILABLE_TTL`] (a holder may come up), never
//!   per sample. A verified bundle whose contract does not read stays
//!   *unreadable*: the same bytes will never read differently.
//! - **A bound that says what it cost** (O6): at most
//!   [`DEFAULT_MAX_CONTRACTS`] revisions, the least recently used evicted
//!   and counted ([`BundleStore::evicted`]). The fingerprints come from
//!   descriptors, which come off the wire.
//! - **Pre-provisioning** (§8.5): [`BundleStore::seed`] holds revisions
//!   loaded offline, for a side of a constrained face that has no holder.
//!
//! The store takes the session per call, like the v1 schema store: which
//! session a tool reads through (namespaced for zk2's resolved verbs, by
//! the maintainer's decision of 2026-10-08) is the caller's, and a bundle's
//! bytes are the same in every namespace.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::grammar::IfaceId;
use zenoh::Session;

use crate::model::bounded::BoundedLru;
use crate::model::catalog::{ContractSet, ContractState, Contracts, Revision};
use crate::report::ContractSource;
use crate::{Error, Result};

/// How many revisions a store holds before evicting (O6). Far past any
/// deployment's interface count times its live revisions, and short of
/// what a descriptor naming made-up fingerprints could make it hold.
pub const DEFAULT_MAX_CONTRACTS: usize = 1_024;

/// How long an *unavailable* answer stands before the revision is asked
/// about again: a holder may come up, and a decode loop must not re-ask on
/// every sample.
pub const UNAVAILABLE_TTL: Duration = Duration::from_secs(30);

type Key = (IfaceId, Fingerprint);

#[derive(Debug)]
struct Entry {
    state: ContractState,
    at: Instant,
    /// Recency for the bound: a monotone counter, bumped on every hit.
    used: u64,
}

#[derive(Debug)]
struct Inner {
    timeout: Duration,
    entries: Mutex<BoundedLru<Key, Entry>>,
    /// One gate per revision being retrieved, so concurrent asks share one
    /// retrieval; removed when the retrieval lands.
    gates: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>,
    clock: AtomicU64,
    retrievals: AtomicU64,
    evicted: AtomicU64,
}

/// Contract revisions, retrieved from the bus by `(iface, fingerprint)`
/// (§8.4), verified and cached. Cloning shares the cache.
#[derive(Debug, Clone)]
pub struct BundleStore {
    inner: Arc<Inner>,
}

impl BundleStore {
    /// A store whose retrievals wait at most `timeout` per attempt (§8.4
    /// makes two: `BestMatching`, then `All`).
    pub fn new(timeout: Duration) -> BundleStore {
        BundleStore::bounded(timeout, DEFAULT_MAX_CONTRACTS)
    }

    /// [`BundleStore::new`] with a bound of `max` revisions.
    pub fn bounded(timeout: Duration, max: usize) -> BundleStore {
        BundleStore {
            inner: Arc::new(Inner {
                timeout,
                entries: Mutex::new(BoundedLru::with_capacity(max)),
                gates: Mutex::new(HashMap::new()),
                clock: AtomicU64::new(0),
                retrievals: AtomicU64::new(0),
                evicted: AtomicU64::new(0),
            }),
        }
    }

    /// Holds every revision of `set` (§8.5: bundles pre-provisioned on a
    /// side with no holder). A revision already held is kept.
    pub fn seed(&self, set: &ContractSet) {
        for r in set.iter() {
            let key = (r.iface().clone(), r.fingerprint().clone());
            let held = self
                .cached(&key, false)
                .is_some_and(|(s, _)| s.revision().is_some());
            if !held {
                self.put(key, ContractState::Held(Arc::clone(r)));
            }
        }
    }

    /// The revision `(iface, fingerprint)`: from the cache, or retrieved
    /// (§8.4) and cached. An error is a retrieval that could not be put on
    /// the bus, and is not cached; *unavailable* is an answer, and is.
    pub async fn fetch(
        &self,
        session: &Session,
        iface: &IfaceId,
        fingerprint: &Fingerprint,
    ) -> Result<ContractState> {
        let key = (iface.clone(), fingerprint.clone());
        if let Some((state, false)) = self.cached(&key, true) {
            return Ok(state);
        }
        let gate = {
            let mut gates = self.inner.gates.lock().expect("gates poisoned");
            Arc::clone(gates.entry(key.clone()).or_default())
        };
        let _held = gate.lock().await;
        // Whoever held the gate before may have landed it.
        if let Some((state, false)) = self.cached(&key, true) {
            return Ok(state);
        }
        self.inner.retrievals.fetch_add(1, Ordering::Relaxed);
        let retrieved =
            zk2::retrieval::fetch_bundle(session, iface, fingerprint, self.inner.timeout).await;
        let state = match retrieved {
            Ok(zk2::retrieval::Retrieved::Bundle(bundle, _)) => {
                match Revision::from_bundle(*bundle, ContractSource::Bus) {
                    Ok(r) => ContractState::Held(Arc::new(r)),
                    Err(e) => ContractState::Unreadable {
                        reason: e.to_string(),
                    },
                }
            }
            Ok(zk2::retrieval::Retrieved::Unavailable { refused }) => {
                ContractState::Unavailable { refused }
            }
            Err(e) => {
                self.ungate(&key);
                return Err(Error::bus(
                    "retrieve contract",
                    format!("{iface} {fingerprint}"),
                    e,
                ));
            }
        };
        self.put(key.clone(), state.clone());
        self.ungate(&key);
        Ok(state)
    }

    /// The contract of `(iface, fingerprint)`, shared, when it can be had:
    /// what a tool's consumer or client is built on
    /// ([`zk2::consumer::Consumer::for_tool`], [`zk2::Client::new`]).
    /// `None` is unavailable or unreadable; [`BundleStore::fetch`] says
    /// which.
    pub async fn contract(
        &self,
        session: &Session,
        iface: &IfaceId,
        fingerprint: &Fingerprint,
    ) -> Result<Option<Arc<Contract>>> {
        Ok(self
            .fetch(session, iface, fingerprint)
            .await?
            .revision()
            .map(|r| r.shared_contract()))
    }

    /// Retrieves every revision in `wanted` at once (typically
    /// [`Catalog::wanted`](crate::model::catalog::Catalog::wanted)), and
    /// returns each one's answer in the same order.
    pub async fn fetch_all(
        &self,
        session: &Session,
        wanted: &[(IfaceId, Fingerprint)],
    ) -> Vec<Result<ContractState>> {
        let mut tasks = tokio::task::JoinSet::new();
        for (i, (iface, fp)) in wanted.iter().cloned().enumerate() {
            let (store, session) = (self.clone(), session.clone());
            tasks.spawn(async move { (i, store.fetch(&session, &iface, &fp).await) });
        }
        let mut out: Vec<Option<Result<ContractState>>> = wanted.iter().map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((i, r)) => out[i] = Some(r),
                Err(e) => tracing::warn!("a contract retrieval task failed: {e}"),
            }
        }
        out.into_iter()
            .map(|r| {
                r.unwrap_or_else(|| Err(Error::Internal("a retrieval task did not finish".into())))
            })
            .collect()
    }

    /// Every revision held, in no particular order.
    pub fn held(&self) -> Vec<Arc<Revision>> {
        let entries = self.inner.entries.lock().expect("entries poisoned");
        entries
            .values()
            .filter_map(|e| e.state.revision().cloned())
            .collect()
    }

    /// How many retrievals this store has put on the bus.
    pub fn retrievals(&self) -> u64 {
        self.inner.retrievals.load(Ordering::Relaxed)
    }

    /// How many revisions the bound has evicted (O6).
    pub fn evicted(&self) -> u64 {
        self.inner.evicted.load(Ordering::Relaxed)
    }

    /// Forgets every answer that is not a revision in hand, so the next ask
    /// retrieves again.
    pub fn forget_absences(&self) {
        let mut entries = self.inner.entries.lock().expect("entries poisoned");
        let absent: Vec<Key> = entries
            .iter()
            .filter(|(_, e)| e.state.revision().is_none())
            .map(|(k, _)| k.clone())
            .collect();
        for k in absent {
            entries.remove(&k);
        }
    }

    /// The cached answer, and whether it is stale: a revision or an
    /// unreadable bundle never is, an unavailable answer is after
    /// [`UNAVAILABLE_TTL`] — still the last thing known, and re-asked by
    /// the next [`BundleStore::fetch`].
    fn cached(&self, key: &Key, touch: bool) -> Option<(ContractState, bool)> {
        let mut entries = self.inner.entries.lock().expect("entries poisoned");
        let e = entries.get_mut(key)?;
        let stale = matches!(e.state, ContractState::Unavailable { .. })
            && e.at.elapsed() >= UNAVAILABLE_TTL;
        if touch {
            e.used = self.inner.clock.fetch_add(1, Ordering::Relaxed);
        }
        Some((e.state.clone(), stale))
    }

    fn put(&self, key: Key, state: ContractState) {
        let mut entries = self.inner.entries.lock().expect("entries poisoned");
        if entries.get(&key).is_none() {
            let dropped = entries.admit(|e| e.used);
            self.inner
                .evicted
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        entries.insert(
            key,
            Entry {
                state,
                at: Instant::now(),
                used: self.inner.clock.fetch_add(1, Ordering::Relaxed),
            },
        );
    }

    fn ungate(&self, key: &Key) {
        self.inner.gates.lock().expect("gates poisoned").remove(key);
    }
}

/// What the store holds, without retrieving: a view built from it shows a
/// revision never asked about as not asked.
impl Contracts for BundleStore {
    fn state(&self, iface: &IfaceId, fingerprint: &Fingerprint) -> Option<ContractState> {
        self.cached(&(iface.clone(), fingerprint.clone()), false)
            .map(|(state, _)| state)
    }
}
