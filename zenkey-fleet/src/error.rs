//! The engine's failure surface — one type, classified by what a caller can
//! do about it.
//!
//! Every public function in this crate returned `anyhow::Result` until #348.
//! For a binary that is fine; for a **published library** it means a consumer
//! cannot tell a bad key expression from a dead bus without matching on the
//! text of a sentence, and the one consumer that most needs to tell them
//! apart is `zenctl`, whose exit codes are a wire surface CI branches on.
//!
//! ## The classification is the point
//!
//! RFC 13 §1 draws the line this enum is built around: **a question that
//! could not be *put* is not the same as a question that was put and
//! answered badly.** [`Error::Unaskable`] is the first; everything else is a
//! failure of an attempt that was actually made.
//!
//! That distinction had a real cost while it was untyped. `zenctl registry
//! lint /nonexistent` exited **1** — "asked, and the answer is a finding" —
//! telling CI that a registry had lint findings when in fact the directory
//! was not there. It exited 1 because `exit::code_for` walks the error chain
//! looking for `zenctl`'s own `Unaskable` marker, and an engine error carried
//! no marker at all. The engine knows perfectly well which of its failures
//! are refusals of the caller's input; it just had nowhere to say so.
//!
//! ## What each variant means
//!
//! | Variant | The caller should | `zenctl` exits |
//! |---|---|---|
//! | [`Unaskable`](Error::Unaskable) | fix the input | 2 — no verdict |
//! | [`Bus`](Error::Bus) | retry, or check the fleet | 1, or 2 on a verdict verb |
//! | [`Io`](Error::Io) | check the path | 1, or 2 on a verdict verb |
//! | [`Malformed`](Error::Malformed) | distrust the peer | 1 |
//! | [`Internal`](Error::Internal) | file a bug against this crate | 1 |
//!
//! ## How these render
//!
//! **`Display` says what failed; `source()` says why**, and a renderer joins
//! them — `zenctl::errors::render` prints `Error: …` then an indented
//! `Caused by:` list, trimming zenoh's build-machine source locations on the
//! way. So no variant's `Display` repeats the text of its own source: doing
//! that once produced `registry dir "/x": No such file` immediately followed
//! by `Caused by: No such file`, which the CLI corpus caught.
//!
//! The exception is deliberate. [`Unaskable`](Error::Unaskable) inlines its
//! cause's text and keeps no `source`, because those causes are one-line
//! refusals ("`*` may only be preceded by `/`") whose entire content *is*
//! the sentence — there is nothing structured under them to reach for.

use std::path::PathBuf;

/// A cause from somewhere else, kept rather than flattened to text.
///
/// Boxed because the causes are genuinely unrelated types — `zenoh::Error`
/// (itself a box), `std::io::Error`, `serde_json::Error`, a `JoinError`.
pub type BoxedCause = Box<dyn std::error::Error + Send + Sync>;

/// [`Error::Io`]'s Display, kept total (see the variant's own doc).
fn path_or_local_io(path: &std::path::Path) -> String {
    match path.as_os_str().is_empty() {
        true => "local I/O".to_string(),
        false => path.display().to_string(),
    }
}

/// Why a fleet operation could not answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// **The caller's input was refused; nothing was attempted.** A selector
    /// that is not a key expression, a name outside a closed vocabulary, a
    /// window of zero seconds, a hostname where an origin belongs.
    ///
    /// This is the variant that carries RFC 13 §1's "the question could not
    /// be asked", and the only one a caller fixes by changing what it passed.
    #[error("{what}: {detail}")]
    Unaskable {
        /// What was refused, as the caller named it — a selector, a flag
        /// value, an id.
        what: String,
        /// Why. This crate's own words, or a one-line parse refusal inlined
        /// from whatever produced it (see the module doc on rendering).
        detail: String,
    },

    /// A bus operation failed: a declare, a get, a put, an undeclare.
    ///
    /// The question was asked and the fabric did not carry it. Distinct from
    /// [`Unaskable`](Error::Unaskable) because retrying is meaningful.
    #[error("{op} {target}")]
    Bus {
        /// The operation, phrased so that `{op} {target}` reads as the thing
        /// that failed — `subscribe v1/**`, `declare queryable …`, `failed to
        /// open the Zenoh session`. It is a label, not a zenoh API name.
        op: &'static str,
        /// What it was against — a key expression, a selector, an origin.
        target: String,
        #[source]
        source: BoxedCause,
    },

    /// Local filesystem I/O — a `.zrec`, a registry directory, a config.
    ///
    /// Carries the path where there is one, and the `io::ErrorKind` is
    /// reachable through [`source`](std::error::Error::source): "no such
    /// directory" and "permission denied" are different answers to give a
    /// user.
    ///
    /// The path may be **empty**, and the Display says so rather than
    /// rendering to nothing: a writer generic over `W: Write` (the `.zrec`
    /// sink) genuinely does not know where its bytes are going. An error
    /// whose `Display` is the empty string is not a smaller error — it is a
    /// blank `Error:` line and a blank `Caused by:` under it.
    #[error("{}", path_or_local_io(path))]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Bytes arrived and could not be read as what they claim to be — a
    /// `.zrec` header, a served slice, a reply payload.
    ///
    /// Not the caller's fault and not the fabric's: a peer said something
    /// this build cannot parse.
    #[error("{what}: {detail}")]
    Malformed {
        what: String,
        /// This crate's own words — short, because the parser's own message
        /// rides underneath as the [`source`](std::error::Error::source).
        detail: String,
        #[source]
        source: Option<BoxedCause>,
    },

    /// An invariant of *this crate* did not hold.
    ///
    /// Every construction site is a place the old code said "a bug in
    /// `blob_*`" or "tasks outlived the run". Kept as a variant rather than a
    /// panic because an explorer that has found a bug in its own engine
    /// should still be able to report it rather than die mid-sweep.
    #[error("internal: {0} — please report this against zenkey-fleet")]
    Internal(String),
}

/// This crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// The caller's input was refused and nothing was attempted — RFC 13
    /// §1's "the question could not be asked".
    ///
    /// `zenctl` maps this to its reserved exit 2. It is a method rather than
    /// a `matches!` at the call site so the mapping has one home, the way
    /// [`judgement_exit_code`](crate::judgement_exit_code) does for verdicts.
    pub fn is_unaskable(&self) -> bool {
        matches!(self, Error::Unaskable { .. })
    }

    /// An input this crate refuses.
    pub fn unaskable(what: impl Into<String>, detail: impl Into<String>) -> Error {
        Error::Unaskable {
            what: what.into(),
            detail: detail.into(),
        }
    }

    /// An input this crate refuses, explained by the error that refused it —
    /// a key-expression parse, a number that would not parse.
    pub fn unaskable_from(what: impl Into<String>, cause: impl Into<BoxedCause>) -> Error {
        Error::Unaskable {
            what: what.into(),
            detail: cause.into().to_string(),
        }
    }

    /// A bus operation that failed.
    pub fn bus(op: &'static str, target: impl Into<String>, cause: impl Into<BoxedCause>) -> Error {
        Error::Bus {
            op,
            target: target.into(),
            source: cause.into(),
        }
    }

    /// Bytes that did not read as what they claimed to be.
    pub fn malformed(what: impl Into<String>, detail: impl Into<String>) -> Error {
        Error::Malformed {
            what: what.into(),
            detail: detail.into(),
            source: None,
        }
    }

    /// Likewise, keeping the parser's own error underneath — where a span or
    /// a line number is worth reaching for.
    pub fn malformed_from(what: impl Into<String>, cause: impl Into<BoxedCause>) -> Error {
        Error::malformed_with(what, "does not parse", cause)
    }

    /// [`malformed_from`](Error::malformed_from) where the caller has a better
    /// sentence than "does not parse" — `is not a header`, `is not a slice`.
    pub fn malformed_with(
        what: impl Into<String>,
        detail: impl Into<String>,
        cause: impl Into<BoxedCause>,
    ) -> Error {
        Error::Malformed {
            what: what.into(),
            detail: detail.into(),
            source: Some(cause.into()),
        }
    }

    /// Local I/O against a named path.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

/// A served slice that does not parse is a *peer* saying something
/// unreadable, never a local mistake — RFC 08 §6's introspect reply.
impl From<zenkey::slice::SliceError> for Error {
    fn from(e: zenkey::slice::SliceError) -> Error {
        Error::malformed_from("registry slice", e)
    }
}

/// A key this crate built or was handed does not parse.
impl From<zenkey::KeyError> for Error {
    fn from(e: zenkey::KeyError) -> Error {
        Error::unaskable_from("key", e)
    }
}

/// This error and every cause beneath it, joined by `: ` — one line.
///
/// The counterpart to the rendering convention in the module doc: `Display`
/// deliberately says only *what* failed, so anywhere that needs the whole
/// story on one line (a log line, a degradation note, a per-item teardown
/// report) asks for it here rather than reaching for `{:#}` — which does
/// nothing on a `thiserror` type and silently drops the cause.
pub fn one_line(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let text = c.to_string();
        // A cause whose text the parent already contains adds nothing.
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        cause = c.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classification is the contract (#348): `zenctl` maps
    /// [`Error::is_unaskable`] onto its reserved exit 2, so which variants
    /// answer yes is a wire-surface decision, not an implementation detail.
    #[test]
    fn only_a_refused_input_is_unaskable() {
        assert!(Error::unaskable("v1/$*/**", "is not a key expression").is_unaskable());
        assert!(Error::unaskable_from("--old-root", "bad expr").is_unaskable());

        // Everything else was *attempted*. Reporting these as "could not ask"
        // would excuse a real failure; reporting the one above as a finding
        // claims a verdict on a question nobody put.
        assert!(!Error::bus("subscribe", "v1/**", "no route").is_unaskable());
        assert!(!Error::io("/tmp/x", std::io::Error::other("nope")).is_unaskable());
        assert!(!Error::malformed(".zrec", "is not a header").is_unaskable());
        assert!(!Error::Internal("a bug".into()).is_unaskable());
    }

    /// `Display` says what failed; `source` says why; `one_line` joins them.
    ///
    /// No variant's `Display` may be empty. The `.zrec` writer is generic
    /// over `W: Write` and has no path to name, so it builds `Error::Io`
    /// with `PathBuf::new()` — which rendered to the empty string, and
    /// surfaced a disk-full mid-capture as a blank `Error:` with a blank
    /// `Caused by:` under it.
    #[test]
    fn no_display_is_blank() {
        let cases = [
            Error::io(
                std::path::PathBuf::new(),
                std::io::Error::other("disk full"),
            ),
            Error::io("/tmp/x", std::io::Error::other("disk full")),
            Error::bus("subscribe", "v1/**", "no route"),
            Error::unaskable_from("v1/$*/**", "`*` may only follow `/`"),
            Error::Internal("something".into()),
        ];
        for e in &cases {
            assert!(!e.to_string().is_empty(), "blank Display: {e:?}");
            assert!(!one_line(e).starts_with(':'), "blank head: {}", one_line(e));
        }
        assert_eq!(
            one_line(&Error::io(
                std::path::PathBuf::new(),
                std::io::Error::other("disk full")
            )),
            "local I/O: disk full"
        );
    }

    /// No variant's `Display` may repeat its own source's text — doing that
    /// once produced `registry dir "/x": No such file` followed immediately
    /// by `Caused by: No such file`, which the CLI corpus caught.
    #[test]
    fn display_never_repeats_its_own_source() {
        use std::error::Error as _;

        let io = Error::io("/tmp/x", std::io::Error::other("no such thing"));
        assert_eq!(io.to_string(), "/tmp/x");
        assert_eq!(io.source().unwrap().to_string(), "no such thing");
        assert_eq!(one_line(&io), "/tmp/x: no such thing");

        let bus = Error::bus("subscribe", "v1/**", "no route to host");
        assert_eq!(bus.to_string(), "subscribe v1/**");
        assert_eq!(one_line(&bus), "subscribe v1/**: no route to host");

        // An `Unaskable` inlines its cause and keeps none, so there is
        // nothing for `one_line` to add.
        let refused = Error::unaskable_from("v1/$*/**", "`*` may only follow `/`");
        assert!(refused.source().is_none());
        assert_eq!(one_line(&refused), refused.to_string());
    }

    /// A slice that does not parse is the *peer* being unreadable — never the
    /// caller's input, and never the fabric.
    #[test]
    fn a_bad_slice_is_malformed_not_unaskable() {
        let e: Error = zenkey::parse_slice("this is not = = toml")
            .unwrap_err()
            .into();
        assert!(matches!(e, Error::Malformed { .. }));
        assert!(!e.is_unaskable());
    }
}
