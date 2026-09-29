//! `zenctl check conform` (#222) — a producer's registry as an executable
//! conformance suite, over the engine's [`zenkey_fleet::run_conform`]. All
//! judgement lives engine-side; this loads the suite, maps flags in, writes
//! the JUnit artifact when asked, and hands the verdict to
//! [`crate::exit::verdict`], which is the only place 0/1/2 is spelled.

use anyhow::Result;

use crate::Bus;

/// This verb's name, spelled once (#355) — the dispatcher uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check conform");

pub async fn run(cli: crate::cli::CheckConformArgs) -> Result<()> {
    // A verdict verb: every failure before the question is asked is the
    // reserved exit 2, never the 1 that would claim a violation.
    let bus = ASKING.ask(Bus::resolve(&cli.bus));
    let args = &bus;
    let crate::cli::CheckConformArgs {
        producer,
        origin,
        for_secs,
        deep,
        junit,
        bus: _,
    } = cli;
    let listen = for_secs.map(|secs| ASKING.ask(super::positive_secs("--for", secs)));

    // The suite is the registry, so the slices *determine* the run: a load
    // that fails is a question that cannot be asked. With `--registry` the
    // suite is the build's own files — the contract it ships — and not the
    // union, where the served slice wins and `slice-sync` would compare the
    // fleet with itself. Without, it is what the fleet serves.
    let dirs = args.registry_dirs();
    let (slices, source) = if dirs.is_empty() {
        (
            ASKING.ask(args.slice_set().await),
            zenkey_fleet::SliceSource::Bus,
        )
    } else {
        (
            ASKING.ask(zenkey_fleet::SliceSet::from_dirs(&dirs)),
            zenkey_fleet::SliceSource::Dirs,
        )
    };
    let session = ASKING.ask(args.session().await);
    let spec = zenkey_fleet::ConformSpec {
        producer,
        origin,
        listen,
        deep,
        timeout: args.timeout(),
        source,
    };
    if let Some(window) = listen {
        eprintln!(
            "{}: calling {}'s procedures, then listening {:.0}s — a window proves \
             presence, never absence (RFC 13 §3)",
            ASKING.verb(),
            spec.producer,
            window.as_secs_f64()
        );
    }
    // An undeclared producer or an origin that is not one is `Unaskable`
    // engine-side, and `ask` lands it on 2 like every other refusal.
    let report = ASKING.ask(zenkey_fleet::run_conform(&args.fleet(&session), &slices, &spec).await);
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    if let Some(path) = junit {
        // The artifact CI reads is part of the answer: a verdict whose file
        // was not written has not been delivered, which is the 2.
        ASKING.ask(
            std::fs::write(&path, report.junit())
                .map_err(|e| anyhow::anyhow!("--junit {}: {e}", path.display())),
        );
    }
    crate::exit::verdict(&report.verdict.to_judgement())
}
