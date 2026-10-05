//! Fixtures shared by the integration suites (#532).
//!
//! `panes.rs` renders surfaces one at a time; `shots.rs` renders the whole
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
use zenkey_fleet::model::stats::StatsTable;
use zenkey_fleet::{KeyTreeSnapshot, SampleView, SliceSet};

/// The ZenSight registry snapshot `fixture-tests/` compiles — the codegen
/// corpus doubles as the GUI's declared world.
pub fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

/// Settings for a window with no bus behind it: nothing to dial, no
/// registry dirs (slices arrive as a message instead), the default scope.
pub fn settings() -> Settings {
    Settings {
        base: String::new(),
        connect: vec!["tcp/10.0.4.2:7447".into()],
        listen: vec![],
        scouting: None,
        zenoh_config: None,
        registry: vec![],
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
/// have: registered leaves under two origins, an unregistered leaf under a
/// conforming origin, foreign keys from a non-convention publisher and from
/// another deployment, a state leaf that retires, and a numeric leaf whose
/// value wanders so a series has something to plot.
pub struct World {
    stats: StatsTable,
    t0: Instant,
    ticks: u32,
}

pub const ORIGIN_A: &str = "h-3fa9c2d41b7e";
pub const ORIGIN_B: &str = "h-9c04d2e1a7f3";
pub const USED: &str = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var-log/used";
pub const HEALTH: &str = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

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

fn json_point(value: f64) -> Vec<u8> {
    format!(r#"{{"value":{value:.2},"unit":"%","ts":"2026-10-05T09:12:44Z"}}"#).into_bytes()
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> World {
        World {
            stats: StatsTable::new(),
            // Far enough back that every rate has settled and every age
            // reads as a plausible live bus.
            t0: Instant::now()
                .checked_sub(Duration::from_secs(40))
                .unwrap_or_else(Instant::now),
            ticks: 0,
        }
    }

    /// The window as the link leaves it once the session is up: registry
    /// in hand, the pump running.
    pub fn connect(&self, app: &mut Zengui) {
        send(
            app,
            Message::Bus(BusMsg::SlicesLoaded(Ok(Arc::new(slices())))),
        );
        send(app, Message::Bus(BusMsg::Link(LinkState::Pumping)));
    }

    /// One 250 ms monitor tick: every feed publishes once, and the tick
    /// carries the snapshot, the samples and the counters a real one would.
    pub fn tick(&mut self, app: &mut Zengui) {
        let n = self.ticks;
        let at = self.t0 + Duration::from_millis(250) * (n + 1);
        let mut samples = Vec::new();
        for (key, payload, encoding) in FEEDS {
            let body = payload(n);
            self.stats.record(key, body.len(), None, at, None, None);
            samples.push(sample(key, &body, encoding, at));
        }
        // Every 12th tick the health leaf retires and comes back (RFC 04
        // §1.2): the tree, the echo and the history all have a tombstone to
        // draw.
        if n % 12 == 11 {
            samples.push(tombstone(HEALTH, at));
        }
        let nodes = if n == 0 {
            vec![
                (format!("v1/{ORIGIN_A}/state/sysinfo/alive"), true),
                (format!("v1/{ORIGIN_B}/state/sysinfo/alive"), true),
            ]
        } else {
            vec![]
        };
        let totals = WatchedTotals {
            samples: u64::from(n + 1) * FEEDS.len() as u64,
            bytes: u64::from(n + 1) * 420,
            rate_hz: FEEDS.len() as f64 * 4.0,
        };
        let tick = BusTick {
            tree: Arc::new(KeyTreeSnapshot::build(&self.stats)),
            samples,
            lagged: 0,
            coalesced: 0,
            nodes,
            keys: FEEDS.len(),
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

/// A read-back with every anatomy the Config tool draws (#481): a group of
/// each class, a sensitive parameter, a value differing from its startup
/// one, one with no value read back, a change pending on the reach group
/// and the last change on the hot one.
pub fn config_view() -> zenkey::config::ConfigView {
    use zenkey::config::{
        ConfigGroup, ConfigSchema, ConfigView, LastChange, ParamClass, ParamKind, ParamSpec,
        ParamValue, PendingChange, ValueSource,
    };
    let schema = ConfigSchema::new()
        .with(
            ConfigGroup::new("queue", ParamClass::Hot, "the transmit queue")
                .with(ParamSpec::new(
                    "tx_queue_len",
                    ParamKind::Integer {
                        min: Some(1),
                        max: Some(10_000),
                        unit: Some("packets".into()),
                    },
                    "packets queued in front of the radio",
                ))
                .with(ParamSpec::new("fq", ParamKind::Bool, "fair queueing"))
                .with(ParamSpec::new("psk", ParamKind::Text, "the network key").sensitive()),
        )
        .with(
            ConfigGroup::new("link", ParamClass::Reach, "how the node reaches the bus").with(
                ParamSpec::new("ssid", ParamKind::Text, "the network joined"),
            ),
        )
        .with(
            ConfigGroup::new(
                "transport",
                ParamClass::Contract,
                "what the transport was started against",
            )
            .with(ParamSpec::new(
                "mtu",
                ParamKind::Integer {
                    min: None,
                    max: None,
                    unit: Some("bytes".into()),
                },
                "the SDU size",
            )),
        );
    let mut view = ConfigView::of("wlan0", &schema);
    view.revision = 4;
    view.pending = Some(PendingChange::new("chg-2", ["link"]).until("2026-10-05T12:00:00Z"));
    view.last_change = Some(LastChange::new("chg-1", ["queue"]));
    let set = |view: &mut ConfigView, g: usize, p: usize, v: ParamValue, s: ValueSource| {
        view.groups[g].parameters[p].value = Some(v);
        view.groups[g].parameters[p].source = Some(s);
    };
    set(
        &mut view,
        0,
        0,
        ParamValue::Integer(2000),
        ValueSource::Overlay,
    );
    view.groups[0].parameters[0].startup = Some(ParamValue::Integer(1000));
    set(&mut view, 0, 1, ParamValue::Bool(false), ValueSource::File);
    view.groups[0].parameters[2].source = Some(ValueSource::File);
    set(
        &mut view,
        1,
        0,
        ParamValue::Text("field".into()),
        ValueSource::Runtime,
    );
    view
}
