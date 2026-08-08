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
//! - a `@media`-shaped key, which a `**` subscriber must *not* see (RFC 03 §4
//!   D2) — if it shows up in the tree, the scope design is broken.
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

    // (key, payload) pairs, one publisher each.
    let plan: Vec<(String, Vec<u8>)> = vec![
        // Conforming, and registered in fixture-tests/registry.
        (
            with_base(&format!("v1/{host}/telemetry/sysinfo/disk/var-log/used")),
            br#"{"value":42.0,"unit":"percent"}"#.to_vec(),
        ),
        (
            with_base(&format!("v1/{host}/state/sysinfo/health")),
            br#"{"status":"ok"}"#.to_vec(),
        ),
        // Conforming, but NOT registered — must read "unregistered", not
        // "no slice", and certainly not "registered".
        (
            with_base(&format!("v1/{host}/telemetry/sysinfo/not/a/real/subject")),
            br#"{"value":1}"#.to_vec(),
        ),
        // A service origin: `**` cannot reach this (D4), so it is the check
        // that the Deployment scope's explicit @catalog selector works.
        (
            with_base("v1/@catalog/state/entity/h-3fa9c2d41b7e"),
            br#"{"entity":"host-a"}"#.to_vec(),
        ),
        // Foreign traffic — the key-agnostic path.
        (
            "demo/example/foo".to_string(),
            b"just a plain string".to_vec(),
        ),
        ("demo/cbor".to_string(), vec![0xA1, 0x61, 0x61, 0x01]),
        ("two/chunks".to_string(), b"{}".to_vec()),
        (
            format!("v2/{host}/telemetry/sysinfo/cpu"),
            b"a v2 key: not this convention".to_vec(),
        ),
        (
            format!("someotherbase/v1/{host}/state/sysinfo/health"),
            br#"{"status":"another deployment"}"#.to_vec(),
        ),
        // Exercises the ring's byte budget and the no-preview path.
        ("demo/huge".to_string(), vec![b'x'; 512 * 1024]),
        // Must NOT appear under a `**` subscription (RFC 03 §4 D2).
        (
            with_base(&format!("v1/{host}/@media/parallax/cam0/video/h264/hi")),
            vec![0u8; 4096],
        ),
    ];

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
    loop {
        ticker.tick().await;
        for (p, payload) in &publishers {
            let _ = p.put(payload.clone()).await;
        }
        if !synth.is_empty() {
            for _ in 0..refresh_per_tick {
                let key = &synth[cursor % synth.len()];
                let _ = session.put(key, b"1".to_vec()).await;
                cursor += 1;
            }
        }
    }
}
