//! A `.zrec` version-4 header in presence's place (the tooling guide's §5;
//! #990 V4): what served the capture's scope when it began, and the bundles
//! it carried, so a capture read later resolves as the live window did.
//!
//! - **Rungs 3 and 4.** An address and interface the header lists have a
//!   provider, and its fingerprint is the revision ([`Recorded::revisions_of`]).
//!   A pair it does not list has none, unless the reader reads it across a
//!   face, with `--contracts`.
//! - **Two fingerprints for one pair** are read as the live lens reads
//!   them: ambiguous where descriptors named them (`named` present), chosen
//!   as across a face where `--contracts` gave them (observer §7.5).
//! - **Rung 5** ([`Contracts`]): the header's bundle, verified by core
//!   §9.6 with its fingerprint the item's ([`Bundled`]), else the reader's
//!   `--contracts` holding that fingerprint. With neither, a revision the
//!   header lists is unavailable: its bundle refused by its tag, or not
//!   carried. A file without `bundles` (`record --no-bundles`) leaves it
//!   not held.
//! - **Times** count from the epoch in force ([`Recorded::stated`]): an
//!   owner whose revisions carry `named` states its catalog, one whose
//!   revisions carry none (across a face) takes the held catalog's.
//!
//! Session-free, as every model is.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::catalog::CatalogId;
use zenkey_model::grammar::{Addr, IfaceId};

use crate::model::catalog::{ContractSet, ContractState, Contracts, Revision};
use crate::model::deployment::Stated;
use crate::report::{ContractSource, ZrecHeader};

/// What a header's bundle came to on read (§5, step 3).
#[derive(Debug, Clone)]
pub enum Bundled {
    /// Verified, its contract read.
    Verified(Arc<Revision>),
    /// Refused by core §9.6, its tag named (`fingerprint` for step 12).
    Refused(String),
    /// Verified, and its contract does not read.
    Unreadable(String),
}

/// A version-4 header read for its rows: the revisions that served the
/// scope, the catalogs their owners named, and the bundles it carried.
#[derive(Debug, Clone, Default)]
pub struct Recorded {
    revisions: BTreeMap<(Addr, IfaceId), BTreeSet<Fingerprint>>,
    /// Each address's `named` values, for the revisions that carry one.
    named: BTreeMap<Addr, BTreeSet<Option<CatalogId>>>,
    /// Whether any revision of the address came with no `named`: across a
    /// face, no descriptor crossed.
    unnamed: BTreeSet<Addr>,
    bundles: BTreeMap<Fingerprint, Bundled>,
    /// Whether the header carries `bundles` at all.
    carried: bool,
    /// The reader's `--contracts`, by fingerprint, for rung 5.
    contracts: ContractSet,
}

impl Recorded {
    /// The header's revisions and bundles, each bundle verified by its
    /// fingerprint; `contracts`, the reader's `--contracts`, beside them.
    /// The header was read by [`crate::tape::record::read_header`], so
    /// every member here parses.
    pub fn of(header: &ZrecHeader, contracts: ContractSet) -> Recorded {
        let mut out = Recorded {
            contracts,
            carried: header.bundles.is_some(),
            ..Recorded::default()
        };
        for r in header.revisions.iter().flatten() {
            let (Ok(addr), Ok(iface), Ok(fp)) = (
                r.address.parse::<Addr>(),
                r.iface.parse::<IfaceId>(),
                Fingerprint::parse(&r.fingerprint),
            ) else {
                continue;
            };
            match &r.named {
                Some(named) => {
                    let id = named.as_deref().and_then(CatalogId::from_hex);
                    out.named.entry(addr.clone()).or_default().insert(id);
                }
                None => {
                    out.unnamed.insert(addr.clone());
                }
            }
            out.revisions.entry((addr, iface)).or_default().insert(fp);
        }
        for b in header.bundles.iter().flatten() {
            let Ok(fp) = Fingerprint::parse(&b.fingerprint) else {
                continue;
            };
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b.bytes)
                    .unwrap_or_default();
            let read = match Bundle::verify_expecting(&bytes, &fp) {
                Err(e) => Bundled::Refused(e.tag().to_owned()),
                Ok(bundle) => match Revision::from_bundle(bundle, ContractSource::Bundle) {
                    Ok(r) => Bundled::Verified(Arc::new(r)),
                    Err(e) => Bundled::Unreadable(e.to_string()),
                },
            };
            out.bundles.insert(fp, read);
        }
        out
    }

    /// Each bundle the header carried, as its read left it.
    pub fn bundles(&self) -> &BTreeMap<Fingerprint, Bundled> {
        &self.bundles
    }

    /// The fingerprints the header lists for `addr` and `iface`; `None`
    /// where it lists none, which is no provider (O2's rung 3).
    pub fn revisions_of(&self, addr: &Addr, iface: &IfaceId) -> Option<&BTreeSet<Fingerprint>> {
        self.revisions.get(&(addr.clone(), iface.clone()))
    }

    /// Whether `addr`'s revisions were named by its descriptors: then two
    /// of them for one interface are ambiguous, as live.
    pub fn named(&self, addr: &Addr) -> bool {
        self.named.contains_key(addr) && !self.unnamed.contains(addr)
    }

    /// What `addr`'s descriptors stated of its catalog at record time: the
    /// catalogs its revisions name, or unread where none carries `named`
    /// (across a face), so the held catalog is the deployment's word.
    pub fn stated(&self, addr: &Addr) -> Stated {
        self.named.get(addr).map_or(Stated::Unread, Stated::of)
    }

    /// Every revision held of `iface`, the header's verified bundles and
    /// the reader's contracts, for a choice across a face.
    pub fn held_among(&self, iface: &IfaceId, among: &BTreeSet<Fingerprint>) -> Vec<Arc<Revision>> {
        among
            .iter()
            .filter_map(|fp| self.state(iface, fp)?.revision().cloned())
            .collect()
    }
}

impl Contracts for Recorded {
    fn state(&self, iface: &IfaceId, fingerprint: &Fingerprint) -> Option<ContractState> {
        match self.bundles.get(fingerprint) {
            Some(Bundled::Verified(r)) if r.iface() == iface => {
                return Some(ContractState::Held(Arc::clone(r)));
            }
            Some(Bundled::Unreadable(reason)) => {
                return Some(ContractState::Unreadable {
                    reason: reason.clone(),
                });
            }
            _ => {}
        }
        if let Some(r) = self.contracts.get(iface, fingerprint) {
            return Some(ContractState::Held(Arc::clone(r)));
        }
        match self.bundles.get(fingerprint) {
            Some(Bundled::Refused(tag)) => Some(ContractState::Unavailable {
                refused: vec![tag.clone()],
            }),
            // Listed and not carried: one `record` could not retrieve.
            _ if self.carried && self.revisions.values().any(|f| f.contains(fingerprint)) => {
                Some(ContractState::Unavailable { refused: vec![] })
            }
            _ => None,
        }
    }

    fn held_of(&self, iface: &IfaceId) -> Vec<Arc<Revision>> {
        let mut out: Vec<Arc<Revision>> = self
            .bundles
            .values()
            .filter_map(|b| match b {
                Bundled::Verified(r) if r.iface() == iface => Some(Arc::clone(r)),
                _ => None,
            })
            .collect();
        out.extend(self.contracts.of_iface(iface).cloned());
        out
    }
}
