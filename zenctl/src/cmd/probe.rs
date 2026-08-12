//! `zenctl probe` (issue #59): the RFC 09 §6 acceptance, half two — a
//! **consumer-shaped, concrete-key** probe.
//!
//! > A probe MUST build its keys the way the product builds them.
//!
//! A `*`-origin probe cannot catch a broken origin path: the wildcard
//! matches any origin, so a caller whose origin concept is garbage still
//! gets replies. This probe resolves the identity the way a consumer must
//! (RFC 06 §6 — an origin id passes through; a hostname goes through the
//! health-document bridge) and then calls **origin-scoped**, through the
//! same typed builders the products use. Every failure mode is distinct
//! and cited, because "absence of replies and absence of *callers* must
//! not look alike" (§6).

use anyhow::{Result, bail};

use crate::BusArgs;
use crate::output;

pub async fn run(target: &str, producer: &str, procedure: &str, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let base = args.base().to_string();

    let (host, via) = match zenkey::origin::HostId::parse(target) {
        // The consumer already holds the origin — §6.2's preferred path.
        Ok(id) => (id, "direct".to_string()),
        Err(_) => {
            // A human identity: resolve through the sanctioned bridge
            // (health documents carry host_id beside source, §6.2).
            let (matches, seen) = zenkey_fleet::roster::bridge_resolve(
                &session,
                &base,
                producer,
                target,
                args.timeout(),
            )
            .await?;
            match matches.len() {
                0 => bail!(
                    "the identity bridge yielded no origin for {target:?} \
                     ({seen} health document(s) answered, none claiming it) — \
                     the probe FAILS here by rule: \"absence of replies and \
                     absence of callers must not look alike\" (RFC 09 §6). A \
                     hostname is a display label, not an identity (RFC 06 \
                     §6.1); if you hold the origin id, probe it directly."
                ),
                1 => {
                    let m = &matches[0];
                    eprintln!(
                        "bridge: {target:?} → {} (claimed by {}, RFC 06 §6.2)",
                        m.host_id.as_str(),
                        m.key
                    );
                    (
                        matches[0].host_id.clone(),
                        format!("bridge:{}", matches[0].key),
                    )
                }
                n => bail!(
                    "hostname {target:?} is claimed by {n} distinct origins — \
                     exactly the collision RFC 06 §6.2 warns about (a table \
                     keyed on hostnames misroutes queries). Name one origin: {}",
                    matches
                        .iter()
                        .map(|m| m.host_id.as_str().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };

    // Origin-scoped, concrete-key, through the same typed builders the
    // products use (zenkey::selector::rpc_at inside the engine's call).
    let call = zenkey_fleet::call(
        &session,
        &base,
        &zenkey_fleet::CallTarget::Host(host.clone()),
        producer,
        procedure,
        &[],
        None,
        None,
        args.timeout(),
        args.slice_set().await.ok().as_ref(),
    )
    .await?;

    let report = zenkey_fleet::report::ProbeReport {
        input: target.to_string(),
        origin: host.as_str().to_string(),
        via,
        call,
    };
    output::probe(&report, args.format);
    // Distinct exits, distinct meanings (RFC 05 §3.1 / 09 §6): 1 = the
    // origin answered with an error; 2 = the origin resolved but did not
    // answer — the probe reached a *name*, not a *responder*, and that is
    // its own finding, never folded into "no such origin".
    let code = report.call.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
