//! What a service says when it fails (#360).
//!
//! Every service used to land `Result<T, String>`, and the string was made at
//! the point of failure by `map_err(|e| e.to_string())` — which keeps the
//! outermost sentence and drops the chain under it. `services::sweep::admin`
//! was the sharpest case: five distinct fleet queries, five `?`s, one
//! `Result<_, String>`, and a rendered message that could not say which of the
//! five had failed.
//!
//! An iced message must be `Clone`, which is why `anyhow::Error` cannot be
//! carried directly. `Arc<anyhow::Error>` can, and it keeps the chain — so
//! the pane renders the whole thing and `tracing` can still walk it.

use std::fmt;
use std::sync::Arc;

/// The result every service task lands.
pub type ServiceResult<T> = Result<T, ServiceError>;

/// A service failure, with its cause chain intact and cheap to clone.
#[derive(Debug, Clone)]
pub struct ServiceError(Arc<anyhow::Error>);

impl ServiceError {
    /// A failure this app is stating in its own words, with no lower cause —
    /// a precondition the caller checked, not something the bus reported.
    pub fn msg(what: impl fmt::Display) -> ServiceError {
        ServiceError(Arc::new(anyhow::Error::msg(what.to_string())))
    }

    /// The whole chain on one line, `outer: inner: innermost`.
    ///
    /// The convention `zenkey_fleet::Error` follows — Display says *what*
    /// failed and `source()` says *why* — only pays off if something reads
    /// the sources. This is the reader.
    pub fn one_line(&self) -> String {
        let mut s = self.0.to_string();
        for cause in self.0.chain().skip(1) {
            s.push_str(": ");
            s.push_str(&cause.to_string());
        }
        s
    }

    /// The `map_err` every service uses, in place of `|e| e.to_string()`.
    ///
    /// Generic over `Into<anyhow::Error>` rather than a `From` impl: a blanket
    /// `From<E: Into<anyhow::Error>>` overlaps the reflexive
    /// `From<ServiceError> for ServiceError`, and bounding it on
    /// `std::error::Error` overlaps `From<anyhow::Error>` — the compiler
    /// cannot assume anyhow will never implement `Error`. An inherent
    /// function has no such problem and reads the same at the call site.
    pub fn of<E: Into<anyhow::Error>>(e: E) -> ServiceError {
        ServiceError(Arc::new(e.into()))
    }

    /// The same failure, restated as something larger having failed.
    ///
    /// The wrapper the callers used to write as
    /// `format!("could not connect: {e}")` — which read the same and threw
    /// the chain away.
    pub fn context(&self, what: impl fmt::Display) -> ServiceError {
        ServiceError::msg(format!("{what}: {}", self.one_line()))
    }

    /// The underlying error, for a caller that wants to downcast.
    pub fn inner(&self) -> &anyhow::Error {
        &self.0
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.one_line())
    }
}

/// A message this app wrote itself — the `ok_or_else(|| format!(…))?` shape.
/// There is no chain under it, and that is honest: nothing failed *beneath*
/// a precondition this code checked.
impl From<String> for ServiceError {
    fn from(s: String) -> ServiceError {
        ServiceError::msg(s)
    }
}

impl From<&str> for ServiceError {
    fn from(s: &str) -> ServiceError {
        ServiceError::msg(s)
    }
}

impl From<anyhow::Error> for ServiceError {
    fn from(e: anyhow::Error) -> ServiceError {
        ServiceError(Arc::new(e))
    }
}

/// Two errors are the same failure if they read the same. Enough for the
/// `PartialEq` the view states derive, and it is what the string they used to
/// carry compared as.
impl PartialEq for ServiceError {
    fn eq(&self, other: &ServiceError) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.one_line() == other.one_line()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Inner;
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the router refused the connection")
        }
    }
    impl std::error::Error for Inner {}

    /// The whole point: `to_string()` on the source kept one sentence, and
    /// the sentence under it was the one that said what actually happened.
    #[test]
    fn the_chain_survives_the_crossing() {
        let e =
            ServiceError::of(anyhow::Error::new(Inner).context("failed to open the Zenoh session"));
        assert_eq!(
            e.to_string(),
            "failed to open the Zenoh session: the router refused the connection"
        );
    }

    /// `of` is what a `map_err` reaches for, and it takes a plain std error.
    #[test]
    fn of_takes_an_ordinary_error() {
        assert_eq!(
            ServiceError::of(Inner).to_string(),
            "the router refused the connection"
        );
    }

    #[test]
    fn a_plain_message_has_no_chain_to_lose() {
        let e = ServiceError::msg("no context named 'lab'");
        assert_eq!(e.to_string(), "no context named 'lab'");
    }

    /// Cloned into N messages and N pane states, so it must be cheap and
    /// must compare equal to itself.
    #[test]
    fn it_clones_by_pointer_and_compares_as_itself() {
        let e = ServiceError::msg("the same failure");
        let c = e.clone();
        assert!(Arc::ptr_eq(&e.0, &c.0));
        assert_eq!(e, c);
        assert_eq!(e, ServiceError::msg("the same failure"));
        assert_ne!(e, ServiceError::msg("a different failure"));
    }
}
