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
//! - a **`@blob` artifact** served by the reference client (issue #68), and
//!   beside it a second origin claiming the same id at a *different* content
//!   root. Two holders that disagree is the case RFC 07 §2.1 exists for — the
//!   id is a name and the root is the anchor — and it is the one thing a
//!   single honest server can never demonstrate.
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

# What this producer serves on the @blob plane (RFC 08 §2, v1.8). `push` and
# `fanout` are omitted because spray serves neither: a slice states capability,
# and claiming one it does not have would be the lie the plane is modelled to
# prevent.
[[blob]]
tier = "artifact"
endpoints = ["manifest", "slice", "have"]
reference = "BlobReference"
encoding = "application/octet-stream"
since = "1.8"
description = "a demo artifact, so the blob pane has something to probe"
"#;

/// The demo artifact's id. **Lowercase**, and the example is where that is
/// documented: RFC 07 §2.2 calls the id a ULID, canonical ULIDs are uppercase
/// Crockford base32, and RFC 03 §2 chunks have no uppercase spelling. An
/// uppercase id here would produce a key nothing in this convention can parse.
const BLOB_ID: &str = "01jqz3demo0001";

/// The `parallax` producer's introspect slice: the media declarations
/// (RFC 08 §2/§6, v1.16) the zengui media pane discovers off the bus.
/// The preview rung is a real, decodable stream (PNG frames below); the
/// video rung is declared and *published as noise* — a viewer that cannot
/// decode h264 must say so instead of pretending (issue #69).
const PARALLAX_SLICE: &str = r#"
[registry]
version = "1.3"
app = "spray"
convention = 1

[producer]
name = "parallax"
description = "demo media producer (issue #69)"

[[media]]
path = "{stream}/preview/png"
encoding = "image/png"
attachment = "FrameMeta"
cardinality = 16
since = "1.0"
description = "the decodable preview rung"

[[media]]
path = "{stream}/video/{codec}/{tier}"
encoding = "video/*"
attachment = "FrameMeta"
cardinality = 128
since = "1.3"
description = "video rungs — declared, shown as metadata only"
"#;

/// A minimal PNG encoder for the demo stream: truecolor 8-bit, no filter,
/// zlib *stored* blocks — ~40 lines instead of an image-crate dependency
/// the fixture does not otherwise need. Fine at 64×64; do not reuse for
/// anything that cares about size.
fn png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xedb8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(body);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }
    // Raw scanlines, each with filter byte 0.
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * width as usize * 3..(y + 1) * width as usize * 3]);
    }
    // zlib: header + stored deflate blocks + adler32.
    let mut idat = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        idat.push(u8::from(last));
        idat.extend_from_slice(&(block.len() as u16).to_le_bytes());
        idat.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        idat.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    idat.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolor
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// One preview frame: a moving diagonal band over a gradient, so motion is
/// obvious at a glance and every frame differs.
fn preview_frame(tick: u64) -> Vec<u8> {
    const W: u32 = 64;
    const H: u32 = 64;
    let mut rgb = Vec::with_capacity((W * H * 3) as usize);
    let band = (tick % u64::from(W)) as i64;
    for y in 0..H as i64 {
        for x in 0..W as i64 {
            let on_band = ((x + y) % i64::from(W) - band).abs() < 4;
            if on_band {
                rgb.extend_from_slice(&[0xff, 0xd0, 0x20]);
            } else {
                rgb.extend_from_slice(&[(x * 3) as u8, 0x30, (y * 3) as u8]);
            }
        }
    }
    png(W, H, &rgb)
}

/// The origin the *disagreeing* holder answers under — a second claimant for
/// `BLOB_ID`, serving different bytes.
const ROGUE_ORIGIN: &str = "h-bbbbbbbbbbbb";

/// Deterministic artifact bytes, so two runs of the demo agree about the root.
fn artifact_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) as u8
        })
        .collect()
}

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
    for (producer, procedure, payload) in [
        ("probe", "introspect", PROBE_SLICE.to_string()),
        ("probe", "describe", probe_schema_set()),
        ("parallax", "introspect", PARALLAX_SLICE.to_string()),
    ] {
        let key = with_base(&format!("v1/{host}/@rpc/{producer}/{procedure}"));
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

    // ── the @blob plane (#68) ────────────────────────────────────────────
    //
    // A real artifact, served by the RFC 07 reference client under this host's
    // tier-1 prefix; and beside it a second origin claiming the same id at a
    // different root. Both are needed: one server proves a probe→fetch round
    // trip works, and only two disagreeing ones prove the explorer *reports*
    // the disagreement instead of trusting whoever answered first (§2.1).
    let blob_prefix = with_base(
        zenkey::grammar::blob_tier_prefix(
            &zenkey::grammar::Origin::Host(zenkey::HostId::parse(host)?),
            zenkey::grammar::BlobTier::Artifact,
        )
        .as_str(),
    );
    let blob_data = artifact_bytes(1 << 20, 42);
    let blob_server = zenkey_fleet::zblob::BlobServer::new(
        &session,
        zenkey_fleet::zblob::ServePrefix::new(blob_prefix.clone())
            .map_err(|e| anyhow::anyhow!("blob prefix: {e}"))?,
    );
    let manifest = blob_server
        .register_source(
            zenkey_fleet::zblob::BlobSpec::new(BLOB_ID).filename("demo-bundle.bin"),
            std::sync::Arc::new(zenkey_fleet::zblob::MemoryBlobSource::new(blob_data)),
        )
        .await
        .map_err(|e| anyhow::anyhow!("register blob: {e}"))?;
    // The serve loop owns its queryable and lives in its own task, so the
    // handle is only a stop switch — and this fixture stops with the process.
    let _blob_handle = blob_server
        .spawn()
        .await
        .map_err(|e| anyhow::anyhow!("spawn blob server: {e}"))?;
    println!(
        "serving: {blob_prefix}/{BLOB_ID}/**  root {}",
        manifest.root
    );

    // The rogue: same id, a manifest with a different root, and an
    // availability bitfield claiming the lot. It serves no slices — it does
    // not need to, because a fetch from it with the honest root pinned must
    // fail before a byte reaches disk, naming this origin.
    let rogue_prefix = with_base(
        zenkey::grammar::blob_tier_prefix(
            &zenkey::grammar::Origin::Host(zenkey::HostId::parse(ROGUE_ORIGIN)?),
            zenkey::grammar::BlobTier::Artifact,
        )
        .as_str(),
    );
    let rogue_manifest = zenkey_fleet::zblob::Manifest {
        version: zenkey_fleet::zblob::wire::WIRE_VERSION,
        id: zenkey_fleet::zblob::BlobId::new(BLOB_ID)
            .map_err(|e| anyhow::anyhow!("rogue id: {e}"))?,
        filename: Some("demo-bundle.bin".into()),
        total_len: manifest.total_len,
        chunk_size: manifest.chunk_size,
        root: zenkey_fleet::zblob::Hash::of(b"a different artifact entirely"),
        created_ms: 0,
        ext: zenkey_fleet::zblob::wire::Ext::default(),
    };
    println!(
        "serving: {rogue_prefix}/{BLOB_ID}/**  root {}  (the disagreeing holder)",
        rogue_manifest.root
    );
    let chunk_count = manifest.total_len.div_ceil(manifest.chunk_size as u64) as u32;
    let rogue_replies: std::collections::HashMap<
        String,
        (Vec<u8>, &'static zenkey_fleet::zblob::wire::WireTag),
    > = std::collections::HashMap::from([
        (
            zenkey_fleet::zblob::keys::manifest_key(&rogue_prefix, BLOB_ID),
            (
                zenkey_fleet::zblob::wire::encode(&rogue_manifest)
                    .map_err(|e| anyhow::anyhow!("encode rogue manifest: {e}"))?,
                &zenkey_fleet::zblob::wire::ENC_MANIFEST,
            ),
        ),
        (
            zenkey_fleet::zblob::keys::availability_key(&rogue_prefix, BLOB_ID),
            (
                zenkey_fleet::zblob::wire::encode(&zenkey_fleet::zblob::wire::Availability::full(
                    chunk_count,
                ))
                .map_err(|e| anyhow::anyhow!("encode rogue availability: {e}"))?,
                &zenkey_fleet::zblob::wire::ENC_AVAIL,
            ),
        ),
    ]);
    let _rogue = session
        .declare_queryable(format!("{rogue_prefix}/**"))
        .callback(move |query| {
            // Replies go on the responder's own concrete key, as a real server
            // does — which is what lets a `*`-origin probe attribute them.
            for (key, (payload, encoding)) in &rogue_replies {
                if !query.key_expr().intersects(
                    &zenoh::key_expr::KeyExpr::try_from(key.as_str()).expect("valid key"),
                ) {
                    continue;
                }
                let q = query.clone();
                let key = key.clone();
                let payload = payload.clone();
                let encoding = *encoding;
                tokio::spawn(async move {
                    let _ = q.reply(key, payload).encoding(encoding).await;
                });
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("rogue queryable: {e}"))?;

    // Liveliness tokens (RFC 04 §5): the roster the node views feed on.
    // Killing this process retracts them — that is the suspect-on-retraction
    // demo for `zenctl node list --watch` and the zengui node dashboard.
    let tokens = [
        with_base(&format!("v1/{host}/state/sysinfo/alive")),
        with_base(&format!("v1/{host}/state/probe/alive")),
        with_base(&format!("v1/{host}/state/parallax/alive")),
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

    // The media preview stream (issue #69): PNG frames on the exact key the
    // registry declares the shape of, metadata on the attachment (RFC 07
    // §1), published through a declared publisher at the demo rate.
    let preview_key = with_base(&format!("v1/{host}/@media/parallax/cam0/preview/png"));
    println!("publishing media preview: {preview_key}");
    let preview = session
        .declare_publisher(preview_key)
        .await
        .map_err(|e| anyhow::anyhow!("preview publisher: {e}"))?;

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
        {
            let frame = preview_frame(tick);
            let meta = format!(r#"{{"seq":{tick},"keyframe":true,"width":64,"height":64}}"#);
            let _ = preview
                .put(frame)
                .encoding("image/png")
                .attachment(meta.into_bytes())
                .await;
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
