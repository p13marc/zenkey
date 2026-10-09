//! `why` (#702) — a zk2 key's or service's silence, explained rung by rung.
//!
//! The ladder is the engine's ([`zenkey_fleet::run_why`]): the reads in its
//! bus layer, the verdict from values in its judge, so a GUI pane asks the
//! same questions. This command is orchestration and rendering: resolve the
//! deployment, open a session **in** its namespace (decided 2026-10-08),
//! run, print, and exit through the report's own verdict — 1 when a cause
//! is established (a cause is the finding), 0 when every rung is healthy
//! and the key answers, 2 when a rung is unobservable.
//!
//! **A key outside the namespace asks the bus nothing.** The judge is
//! consulted before any session opens: a key of another deployment, or one
//! that is not zk2, is a cause established from the key alone, and needs no
//! bus to say so. A verdict verb: every failure before the run is the
//! reserved 2 (`crate::exit::asked`), never a 1 that would claim a cause.

use anyhow::Result;
use zenkey_fleet::report::RungId;
use zenkey_fleet::{BundleStore, WhyObservation, WhySpec, WhyTarget};

use crate::bus::Deployment;
use crate::cli::WhyArgs;
use crate::cmd::zk2;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("why");

pub async fn run(cli: WhyArgs) -> Result<()> {
    let WhyArgs {
        target,
        for_secs,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let target = ASKING.ask(WhyTarget::parse(&target).map_err(anyhow::Error::from));
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    let spec = WhySpec {
        timeout: dep.timeout(),
        window,
    };
    // The rungs a key alone decides: no session for them.
    let offline =
        zenkey_fleet::judge_why(&WhyObservation::new(dep.namespace(), target.clone(), spec));
    let report = if matches!(offline.stopped_at, Some(RungId::Namespace | RungId::Key)) {
        offline
    } else {
        let set = ASKING.ask(zk2::load_contracts(&contracts));
        let store = BundleStore::new(dep.timeout());
        store.seed(&set);
        let session = ASKING.ask(dep.session().await);
        zenkey_fleet::run_why(&session, &store, dep.namespace(), target, spec).await
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    match &report.verdict {
        zenkey_fleet::Judgement::Unobservable { reason } => {
            eprintln!("why: {reason} — exit 2, the reserved non-verdict");
        }
        zenkey_fleet::Judgement::NotAsked => {
            eprintln!("why: the key's answer was not asked — exit 2, the reserved non-verdict");
        }
        _ => {}
    }
    crate::exit::verdict(&report.verdict)
}
