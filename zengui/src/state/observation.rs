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

/// What every seed boundary this session has closed contributed — zk2's
/// late joiner (core §4.2; #614, FL6b), leg by leg.
///
/// Named fields, because as `(usize, usize, u64)` the first two were written
/// as `history_replies`/`storage_replies` and read as `cache`/`storage` — two
/// `usize`s whose names already disagreed at the two ends, so a transposition
/// compiled and misreported coverage (#357). The names are now the legs'
/// own, at both ends: `SeedCoverage { history, state, superseded }` in, the
/// strip's "h history · s state · k superseded" out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeedTotals {
    /// Replies from the history leg: the `@adv` caches of the advanced
    /// publishers present (core §2.5).
    pub history: usize,
    /// Replies from the state leg: S4's GET on the watched selector, target
    /// `All` and consolidation `Latest` — answered by each key's owner, and
    /// for a foreign key by whatever holds it. Never a storage read: a
    /// storage on an owner's state is a deployment fault (S4).
    pub state: usize,
    /// Seed legs that did not run to their final reply — a GET that reached
    /// its timeout (`SeedLeg::complete`), so what it seeded is possibly
    /// incomplete (core §4.2, §8.1). Zero is "every leg ran to its end".
    pub incomplete: usize,
    /// Samples a newer stamp superseded during the merge.
    pub superseded: u64,
    /// Keys the merges forgot to stay within their bound, the monitor's
    /// total as the last tick read it ([`crate::message::BusTick::merge_evicted`]),
    /// never folded from a boundary: the merge runs for each watch's life
    /// (observer §4.1).
    pub evicted: u64,
}

impl SeedTotals {
    /// Fold one seed boundary's coverage in: each leg that ran adds its
    /// replies, and is counted when it ran short of its final reply; a leg
    /// switched off adds nothing (it was not asked).
    pub fn add(&mut self, coverage: &zenkey_fleet::SeedCoverage) {
        if let Some(leg) = coverage.history.get() {
            self.history += leg.replies;
            self.incomplete += usize::from(!leg.complete);
        }
        if let Some(leg) = coverage.state.get() {
            self.state += leg.replies;
            self.incomplete += usize::from(!leg.complete);
        }
        self.superseded += coverage.superseded;
    }
}

sub_state! {
    pub(crate) struct Observation {
        pub(crate) monitor: Option<Arc<Monitor>>,
        /// Bumped per monitor generation — the pump's identity.
        pub(crate) epoch: u64,
        pub(crate) link: LinkState,
        /// The presence monitor (#614, FL6a): its own, on the session in the
        /// namespace, watching nothing but the token selectors — so the data
        /// pump's `Dropped(n)` can never knock presence to lost. `None` until
        /// a namespaced session exists.
        pub(crate) presence_monitor: Option<Arc<zenkey_fleet::Monitor>>,
        /// Bumped whenever the presence link must restart — a new presence
        /// monitor, or a new contract store under the lens: the link's
        /// identity, and the epoch every presence landing carries.
        pub(crate) presence_epoch: u64,

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
        /// Cumulative seed coverage since connect, leg by leg — zeros are
        /// observations (O4).
        pub(crate) seed_totals: SeedTotals,
        /// Seed phases completed since connect.
        pub(crate) seeded_watches: usize,
        pub(crate) keys: usize,
        pub(crate) keys_evicted: u64,
        pub(crate) keys_unwatched: u64,
        pub(crate) totals: crate::message::WatchedTotals,
        /// The monitor's retained window, as of the last live tick (#217):
        /// the budget the status strip states, and the ring's own costs —
        /// counted apart from `keys_evicted`, `keys_unwatched` and the
        /// broadcast's lag (tooling guide O6). `None` until a monitor exists,
        /// which is not "empty" (O4) — and left stale during replay, when
        /// the strip does not show it.
        pub(crate) retention: Option<zenkey_fleet::RetentionStats>,
        /// The population budget judgement (#221; zk2's since #614, FL7):
        /// each templated resource's bound against the tree's window,
        /// through the core's judge, recomputed on a throttled cadence.
        /// `None` = not computed yet, which draws no badge — never a pass.
        pub(crate) budgets: Option<Arc<crate::budget::BudgetBadges>>,
        /// When this window first saw each active watch selector (#614,
        /// FL7): where a budget window over a family begins. Kept on live
        /// ticks only — a replayed watch set describes another world.
        pub(crate) watch_since: std::collections::BTreeMap<String, std::time::Instant>,
        /// The longest gap between two live ticks (#614, FL7): the
        /// window's reader stall against core §2.7's bound (0.27).
        pub(crate) tick_gap: crate::budget::TickGap,
    }
}

impl Default for Observation {
    fn default() -> Observation {
        Observation {
            monitor: None,
            epoch: 0,
            presence_monitor: None,
            presence_epoch: 0,
            link: LinkState::Connecting,
            observed: Arc::new(KeyTreeSnapshot::default()),
            watched: std::sync::Arc::from([] as [String; 0]),
            my_watches: HashMap::new(),
            my_watch_paths: BTreeSet::new(),
            scope_watches: Vec::new(),
            seeding: HashMap::new(),
            seeding_paths: BTreeSet::new(),
            seed_totals: SeedTotals::default(),
            seeded_watches: 0,
            keys: 0,
            keys_evicted: 0,
            keys_unwatched: 0,
            totals: crate::message::WatchedTotals::default(),
            retention: None,
            budgets: None,
            watch_since: std::collections::BTreeMap::new(),
            tick_gap: crate::budget::TickGap::default(),
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
        self.seed_totals = SeedTotals::default();
        self.seeded_watches = 0;
        // The ring's account rides with the coverage (#217): it describes
        // what the departing monitor retained for the departing watches, and
        // "not asked yet" is the honest strip line until the new one ticks.
        self.retention = None;
        // The budget join described the departing fleet's population (#221),
        // over windows the departing watches opened.
        self.budgets = None;
        self.watch_since.clear();
        self.tick_gap = crate::budget::TickGap::default();
    }
}
