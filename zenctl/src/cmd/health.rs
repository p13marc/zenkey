//! `health` (#721, PF; #986's Q2) — every service's `health.v1` or
//! `health.v2`, or one's, read as the profile's reader reads it
//! (`spec/profiles/health/v1.md` §2.11, `v2.md` §2.11), each by the major
//! its descriptor lists, `health.v2` where it lists both.
//!
//! The reading is the engine's ([`zenkey_fleet::run_health`]): presence and
//! descriptors, a window's subscription to every status and to the faults
//! under both majors, two readings of the owners' `health.v2/state/**` and
//! `health.v1/state/**`, the tables of each `health.v2` owner's words from
//! the revisions its descriptor names (`--contracts`, else retrieved), and
//! for an absent owner an archive's last-known status; every verdict
//! decided from values by `zenkey_model::health`. This command is orchestration and rendering:
//! resolve the deployment, open a session **in** its namespace, read, print,
//! and exit through the report's own judgement — 1 on a service unhealthy,
//! stale, breaking §2.2 in both readings or running its clock ahead, 0 when
//! every service asked is healthy, 2 when one is unobservable, none was
//! asked, or the run could not start. A verdict verb: every failure before
//! the run is the reserved 2.
//!
//! **The clock** (`freshness.v1` §2.6). A status reply's stamp is aged only
//! against a clock trusted to the HLC delta. `--clocks-synced` is the
//! deployment's word; without it the window measures this host's clock on
//! the owners' live puts, and lasts by default just over the 30 s an owner
//! re-puts its status within, so that every live status is heard once
//! (health.v1 0.2, F-104).

use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::{AcrossFace, HealthSpec, HealthTarget, Judgement};

use crate::bus::Deployment;
use crate::cli::{FaceStatus, HealthArgs, HealthIface};

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("health");

/// The default window: just over the 30 s an owner re-puts its status
/// within (health.v1 §2.3), so every live status is heard at least once.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(31);

pub async fn run(cli: HealthArgs) -> Result<()> {
    let HealthArgs {
        address,
        clocks_synced,
        for_secs,
        grace,
        across_face,
        iface,
        contracts,
        catalog,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let offline = crate::cli::ContractArgs { contracts, catalog };
    let catalogs = ASKING.ask(super::zk2::load_catalogs(&offline));
    let contracts = ASKING.ask(super::zk2::load_contracts(&offline));
    let grace = ASKING.ask(super::positive_secs("--grace", grace));
    let window = match for_secs {
        Some(s) => Some(ASKING.ask(super::positive_secs("--for", s))),
        None if clocks_synced => None,
        None => Some(DEFAULT_WINDOW),
    };
    let spec = HealthSpec {
        timeout: dep.timeout(),
        window,
        grace,
        clocks_synced,
        face: across_face.map(|f| AcrossFace {
            status_crosses: f == FaceStatus::Crosses,
            major: match iface {
                Some(HealthIface::V2) => zenkey_model::health::Major::V2,
                Some(HealthIface::V1) | None => zenkey_model::health::Major::V1,
            },
        }),
        catalogs,
        contracts,
    };
    let target = address.map_or(HealthTarget::All, HealthTarget::One);
    let session = ASKING.ask(dep.session().await);
    // The line comes once the window opens (#819 TE, as zk2py round 47's
    // F-154 found of `telemetry`): the subscriptions declared and presence
    // read after them, so a status or a fault put once it is read is heard.
    let listening = move |opening: zenkey_fleet::HealthOpening| {
        if let Some(w) = window {
            eprintln!(
                "health: listening {}s to every status and to the faults before judging (--for; \
                 --clocks-synced takes the deployment's word for this host's clock instead){}",
                w.as_secs_f64(),
                match opening {
                    zenkey_fleet::HealthOpening::Answered => String::new(),
                    zenkey_fleet::HealthOpening::Unanswered(why) => format!(
                        "; {why}, so nothing proved the owners hold the subscription: a status \
                         or a fault put at once may be missed"
                    ),
                }
            );
        }
    };
    let report = zenkey_fleet::run_health(&session, dep.namespace(), target, spec, listening).await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("health: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
