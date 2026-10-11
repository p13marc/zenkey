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
//! second.
//!
//! [`Deployment::pointed_at`] is the reset, and it is a struct *replacement*
//! rather than a list of assignments. That inverts #179's and #194's failure
//! mode: a field added below is cleared by default, so forgetting to think
//! about it is now the safe side.

use std::sync::Arc;
use std::time::Duration;

use zenkey_fleet::{BundleStore, NamespaceListing};
use zenkey_fleet_v1::Skeleton;

use crate::config::{NamespaceChoice, Settings};
use crate::services::contracts::LoadedContracts;

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
        pub(crate) skeleton: Option<Arc<Skeleton>>,
        /// The contracts held offline, as the last load gave them (#614,
        /// FL6a). `None` until a load lands, which is "not loaded" — never
        /// "none". User input, not evidence: kept across a namespace
        /// switch, reloaded on every session open.
        pub(crate) contracts: Option<Arc<LoadedContracts>>,
        /// The contract store (spec §8.4): seeded with `contracts`, filled
        /// by retrieval. Rebuilt whenever the contracts or the namespaced
        /// session change, so a revision dropped from `--contracts` does not
        /// linger.
        pub(crate) store: Option<BundleStore>,
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
        /// Bounded, and it says what the bound costs (#107). v1's projection
        /// cache, fed no slices since #614 (FL6a): its badges read "not
        /// asked" until FL7 replaces it with zk2's identity cache.
        pub(crate) facts: zenkey_fleet_v1::FactsCache,
    }
}

impl Deployment {
    pub(crate) fn new(settings: Settings) -> Deployment {
        let max_keys = settings.max_keys;
        let mut dep = Deployment {
            settings,
            raw: None,
            session: None,
            schema_store: None,
            skeleton: None,
            contracts: None,
            store: None,
            contract_counts: ContractCounts::default(),
            namespaces: None,
            namespace_options: Vec::new(),
            // The projection cache takes the *same* bound as the engine's key
            // table: it cannot usefully outgrow the table it shadows, and one
            // number keeps that one sentence (#107).
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
    pub(crate) fn reseed_store(&mut self) {
        let store = BundleStore::new(self.timeout());
        if let Some(c) = &self.contracts {
            store.seed(&c.set);
        }
        self.store = Some(store);
        self.recount_contracts();
    }

    /// Count what the store holds, for the strip. Cheap enough per change
    /// (a bounded store, at most `DEFAULT_MAX_CONTRACTS` entries), and never
    /// per frame.
    pub(crate) fn recount_contracts(&mut self) {
        use zenkey_fleet::report::ContractSource;
        let offline = self.contracts.as_ref().map_or(0, |c| c.set.len());
        let retrieved = self.store.as_ref().map_or(0, |s| {
            s.held()
                .iter()
                .filter(|r| r.source() == ContractSource::Bus)
                .count()
        });
        self.contract_counts = ContractCounts {
            offline,
            retrieved,
            ..self.contract_counts
        };
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
