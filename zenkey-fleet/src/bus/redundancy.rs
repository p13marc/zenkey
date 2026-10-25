//! `redundancy.v1` (#841, RC), the reads: who serves each governed
//! interface (`spec/profiles/redundancy/v1.md` §2.15), read as the
//! profile's tool reads it, through a session in the deployment's
//! namespace.
//!
//! What is read, in order:
//! 1. whether the session reaches another zenoh node (core §8.1, 0.37): a
//!    session cut off makes no read, and says why;
//! 2. two presence reads of the scope's `@zk/**` a grace apart (core §6),
//!    each with the descriptor of every instance it found (§3.3), on a
//!    session that has never held a liveliness subscriber and holds no
//!    claim of its own: a tool's session, which declares no token
//!    (redundancy.v1 §2.5 step 1, §2.15; the runtime refuses a read on a
//!    watch session). Each carries its completeness, core §8.1's (0.37,
//!    0.38, 0.40);
//! 3. the revisions the descriptors of either read name, retrieved from
//!    their holders (§8.4) or held offline (`--contracts`).
//!
//! Across a constrained face (§2.13) nothing crosses: nothing is read, and
//! the deployment's word names the address and the interface.
//!
//! Nothing is judged here: [`crate::judge::redundancy`] answers every
//! question from the [`RedundancyObservation`] this returns.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{Addr, IfaceId};
use zenoh::Session;

use crate::bus::contracts::BundleStore;
use crate::bus::presence::Scope;
use crate::model::catalog::{Catalog, Observed};
use crate::model::redundancy::Contracts;
use crate::report::RedundancyReport;

/// The grace between the two reads, by default: core §6's, above an
/// owner's re-mint overlap (under 1 s), as the scenarios' 2 s.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(2);

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct RedundancySpec {
    /// Each read's timeout: presence, a descriptor, a retrieval.
    pub timeout: Duration,
    /// How far apart the two reads are taken (core §6).
    pub grace: Duration,
    /// Only this interface; `None`: every governed one in scope.
    pub iface: Option<IfaceId>,
    /// The address sits across a constrained face (§2.13), on the
    /// deployment's word: nothing is read.
    pub across_face: bool,
}

impl RedundancySpec {
    /// Every governed interface, the default grace.
    pub fn new(timeout: Duration) -> RedundancySpec {
        RedundancySpec {
            timeout,
            grace: DEFAULT_GRACE,
            iface: None,
            across_face: false,
        }
    }
}

/// What a reading read, as values.
#[derive(Debug, Clone, Default)]
pub struct RedundancyObservation {
    /// Why no read was made: the session reaches no other zenoh node.
    pub cut_off: Option<String>,
    /// The first read, with its descriptors; `None` when not made.
    pub first: Option<Result<Observed, String>>,
    /// The second, a grace later.
    pub second: Option<Result<Observed, String>>,
    /// How far apart the two reads began.
    pub apart: Duration,
    /// The revisions either read's descriptors name.
    pub contracts: Contracts,
    /// The selectors put to the bus.
    pub asked: Vec<String>,
}

/// One reading (the module doc), judged. `store` holds the revisions the
/// caller has offline already, seeded once by it (#862's review: zenctl's
/// `zk2::store`, zengui's window store), so the reading asks the bus only
/// for the others.
pub async fn run_redundancy(
    session: &Session,
    namespace: &str,
    target: Option<Addr>,
    spec: RedundancySpec,
    store: &BundleStore,
) -> RedundancyReport {
    let obs = observe(session, target.as_ref(), &spec, store).await;
    crate::judge::redundancy::judge(namespace, target.as_ref(), &spec, &obs)
}

/// The reads of the module doc, as values.
pub async fn observe(
    session: &Session,
    target: Option<&Addr>,
    spec: &RedundancySpec,
    store: &BundleStore,
) -> RedundancyObservation {
    let mut obs = RedundancyObservation::default();
    if spec.across_face {
        return obs;
    }
    if let Some(why) = crate::bus::presence::cut_off(session).await {
        obs.cut_off = Some(why);
        return obs;
    }
    let scope = target.map_or_else(Scope::all, Scope::service);
    obs.asked.push(scope.selector());
    let t = spec.timeout;
    let read = || async {
        crate::bus::presence::observe(session, &scope, t)
            .await
            .map_err(|e| crate::one_line(&e))
    };
    let began = Instant::now();
    obs.first = Some(read().await);
    tokio::time::sleep_until((began + spec.grace).into()).await;
    let second_began = Instant::now();
    obs.second = Some(read().await);
    obs.apart = second_began - began;
    let mut wanted: BTreeSet<(IfaceId, Fingerprint)> = BTreeSet::new();
    for o in [&obs.first, &obs.second].into_iter().flatten().flatten() {
        wanted.extend(Catalog::new(o).wanted());
    }
    let wanted: Vec<(IfaceId, Fingerprint)> = wanted.into_iter().collect();
    let got = store.fetch_all(session, &wanted).await;
    obs.contracts = wanted
        .into_iter()
        .zip(got.into_iter().map(|g| g.map_err(|e| crate::one_line(&e))))
        .collect();
    obs
}
