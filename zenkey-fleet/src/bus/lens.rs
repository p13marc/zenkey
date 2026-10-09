//! What a raw observer's lens is read from (#612, FJ8b): one presence read
//! in the deployment's namespace, every instance's descriptor, and every
//! revision those descriptors name, retrieved into a [`BundleStore`]
//! (spec §8.1, §3.3, §8.4).
//!
//! [`read`] makes it once, for a windowed verb (`rate`, `field`,
//! `timeline`, `snapshot`, `check expect`); a [`LensFeed`] keeps it current
//! for a stream that runs until stopped (`echo`), re-reading in the
//! background when a key falls short of a provider or a revision — never
//! inside the drain, whose broadcast would overflow while it waited (#338).
//!
//! The session is the caller's, **in** the namespace: the presence and
//! contract keys are base-relative (decided 2026-10-08). The observer's
//! own subscription stays on a session in no namespace, which is what lets
//! it see a key outside the namespace and say so (O1, O3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use zenoh::Session;

use crate::Result;
use crate::bus::contracts::BundleStore;
use crate::bus::presence::{Scope, observe};
use crate::model::catalog::Catalog;
use crate::report::Unresolved;

/// How long a feed waits between two background re-reads, however often it
/// is nudged: a decode loop must not turn every unresolved key into a
/// presence read.
pub const REFRESH_MIN: Duration = Duration::from_secs(5);

/// Every service in the namespace from its tokens and descriptors, and every
/// revision the descriptors name retrieved into `store`. A retrieval that
/// failed is the store's to say (unavailable, or not held): the catalog is
/// returned either way.
pub async fn read(session: &Session, store: &BundleStore, timeout: Duration) -> Result<Catalog> {
    let observed = observe(session, &Scope::all(), timeout).await?;
    let catalog = Catalog::new(&observed);
    let _ = store.fetch_all(session, &catalog.wanted()).await;
    Ok(catalog)
}

/// A lens's presence read kept current in the background.
#[derive(Clone)]
pub struct LensFeed {
    session: Session,
    store: BundleStore,
    timeout: Duration,
    current: Arc<ArcSwapOption<Catalog>>,
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
        let (catalog, error) = match read(session, store, timeout).await {
            Ok(c) => (Some(Arc::new(c)), None),
            Err(e) => (None, Some(e)),
        };
        let feed = LensFeed {
            session: session.clone(),
            store: store.clone(),
            timeout,
            current: Arc::new(ArcSwapOption::new(catalog)),
            refreshing: Arc::new(AtomicBool::new(false)),
            last: Arc::new(std::sync::Mutex::new(tokio::time::Instant::now())),
        };
        (feed, error)
    }

    /// The latest catalog; `None` when no read has succeeded.
    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        self.current.load_full()
    }

    /// The store the feed retrieves revisions into.
    pub fn store(&self) -> &BundleStore {
        &self.store
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
            if let Ok(c) = read(&feed.session, &feed.store, feed.timeout).await {
                feed.current.store(Some(Arc::new(c)));
            }
            feed.refreshing.store(false, Ordering::Release);
        });
    }
}
