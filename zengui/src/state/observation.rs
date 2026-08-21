//! What the bus has actually said, and what this window asked it to keep
//! saying.
//!
//! The split against [`super::deployment`] is declared-versus-observed: a
//! skeleton is what a registry *claims* exists, and everything here is what
//! arrived. `epoch` sits beside `monitor` because `LinkKey { monitor, epoch }`
//! is one value — the old field comment filed it under "the window and the
//! process", which was already wrong: an epoch is a monitor generation, and a
//! monitor is a deployment's pump.
//!
//! A base change does not *forget* this group so much as rebuild it: the
//! monitor restarts, and `observed`, `watched` and the key counters are
//! replaced on the way. What it does clear is the coverage — the watches this
//! window declared against a fleet it is no longer pointed at.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use zenkey_fleet::{KeyTreeSnapshot, Monitor, WatchId};

use crate::message::LinkState;

sub_state! {
    pub(crate) struct Observation {
        pub(crate) monitor: Option<Arc<Monitor>>,
        /// Bumped per monitor generation — the pump's identity.
        pub(crate) epoch: u64,
        pub(crate) link: LinkState,

        pub(crate) observed: Arc<KeyTreeSnapshot>,
        /// Active watch selectors, as the last tick reported them (coverage, O5).
        pub(crate) watched: std::sync::Arc<[String]>,
        /// This app's per-subtree watches: display path → watch id.
        pub(crate) my_watches: HashMap<String, WatchId>,
        /// The same key set, cached for the view (a borrowed pane cannot borrow
        /// a per-frame local).
        pub(crate) my_watch_paths: BTreeSet<String>,
        /// The scope-preset watch ids, when "observe scope" is on.
        pub(crate) scope_watches: Vec<WatchId>,
        /// Watches whose seed phase has not resolved yet (issue #92):
        /// id → the tree display path when the watch came from a tree toggle
        /// (`None` for scope watches, whose selectors are not tree paths).
        pub(crate) seeding: HashMap<WatchId, Option<String>>,
        /// The same tree paths as a set, for the tree's "seeding…" badges.
        pub(crate) seeding_paths: BTreeSet<String>,
        /// Cumulative seed coverage since connect: (cache replies, storage
        /// replies, superseded) — zeros are observations (O4).
        pub(crate) seed_totals: (usize, usize, u64),
        /// Seed phases completed since connect.
        pub(crate) seeded_watches: usize,
        pub(crate) keys: usize,
        pub(crate) keys_evicted: u64,
        pub(crate) keys_unwatched: u64,
        pub(crate) totals: (u64, u64, f64),
    }
}

impl Default for Observation {
    fn default() -> Observation {
        Observation {
            monitor: None,
            epoch: 0,
            link: LinkState::Connecting,
            observed: Arc::new(KeyTreeSnapshot::default()),
            watched: std::sync::Arc::from([] as [String; 0]),
            my_watches: HashMap::new(),
            my_watch_paths: BTreeSet::new(),
            scope_watches: Vec::new(),
            seeding: HashMap::new(),
            seeding_paths: BTreeSet::new(),
            seed_totals: (0, 0, 0),
            seeded_watches: 0,
            keys: 0,
            keys_evicted: 0,
            keys_unwatched: 0,
            totals: (0, 0, 0.0),
        }
    }
}

impl Observation {
    /// Drop the coverage this window declared, keeping the pump.
    ///
    /// The watch *ids* are dropped rather than released: a base change
    /// restarts the monitor, and the ids belong to the monitor that is going
    /// away. Releasing them against the new one would be asking a stranger to
    /// forget something it never knew.
    pub(crate) fn forget_coverage(&mut self) {
        self.my_watches.clear();
        self.my_watch_paths.clear();
        self.scope_watches.clear();
        self.seeding.clear();
        self.seeding_paths.clear();
        self.seed_totals = (0, 0, 0);
        self.seeded_watches = 0;
    }
}
