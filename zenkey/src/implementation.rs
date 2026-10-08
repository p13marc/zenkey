//! An interface a service implements: its contract, and the bundle it
//! serves (spec §8.2 step 3, §9.6).

use std::sync::Arc;

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, Resource};
use zenkey_model::grammar::IfaceId;

use crate::error::{Error, Result};

/// A contract revision this service implements, with its bundle bytes.
///
/// The bytes are built once, by `zenkey-model`'s bundle builder, and served
/// as they are: a running service never rebuilds them differently (#611).
#[derive(Debug, Clone)]
pub struct Implementation {
    contract: Arc<Contract>,
    bundle: Arc<[u8]>,
    fingerprint: Fingerprint,
}

impl Implementation {
    /// Builds the bundle of `contract`.
    #[must_use]
    pub fn new(contract: Contract) -> Self {
        let bundle = Bundle::build(&contract);
        Self {
            fingerprint: bundle.fingerprint(),
            bundle: bundle.to_bytes().into(),
            contract: Arc::new(contract),
        }
    }

    /// Pairs `contract` with bundle bytes built earlier (embedded by
    /// codegen), verifying that they are its bundle (§9.6, with the
    /// expected fingerprint).
    pub fn with_bundle(contract: Contract, bundle: Vec<u8>) -> Result<Self> {
        let want = Fingerprint::of(&contract);
        Bundle::verify_expecting(&bundle, &want).map_err(|e| {
            Error::Contract(format!(
                "the bundle for {} does not verify: {e}",
                contract.iface
            ))
        })?;
        Ok(Self {
            fingerprint: want,
            bundle: bundle.into(),
            contract: Arc::new(contract),
        })
    }

    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.contract.iface
    }

    #[must_use]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    #[must_use]
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// The bundle, as served on the contract key.
    #[must_use]
    pub fn bundle_bytes(&self) -> &Arc<[u8]> {
        &self.bundle
    }

    /// The resource named `<kind token>/<template>` (the descriptor's
    /// naming, §3.3).
    pub fn resource(&self, name: &str) -> Result<&Resource> {
        let found = name.split_once('/').and_then(|(token, template)| {
            let token = token.parse().ok()?;
            self.contract.resource(token, template)
        });
        found.ok_or_else(|| Error::NoResource {
            iface: self.contract.iface.clone(),
            resource: name.to_owned(),
        })
    }
}

/// A resource's name in the descriptor: `<kind token>/<template>`.
#[must_use]
pub fn resource_name(r: &Resource) -> String {
    format!("{}/{}", r.token, r.template)
}

/// The capability a resource is gated on that `held` lacks, if any (§3.3).
pub(crate) fn missing_capability<'r>(
    r: &'r Resource,
    held: &std::collections::BTreeSet<String>,
) -> Option<&'r str> {
    r.gate
        .iter()
        .filter_map(|g| g.strip_prefix("capability:"))
        .find(|cap| !held.contains(*cap))
}
