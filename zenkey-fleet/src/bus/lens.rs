//! What a raw observer's lens is read from (#612, FJ8b): one presence read
//! in the deployment's namespace, every instance's descriptor, and every
//! revision those descriptors name, retrieved into a [`BundleStore`]
//! (spec §8.1, §3.3, §8.4).
//!
//! [`read`] makes it once, for a windowed verb (`rate`, `field`,
//! `timeline`, `snapshot`, `check expect`); a [`LensFeed`] keeps it current
//! for a stream that runs until stopped (`echo`, zengui's panes), two ways:
//!
//! - **following presence** ([`LensFeed::on_presence`], #614 FL4): the
//!   moves a [`crate::PresenceFeed`] hands out are folded into the feed's
//!   read, and an instance that appears is described alone, its revisions
//!   retrieved, before the catalog is swapped. Its interfaces resolve at
//!   once — a tokenless one (U22) too, which no token selector shows —
//!   rather than after the next re-read;
//! - **re-reading in the background** when a key falls short of a provider
//!   or a revision ([`LensFeed::nudge`]), at most once per [`REFRESH_MIN`]
//!   and never inside the drain, whose broadcast would overflow while it
//!   waited (#338).
//!
//! **A descriptor kept across a token's return** (observer §3.11's MAY;
//! #982 H3). An instance that leaves takes its descriptor with it, and the
//! feed keeps it, by address and instance id
//! ([`crate::model::catalog::KeptDescriptors`]): when the same instance's
//! token returns — a first-hop owner's link opening again, several times a
//! day over a radio — the kept descriptor is its read again, and no GET
//! crosses the hop. A new instance id is described as any, and a full
//! re-read's descriptors replace what is kept. The cache is by instance and
//! run (#1074): a run can move within an instance with no token moving, so
//! the feed reads it from the runs its caller's live puts state in band
//! ([`LensFeed::runs_in`], [`LensFeed::on_runs`]). A kept descriptor a put
//! under another run outran is read again at its token's return, and a
//! reused one at that put. What the owner put while its token was gone is
//! lost to the feed until then, or until its next full read. Each
//! `(address, run)` is read once (#1079): the GET it owed, in flight or
//! answered silent, is not asked again until a token of the address goes
//! or comes, or the next full read.
//!
//! The patch and the re-read never interleave: each holds the feed's read
//! for its whole run. A put's run takes it twice, never across a GET: once
//! to say what is owed, and again to apply what the GETs served, an answer
//! something newer replaced meanwhile dropped. [`LensFeed::generation`]
//! moves whenever any of them swaps the catalog or the store's answers
//! change, which is what an [`IdentityCache`](crate::IdentityCache) keys
//! its resolutions on.
//!
//! The session is the caller's, **in** the namespace: the presence and
//! contract keys are base-relative (decided 2026-10-08). The observer's
//! own subscription stays on a session in no namespace, which is what lets
//! it see a key outside the namespace and say so (O1, O3).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::{ArcSwap, ArcSwapOption};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::wire::Run;
use zenoh::Session;

use crate::Result;
use crate::bus::contracts::BundleStore;
use crate::bus::monitor::SampleView;
use crate::bus::presence::{Scope, describe_each, observe};
use crate::model::catalog::{Catalog, DescriptorRead, KeptDescriptors, Observed};
use crate::report::{PresenceTransition, Unresolved};

/// How long a feed waits between two background re-reads, however often it
/// is nudged: a decode loop must not turn every unresolved key into a
/// presence read.
pub const REFRESH_MIN: Duration = Duration::from_secs(5);

/// Every service in the namespace from its tokens and descriptors, and every
/// revision the descriptors name retrieved into `store`. A retrieval that
/// failed is the store's to say (unavailable, or not held): the catalog is
/// returned either way.
pub async fn read(session: &Session, store: &BundleStore, timeout: Duration) -> Result<Catalog> {
    read_observed(session, store, timeout)
        .await
        .map(|o| Catalog::new(&o))
}

/// [`read`], keeping the read itself: what a [`LensFeed`] patches.
async fn read_observed(
    session: &Session,
    store: &BundleStore,
    timeout: Duration,
) -> Result<Observed> {
    let observed = observe(session, &Scope::all(), timeout).await?;
    let _ = store
        .fetch_all(session, &Catalog::new(&observed).wanted())
        .await;
    Ok(observed)
}

/// What a [`LensFeed`] holds under its lock: the read, and the descriptors
/// kept of the instances that left it (observer §3.11's MAY).
#[derive(Default)]
struct FeedRead {
    observed: Option<Observed>,
    kept: KeptDescriptors,
}

/// The runs already read at each address a put's run tells something of
/// ([`KeptDescriptors::addresses`], [`KeptDescriptors::asked_of`]), by
/// address as spelt: what [`LensFeed::runs_in`] filters by.
type Awaited = BTreeMap<String, Vec<Run>>;

/// What a test describes instances with, in place of the bus's GETs.
#[cfg(test)]
type Describe = Arc<
    dyn Fn(
            Vec<(Addr, InstanceId)>,
        )
            -> futures_util::future::BoxFuture<'static, BTreeMap<(Addr, InstanceId), DescriptorRead>>
        + Send
        + Sync,
>;

/// A lens's presence read kept current: following presence, and re-read in
/// the background when nudged.
#[derive(Clone)]
pub struct LensFeed {
    session: Session,
    store: BundleStore,
    timeout: Duration,
    current: Arc<ArcSwapOption<Catalog>>,
    /// The read the catalog is built from, and the descriptors kept of the
    /// instances that left it, held for the whole of a patch or a re-read
    /// so the two never interleave.
    observed: Arc<tokio::sync::Mutex<FeedRead>>,
    /// Descriptor GETs the feed made following presence, one per instance
    /// that appeared undescribed or whose reused descriptor a put outran; a
    /// re-read's are not counted.
    described: Arc<AtomicU64>,
    /// The addresses a put's run tells something of, each with the runs
    /// already read there, as each patch leaves them: what
    /// [`LensFeed::runs_in`] filters by without the read's lock.
    awaited: Arc<ArcSwap<Awaited>>,
    /// Catalog swaps since the feed opened.
    swaps: Arc<AtomicU64>,
    refreshing: Arc<AtomicBool>,
    last: Arc<std::sync::Mutex<tokio::time::Instant>>,
    #[cfg(test)]
    describe_with: Option<Describe>,
}

impl LensFeed {
    /// The first read, awaited: a feed starts with what the namespace holds
    /// now. A read that could not be put on the bus leaves the feed without
    /// a catalog — every key then resolves to its address and says presence
    /// was not read (O4) — and the error is returned beside it.
    pub async fn open(
        session: &Session,
        store: &BundleStore,
        timeout: Duration,
    ) -> (LensFeed, Option<crate::Error>) {
        let (observed, error) = match read_observed(session, store, timeout).await {
            Ok(o) => (Some(o), None),
            Err(e) => (None, Some(e)),
        };
        (LensFeed::over(session, store, timeout, observed), error)
    }

    /// A feed over `observed`, as [`LensFeed::open`] read it.
    fn over(
        session: &Session,
        store: &BundleStore,
        timeout: Duration,
        observed: Option<Observed>,
    ) -> LensFeed {
        let catalog = observed.as_ref().map(|o| Arc::new(Catalog::new(o)));
        LensFeed {
            session: session.clone(),
            store: store.clone(),
            timeout,
            current: Arc::new(ArcSwapOption::new(catalog)),
            observed: Arc::new(tokio::sync::Mutex::new(FeedRead {
                observed,
                kept: KeptDescriptors::default(),
            })),
            described: Arc::new(AtomicU64::new(0)),
            awaited: Arc::default(),
            swaps: Arc::new(AtomicU64::new(0)),
            refreshing: Arc::new(AtomicBool::new(false)),
            last: Arc::new(std::sync::Mutex::new(tokio::time::Instant::now())),
            #[cfg(test)]
            describe_with: None,
        }
    }

    /// The latest catalog; `None` when no read has succeeded and presence
    /// has moved nothing yet.
    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        self.current.load_full()
    }

    /// The store the feed retrieves revisions into.
    pub fn store(&self) -> &BundleStore {
        &self.store
    }

    /// A number that moves whenever what a lens over this feed resolves
    /// with may have changed: a catalog swapped, by a patch or a re-read, or
    /// the store's answers moved ([`BundleStore::generation`]). A frontend
    /// compares it on each tick and hands it to its lens
    /// ([`crate::Lens::at_generation`]), so an
    /// [`IdentityCache`](crate::IdentityCache) resolves again exactly when
    /// it must. Never moves for a read that changed nothing it swapped.
    pub fn generation(&self) -> u64 {
        self.swaps.load(Ordering::Acquire) + self.store.generation()
    }

    /// How many descriptor GETs following presence made: one per instance
    /// that appeared and was neither described nor kept, and one per
    /// reused descriptor a put under another run outran, once per run
    /// ([`LensFeed::on_runs`]). A re-read's are not counted.
    pub fn descriptors_read(&self) -> u64 {
        self.described.load(Ordering::Acquire)
    }

    /// How many times an instance token's return reused the descriptor the
    /// instance took with it (observer §3.11's MAY; #982 H3): each a
    /// descriptor GET not made across the hop. Waits for a patch or a
    /// re-read in flight, never for a put's GET ([`LensFeed::on_runs`]).
    pub async fn descriptors_kept(&self) -> u64 {
        self.observed.lock().await.kept.reused()
    }

    /// Fold presence moves into the feed's read — the transitions a
    /// [`crate::PresenceFeed`] hands out, a seed's included — describe every
    /// instance that now holds its instance token and was never described
    /// here (each alone, §3.3), retrieve the revisions those descriptors
    /// name (§8.4), and swap the catalog. Returns whether the catalog was
    /// swapped; a batch that moved no token (statuses alone) swaps nothing.
    ///
    /// An instance that appears resolves at once, its tokenless interfaces
    /// (U22) among them, rather than after the next re-read; one that leaves
    /// takes its instance token and its descriptor with it, and the feed
    /// keeps that descriptor: the same instance's token returning reads it
    /// again, with no GET (observer §3.11's MAY; #982 H3). A token of an
    /// address gone or come reads every run its puts stated again (#1079).
    /// A feed whose first read failed starts a read from these moves alone,
    /// marked possibly incomplete (§8.1): what presence has not moved is not
    /// in it. Waits for a background re-read in flight, and the re-read for
    /// it; never for a put's GET ([`LensFeed::on_runs`]).
    pub async fn on_presence(&self, moved: &[PresenceTransition]) -> bool {
        let mut guard = self.observed.lock().await;
        let FeedRead { observed, kept } = &mut *guard;
        let observed = observed.get_or_insert_with(|| {
            let mut o = Observed::from_keys(Scope::all().selector(), &[], false);
            o.descriptors = Some(Default::default());
            o
        });
        let (changed, _reused) = observed.follow_keeping(moved, kept);
        self.awaits(kept);
        if !changed {
            return false;
        }
        let owed = observed.undescribed();
        if !owed.is_empty() {
            self.described
                .fetch_add(owed.len() as u64, Ordering::AcqRel);
            let found = self.describe(owed).await;
            let mut wanted = Vec::new();
            for ((addr, instance), read) in found {
                if let Some(d) = read.descriptor() {
                    wanted.extend(revisions_named(d));
                }
                observed.with_descriptor(&addr, &instance, read);
            }
            let _ = self.store.fetch_all(&self.session, &wanted).await;
        }
        self.swap(Catalog::new(observed));
        true
    }

    /// Of `samples`, a monitor's on a session in no namespace under
    /// `namespace` (a tick's, as zengui's pump hands them), the run each put
    /// of a zk2 data key states in band (core §4.3), by address, once each:
    /// what [`LensFeed::on_runs`] reads (#1074). Only for an address whose
    /// descriptor the feed keeps, or holds reused on trust
    /// ([`KeptDescriptors::addresses`]), so it costs one load while the
    /// feed holds none, and only a run not read there yet
    /// ([`KeptDescriptors::asked`], #1079): a tick that hears a run whose
    /// GET is in flight, or answered silent, returns nothing, and its
    /// caller has nothing to hand on. Hand it live samples: a GET's reply
    /// keeps the run it was put under.
    pub fn runs_in(&self, namespace: &str, samples: &[Arc<SampleView>]) -> Vec<(Addr, Run)> {
        let awaited = self.awaited.load();
        if awaited.is_empty() {
            return Vec::new();
        }
        let mut heard: Vec<(Addr, Run)> = Vec::new();
        for s in samples {
            let Some(relative) = crate::model::namespace::strip(namespace, &s.key) else {
                continue;
            };
            let Some(asked) = address_of(relative).and_then(|a| awaited.get(a)) else {
                continue;
            };
            let Some(stamp) = s.inband() else {
                continue;
            };
            if asked.contains(&stamp.run) {
                continue;
            }
            let Ok(ZkKey::Data { addr, .. }) = zenkey_model::grammar::parse(relative) else {
                continue;
            };
            if !heard.iter().any(|(a, r)| *a == addr && *r == stamp.run) {
                heard.push((addr, stamp.run));
            }
        }
        heard
    }

    /// The runs live puts stated in band, by address ([`LensFeed::runs_in`];
    /// core §4.3, #1074), read against the descriptors kept and reused
    /// ([`Observed::heard_run`]): a kept one stating another run is
    /// forgotten, so its token's return owes its GET, and a reused one on
    /// trust stating another run is read again, one GET each, its revisions
    /// retrieved and the catalog swapped. Returns whether the catalog was
    /// swapped.
    ///
    /// **Asked once** (#1079). Each `(address, run)` is read once
    /// ([`KeptDescriptors::asked`]): the GETs it owed, in flight or answered
    /// silent, are not asked again until a token of the address goes or
    /// comes, or the next full read. A GET that serves no descriptor leaves
    /// the reused one held, on trust: a silent GET proves nothing (§3.3).
    ///
    /// **Outside the lock.** The feed's read is held only to say what is
    /// owed, marked asked, and again to apply what the GETs served: never
    /// across a GET or a retrieval, so [`LensFeed::on_presence`], a re-read
    /// and another call wait for none. An answer is applied only where the
    /// instance still stands on trust under the reuse it was asked of
    /// ([`KeptDescriptors::served`]); one that presence or a re-read
    /// outran meanwhile, or another answer came before, is dropped.
    pub async fn on_runs(&self, heard: &[(Addr, Run)]) -> bool {
        if heard.is_empty() {
            return false;
        }
        // What is owed, each instance by the reuse it stands on trust under.
        let owed: BTreeMap<(Addr, InstanceId), u64> = {
            let mut guard = self.observed.lock().await;
            let FeedRead { observed, kept } = &mut *guard;
            let Some(observed) = observed.as_mut() else {
                return false;
            };
            let mut owed = BTreeMap::new();
            for (addr, run) in heard {
                for (a, i) in observed.heard_run(addr, *run, kept) {
                    if let Some(reuse) = kept.reuse_of(&a, &i) {
                        owed.insert((a, i), reuse);
                    }
                }
            }
            self.awaits(kept);
            owed
        };
        if owed.is_empty() {
            return false;
        }
        self.described
            .fetch_add(owed.len() as u64, Ordering::AcqRel);
        let found = self.describe(owed.keys().cloned().collect()).await;
        let served: Vec<_> = found
            .into_iter()
            .filter(|(_, read)| read.descriptor().is_some())
            .collect();
        if served.is_empty() {
            return false;
        }
        let wanted: Vec<_> = served
            .iter()
            .filter_map(|(_, read)| read.descriptor())
            .flat_map(revisions_named)
            .collect();
        let _ = self.store.fetch_all(&self.session, &wanted).await;
        // What still stands, applied.
        let mut guard = self.observed.lock().await;
        let FeedRead { observed, kept } = &mut *guard;
        let Some(observed) = observed.as_mut() else {
            return false;
        };
        let mut replaced = false;
        for (key, read) in served {
            let Some(&reuse) = owed.get(&key) else {
                continue;
            };
            if kept.served(&key.0, &key.1, reuse) {
                observed.with_descriptor(&key.0, &key.1, read);
                replaced = true;
            }
        }
        self.awaits(kept);
        if !replaced {
            return false;
        }
        self.swap(Catalog::new(observed));
        true
    }

    /// A key resolved short of what a fresh read might give it — no
    /// provider, no revision, a revision not yet held, or no read at all:
    /// re-read in the background, at most once per [`REFRESH_MIN`].
    pub fn nudge(&self, why: Option<&Unresolved>) {
        let worth = matches!(
            why,
            Some(
                Unresolved::NoProvider
                    | Unresolved::NoRevision
                    | Unresolved::ContractNotHeld { .. }
                    | Unresolved::PresenceNotRead
            )
        );
        if !worth {
            return;
        }
        {
            let mut last = self.last.lock().expect("feed clock poisoned");
            if last.elapsed() < REFRESH_MIN {
                return;
            }
            if self.refreshing.swap(true, Ordering::AcqRel) {
                return;
            }
            *last = tokio::time::Instant::now();
        }
        let feed = self.clone();
        tokio::spawn(async move {
            let mut guard = feed.observed.lock().await;
            if let Ok(o) = read_observed(&feed.session, &feed.store, feed.timeout).await {
                feed.swap(Catalog::new(&o));
                guard.kept.forget_described(&o);
                feed.awaits(&guard.kept);
                guard.observed = Some(o);
            }
            drop(guard);
            feed.refreshing.store(false, Ordering::Release);
        });
    }

    fn swap(&self, catalog: Catalog) {
        self.current.store(Some(Arc::new(catalog)));
        self.swaps.fetch_add(1, Ordering::AcqRel);
    }

    /// Publish the addresses `kept` leaves a put's run telling something
    /// of, each with the runs already read there, for
    /// [`LensFeed::runs_in`].
    fn awaits(&self, kept: &KeptDescriptors) {
        let now: Awaited = kept
            .addresses()
            .iter()
            .map(|a| (a.to_string(), kept.asked_of(a).to_vec()))
            .collect();
        if **self.awaited.load() != now {
            self.awaited.store(Arc::new(now));
        }
    }

    /// Each instance's descriptor GET (§3.3), as [`describe_each`] makes
    /// them, or a test's.
    async fn describe(
        &self,
        wanted: Vec<(Addr, InstanceId)>,
    ) -> BTreeMap<(Addr, InstanceId), DescriptorRead> {
        #[cfg(test)]
        if let Some(describe) = &self.describe_with {
            return describe(wanted).await;
        }
        describe_each(&self.session, wanted, self.timeout).await
    }
}

/// The revisions a descriptor names, as the store retrieves them (§8.4).
fn revisions_named(d: &Descriptor) -> impl Iterator<Item = (IfaceId, Fingerprint)> + '_ {
    d.interfaces.iter().filter_map(|e| {
        Some((
            e.iface.parse::<IfaceId>().ok()?,
            Fingerprint::parse(&e.contract).ok()?,
        ))
    })
}

/// The `<system>/<service>` a base-relative zk2 key starts with, as it is
/// spelt, its other chunks unread: [`LensFeed::runs_in`]'s filter before
/// it parses a key.
fn address_of(relative: &str) -> Option<&str> {
    let rest = relative.strip_prefix("zk2/")?;
    let system = rest.find('/')?;
    let service = rest[system + 1..]
        .find('/')
        .map_or(rest.len(), |i| system + 1 + i);
    Some(&rest[..service])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use futures_util::FutureExt;
    use zenkey_model::grammar::{Addr, InstanceId};
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

    use super::{Describe, LensFeed, address_of};
    use crate::bus::contracts::BundleStore;
    use crate::bus::monitor::SampleView;
    use crate::model::catalog::{DescriptorRead, Observed};
    use crate::report::{PresenceChange, PresenceSource, PresenceTransition};

    /// The filter reads a key's address as spelt, and nothing that is not
    /// one.
    #[test]
    fn a_key_s_address_is_its_first_two_chunks_after_zk2() {
        assert_eq!(
            address_of("zk2/lab/radio-a/hop.v1/stream/level"),
            Some("lab/radio-a")
        );
        assert_eq!(address_of("zk2/lab/radio-a"), Some("lab/radio-a"));
        assert_eq!(address_of("zk2/lab"), None);
        assert_eq!(address_of("other/lab/radio-a/x"), None);
    }

    // ─── a put's run, asked once and outside the lock (#1079) ──────────────

    /// The device every test here watches, a first-hop owner.
    const DEVICE: &str = "lab/radio-a";

    fn device() -> Addr {
        DEVICE.parse().expect("an address")
    }

    /// The owners a test's feed asks in place of the bus: each instance's
    /// descriptor as its owner serves it when the GET answers (none is
    /// silence), every GET counted, and, while a gate is set, each GET held
    /// until the test lets one through.
    struct Owners {
        serves: std::sync::Mutex<BTreeMap<(Addr, InstanceId), DescriptorRead>>,
        gets: AtomicU64,
        gate: std::sync::Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        /// One permit per GET started.
        started: tokio::sync::Semaphore,
    }

    impl Owners {
        fn new() -> Arc<Owners> {
            Arc::new(Owners {
                serves: std::sync::Mutex::default(),
                gets: AtomicU64::new(0),
                gate: std::sync::Mutex::default(),
                started: tokio::sync::Semaphore::new(0),
            })
        }

        /// `instance`'s descriptor, stating `run`, served from here on.
        fn serve(&self, instance: &InstanceId, run: &str) {
            let d = serde_json::json!({
                "format": "zk2-descriptor/0.1",
                "service": DEVICE,
                "instance": instance.to_string(),
                "interfaces": [],
                "run": run,
            });
            let d = serde_json::from_value(d).expect("a descriptor");
            self.serves.lock().unwrap().insert(
                (device(), instance.clone()),
                DescriptorRead::Served(Box::new(d)),
            );
        }

        /// `instance`'s GET silent from here on.
        fn silence(&self, instance: &InstanceId) {
            self.serves
                .lock()
                .unwrap()
                .remove(&(device(), instance.clone()));
        }

        fn gets(&self) -> u64 {
            self.gets.load(Ordering::SeqCst)
        }

        /// Hold every GET from here on; the gate lets one through per
        /// permit added. [`Owners::started`] waits for a GET started after
        /// this.
        fn shut(&self) -> Arc<tokio::sync::Semaphore> {
            while let Ok(started) = self.started.try_acquire() {
                started.forget();
            }
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            *self.gate.lock().unwrap() = Some(Arc::clone(&gate));
            gate
        }

        /// The GETs answer as soon as asked again.
        fn open(&self) {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                gate.add_permits(64);
            }
        }

        /// Wait until a GET has started.
        async fn started(&self) {
            self.started.acquire().await.expect("open").forget();
        }

        fn describe(self: &Arc<Owners>) -> Describe {
            let owners = Arc::clone(self);
            Arc::new(move |wanted: Vec<(Addr, InstanceId)>| {
                let owners = Arc::clone(&owners);
                async move {
                    owners.gets.fetch_add(wanted.len() as u64, Ordering::SeqCst);
                    owners.started.add_permits(1);
                    let gate = owners.gate.lock().unwrap().clone();
                    if let Some(gate) = gate {
                        gate.acquire().await.expect("open").forget();
                    }
                    let serves = owners.serves.lock().unwrap();
                    wanted
                        .into_iter()
                        .map(|k| {
                            let read = serves.get(&k).cloned().unwrap_or(DescriptorRead::Silent);
                            (k, read)
                        })
                        .collect::<BTreeMap<_, _>>()
                }
                .boxed()
            })
        }
    }

    /// A feed whose first read was complete and showed nothing, describing
    /// through `owners`.
    async fn feed(owners: &Arc<Owners>) -> LensFeed {
        let session = crate::bus::session::standalone().await;
        let timeout = Duration::from_secs(1);
        let mut read = Observed::from_keys("zk2/*/*/@zk/**", &[], true);
        read.descriptors = Some(BTreeMap::new());
        let mut lens = LensFeed::over(&session, &BundleStore::new(timeout), timeout, Some(read));
        lens.describe_with = Some(owners.describe());
        lens
    }

    fn moved(address: &Addr, change: PresenceChange) -> PresenceTransition {
        PresenceTransition {
            at: std::time::SystemTime::UNIX_EPOCH,
            source: PresenceSource::Event,
            address: address.clone(),
            change,
        }
    }

    fn appeared(instance: &InstanceId) -> PresenceTransition {
        moved(
            &device(),
            PresenceChange::InstanceAppeared {
                instance: instance.clone(),
            },
        )
    }

    fn gone(instance: &InstanceId) -> PresenceTransition {
        moved(
            &device(),
            PresenceChange::InstanceGone {
                instance: instance.clone(),
            },
        )
    }

    /// A live put of the device's under `run`, as a monitor in no
    /// namespace hears it.
    fn put(run: Run) -> Arc<SampleView> {
        let stamp = Stamp {
            run,
            seq: Seq::new(1).expect("a seq"),
        };
        Arc::new(SampleView {
            key: format!("zk2/{DEVICE}/hop.v1/stream/level"),
            payload: Envelope::mutation(Tag::NONE, stamp, b"1").write().into(),
            encoding: zenkey_model::wire::ENCODING.to_owned(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        })
    }

    /// The runs a tick hearing one put under each of `runs` hands on.
    fn tick(lens: &LensFeed, runs: &[Run]) -> Vec<(Addr, Run)> {
        let puts: Vec<_> = runs.iter().map(|r| put(*r)).collect();
        lens.runs_in("", &puts)
    }

    /// The runs the lens's catalog reads in the device's descriptors.
    fn runs_listed(lens: &LensFeed) -> Vec<String> {
        lens.catalog()
            .map(|c| c.descriptors().filter_map(|d| d.run.clone()).collect())
            .unwrap_or_default()
    }

    /// `instance` described under `run`, then its token gone and back: its
    /// kept descriptor reused, on trust.
    async fn reused(lens: &LensFeed, owners: &Owners, instance: &InstanceId, run: &str) {
        owners.serve(instance, run);
        assert!(lens.on_presence(&[appeared(instance)]).await);
        assert!(lens.on_presence(&[gone(instance)]).await);
        let kept = lens.descriptors_kept().await;
        assert!(lens.on_presence(&[appeared(instance)]).await);
        assert_eq!(lens.descriptors_kept().await, kept + 1, "reused");
    }

    /// The issue's first case: a descriptor GET that serves nothing, as
    /// across a constrained face, and ten ticks hearing the same run, make
    /// one GET. The reused descriptor stays held and on trust, so a later
    /// run asks again; a token of the address gone and back reads every
    /// run again, and an answer that serves takes it off trust.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_silent_get_is_asked_once_however_many_ticks_hear_its_run() {
        let owners = Owners::new();
        let lens = feed(&owners).await;
        let a = InstanceId::from_u64(9);
        reused(&lens, &owners, &a, "00ab").await;
        owners.silence(&a);
        let (gets, read) = (owners.gets(), lens.descriptors_read());
        let (r2, r3) = (Run::U16(0xac), Run::U16(0xad));

        let heard = tick(&lens, &[r2]);
        assert_eq!(heard, [(device(), r2)]);
        assert!(!lens.on_runs(&heard).await, "silent: nothing swapped");
        for _ in 0..10 {
            assert!(tick(&lens, &[r2]).is_empty(), "a tick hands nothing on");
            assert!(!lens.on_runs(&[(device(), r2)]).await, "nor a caller");
        }
        assert_eq!(owners.gets(), gets + 1, "one GET");
        assert_eq!(lens.descriptors_read(), read + 1);
        assert_eq!(runs_listed(&lens), ["00ab"], "the reused one held");

        // On trust still: a later run asks again, once.
        assert_eq!(tick(&lens, &[r3, r2]), [(device(), r3)]);
        assert!(!lens.on_runs(&[(device(), r3)]).await);
        assert_eq!(owners.gets(), gets + 2);

        // A token of the address gone and back: every run read again, and
        // this time the GET serves.
        assert!(lens.on_presence(&[gone(&a), appeared(&a)]).await);
        owners.serve(&a, "00ac");
        let heard = tick(&lens, &[r2]);
        assert_eq!(heard, [(device(), r2)]);
        assert!(lens.on_runs(&heard).await, "served: swapped");
        assert_eq!(owners.gets(), gets + 3);
        assert_eq!(runs_listed(&lens), ["00ac"]);
        assert!(
            tick(&lens, &[r2, r3]).is_empty(),
            "off trust: no put is read against it"
        );
    }

    /// The issue's second case: two instances of one address, an old
    /// session putting under r1 and a new one under r2, both reused on
    /// trust, their puts alternating. A put names its address, never its
    /// instance, so each run heard owes the GET of the instance stating the
    /// other. The bound: one GET per instance on trust per run heard, until
    /// a token of the address goes or comes. Here two, however long the
    /// puts alternate, silent or served.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_instances_alternating_runs_cost_one_get_each() {
        let owners = Owners::new();
        let lens = feed(&owners).await;
        let (old, new) = (InstanceId::from_u64(9), InstanceId::from_u64(10));
        let (r1, r2) = (Run::U16(0xab), Run::U16(0xac));
        reused(&lens, &owners, &old, "00ab").await;
        reused(&lens, &owners, &new, "00ac").await;

        // Silent.
        owners.silence(&old);
        owners.silence(&new);
        let gets = owners.gets();
        for _ in 0..10 {
            for r in [r1, r2] {
                let heard = tick(&lens, &[r]);
                lens.on_runs(&heard).await;
                lens.on_runs(&[(device(), r)]).await;
            }
        }
        assert_eq!(owners.gets(), gets + 2, "one each");

        // Both tokens gone and back, and the GETs serve: one each again,
        // then neither is on trust, and no put is read at all.
        assert!(
            lens.on_presence(&[gone(&old), gone(&new), appeared(&old), appeared(&new)])
                .await
        );
        owners.serve(&old, "00ab");
        owners.serve(&new, "00ac");
        let gets = owners.gets();
        for _ in 0..10 {
            for r in [r1, r2] {
                let heard = tick(&lens, &[r]);
                lens.on_runs(&heard).await;
            }
        }
        assert_eq!(owners.gets(), gets + 2, "one each");
        let mut listed = runs_listed(&lens);
        listed.sort();
        assert_eq!(listed, ["00ab", "00ac"]);
        assert!(tick(&lens, &[r1, r2]).is_empty());
    }

    /// The issue's third case: a put's GET holds no lock while it is out.
    /// `on_presence` folds a move at once, the kept count reads at once, a
    /// tick hearing the run hands nothing on, and a second call for it
    /// returns at once; the GET's answer lands after.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn on_presence_does_not_wait_on_a_get_in_flight() {
        let owners = Owners::new();
        let lens = feed(&owners).await;
        let a = InstanceId::from_u64(9);
        reused(&lens, &owners, &a, "00ab").await;
        // An instance at another address, to move while the GET is out.
        let ground: Addr = "lab/ground".parse().expect("an address");
        let other = InstanceId::from_u64(11);
        let at_ground = |change| moved(&ground, change);
        assert!(
            lens.on_presence(&[at_ground(PresenceChange::InstanceAppeared {
                instance: other.clone(),
            })])
            .await
        );
        owners.serve(&a, "00ac");
        let gate = owners.shut();
        let r2 = Run::U16(0xac);
        let asking = {
            let lens = lens.clone();
            tokio::spawn(async move { lens.on_runs(&[(device(), r2)]).await })
        };
        owners.started().await;

        assert!(lens.observed.try_lock().is_ok(), "the read is free");
        assert_eq!(
            lens.on_presence(&[at_ground(PresenceChange::InstanceGone {
                instance: other.clone(),
            })])
            .now_or_never(),
            Some(true),
            "a move folds at once"
        );
        assert!(lens.descriptors_kept().now_or_never().is_some());
        assert!(tick(&lens, &[r2]).is_empty(), "the tick hands nothing on");
        assert_eq!(lens.on_runs(&[(device(), r2)]).now_or_never(), Some(false));

        gate.add_permits(1);
        assert!(asking.await.expect("the call"), "its answer lands");
        assert_eq!(runs_listed(&lens), ["00ac"]);
        assert_eq!(owners.gets(), 3, "two instances described, and the one GET");
    }

    /// An answer something newer replaced meanwhile is dropped: a token of
    /// the address went and came back while the GET was out (the read on
    /// trust is another reuse now), or a full re-read replaced the read.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_answer_presence_or_a_re_read_outran_is_dropped() {
        let owners = Owners::new();
        let lens = feed(&owners).await;
        let a = InstanceId::from_u64(9);
        let r2 = Run::U16(0xac);
        reused(&lens, &owners, &a, "00ab").await;
        owners.serve(&a, "00ac");

        // Presence moves while the GET is out.
        let gate = owners.shut();
        let asking = {
            let lens = lens.clone();
            tokio::spawn(async move { lens.on_runs(&[(device(), r2)]).await })
        };
        owners.started().await;
        assert_eq!(
            lens.on_presence(&[gone(&a), appeared(&a)]).now_or_never(),
            Some(true)
        );
        gate.add_permits(1);
        assert!(!asking.await.expect("the call"), "dropped");
        assert_eq!(runs_listed(&lens), ["00ab"]);

        // The run is read again; a re-read lands while the GET is out.
        let heard = tick(&lens, &[r2]);
        assert_eq!(heard, [(device(), r2)]);
        let gate = owners.shut();
        let asking = {
            let lens = lens.clone();
            tokio::spawn(async move { lens.on_runs(&heard).await })
        };
        owners.started().await;
        {
            let mut guard = lens.observed.lock().await;
            let mut read = guard.observed.clone().expect("a read");
            let d = owners.serves.lock().unwrap()[&(device(), a.clone())].clone();
            read.with_descriptor(&device(), &a, d);
            guard.kept.forget_described(&read);
            lens.awaits(&guard.kept);
            guard.observed = Some(read);
        }
        gate.add_permits(1);
        assert!(!asking.await.expect("the call"), "dropped");
        assert!(tick(&lens, &[r2]).is_empty(), "the re-read stands");
        owners.open();
    }
}
