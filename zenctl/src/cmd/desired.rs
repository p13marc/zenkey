//! `desired` (#840, DD; #986's J4) — each desired.v1 or desired.v2 target's
//! roles against their authors' documents, read as the profile's tool reads
//! them (`spec/profiles/desired/v1.md` §2.17, §5; `v2.md` §2.10, §2.17), each
//! role by the major its author's revision lists (`desired.v2` §2.18).
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
//! Under desired.v2 the window is the only way: a version is a run and a
//! seq, with no time for `--clocks-synced` to age (v2 §2.17).
//!
//! **`--across-face`** (#986's J5). Across a constrained face neither
//! presence nor the descriptor crosses, so the deployment's word says the
//! role (`--role`), its binding (`--author`, or `--unbound`, the kill
//! switch) and its parameters (`--param`, else the author's declaration's
//! order), and `--contracts` holds the revisions: the one declaring the
//! role and the author's, the slice's. The engine takes that word
//! ([`zenkey_fleet::DesiredFaceWord`]) and reads no presence, the target's
//! nor its author's; the report and the slice are one GET each. With
//! `--face-policy` and `--face` the session runs the face's transport, and
//! every read waits at least the face's `reply_timeout_ms` (core §8.1,
//! observer §7.5), a floor no `--timeout` lowers: a reply that crosses after
//! a shorter wait crosses for nothing.

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
        across_face,
        author,
        unbound,
        params,
        face_policy,
        face,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let dep = ASKING.ask(zk2::across(dep, face_policy.as_deref(), face.as_deref()));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let window = for_secs.map(|s| ASKING.ask(super::positive_secs("--for", s)));
    // Across a face's policy, its reply timeout is every read's floor (core
    // §8.1, observer §7.5), whatever --timeout says.
    let timeout = dep
        .far()
        .map_or(dep.timeout(), |f| f.at_least(dep.timeout()));
    let mut spec = DesiredSpec::new(timeout);
    // desired.v2 §2.17 step 4: the epoch a report under freshness.v2 is
    // dated under (core §8.6), from `--catalog`.
    spec.catalogs = ASKING.ask(zk2::load_catalogs(&contracts));
    spec.window = window;
    spec.clocks_synced = clocks_synced;
    if across_face {
        let role = role.as_deref().unwrap_or_default();
        spec.face = Some(ASKING.ask(face_word(role, author, unbound, params, &offline)));
    }
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

/// The deployment's word for one role across a face (`--across-face`).
/// Refused, exit 2: no word on the binding, no contract in hand, and a
/// declaration of one target parameter with no `--param`
/// ([`zenkey_fleet::DesiredFaceWord::of_contracts`]).
fn face_word(
    role: &str,
    author: Option<zenkey_model::grammar::Addr>,
    unbound: bool,
    params: Vec<(String, String)>,
    offline: &zenkey_fleet::ContractSet,
) -> Result<zenkey_fleet::DesiredFaceWord> {
    if author.is_none() && !unbound {
        return Err(crate::exit::unaskable!(
            "--across-face: no descriptor crosses a face, so the deployment's word names the \
             role's binding: --author SYSTEM/SERVICE, or --unbound for the kill switch \
             (desired.v1 §2.3, §2.12)"
        ));
    }
    if offline.is_empty() {
        return Err(crate::exit::unaskable!(
            "--across-face: no descriptor crosses a face, so --contracts must hold the target's \
             revision that declares role {role} and its author's (observer §7.5)"
        ));
    }
    zenkey_fleet::DesiredFaceWord::of_contracts(role, author, params.into_iter().collect(), offline)
        .map_err(|why| crate::exit::unaskable!("--across-face: {why}"))
}
