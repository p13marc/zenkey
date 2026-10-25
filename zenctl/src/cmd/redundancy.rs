//! `redundancy` (#841, RC) — who serves each governed interface, read as
//! the profile's tool reads it (`spec/profiles/redundancy/v1.md` §2.15,
//! §5).
//!
//! The reading is the engine's ([`zenkey_fleet::run_redundancy`]): two
//! presence reads of the address's `@zk/**` a grace apart, each with its
//! descriptors, and the revisions they name; every answer decided from
//! values by `zenkey_model::redundancy`. This command is orchestration and
//! rendering: resolve the deployment, open a session **in** its namespace,
//! read, print, and exit through the report's own judgement.
//!
//! **The exit contract.** 1 a split (two or more holders of a governed
//! interface's token in both reads), a vacancy in both reads, or a twin:
//! §5's "Finding on" column, whose first question finds a no and whose
//! second a yes, `lasting`. A lasting vacancy is a 1 because §5 makes it
//! the second question's finding: no candidate serves a governed interface
//! across a whole grace, the election stuck or, under `external`, no
//! instance holding the lock. 0 every governed interface asked served by
//! exactly one incumbent, each instance the incumbent, a standby or no
//! candidate. 2 everything else: a read not complete where it decides, an
//! unread descriptor or contract, a shape seen in one read only (a
//! takeover due, a re-mint's overlap), nothing governed in scope, an
//! interface not this profile's, or a run that could not start.
//!
//! **Why a root judgement, beside `health` and `desired`.** "Does exactly
//! one instance serve it?" is a question whose finding is the no, in five
//! states, so it is exit-coded; and it reads one profile across the
//! deployment, as they do.

use anyhow::Result;
use zenkey_fleet::{Judgement, RedundancySpec};

use crate::bus::Deployment;
use crate::cli::RedundancyArgs;
use crate::cmd::zk2;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("redundancy");

pub async fn run(cli: RedundancyArgs) -> Result<()> {
    let RedundancyArgs {
        address,
        iface,
        grace,
        across_face,
        contracts,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = ASKING.ask(zk2::load_contracts(&contracts));
    let grace = ASKING.ask(super::positive_secs("--grace", grace));
    let mut spec = RedundancySpec::new(dep.timeout());
    spec.grace = grace;
    spec.iface = iface;
    spec.across_face = across_face;
    let session = ASKING.ask(dep.session().await);
    let store = zk2::store(&dep, &offline);
    let report =
        zenkey_fleet::run_redundancy(&session, dep.namespace(), address, spec, &store, &offline)
            .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("redundancy: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
