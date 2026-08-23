//! `zenctl expect` (#160) — the CI assertion verb over the engine's
//! [`zenkey_fleet::run_expect`]. All judgement lives engine-side; this maps
//! flags in and the three-state verdict onto the cutover/probe exit codes.

use anyhow::Result;

use crate::Bus;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    selector: &str,
    within: f64,
    count: Option<u64>,
    rate_min: Option<f64>,
    rate_max: Option<f64>,
    valid_payload: bool,
    qos: Option<&str>,
    absent: bool,
    args: &Bus,
) -> Result<()> {
    let qos = match qos {
        None => None,
        Some("declared") => Some(zenkey_fleet::QosCheck::Declared),
        Some(name) => Some(zenkey_fleet::QosCheck::Profile(
            zenkey::qos::QosProfile::from_name(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "--qos takes `declared` or a profile name — \
                     sampled|refreshed|transition|alert|frame (RFC 04 §3)"
                )
            })?,
        )),
    };
    if within <= 0.0 {
        anyhow::bail!("--within must be a positive number of seconds");
    }

    // A session that will not open is the impaired exit (`asked`'s 2), never
    // 1 — "not met" is a claim about a window that was actually watched.
    let session = super::asked("expect", args.session().await);
    // Slices enrich: `--valid-payload` and `--qos declared` degrade to
    // explained violations when nothing is loaded, and the report says why.
    // `None` stays `None` into the engine so each violation names the
    // missing registry (`no registry loaded…`) rather than claiming
    // `no schema served` about types nobody looked up (RFC 09 §5.1 O4; #246).
    let slices = super::asked("expect", args.slices_optional().await);
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let spec = zenkey_fleet::ExpectSpec {
        selector: selector.to_string(),
        within: std::time::Duration::from_secs_f64(within),
        count,
        rate_min,
        rate_max,
        valid_payload,
        qos,
        absent,
    };

    eprintln!(
        "expect: watching {selector} for {within}s — subscriber declared before \
         the window opened (RFC 09 §5.1 O4)"
    );
    let report =
        match zenkey_fleet::run_expect(&session, args.base(), slices.as_ref(), &store, &spec).await
        {
            Ok(r) => r,
            Err(e) => {
                // The observation never stood up — that is the impaired exit,
                // never "not met".
                eprintln!("expect: observation could not be established: {e}");
                std::process::exit(2);
            }
        };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    match report.verdict {
        zenkey_fleet::report::ExpectVerdict::Met => Ok(()),
        zenkey_fleet::report::ExpectVerdict::NotMet => std::process::exit(1),
        zenkey_fleet::report::ExpectVerdict::Impaired => std::process::exit(2),
    }
}
