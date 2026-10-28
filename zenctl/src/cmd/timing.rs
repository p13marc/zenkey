//! `timing` (#908, MC) — the timing.v1 streams a scope reaches, over one
//! window, read as the profile's tool reads them
//! (`spec/profiles/timing/v1.md` §2.5–§2.8, §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_timing`]): presence and
//! descriptors, the revisions they name, every timed stream they declare,
//! then a window over one queue of the owners' tokens and the members'
//! samples, each arrival read in its subscriber's callback; every verdict
//! decided from values by `zenkey_model::timing`. This command is
//! orchestration and rendering: resolve the deployment, open a session
//! **in** its namespace and a watch session in none, read, print, and exit
//! through the report's own judgement — 1 a finding (a deadline missed with
//! the owner present and nothing lost, a period not kept likewise, a sample
//! expired), 0 every question asked answered on its other pole, 2 no
//! verdict. A verdict verb: every failure before the run is the reserved 2.
//!
//! **Why a root judgement, beside `telemetry`.** "Did this member meet its
//! deadline?" is a question whose finding is a pole, in three states, so it
//! is exit-coded; and it reads one profile across the deployment, as
//! `health` and `telemetry` do. `rate` stays message throughput: a period
//! is the contract's promise, and only this verb holds a member to it.
//!
//! **ε is the operator's word** (`--clock-bound`, §2.6): never the HLC's
//! delta. Without it every lifespan is unobservable, so a run over a
//! resource that declares one exits 2 — honest, and `--clock-bound` the
//! remedy, as the line says.

use anyhow::Result;
use zenkey_fleet::{Judgement, TimingScope, TimingSpec};

use crate::bus::Deployment;
use crate::cli::TimingArgs;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("timing");

/// What the "listening" line says the window started on (§2.7, F-263).
pub fn opening_words(o: &zenkey_fleet::report::TimingOpening) -> String {
    if o.presence == "across_face" {
        return "; across the face no presence or descriptor is read: every owner is unseen, \
                and the window starts after --settle, a margin and never proof"
            .to_owned();
    }
    if o.stood {
        return format!(
            "; its start stands: a complete presence read after the watch session's round \
             trip, and the descriptor of each of the {} instance(s) in scope it showed",
            o.instances
        );
    }
    let mut why = Vec::new();
    if o.presence != "complete" {
        why.push(format!("the presence read was {}", o.presence));
    }
    if let Some(first) = o.unanswered.first() {
        why.push(format!(
            "{} of {} descriptor(s) did not answer (the first, {first})",
            o.instances - o.descriptors,
            o.instances
        ));
    }
    format!(
        "; its start does not stand ({}): an owner it did not see is unseen at first, which \
         §5 reads as not present, and a sample it cannot attribute unattributed",
        why.join("; ")
    )
}

pub async fn run(cli: TimingArgs) -> Result<()> {
    let TimingArgs {
        selector,
        for_secs,
        clock_bound,
        lease,
        wait_before_close,
        across_face,
        owner_zid,
        settle,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let scope = match &selector {
        Some(s) => ASKING.ask(TimingScope::parse(s).map_err(|why| unaskable!("{why}"))),
        None => TimingScope::all(),
    };
    if across_face && scope.one_owner().is_none() {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--across-face takes the deployment's word for one owner: name its system and \
             service in the selector (zk2/<system>/<service>/…)"
        )));
    }
    if across_face && offline.is_empty() {
        ASKING.ask::<(), _>(Err(unaskable!(
            "--across-face reads the owner's timed streams from the contracts in hand, since \
             descriptors do not cross a face: give --contracts"
        )));
    }
    let owner_zids: Vec<u128> = owner_zid
        .iter()
        .map(|z| {
            ASKING.ask(zenkey_model::timing::zid_value(z).ok_or_else(|| {
                unaskable!("--owner-zid {z:?} is not a zid: 1 to 32 hexadecimal digits")
            }))
        })
        .collect();
    let window = ASKING.ask(super::positive_secs("--for", for_secs));
    let mut spec = TimingSpec::new(dep.timeout(), window);
    spec.clock_bound = clock_bound.map(std::time::Duration::from_millis);
    spec.lease = ASKING.ask(super::positive_secs("--lease", lease));
    spec.wait_before_close = ASKING.ask(super::positive_secs(
        "--wait-before-close",
        wait_before_close,
    ));
    spec.across_face = across_face;
    spec.owner_zids = owner_zids;
    spec.settle = ASKING.ask(std::time::Duration::try_from_secs_f64(settle).map_err(|_| {
        if settle.is_finite() && settle > 0.0 {
            unaskable!("--settle is longer than a wait can be: {settle:e} seconds")
        } else {
            unaskable!("--settle must be zero or a positive number of seconds, got {settle}")
        }
    }));
    let session = ASKING.ask(dep.session().await);
    // The owners' tokens are followed on a watch session of its own (#830,
    // core §8.1, 0.37); across a face no presence is read, and none opens.
    let watch = match across_face {
        true => None,
        false => Some(ASKING.ask(dep.watch_session().await)),
    };
    let store = zk2::store(&dep, &offline);
    let listening = move |opening: &zenkey_fleet::report::TimingOpening| {
        eprintln!(
            "timing: listening {}s to the timed members in scope and their owners' tokens \
             (--for){}{}",
            window.as_secs_f64(),
            opening_words(opening),
            match clock_bound {
                Some(ms) => format!("; lifespans judged under ε = {ms} ms (--clock-bound)"),
                None => "; no --clock-bound: every lifespan is unobservable (no_bound), never \
                         judged on the HLC's delta"
                    .to_owned(),
            }
        );
    };
    let report = zenkey_fleet::run_timing(
        &session,
        watch.as_ref(),
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
        eprintln!("timing: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
