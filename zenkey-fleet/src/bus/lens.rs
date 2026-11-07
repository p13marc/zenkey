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
//! lost to the feed until then, or until its next full read.
//!
//! The two never interleave: each holds the feed's read for its whole run.
//! [`LensFeed::generation`] moves whenever either swaps the catalog or the
//! store's answers change, which is what an
//! [`IdentityCache`](crate::IdentityCache) keys its resolutions on.
//!
//! The session is the caller's, **in** the namespace: the presence and
//! contract keys are base-relative (decided 2026-10-08). The observer's
//! own subscription stays on a session in no namespace, which is what lets
//! it see a key outside the namespace and say so (O1, O3).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::{ArcSwap, ArcSwapOption};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, IfaceId, ZkKey};
use zenkey_model::wire::Run;
use zenoh::Session;

use crate::Result;
use crate::bus::contracts::BundleStore;
use crate::bus::monitor::SampleView;
use crate::bus::presence::{Scope, describe_each, observe};
use crate::model::catalog::{Catalog, KeptDescriptors, Observed};
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
    /// The addresses a put's run tells something of
    /// ([`KeptDescriptors::addresses`]), as each patch leaves them: what
    /// [`LensFeed::runs_in`] filters by without the read's lock.
    awaited: Arc<ArcSwap<BTreeSet<String>>>,
    /// Catalog swaps since the feed opened.
    swaps: Arc<AtomicU64>,
    refreshing: Arc<AtomicBool>,
    last: Arc<std::sync::Mutex<tokio::time::Instant>>,
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
        let catalog = observed.as_ref().map(|o| Arc::new(Catalog::new(o)));
        let feed = LensFeed {
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
        };
        (feed, error)
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
    /// reused descriptor a put under another run outran
    /// ([`LensFeed::on_runs`]). A re-read's are not counted.
    pub fn descriptors_read(&self) -> u64 {
        self.described.load(Ordering::Acquire)
    }

    /// How many times an instance token's return reused the descriptor the
    /// instance took with it (observer §3.11's MAY; #982 H3): each a
    /// descriptor GET not made across the hop. Waits for a patch or a
    /// re-read in flight.
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
    /// again, with no GET (observer §3.11's MAY; #982 H3). A feed whose
    /// first read failed starts a read from these moves alone, marked
    /// possibly incomplete (§8.1): what presence has not moved is not in it.
    /// Waits for a background re-read in flight, and the re-read for it.
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
            let found = describe_each(&self.session, owed, self.timeout).await;
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
    /// feed holds none. Hand it live samples: a GET's reply keeps the run it
    /// was put under.
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
            if !address_of(relative).is_some_and(|a| awaited.contains(a)) {
                continue;
            }
            let Ok(ZkKey::Data { addr, .. }) = zenkey_model::grammar::parse(relative) else {
                continue;
            };
            let Some(stamp) = s.inband() else {
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
    /// retrieved and the catalog swapped. A GET that serves no descriptor
    /// leaves the reused one held: a silent GET proves nothing (§3.3).
    /// Returns whether the catalog was swapped. Waits for a patch or a
    /// re-read in flight.
    pub async fn on_runs(&self, heard: &[(Addr, Run)]) -> bool {
        if heard.is_empty() {
            return false;
        }
        let mut guard = self.observed.lock().await;
        let FeedRead { observed, kept } = &mut *guard;
        let Some(observed) = observed.as_mut() else {
            return false;
        };
        let mut owed = Vec::new();
        for (addr, run) in heard {
            owed.extend(observed.heard_run(addr, *run, kept));
        }
        self.awaits(kept);
        if owed.is_empty() {
            return false;
        }
        self.described
            .fetch_add(owed.len() as u64, Ordering::AcqRel);
        let found = describe_each(&self.session, owed, self.timeout).await;
        let mut wanted = Vec::new();
        let mut replaced = false;
        for ((addr, instance), read) in found {
            if let Some(d) = read.descriptor() {
                wanted.extend(revisions_named(d));
                observed.with_descriptor(&addr, &instance, read);
                replaced = true;
            }
        }
        if !replaced {
            return false;
        }
        let _ = self.store.fetch_all(&self.session, &wanted).await;
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
    /// of, for [`LensFeed::runs_in`].
    fn awaits(&self, kept: &KeptDescriptors) {
        let now: BTreeSet<String> = kept.addresses().iter().map(ToString::to_string).collect();
        if **self.awaited.load() != now {
            self.awaited.store(Arc::new(now));
        }
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
    use super::address_of;

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
}
