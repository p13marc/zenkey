//! `arbitration` (#909, BC) — each consumer's arbitrated role, read as the
//! profile's tool reads it (`spec/profiles/arbitration/v1.md` §2.14, §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_arbitration`]): the
//! consumers' presence and descriptors, the declaring contracts, every
//! revision of the required interface in hand, and the tool's own view of
//! each arbitrated, bound role over `--for`; every answer decided from
//! values by `zenkey_model::arbitration`. This command is orchestration and
//! rendering: resolve the deployment, open a session **in** its namespace
//! and a watch session in none, read, print, and exit through the report's
//! own judgement — 1 a finding (a declaration invalid, the view in the
//! dead-man, a wildcard or shadowed entry), 0 every question asked answered
//! on its other pole, 2 no verdict. A verdict verb: every failure before the
//! run is the reserved 2.
//!
//! **Why a root judgement, beside `timing` and `redundancy`.** "Which
//! provider does this role follow?" is §5's question in five states, and
//! its finding a pole, so it is exit-coded; it reads one profile across
//! the deployment, as they do. Its answer is always **the tool's own
//! view**: the consumer's switches are the consumer's to tell (§2.10).

use anyhow::Result;
use zenkey_fleet::{ArbitrationSpec, Judgement};

use crate::bus::Deployment;
use crate::cli::ArbitrationArgs;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("arbitration");

pub async fn run(cli: ArbitrationArgs) -> Result<()> {
    let ArbitrationArgs {
        address,
        role,
        for_secs,
        lease,
        wait_before_close,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    if !for_secs.is_finite() || for_secs < 0.0 {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--for must be zero or a positive number of seconds, got {for_secs}"
        )));
    }
    if let Some(r) = &role
        && !zenkey_model::chunk::is_ident(r)
    {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--role {r:?} is not a role's name: [a-z][a-z0-9_]*"
        )));
    }
    let mut spec = ArbitrationSpec::new(dep.timeout());
    spec.listen = std::time::Duration::from_secs_f64(for_secs);
    spec.role = role;
    spec.lease = ASKING.ask(super::positive_secs("--lease", lease));
    spec.wait_before_close = ASKING.ask(super::positive_secs(
        "--wait-before-close",
        wait_before_close,
    ));
    let session = ASKING.ask(dep.session().await);
    // The providers' interface tokens are followed on a watch session of
    // its own (#830, core §8.1, 0.37), in no namespace on the wire keys.
    let watch = match spec.listen.is_zero() {
        true => None,
        false => Some(ASKING.ask(dep.watch_session().await)),
    };
    let store = zk2::store(&dep, &offline);
    // No scope note here, as `timing` and `telemetry` give (#950): the view
    // subscribes to each binding entry's stream by its contract's key
    // expression, which names its kind token, and offers no wildcard scope.
    if !spec.listen.is_zero() {
        eprintln!(
            "arbitration: each arbitrated role's view listens {}s (--for): the tool's own, on \
             its own subscriptions, watch and clock, never the consumer's",
            spec.listen.as_secs_f64()
        );
    }
    let report = zenkey_fleet::run_arbitration(
        &session,
        watch.as_ref(),
        dep.namespace(),
        address,
        &store,
        &offline,
        spec,
    )
    .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("arbitration: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
