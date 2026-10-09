//! The resolved bus arguments — one context read, once, fallible (#209).
//!
//! [`crate::cli::SessionArgs`] and [`crate::cli::NamespaceArgs`] are what
//! clap parses: flags, verbatim. [`Link`] and [`Deployment`] are what a
//! command actually runs against, with every ladder in [`crate::resolve`]
//! already climbed. The conversion is the tool's **single impure edge**: it
//! reads `~/.config`, once, at a moment the caller chose.
//!
//! v1's `Bus` — a deployment base, `--registry` dirs, and the slice ladder
//! that loaded a registry from them or from the live bus — left with the v1
//! registry (#612, FJ9).
//!
//! ## Why not memoise instead
//!
//! It used to. `BusArgs::stored()` cached the active context in a
//! `static OnceLock` and called `std::process::exit(2)` when the name was bad,
//! and both halves were the same mistake in different clothes — the cache
//! because nothing in the type system said one resolution, the `exit` because
//! a getter that can fail has nowhere to put the failure. The engine had
//! already written the rule down: `zenkey_explorer_config` opens with
//! *"Everything here is pure and fallible. No process-global caches, no
//! `exit()` … zenctl turns the `Err` into its own exit code at its own edge."*
//! zengui built its half (`zengui/src/config.rs`) against exactly this
//! objection and named `stored()` in the comment. This is zenctl's half,
//! finally.
//!
//! Resolving eagerly rather than per-accessor is what buys that: a bad
//! `--context` is refused before a command prints anything, instead of at
//! whichever of a hundred accessor calls happened to come first.
//!
//! *One user-visible change, twice reversed:* a bad `--context` used to exit
//! 2 through a `std::process::exit` inside a getter, then 1 through the anyhow
//! edge (#209's reading: "your input is a 1"), and exits **2** again since
//! #307 — this time because the whole tool agrees. Clap already exits 2 for
//! every mis-shaped command line, which fixes the meaning of 2 for user input
//! whether the rest of the tool likes it or not; the refusals clap cannot
//! express joined it rather than competing with it. [`crate::exit`] is the
//! one statement of that, and `crate::context::active` is where this
//! particular refusal is tagged.

use std::time::Duration;

use anyhow::Result;

use zenkey_explorer_config::StoredContext;

use crate::cli::{NamespaceArgs, OutputArgs, SessionArgs};
use crate::resolve;

/// A zk2 verb's connection, resolved (#612, FJ4): `SessionArgs` with every
/// ladder climbed, and no deployment in it.
///
/// The single impure edge: the context file is read once,
/// when this is built. `namespace list` runs on one of these alone,
/// because it looks across namespaces; a resolved verb runs on a
/// [`Deployment`], which carries one.
#[derive(Clone)]
pub(crate) struct Link {
    context: Option<String>,
    transport: resolve::Transport,
    timeout: Duration,
    /// The timeout the user chose, by flag or context; `None` when the
    /// default stands.
    chosen_timeout: Option<Duration>,
    out: OutputArgs,
}

impl Link {
    /// Resolve against the user's config file. The impure edge.
    pub(crate) fn resolve(args: &SessionArgs) -> Result<Link> {
        let stored = crate::context::active(args.context.as_deref())?;
        Ok(Link::resolve_with(args, stored.as_ref()))
    }

    /// The same, against a caller-supplied context.
    pub(crate) fn resolve_with(args: &SessionArgs, stored: Option<&StoredContext>) -> Link {
        Link {
            context: args.context.clone(),
            transport: resolve::transport(
                args.zenoh_config.as_deref(),
                &args.connect,
                &args.listen,
                args.scouting,
                stored,
            ),
            timeout: resolve::timeout(args.timeout, stored),
            chosen_timeout: args
                .timeout
                .or_else(|| stored.and_then(|c| c.timeout))
                .map(Duration::from_secs),
            out: args.out,
        }
    }

    pub(crate) fn format(&self) -> crate::render::Format {
        self.out.format
    }

    pub(crate) fn color(&self) -> crate::render::ColorChoice {
        self.out.color
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The timeout the user chose (`--timeout`, or the context's), `None`
    /// when the default stands: a call then waits what its contract
    /// recommends (spec §5.1, "The timeout").
    pub(crate) fn chosen_timeout(&self) -> Option<Duration> {
        self.chosen_timeout
    }

    /// The `--context` name this invocation was given, if any — what the
    /// completion cache is keyed by.
    pub(crate) fn context_name(&self) -> Option<&str> {
        self.context.as_deref()
    }

    /// An un-namespaced session: the raw half of the FJ decision, for a
    /// read that looks across namespaces.
    pub(crate) async fn session(&self) -> Result<zenoh::Session> {
        let t = &self.transport;
        zenkey_fleet::open_reporting(t.file.as_deref(), &t.connect, &t.listen, t.scouting)
            .await
            .map_err(open_error)
    }

    /// A session **in** `namespace` over this connection: the one act that
    /// writes as a deployment's own participant, `replay --namespace`
    /// (#612, FJ5; spike S13).
    pub(crate) async fn session_in(&self, namespace: &str) -> Result<zenoh::Session> {
        let t = &self.transport;
        zenkey_fleet::open_in_namespace(
            namespace,
            t.file.as_deref(),
            &t.connect,
            &t.listen,
            t.scouting,
        )
        .await
        .map_err(open_error)
    }
}

/// A zk2 resolved verb's bus (#612, FJ4): the deployment's namespace and the
/// connection, resolved.
///
/// The namespace climbs the base's ladder — `--namespace` (alias `--base`,
/// env `ZENCTL_BASE`) > the active context's `base` > empty — because they
/// are one fact: the deployment base *is* the session namespace its
/// services run in. Empty is the bus-root deployment and sets no
/// namespace.
#[derive(Clone)]
pub(crate) struct Deployment {
    namespace: String,
    link: Link,
}

impl Deployment {
    /// Resolve against the user's config file. The impure edge, once.
    pub(crate) fn resolve(args: &NamespaceArgs) -> Result<Deployment> {
        let stored = crate::context::active(args.session.context.as_deref())?;
        Ok(Deployment::resolve_with(args, stored.as_ref()))
    }

    /// The same, against a caller-supplied context.
    pub(crate) fn resolve_with(args: &NamespaceArgs, stored: Option<&StoredContext>) -> Deployment {
        Deployment {
            namespace: resolve::base(args.namespace.as_deref(), stored).to_string(),
            link: Link::resolve_with(&args.session, stored),
        }
    }

    /// The namespace; empty for the bus-root deployment.
    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }

    pub(crate) fn link(&self) -> &Link {
        &self.link
    }

    pub(crate) fn format(&self) -> crate::render::Format {
        self.link.format()
    }

    pub(crate) fn color(&self) -> crate::render::ColorChoice {
        self.link.color()
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.link.timeout()
    }

    pub(crate) fn chosen_timeout(&self) -> Option<Duration> {
        self.link.chosen_timeout()
    }

    /// A session **in** the namespace (decided 2026-10-08): what this
    /// verb reads, it reads as the deployment's own consumers do.
    pub(crate) async fn session(&self) -> Result<zenoh::Session> {
        let t = &self.link.transport;
        zenkey_fleet::open_in_namespace(
            &self.namespace,
            t.file.as_deref(),
            &t.connect,
            &t.listen,
            t.scouting,
        )
        .await
        .map_err(open_error)
    }
}

/// An [`OpenFailure`](zenkey_fleet::OpenFailure), as an error with an exit
/// code attached.
///
/// A `--zenoh-config` this tool refuses — one that sets a session namespace,
/// say, which an explorer must not have (RFC 09 §5) — or an endpoint that is
/// not one (#503) is *your input*, so it is refused and exits 2. A transport
/// that would not come up is the world being unavailable — and since the
/// session is a client (#501) that is what an unreachable router *is*. It
/// exits 2 as well, for its own reason: nothing was asked ([`NoSession`]).
///
/// [`NoSession`]: crate::exit::NoSession
fn open_error(f: zenkey_fleet::OpenFailure) -> anyhow::Error {
    match f {
        // `one_line` defensively: `Config` carries an `Unaskable` today, which
        // has no source, but `{e}` would silently drop one the day it does.
        zenkey_fleet::OpenFailure::Config(e) => {
            crate::exit::unaskable!("{}", zenkey_fleet::one_line(&e))
        }
        zenkey_fleet::OpenFailure::Transport(e) => anyhow::Error::new(crate::exit::NoSession(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FJ4: a resolved verb's namespace climbs the base's ladder — the flag
    /// (which `--base` and `ZENCTL_BASE` both feed), then the context's
    /// `base`, then the bus root — and an empty flag is the bus root, never
    /// a fall-through to the context.
    #[test]
    fn a_namespace_climbs_the_base_s_ladder() {
        let ns = |flag: Option<&str>, stored: Option<&StoredContext>| {
            let args = NamespaceArgs {
                namespace: flag.map(str::to_string),
                session: SessionArgs {
                    context: None,
                    connect: vec![],
                    listen: vec![],
                    scouting: false,
                    timeout: Some(2),
                    zenoh_config: None,
                    out: OutputArgs {
                        format: crate::render::Format::Table,
                        color: crate::render::ColorChoice::Never,
                    },
                },
            };
            let d = Deployment::resolve_with(&args, stored);
            assert_eq!(d.timeout(), Duration::from_secs(2));
            d.namespace().to_owned()
        };
        let ctx = StoredContext {
            base: Some("prod".into()),
            ..StoredContext::default()
        };
        assert_eq!(ns(None, None), "");
        assert_eq!(ns(None, Some(&ctx)), "prod");
        assert_eq!(ns(Some("site/a"), Some(&ctx)), "site/a");
        assert_eq!(ns(Some(""), Some(&ctx)), "", "an empty flag is the root");
    }

    /// The `--context` **name** survives resolution, because the completion
    /// cache is keyed by it and `zenctl cache` has to agree (#197). Resolving
    /// the context into settings and dropping the name is exactly how those
    /// two spellings drifted apart.
    #[test]
    fn the_context_name_outlives_the_context() {
        let args = SessionArgs {
            context: Some("lab".into()),
            connect: vec![],
            listen: vec![],
            scouting: false,
            timeout: None,
            zenoh_config: None,
            out: OutputArgs {
                format: crate::render::Format::Table,
                color: crate::render::ColorChoice::Never,
            },
        };
        let link = Link::resolve_with(&args, None);
        assert_eq!(link.context_name(), Some("lab"));
        assert_eq!(
            link.timeout(),
            Duration::from_secs(resolve::DEFAULT_TIMEOUT_S),
            "no flag and no context: the default, not a failure"
        );
    }
}
