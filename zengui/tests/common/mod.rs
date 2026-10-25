//! Fixtures shared by the integration suites (#532).
//!
//! `tests/panes/` renders surfaces one at a time; `shots.rs` renders the whole
//! window. Both need the same contracts, and the window needs a bus that is
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

/// The metered world's contracts (#819, TD): [`contract_paths`], the
/// runtime's `telemetry.v1` scenario contract `meter.v1` (a Point counter
/// with its epoch in the Point, a bare counter with its epoch in the
/// attachment, one with neither, a Point histogram and a bare gauge), and
/// zenoh-modem's `modem.v3`, whose bare numbers carry their units only in
/// their declarations.
pub fn metered_contract_paths() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut paths = contract_paths();
    paths.push(root.join("../zenkey/tests/contracts/meter.v1.toml"));
    paths.push(root.join("../examples/zk2/zenoh-modem"));
    paths
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

/// The wire axes a sample rides (core §2.4): what the monitor reads off
/// every sample, and what the Inspector compares against the declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axes {
    pub priority: zenoh::qos::Priority,
    pub congestion: zenoh::qos::CongestionControl,
    pub reliability: zenoh::qos::Reliability,
    pub express: bool,
}

impl Axes {
    /// zenoh's defaults: what a raw put rides, and all a key no contract
    /// types can ride.
    pub const DEFAULT: Axes = Axes {
        priority: zenoh::qos::Priority::DEFAULT,
        congestion: zenoh::qos::CongestionControl::DEFAULT,
        reliability: zenoh::qos::Reliability::DEFAULT,
        express: false,
    };

    /// The axes `key` rides when its owner writes it (#614, FL12): its
    /// resource's declared QoS, mapped as the runtime's writer maps it
    /// (`zenkey::qos`), when `lens` resolves the key to a stream, state or
    /// event; zenoh's defaults for any other key.
    pub fn declared(lens: &zenkey_fleet::Lens<'_>, key: &str) -> Axes {
        use zenkey_model::contract::Body;
        let resolution = lens.resolve(key);
        match resolution.resolved.as_ref().map(|r| &r.resource().body) {
            Some(Body::Data(d)) => Axes {
                priority: zenkey::qos::priority(d.priority),
                congestion: zenkey::qos::congestion(d.congestion),
                reliability: zenkey::qos::reliability(d.reliability),
                express: d.express,
            },
            _ => Axes::DEFAULT,
        }
    }
}

/// One sample, as the monitor would hand it over, on zenoh's default axes.
pub fn sample(key: &str, payload: &[u8], encoding: &str, at: Instant) -> Arc<SampleView> {
    sample_on(key, payload, encoding, at, Axes::DEFAULT)
}

/// One sample on the axes it rode.
pub fn sample_on(
    key: &str,
    payload: &[u8],
    encoding: &str,
    at: Instant,
    axes: Axes,
) -> Arc<SampleView> {
    view(
        key,
        payload,
        encoding,
        zenoh::sample::SampleKind::Put,
        at,
        axes,
    )
}

/// [`sample_on`], carrying `attachment` (#819, TD): a bare counter's
/// in-band epoch rides one (`telemetry.v1` §2.4).
pub fn sample_attached(
    key: &str,
    payload: &[u8],
    encoding: &str,
    at: Instant,
    axes: Axes,
    attachment: Option<&[u8]>,
) -> Arc<SampleView> {
    let mut v = view(
        key,
        payload,
        encoding,
        zenoh::sample::SampleKind::Put,
        at,
        axes,
    );
    Arc::get_mut(&mut v).expect("just made").attachment =
        attachment.map(|a| zenoh::bytes::ZBytes::from(a.to_vec()));
    v
}

/// A tombstone (RFC 04 §1.2) — retirement, not an empty put.
pub fn tombstone(key: &str, at: Instant) -> Arc<SampleView> {
    view(
        key,
        b"",
        "",
        zenoh::sample::SampleKind::Delete,
        at,
        Axes::DEFAULT,
    )
}

fn view(
    key: &str,
    payload: &[u8],
    encoding: &str,
    kind: zenoh::sample::SampleKind,
    at: Instant,
    axes: Axes,
) -> Arc<SampleView> {
    Arc::new(SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
        encoding: encoding.to_string(),
        kind,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: axes.priority,
        congestion_control: axes.congestion,
        reliability: axes.reliability,
        express: axes.express,
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
///
/// Every zk2 key rides the axes its contract declares (#614, FL12), as an
/// owner writing through the runtime does, so the Inspector reads a match;
/// a key no contract types rides zenoh's defaults. A stranger's raw put on
/// an owned key is the one sample that rides other axes, and only when a
/// scene asks for it ([`World::tick_with_stranger`]).
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
    /// Each publisher's axes ([`Axes::declared`]), resolved once.
    axes: std::collections::BTreeMap<&'static str, Axes>,
    /// Whether `lab/meter` and `site-a/rf0` publish beside the rest
    /// ([`World::metered`], #819, TD).
    metered: bool,
    /// What the last tick published, as [`World::verdicts`] checks it.
    last: Vec<Published>,
}

/// One publisher's payload in one tick: `(key, bytes, encoding,
/// attachment)`.
type Published = (&'static str, Vec<u8>, &'static str, Option<Vec<u8>>);

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

/// The `config.v1` owner of the Config scenes (#768, CD): the profile's
/// `rf`, as `scenarios.md` names it.
pub const RF0: &str = "lab/rf0";
/// Its instance; its tokens are `<instance>-<n>` (§2.6, the reference's).
pub const RF0_INSTANCE: &str = "00000000000000a1";

/// The `redundancy.v1` pair of the redundancy scene (#841, RC): ZenSight's
/// catalog singleton, `zs.catalog.v1`, with an incumbent and its standby.
pub const CATALOG: &str = "lab/catalog";
/// Its incumbent, holding the interface's token, and its standby, marked
/// and holding none.
pub const CATALOG_SERVING: &str = "00000000000000c1";
pub const CATALOG_STANDBY: &str = "00000000000000c2";

/// `zs.catalog.v1`, as `examples/zk2/zensight` holds it: governed by
/// `redundancy.v1` (election `claim`).
pub fn catalog_revision() -> Arc<zenkey_fleet::model::catalog::Revision> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/zk2/zensight/zs.catalog.v1.toml");
    let l = zenkey_model::contract::load_path(&path);
    Arc::new(zenkey_fleet::model::catalog::Revision::from_contract(
        l.contract.unwrap_or_else(|| panic!("{}", l.report)),
        zenkey_fleet::report::ContractSource::File,
    ))
}

/// The instant a config reading of this world ended, on this host's clock:
/// fixed, so a countdown renders the same twice.
pub fn config_read_at() -> std::time::SystemTime {
    let ms = zenkey_model::config::Time::parse("2026-10-18T09:00:00.000Z")
        .expect("an instant")
        .millis();
    std::time::SystemTime::UNIX_EPOCH + Duration::from_millis(ms as u64)
}

/// The tick a Config scene's countdown reads: 13 s after the reading, so
/// a deadline a minute after it is 47 s away.
pub fn config_now() -> std::time::SystemTime {
    config_read_at() + Duration::from_secs(13)
}

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

/// `lab/meter`, implementing `meter.v1` (#819, TD).
pub const METER: &str = "lab/meter";
/// A Point declared a counter, unit `1`, its epoch in the Point: an epoch
/// that changes at tick [`FRAMES_RESET_AT`] (a restart the owner marked),
/// the count starting again — a reset, with no rate across it.
pub const FRAMES: &str = "zk2/lab/meter/meter.v1/stream/frames";
/// The tick `frames` starts its second epoch at.
pub const FRAMES_RESET_AT: u32 = 16;
/// A bare counter in `By`, its epoch in the Epoch attachment: a drop at
/// tick [`BYTES_DROP_AT`] within that one epoch — `decreased`, the finding.
pub const BYTES: &str = "zk2/lab/meter/meter.v1/stream/bytes";
/// The tick `bytes` goes backwards at.
pub const BYTES_DROP_AT: u32 = 22;
/// A bare counter with no unit and no epoch: a drop at tick 14, which no
/// epoch places, so a reset, assumed — and its unit unknown.
pub const DROPS: &str = "zk2/lab/meter/meter.v1/stream/drops";
/// A Point declared a histogram, in `s`, over `[0.005, 0.01, 0.025]`.
pub const LATENCY: &str = "zk2/lab/meter/meter.v1/stream/latency";
/// A bare gauge in `dBm`.
pub const LEVEL: &str = "zk2/lab/meter/meter.v1/stream/level";
/// `site-a/rf0`, one zenoh-modem device, implementing `modem.v3`.
pub const MODEM: &str = "site-a/rf0";
/// modem's `sdu/tx_bytes_total`: a bare JSON integer, a counter in `By` by
/// its declaration alone.
pub const TX_BYTES: &str = "zk2/site-a/rf0/modem.v3/stream/sdu/tx_bytes_total";
/// modem's `radio/rssi`: a bare JSON number, a gauge in `dBm`.
pub const RSSI: &str = "zk2/site-a/rf0/modem.v3/stream/radio/rssi";

/// The instant tick 0's Points were measured at, Unix epoch milliseconds;
/// a tick is 250 ms.
const MEASURED_T0_MS: i64 = 1_792_400_000_000;

/// A Point of `kind` measured at tick `n`, carrying `value`, and `epoch`
/// and `unit` when given.
pub fn metric_point(
    n: u32,
    kind: &str,
    value: serde_json::Value,
    unit: Option<&str>,
    epoch: Option<&str>,
) -> Vec<u8> {
    let mut p = serde_json::json!({
        "timestamp": MEASURED_T0_MS + 250 * i64::from(n),
        "value": {"type": kind, "value": value},
    });
    if let Some(u) = unit {
        p["unit"] = serde_json::json!(u);
    }
    if let Some(e) = epoch {
        p["epoch"] = serde_json::json!(e);
    }
    serde_json::to_vec(&p).expect("a Point")
}

/// A counter's count `k` steps into an epoch: never decreasing, its steps
/// uneven so its rate moves.
fn count(k: u32) -> u64 {
    40 * u64::from(k) + 10 * u64::from(k % 4)
}

/// `(key, payload and attachment at tick n, encoding)`, one row per
/// metered publisher.
type MeteredFeed = (
    &'static str,
    fn(u32) -> (Vec<u8>, Option<Vec<u8>>),
    &'static str,
);

/// The metered publishers (#819, TD): what `lab/meter` and `site-a/rf0`
/// put each tick, each conforming to its declaration.
const METERED_FEEDS: &[MeteredFeed] = &[
    (
        FRAMES,
        |n| {
            let (epoch, k) = if n < FRAMES_RESET_AT {
                ("00000000000000a1", n)
            } else {
                ("00000000000000a2", n - FRAMES_RESET_AT)
            };
            (
                metric_point(
                    n,
                    "counter",
                    serde_json::json!(count(k)),
                    Some("1"),
                    Some(epoch),
                ),
                None,
            )
        },
        "application/json",
    ),
    (
        BYTES,
        |n| {
            let v = if n < BYTES_DROP_AT {
                1500 * u64::from(n) + 300 * u64::from(n % 3)
            } else {
                9000 + 1500 * u64::from(n - BYTES_DROP_AT)
            };
            (
                v.to_string().into_bytes(),
                Some(br#""00000000000000b1""#.to_vec()),
            )
        },
        "application/json",
    ),
    (
        DROPS,
        |n| {
            let v = if n < 14 { n / 2 } else { (n - 14) / 3 };
            (v.to_string().into_bytes(), None)
        },
        "application/json",
    ),
    (
        LATENCY,
        |n| {
            let n = u64::from(n);
            let counts = [n, 2 * n, n, n / 4];
            let total: u64 = counts.iter().sum();
            let mean = 0.0105 + 0.0025 * (n as f64 * 0.45).sin();
            let value = serde_json::json!({
                "buckets": [0.005, 0.01, 0.025],
                "counts": counts,
                "count": total,
                "sum": (total as f64 * mean * 1e6).round() / 1e6,
            });
            (
                metric_point(
                    n as u32,
                    "histogram",
                    value,
                    Some("s"),
                    Some("00000000000000a1"),
                ),
                None,
            )
        },
        "application/json",
    ),
    (
        LEVEL,
        |n| {
            let v = -71.0 + 6.0 * (f64::from(n) * 0.4).sin();
            (format!("{v:.1}").into_bytes(), None)
        },
        "application/json",
    ),
    (
        TX_BYTES,
        |n| {
            let v = 4096 * u64::from(n) + 512 * u64::from(n % 5);
            (v.to_string().into_bytes(), None)
        },
        "application/json",
    ),
    (
        RSSI,
        |n| {
            let v = -67.5 + 3.0 * (f64::from(n) * 0.3).cos();
            (format!("{v:.1}").into_bytes(), None)
        },
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
    addresses: &[(&str, &str)],
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
    for (id, (address, iface)) in (1u64..).zip(addresses.iter().copied()) {
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
                    standby: false,
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
        World::build(contracts(), &World::ADDRESSES, false)
    }

    /// The world with `lab/meter` and `site-a/rf0` beside it (#819, TD):
    /// [`metered_contract_paths`] loaded, both addresses placed by presence,
    /// and their metric streams publishing each tick ([`METERED_FEEDS`]).
    pub fn metered() -> World {
        let contracts = Arc::new(zengui::services::contracts::load_all(
            &metered_contract_paths(),
        ));
        let mut addresses = World::ADDRESSES.to_vec();
        addresses.push((METER, "meter.v1"));
        addresses.push((MODEM, "modem.v3"));
        World::build(contracts, &addresses, true)
    }

    fn build(
        contracts: Arc<zengui::services::contracts::LoadedContracts>,
        addresses: &[(&str, &str)],
        metered: bool,
    ) -> World {
        let (catalog, presence, seeded, observed) = deployment(&contracts.set, addresses);
        let lens =
            zenkey_fleet::Lens::new("", Some(&catalog), &*contracts.set).offline(&contracts.set);
        let axes = FEEDS
            .iter()
            .chain(REFUSED_FEEDS)
            .map(|(key, ..)| *key)
            .chain(SYNTH_FEEDS.iter().map(|(key, ..)| *key))
            .chain(METERED_FEEDS.iter().map(|(key, ..)| *key))
            .map(|key| (key, Axes::declared(&lens, key)))
            .collect();
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
            axes,
            metered,
            last: Vec::new(),
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

    /// A presence landing for this world with an archive beside it (#614,
    /// FL8a): `ground/archive` holding its instance token and an
    /// `archive.v1` interface token, so a last-known read has a provider to
    /// ask. The lens moves with it (generation 2).
    pub fn presence_update_with_archive(&self) -> Arc<zengui::presence_link::PresenceUpdate> {
        use zenkey_model::grammar::{Addr, InstanceId, Sha256Hex, ZkKey};
        let addr: Addr = "ground/archive".parse().expect("an address");
        let instance = InstanceId::from_u64(9);
        let fp = Sha256Hex::new(&"cd".repeat(32))
            .expect("a fingerprint")
            .fp16();
        let mut observed = self.observed.clone();
        observed.with_token(ZkKey::Instance {
            addr: addr.clone(),
            instance: instance.clone(),
        });
        observed.with_token(ZkKey::Alive {
            addr,
            iface: zengui::value::ARCHIVE.clone(),
            instance,
            fp,
        });
        let mut presence = zenkey_fleet::LivePresence::new(zenkey_fleet::PresenceScope::all());
        let moved = presence.seed(&observed, std::time::SystemTime::now());
        Arc::new(zengui::presence_link::PresenceUpdate {
            presence: Arc::new(presence),
            moved,
            catalog: Some(Arc::new(zenkey_fleet::Catalog::new(&observed))),
            generation: 2,
            seed: zenkey_fleet::SeedOutcome::Complete,
        })
    }

    /// The world's presence read with `lab/rf0` beside it (#768, CD): its
    /// instance token, its `config.v1` interface token, and a descriptor
    /// that lists `config.v1` — the gate the profile names (§2.1) — and
    /// `freshness.v1`, which the contract uses (core §3.3).
    pub fn observed_with_config(&self) -> zenkey_fleet::model::catalog::Observed {
        use std::collections::BTreeMap;
        use zenkey_fleet::model::catalog::DescriptorRead;
        use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
        use zenkey_model::grammar::{Addr, InstanceId, Sha256Hex, ZkKey};
        let addr: Addr = RF0.parse().expect("an address");
        let instance: InstanceId = RF0_INSTANCE.parse().expect("an instance id");
        let fp = Sha256Hex::new(zenkey::config_v1::FINGERPRINT)
            .expect("a fingerprint")
            .fp16();
        let mut observed = self.observed.clone();
        observed.with_token(ZkKey::Instance {
            addr: addr.clone(),
            instance: instance.clone(),
        });
        observed.with_token(ZkKey::Alive {
            addr: addr.clone(),
            iface: zenkey::config_v1::iface(),
            instance: instance.clone(),
            fp,
        });
        let mut descriptors = observed.descriptors.clone().unwrap_or_default();
        descriptors.insert(
            (addr, instance),
            DescriptorRead::Served(Box::new(Descriptor {
                format: "zk2-descriptor/0.1".into(),
                service: RF0.into(),
                instance: RF0_INSTANCE.into(),
                interfaces: vec![InterfaceEntry {
                    iface: zenkey_model::config::IFACE.into(),
                    contract: format!("sha256:{}", zenkey::config_v1::FINGERPRINT),
                    minor: 0,
                    standby: false,
                    token: true,
                    unavailable: Vec::new(),
                    cardinality: BTreeMap::new(),
                }],
                capabilities: Vec::new(),
                requires: Vec::new(),
                profiles: vec!["freshness.v1".into()],
                meta: BTreeMap::new(),
            })),
        );
        observed.descriptors = Some(descriptors);
        observed
    }

    /// This world's presence with `lab/catalog` beside it (#841, RC): the
    /// incumbent holding `zs.catalog.v1`'s token, the standby holding its
    /// instance token alone, its descriptor marking the interface standby.
    pub fn observed_with_catalog(&self) -> zenkey_fleet::model::catalog::Observed {
        use std::collections::BTreeMap;
        use zenkey_fleet::model::catalog::DescriptorRead;
        use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
        use zenkey_model::grammar::{Addr, InstanceId, ZkKey};
        let addr: Addr = CATALOG.parse().expect("an address");
        let rev = catalog_revision();
        let mut observed = self.observed.clone();
        let mut descriptors = observed.descriptors.clone().unwrap_or_default();
        for (id, serving) in [(CATALOG_SERVING, true), (CATALOG_STANDBY, false)] {
            let instance: InstanceId = id.parse().expect("an instance id");
            observed.with_token(ZkKey::Instance {
                addr: addr.clone(),
                instance: instance.clone(),
            });
            if serving {
                observed.with_token(ZkKey::Alive {
                    addr: addr.clone(),
                    iface: rev.iface().clone(),
                    instance: instance.clone(),
                    fp: rev.fingerprint().hex().fp16(),
                });
            }
            descriptors.insert(
                (addr.clone(), instance),
                DescriptorRead::Served(Box::new(Descriptor {
                    format: "zk2-descriptor/0.1".into(),
                    service: CATALOG.into(),
                    instance: id.into(),
                    interfaces: vec![InterfaceEntry {
                        iface: rev.iface().to_string(),
                        contract: rev.fingerprint().to_string(),
                        minor: 0,
                        standby: !serving,
                        token: true,
                        unavailable: Vec::new(),
                        cardinality: BTreeMap::new(),
                    }],
                    capabilities: Vec::new(),
                    requires: Vec::new(),
                    profiles: vec!["redundancy.v1".into(), "freshness.v1".into()],
                    meta: BTreeMap::from([
                        ("zid".to_owned(), serde_json::json!(format!("{id}0"))),
                        ("host".to_owned(), serde_json::json!("catalog-01")),
                    ]),
                })),
            );
        }
        observed.descriptors = Some(descriptors);
        observed
    }

    /// A presence landing for this world with `lab/catalog` beside it. The
    /// lens moves with it (generation 2).
    pub fn presence_update_with_catalog(&self) -> Arc<zengui::presence_link::PresenceUpdate> {
        let observed = self.observed_with_catalog();
        let mut presence = zenkey_fleet::LivePresence::new(zenkey_fleet::PresenceScope::all());
        let moved = presence.seed(&observed, std::time::SystemTime::now());
        Arc::new(zengui::presence_link::PresenceUpdate {
            presence: Arc::new(presence),
            moved,
            catalog: Some(Arc::new(zenkey_fleet::Catalog::new(&observed))),
            generation: 2,
            seed: zenkey_fleet::SeedOutcome::Complete,
        })
    }

    /// `lab/catalog`'s `redundancy.v1` reading, judged by the engine from
    /// two reads of [`World::observed_with_catalog`] a grace apart, the
    /// catalog's revision in hand: one incumbent, one standby.
    pub fn catalog_redundancy(&self) -> Arc<zenkey_fleet::report::RedundancyReport> {
        use zenkey_fleet::model::catalog::ContractState;
        let rev = catalog_revision();
        let read = self.observed_with_catalog();
        let mut contracts = std::collections::BTreeMap::new();
        contracts.insert(
            (rev.iface().clone(), rev.fingerprint().clone()),
            Ok(ContractState::Held(Arc::clone(&rev))),
        );
        let obs = zenkey_fleet::RedundancyObservation {
            cut_off: None,
            first: Some(Ok(read.clone())),
            second: Some(Ok(read)),
            apart: Duration::from_secs(2),
            contracts,
            asked: vec!["zk2/lab/catalog/@zk/**".into()],
        };
        let addr: zenkey_model::grammar::Addr = CATALOG.parse().expect("an address");
        Arc::new(zenkey_fleet::judge_redundancy(
            "",
            Some(&addr),
            &zenkey_fleet::RedundancySpec::new(Duration::from_secs(2)),
            &obs,
        ))
    }

    /// A presence landing for this world with `lab/rf0` beside it, the
    /// `config.v1` owner (#768, CD). The lens moves with it (generation 2).
    pub fn presence_update_with_config(&self) -> Arc<zengui::presence_link::PresenceUpdate> {
        let observed = self.observed_with_config();
        let mut presence = zenkey_fleet::LivePresence::new(zenkey_fleet::PresenceScope::all());
        let moved = presence.seed(&observed, std::time::SystemTime::now());
        Arc::new(zengui::presence_link::PresenceUpdate {
            presence: Arc::new(presence),
            moved,
            catalog: Some(Arc::new(zenkey_fleet::Catalog::new(&observed))),
            generation: 2,
            seed: zenkey_fleet::SeedOutcome::Complete,
        })
    }

    /// `lab/rf0`'s view (§2.3), the profile's `rf` owner as the scenarios
    /// run it (`scenarios.md`'s conventions), at revision 3: `duty` hot,
    /// its fraction a runtime change a restart would not keep; `sim` hot,
    /// its `pin` sensitive — declared, sourced, never valued (§2.15);
    /// `queue` outside the allowlist; `radio` reach; `modulation` contract.
    /// The last change set `duty`; with `pending`, a reach change on
    /// `radio` is pending and applied, its deadline a minute after
    /// [`config_read_at`]. Its desired report: revision 4, converged.
    pub fn config_view(&self, pending: bool) -> zenkey_model::config::ConfigView {
        let mut v = serde_json::json!({
            "service": RF0, "instance": RF0_INSTANCE, "revision": 3, "confirm_floor_s": 10,
            "last_change": {"token": format!("{RF0_INSTANCE}-2"), "groups": ["duty"]},
            "desired": {"observed_revision": 4, "outcome": "converged"},
            "groups": [
                {"name": "duty", "class": "hot", "description": "the transmit duty cycle",
                 "writable": true,
                 "parameters": [{"name": "fraction_ppm", "kind": "integer", "min": 0,
                                 "max": 1_000_000, "unit": "ppm",
                                 "description": "the share of airtime", "value": 20_000,
                                 "source": "runtime", "startup": 10_000}]},
                {"name": "radio", "class": "reach", "description": "the radio channel",
                 "writable": true,
                 "parameters": [{"name": "frequency_hz", "kind": "integer",
                                 "min": 150_000_000, "max": 960_000_000, "unit": "Hz",
                                 "description": "the carrier frequency",
                                 "value": 868_100_000, "source": "file"}]},
                {"name": "sim", "class": "hot", "description": "the SIM", "writable": true,
                 "parameters": [{"name": "pin", "kind": "text",
                                 "description": "the SIM's PIN", "sensitive": true,
                                 "source": "file"}]},
                {"name": "queue", "class": "hot", "description": "the transmit queue",
                 "writable": false,
                 "parameters": [{"name": "tx_queue_len", "kind": "integer", "min": 0,
                                 "unit": "packets", "description": "the queue's length",
                                 "value": 100, "source": "default"}]},
                {"name": "modulation", "class": "contract",
                 "description": "what the transport was started against", "writable": true,
                 "parameters": [{"name": "spreading_factor", "kind": "integer", "min": 7,
                                 "max": 12, "description": "the LoRa spreading factor",
                                 "value": 9, "source": "file"}]}
            ]
        });
        if pending {
            v["revision"] = serde_json::json!(4);
            v["pending"] = serde_json::json!({
                "token": format!("{RF0_INSTANCE}-3"), "groups": ["radio"],
                "deadline": "2026-10-18T09:01:00.000Z", "applied": true
            });
            v["groups"][1]["parameters"][0]["value"] = serde_json::json!(869_525_000);
            v["groups"][1]["parameters"][0]["source"] = serde_json::json!("runtime");
            v["groups"][1]["parameters"][0]["startup"] = serde_json::json!(868_100_000);
        }
        serde_json::from_value(v).expect("a ConfigView")
    }

    /// The change events of `lab/rf0`'s first two changes (§2.17): a `sim`
    /// change whose pin edit is redacted, rolled back at its deadline, then
    /// `duty` applied without a window.
    pub fn config_events(&self) -> Vec<zenkey_model::config::ConfigChangeEvent> {
        serde_json::from_value(serde_json::json!([
            {"service": RF0, "instance": RF0_INSTANCE, "revision": 2,
             "token": format!("{RF0_INSTANCE}-1"), "outcome": "rolled-back",
             "edits": [{"group": "sim", "parameter": "pin", "redacted": true}],
             "actor": "ops"},
            {"service": RF0, "instance": RF0_INSTANCE, "revision": 3,
             "token": format!("{RF0_INSTANCE}-2"), "outcome": "applied",
             "edits": [{"group": "duty", "parameter": "fraction_ppm", "old": 10_000,
                        "new": 20_000}],
             "actor": "ops"}
        ]))
        .expect("change events")
    }

    /// A reading of `lab/rf0` as the engine judges it (#768, CD): presence
    /// and the descriptor as [`World::observed_with_config`] holds them, the
    /// GET answered with `view` and its status stamped four seconds before
    /// [`config_read_at`], the operator's word on the clocks (so a stamp's
    /// age is judged), and `events` replayed for the change `token` names.
    pub fn config_report(
        &self,
        view: &zenkey_model::config::ConfigView,
        events: Vec<zenkey_model::config::ConfigChangeEvent>,
        token: Option<&str>,
    ) -> Arc<zenkey_fleet::report::ConfigReport> {
        use zenkey_fleet::bus::config::{ConfigReplay, ServiceDocs};
        use zenkey_fleet::bus::health::{Replied, Stamped};
        use zenkey_model::config::ConfigStatus;
        let addr: zenkey_model::grammar::Addr = RF0.parse().expect("an address");
        let stamp = Some(Stamped {
            time: config_read_at() - Duration::from_secs(4),
            clock: "a1b2c3d4e5f60718".into(),
        });
        let docs = ServiceDocs {
            view: Some(Replied {
                value: Some(Ok(view.clone())),
                stamp: stamp.clone(),
            }),
            status: Some(Replied {
                value: Some(Ok(ConfigStatus::of(view))),
                stamp,
            }),
        };
        let target = zenkey_fleet::ConfigTarget::One(addr.clone());
        let mut spec = zenkey_fleet::ConfigSpec::new(Duration::from_secs(5));
        spec.clocks_synced = true;
        spec.token = token.map(str::to_owned);
        spec.instance = token.map(|_| RF0_INSTANCE.to_owned());
        let replay = token.map(|_| {
            Ok(ConfigReplay {
                selector: format!("zk2/{RF0}/config.v1/events/change/*"),
                events,
                undecodable: 0,
            })
        });
        Arc::new(zenkey_fleet::judge_config(
            &zenkey_fleet::ConfigObservation {
                namespace: String::new(),
                address: addr.clone(),
                spec,
                presence: Some(Ok(self.observed_with_config())),
                get: Ok(zenkey_fleet::ConfigGet {
                    selector: target.state_selector(false),
                    read_at: config_read_at(),
                    complete: true,
                    services: [(addr, docs)].into(),
                }),
                window: None,
                replay,
            },
        ))
    }

    /// Every publisher's payload at tick `n`.
    fn payloads(&self, n: u32) -> Vec<Published> {
        let mut out: Vec<Published> = FEEDS
            .iter()
            .map(|(key, payload, encoding)| (*key, payload(n), *encoding, None))
            .collect();
        for (key, iface, token, template, encoding) in SYNTH_FEEDS {
            let bytes = synth(&self.contracts.set, iface, token, template, n);
            out.push((key, bytes, encoding, None));
        }
        if self.metered {
            for (key, payload, encoding) in METERED_FEEDS {
                let (bytes, attachment) = payload(n);
                out.push((key, bytes, encoding, attachment));
            }
        }
        out.extend(
            REFUSED_FEEDS
                .iter()
                .map(|(key, payload, encoding)| (*key, payload(n), *encoding, None)),
        );
        out
    }

    /// The lens the window resolves with once connected — the same inputs,
    /// session-free — for the checks a scene answers itself.
    pub fn check(&self, key: &str, bytes: &[u8], encoding: &str) -> zenkey_fleet::Checked {
        self.check_attached(key, bytes, encoding, None)
    }

    /// [`World::check`] with the sample's attachment (#819, TD): a bare
    /// counter's in-band epoch is read there (`telemetry.v1` §2.4).
    pub fn check_attached(
        &self,
        key: &str,
        bytes: &[u8],
        encoding: &str,
        attachment: Option<&[u8]>,
    ) -> zenkey_fleet::Checked {
        zenkey_fleet::Lens::new("", Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set)
            .check_sample(
                key,
                zenkey_fleet::Member::Type,
                (!encoding.is_empty()).then_some(encoding),
                bytes,
                attachment,
            )
    }

    /// What the last tick published: `(key, bytes, encoding, attachment)`.
    pub fn published(&self) -> impl Iterator<Item = (&str, &[u8], &str, Option<&[u8]>)> {
        self.last
            .iter()
            .map(|(k, b, e, a)| (*k, b.as_slice(), *e, a.as_deref()))
    }

    /// The metered publishers' keys (#819, TD).
    pub fn metered_keys() -> impl Iterator<Item = &'static str> {
        METERED_FEEDS.iter().map(|(key, ..)| *key)
    }

    /// The contracts this world loaded, as `ContractsLoaded` hands them.
    pub fn loaded(&self) -> Arc<zengui::services::contracts::LoadedContracts> {
        Arc::clone(&self.contracts)
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

    /// A state read of `key` as the window's task lands it (#614, FL8a):
    /// the S4 report the fleet hands back — its row rendered through the
    /// world's contract, stamped — or an archive's last-known one, checked
    /// and attributed through the world's lens. `body: None` is silence.
    pub fn state_read(
        &self,
        key: &str,
        body: Option<&[u8]>,
        archive: Option<&str>,
    ) -> zengui::value::StateAnswer {
        use zenkey_fleet::report::{StateReading, StateReport, StateRow, StateValue};
        let resolution = self.resolve(key);
        let resolved = resolution.resolved.as_ref().expect("a state key resolves");
        let rows = body
            .map(|body| StateRow {
                key: key.to_string(),
                value: StateValue::Value {
                    payload: Box::new(self.check(key, body, "").rendering),
                },
                bytes: Some(body.to_vec().into()),
                timestamp: Some(zenkey_fleet::report::Stamp {
                    time: "2026-10-05T09:12:44.250000000Z".into(),
                    clock: "a1b2c3d4e5f60718".into(),
                }),
                confirmed: archive.map(|_| true),
                identity: None,
            })
            .into_iter()
            .collect();
        let report = StateReport {
            reading: match archive {
                Some(_) => StateReading::LastKnown,
                None => StateReading::Current,
            },
            address: resolved.address.to_string(),
            iface: resolved.iface.to_string(),
            fingerprint: resolved.revision.fingerprint().to_string(),
            resource: resolved.resource_name(),
            values: resolved.values.clone(),
            selectors: vec![key.to_string()],
            archive: archive.map(str::to_string),
            timeout_s: 5.0,
            rows,
        };
        let lens = zenkey_fleet::Lens::new("", Some(&self.catalog), &*self.contracts.set)
            .offline(&self.contracts.set);
        let owner = resolved.address.to_string();
        zengui::value::StateAnswer::new(report, &resolution, |clock| {
            lens.provenance(Some(&owner), clock)
        })
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
        let batch = self
            .last
            .iter()
            .map(|(key, bytes, encoding, attachment)| {
                (
                    key.to_string(),
                    self.check_attached(key, bytes, encoding, attachment.as_deref())
                        .conformance,
                )
            })
            .collect();
        send(app, Message::Bus(BusMsg::VerdictsChecked(batch)));
    }

    /// One 250 ms monitor tick: every feed publishes once, on its axes, and
    /// the tick carries the snapshot, the samples and the counters a real
    /// one would.
    pub fn tick(&mut self, app: &mut Zengui) {
        self.tick_inner(app, None, None);
    }

    /// One tick in which `key` publishes `bytes` in place of its feed's
    /// payload (#819, TD): a scene's one odd sample — a Point whose tag
    /// disagrees with its declaration — as the newest the key holds.
    pub fn tick_with(&mut self, app: &mut Zengui, key: &'static str, bytes: Vec<u8>) {
        self.tick_inner(app, None, Some((key, bytes)));
    }

    /// One tick in which, after every feed, a stranger puts `key`'s payload
    /// again, raw, on zenoh's default axes (#614, FL12): the second writer
    /// on an owned key that the spray sends and no owner writing through
    /// the runtime would. The key's newest sample is the stranger's, so its
    /// Inspector compares the declared QoS against axes the owner did not
    /// choose.
    pub fn tick_with_stranger(&mut self, app: &mut Zengui, key: &'static str) {
        self.tick_inner(app, Some(key), None);
    }

    fn tick_inner(
        &mut self,
        app: &mut Zengui,
        stranger: Option<&'static str>,
        swap: Option<(&'static str, Vec<u8>)>,
    ) {
        let n = self.ticks;
        let at = self.t0 + Duration::from_millis(250) * (n + 1);
        let mut samples = Vec::new();
        let mut payloads = self.payloads(n);
        if let Some((key, bytes)) = swap {
            let row = payloads
                .iter_mut()
                .find(|p| p.0 == key)
                .unwrap_or_else(|| panic!("{key} is no publisher of this world"));
            row.1 = bytes;
        }
        let feeds = payloads.len();
        let mut stray = None;
        for (key, body, encoding, attachment) in &payloads {
            self.stats.record(key, body.len(), None, at, None, None);
            let axes = self.axes.get(key).copied().unwrap_or(Axes::DEFAULT);
            samples.push(sample_attached(
                key,
                body,
                encoding,
                at,
                axes,
                attachment.as_deref(),
            ));
            if stranger == Some(*key) {
                stray = Some(sample(key, body, encoding, at));
            }
        }
        self.last = payloads;
        if let Some(key) = stranger {
            let stray = stray.unwrap_or_else(|| panic!("{key} is no publisher of this world"));
            self.stats
                .record(key, stray.payload.len(), None, at, None, None);
            samples.push(stray);
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
