//! What fleet the window is pointed at, and everything known *about* that
//! fleet rather than observed from it.
//!
//! `settings` lives here, and that placement is the load-bearing one: of 82
//! reads of it, 41 are `.base` and 14 are `.timeout()`, and `apply_context`
//! rewrites seven of its thirteen fields as a single act — which is exactly
//! what a deployment change is. The four values that are genuinely *not*
//! deployment state (the connection endpoints, `max_keys`, `registry`) are
//! launch-time bounds: two are consumed at construction and never read again,
//! and four read sites in total is not worth a third type. Stated here rather
//! than hidden.
//!
//! [`Deployment::pointed_at`] is the reset, and it is a struct *replacement*
//! rather than a list of assignments. That inverts #179's and #194's failure
//! mode: a field added below is cleared by default, so forgetting to think
//! about it is now the safe side.

use std::sync::Arc;
use std::time::Duration;

use zenkey_fleet::{DiscoveredBase, Skeleton, SliceSet};

use crate::config::Settings;
use crate::view::status::SliceSource;

sub_state! {
    pub(crate) struct Deployment {
        pub(crate) settings: Settings,
        pub(crate) session: Option<zenoh::Session>,
        /// Process-lifetime schema cache (RFC 08 §7), rebuilt on base change.
        pub(crate) schema_store: Option<Arc<zenkey_fleet::decode::SchemaStore>>,
        pub(crate) skeleton: Option<Arc<Skeleton>>,
        pub(crate) slices: Option<Arc<SliceSet>>,
        pub(crate) slice_source: SliceSource,

        pub(crate) bases: Vec<DiscoveredBase>,

        /// The base picker's option list, rebuilt when `bases` changes rather
        /// than on every frame (#178). The empty string leads: it is a
        /// deployment — the bus root — not a "no selection" placeholder.
        pub(crate) base_options: Vec<String>,
        /// Bounded, and it says what the bound costs (#107).
        pub(crate) facts: zenkey_fleet::FactsCache,
    }
}

impl Deployment {
    pub(crate) fn new(settings: Settings) -> Deployment {
        let max_keys = settings.max_keys;
        Deployment {
            settings,
            session: None,
            schema_store: None,
            skeleton: None,
            slices: None,
            slice_source: SliceSource::None,
            bases: Vec::new(),
            // The empty string leads: it is a deployment — the bus root — not
            // a "no selection" placeholder.
            base_options: vec![String::new()],
            // The projection cache takes the *same* bound as the engine's key
            // table: it cannot usefully outgrow the table it shadows, and one
            // number keeps that one sentence (#107).
            facts: zenkey_fleet::FactsCache::with_capacity(max_keys),
        }
    }

    pub(crate) fn base(&self) -> &str {
        &self.settings.base
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.settings.timeout()
    }

    pub(crate) fn base_label(&self) -> &str {
        self.settings.base_label()
    }

    /// Forget this fleet, keeping only what points at the *next* one.
    ///
    /// The three carried forward are the whole of the old 43-entry `KEPT`
    /// list that belonged to this group: the settings (whose `base` the
    /// caller has already rewritten), the session, and the schema store —
    /// none of which is evidence *about* a fleet, all of which is machinery
    /// for reaching one.
    pub(crate) fn pointed_at(&mut self) {
        let session = self.session.take();
        let schema_store = self.schema_store.take();
        let settings = self.settings.clone();
        *self = Deployment::new(settings);
        self.session = session;
        self.schema_store = schema_store;
    }
}
