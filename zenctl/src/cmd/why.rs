//! `zenctl why` (issue #214): why is this key silent — the non-verdict,
//! itemised.
//!
//! Thin by design (#206/#209): the rung ladder, the stable id vocabulary and
//! the verdict rules are judgement over bus data, so they live in
//! [`zenkey_fleet::why`] where the other explorer can reach them. What is
//! left here is what only a CLI has: the resolved bus, the registry ladder
//! (`--registry` dirs, or the live sweep, degrading through
//! `slices_optional` — a registry that could not be loaded turns the
//! declaration rung `NotAsked`, never `No`), the rendering, and the exit
//! code:
//!
//! * **0** — an explanation was established;
//! * **1** — none was, and everything checked looks healthy ("declared,
//!   alive, never published" lands here: lazy publisher declaration is not a
//!   bug, RFC 08 §6.1);
//! * **2** — the observation was impaired: an input the ladder wanted could
//!   not be obtained, so "healthy" cannot be claimed (RFC 09 §5.1 O4).

use anyhow::Result;

use crate::Bus;
use crate::cli::WhyArgs;

pub async fn run(args: WhyArgs) -> Result<()> {
    // A verdict verb: every pre-run failure is `asked`'s exit 2, never 1 —
    // exit 1 here means "no cause found and everything looks healthy", which
    // a bus that would not open has no standing to claim.
    let bus = super::asked("why", Bus::resolve(&args.bus));
    // The registry through the one degradation door (#210): unavailable is
    // `None` — announced once, and rendered as "not asked" by the rung.
    let slices = super::asked("why", bus.slices_optional().await);
    let session = super::asked("why", bus.session().await);

    // Stated before the window opens, not after (O5): a user watching a
    // silence deserves to know what is being watched, and that the window is
    // the only data-plane cost of this run.
    if let Some(secs) = args.listen {
        eprintln!(
            "why: listening {secs}s on {} — the one rung that costs the data \
             plane; everything else was control-plane sweeps (RFC 09 §5.1).",
            args.key
        );
    }

    let spec = zenkey_fleet::WhySpec {
        timeout: bus.timeout(),
        listen: args.listen.map(std::time::Duration::from_secs_f64),
    };
    let report = super::asked(
        "why",
        zenkey_fleet::run_why(&session, bus.base(), &args.key, slices.as_ref(), &spec).await,
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;

    // A library returns a verdict, a command exits with it (the `cutover`
    // discipline): 0 = explained, 1 = no cause and healthy, 2 = impaired.
    match report.verdict {
        zenkey_fleet::WhyVerdict::Explained => Ok(()),
        zenkey_fleet::WhyVerdict::Healthy => std::process::exit(1),
        zenkey_fleet::WhyVerdict::Impaired => std::process::exit(2),
    }
}
