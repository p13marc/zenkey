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
//! cargo run -p zengui --example spray -- -c tcp/127.0.0.1:7447
//! ```

use std::time::Duration;

use clap::Parser;

#[derive(Parser)]
#[command(about = "publish mixed conforming and foreign traffic for zengui")]
struct Args {
    /// Endpoint to connect to, repeatable.
    #[arg(long, short = 'c')]
    connect: Vec<String>,
    /// Deployment base for the conforming keys. Empty = the bus root.
    #[arg(long, default_value = "")]
    base: String,
    /// Publications per second, per key.
    #[arg(long, default_value_t = 5.0)]
    hz: f64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut config = zenoh::Config::default();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .ok();
    if !args.connect.is_empty() {
        let eps: Vec<String> = args.connect.iter().map(|e| format!("{e:?}")).collect();
        config
            .insert_json5("connect/endpoints", &format!("[{}]", eps.join(",")))
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
    println!("\nCtrl-C to stop.");

    let period = Duration::from_secs_f64(1.0 / args.hz.max(0.1));
    let mut ticker = tokio::time::interval(period);
    loop {
        ticker.tick().await;
        for (p, payload) in &publishers {
            let _ = p.put(payload.clone()).await;
        }
    }
}
