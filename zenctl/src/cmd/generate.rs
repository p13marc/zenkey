//! `zenctl gen` (#162) — registry-driven pattern generation, guarded.
//! `--fault` (#163) is a mode of the same generator: near-valid traffic for
//! consumer-robustness testing.
//!
//! Orchestration only: the plan, the schedule, and the synthesis live in
//! `zenkey_fleet::generate`/`synth`. This command's job is the etiquette —
//! print the full plan before anything is published (the replay dry-run
//! precedent), refuse a wide run without `--i-know`, and stamp the RFC 09
//! §5.3 synthetic marker via the engine.
//!
//! Fault injection is **double-guarded** (#163, RFC 09 §5.3): it requires
//! `--i-know` *and* an explicit endpoint or `--base` — never the ambient
//! named-context default — because a fault injector publishes deliberately
//! non-conforming traffic and must not land on whatever bus the shell was
//! pointed at. Each fault perturbs one dimension of a synthesized sample
//! post-synthesis, so the plan prints exactly what deviates per key, and
//! every faulted sample's marker carries `fault=<kind>`.

use crate::cli::Pattern;
use anyhow::Result;
use zenkey_fleet::generate::{Fault, GenPattern, GenSpec};

use crate::Bus;

/// A run wider than this needs `--i-know` — a generator pointed at a full
/// registry is a fleet-wide impersonation, and that is a decision, not a
/// default.
const WIDE_ENTRIES: usize = 10;

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
    fault: &[String],
    explicit_target: bool,
    schema_set: Option<&std::path::Path>,
    serve_describe: bool,
    dry_run: bool,
    i_know: bool,
    args: &Bus,
) -> Result<()> {
    // Fault injection is double-guarded (#163): it produces deliberately
    // near-valid traffic, so it may only ever run knowingly, and only against
    // a bus the operator named — never the ambient context default.
    let faults: Vec<Fault> = fault.iter().map(|s| Fault::parse(s)).collect::<Result<_>>()?;
    if !faults.is_empty() {
        if !i_know {
            anyhow::bail!(
                "--fault injects deliberately non-conforming traffic — that is \
                 consumer-robustness testing on a bus you own, not a default. \
                 Pass --i-know to mean it."
            );
        }
        if !explicit_target {
            anyhow::bail!(
                "--fault refuses the ambient context: name the bus explicitly with \
                 --base or an endpoint (--connect/--listen/--zenoh-config), so faults \
                 cannot land on whatever bus your shell happened to be pointed at."
            );
        }
    }

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
        faults,
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
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::GenPlan {
            origin: &origin,
            duration_s: duration,
            entries: &plan,
        },
        args.format(),
        args.color(),
    )?;
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

    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    if report.refused > 0 {
        std::process::exit(1);
    }
    Ok(())
}
