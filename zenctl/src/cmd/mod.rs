//! Command implementations — one module per command family (issue #46).
//!
//! `main.rs` stays a clap tree plus dispatch; everything a command *does*
//! lives here, and everything a command *puts on stdout* goes through a typed
//! report — `zenkey_fleet::report` for the shapes a second frontend would
//! render, `crate::render::impls::local` for the ones only this tool has —
//! put through the `render` seam.
//!
//! Two verbs put **nothing** on stdout, and that is the contract rather than
//! an omission: `pub` and `retire` answer "it went out", which is
//! not a document. All of their prose is stderr, which is what lets
//! `echo --format ndjson | pub --from ndjson` compose in either
//! direction without a wire shape being invented for a verb that has no
//! answer to give (#242). Their `--format` still chooses how the *notes* are
//! spelled, and `--format json` on them is an empty stdout by design.

pub mod acl;
pub mod admin;
pub mod base;
pub mod bench;
pub mod blob;
pub mod budget;
pub mod cache;
pub mod call;
pub mod cutover;
pub mod doctor;
pub mod echo;
pub mod expect;
pub mod field;
pub mod generate;
pub mod get;
pub mod interface;
pub mod key;
pub mod node;
pub mod probe;
pub mod publish;
pub mod rate;
pub mod record;
pub mod registry;
pub mod replay;
pub mod sample;
pub mod schema;
pub mod scout;
pub mod serve;
pub mod service;
pub mod snapshot;
pub mod storage;
pub mod topic;
pub mod watch;
pub mod watchdog;
pub mod why;

use anyhow::Result;

use crate::Bus;
use crate::cli::SelectorArgs;
use crate::exit::unaskable;

/// The raw-selector seam: every selector (or key) a user types, rather than
/// composes through positions, passes here before anything reaches the
/// session. RFC 03 §2 forbids `$*` in selectors, not merely in published
/// keys (RFC 02 P6: it is markedly slower and strains the infrastructure —
/// if a chunk needs `$*`, the key is wrong and wants splitting). zengui's
/// scope seam refuses it with this wording; the CLI must not be the frontend
/// that lets it through.
///
/// An [`Unaskable`](crate::exit::Unaskable), because it is this tool refusing
/// what you typed: exit 2, the same code clap uses, on every verb (#307). It
/// used to be a 1 on the listings and a 2 on the verdict verbs, which is one
/// mistake with two exit codes.
pub fn raw_selector(sel: &str) -> Result<&str> {
    if sel.contains("$*") {
        return Err(unaskable!(
            "`$*` must not be used in selectors (RFC 03 §2): {sel:?}"
        ));
    }
    Ok(sel)
}

/// Where a wire watcher looks: the typed selector, or the composed positions,
/// or the base's whole `v1` subtree (#307).
///
/// The one resolution of [`SelectorArgs`], so that `echo`, `rate`, `record`,
/// `field`, `check expect` and `why` cannot disagree about what "no selector"
/// means. Clap has already refused the both-at-once shape.
pub fn selector_of(sel: &SelectorArgs, args: &Bus) -> Result<String> {
    match sel.selector.as_deref() {
        // Typed selectors pass the raw seam (`$*` refusal, RFC 03 §2);
        // composed ones cannot spell it.
        Some(s) => Ok(raw_selector(s)?.to_string()),
        None => compose_selector(
            args,
            sel.origin.as_deref(),
            sel.class,
            sel.producer.as_deref(),
        ),
    }
}

/// Compose a server-side selector from origin/class/producer positions
/// (RFC 03: positions, not filters — never client-filter what the grammar
/// can say). `None` positions wildcard.
pub fn compose_selector(
    args: &Bus,
    origin: Option<&str>,
    class: Option<zenkey::Class>,
    producer: Option<&str>,
) -> Result<String> {
    // No validation here: `--class` is a `zenkey::Class` and clap rejected
    // anything else at the edge, with the vocabulary in the message (#351).
    let origin = origin.unwrap_or("*");
    let class = class.map_or("*", zenkey::Class::chunk);
    let rel = match producer {
        Some(p) => format!("v1/{origin}/{class}/{p}/**"),
        None if class == "*" => format!("v1/{origin}/**"),
        None => format!("v1/{origin}/{class}/**"),
    };
    args.wire(rel)
}

/// A positive number of seconds, or the refusal — the one spelling of the
/// check every `--for`/`--every` shares (#307).
///
/// Clap cannot express "greater than zero" on an `f64`, so this is the
/// nearest edge that can, and it answers the way clap would: exit 2.
pub fn positive_secs(flag: &str, secs: f64) -> Result<std::time::Duration> {
    if !secs.is_finite() || secs <= 0.0 {
        return Err(unaskable!(
            "{flag} must be a positive number of seconds, got {secs}"
        ));
    }
    Ok(std::time::Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_selector_places_positions() {
        // No config file: `bus_of` resolves against an absent context, so
        // this test no longer passes merely by pinning `base` and never
        // letting the ladder reach its second rung (#209).
        let args = crate::bus::tests::bus_of(Some("zs"));
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "zs/v1/*/**"
        );
        assert_eq!(
            compose_selector(
                &args,
                Some("h-3fa9c2d41b7e"),
                Some(zenkey::Class::State),
                None
            )
            .unwrap(),
            "zs/v1/h-3fa9c2d41b7e/state/**"
        );
        assert_eq!(
            compose_selector(&args, None, None, Some("tc")).unwrap(),
            "zs/v1/*/*/tc/**"
        );
        // The rejection moved to the edge: `--class` is a `zenkey::Class`,
        // so an unknown one never reaches this function — clap refuses it,
        // naming the vocabulary once rather than in three places (#351).
        let bad = "alerts".parse::<zenkey::Class>().unwrap_err().to_string();
        assert!(bad.contains("telemetry, state, events"), "{bad}");
        assert!(bad.contains("RFC 04 §1"), "{bad}");

        // The empty base composes bare `v1/…` selectors (observer identity).
        let args = crate::bus::tests::bus_of(Some(""));
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "v1/*/**"
        );
        assert_eq!(
            compose_selector(&args, None, Some(zenkey::Class::State), None).unwrap(),
            "v1/*/state/**"
        );
    }
}
