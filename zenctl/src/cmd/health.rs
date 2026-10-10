//! `health` (#721, PF) — every service's `health.v1`, or one's, read as the
//! profile's reader reads it (`spec/profiles/health/v1.md` §2.11).
//!
//! The reading is the engine's ([`zenkey_fleet::run_health`]): presence and
//! descriptors, a window's subscription to every status and to the faults,
//! two readings of the owners' `health.v1/state/**`, and for an absent owner
//! an archive's last-known status; every verdict decided from values by
//! `zenkey_model::health`. This command is orchestration and rendering:
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
use crate::cli::{FaceStatus, HealthArgs};

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
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
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
        }),
    };
    let target = address.map_or(HealthTarget::All, HealthTarget::One);
    let session = ASKING.ask(dep.session().await);
    if let Some(w) = window {
        eprintln!(
            "health: listening {}s to every status and to the faults before judging (--for; \
             --clocks-synced takes the deployment's word for this host's clock instead)",
            w.as_secs_f64()
        );
    }
    let report = zenkey_fleet::run_health(&session, dep.namespace(), target, spec).await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    let judgement = report.judgement();
    if let Judgement::Unobservable { reason } = &judgement {
        eprintln!("health: {reason} — exit 2, the reserved non-verdict");
    }
    crate::exit::verdict(&judgement)
}
