//! The standard revisions every tool holds compiled in: the bundles
//! `zenkey` embeds byte for byte, one for each standard contract it
//! implements (core §10 point 1): `health.v1`, `health.v2`, `freshness.v2`,
//! `alarms.v1`, `alarms.v2` and `config.v1`.
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

/// Each bundle `zenkey` embeds, by the interface it is the standard
/// contract of.
const EMBEDDED: [(&str, &[u8]); 6] = [
    ("health.v1", zenkey::health::BUNDLE),
    ("health.v2", zenkey::health::v2::BUNDLE),
    ("freshness.v2", zenkey::heartbeat::BUNDLE),
    ("alarms.v1", zenkey::alarms::BUNDLE),
    ("alarms.v2", zenkey::alarms::v2::BUNDLE),
    ("config.v1", zenkey::config_v1::BUNDLE),
];

static STANDARD: LazyLock<ContractSet> = LazyLock::new(|| {
    let mut set = ContractSet::new();
    for (iface, bytes) in EMBEDDED {
        let bundle = zenkey_model::bundle::Bundle::verify(bytes)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    /// Every bundle under `zenkey/bundles/`, the directory the runtime's
    /// `include_bytes!` read, is held here, and nothing else is: a
    /// standard contract `zenkey` comes to embed is one every tool holds.
    #[test]
    fn every_bundle_zenkey_embeds_is_held() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/bundles");
        let mut embedded = BTreeSet::new();
        for iface in std::fs::read_dir(&root).expect("zenkey's bundles") {
            let iface = iface.expect("an entry").path();
            let name = iface
                .file_name()
                .and_then(|n| n.to_str())
                .expect("a name")
                .to_owned();
            for file in std::fs::read_dir(&iface).expect("an interface's bundles") {
                let file = file.expect("an entry").file_name();
                let file = file.to_str().expect("a name");
                let hex = file.strip_suffix(".bundle.json").expect("a bundle");
                embedded.insert((name.clone(), format!("sha256:{hex}")));
            }
        }
        let held: BTreeSet<(String, String)> = revisions()
            .iter()
            .map(|r| (r.iface().to_string(), r.fingerprint().to_string()))
            .collect();
        assert_eq!(held, embedded);
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
