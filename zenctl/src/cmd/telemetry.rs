//! `telemetry` (#819, TC) — the telemetry.v1 metric members a scope
//! reaches, read as the profile's reader reads them
//! (`spec/profiles/telemetry/v1.md` §2.5–§2.11, §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_telemetry`]): presence
//! and descriptors, the revisions they name, every metric resource they
//! declare, then a window over one queue of the owners' tokens and the
//! members' samples, a presence seed, and S4's GET of each state metric
//! resource; every verdict decided from values by
//! `zenkey_model::telemetry`. This command is orchestration and rendering:
//! resolve the deployment, open a session **in** its namespace, read, print,
//! and exit through the report's own judgement — 1 a sample that does not
//! conform or a decrease within an established epoch, 0 every question
//! asked answered without one, 2 no verdict. A verdict verb: every failure
//! before the run is the reserved 2.
//!
//! **Why a root judgement, beside `health` and `alarms`.** "Did a counter go
//! backwards within an epoch?" is a question whose finding is the yes, in
//! three states, so it is exit-coded; and it reads one profile across the
//! deployment, as they do, rather than asserting one thing an operator
//! names, as the `check` family does. `rate` stays message throughput: a
//! counter's rate is a pair's, and a pair is this verb's.
//!
//! **No `--clocks-synced`.** §2.11's time bases are a Point's `timestamp`,
//! the owner's stamp and this host's receipt, and a pair is timed by two of
//! one base: two of the owner's clock, or two of this host's. Nothing
//! compares the owner's clock with this host's, so nothing here trusts one,
//! and there is no word of the deployment's to take.

use anyhow::Result;
use zenkey_fleet::{Judgement, TelemetryScope, TelemetrySpec};

use crate::bus::Deployment;
use crate::cli::TelemetryArgs;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("telemetry");

pub async fn run(cli: TelemetryArgs) -> Result<()> {
    let TelemetryArgs {
        selector,
        for_secs,
        lease,
        wait_before_close,
        across_face,
        settle,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let scope = match &selector {
        Some(s) => ASKING.ask(TelemetryScope::parse(s).map_err(|why| unaskable!("{why}"))),
        None => TelemetryScope::all(),
    };
    if across_face && scope.one_owner().is_none() {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--across-face takes the deployment's word for one owner: name its system and \
             service in the selector (zk2/<system>/<service>/…)"
        )));
    }
    if across_face && offline.is_empty() {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--across-face reads the owner's metric resources from the contracts in hand, since \
             descriptors do not cross a face: give --contracts"
        )));
    }
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    let mut spec = TelemetrySpec::new(dep.timeout(), window);
    spec.lease = ASKING.ask(super::positive_secs("--lease", lease));
    spec.wait_before_close = ASKING.ask(super::positive_secs(
        "--wait-before-close",
        wait_before_close,
    ));
    spec.across_face = across_face;
    if !settle.is_finite() || settle < 0.0 {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--settle must be zero or a positive number of seconds, got {settle}"
        )));
    }
    spec.settle = std::time::Duration::from_secs_f64(settle);
    let session = ASKING.ask(dep.session().await);
    let store = zk2::store(&dep, &offline);
    // The line comes once the window opens (zk2py round 47's F-154): the
    // subscriptions declared and a round trip made after them that reaches
    // each owner — its descriptor GET after the presence seed, or across a
    // face S4's GET of a state metric resource (round 49's F-159; #819 TG)
    // — so a sample put once it is read is inside the window. When an
    // owner did not answer, or across a face nothing could, the line says
    // so.
    let listening = move |opening: zenkey_fleet::TelemetryOpening| {
        use zenkey_fleet::TelemetryOpening as O;
        eprintln!(
            "telemetry: listening {}s to the metric members in scope{} (--for){}",
            window.as_secs_f64(),
            if across_face {
                "; across a face this reader reads no presence (the session opens as any \
                 other's), so only an in-band epoch establishes one"
            } else {
                " and their owners' tokens"
            },
            match opening {
                O::Answered { .. } => String::new(),
                O::Unanswered { silent, asked } => format!(
                    "; {silent} of the {asked} owner instance(s) in scope answered no GET \
                     after the subscriptions within --timeout, so they may not hold them yet: \
                     a sample one puts at once may be dropped before it leaves"
                ),
                O::Margin(m) => format!(
                    "; across the face no state metric resource in scope answered a GET, so \
                     nothing proved the far side subscribed: this line comes after a {}s \
                     margin (--settle), never proof",
                    m.as_secs_f64()
                ),
            }
        );
    };
    let report = zenkey_fleet::run_telemetry(
        &session,
        dep.namespace(),
        scope,
        &store,
        &offline,
        spec,
        listening,
    )
    .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("telemetry: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
