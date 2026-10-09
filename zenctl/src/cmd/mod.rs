//! Command implementations — one module per command family (issue #46).
//!
//! `main.rs` stays a clap tree plus dispatch; everything a command *does*
//! lives here, and everything a command *puts on stdout* goes through a typed
//! report — `zenkey_fleet::report` for the shapes a second frontend would
//! render, `crate::render::impls::local` for the ones only this tool has —
//! put through the `render` seam.
//!
//! One verb puts **nothing** on stdout, and that is the contract rather than
//! an omission: `pub` answers "it went out", which is not a document. All of
//! its prose is stderr, which is what lets `echo --format ndjson | pub
//! --from ndjson` compose in either direction without a wire shape being
//! invented for a verb that has no answer to give (#242). Its `--format`
//! still chooses how the *notes* are spelled, and `--format json` on it is an
//! empty stdout by design. (`retire` was the other, until FJ5 dropped it.)

pub mod acl;
pub mod admin;
pub mod bench;
pub mod blob;
pub mod cache;
pub mod call;
pub mod compat;
pub mod config;
pub mod doctor;
pub mod echo;
pub mod expect;
pub mod export;
pub mod field;
pub mod generate;
pub mod get;
pub mod graph;
pub mod iface;
pub mod key;
pub mod namespace;
pub mod probe;
pub mod publish;
pub mod rate;
pub mod record;
pub mod replay;
pub mod sample;
pub mod schema;
pub mod scout;
pub mod serve;
pub mod service;
pub mod snapshot;
pub mod storage;
pub mod subscribe;
pub mod timeline;
pub mod watch;
pub mod watchdog;
pub mod zk2;

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

/// The hint for a base-relative selector typed under a non-empty base, or
/// `None` when there is nothing to say (#512).
///
/// Wire verbs take **wire keys**: an explorer runs un-namespaced and `--base`
/// is for discovery (RFC 09 §5), so `v1/**` under `--base prod` is a
/// subscription to a keyspace nobody publishes on — silence, with no error to
/// say why. Not rewritten: the wire is what you typed, and this only says so.
///
/// Only a selector whose first chunk is the grammar's root `v1` is the
/// mistake. The rest that do not start with the base are deliberate and stay
/// quiet: `@/…` (the admin space, under no base), a leading wildcard (`**/v1/…`
/// spans every base, which is how a leak is found), and another deployment's
/// own wire keys (`staging/v1/…`) — those *are* wire keys, which is the thing
/// the hint would ask for.
pub fn off_base_hint(sel: &str, base: &str) -> Option<String> {
    if base.is_empty() || zenkey::grammar::strip_base(base, sel).is_some() {
        return None;
    }
    let first = sel.split(['/', '?']).next().unwrap_or_default();
    (first == "v1").then(|| {
        let wire = zenkey::grammar::with_base(base, sel);
        format!(
            "hint: {sel:?} does not sit under base {base:?} — selectors are wire \
             keys (RFC 09 §5); did you mean {wire:?}?"
        )
    })
}

/// [`off_base_hint`] onto stderr — never stdout, which `--format json|ndjson`
/// keeps for the document. One line, and the run carries on.
pub fn hint_off_base(sel: &str, args: &Bus) {
    if let Some(hint) = off_base_hint(sel, args.base()) {
        eprintln!("{hint}");
    }
}

/// Where a wire watcher looks: the typed selector, or the composed positions,
/// or the base's whole `v1` subtree (#307).
///
/// The one resolution of [`SelectorArgs`], so that `echo`, `rate`, `record`,
/// `field`, `check expect` and `why` cannot disagree about what "no selector"
/// means. Clap has already refused the both-at-once shape. A typed selector
/// that reads base-relative under a non-empty base gets the one-line
/// [`off_base_hint`] (#512); composed ones are under the base by
/// construction.
pub fn selector_of(sel: &SelectorArgs, args: &Bus) -> Result<String> {
    let selector = selector_unhinted(sel, args)?;
    if sel.selector.is_some() {
        hint_off_base(&selector, args);
    }
    Ok(selector)
}

/// [`selector_of`] without the hint — for `why`, whose `key-parse` rung
/// already says the same thing as a finding, with its citation.
pub fn selector_unhinted(sel: &SelectorArgs, args: &Bus) -> Result<String> {
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

/// How an output file is opened — decided before any session opens (#514).
///
/// `record -o` and `snapshot -o` used to `File::create`, which truncates:
/// re-running yesterday's command line during an incident destroyed
/// yesterday's capture, the one artifact either verb exists to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Nothing is there: create it, refusing one that appears meanwhile
    /// (`create_new` — the check and the create are one syscall).
    New,
    /// `--overwrite`: create or truncate, as asked.
    Replace,
    /// Something that is not a regular file — `/dev/null`, a fifo. There is
    /// no capture to destroy, so it is written to as it stands.
    Device,
}

/// Decide how `out` will be opened, refusing an existing regular file
/// unless `overwrite` — exit 2 through [`Unaskable`](crate::exit::Unaskable),
/// because it is this tool refusing what you typed, and before the session
/// opens because nothing about the bus can change the answer.
pub fn output_mode(out: &str, overwrite: bool) -> Result<OutputMode> {
    if overwrite {
        return Ok(OutputMode::Replace);
    }
    match std::fs::metadata(out) {
        Ok(m) if m.is_file() => Err(refuse_existing(out)),
        Ok(_) => Ok(OutputMode::Device),
        // Absent — or a dangling link, which `create_new` refuses at open.
        Err(_) => Ok(OutputMode::New),
    }
}

/// The refusal [`output_mode`] and a lost `create_new` race share.
pub fn refuse_existing(out: &str) -> anyhow::Error {
    unaskable!("--out {out}: the file already exists — pass --overwrite to replace it")
}

/// Open `out` the way [`output_mode`] decided, off the runtime (#332).
pub async fn open_output(out: &str, mode: OutputMode) -> std::io::Result<std::fs::File> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true);
    match mode {
        OutputMode::New => opts.create_new(true),
        OutputMode::Replace => opts.create(true).truncate(true),
        OutputMode::Device => &mut opts,
    };
    Ok(opts.open(out).await?.into_std().await)
}

/// [`open_output`] with the refusal and the path in the error.
pub async fn open_output_or_refuse(out: &str, mode: OutputMode) -> Result<std::fs::File> {
    open_output(out, mode).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            refuse_existing(out)
        } else {
            anyhow::Error::new(e).context(format!("create {out}"))
        }
    })
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
    fn a_base_relative_selector_under_a_base_is_hinted_and_nothing_else_is() {
        let hint = off_base_hint("v1/**", "prod").expect("the mistake is hinted");
        assert!(
            hint.contains(r#""v1/**" does not sit under base "prod""#),
            "{hint}"
        );
        assert!(hint.contains(r#"did you mean "prod/v1/**"?"#), "{hint}");
        assert!(off_base_hint("v1/*/state/sysinfo/health", "site/prod").is_some());
        // A GET's parameters ride along into the suggestion.
        assert!(
            off_base_hint("v1/**?_time=[now(-1h)..]", "prod")
                .unwrap()
                .contains(r#""prod/v1/**?_time=[now(-1h)..]""#)
        );
        // Quiet: the wire-key form, the empty base, the admin space, a
        // leading wildcard, another deployment's wire keys, a bare `v1x`.
        for (sel, base) in [
            ("prod/v1/**", "prod"),
            ("v1/**", ""),
            ("@/**", "prod"),
            ("**/v1/**", "prod"),
            ("*/v1/*/state/**", "prod"),
            ("staging/v1/**", "prod"),
            ("v1x/**", "prod"),
            ("prod/@catalog/**", "prod"),
        ] {
            assert_eq!(off_base_hint(sel, base), None, "{sel} under {base:?}");
        }
    }

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
