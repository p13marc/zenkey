//! `desired` (#840, DD) — each desired.v1 target's roles against their
//! authors' documents, read as the profile's tool reads them
//! (`spec/profiles/desired/v1.md` §2.17, §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_desired`]): presence
//! and descriptors, the revisions they name, each target's reports by type
//! identity and S4's GET of each slice from its author, again within a
//! window; every verdict decided from values by `zenkey_model::desired`.
//! This command is orchestration and rendering: resolve the deployment,
//! open a session **in** its namespace, read, print, and exit through the
//! report's own judgement — 1 a refused or failed act, or a target behind
//! past `--for`, 0 every role asked converged on its author's current
//! document, 2 no verdict. A verdict verb: every failure before the run is
//! the reserved 2.
//!
//! **Why a root judgement, beside `health` and `alarms`.** "Has the target
//! converged?" is a question whose finding is the no, in five states, so
//! it is exit-coded, a judgement, not a noun; and it reads one profile
//! across the deployment, as they do, rather than asserting one thing an
//! operator names, as the `check` family does. A `desired list` would print
//! a report a target never put, or one past its horizon, as converged.
//!
//! **Why a window, and what it is.** §2.17's "not yet" is no verdict: time
//! or the target's next seed may change it. `--for` turns "not yet" into a
//! question with a finding, "has the target been behind one version of its
//! author's for this long?" (text 0.4): the reading is made again every
//! 500 ms while an answer may move, so a target that converges inside the
//! window is read converged, and one still "not yet" on the same version
//! at its end is behind. A single reading never finds a target behind.
//!
//! **No `--across-face`.** Across a constrained face neither presence nor
//! the descriptor crosses, so the deployment's word must say the role, its
//! binding and its author, and `--contracts` must hold both contracts. The
//! engine takes that word ([`zenkey_fleet::DesiredFaceWord`]); this verb
//! does not yet, for want of a deployment that reads desired state across a
//! face.

use anyhow::Result;
use zenkey_fleet::{DesiredSpec, Judgement};

use crate::bus::Deployment;
use crate::cli::DesiredArgs;
use crate::cmd::zk2;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("desired");

pub async fn run(cli: DesiredArgs) -> Result<()> {
    let DesiredArgs {
        target,
        role,
        for_secs,
        clocks_synced,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let window = for_secs.map(|s| ASKING.ask(super::positive_secs("--for", s)));
    let mut spec = DesiredSpec::new(dep.timeout());
    spec.window = window;
    spec.clocks_synced = clocks_synced;
    spec.role = role;
    let session = ASKING.ask(dep.session().await);
    let store = zk2::store(&dep, &offline);
    if let Some(w) = window {
        eprintln!(
            "desired: reading again every 500 ms for up to {}s while an answer may move (--for)",
            w.as_secs_f64()
        );
    }
    let report = zenkey_fleet::run_desired(
        &session,
        dep.namespace(),
        target,
        spec,
        &store,
        &offline,
        |_| {},
    )
    .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    match &judgement {
        Judgement::Unobservable { reason } => {
            eprintln!("desired: {reason} — exit 2, the reserved non-verdict");
        }
        Judgement::NotAsked => eprintln!(
            "desired: no role was asked — no target in scope, or every one's role unbound or \
             not listed — exit 2, the reserved non-verdict"
        ),
        _ => {}
    }
    crate::exit::verdict(&judgement)
}
