//! Ownership and serving (spec §6): the split-brain diagnosis, a tool's.
//!
//! - **The finding:** two instances of one service holding an interface
//!   token for the same interface, for longer than a grace period.
//! - **The grace period** is the tool's setting. It SHOULD exceed the
//!   longest re-mint overlap the deployment allows (§8.1), which an owner
//!   keeps below one second: a re-mint is the same shape, for a moment.
//! - **A standby** holds its instance token only, so it is never part of
//!   one.
//! - **The runtime fences nothing.** A finding is reported and nothing is
//!   undeclared: election and fencing are `redundancy.v1`'s (#613).
//! - **Not covered:** the tokenless set (§8.1) holds no interface token.
//!
//! The check samples presence twice, `grace` apart, each with an unbounded
//! liveliness GET (§8.1). A service and interface held by two or more
//! instances in both samples is a finding: a re-mint's overlap is over by
//! the second sample, and a standby is in neither.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};

use crate::error::Result;

/// Two or more instances of one service holding one interface's token, in
/// both samples (§6).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SplitBrain {
    pub service: Addr,
    pub iface: IfaceId,
    /// The instances holding it at the second sample, sorted.
    pub instances: Vec<InstanceId>,
}

/// The holders of each interface token in a set of tokens.
fn holders(tokens: &[ZkKey]) -> BTreeMap<(Addr, IfaceId), BTreeSet<InstanceId>> {
    let mut out: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for t in tokens {
        if let ZkKey::Alive {
            addr,
            iface,
            instance,
            ..
        } = t
        {
            out.entry((addr.clone(), iface.clone()))
                .or_default()
                .insert(instance.clone());
        }
    }
    out
}

/// The findings between two samples of presence taken a grace period
/// apart: every (service, interface) held by two or more instances in
/// both. For a tool that samples presence itself.
#[must_use]
pub fn compare(before: &[ZkKey], after: &[ZkKey]) -> Vec<SplitBrain> {
    let before = holders(before);
    holders(after)
        .into_iter()
        .filter(|(k, now)| now.len() > 1 && before.get(k).is_some_and(|then| then.len() > 1))
        .map(|((service, iface), now)| SplitBrain {
            service,
            iface,
            instances: now.into_iter().collect(),
        })
        .collect()
}

/// Diagnoses split-brains among the interface tokens matching `selector`
/// (`zk2/*/*/@zk/alive/**` for the whole bus): two samples `grace` apart,
/// each a liveliness GET waiting at most `timeout`. It takes `grace` and
/// fences nothing.
pub async fn split_brain(
    session: &zenoh::Session,
    selector: &str,
    grace: Duration,
    timeout: Duration,
) -> Result<Vec<SplitBrain>> {
    let before = crate::presence::tokens(session, selector, timeout).await?;
    tokio::time::sleep(grace).await;
    let after = crate::presence::tokens(session, selector, timeout).await?;
    Ok(compare(&before, &after))
}

#[cfg(test)]
mod tests {
    use super::compare;
    use zenkey_model::grammar::{ZkKey, parse};

    fn alive(service: &str, instance: &str) -> ZkKey {
        parse(&format!(
            "zk2/h1/{service}/@zk/alive/tc.v1/{instance}/0123456789abcdef"
        ))
        .unwrap()
    }

    #[test]
    fn two_holders_in_both_samples_is_a_finding() {
        let (a, b, c) = ("000000000000000a", "000000000000000b", "000000000000000c");
        // A split-brain, even with one side re-minted in between.
        let found = compare(
            &[alive("tc", a), alive("tc", b)],
            &[alive("tc", a), alive("tc", c)],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].service.to_string(), "h1/tc");
        assert_eq!(found[0].instances.len(), 2);
        // A re-mint's overlap, caught by the first sample only.
        assert!(compare(&[alive("tc", a), alive("tc", b)], &[alive("tc", b)]).is_empty());
        // Two services, one instance each.
        assert!(
            compare(
                &[alive("tc", a), alive("tc2", b)],
                &[alive("tc", a), alive("tc2", b)]
            )
            .is_empty()
        );
    }
}
