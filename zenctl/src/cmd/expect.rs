//! `zenctl check expect` (#160) — the CI assertion verb over the engine's
//! [`zenkey_fleet::run_expect`]. All judgement lives engine-side; this maps
//! flags in and hands the three-state verdict to [`crate::exit::verdict`],
//! which is the only place 0/1/2 is spelled.

use anyhow::Result;

use crate::Bus;
use crate::cli::SelectorArgs;
use crate::exit::unaskable;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    sel: &SelectorArgs,
    for_secs: f64,
    at_least: Option<u64>,
    rate_min: Option<f64>,
    rate_max: Option<f64>,
    valid_payload: bool,
    qos: Option<&str>,
    absent: bool,
    args: &Bus,
) -> Result<()> {
    // A verdict verb: a selector this tool refuses, or a window of zero
    // seconds, is a question that cannot be asked — `asked`'s reserved 2,
    // never the 1 that would claim a verdict.
    let selector = crate::exit::asked("check expect", super::selector_of(sel, args));
    let within = crate::exit::asked("check expect", super::positive_secs("--for", for_secs));
    let qos = crate::exit::asked("check expect", qos_check(qos));

    // A session that will not open is the impaired exit (`asked`'s 2), never
    // 1 — "not met" is a claim about a window that was actually watched.
    let session = crate::exit::asked("check expect", args.session().await);
    // Slices enrich: `--valid-payload` and `--qos declared` degrade to
    // explained violations when nothing is loaded, and the report says why.
    // `None` stays `None` into the engine so each violation names the
    // missing registry (`no registry loaded…`) rather than claiming
    // `no schema served` about types nobody looked up (RFC 09 §5.1 O4; #246).
    let slices = crate::exit::asked("check expect", args.slices_optional().await);
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let spec = zenkey_fleet::ExpectSpec {
        selector: selector.clone(),
        within,
        count: at_least,
        rate_min,
        rate_max,
        valid_payload,
        qos,
        absent,
    };

    eprintln!(
        "check expect: watching {selector} for {for_secs}s — subscriber declared \
         before the window opened (RFC 09 §5.1 O4)"
    );
    let report =
        match zenkey_fleet::run_expect(&args.fleet(&session), slices.as_ref(), &store, &spec).await
        {
            Ok(r) => r,
            Err(e) => {
                // The observation never stood up — that is the impaired exit,
                // never "not met".
                eprintln!("check expect: observation could not be established: {e}");
                std::process::exit(crate::exit::NO_VERDICT);
            }
        };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    crate::exit::verdict(&report.verdict.to_judgement())
}

/// `--qos declared`, or one of RFC 04 §3's five profile names. Anything else
/// is a closed vocabulary the user missed, which is an exit 2 (`crate::exit`)
/// — it used to be a 1, indistinguishable from "the QoS did not match".
fn qos_check(qos: Option<&str>) -> Result<Option<zenkey_fleet::QosCheck>> {
    match qos {
        None => Ok(None),
        Some("declared") => Ok(Some(zenkey_fleet::QosCheck::Declared)),
        Some(name) => match zenkey::qos::QosProfile::from_name(name) {
            Some(p) => Ok(Some(zenkey_fleet::QosCheck::Profile(p))),
            None => Err(unaskable!(
                "--qos takes `declared` or a profile name — \
                 sampled|refreshed|transition|alert|frame (RFC 04 §3), got {name:?}"
            )),
        },
    }
}
