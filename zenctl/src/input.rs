//! Where zenctl reads a body from (#210).
//!
//! Seven arguments name bytes — `topic pub`'s payload and attachment, `get`'s
//! body, `service call`'s body and attachment, `serve`'s reply, `schema
//! check`'s payload — and each one accepted the same three spellings: `-` for
//! stdin, `@path` for a file, anything else for itself.
//!
//! They were seven copies. Four were byte-identical; three had dropped the `-`
//! arm because they take an `Option`, so `zenctl get --body -` read stdin while
//! `zenctl service call --body -` published a literal one-byte `-`. Nothing
//! documented the difference, because nobody chose it: it is what `match` does
//! when an arm is missing.
//!
//! ## `-` means stdin, at all seven
//!
//! The four that accept it are the four whose help text advertises it, and a
//! one-byte `-` is not a payload anyone means. Where it genuinely is,
//! `printf - | zenctl … --body -` says so without needing syntax.
//!
//! ## The error names the flag, and no call site passes one
//!
//! A [`Source`] is built by clap, through [`ValueParserFactory`], which hands
//! the parser the [`clap::Arg`] it came from. So `--body: cannot read @cfg.json`
//! costs nothing at any call site — the label is a fact clap already had and
//! used to discard.
//!
//! [`ValueParserFactory`]: clap::builder::ValueParserFactory

use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};

/// Stdin can be read once, and this records that it was.
///
/// `--body - --attachment -` would otherwise ship an empty attachment without
/// saying so — the second reader finding the stream already at EOF.
static STDIN_CLAIMED: AtomicBool = AtomicBool::new(false);

/// One argument that names bytes: `-`, `@path`, or the text itself.
///
/// Constructed by clap, never by hand, which is how [`Source::read`]'s error
/// knows which flag it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// As clap spells it: `--body`, `--attachment`, `<REPLY>`.
    flag: String,
    spec: String,
}

impl Source {
    /// For tests and for callers that build one by hand.
    pub fn new(flag: impl Into<String>, spec: impl Into<String>) -> Source {
        Source {
            flag: flag.into(),
            spec: spec.into(),
        }
    }

    /// What the user typed — for a verb that echoes the argument back.
    pub fn spec(&self) -> &str {
        &self.spec
    }

    /// The flag this came from, for a caller composing its own message.
    pub fn flag(&self) -> &str {
        &self.flag
    }

    /// The bytes this argument names.
    pub fn read(&self) -> Result<Vec<u8>> {
        self.read_with(&STDIN_CLAIMED)
    }

    /// The same, against a caller-supplied latch — so a test can drive the
    /// stdin claim without touching the process-wide one.
    pub fn read_with(&self, claimed: &AtomicBool) -> Result<Vec<u8>> {
        match Kind::of(&self.spec) {
            Kind::Stdin => {
                if claimed.swap(true, Ordering::Relaxed) {
                    bail!(
                        "{}: `-` would read stdin, and another argument already \
                         consumed it — only one argument may read `-`",
                        self.flag
                    );
                }
                use std::io::Read as _;
                let mut buf = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut buf)
                    .with_context(|| format!("{}: cannot read stdin", self.flag))?;
                Ok(buf)
            }
            // Naming the *interpretation* and not only the path is the part
            // that helps: it is how a user who meant a literal `@` finds out
            // what happened to it.
            Kind::File(path) => std::fs::read(path).with_context(|| {
                format!(
                    "{}: cannot read {} — a leading @ means \"read this path\"",
                    self.flag, path
                )
            }),
            Kind::Literal(text) => Ok(text.as_bytes().to_vec()),
        }
    }
}

/// What a spec names. Split out so the classification is testable on its own,
/// without a filesystem or a stdin.
#[derive(Debug, PartialEq, Eq)]
enum Kind<'a> {
    Stdin,
    File(&'a str),
    Literal(&'a str),
}

impl<'a> Kind<'a> {
    fn of(spec: &'a str) -> Kind<'a> {
        match spec {
            "-" => Kind::Stdin,
            // `@` alone names no path; it is a one-character payload.
            "@" => Kind::Literal(spec),
            s => match s.strip_prefix('@') {
                Some(path) => Kind::File(path),
                None => Kind::Literal(s),
            },
        }
    }
}

/// Clap builds a [`Source`], so the flag name rides along for free.
#[derive(Clone, Debug)]
pub struct SourceParser;

impl clap::builder::ValueParserFactory for Source {
    type Parser = SourceParser;
    fn value_parser() -> SourceParser {
        SourceParser
    }
}

impl clap::builder::TypedValueParser for SourceParser {
    type Value = Source;

    fn parse_ref(
        &self,
        _cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<Source, clap::Error> {
        let flag = match arg {
            Some(a) => match a.get_long() {
                Some(long) => format!("--{long}"),
                // A positional has no flag; clap's own usage line names it in
                // angle brackets, so an error about one should too.
                None => format!("<{}>", a.get_id().as_str().to_uppercase()),
            },
            None => "<input>".to_string(),
        };
        Ok(Source {
            flag,
            spec: value.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The acceptance #210 asks for: a missing `@file` says which argument
    /// asked for it. `std::fs::read(path)?` alone yields "No such file or
    /// directory (os error 2)" — no path, no flag, nothing to act on.
    #[test]
    fn a_missing_at_file_names_the_flag_that_asked_for_it() {
        let e = Source::new("--body", "@/nonexistent/zenctl/test")
            .read()
            .expect_err("a missing file is an error");
        let text = format!("{e:#}");
        assert!(text.contains("--body"), "{text}");
        assert!(text.contains("/nonexistent/zenctl/test"), "{text}");
        assert!(
            text.contains("a leading @ means"),
            "the interpretation is named, so a literal `@` is diagnosable: {text}"
        );
    }

    /// One classification, so `get --body -` and `call --body -` cannot mean
    /// different things again. They did: `-` was stdin at four sites and a
    /// one-byte payload at three.
    #[test]
    fn a_dash_means_stdin_for_every_argument_that_names_bytes() {
        assert_eq!(Kind::of("-"), Kind::Stdin);
        assert_eq!(Kind::of("@p"), Kind::File("p"));
        assert_eq!(Kind::of("@/a/b"), Kind::File("/a/b"));
        // The near-misses, each a literal:
        assert_eq!(Kind::of("@"), Kind::Literal("@"), "`@` alone names no path");
        assert_eq!(Kind::of("--"), Kind::Literal("--"));
        assert_eq!(Kind::of("-x"), Kind::Literal("-x"));
        assert_eq!(Kind::of("x"), Kind::Literal("x"));
        assert_eq!(Kind::of(""), Kind::Literal(""));
    }

    /// Two arguments reading `-` is an error, not a silently empty second
    /// payload. The second reader would find the stream at EOF and ship
    /// nothing, which is the failure this codebase files as a defect.
    #[test]
    fn only_one_argument_may_read_stdin() {
        let latch = AtomicBool::new(false);
        // The first claim is allowed to proceed; it will read a real (empty
        // or not) stdin under the test harness, and either outcome is fine.
        let _ = Source::new("--body", "-").read_with(&latch);
        let e = Source::new("--attachment", "-")
            .read_with(&latch)
            .expect_err("the second claim is refused");
        let text = format!("{e:#}");
        assert!(text.contains("--attachment"), "{text}");
        assert!(text.contains("only one argument may read"), "{text}");
    }

    /// A literal payload is handed over unchanged, including one that looks
    /// like a flag.
    #[test]
    fn a_literal_payload_is_its_own_bytes() {
        assert_eq!(
            Source::new("--body", r#"{"v":1}"#).read().unwrap(),
            br#"{"v":1}"#
        );
    }
}
