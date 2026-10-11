//! Fixtures shared by the integration suites (#532).
//!
//! `tests/panes/` renders surfaces one at a time; `shots.rs` renders the whole
//! window. Both need the same registry, and the window needs a bus that is
//! not there — so the bus is a fixture too: [`World`] drives a bus-less
//! [`Zengui`] through its public messages exactly as the link would, minus the
//! link. Nothing here reaches past `zengui`'s public API.

#![allow(dead_code)]

pub mod scenes;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zengui::app::Zengui;
use zengui::config::Settings;
use zengui::message::{BusMsg, BusTick, LinkState, Message, WatchedTotals};
use zengui::prefs::Prefs;
use zenkey_fleet::StatsTable;
use zenkey_fleet::{KeyTreeSnapshot, SampleView};
use zenkey_fleet_v1::SliceSet;

/// The ZenSight registry snapshot `fixture-tests/` compiles — v1's declared
/// world, still what the v1 panes' own tests render against until each
/// moves (FL7–FL11). The window itself no longer loads it (#614, FL6a).
pub fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

/// The zk2 contracts the window's world is described by (#614, FL6a): the
/// walkthrough and tcgui examples, loaded exactly as `--contracts` loads
/// them — tcgui's bindings file beside its contracts included, said as a
/// problem and skipped.
pub fn contract_paths() -> Vec<PathBuf> {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2");
    vec![examples.join("walkthrough"), examples.join("tcgui")]
}

/// [`contract_paths`], loaded.
pub fn contracts() -> Arc<zengui::services::contracts::LoadedContracts> {
    Arc::new(zengui::services::contracts::load_all(&contract_paths()))
}

/// Settings for a window with no bus behind it: nothing to dial, the
/// contracts named (they arrive as a message instead), the default scope.
pub fn settings() -> Settings {
    Settings {
        namespace: String::new(),
        connect: vec!["tcp/10.0.4.2:7447".into()],
        listen: vec![],
        scouting: None,
        zenoh_config: None,
        contracts: contract_paths(),
        timeout_secs: 5,
        scope: zengui::scope::ScopePreset::Everything,
        selectors: vec![],
        eager: false,
        echo_lines: 500,
        history_entries: 50,
        max_keys: 50_000,
    }
}

/// A window, constructed and never connected: the link `Task` is dropped,
/// so no session ever opens.
pub fn window(prefs: Prefs) -> Zengui {
    Zengui::with_prefs(settings(), prefs, None).0
}

/// Feed one message, dropping the task it returns — there is no runtime
/// here, which is the point: whatever a task would have answered, the scene
/// answers itself with the message it would have produced.
pub fn send(app: &mut Zengui, message: Message) {
    let _ = app.update(message);
}

/// One sample, as the monitor would hand it over.
pub fn sample(key: &str, payload: &[u8], encoding: &str, at: Instant) -> Arc<SampleView> {
    view(key, payload, encoding, zenoh::sample::SampleKind::Put, at)
}

/// A tombstone (RFC 04 §1.2) — retirement, not an empty put.
pub fn tombstone(key: &str, at: Instant) -> Arc<SampleView> {
    view(key, b"", "", zenoh::sample::SampleKind::Delete, at)
}

fn view(
    key: &str,
    payload: &[u8],
    encoding: &str,
    kind: zenoh::sample::SampleKind,
    at: Instant,
) -> Arc<SampleView> {
    Arc::new(SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
        encoding: encoding.to_string(),
        kind,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: zenoh::qos::Priority::DEFAULT,
        congestion_control: zenoh::qos::CongestionControl::DEFAULT,
        reliability: zenoh::qos::Reliability::DEFAULT,
        express: false,
        source: None,
        received: at,
    })
}

/// A small fleet, built to exercise every row anatomy the tree and echo
/// have: zk2 keys the lens resolves (#614, FL6a) — a protobuf stream and
/// state under `robot1/nav`, JSON state under `host-a/tc` — beside one whose
/// value fails its schema and one whose bytes are not their declared type,
/// so all four conformance poles have a key; v1 keys, foreign since the
/// port; foreign keys from a non-convention publisher and from another
/// deployment; a state leaf that retires; and numeric leaves whose values
/// wander so a series has something to plot.
pub struct World {
    stats: StatsTable,
    t0: Instant,
    ticks: u32,
    contracts: Arc<zengui::services::contracts::LoadedContracts>,
    catalog: Arc<zenkey_fleet::Catalog>,
    presence: Arc<zenkey_fleet::LivePresence>,
    seeded: Vec<zenkey_fleet::PresenceTransition>,
    /// The presence read both were built from: what a health reading of
    /// this world reads presence and descriptors as (#614, FL9).
    observed: zenkey_fleet::model::catalog::Observed,
}

pub const ORIGIN_A: &str = "h-3fa9c2d41b7e";
pub const ORIGIN_B: &str = "h-9c04d2e1a7f3";
pub const USED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
pub const HEALTH: &str = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

/// `robot1/nav`'s position stream: a `nav.v2.Position` protobuf, valid.
pub const POSITION: &str = "zk2/robot1/nav/nav.v2/stream/position";
/// `robot1/nav`'s status state: a `nav.v2.Status` protobuf, valid.
pub const NAV_STATUS: &str = "zk2/robot1/nav/nav.v2/state/status";
/// `robot1/nav`'s origin state, carrying bytes that are no `nav.v2.Origin`:
/// the undecodable pole.
pub const NAV_ORIGIN: &str = "zk2/robot1/nav/nav.v2/state/origin";
/// `host-a/tc`'s `eth0`: a `json:NetworkInterface`, valid.
pub const ETH0: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
/// `host-a/tc`'s `wlan0`: JSON that decodes and fails its schema — the
/// invalid pole.
pub const WLAN0: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/wlan0";

/// `(key, payload at tick n, encoding)`, one row per publisher.
type Feed = (&'static str, fn(u32) -> Vec<u8>, &'static str);

const FEEDS: &[Feed] = &[
    (
        USED,
        |n| json_point(61.0 + f64::from(n % 7) * 0.8),
        "application/json",
    ),
    (
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load",
        |n| json_point(0.42 + f64::from(n % 5) * 0.11),
        "application/json",
    ),
    (
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/uptime",
        |n| json_point(86_400.0 + f64::from(n)),
        "application/json",
    ),
    (
        HEALTH,
        |_| br#"{"state":"ok","since":"2026-10-05T08:00:00Z"}"#.to_vec(),
        "application/json",
    ),
    (
        "v1/h-9c04d2e1a7f3/telemetry/sysinfo/system/load",
        |n| json_point(1.7 + f64::from(n % 3) * 0.2),
        "application/json",
    ),
    (
        "v1/h-9c04d2e1a7f3/telemetry/sysinfo/no/such/thing",
        |n| format!("{n}").into_bytes(),
        "text/plain",
    ),
    (
        "demo/example/foo",
        |n| format!("hello #{n}").into_bytes(),
        "text/plain",
    ),
    (
        "someotherbase/v1/h-0123456789ab/state/p/a",
        |_| vec![0xa1, 0x61, 0x6b, 0x18, 0x2a],
        "application/cbor",
    ),
];

/// The zk2 publishers a contract refuses (#614, FL6a): a value its schema
/// rejects, and bytes that are not their declared type. Published last in
/// a tick, so they lead echo's newest-first stream.
const REFUSED_FEEDS: &[Feed] = &[
    (
        WLAN0,
        |_| br#"{"name":"wlan0","index":-1,"namespace":"default","is_up":"yes"}"#.to_vec(),
        "application/json",
    ),
    (
        NAV_ORIGIN,
        // A field tag no varint can finish: not a `nav.v2.Origin`.
        |_| vec![0x0a, 0xff, 0xff, 0xff],
        "application/protobuf",
    ),
];

/// The zk2 publishers whose payloads the contract itself makes (#614,
/// FL6a): `(key, iface, kind token, template, encoding)`, synthesized per
/// tick as `zenctl gen` would (`zenkey_fleet::Synth`), so each is a valid
/// instance of its declared type and wanders like a live owner's.
const SYNTH_FEEDS: &[(&str, &str, &str, &str, &str)] = &[
    (
        POSITION,
        "nav.v2",
        "stream",
        "position",
        "application/protobuf",
    ),
    (
        NAV_STATUS,
        "nav.v2",
        "state",
        "status",
        "application/protobuf",
    ),
    (
        ETH0,
        "tc.netif.v1",
        "state",
        "interfaces/{ns}/{iface}",
        "application/json",
    ),
];

fn json_point(value: f64) -> Vec<u8> {
    format!(r#"{{"value":{value:.2},"unit":"%","ts":"2026-10-05T09:12:44Z"}}"#).into_bytes()
}

/// One synthesized payload of `iface`'s `token/template` resource at `tick`.
fn synth(
    set: &zenkey_fleet::ContractSet,
    iface: &str,
    token: &str,
    template: &str,
    tick: u32,
) -> Vec<u8> {
    let iface: zenkey_model::grammar::IfaceId = iface.parse().expect("an interface");
    let revision = set.of_iface(&iface).next().expect("the contract is held");
    let resource = revision
        .contract()
        .resources
        .iter()
        .find(|r| r.token.as_str() == token && r.template.as_str() == template)
        .expect("the resource is declared");
    zenkey_fleet::Synth::new(7)
        .sample(
            revision,
            resource,
            zenkey_fleet::Member::Type,
            u64::from(tick),
        )
        .expect("the contract's own type synthesizes")
        .bytes
}

/// The deployment the lens sees (#614, FL6a), as a catalog built by hand
/// from tokens and descriptors with the real fingerprints of what
/// [`contracts`] loaded — the fleet's own skeleton tests' recipe — and the
/// presence projection seeded from the same read: `robot1/nav` serving
/// `nav.v2` and `host-a/tc` serving `tc.netif.v1`, each with one instance.
fn deployment(
    set: &zenkey_fleet::ContractSet,
) -> (
    Arc<zenkey_fleet::Catalog>,
    zenkey_fleet::LivePresence,
    Vec<zenkey_fleet::PresenceTransition>,
    zenkey_fleet::model::catalog::Observed,
) {
    use std::collections::BTreeMap;
    use zenkey_fleet::model::catalog::{DescriptorRead, Observed};
    use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
    use zenkey_model::grammar::{Addr, IfaceId, InstanceId, alive_key, instance_key};

    let mut keys = Vec::new();
    let mut descriptors = BTreeMap::new();
    for (id, (address, iface)) in (1u64..).zip(World::ADDRESSES) {
        let addr: Addr = address.parse().expect("an address");
        let iface: IfaceId = iface.parse().expect("an interface");
        let revision = set.of_iface(&iface).next().expect("held");
        let instance = InstanceId::from_u64(id);
        keys.push(instance_key(&addr, &instance).expect("a key").to_string());
        keys.push(
            alive_key(
                &addr,
                &iface,
                &instance,
                &revision.fingerprint().hex().fp16(),
            )
            .expect("a key")
            .to_string(),
        );
        descriptors.insert(
            (addr, instance.clone()),
            DescriptorRead::Served(Box::new(Descriptor {
                format: "zk2-descriptor/0.1".into(),
                service: address.into(),
                instance: instance.to_string(),
                interfaces: vec![InterfaceEntry {
                    iface: iface.to_string(),
                    contract: revision.fingerprint().to_string(),
                    minor: 0,
                    token: true,
                    unavailable: Vec::new(),
                    cardinality: BTreeMap::new(),
                }],
                capabilities: Vec::new(),
                requires: Vec::new(),
                profiles: Vec::new(),
                meta: BTreeMap::new(),
            })),
        );
    }
    let mut observed = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
    observed.descriptors = Some(descriptors);
    let catalog = Arc::new(zenkey_fleet::Catalog::new(&observed));
    let mut presence = zenkey_fleet::LivePresence::new(zenkey_fleet::PresenceScope::all());
    let seeded = presence.seed(&observed, std::time::SystemTime::now());
    (catalog, presence, seeded, observed)
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> World {
        let contracts = contracts();
        let (catalog, presence, seeded, observed) = deployment(&contracts.set);
        World {
            stats: StatsTable::new(),
            // Far enough back that every rate has settled and every age
            // reads as a plausible live bus.
            t0: Instant::now()
                .checked_sub(Duration::from_secs(40))
                .unwrap_or_else(Instant::now),
            ticks: 0,
            contracts,
            catalog,
            presence: Arc::new(presence),
            seeded,
            observed,
        }
    }

    /// `addr`'s `health.v1`, as the engine judges a reading of this world
    /// (#614, FL9): presence and the descriptors as the world holds them,
    /// and both readings' GETs answered by nobody — no address here
    /// implements `health.v1`, so its verdict is "not asked", in the
    /// engine's own words.
    pub fn health(&self, addr: &str) -> Arc<zenkey_fleet::report::HealthReport> {
        let addr: zenkey_model::grammar::Addr = addr.parse().expect("an address");
        let target = zenkey_fleet::HealthTarget::One(addr);
        let got = zenkey_fleet::bus::health::HealthGet {
            selector: target.state_selector(),
            read_at: std::time::SystemTime::now(),
            complete: true,
            services: Default::default(),
        };
        Arc::new(zenkey_fleet::judge_health(
            &zenkey_fleet::HealthObservation {
                namespace: String::new(),
                target,
                spec: zenkey_fleet::HealthSpec {
                    timeout: Duration::from_secs(2),
                    window: None,
                    grace: Duration::from_secs(1),
                    clocks_synced: false,
                    face: None,
                },
                presence: Some(Ok(self.observed.clone())),
                first: Ok(got.clone()),
                second: Ok(got),
                window: None,
                archived: None,
            },
        ))
    }

    /// The window as the link leaves it once the sessions are up (#614,
    /// FL6a): the contracts in hand, the presence link's first landing —
    /// the projection seeded, the lens's catalog beside it — and the pump
    /// running.
    pub fn connect(&self, app: &mut Zengui) {
        send(
            app,
            Message::Bus(BusMsg::ContractsLoaded(Arc::clone(&self.contracts))),
        );
        send(
            app,
            Message::Bus(BusMsg::Presence(0, self.presence_update())),
        );
        send(app, Message::Bus(BusMsg::Link(LinkState::Pumping)));
    }

    /// The presence link's landing for this world: what `connect` sends.
    pub fn presence_update(&self) -> Arc<zengui::presence_link::PresenceUpdate> {
        Arc::new(zengui::presence_link::PresenceUpdate {
            presence: Arc::clone(&self.presence),
            moved: self.seeded.clone(),
            catalog: Some(Arc::clone(&self.catalog)),
            generation: 1,
            seed: zenkey_fleet::SeedOutcome::Complete,
        })
    }

    /// Every publisher's payload at tick `n`: `(key, bytes, encoding)`.
    fn payloads(&self, n: u32) -> Vec<(&'static str, Vec<u8>, &'static str)> {
        let mut out: Vec<_> = FEEDS
            .iter()
            .map(|(key, payload, encoding)| (*key, payload(n), *encoding))
            .collect();
        for (key, iface, token, template, encoding) in SYNTH_FEEDS {
            let bytes = synth(&self.contracts.set, iface, token, template, n);
            out.push((key, bytes, encoding));
        }
        out.extend(
            REFUSED_FEEDS
                .iter()
                .map(|(key, payload, encoding)| (*key, payload(n), *encoding)),
        );
        out
    }

    /// The lens the window resolves with once connected — the same inputs,
    /// session-free — for the checks a scene answers itself.
    pub fn check(&self, key: &str, bytes: &[u8], encoding: &str) -> zenkey_fleet::Checked {
        zenkey_fleet::Lens::new("", Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set)
            .check(
                key,
                zenkey_fleet::Member::Type,
                (!encoding.is_empty()).then_some(encoding),
                bytes,
            )
    }

    /// A key's identity through the lens the window resolves with once
    /// connected (#614, FL7) — what its identity cache holds for the key.
    pub fn resolve(&self, key: &str) -> zenkey_fleet::Resolution {
        zenkey_fleet::Lens::new("", Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set)
            .resolve(key)
    }

    /// `keys`' identities through this world's lens over `namespace`, held
    /// as the window's identity cache holds them (#614, FL7). The catalog is
    /// namespace-relative, so the same deployment reads under any namespace.
    pub fn identities(&self, namespace: &str, keys: &[&str]) -> zenkey_fleet::IdentityCache {
        let lens = zenkey_fleet::Lens::new(namespace, Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set);
        let mut cache = zenkey_fleet::IdentityCache::default();
        for key in keys {
            cache.ensure(&lens, key);
        }
        cache
    }

    /// The presence read the lens resolves addresses through.
    pub fn catalog(&self) -> Arc<zenkey_fleet::Catalog> {
        Arc::clone(&self.catalog)
    }

    /// The presence projection, seeded when the world was built.
    pub fn presence(&self) -> Arc<zenkey_fleet::LivePresence> {
        Arc::clone(&self.presence)
    }

    /// A contract store seeded with the world's contracts, as the window's
    /// is once they load (§8.5).
    pub fn store(&self) -> zenkey_fleet::BundleStore {
        let store = zenkey_fleet::BundleStore::new(Duration::from_secs(1));
        store.seed(&self.contracts.set);
        store
    }

    /// The declared keyspace the window builds once connected (#614, FL6b):
    /// zk2's contract skeleton over the same lens [`World::check`] resolves
    /// with, and no admin space — what `Deployment::refresh_skeleton` makes
    /// of this world.
    pub fn skeleton(&self) -> zenkey_fleet::Skeleton {
        let lens = zenkey_fleet::Lens::new("", Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set);
        zenkey_fleet::Skeleton::build(&lens, None)
    }

    /// The addresses this world's catalog places, with the interface each
    /// provides: `robot1/nav` serving `nav.v2`, `host-a/tc` `tc.netif.v1`.
    pub const ADDRESSES: [(&'static str, &'static str); 2] =
        [("robot1/nav", "nav.v2"), ("host-a/tc", "tc.netif.v1")];

    /// What the bounded validation batch would have answered (#164) for
    /// every publisher's newest payload, through the lens: the four poles,
    /// each on the key that earns it.
    pub fn verdicts(&self, app: &mut Zengui) {
        let n = self.ticks.saturating_sub(1);
        let batch = self
            .payloads(n)
            .into_iter()
            .map(|(key, bytes, encoding)| {
                (
                    key.to_string(),
                    self.check(key, &bytes, encoding).conformance,
                )
            })
            .collect();
        send(app, Message::Bus(BusMsg::VerdictsChecked(batch)));
    }

    /// One 250 ms monitor tick: every feed publishes once, and the tick
    /// carries the snapshot, the samples and the counters a real one would.
    pub fn tick(&mut self, app: &mut Zengui) {
        let n = self.ticks;
        let at = self.t0 + Duration::from_millis(250) * (n + 1);
        let mut samples = Vec::new();
        let payloads = self.payloads(n);
        let feeds = payloads.len();
        for (key, body, encoding) in payloads {
            self.stats.record(key, body.len(), None, at, None, None);
            samples.push(sample(key, &body, encoding, at));
        }
        // Every 12th tick the health leaf retires and comes back (RFC 04
        // §1.2): the tree, the echo and the history all have a tombstone to
        // draw.
        if n % 12 == 11 {
            samples.push(tombstone(HEALTH, at));
        }
        let totals = WatchedTotals {
            samples: u64::from(n + 1) * feeds as u64,
            bytes: u64::from(n + 1) * 420,
            rate_hz: feeds as f64 * 4.0,
        };
        let tick = BusTick {
            tree: Arc::new(KeyTreeSnapshot::build(&self.stats)),
            samples,
            lagged: 0,
            coalesced: 0,
            keys: feeds,
            keys_evicted: 0,
            keys_unwatched: 0,
            watched: Arc::from(vec!["**".to_string()]),
            seeded: Vec::new(),
            totals,
        };
        self.ticks += 1;
        send(app, Message::Bus(BusMsg::Tick(Arc::new(tick))));
    }

    pub fn ticks(&mut self, app: &mut Zengui, n: u32) {
        for _ in 0..n {
            self.tick(app);
        }
    }
}
