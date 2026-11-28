//! The standard contracts this crate embeds, every reader's own (core §10
//! point 1, §8.4): `health.v1`, `health.v2`, `freshness.v2`, `alarms.v1`,
//! `alarms.v2` and `config.v1`, each its bundle byte for byte as published
//! in `spec/profiles/.history/`, implemented once.
//!
//! An owner on a first hop holds no bundle, and a deployment holds each of
//! its bundles "on the near side of the hop: in a holder there, or
//! provisioned to each reader" (core §8.4). A reader built on this crate is
//! provisioned with these. So a compact descriptor naming one reads
//! through the catalog alone ([`crate::presence::Held`]), never retrieved:
//! a consumer of a first-hop device's own interface, holding only its own
//! contract and the catalog, reads the device's descriptor, and binds.
//! Every other revision is the reader's or retrieved, and one that cannot
//! be had leaves the descriptor unread, never guessed (core §3.3).
//! `zenkey-fleet`'s tools hold the same list.

use std::sync::{Arc, LazyLock};

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::grammar::IfaceId;

use crate::implementation::Implementation;

static STANDARD: LazyLock<Vec<Implementation>> = LazyLock::new(|| {
    vec![
        crate::health::implementation(),
        crate::health::v2::implementation(),
        crate::heartbeat::implementation(),
        crate::alarms::implementation(),
        crate::alarms::v2::implementation(),
        crate::config_v1::implementation(),
    ]
});

/// Every standard contract embedded, implemented from its bundle
/// ([`Implementation::bundle_bytes`] are the published bytes).
#[must_use]
pub fn implementations() -> &'static [Implementation] {
    &STANDARD
}

/// The standard contract `(iface, fingerprint)`, when it is one.
#[must_use]
pub fn contract(iface: &IfaceId, fingerprint: &Fingerprint) -> Option<Arc<Contract>> {
    STANDARD
        .iter()
        .find(|i| i.iface() == iface && i.fingerprint() == fingerprint)
        .map(Implementation::shared_contract)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    /// Every bundle under `bundles/`, the directory the profile modules'
    /// `include_bytes!` read, is one of these, and nothing else is: a
    /// standard contract this crate comes to embed is one every reader
    /// holds.
    #[test]
    fn every_bundle_embedded_is_a_standard_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundles");
        let mut embedded = BTreeSet::new();
        for iface in std::fs::read_dir(&root).expect("the bundles") {
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
        let held: BTreeSet<(String, String)> = implementations()
            .iter()
            .map(|i| (i.iface().to_string(), i.fingerprint().to_string()))
            .collect();
        assert_eq!(held, embedded);
    }

    #[test]
    fn a_standard_contract_is_found_by_its_fingerprint_alone() {
        let fp = |hex: &str| Fingerprint::parse(&format!("sha256:{hex}")).expect("a fp");
        let health = crate::health::iface();
        let c = contract(&health, &fp(crate::health::FINGERPRINT)).expect("health.v1's");
        assert_eq!(c.iface, health);
        assert!(contract(&health, &fp(&"0".repeat(64))).is_none());
    }
}
