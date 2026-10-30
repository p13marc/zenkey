//! `alarms` (#800, AC) — the alarms.v1 alarms a selector reaches, read as
//! the profile's reader reads them (`spec/profiles/alarms/v1.md` §2.14,
//! §2.15, §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_alarms`]): presence and
//! descriptors, S4's GET merged with a window's subscription when `--for`
//! listens, every archive's last-known documents, and every verdict decided
//! from values by `zenkey_model::alarms`. This command is orchestration and
//! rendering: resolve the deployment, open a session **in** its namespace,
//! read, print, and exit through the report's own judgement — §2.15's
//! answer at the floor: 1 an alarm firing at or above it, 0 none with every
//! owner asked known whole and the presence read that found them complete,
//! 2 no verdict. A verdict verb: every failure before the run is the
//! reserved 2.
//!
//! **Why a root judgement, beside `health`.** "Is anything firing?" is a
//! question whose finding is the yes, in three states — so it is exit-coded,
//! a judgement, not a noun: an `alarm list` would print an empty list and
//! exit 0 over an owner it could not read, the "nothing fires" alarms.v1
//! §2.11 forbids. It reads one profile across the deployment, as `health`
//! does, rather than asserting one thing an operator names, as the `check`
//! family does.

use anyhow::Result;
use zenkey_fleet::{AlarmScope, AlarmSpec, Judgement};

use crate::bus::Deployment;
use crate::cli::{AlarmsArgs, FaceGrant};
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("alarms");

pub async fn run(cli: AlarmsArgs) -> Result<()> {
    let AlarmsArgs {
        selector,
        at_least,
        clocks_synced,
        for_secs,
        lease,
        wait_before_close,
        across_face,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    // No scope note here, as `timing` and `telemetry` give (#950): every
    // alarms.v1 key is plain state, `zk2/<s>/<v>/alarms.v1/state/alarms/<key>`,
    // so the verbatim chunks a wildcard excludes hold no alarm.
    let scope = ASKING.ask(AlarmScope::parse(&selector).map_err(|why| unaskable!("{why}")));
    if across_face.is_some() && scope.one_owner().is_none() {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--across-face takes the deployment's word for one owner: name its system and \
             service in the selector (zk2/<system>/<service>/…)"
        )));
    }
    let window = for_secs.map(|s| ASKING.ask(super::positive_secs("--for", s)));
    let mut spec = AlarmSpec::new(dep.timeout());
    spec.window = window;
    spec.clocks_synced = clocks_synced;
    spec.lease = ASKING.ask(super::positive_secs("--lease", lease));
    spec.wait_before_close = ASKING.ask(super::positive_secs(
        "--wait-before-close",
        wait_before_close,
    ));
    spec.face = across_face.map(|f| f == FaceGrant::Granted);
    let session = ASKING.ask(dep.session().await);
    // A window follows the owners' instance tokens, on a watch session of
    // its own (#830, core §8.1, 0.37); the reads stay on `session`.
    let watch = match window {
        Some(_) => Some(ASKING.ask(dep.watch_session().await)),
        None => None,
    };
    // The line comes once the window opens (#819 TE, as zk2py round 47's
    // F-154 found of `telemetry`): the subscriptions declared and the first
    // refresh read after them, so a put once it is read is heard.
    // The watch after the window: the longer of --lease and
    // --wait-before-close, and --timeout (observer §5.2, zk2py round 85's
    // F-320).
    let tail = spec.book().tail().as_secs_f64();
    let listening = move || {
        if let Some(w) = window {
            eprintln!(
                "alarms: listening {}s to the alarms and the owners' tokens, then {tail}s more \
                 while the owners are watched (--for, the longer of --lease and \
                 --wait-before-close, and --timeout)",
                w.as_secs_f64(),
            );
        }
    };
    let report = zenkey_fleet::run_alarms(
        &session,
        watch.as_ref(),
        dep.namespace(),
        scope,
        spec,
        at_least.severity(),
        listening,
    )
    .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("alarms: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
