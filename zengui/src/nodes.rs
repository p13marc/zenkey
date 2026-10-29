//! The nodes pane's model (#61; zk2's since #614, FL9): one row per
//! **address**, read from presence, never from a roster of its own.
//!
//! Three sources, each costing what it says:
//!
//! - **presence** — the [`LivePresence`] the presence link keeps current on
//!   its own monitor (`presence_link.rs`): each address up, down or
//!   unobservable, since when, its instances with theirs, its members. Zero
//!   payload, already in memory; this module holds none of it, it reads
//!   the window's;
//! - **descriptors** — what each instance says of itself (core §3.3): host,
//!   process, build, session zid, profiles, interfaces. Read off the lens's
//!   catalog, indexed once per catalog the feed swaps in
//!   ([`NodesState::describe`]), never per frame;
//! - **health** — `health.v1`, read lazily for the one address selected
//!   (the laziness ground rule, #84/#85), never polled: [`HealthState`];
//! - **redundancy** — `redundancy.v1`'s reading of the address selected
//!   (#841, RC), on selection too: two presence reads a grace apart, each
//!   instance's role on every governed interface the fleet's judge's,
//!   never derived here: [`RedundancyState`];
//! - **arbitration** — `arbitration.v1`'s reading of the address selected
//!   (#909, BC), when its descriptor requires a role, at the same settle:
//!   each arbitrated role's declaration and binding, and the selection of
//!   the tool's own view over a listen of two deadlines, every answer the
//!   fleet's judge's: [`ArbitrationState`].
//!
//! Plus the zero-cost join v1's roster made: for every address whose state
//! an active watch covers, the age of its newest state sample, read from
//! the tick's tree snapshot ([`NodesState::refresh`]).
//!
//! **The three states stay three** (spec §8.1, O5, R7): an address with no
//! instance token is *down* only under a complete seed; with no seed, an
//! incomplete one, or events lost since, it is *unobservable* — which never
//! reads as down, and "presence not read" never reads as an empty
//! deployment.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zenkey_fleet::report::{
    ArbitrationReport, Asked, DescriptorAnswer, HealthReport, RedundancyReport, RedundancyRow,
};
use zenkey_fleet::{Catalog, KeyTreeSnapshot, LivePresence};
use zenkey_model::grammar::{Addr, GRAMMAR, KindToken};
use zenoh::key_expr::keyexpr;

/// The profile whose listing in a descriptor makes a system minted
/// (`hostid.v1` §2.8) — and the only thing that does: never the shape of a
/// system's name (§2.11).
pub const HOSTID: &str = zenkey_model::hostid::PROFILE;

/// How old one address's state is, as far as this window can tell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Freshness {
    /// Whether an active watch covers the address's state — `state/**` or
    /// `@state/**` under any of its interfaces: what makes the age's
    /// absence legible.
    pub watched: bool,
    /// The age of the newest state sample under the address, when watched
    /// and one arrived. `None` = unknown, never "fresh" (O4).
    pub last_state_age: Option<Duration>,
}

/// What one instance's descriptor says (core §3.3), as the pane shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Described {
    /// The descriptor was not asked for: the catalog holds tokens alone.
    NotAsked,
    /// A descriptor that passes the syntax checks.
    Served(DescriptorFacts),
    /// A reply that fails the descriptor check, with its findings.
    Invalid(String),
    /// No reply within the timeout: not a verdict.
    Silent,
    /// The GET could not be put on the bus.
    Failed(String),
}

/// The informative half of a descriptor, plus what it implements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DescriptorFacts {
    /// `meta.host`, `meta.process`, `meta.build`, `meta.zid`: informative,
    /// never interpreted (§3.3); absent when the descriptor names none.
    pub host: Option<String>,
    pub process: Option<String>,
    pub build: Option<String>,
    pub zid: Option<String>,
    /// The profiles it lists (`<name>.v<major>`).
    pub profiles: Vec<String>,
    /// Each interface it implements, with the first chunk of the
    /// fingerprint it names.
    pub interfaces: Vec<(String, String)>,
    /// The interfaces its entries mark `"standby": true` (core §3.3, 0.39):
    /// the descriptor's own word, which `redundancy.v1` reads (#841, RC).
    pub standby: Vec<String>,
}

impl DescriptorFacts {
    /// Whether the descriptor lists `hostid.v1` (§2.8) — the hostid chip's
    /// one premise. A system *named* like a minted one says nothing (§2.11).
    pub fn lists_hostid(&self) -> bool {
        self.profiles.iter().any(|p| p == HOSTID)
    }

    /// Whether the descriptor lists `config.v1` in its interfaces — the
    /// gate the profile names (`config.v1` §2.1: "the interface is the
    /// gate"), and the Config tool's one premise (#768, CD).
    pub fn lists_config(&self) -> bool {
        self.interfaces
            .iter()
            .any(|(iface, _)| iface == zenkey_model::config::IFACE)
    }
}

/// One instance of an address, as its descriptor describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceFacts {
    /// 16 lowercase hex digits.
    pub instance: String,
    pub described: Described,
}

/// The catalog's descriptors, indexed by address — built once per catalog.
#[derive(Debug, Clone, Default)]
struct DescriptorIndex {
    /// The catalog the index was built from, held so a refresh can ask
    /// whether the feed swapped another in (pointer identity: a held `Arc`
    /// cannot be freed and reused).
    from: Option<Arc<Catalog>>,
    by_address: BTreeMap<Addr, Vec<InstanceFacts>>,
}

/// `health.v1` for the one address selected (D11): asked on selection,
/// never polled.
#[derive(Debug, Clone, Default)]
pub enum HealthState {
    /// Nothing selected yet, or the selection is not an address.
    #[default]
    NotAsked,
    /// The reading is in flight — two readings a grace apart.
    Asking(Addr),
    /// The engine's report, as `zenctl health <address>` prints it.
    Read(Addr, Arc<HealthReport>),
}

impl HealthState {
    /// The address this state is about, if any.
    pub fn address(&self) -> Option<&Addr> {
        match self {
            HealthState::NotAsked => None,
            HealthState::Asking(a) | HealthState::Read(a, _) => Some(a),
        }
    }
}

/// `redundancy.v1` for the one address selected (#841, RC): asked on
/// selection, never polled.
#[derive(Debug, Clone, Default)]
pub enum RedundancyState {
    /// Nothing selected yet, or the selection is not an address.
    #[default]
    NotAsked,
    /// Selected, the reading owed once the selection settles (#862's
    /// review): rapid selection queues no reading.
    Due(Addr),
    /// The reading is in flight — two presence reads a grace apart.
    Asking(Addr),
    /// The engine's report, as `zenctl redundancy <address>` prints it.
    Read(Addr, Arc<RedundancyReport>),
}

impl RedundancyState {
    /// The address this state is about, if any.
    pub fn address(&self) -> Option<&Addr> {
        match self {
            RedundancyState::NotAsked => None,
            RedundancyState::Due(a) | RedundancyState::Asking(a) | RedundancyState::Read(a, _) => {
                Some(a)
            }
        }
    }

    /// The reading of `addr`, when it is the one landed.
    pub fn report(&self, addr: &Addr) -> Option<&RedundancyReport> {
        match self {
            RedundancyState::Read(a, r) if a == addr => Some(r),
            _ => None,
        }
    }

    /// Each governed interface's row of `addr` holding `instance`, with
    /// that instance's entry: its role and its fourth answer, as the
    /// fleet's judge gave them.
    pub fn roles<'r>(
        &'r self,
        addr: &Addr,
        instance: &str,
    ) -> Vec<(
        &'r RedundancyRow,
        &'r zenkey_fleet::report::RedundancyInstance,
    )> {
        self.report(addr)
            .into_iter()
            .flat_map(|r| r.interfaces.iter())
            .filter_map(|row| {
                row.instances
                    .iter()
                    .find(|i| i.instance == instance)
                    .map(|i| (row, i))
            })
            .collect()
    }
}

/// `arbitration.v1` for the one address selected (#909, BC): asked at the
/// selection's settle, beside `redundancy.v1`, when a descriptor of the
/// address requires a role; never polled.
#[derive(Debug, Clone, Default)]
pub enum ArbitrationState {
    /// Nothing selected, or the address requires no role.
    #[default]
    NotAsked,
    /// Selected, requiring a role: the reading owed once the selection
    /// settles.
    Due(Addr),
    /// The reading is in flight: the descriptor, the contracts, and the
    /// tool's own view listening two deadlines.
    Asking(Addr),
    /// The engine's report, as `zenctl arbitration <address>` prints it.
    Read(Addr, Arc<ArbitrationReport>),
}

impl ArbitrationState {
    /// The address this state is about, if any.
    pub fn address(&self) -> Option<&Addr> {
        match self {
            ArbitrationState::NotAsked => None,
            ArbitrationState::Due(a)
            | ArbitrationState::Asking(a)
            | ArbitrationState::Read(a, _) => Some(a),
        }
    }

    /// The reading of `addr`, when it is the one landed.
    pub fn report(&self, addr: &Addr) -> Option<&ArbitrationReport> {
        match self {
            ArbitrationState::Read(a, r) if a == addr => Some(r),
            _ => None,
        }
    }
}

/// The pane's whole state.
#[derive(Debug, Default)]
pub struct NodesState {
    described: DescriptorIndex,
    freshness: BTreeMap<Addr, Freshness>,
    /// `health.v1` for the address selected.
    pub health: HealthState,
    /// `redundancy.v1` for the address selected (#841, RC).
    pub redundancy: RedundancyState,
    /// Counts the selections a reading was owed for: only the latest one's
    /// settle asks (#862's review).
    pub redundancy_due: u64,
    /// `arbitration.v1` for the address selected (#909, BC), asked at the
    /// same settle.
    pub arbitration: ArbitrationState,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1). Off by default
    /// (D11): without it no reply's age can be judged, and health reads
    /// "age unobservable" rather than a level. User input, kept across a
    /// forget, like the doctor's `deep`.
    pub clocks_synced: bool,
}

impl NodesState {
    /// Index the catalog's descriptors, when the feed swapped another
    /// catalog in. A pointer compare when it did not — cheap enough per
    /// tick; one `Catalog::services` walk when it did, which is the same
    /// listing `Catalog::service(addr)` takes for one address, taken once
    /// for all of them.
    pub fn describe(&mut self, catalog: Option<&Arc<Catalog>>) {
        let same = match (catalog, &self.described.from) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let mut by_address = BTreeMap::new();
        if let Some(catalog) = catalog {
            for s in catalog.services().services {
                let Ok(addr) = s.address.parse::<Addr>() else {
                    continue;
                };
                let instances = s
                    .instances
                    .into_iter()
                    .map(|i| InstanceFacts {
                        instance: i.instance,
                        described: described(&i.descriptor),
                    })
                    .collect();
                by_address.insert(addr, instances);
            }
        }
        self.described = DescriptorIndex {
            from: catalog.cloned(),
            by_address,
        };
    }

    /// What `addr`'s instances' descriptors say; `None` when the catalog
    /// holds no record of it (no presence read, or not in it).
    pub fn instances(&self, addr: &Addr) -> Option<&[InstanceFacts]> {
        self.described.by_address.get(addr).map(Vec::as_slice)
    }

    /// Whether a served descriptor of `addr` lists `config.v1` (§2.1): what
    /// earns its card and its Inspector view a "configure" (#768, CD).
    pub fn lists_config(&self, addr: &Addr) -> bool {
        self.instances(addr).is_some_and(|all| {
            all.iter()
                .any(|i| matches!(&i.described, Described::Served(f) if f.lists_config()))
        })
    }

    /// The watched-freshness join for one address.
    pub fn freshness(&self, addr: &Addr) -> Freshness {
        self.freshness.get(addr).copied().unwrap_or_default()
    }

    /// The watched-only freshness join, per live tick: for each address
    /// presence holds, whether a watch covers its state, and if so the age
    /// of its newest state sample in the tick's tree. Purely in memory.
    ///
    /// `namespace` is the deployment's: the tree holds wire keys, and an
    /// address's state lives under `<namespace>/zk2/<system>/<service>`.
    pub fn refresh(
        &mut self,
        presence: Option<&LivePresence>,
        tree: &KeyTreeSnapshot,
        namespace: &str,
        watched: &[String],
        now: Instant,
    ) {
        self.freshness.clear();
        let Some(presence) = presence else {
            return;
        };
        // Validated once per tick, borrowed (#178).
        let watched: Vec<&keyexpr> = watched
            .iter()
            .filter_map(|w| keyexpr::new(w.as_str()).ok())
            .collect();
        let state_tokens = [KindToken::State.as_str(), KindToken::ExplicitState.as_str()];
        for (addr, _) in presence.addresses() {
            let (system, service) = (addr.system.as_str(), addr.service.as_str());
            // `state/**` and `@state/**` named apart: `*` and `**` never
            // cross a verbatim chunk (core §1.3), so one selector cannot
            // stand for both.
            let covered = state_tokens.iter().any(|token| {
                let sel = zenkey_fleet::with_namespace(
                    namespace,
                    [GRAMMAR, system, service, "*", token, "**"].join("/"),
                );
                keyexpr::new(sel.as_str())
                    .map(|s| watched.iter().any(|w| w.intersects(s)))
                    .unwrap_or(false)
            });
            let mut fresh = Freshness {
                watched: covered,
                last_state_age: None,
            };
            if covered {
                let mut chunks: Vec<&str> =
                    namespace.split('/').filter(|c| !c.is_empty()).collect();
                chunks.extend([GRAMMAR, system, service]);
                let newest = tree.node(&chunks).and_then(|node| {
                    node.children
                        .values()
                        .flat_map(|iface| {
                            state_tokens
                                .iter()
                                .filter_map(|t| iface.children.get(*t))
                                .filter_map(|n| n.subtree_last_seen)
                        })
                        .max()
                });
                fresh.last_state_age = newest.map(|seen| now.saturating_duration_since(seen));
            }
            self.freshness.insert(addr.clone(), fresh);
        }
    }

    /// A health reading was asked for `addr`.
    pub fn asking(&mut self, addr: Addr) {
        self.health = HealthState::Asking(addr);
    }

    /// A health reading landed. The handler admits only one about the
    /// address the window points at, in the namespace it shows (#109) —
    /// the staleness guard is the selection's, as v1's `node_info` was.
    pub fn health_read(&mut self, addr: Addr, report: Arc<HealthReport>) {
        self.health = HealthState::Read(addr, report);
    }

    /// `addr` was selected: its reading is owed once the selection
    /// settles. Returns the selection's count, which the settle carries.
    pub fn due_redundancy(&mut self, addr: Addr) -> u64 {
        self.redundancy_due += 1;
        self.redundancy = RedundancyState::Due(addr);
        self.redundancy_due
    }

    /// Whether the settle of selection `due` of `addr` asks the reading:
    /// only the latest selection's, while its reading is still owed.
    pub fn settled(&self, addr: &Addr, due: u64) -> bool {
        due == self.redundancy_due
            && matches!(&self.redundancy, RedundancyState::Due(a) if a == addr)
    }

    /// A `redundancy.v1` reading was asked for `addr` (#841, RC).
    pub fn asking_redundancy(&mut self, addr: Addr) {
        self.redundancy = RedundancyState::Asking(addr);
    }

    /// A `redundancy.v1` reading landed, under the health reading's guard.
    pub fn redundancy_read(&mut self, addr: Addr, report: Arc<RedundancyReport>) {
        self.redundancy = RedundancyState::Read(addr, report);
    }

    /// `addr` was selected (#909, BC): its `arbitration.v1` reading owed at
    /// the selection's settle when a descriptor of it requires a role
    /// (`consumer`), and not asked otherwise.
    pub fn due_arbitration(&mut self, addr: Addr, consumer: bool) {
        self.arbitration = if consumer {
            ArbitrationState::Due(addr)
        } else {
            ArbitrationState::NotAsked
        };
    }

    /// Whether the settle asks `addr`'s `arbitration.v1` reading.
    pub fn arbitration_owed(&self, addr: &Addr) -> bool {
        matches!(&self.arbitration, ArbitrationState::Due(a) if a == addr)
    }

    /// An `arbitration.v1` reading was asked for `addr` (#909, BC).
    pub fn asking_arbitration(&mut self, addr: Addr) {
        self.arbitration = ArbitrationState::Asking(addr);
    }

    /// An `arbitration.v1` reading landed, under the health reading's
    /// guard.
    pub fn arbitration_read(&mut self, addr: Addr, report: Arc<ArbitrationReport>) {
        self.arbitration = ArbitrationState::Read(addr, report);
    }

    /// A namespace switch or reconnect: what was read described another
    /// deployment. The clock toggle is the operator's word, and stays.
    pub fn clear(&mut self) {
        *self = NodesState {
            clocks_synced: self.clocks_synced,
            ..NodesState::default()
        };
    }
}

/// A descriptor answer, as the pane shows it.
fn described(answer: &Asked<DescriptorAnswer>) -> Described {
    match answer.as_option() {
        None => Described::NotAsked,
        Some(DescriptorAnswer::Served { descriptor }) => {
            let meta = |k: &str| {
                descriptor.meta.get(k).map(|v| match v.as_str() {
                    Some(s) => s.to_owned(),
                    None => v.to_string(),
                })
            };
            Described::Served(DescriptorFacts {
                host: meta("host"),
                process: meta("process"),
                build: meta("build"),
                zid: meta("zid"),
                profiles: descriptor.profiles.clone(),
                interfaces: descriptor
                    .interfaces
                    .iter()
                    .map(|e| {
                        let short = e
                            .contract
                            .strip_prefix("sha256:")
                            .map(|h| h.chars().take(12).collect::<String>())
                            .unwrap_or_else(|| e.contract.clone());
                        (e.iface.clone(), short)
                    })
                    .collect(),
                standby: descriptor
                    .interfaces
                    .iter()
                    .filter(|e| e.standby)
                    .map(|e| e.iface.clone())
                    .collect(),
            })
        }
        Some(DescriptorAnswer::Invalid { findings }) => Described::Invalid(findings.clone()),
        Some(DescriptorAnswer::Silent) => Described::Silent,
        Some(DescriptorAnswer::Failed { reason }) => Described::Failed(reason.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    use zenkey_fleet::model::catalog::{DescriptorRead, Observed};
    use zenkey_fleet::{PresenceScope, StatsTable};
    use zenkey_model::descriptor::Descriptor;

    const NAV: &str = "zk2/robot1/nav/@zk/instance/0000000000000001";

    fn addr(s: &str) -> Addr {
        s.parse().expect("an address")
    }

    fn presence(keys: &[&str], complete: bool) -> LivePresence {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let mut live = LivePresence::new(PresenceScope::all());
        live.seed(
            &Observed::from_keys("zk2/*/*/@zk/**", &keys, complete),
            SystemTime::now(),
        );
        live
    }

    /// The join reaches only watched addresses, and an address watched with
    /// nothing arrived is unknown, not fresh (O4) — `state` and `@state`
    /// both count, a stream does not.
    #[test]
    fn freshness_joins_watched_state_only() {
        let live = presence(&[NAV], true);
        let mut stats = StatsTable::new();
        let now = Instant::now();
        stats.record(
            "acme/zk2/robot1/nav/nav.v2/stream/position",
            8,
            None,
            now,
            None,
            None,
        );
        let tree = KeyTreeSnapshot::build(&stats);
        let mut nodes = NodesState::default();
        nodes.refresh(
            Some(&live),
            &tree,
            "acme",
            &["acme/zk2/**".to_string()],
            now,
        );
        let f = nodes.freshness(&addr("robot1/nav"));
        assert!(f.watched, "acme/zk2/** covers robot1/nav's state/**");
        assert_eq!(f.last_state_age, None, "a stream is no state sample");

        stats.record(
            "acme/zk2/robot1/nav/nav.v2/state/status",
            8,
            None,
            now,
            None,
            None,
        );
        let tree = KeyTreeSnapshot::build(&stats);
        nodes.refresh(
            Some(&live),
            &tree,
            "acme",
            &["acme/zk2/**".to_string()],
            now,
        );
        assert!(
            nodes
                .freshness(&addr("robot1/nav"))
                .last_state_age
                .is_some()
        );

        nodes.refresh(Some(&live), &tree, "acme", &[], now);
        assert!(!nodes.freshness(&addr("robot1/nav")).watched);
        // `**` never crosses `@state` (core §1.3): only a selector naming it
        // covers an explicit state population.
        nodes.refresh(
            Some(&live),
            &tree,
            "acme",
            &["acme/zk2/robot1/nav/*/@state/**".to_string()],
            now,
        );
        assert!(nodes.freshness(&addr("robot1/nav")).watched);
    }

    /// The index is built from the catalog's descriptors, rebuilt only when
    /// another catalog is swapped in, and a hostid listing comes from the
    /// descriptor's `profiles` alone.
    #[test]
    fn descriptors_are_indexed_once_per_catalog() {
        let mut observed = Observed::from_keys("zk2/*/*/@zk/**", &[NAV.to_string()], true);
        let mut meta = BTreeMap::new();
        meta.insert("host".to_owned(), serde_json::json!("rover-01"));
        meta.insert("zid".to_owned(), serde_json::json!("a1b2"));
        observed.descriptors = Some(BTreeMap::from([(
            (
                addr("robot1/nav"),
                "0000000000000001".parse().expect("an id"),
            ),
            DescriptorRead::Served(Box::new(Descriptor {
                format: "zk2-descriptor/0.1".into(),
                service: "robot1/nav".into(),
                instance: "0000000000000001".into(),
                interfaces: Vec::new(),
                capabilities: Vec::new(),
                requires: Vec::new(),
                profiles: vec![HOSTID.into()],
                meta,
            })),
        )]));
        let catalog = Arc::new(Catalog::new(&observed));
        let mut nodes = NodesState::default();
        assert!(nodes.instances(&addr("robot1/nav")).is_none());
        nodes.describe(Some(&catalog));
        let got = nodes.instances(&addr("robot1/nav")).expect("indexed");
        let Described::Served(facts) = &got[0].described else {
            panic!("served: {got:?}");
        };
        assert_eq!(facts.host.as_deref(), Some("rover-01"));
        assert_eq!(facts.zid.as_deref(), Some("a1b2"));
        assert!(facts.lists_hostid());
        nodes.describe(None);
        assert!(nodes.instances(&addr("robot1/nav")).is_none());
    }

    /// #862's review: a reading is owed per selection, and only the
    /// latest selection's settle asks it.
    #[test]
    fn only_the_latest_selection_settles_into_a_reading() {
        let mut nodes = NodesState::default();
        let first = nodes.due_redundancy(addr("robot1/nav"));
        let second = nodes.due_redundancy(addr("host-a/tc"));
        assert!(!nodes.settled(&addr("robot1/nav"), first), "superseded");
        assert!(!nodes.settled(&addr("host-a/tc"), first), "an older count");
        assert!(nodes.settled(&addr("host-a/tc"), second));
        nodes.asking_redundancy(addr("host-a/tc"));
        assert!(!nodes.settled(&addr("host-a/tc"), second), "already asked");
    }

    /// A forget drops the reading and keeps the operator's word.
    #[test]
    fn a_forget_keeps_the_clock_word_and_drops_the_reading() {
        let mut nodes = NodesState {
            clocks_synced: true,
            ..NodesState::default()
        };
        nodes.asking(addr("robot1/nav"));
        assert_eq!(nodes.health.address(), Some(&addr("robot1/nav")));
        nodes.clear();
        assert!(nodes.clocks_synced, "the operator's word is kept");
        assert!(matches!(nodes.health, HealthState::NotAsked));
    }
}
