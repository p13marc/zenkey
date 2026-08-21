//! The resolved bus arguments — one context read, once, fallible (#209).
//!
//! [`crate::cli::BusArgs`] is what clap parses: flags, verbatim. [`Bus`] is
//! what a command actually runs against, with every ladder in
//! [`crate::resolve`] already climbed. The conversion is the tool's **single
//! impure edge**: it reads `~/.config`, once, at a moment the caller chose.
//!
//! ## Why not memoise instead
//!
//! It used to. `BusArgs::stored()` cached the active context in a
//! `static OnceLock` and called `std::process::exit(2)` when the name was bad,
//! and both halves were the same mistake in different clothes — the cache
//! because nothing in the type system said one resolution, the `exit` because
//! a getter that can fail has nowhere to put the failure. The engine had
//! already written the rule down: `zenkey_fleet::context_store` opens with
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
//! *One user-visible change:* a bad `--context` exits **1**, not 2. Nothing
//! pinned the 2, and 1 is the consistent answer — in this tool exit 2 means
//! *not proven* (`expect`, `cutover`, `schema check`), while a name that is
//! not in your config file is your input, which is what
//! `session-config-error.trycmd` already pins as 1.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use zenkey::RegistrySlice;
use zenkey_fleet::context_store::StoredContext;

use crate::cli::{BusArgs, OutputArgs};
use crate::resolve;

/// Everything a command needs from the bus flags, resolved.
#[derive(Clone)]
pub(crate) struct Bus {
    base: String,
    /// The `--context` **name** as given, which is not the resolved context:
    /// the completion cache is keyed by it, and `zenctl cache` has to key it
    /// the same way (#197).
    context: Option<String>,
    registry: Vec<PathBuf>,
    transport: resolve::Transport,
    timeout: Duration,
    out: OutputArgs,
}

impl Bus {
    /// Resolve against the user's config file. The impure edge.
    pub(crate) fn resolve(args: &BusArgs) -> Result<Bus> {
        let stored = crate::context::active(args.context.as_deref())?;
        Ok(Bus::resolve_with(args, stored.as_ref()))
    }

    /// The same, against a caller-supplied context — pure, and the reason a
    /// test never has to touch the developer's real `~/.config`.
    ///
    /// Modelled on `zengui/src/config.rs`'s `settings_with()`, which split for
    /// the same reason and got there first.
    pub(crate) fn resolve_with(args: &BusArgs, stored: Option<&StoredContext>) -> Bus {
        Bus {
            base: resolve::base(args.base.as_deref(), stored).to_string(),
            context: args.context.clone(),
            registry: resolve::registry_dirs(&args.registry, stored),
            transport: resolve::transport(
                args.zenoh_config.as_deref(),
                &args.connect,
                &args.listen,
                args.scouting,
                stored,
            ),
            timeout: resolve::timeout(args.timeout, stored),
            out: args.out,
        }
    }

    /// The chosen format.
    pub(crate) fn format(&self) -> crate::render::Format {
        self.out.format
    }

    /// The chosen colour policy.
    pub(crate) fn color(&self) -> crate::render::ColorChoice {
        self.out.color
    }

    /// The deployment base — empty means the bus root, which is a deployment.
    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// The `--context` name this invocation was given, if any.
    pub(crate) fn context_name(&self) -> Option<&str> {
        self.context.as_deref()
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) fn registry_dirs(&self) -> Vec<PathBuf> {
        self.registry.clone()
    }

    /// Compose a base-relative key into the full wire key this un-namespaced
    /// tool must actually use.
    pub(crate) fn wire(&self, relative: impl AsRef<str>) -> Result<String> {
        Ok(zenkey::grammar::with_base(self.base(), relative))
    }

    pub(crate) async fn session(&self) -> Result<zenoh::Session> {
        self.session_reporting()
            .await
            .map_err(zenkey_fleet::OpenFailure::into_error)
    }

    /// The same, saying which half failed — so a caller holding `--registry`
    /// dirs can tell "the transport would not come up" (answerable from disk)
    /// from "the config file you named does not parse" (yours to fix, #196).
    pub(crate) async fn session_reporting(
        &self,
    ) -> Result<zenoh::Session, zenkey_fleet::OpenFailure> {
        let t = &self.transport;
        zenkey_fleet::open_reporting(t.file.as_deref(), &t.connect, &t.listen, t.scouting).await
    }

    /// Registry slices from whichever source the flags select: local
    /// `--registry` dirs when given (offline), otherwise the live bus
    /// (RFC 08 §6 introspection). Both yield the same `Vec<RegistrySlice>`,
    /// so every renderer is source-agnostic.
    pub(crate) async fn slices(&self) -> Result<Vec<RegistrySlice>> {
        Ok(self.slice_set().await?.slices().to_vec())
    }

    /// The same, as the fleet engine's indexed set (echo's decode path).
    pub(crate) async fn slice_set(&self) -> Result<zenkey_fleet::SliceSet> {
        let base = self.base();
        let dirs = match resolve::slice_source(self.registry_dirs()) {
            resolve::SliceSource::Bus => {
                let session = self.session().await?;
                let set = zenkey_fleet::SliceSet::from_bus(&session, base, self.timeout()).await?;
                if set.slices().is_empty() {
                    eprintln!("{}", resolve::notes::no_slices(base).to_line());
                }
                self.cache(&set);
                return Ok(set);
            }
            resolve::SliceSource::Union(dirs) => dirs,
        };
        // The README promises this works when the fleet is down, and it
        // mostly did: zenoh opens a session against an unreachable endpoint,
        // so the union simply falls back to the dirs. What it could not
        // survive was a transport that would not come up at all — a taken
        // listener port, say — which failed a question the dirs could answer
        // on their own (#196).
        let session = match self.session_reporting().await {
            Ok(s) => s,
            Err(zenkey_fleet::OpenFailure::Config(e)) => return Err(e),
            Err(zenkey_fleet::OpenFailure::Transport(e)) => {
                let set = zenkey_fleet::SliceSet::from_dirs(&dirs)?;
                eprintln!(
                    "{}",
                    resolve::notes::registry_only(&e.to_string(), &dirs).to_line()
                );
                self.cache(&set);
                return Ok(set);
            }
        };
        // §6.1's decision, delivered by issue #43: --registry and the bus stop
        // being exclusive. Union: served wins per producer, dirs fill the
        // gaps, disagreement is reported — never silently overwritten.
        let out = zenkey_fleet::SliceSet::from_union(&session, base, &dirs, self.timeout()).await?;
        for d in &out.disagreements {
            eprintln!(
                "{}",
                resolve::notes::disagreement(
                    &d.producer,
                    &d.bus_version.to_string(),
                    &d.dirs_version.to_string(),
                    d.shape_differs,
                )
                .to_line()
            );
        }
        self.cache(&out.set);
        Ok(out.set)
    }

    /// Persist the slice set for shell completion (issue #54).
    ///
    /// Best-effort by design: a cache that cannot be written must not fail the
    /// command the user actually ran, and an unwritable cache dir is a
    /// completion that stays static — not an outage. An *empty* set is never
    /// written, because overwriting a good cache with a sweep that found
    /// nothing would turn one bad moment on the bus into a permanently blank
    /// completion.
    pub(crate) fn cache(&self, set: &zenkey_fleet::SliceSet) {
        if set.slices().is_empty() {
            return;
        }
        // Through the accessor, not the field: the reader
        // (`completion::cached`) and `zenctl cache` both resolve the name the
        // same way, and three spellings of one rule is how they drifted
        // (#197).
        let dir =
            zenkey_fleet::cache_dir(zenkey_fleet::active_name(self.context_name()).as_deref());
        // Nothing is logged on failure: this runs on every command, and a
        // warning about a cache the user did not ask for would be noise on
        // the output they did.
        let _ = set.write_cache(&dir);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `BusArgs` as clap would hand it over with nothing but `--base` given.
    pub(crate) fn args(base: Option<&str>) -> BusArgs {
        BusArgs {
            base: base.map(str::to_string),
            context: None,
            registry: vec![],
            connect: vec![],
            listen: vec![],
            scouting: false,
            timeout: None,
            zenoh_config: None,
            out: OutputArgs {
                format: crate::render::Format::Table,
                color: crate::render::ColorChoice::Never,
            },
        }
    }

    /// A `Bus` a test can build with no config file, no bus and no `exit`.
    ///
    /// This is the whole point of the split: before it, the crate's only unit
    /// test of a bus accessor passed *because it pinned `base` and so never
    /// let the ladder reach its second rung*. `resolve_with(_, None)` is that
    /// second rung, made reachable and empty.
    pub(crate) fn bus_of(base: Option<&str>) -> Bus {
        Bus::resolve_with(&args(base), None)
    }

    #[test]
    fn a_bus_with_no_context_falls_all_the_way_to_the_defaults() {
        let b = bus_of(None);
        assert_eq!(
            b.base(),
            "",
            "no flag and no context: the base-less deployment, not a failure"
        );
        assert_eq!(b.timeout(), Duration::from_secs(resolve::DEFAULT_TIMEOUT_S));
        assert!(b.registry_dirs().is_empty());
        assert_eq!(b.context_name(), None);
        assert_eq!(bus_of(Some("zs")).base(), "zs");
    }

    /// The `--context` **name** survives resolution, because the completion
    /// cache is keyed by it and `zenctl cache` has to agree (#197). Resolving
    /// the context into settings and dropping the name is exactly how those
    /// two spellings drifted apart.
    #[test]
    fn the_context_name_outlives_the_context() {
        let mut a = args(None);
        a.context = Some("lab".into());
        let b = Bus::resolve_with(&a, None);
        assert_eq!(b.context_name(), Some("lab"));
    }
}
