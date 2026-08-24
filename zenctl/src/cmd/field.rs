//! `zenctl field` (#223) — field intelligence over a window, through the
//! engine's [`zenkey_fleet::run_field`]. All judgement lives engine-side
//! (RFC 09 §5.1's rules are the engine's to keep); this maps flags in and
//! renders the report. Findings are output, not verdicts — exit 0, the
//! `doctor` discipline (`watchdog --rule 'doctor field-stuck'` is the
//! alerting surface).

use anyhow::Result;

use crate::Bus;

pub async fn run(selector: &str, window: f64, max_paths: usize, args: &Bus) -> Result<()> {
    // The raw seam: `$*` never reaches the session (RFC 03 §2).
    let selector = super::raw_selector(selector)?;
    if window <= 0.0 {
        anyhow::bail!("--window must be a positive number of seconds");
    }
    if max_paths == 0 {
        anyhow::bail!("--max-paths must be at least 1 — a zero bound observes nothing");
    }

    let session = args.session().await?;
    // Slices enrich: declared ttl_s (field-stuck's yardstick) and type names
    // (field-new's schema lookup) come from the registry. `None` stays `None`
    // into the engine so the report says stuck/new were unjudgeable rather
    // than reading clean (RFC 09 §5.1 O4; #246).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let spec = zenkey_fleet::FieldSpec {
        selector: selector.to_string(),
        window: std::time::Duration::from_secs_f64(window),
        max_paths,
    };

    eprintln!(
        "field: watching {selector} for {window}s — subscriber declared before \
         the window opened (RFC 09 §5.1 O4)"
    );
    let report =
        zenkey_fleet::run_field(&args.fleet(&session), slices.as_ref(), &store, &spec).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}
