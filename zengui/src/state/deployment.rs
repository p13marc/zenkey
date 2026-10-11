//! What fleet the window is pointed at, and everything known *about* that
//! fleet rather than observed from it.
//!
//! `settings` lives here, and that placement is the load-bearing one: most
//! reads of it are `.namespace` and `.timeout()`, and `apply_context`
//! rewrites seven of its fields as a single act — which is exactly what a
//! deployment change is. The values that are genuinely *not* deployment
//! state (the connection endpoints, `max_keys`, `contracts`) are launch-time
//! bounds or user input, and four read sites in total is not worth a third
//! type. Stated here rather than hidden.
//!
//! **Two sessions** since #614 (FL6a): `raw`, in no namespace, carries the
//! data pump's wire selectors and the admin space; `session`, in the
//! deployment's namespace, reads presence, descriptors and contracts the way
//! the deployment's own consumers do. A namespace switch reopens only the
//! second — and with it the presence projection, the lens feed and its
//! catalog, which live here because they are what the window knows *about*
//! the deployment it points at.
//!
//! [`Deployment::pointed_at`] is the reset, and it is a struct *replacement*
//! rather than a list of assignments. That inverts #179's and #194's failure
//! mode: a field added below is cleared by default, so forgetting to think
//! about it is now the safe side.

use std::sync::Arc;
use std::time::Duration;

use zenkey_fleet::{
    BundleStore, Catalog, ContractSet, DeclaredEntities, IdentityCache, NamespaceListing, Skeleton,
};

use crate::config::{NamespaceChoice, Settings};
use crate::presence_link::{LensHandle, PresenceUpdate};
use crate::services::contracts::LoadedContracts;
use crate::services::value::LensInputs;

/// What a skeleton was built from (#614, FL6b), held so a refresh can ask
/// whether any of it moved: the namespace and the lens's generation by
/// value, the catalog, the contracts and the admin answer by identity.
///
/// The `Arc`s are held, not their addresses: a held allocation cannot be
/// freed and reused by a different value, so pointer equality here is
/// exact. The contract store is not among them — it moves only with the
/// generation, and without a catalog the skeleton places the offline set
/// alone and never consults it.
#[derive(Debug, Clone)]
pub(crate) struct SkeletonInputs {
    namespace: String,
    generation: u64,
    catalog: Option<Arc<Catalog>>,
    contracts: Option<Arc<ContractSet>>,
    admin: Option<Arc<DeclaredEntities>>,
}

impl PartialEq for SkeletonInputs {
    fn eq(&self, other: &SkeletonInputs) -> bool {
        fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        self.namespace == other.namespace
            && self.generation == other.generation
            && same(&self.catalog, &other.catalog)
            && same(&self.contracts, &other.contracts)
            && same(&self.admin, &other.admin)
    }
}

/// What the contract store holds, counted when it can change rather than per
/// frame — the status strip's contracts cell (#614, FL6a).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContractCounts {
    /// Revisions loaded offline (`--contracts`).
    pub offline: usize,
    /// Revisions retrieved from a holder on the bus (§8.4).
    pub retrieved: usize,
    /// Revisions a descriptor names that no holder served (§8.4's
    /// *unavailable*): asked, and not found — never "not asked".
    pub unavailable: usize,
    /// Revisions that verified and do not read.
    pub unreadable: usize,
}

sub_state! {
    pub(crate) struct Deployment {
        pub(crate) settings: Settings,
        /// The session in **no** namespace (#614, FL6a): the data pump, the
        /// wire selectors, the admin space — and every v1 pane still on it.
        pub(crate) raw: Option<zenoh::Session>,
        /// The session **in** the deployment's namespace (#614, FL6a):
        /// presence, descriptors, contracts. Reopened by a namespace switch.
        pub(crate) session: Option<zenoh::Session>,
        /// Process-lifetime v1 schema cache (RFC 08 §7), kept for the v1
        /// panes still alive — why, fields, the request scaffold and the
        /// doctor's re-ask — until FL8, FL9 and FL10 move them.
        pub(crate) schema_store: Option<Arc<zenkey_fleet_v1::SchemaStore>>,
        /// The declared keyspace the tree starts from (#614, FL6b): zk2's
        /// contract skeleton, built from the lens's own inputs — the
        /// namespace, the catalog, the store and the contracts held offline
        /// — and the admin space's declared entities. Session-free, so it is
        /// built here on the update thread, and rebuilt only when what it
        /// was built from moved ([`Deployment::refresh_skeleton`]). `None` =
        /// not built yet.
        pub(crate) skeleton: Option<Arc<Skeleton>>,
        /// What [`Self::skeleton`] was built from, compared by identity on
        /// every refresh: the one test of "the lens moved".
        pub(crate) skeleton_inputs: Option<SkeletonInputs>,
        /// The admin space's declared entities, read on the session in no
        /// namespace (`services::sweep::declared`). `None` = not asked yet,
        /// or no admin space answered — never "nothing declared" (O4).
        pub(crate) admin_declared: Option<Arc<DeclaredEntities>>,
        /// The contracts held offline, as the last load gave them (#614,
        /// FL6a). `None` until a load lands, which is "not loaded" — never
        /// "none". User input, not evidence: kept across a namespace
        /// switch, reloaded on every session open.
        pub(crate) contracts: Option<Arc<LoadedContracts>>,
        /// The contract store (spec §8.4): seeded with `contracts`, filled
        /// by the lens feed's retrievals. Rebuilt whenever the contracts or
        /// the namespaced session change, so a revision dropped from
        /// `--contracts` does not linger.
        pub(crate) store: BundleStore,
        /// `store` counted, for the strip.
        pub(crate) contract_counts: ContractCounts,
        /// The namespace listing (`zenkey_fleet::namespace_listing`), read
        /// on the session in no namespace. `None` = not read yet (O4).
        pub(crate) namespaces: Option<Arc<NamespaceListing>>,
        /// The namespace picker's option list, rebuilt when `namespaces`
        /// changes rather than on every frame (#178). The bus root leads: it
        /// is a deployment — displayed by its label, never as a blank row
        /// (#185) — not a "no selection" placeholder.
        pub(crate) namespace_options: Vec<NamespaceChoice>,
        /// The last presence landing (#614, FL6a): each address up, down or
        /// unobservable, on the basis its seed allows. `None` = presence not
        /// read yet, never "nobody there" (O4).
        pub(crate) presence: Option<Arc<PresenceUpdate>>,
        /// The lens feed, once the presence link opened it: what a check
        /// nudges, and what each tick polls for a catalog a background
        /// re-read swapped in.
        pub(crate) lens_feed: Option<LensHandle>,
        /// The catalog the lens resolves addresses through — the feed's
        /// latest, pointer-compared each tick. `None` = no presence read
        /// stands, and every zk2 key says so at its rung.
        pub(crate) catalog: Option<Arc<Catalog>>,
        /// The generation `catalog` and `store` are at
        /// (`LensFeed::generation`): an identity or a verdict judged under
        /// another one is due again.
        pub(crate) lens_generation: u64,
        /// Each wire key's resolution through the lens, kept and bounded by
        /// the data pump's own key bound, every drop counted (O6). Fed per
        /// sample; FL7 draws its badges from it.
        pub(crate) identity: IdentityCache,
        /// Bounded, and it says what the bound costs (#107). v1's projection
        /// cache, fed no slices since #614 (FL6a): its badges read "not
        /// asked" until FL7 replaces it with zk2's identity cache.
        pub(crate) facts: zenkey_fleet_v1::FactsCache,
    }
}

impl Deployment {
    pub(crate) fn new(settings: Settings) -> Deployment {
        let max_keys = settings.max_keys;
        let timeout = settings.timeout();
        let mut dep = Deployment {
            settings,
            raw: None,
            session: None,
            schema_store: None,
            skeleton: None,
            skeleton_inputs: None,
            admin_declared: None,
            contracts: None,
            store: BundleStore::new(timeout),
            contract_counts: ContractCounts::default(),
            namespaces: None,
            namespace_options: Vec::new(),
            presence: None,
            lens_feed: None,
            catalog: None,
            lens_generation: 0,
            // Both caches take the *same* bound as the engine's key table:
            // neither can usefully outgrow the table it shadows, and one
            // number keeps that one sentence (#107).
            identity: IdentityCache::with_capacity(max_keys),
            facts: zenkey_fleet_v1::FactsCache::with_capacity(max_keys),
        };
        dep.rebuild_namespace_options();
        dep
    }

    pub(crate) fn namespace(&self) -> &str {
        &self.settings.namespace
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.settings.timeout()
    }

    pub(crate) fn namespace_label(&self) -> &str {
        self.settings.namespace_label()
    }

    /// Forget this fleet, keeping only what points at the *next* one.
    ///
    /// Carried forward: the settings (whose `namespace` the caller has
    /// already rewritten), the two sessions, the v1 schema store and the
    /// contracts — none of which is evidence *about* a fleet, all of which
    /// is machinery for reaching one, or the user's own input. The contract
    /// store is not carried: what it retrieved came off the departing
    /// deployment's holders, and a fresh one is seeded from the contracts.
    /// Neither is presence, the lens or its catalog: they described the
    /// deployment the window is leaving.
    pub(crate) fn pointed_at(&mut self) {
        let raw = self.raw.take();
        let session = self.session.take();
        let schema_store = self.schema_store.take();
        let contracts = self.contracts.take();
        let settings = self.settings.clone();
        *self = Deployment::new(settings);
        self.raw = raw;
        self.session = session;
        self.schema_store = schema_store;
        self.contracts = contracts;
        self.reseed_store();
    }

    /// A fresh contract store, seeded with the contracts held offline
    /// (§8.5): what a contracts load and every namespace move start from.
    /// The lens over the old store goes with it — its feed retrieves into
    /// the store it was opened on — and the presence link that opens the
    /// next one is restarted by the caller.
    pub(crate) fn reseed_store(&mut self) {
        let store = BundleStore::new(self.timeout());
        if let Some(c) = &self.contracts {
            store.seed(&c.set);
        }
        self.store = store;
        self.lens_feed = None;
        self.catalog = None;
        self.recount_contracts();
        // The lens's inputs moved — no catalog, maybe other contracts — and
        // the declared keyspace is built from exactly those.
        self.refresh_skeleton();
    }

    /// Rebuild the skeleton when what it is built from moved (#614, FL6b):
    /// the namespace, the lens's generation, its catalog, the contracts
    /// held offline, or the admin space's answer. `true` when it rebuilt.
    ///
    /// The one place the skeleton is built, and the reason it is not a
    /// task: `Skeleton::build` is session-free and costs one walk over the
    /// revisions placed — the same order of work as one flatten, paid on a
    /// lens move rather than per tick — so the tree never shows a skeleton
    /// built under one lens beside identities resolved under another.
    pub(crate) fn refresh_skeleton(&mut self) -> bool {
        let inputs = SkeletonInputs {
            namespace: self.settings.namespace.clone(),
            generation: self.lens_generation,
            catalog: self.catalog.clone(),
            contracts: self.contracts.as_ref().map(|c| Arc::clone(&c.set)),
            admin: self.admin_declared.clone(),
        };
        if self.skeleton.is_some() && self.skeleton_inputs.as_ref() == Some(&inputs) {
            return false;
        }
        let lens = self.lens_inputs();
        let skeleton = Skeleton::build(&lens.lens(), self.admin_declared.as_deref());
        self.skeleton = Some(Arc::new(skeleton));
        self.skeleton_inputs = Some(inputs);
        true
    }

    /// Count what the store holds, for the strip. Cheap enough per change
    /// (a bounded store, at most `DEFAULT_MAX_CONTRACTS` entries, and the
    /// catalog's wanted revisions), and never per frame.
    pub(crate) fn recount_contracts(&mut self) {
        use zenkey_fleet::report::ContractSource;
        use zenkey_fleet::{ContractState, Contracts as _};
        let offline = self.contracts.as_ref().map_or(0, |c| c.set.len());
        let retrieved = self
            .store
            .held()
            .iter()
            .filter(|r| r.source() == ContractSource::Bus)
            .count();
        let (mut unavailable, mut unreadable) = (0, 0);
        if let Some(catalog) = &self.catalog {
            for (iface, fp) in catalog.wanted() {
                match self.store.state(&iface, &fp) {
                    Some(ContractState::Unavailable { .. }) => unavailable += 1,
                    Some(ContractState::Unreadable { .. }) => unreadable += 1,
                    _ => {}
                }
            }
        }
        self.contract_counts = ContractCounts {
            offline,
            retrieved,
            unavailable,
            unreadable,
        };
    }

    /// What a lens is built from, owned, for a task (#614, FL6a).
    pub(crate) fn lens_inputs(&self) -> LensInputs {
        LensInputs {
            namespace: self.settings.namespace.clone(),
            catalog: self.catalog.clone(),
            store: self.store.clone(),
            contracts: self.contracts.as_ref().map(|c| Arc::clone(&c.set)),
            generation: self.lens_generation,
            feed: self.lens_feed.as_ref().map(|h| h.0.clone()),
        }
    }

    /// Resolve `wire` into the identity cache, through the lens over this
    /// deployment's inputs — the cache's one insert point. Split borrows:
    /// the lens reads the catalog, the store and the contracts while the
    /// cache is written.
    pub(crate) fn ensure_identity(&mut self, wire: &str) {
        let lens = zenkey_fleet::Lens::new(
            &self.settings.namespace,
            self.catalog.as_deref(),
            &self.store,
        )
        .at_generation(self.lens_generation);
        let lens = match &self.contracts {
            Some(c) => lens.offline(&c.set),
            None => lens,
        };
        self.identity.ensure(&lens, wire);
    }

    /// The namespace picker's options, after a listing landed or the
    /// namespace moved.
    ///
    /// The bus root leads — a deployment, not a placeholder — then every
    /// namespace the listing saw, then the one in force when the listing did
    /// not see it: a picker whose selected value is not among its own
    /// options is #187's defect. Dedup is set-semantics, whatever order the
    /// listing answered in.
    pub(crate) fn rebuild_namespace_options(&mut self) {
        let mut seen = std::collections::BTreeSet::new();
        let listed = self
            .namespaces
            .iter()
            .flat_map(|l| l.namespaces.iter().map(|n| n.namespace.clone()));
        self.namespace_options = std::iter::once(String::new())
            .chain(listed)
            .chain(std::iter::once(self.settings.namespace.clone()))
            .filter(|n| seen.insert(n.clone()))
            .map(NamespaceChoice::new)
            .collect();
    }
}
