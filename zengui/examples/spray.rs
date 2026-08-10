//! A traffic generator for exercising zengui by hand.
//!
//! Neither zenkey nor zensight can produce *non-conforming* traffic — that is
//! the whole point of them — so there was no way to test the key-agnostic path
//! against a real bus. This publishes both kinds at once:
//!
//! - keyspace-v2 keys that parse and (against `fixture-tests/registry`)
//!   resolve to registered subjects, plus one that deliberately does not;
//! - plain foreign keys, a `v2/` key, a key under another base, and a short
//!   two-chunk key — one for each `KeyShape` / `UnparsedReason` arm;
//! - a large payload, to exercise the echo ring's byte budget and the
//!   "too large to preview" path;
//! - **payloads that move** (issues #63/#64): a wandering numeric telemetry
//!   subject to plot and to diff, and a state key that is retired by tombstone
//!   and revived on a cycle. Everything else stays byte-stable, because the
//!   tree and badge demos are about keys and a moving payload would be noise;
//! - a `@media`-shaped key, which a `**` subscriber must *not* see (RFC 03 §4
//!   D2) — if it shows up in the tree, the scope design is broken;
//! - a **protobuf** subject with a producer that serves both RFC 08 halves
//!   (`introspect` and `describe`, issue #97): the leaf decodes to named
//!   fields with no registry directory involved, and it is the target the
//!   publish pane can actually be pointed at. It is served from the bus rather
//!   than added to `fixture-tests/registry` on purpose — that directory is the
//!   codegen regression corpus, not a demo prop.
//!
//! ```text
//! cargo run -p zengui --example spray -- -l tcp/127.0.0.1:7449   # listen, no router
//! cargo run -p zengui --example spray -- -c tcp/127.0.0.1:7447   # or join one
//! ```

use std::time::Duration;

use clap::Parser;

#[derive(Parser)]
#[command(about = "publish mixed conforming and foreign traffic for zengui")]
struct Args {
    /// Endpoint to connect to, repeatable.
    #[arg(long, short = 'c')]
    connect: Vec<String>,
    /// Endpoint to listen on, repeatable.
    ///
    /// Listening lets zengui connect straight to this process, so the demo
    /// needs no router: two cargo commands and nothing else installed.
    #[arg(long, short = 'l')]
    listen: Vec<String>,
    /// Deployment base for the conforming keys. Empty = the bus root.
    #[arg(long, default_value = "")]
    base: String,
    /// Publications per second, per key.
    #[arg(long, default_value_t = 5.0)]
    hz: f64,
    /// Extra synthetic telemetry keys (`v1/<host>/telemetry/synth/g<n>/k<i>`,
    /// grouped in hundreds) — the big-tree soak for issue #65. Each publishes
    /// once at startup so the tree fills, then a rotating slice refreshes on
    /// every tick so freshness dots keep moving.
    #[arg(long, default_value_t = 0)]
    keys: usize,
}

/// The `probe` producer's introspect slice: one subject, declared protobuf.
const PROBE_SLICE: &str = r#"
[registry]
version = "1.0"
app = "spray"
convention = 1

[producer]
name = "probe"
description = "demo producer serving a protobuf-encoded subject (#97)"

[[subject]]
path = "reading"
class = "telemetry"
type = "Reading"
encoding = "application/protobuf"
since = "1.0"
description = "a protobuf payload — decodable only via the served descriptor set"
"#;

/// `package demo; message Reading { double value = 1; string label = 2; }`,
/// built through prost-types so the fixture reads as the schema it is.
fn probe_descriptor_set() -> Vec<u8> {
    use prost_reflect::prost::Message as _;
    use prost_reflect::prost_types::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
        field_descriptor_proto,
    };
    let field = |name: &str, number: i32, ty: field_descriptor_proto::Type| FieldDescriptorProto {
        name: Some(name.to_string()),
        number: Some(number),
        label: Some(field_descriptor_proto::Label::Optional as i32),
        r#type: Some(ty as i32),
        json_name: Some(name.to_string()),
        ..Default::default()
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("demo.proto".into()),
            package: Some("demo".into()),
            message_type: vec![DescriptorProto {
                name: Some("Reading".into()),
                field: vec![
                    field("value", 1, field_descriptor_proto::Type::Double),
                    field("label", 2, field_descriptor_proto::Type::String),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

/// The `describe` reply (RFC 08 §7).
fn probe_schema_set() -> String {
    zenkey::schema::SchemaSet::builder("spray")
        .entry(
            "Reading",
            zenkey::schema::TypeSchema::protobuf("demo.Reading", &probe_descriptor_set()),
        )
        .build()
        .to_json()
}

/// One `Reading`, on the wire. Deliberately *not* JSON: every sniff a generic
/// tool has renders this as bytes, which is the point of serving a schema.
fn probe_sample(value: f64) -> Vec<u8> {
    use prost_reflect::prost::Message as _;
    let pool = prost_reflect::DescriptorPool::decode(probe_descriptor_set().as_slice())
        .expect("fixture descriptor set");
    let desc = pool
        .get_message_by_name("demo.Reading")
        .expect("fixture message");
    let mut msg = prost_reflect::DynamicMessage::new(desc);
    msg.set_field_by_name("value", prost_reflect::Value::F64(value));
    msg.set_field_by_name("label", prost_reflect::Value::String("probe".into()));
    msg.encode_to_vec()
}

/// How a plan entry's payload is produced on each tick.
///
/// Most of the fixture is byte-stable, because the tree, badge and scope demos
/// are about *keys* and a moving payload would only be noise. The three that
/// move exist for #63/#64: a diff of two identical payloads shows nothing, and
/// a sparkline of a constant is a flat line — neither demonstrates anything.
enum Payload {
    /// Identical every tick.
    Fixed(Vec<u8>),
    /// A JSON document whose `value` wanders — the plotting subject.
    Wander,
    /// The protobuf `Reading`, wandering too: proof that the codec path is
    /// live, and the thing that shows a schema-decoded leaf offers no chart.
    Reading,
    /// A state key that is periodically **retired and revived**: puts, then a
    /// tombstone, then puts again. The only way to see RFC 04 §1.2's
    /// distinction by hand.
    Cycle,
}

/// How many ticks one retire/revive cycle takes; the last tick of each is the
/// tombstone.
const CYCLE: u64 = 20;

/// A deterministic wander in `[lo, hi]` — a fixture should look the same on
/// every run, so this is a sine of the tick, not a random number.
fn wander(tick: u64, lo: f64, hi: f64) -> f64 {
    let t = (tick as f64) * 0.37;
    let unit = (t.sin() + 1.0) / 2.0;
    // One decimal: enough movement to see, few enough digits to read in a diff.
    ((lo + unit * (hi - lo)) * 10.0).round() / 10.0
}

impl Payload {
    /// The bytes for this tick, or `None` to publish a tombstone instead.
    fn bytes(&self, tick: u64) -> Option<Vec<u8>> {
        match self {
            Payload::Fixed(b) => Some(b.clone()),
            Payload::Wander => Some(
                format!(
                    r#"{{"value":{},"unit":"percent","inodes":{}}}"#,
                    wander(tick, 20.0, 90.0),
                    1000 + tick % 7,
                )
                .into_bytes(),
            ),
            Payload::Reading => Some(probe_sample(wander(tick, 5.0, 25.0))),
            Payload::Cycle => (tick % CYCLE != CYCLE - 1).then(|| {
                format!(
                    r#"{{"status":"ok","checks":{},"epoch":{}}}"#,
                    tick % CYCLE,
                    tick / CYCLE,
                )
                .into_bytes()
            }),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut config = zenoh::Config::default();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .ok();
    let json_list = |v: &[String]| {
        let items: Vec<String> = v.iter().map(|e| format!("{e:?}")).collect();
        format!("[{}]", items.join(","))
    };
    if !args.connect.is_empty() {
        config
            .insert_json5("connect/endpoints", &json_list(&args.connect))
            .ok();
    }
    if !args.listen.is_empty() {
        config
            .insert_json5("listen/endpoints", &json_list(&args.listen))
            .ok();
    }
    let session = zenoh::open(config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let with_base = |k: &str| zenkey::grammar::with_base(&args.base, k);
    let host = "h-3fa9c2d41b7e";

    // (key, payload source) pairs, one publisher each.
    let plan: Vec<(String, Payload)> = vec![
        // Conforming, and registered in fixture-tests/registry. This one
        // moves: it is the sparkline and payload-diff subject (#63/#64).
        (
            with_base(&format!("v1/{host}/telemetry/sysinfo/disk/var-log/used")),
            Payload::Wander,
        ),
        // State, retired and revived on a cycle — the tombstone demo.
        (
            with_base(&format!("v1/{host}/state/sysinfo/health")),
            Payload::Cycle,
        ),
        // Conforming, but NOT registered — must read "unregistered", not
        // "no slice", and certainly not "registered".
        (
            with_base(&format!("v1/{host}/telemetry/sysinfo/not/a/real/subject")),
            Payload::Fixed(br#"{"value":1}"#.to_vec()),
        ),
        // A service origin: `**` cannot reach this (D4), so it is the check
        // that the Deployment scope's explicit @catalog selector works.
        (
            with_base("v1/@catalog/state/entity/h-3fa9c2d41b7e"),
            Payload::Fixed(br#"{"entity":"host-a"}"#.to_vec()),
        ),
        // Foreign traffic — the key-agnostic path.
        (
            "demo/example/foo".to_string(),
            Payload::Fixed(b"just a plain string".to_vec()),
        ),
        (
            "demo/cbor".to_string(),
            Payload::Fixed(vec![0xA1, 0x61, 0x61, 0x01]),
        ),
        ("two/chunks".to_string(), Payload::Fixed(b"{}".to_vec())),
        (
            format!("v2/{host}/telemetry/sysinfo/cpu"),
            Payload::Fixed(b"a v2 key: not this convention".to_vec()),
        ),
        (
            format!("someotherbase/v1/{host}/state/sysinfo/health"),
            Payload::Fixed(br#"{"status":"another deployment"}"#.to_vec()),
        ),
        // Exercises the ring's byte budget and the no-preview path.
        (
            "demo/huge".to_string(),
            Payload::Fixed(vec![b'x'; 512 * 1024]),
        ),
        // Must NOT appear under a `**` subscription (RFC 03 §4 D2).
        (
            with_base(&format!("v1/{host}/@media/parallax/cam0/video/h264/hi")),
            Payload::Fixed(vec![0u8; 4096]),
        ),
        // A protobuf payload (#97). Opaque to every sniff — it decodes only
        // because `probe` serves its descriptor set on `describe` below.
        (
            with_base(&format!("v1/{host}/telemetry/probe/reading")),
            Payload::Reading,
        ),
    ];

    // The `probe` producer's RFC 08 §6/§7 halves. Served from the bus, so the
    // protobuf leaf decodes with `--registry` pointing anywhere (or nowhere):
    // the union takes served-wins, and this producer is only served.
    let mut described = Vec::new();
    for (procedure, payload) in [
        ("introspect", PROBE_SLICE.to_string()),
        ("describe", probe_schema_set()),
    ] {
        let key = with_base(&format!("v1/{host}/@rpc/probe/{procedure}"));
        println!("serving: {key}");
        let reply_key = key.clone();
        described.push(
            session
                .declare_queryable(&key)
                .callback(move |query| {
                    let q = query.clone();
                    let reply_key = reply_key.clone();
                    let payload = payload.clone();
                    tokio::spawn(async move {
                        let _ = q.reply(reply_key, payload).await;
                    });
                })
                .await
                .map_err(|e| anyhow::anyhow!("queryable {key}: {e}"))?,
        );
    }

    // Liveliness tokens (RFC 04 §5): the roster the node views feed on.
    // Killing this process retracts them — that is the suspect-on-retraction
    // demo for `zenctl node list --watch` and the zengui node dashboard.
    let tokens = [
        with_base(&format!("v1/{host}/state/sysinfo/alive")),
        with_base(&format!("v1/{host}/state/probe/alive")),
        with_base("v1/@catalog/state/alive"),
    ];
    let mut held = Vec::new();
    for key in tokens {
        println!("liveliness token: {key}");
        let t = session
            .liveliness()
            .declare_token(key.clone())
            .await
            .map_err(|e| anyhow::anyhow!("token {key}: {e}"))?;
        held.push(t);
    }

    println!("publishing {} keys at {} Hz each:", plan.len(), args.hz);
    let mut publishers = Vec::new();
    for (key, payload) in plan {
        println!("  {key}");
        let p = session
            .declare_publisher(key.clone())
            .await
            .map_err(|e| anyhow::anyhow!("declare {key}: {e}"))?;
        publishers.push((p, payload));
    }
    // The synthetic fan: bulk keys via ad-hoc puts (this is a test fixture,
    // not a conforming producer — 50k declared publishers would prove
    // nothing about the tree).
    let synth: Vec<String> = (0..args.keys)
        .map(|i| with_base(&format!("v1/{host}/telemetry/synth/g{}/k{i}", i / 100)))
        .collect();
    if !synth.is_empty() {
        println!("seeding {} synthetic keys…", synth.len());
        for key in &synth {
            let _ = session.put(key, b"1".to_vec()).await;
        }
        println!("…seeded.");
    }
    println!("\nCtrl-C to stop.");

    let period = Duration::from_secs_f64(1.0 / args.hz.max(0.1));
    let mut ticker = tokio::time::interval(period);
    let refresh_per_tick = (args.keys / 1000).max(1);
    let mut cursor = 0usize;
    let mut tick: u64 = 0;
    loop {
        ticker.tick().await;
        for (p, payload) in &publishers {
            match payload.bytes(tick) {
                Some(bytes) => {
                    let _ = p.put(bytes).await;
                }
                // A tombstone, not an empty put — the whole point of the
                // cycle (RFC 04 §1.2).
                None => {
                    let _ = p.delete().await;
                }
            }
        }
        tick = tick.wrapping_add(1);
        if !synth.is_empty() {
            for _ in 0..refresh_per_tick {
                let key = &synth[cursor % synth.len()];
                let _ = session.put(key, b"1".to_vec()).await;
                cursor += 1;
            }
        }
    }
}
