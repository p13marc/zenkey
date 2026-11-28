//! The standard revisions every tool holds compiled in: the contracts
//! `zenkey` embeds byte for byte ([`zenkey::standard`], which owns the
//! list), one for each standard contract it implements (core §10 point 1):
//! `health.v1`, `health.v2`, `freshness.v2`, `alarms.v1`, `alarms.v2` and
//! `config.v1`, here as revisions read from their bundles.
//!
//! An owner on a first hop holds no bundle, and a deployment holds each of
//! its bundles "on the near side of the hop: in a holder there, or
//! provisioned to each reader" (core §8.4). A reader built on this crate
//! is provisioned with these. So a compact descriptor naming one reads
//! with the deployment catalog alone
//! ([`crate::bus::presence::Describing`]), and a heartbeat's own key
//! resolves with no contract given (#990). Every other revision is held by
//! `--contracts` or retrieved, and one that cannot be had is never guessed
//! (core §3.3).

use std::sync::{Arc, LazyLock};

use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::IfaceId;

use crate::model::catalog::{ContractSet, Revision};
use crate::report::ContractSource;

static STANDARD: LazyLock<ContractSet> = LazyLock::new(|| {
    let mut set = ContractSet::new();
    for imp in zenkey::standard::implementations() {
        let iface = imp.iface();
        let bundle = zenkey_model::bundle::Bundle::verify(imp.bundle_bytes())
            .unwrap_or_else(|e| panic!("zenkey's {iface} bundle verifies: {e:?}"));
        let revision = Revision::from_bundle(bundle, ContractSource::Bundle)
            .unwrap_or_else(|e| panic!("zenkey's {iface} bundle reads: {e:?}"));
        set.insert(revision);
    }
    set
});

/// Every standard revision, by interface then fingerprint. Each one's
/// source is [`ContractSource::Bundle`]: it was read from a bundle file,
/// `zenkey`'s.
pub fn revisions() -> &'static ContractSet {
    &STANDARD
}

/// The standard revision `(iface, fingerprint)`, when it is one.
pub fn revision(iface: &IfaceId, fingerprint: &Fingerprint) -> Option<&'static Arc<Revision>> {
    STANDARD.get(iface, fingerprint)
}

/// Whether `revision` is the one compiled in, this set's own: what a
/// reader holds with nothing given and nothing retrieved, so its holding
/// says nothing of whether anyone on the bus serves the bundle. The same
/// revision given by `--contracts` or retrieved is another value.
pub fn is_compiled(revision: &Arc<Revision>) -> bool {
    STANDARD
        .get(revision.iface(), revision.fingerprint())
        .is_some_and(|r| Arc::ptr_eq(r, revision))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// One revision for each standard contract `zenkey` embeds, at its
    /// fingerprint: the runtime's readers and the tools hold one list
    /// (`zenkey::standard`'s test pins it to `zenkey/bundles/`).
    #[test]
    fn every_contract_zenkey_embeds_is_held() {
        let embedded: BTreeSet<(String, String)> = zenkey::standard::implementations()
            .iter()
            .map(|i| (i.iface().to_string(), i.fingerprint().to_string()))
            .collect();
        let held: BTreeSet<(String, String)> = revisions()
            .iter()
            .map(|r| (r.iface().to_string(), r.fingerprint().to_string()))
            .collect();
        assert_eq!(held, embedded);
        assert_eq!(held.len(), 6, "{held:?}");
    }

    #[test]
    fn a_standard_revision_is_found_by_its_fingerprint_alone() {
        let iface: IfaceId = "health.v1".parse().expect("an interface");
        let fp = Fingerprint::parse(&format!("sha256:{}", zenkey::health::FINGERPRINT))
            .expect("a fingerprint");
        let r = revision(&iface, &fp).expect("health.v1's standard revision");
        assert_eq!(r.source(), ContractSource::Bundle);
        let other = Fingerprint::parse(&format!("sha256:{}", "0".repeat(64))).expect("a fp");
        assert!(
            revision(&iface, &other).is_none(),
            "another revision is not one"
        );
    }
}
