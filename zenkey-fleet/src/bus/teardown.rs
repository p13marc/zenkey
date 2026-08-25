//! The teardown shape the crate shares: **drain everything, then report**.
//!
//! [`crate::Monitor::shutdown`] states the rule — "every watch is drained even
//! if one fails to undeclare … and the failures are reported together" —
//! because a set half torn down is worse than one torn down noisily. The other
//! teardowns wrote the loop themselves and bailed on the first failure with a
//! `?`, leaving everything after it declared (#327, #346).
//!
//! This is that loop, once, for anything with an `undeclare`.

use std::future::Future;

use crate::{Error, Result};

/// Undeclare every item, then report what failed — all of it, in one error
/// naming each item by the label it was drained under.
///
/// The label is what a reader needs to find the thing that would not go away:
/// a key, a selector, a querier's role. Nothing is skipped for an earlier
/// failure.
pub(crate) async fn drain_undeclare<T, F, Fut>(items: Vec<(String, T)>, undeclare: F) -> Result<()>
where
    F: Fn(T) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut failed = Vec::new();
    for (label, item) in items {
        if let Err(e) = undeclare(item).await {
            // The whole chain: `Display` alone names the operation, and the
            // per-handle reason is what a teardown report is for (#348).
            failed.push(format!("{label}: {}", crate::one_line(&e)));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::bus(
            "undeclare",
            failed.join("; "),
            "one or more handles refused",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The property the `?`-in-a-loop shape did not have: a failure in the
    /// middle stops nothing, and the report names every one of them.
    #[tokio::test]
    async fn a_failure_stops_nothing_and_every_failure_is_reported() {
        let visited = AtomicUsize::new(0);
        let err = drain_undeclare(
            vec![
                ("first".to_string(), Ok(())),
                ("second".to_string(), Err("busy")),
                ("third".to_string(), Ok(())),
                ("fourth".to_string(), Err("gone")),
            ],
            |outcome: std::result::Result<(), &str>| {
                visited.fetch_add(1, Ordering::Relaxed);
                async move { outcome.map_err(|e| Error::bus("undeclare", "handle", e)) }
            },
        )
        .await
        .expect_err("two of the four would not undeclare")
        .to_string();

        assert_eq!(visited.load(Ordering::Relaxed), 4, "every item was drained");
        // Each failure is named *and* carries its reason. The exact join is
        // not the property; both halves being present is (#348 moved the
        // reason from `Display` into `source`, so this reads the chain).
        for (label, reason) in [("second", "busy"), ("fourth", "gone")] {
            assert!(err.contains(label), "{label} missing from: {err}");
            assert!(err.contains(reason), "{reason} missing from: {err}");
        }
    }

    /// A clean teardown says nothing.
    #[tokio::test]
    async fn everything_undeclared_is_silent() {
        let all_fine = drain_undeclare(vec![("only".to_string(), ())], |()| async { Ok(()) }).await;
        assert!(all_fine.is_ok());
    }
}
