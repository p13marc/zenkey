//! Ownership and serving (spec §6): the split-brain diagnosis, a tool's.
//!
//! - **The finding:** two instances of one service holding an interface
//!   token for the same interface, for longer than a grace period.
//! - **Replicated serving** (0.7, O-12): each replica of a replicated
//!   operation holds its interface's token, so holders of which at most one
//!   exposes an exclusive resource are no finding. It is decided from the
//!   contract and the holders' descriptors (§3.3's exposure), and only for
//!   an interface whose contract declares a replicated operation. Holders
//!   the check cannot decide, because a descriptor or the contract cannot
//!   be read, are **undecided**: neither a finding nor clear, as silence is
//!   never a verdict (O5).
//! - **The grace period** is the tool's setting. It SHOULD exceed the
//!   longest re-mint overlap the deployment allows (§8.1), which an owner
//!   keeps below one second: a re-mint is the same shape, for a moment.
//! - **A standby** holds its instance token only, so it is never part of
//!   one.
//! - **The runtime fences nothing.** A finding is reported and nothing is
//!   undeclared: election and fencing are `redundancy.v1`'s (#613).
//! - **Not covered:** the tokenless set (§8.1) holds no interface token.
//!
//! The check reads presence twice, `grace` apart, each with an unbounded
//! liveliness GET (§8.1). A service and interface held by two or more
//! instances in both reads, not necessarily the same, is a candidate
//! ([`candidates`]): a re-mint's overlap is over by the second read, and a
//! standby is in neither. A candidate is then decided ([`decide`]).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::{Addr, Fp16, IfaceId, InstanceId, ZkKey};

use crate::error::Result;
use crate::operation::is_replicated;
use crate::presence::Found;
use crate::retrieval::Retrieved;

/// Two or more instances of one service holding one interface's token, in
/// both reads (§6): a candidate, and, once decided, a finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SplitBrain {
    pub service: Addr,
    pub iface: IfaceId,
    /// The instances holding it at the second read, sorted.
    pub instances: Vec<InstanceId>,
}

/// Holders the check could not decide (§6, 0.7): a descriptor or the
/// contract it needs could not be read. Neither a finding nor clear (O5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Undecided {
    pub holders: SplitBrain,
    /// What could not be read, one entry per holder or contract.
    pub why: Vec<String>,
}

/// What the check found (§6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnosis {
    /// Two or more holders exposing an exclusive resource: split-brains.
    pub findings: Vec<SplitBrain>,
    /// Holders the check could not decide.
    pub undecided: Vec<Undecided>,
}

impl Diagnosis {
    /// No finding, and nothing undecided.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.findings.is_empty() && self.undecided.is_empty()
    }
}

/// How one candidate is decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// At least two holders expose an exclusive resource.
    Finding,
    /// At most one does: replicated serving explains the rest (§6).
    Clear,
    /// The check cannot tell, and says what it could not read.
    Undecided(Vec<String>),
}

/// The holders of each interface token in a read, each with the revision
/// its token names.
fn holders(tokens: &[ZkKey]) -> BTreeMap<(Addr, IfaceId), BTreeMap<InstanceId, Fp16>> {
    let mut out: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
    for t in tokens {
        if let ZkKey::Alive {
            addr,
            iface,
            instance,
            fp,
        } = t
        {
            out.entry((addr.clone(), iface.clone()))
                .or_default()
                .insert(instance.clone(), fp.clone());
        }
    }
    out
}

/// The candidates between two reads of presence taken a grace period
/// apart: every (service, interface) held by two or more instances in both,
/// from the tokens alone.
#[must_use]
pub fn candidates(before: &[ZkKey], after: &[ZkKey]) -> Vec<SplitBrain> {
    candidates_with_revisions(before, after)
        .into_iter()
        .map(|(c, _)| c)
        .collect()
}

fn candidates_with_revisions(
    before: &[ZkKey],
    after: &[ZkKey],
) -> Vec<(SplitBrain, BTreeMap<InstanceId, Fp16>)> {
    let before = holders(before);
    holders(after)
        .into_iter()
        .filter(|(k, now)| now.len() > 1 && before.get(k).is_some_and(|then| then.len() > 1))
        .map(|((service, iface), now)| {
            (
                SplitBrain {
                    service,
                    iface,
                    instances: now.keys().cloned().collect(),
                },
                now,
            )
        })
        .collect()
}

/// Whether a holder exposes an exclusive resource of its interface, or why
/// that cannot be told.
fn exposes_exclusive(
    holder: &SplitBrain,
    instance: &InstanceId,
    fp16: Option<&Fp16>,
    descriptors: &[Descriptor],
    contracts: &[(Fingerprint, &Contract)],
) -> std::result::Result<bool, String> {
    let iface = holder.iface.to_string();
    let service = holder.service.to_string();
    let descriptor = descriptors
        .iter()
        .find(|d| d.service == service && d.instance == instance.as_str());
    let entry = descriptor.and_then(|d| d.interfaces.iter().find(|e| e.iface == iface));
    // The holder's revision: its descriptor's fingerprint, else its token's
    // fp16 (§8.1).
    let contract = contracts
        .iter()
        .find(|(fp, c)| {
            c.iface == holder.iface
                && match (entry, fp16) {
                    (Some(e), _) => fp.to_string() == e.contract,
                    (None, Some(t)) => fp.hex().fp16() == *t,
                    (None, None) => false,
                }
        })
        .map(|(_, c)| *c);
    let Some(contract) = contract else {
        return Err(format!(
            "instance {instance}: the contract of its revision of {iface} cannot be read"
        ));
    };
    // Without a replicated operation, every resource is exclusive, and a
    // token holder exposes one (§8.1): no descriptor is needed.
    if !contract.resources.iter().any(is_replicated) {
        return Ok(true);
    }
    let Some(d) = descriptor else {
        return Err(format!(
            "instance {instance}: its descriptor cannot be read"
        ));
    };
    let exposed = d
        .exposed(contract)
        .ok_or_else(|| format!("instance {instance}: its descriptor does not list {iface}"))?;
    Ok(exposed.into_iter().any(|r| !is_replicated(r)))
}

/// Decides one candidate (§6, 0.7): a finding when at least two holders
/// expose an exclusive resource, clear when the holders that can be read
/// show at most one and none is left unread, undecided otherwise.
///
/// `descriptors` are the holders' (matched by service and instance), and
/// `contracts` the revisions they implement (matched by the descriptor's
/// fingerprint, else the token's fp16). A contract that declares no
/// replicated operation decides its holders without their descriptors.
#[must_use]
pub fn decide(
    candidate: &SplitBrain,
    revisions: &BTreeMap<InstanceId, Fp16>,
    descriptors: &[Descriptor],
    contracts: &[&Contract],
) -> Decision {
    let contracts: Vec<(Fingerprint, &Contract)> =
        contracts.iter().map(|c| (Fingerprint::of(c), *c)).collect();
    let mut exclusive = 0;
    let mut unread = Vec::new();
    for i in &candidate.instances {
        match exposes_exclusive(candidate, i, revisions.get(i), descriptors, &contracts) {
            Ok(true) => exclusive += 1,
            Ok(false) => {}
            Err(why) => unread.push(why),
        }
    }
    if exclusive >= 2 {
        Decision::Finding
    } else if unread.is_empty() {
        Decision::Clear
    } else {
        Decision::Undecided(unread)
    }
}

/// The diagnosis between two reads of presence taken a grace period apart,
/// for a tool that reads presence itself: the candidates ([`candidates`]),
/// each decided from what the tool read of the holders' descriptors and
/// contracts ([`decide`]).
#[must_use]
pub fn compare(
    before: &[ZkKey],
    after: &[ZkKey],
    descriptors: &[Descriptor],
    contracts: &[&Contract],
) -> Diagnosis {
    let mut out = Diagnosis::default();
    for (c, revisions) in candidates_with_revisions(before, after) {
        match decide(&c, &revisions, descriptors, contracts) {
            Decision::Finding => out.findings.push(c),
            Decision::Clear => {}
            Decision::Undecided(why) => out.undecided.push(Undecided { holders: c, why }),
        }
    }
    out
}

/// Diagnoses split-brains among the interface tokens matching `selector`
/// (`zk2/*/*/@zk/alive/**` for the whole bus): two reads `grace` apart,
/// each a liveliness GET waiting at most `timeout`; then, for each
/// candidate, its holders' descriptors (§3.3) and the bundles of the
/// revisions they name (§8.4), each GET waiting at most `timeout`. It takes
/// `grace` and fences nothing.
pub async fn split_brain(
    session: &zenoh::Session,
    selector: &str,
    grace: Duration,
    timeout: Duration,
) -> Result<Diagnosis> {
    let before = crate::presence::tokens(session, selector, timeout).await?;
    tokio::time::sleep(grace).await;
    let after = crate::presence::tokens(session, selector, timeout).await?;
    let candidates = candidates(&before, &after);
    let mut descriptors = Vec::new();
    let mut fingerprints = BTreeSet::new();
    for c in &candidates {
        for i in &c.instances {
            if let Found::Descriptor(d, _) =
                crate::presence::descriptor(session, &c.service, i, timeout).await?
            {
                let iface = c.iface.to_string();
                fingerprints.extend(
                    d.interfaces
                        .iter()
                        .filter(|e| e.iface == iface)
                        .filter_map(|e| Fingerprint::parse(&e.contract).ok())
                        .map(|fp| (c.iface.clone(), fp)),
                );
                descriptors.push(*d);
            }
        }
    }
    let mut contracts = Vec::new();
    for (iface, fp) in &fingerprints {
        if let Retrieved::Bundle(b, _) =
            crate::retrieval::fetch_bundle(session, iface, fp, timeout).await?
            && let Ok(c) = Contract::from_bundle(&b)
        {
            contracts.push(c);
        }
    }
    let refs: Vec<&Contract> = contracts.iter().collect();
    Ok(compare(&before, &after, &descriptors, &refs))
}

#[cfg(test)]
mod tests {
    use super::{Decision, candidates, compare};
    use zenkey_model::canonical::Fingerprint;
    use zenkey_model::contract::Contract;
    use zenkey_model::descriptor::Descriptor;
    use zenkey_model::grammar::{ZkKey, parse};

    fn alive(service: &str, instance: &str) -> ZkKey {
        parse(&format!(
            "zk2/h1/{service}/@zk/alive/tc.v1/{instance}/0123456789abcdef"
        ))
        .unwrap()
    }

    #[test]
    fn two_holders_in_both_reads_is_a_candidate() {
        let (a, b, c) = ("000000000000000a", "000000000000000b", "000000000000000c");
        // A split-brain, even with one side re-minted in between.
        let found = candidates(
            &[alive("tc", a), alive("tc", b)],
            &[alive("tc", a), alive("tc", c)],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].service.to_string(), "h1/tc");
        assert_eq!(found[0].instances.len(), 2);
        // A re-mint's overlap, caught by the first read only.
        assert!(candidates(&[alive("tc", a), alive("tc", b)], &[alive("tc", b)]).is_empty());
        // Two services, one instance each.
        assert!(
            candidates(
                &[alive("tc", a), alive("tc2", b)],
                &[alive("tc", a), alive("tc2", b)]
            )
            .is_empty()
        );
    }

    /// `tc.v1` with `set` exclusive and optional, `diagnostics` replicated
    /// when `replicated`.
    fn tc(replicated: bool) -> Contract {
        let toml = format!(
            "[interface]\nname = \"tc\"\nmajor = 1\nminor = 0\n\
             [resources.set]\nkind = \"operation\"\noptional = true\n\
             request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n\
             [resources.diagnostics]\nkind = \"operation\"\nidempotent = true\n{}\
             request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n",
            if replicated {
                "serving = \"replicated\"\n"
            } else {
                ""
            }
        );
        let l = zenkey_model::contract::load_str(&toml, std::path::Path::new("."), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn token(c: &Contract, instance: &str) -> ZkKey {
        let fp = Fingerprint::of(c);
        parse(&format!(
            "zk2/h1/tc/@zk/alive/tc.v1/{instance}/{}",
            fp.hex().fp16()
        ))
        .unwrap()
    }

    fn descriptor(c: &Contract, instance: &str, unavailable: &[&str]) -> Descriptor {
        let unavailable: Vec<String> = unavailable
            .iter()
            .map(|r| format!(r#"{{"resource": "{r}", "cause": "config"}}"#))
            .collect();
        serde_json::from_str(&format!(
            r#"{{"format": "zk2-descriptor/0.1", "service": "h1/tc", "instance": "{instance}",
                "interfaces": [{{"iface": "tc.v1", "contract": "{}", "minor": 0,
                                 "unavailable": [{}]}}]}}"#,
            Fingerprint::of(c),
            unavailable.join(", ")
        ))
        .unwrap()
    }

    /// Spec §6 (0.7, O-12): holders of which at most one exposes an
    /// exclusive resource are no finding; the check decides it from the
    /// contract and the descriptors, and says "undecided" without them.
    #[test]
    fn replicated_serving_is_exempt_and_decided_from_the_descriptors() {
        let (a, b) = ("000000000000000a", "000000000000000b");
        let rep = tc(true);
        let reads = [token(&rep, a), token(&rep, b)];
        let both = |ds: &[Descriptor], cs: &[&Contract]| compare(&reads, &reads, ds, cs);

        // One serves both, the other the replicated operation only: clear.
        let full = descriptor(&rep, a, &[]);
        let replica = descriptor(&rep, b, &["@op/set"]);
        assert!(both(&[full.clone(), replica.clone()], &[&rep]).is_clear());
        // Two replicas, the exclusive operation nowhere: clear.
        let other = descriptor(&rep, a, &["@op/set"]);
        assert!(both(&[other, replica.clone()], &[&rep]).is_clear());
        // Both expose the exclusive operation: a finding.
        let d = both(&[full.clone(), descriptor(&rep, b, &[])], &[&rep]);
        assert_eq!(d.findings.len(), 1);
        // A descriptor or the contract unread: undecided, never clear.
        let d = both(std::slice::from_ref(&full), &[&rep]);
        assert!(d.findings.is_empty());
        assert_eq!(d.undecided.len(), 1);
        assert!(d.undecided[0].why[0].contains(b), "{:?}", d.undecided);
        let d = both(&[full, replica], &[]);
        assert_eq!(d.undecided.len(), 1);

        // A contract without a replicated operation: a finding from the
        // tokens and the contract alone, no descriptor read.
        let excl = tc(false);
        let reads = [token(&excl, a), token(&excl, b)];
        let d = compare(&reads, &reads, &[], &[&excl]);
        assert_eq!(d.findings.len(), 1);
        assert!(d.undecided.is_empty());
        let d = compare(&reads, &reads, &[], &[]);
        assert!(matches!(
            super::decide(&d.undecided[0].holders, &Default::default(), &[], &[]),
            Decision::Undecided(_)
        ));
    }
}
