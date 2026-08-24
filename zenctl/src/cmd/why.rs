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
//! code.
//!
//! ## The exit codes flipped (#264)
//!
//! They used to read 0 = explained, 1 = healthy, 2 = impaired — which paged
//! you for a **healthy** fleet and stayed quiet when a cause was found. Under
//! the one contract in [`crate::exit`], 1 is the *finding*, and a cause is
//! exactly that:
//!
//! * **0** — no cause was established and everything checked looks healthy
//!   ("declared, alive, never published" lands here: lazy publisher
//!   declaration is not a bug, RFC 08 §6.1);
//! * **1** — an explanation was established — the finding;
//! * **2** — the observation was impaired: an input the ladder wanted could
//!   not be obtained, so neither answer can be claimed (RFC 09 §5.1 O4).
//!
//! Nothing here spells that: [`zenkey_fleet::WhyVerdict::to_judgement`]
//! carries the inversion (RFC 13's convention — the judged claim is the
//! finding) and [`crate::exit::verdict`] projects it. The flip is a property
//! of the mapping, not of this file.

use anyhow::Result;

use crate::Bus;
use crate::cli::WhyArgs;

pub async fn run(args: WhyArgs) -> Result<()> {
    // A verdict verb: every pre-run failure is `asked`'s exit 2, never 1 —
    // exit 1 here means "a cause was found", which a bus that would not open
    // has no standing to claim. A `$*` selector is the same kind of failure:
    // a question that cannot be asked (RFC 03 §2).
    let bus = crate::exit::asked("why", Bus::resolve(&args.bus));
    let selector = crate::exit::asked("why", super::selector_of(&args.selector, &bus));
    // The registry through the one degradation door (#210): unavailable is
    // `None` — announced once, and rendered as "not asked" by the rung.
    let slices = crate::exit::asked("why", bus.slices_optional().await);
    let session = crate::exit::asked("why", bus.session().await);

    // Stated before the window opens, not after (O5): a user watching a
    // silence deserves to know what is being watched, and that the window is
    // the only data-plane cost of this run.
    let listen = match args.for_secs {
        Some(secs) => {
            let d = crate::exit::asked("why", super::positive_secs("--for", secs));
            eprintln!(
                "why: listening {secs}s on {selector} — the one rung that costs the \
                 data plane; everything else was control-plane sweeps (RFC 09 §5.1)."
            );
            Some(d)
        }
        None => None,
    };

    let spec = zenkey_fleet::WhySpec {
        timeout: bus.timeout(),
        listen,
    };
    let report = crate::exit::asked(
        "why",
        zenkey_fleet::run_why(&bus.fleet(&session), &selector, slices.as_ref(), &spec).await,
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;

    // A library returns a verdict, a command exits with it — through the
    // engine's own projection, never a hand-rolled `match` (`crate::exit`).
    crate::exit::verdict(&report.verdict.to_judgement())
}
