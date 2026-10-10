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
//! The two never interleave: each holds the feed's read for its whole run.
//! [`LensFeed::generation`] moves whenever either swaps the catalog or the
//! store's answers change, which is what an
//! [`IdentityCache`](crate::IdentityCache) keys its resolutions on.
//!
//! The session is the caller's, **in** the namespace: the presence and
//! contract keys are base-relative (decided 2026-10-08). The observer's
//! own subscription stays on a session in no namespace, which is what lets
//! it see a key outside the namespace and say so (O1, O3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::IfaceId;
use zenoh::Session;

use crate::Result;
use crate::bus::contracts::BundleStore;
use crate::bus::presence::{Scope, describe_each, observe};
use crate::model::catalog::{Catalog, Observed};
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

/// A lens's presence read kept current: following presence, and re-read in
/// the background when nudged.
#[derive(Clone)]
pub struct LensFeed {
    session: Session,
    store: BundleStore,
    timeout: Duration,
    current: Arc<ArcSwapOption<Catalog>>,
    /// The read the catalog is built from, held for the whole of a patch or
    /// a re-read so the two never interleave.
    observed: Arc<tokio::sync::Mutex<Option<Observed>>>,
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
            observed: Arc::new(tokio::sync::Mutex::new(observed)),
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

    /// Fold presence moves into the feed's read — the transitions a
    /// [`crate::PresenceFeed`] hands out, a seed's included — describe every
    /// instance that now holds its instance token and was never described
    /// here (each alone, §3.3), retrieve the revisions those descriptors
    /// name (§8.4), and swap the catalog. Returns whether the catalog was
    /// swapped; a batch that moved no token (statuses alone) swaps nothing.
    ///
    /// An instance that appears resolves at once, its tokenless interfaces
    /// (U22) among them, rather than after the next re-read; one that leaves
    /// takes its instance token and its descriptor with it. A feed whose
    /// first read failed starts a read from these moves alone, marked
    /// possibly incomplete (§8.1): what presence has not moved is not in it.
    /// Waits for a background re-read in flight, and the re-read for it.
    pub async fn on_presence(&self, moved: &[PresenceTransition]) -> bool {
        let mut guard = self.observed.lock().await;
        let observed = guard.get_or_insert_with(|| {
            let mut o = Observed::from_keys(Scope::all().selector(), &[], false);
            o.descriptors = Some(Default::default());
            o
        });
        if !observed.follow(moved) {
            return false;
        }
        let owed = observed.undescribed();
        if !owed.is_empty() {
            let found = describe_each(&self.session, owed, self.timeout).await;
            let mut wanted = Vec::new();
            for ((addr, instance), read) in found {
                if let Some(d) = read.descriptor() {
                    wanted.extend(d.interfaces.iter().filter_map(|e| {
                        Some((
                            e.iface.parse::<IfaceId>().ok()?,
                            Fingerprint::parse(&e.contract).ok()?,
                        ))
                    }));
                }
                observed.with_descriptor(&addr, &instance, read);
            }
            let _ = self.store.fetch_all(&self.session, &wanted).await;
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
                *guard = Some(o);
            }
            drop(guard);
            feed.refreshing.store(false, Ordering::Release);
        });
    }

    fn swap(&self, catalog: Catalog) {
        self.current.store(Some(Arc::new(catalog)));
        self.swaps.fetch_add(1, Ordering::AcqRel);
    }
}
