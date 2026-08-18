//! `zenctl gen` (#162) — registry-driven pattern generation, guarded.
//!
//! Orchestration only: the plan, the schedule, and the synthesis live in
//! `zenkey_fleet::gen`/`synth`. This command's job is the etiquette — print
//! the full plan before anything is published (the replay dry-run
//! precedent), refuse a wide run without `--i-know`, and stamp the RFC 09
//! §5.3 synthetic marker via the engine.

use anyhow::Result;
use zenkey_fleet::generate::{GenPattern, GenSpec};

use crate::BusArgs;
use crate::output::Format;

/// A run wider than this needs `--i-know` — a generator pointed at a full
/// registry is a fleet-wide impersonation, and that is a decision, not a
/// default.
const WIDE_ENTRIES: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Pattern {
    Steady,
    Jitter,
    Burst,
    Ramp,
}

impl From<Pattern> for GenPattern {
    fn from(p: Pattern) -> GenPattern {
        match p {
            Pattern::Steady => GenPattern::Steady,
            Pattern::Jitter => GenPattern::Jitter,
            Pattern::Burst => GenPattern::Burst,
            Pattern::Ramp => GenPattern::Ramp,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    producer: Option<&str>,
    subject: Option<&str>,
    vars: &[String],
    origin: Option<&str>,
    rate: Option<f64>,
    pattern: Pattern,
    duration: f64,
    seed: u64,
    schema_set: Option<&std::path::Path>,
    serve_describe: bool,
    dry_run: bool,
    i_know: bool,
    args: &BusArgs,
) -> Result<()> {
    let vars: Vec<(String, String)> = vars
        .iter()
        .map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| anyhow::anyhow!("--var takes k=v, got {kv:?}"))
        })
        .collect::<Result<_>>()?;
    if duration <= 0.0 {
        anyhow::bail!("--duration must be a positive number of seconds");
    }

    let session = args.session().await?;
    let slices = args.slice_set().await?;
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let set = match schema_set {
        Some(path) => Some(
            zenkey::schema::SchemaSet::parse(&std::fs::read_to_string(path)?).map_err(|e| {
                anyhow::anyhow!("{}: not a SchemaSet document: {e}", path.display())
            })?,
        ),
        None => None,
    };

    // The generating origin: stated or derived from this session's zid —
    // either way printed and stamped into every sample's marker.
    let origin = match origin {
        Some(o) => {
            zenkey::HostId::parse(o)
                .map_err(|e| anyhow::anyhow!("--origin {o:?}: {e} (h-<12 hex>)"))?;
            o.to_string()
        }
        None => {
            let zid = session.zid().to_string();
            let hex: String = zid
                .chars()
                .filter(char::is_ascii_hexdigit)
                .flat_map(char::to_lowercase)
                .take(12)
                .collect();
            format!("h-{hex:0>12}")
        }
    };

    let spec = GenSpec {
        origin: origin.clone(),
        producer: producer.map(str::to_string),
        subject: subject.map(str::to_string),
        vars,
        rate_hz: rate,
        pattern: pattern.into(),
        duration: std::time::Duration::from_secs_f64(duration),
        seed,
        tool: "zenctl gen".into(),
    };

    let plan = zenkey_fleet::generate::build_plan(
        Some(&session),
        &store,
        &slices,
        args.base(),
        set.as_ref(),
        &spec,
    )
    .await?;
    if plan.is_empty() {
        anyhow::bail!(
            "the registry resolves to no generatable subject (host producers only; \
             service origins are out of scope — impersonating @catalog would fight \
             the real service's claim, RFC 06 §5.3)"
        );
    }

    // The plan, before anything is published (the replay dry-run precedent).
    match args.format.resolved() {
        Format::Json | Format::Ndjson => {
            for e in &plan {
                println!("{}", serde_json::to_string(e)?);
            }
        }
        _ => {
            eprintln!(
                "plan: {} subject(s) as {origin}, {duration}s, marker {{\"synthetic\":true}} \
                 on every sample (RFC 09 §5.3):",
                plan.len()
            );
            for e in &plan {
                eprintln!(
                    "  {} [{}] {:.2} Hz, qos {} ({}), body {}{}{}",
                    e.key,
                    e.type_name,
                    e.rate_hz,
                    e.qos,
                    e.qos_source,
                    e.body_source,
                    e.events_cap
                        .map(|c| format!(", capped at {c} event(s)"))
                        .unwrap_or_default(),
                    e.note
                        .as_deref()
                        .map(|n| format!(" — {n}"))
                        .unwrap_or_default(),
                );
            }
        }
    }
    if dry_run {
        eprintln!("--dry-run: nothing published");
        return Ok(());
    }
    if plan.len() > WIDE_ENTRIES && !i_know {
        anyhow::bail!(
            "{} subjects is a fleet-wide impersonation — narrow with --producer/--subject, \
             or pass --i-know",
            plan.len()
        );
    }

    // The RFC 08 halves for the impersonated producers, on request.
    let mock = if serve_describe {
        let m = zenkey_fleet::generate::serve_describe(
            &session,
            args.base(),
            &origin,
            &slices,
            set.as_ref(),
            producer,
        )
        .await?;
        eprintln!(
            "serving introspect{} on {} impersonated @rpc key(s)",
            if set.is_some() { "+describe" } else { "" },
            m.keys
        );
        Some(m)
    } else {
        None
    };

    let report = zenkey_fleet::generate::run_gen(&session, &store, &plan, &spec).await?;
    drop(mock);

    match args.format.resolved() {
        Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Format::Ndjson => println!("{}", serde_json::to_string(&report)?),
        _ => {
            eprintln!(
                "sent {} sample(s) over {:.1}s across {} subject(s); {} refused by schema",
                report.sent, report.duration_s, report.entries, report.refused
            );
            for e in &report.first_errors {
                eprintln!("  ✗ {e}");
            }
        }
    }
    if report.refused > 0 {
        std::process::exit(1);
    }
    Ok(())
}
