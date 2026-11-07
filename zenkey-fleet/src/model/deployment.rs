//! The deployment catalogs a tool holds (core §8.6; #980 C1c): what
//! `--catalog` loads, and the epoch a reader counts an owner's packed
//! `time` fields from.
//!
//! A catalog is a file the deployment provisions, never fetched and never
//! served (§8.6), so a tool holds one only when it is given one. Its type
//! is `zenkey_model`'s; here it is [`DeploymentCatalog`], since
//! [`crate::model::catalog::Catalog`] is the lens's index over a presence
//! read.
//!
//! **The epoch in force** ([`Catalogs::epoch`], §8.6): the owner's
//! descriptor names the catalog it writes under, or names none.
//! - It names one: read through it, or through a held catalog whose
//!   lineage holds its id. One no held catalog is or succeeds is refused,
//!   `unknown_catalog`, never guessed ([`Epoch::Unknown`]).
//! - It names none: its times count from 0, the Unix epoch.
//! - No descriptor in hand (across a constrained face, a capture read
//!   offline): the held catalog's epoch, the deployment's word, as its
//!   bindings are (R7); with none held, 0, assumed and said so.
//!
//! Session-free, so a `.zrec` reads through the same rule as live traffic.

use std::collections::BTreeSet;
use std::path::Path;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::catalog::EPOCH_WITHOUT_CATALOG;
use zenkey_model::grammar::IfaceId;

pub use zenkey_model::catalog::{Catalog as DeploymentCatalog, CatalogId};

/// The catalogs a tool was given, in the order given.
#[derive(Debug, Clone, Default)]
pub struct Catalogs {
    held: Vec<DeploymentCatalog>,
}

/// What a reader holds of the catalog an owner writes under: what its
/// descriptors say, when a descriptor is in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stated {
    /// No descriptor of the owner is in hand.
    Unread,
    /// Its descriptors name this catalog, or none (`None`).
    Names(Option<CatalogId>),
    /// Two of its instances' descriptors name different catalogs: a key
    /// does not say which instance wrote it.
    Disagree,
}

impl Stated {
    /// What a set of descriptors states: each instance's `catalog`.
    pub fn of(named: &BTreeSet<Option<CatalogId>>) -> Stated {
        let mut it = named.iter();
        match (it.next(), it.next()) {
            (None, _) => Stated::Unread,
            (Some(one), None) => Stated::Names(*one),
            (Some(_), Some(_)) => Stated::Disagree,
        }
    }
}

/// The deployment epoch a reader counts an owner's `time` fields from
/// (core §7.6, §8.6), and how it knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Epoch {
    /// The owner's word: the catalog its descriptor names, read through a
    /// held one, or 0 when it names none.
    Stated(i64),
    /// No descriptor in hand: the held catalogs' epoch, the deployment's
    /// word.
    Deployment(i64),
    /// Nothing stated it: 0, assumed, which a rendering says
    /// (`epoch_assumed`).
    Assumed,
    /// The owner names a catalog no held one is or succeeds
    /// (`unknown_catalog`).
    Unknown(CatalogId),
    /// No descriptor in hand, and the held catalogs state different
    /// epochs; or the owner's instances name different catalogs.
    Ambiguous,
}

impl Epoch {
    /// The epoch a decode is given: `None` where it is assumed or refused.
    pub fn given(self) -> Option<i64> {
        match self {
            Epoch::Stated(e) | Epoch::Deployment(e) => Some(e),
            Epoch::Assumed | Epoch::Unknown(_) | Epoch::Ambiguous => None,
        }
    }

    /// Why a record holding a `time` field is not read under it: the
    /// reader's refusal (§8.6), `None` where an epoch is given or assumed.
    pub fn refusal(self) -> Option<String> {
        match self {
            Epoch::Unknown(id) => Some(format!(
                "its owner writes under catalog {id}, which no --catalog holds or succeeds, so \
                 its time fields cannot be dated (unknown_catalog, core §8.6)"
            )),
            Epoch::Ambiguous => Some(
                "the catalogs in hand state different epochs, and nothing names the one its \
                 owner writes under, so its time fields cannot be dated (core §8.6)"
                    .to_owned(),
            ),
            Epoch::Stated(_) | Epoch::Deployment(_) | Epoch::Assumed => None,
        }
    }
}

/// Where a revision sits in one held catalog (`schema show`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub catalog: CatalogId,
    /// Its index, `None` when the catalog lacks it.
    pub index: Option<u16>,
}

impl Catalogs {
    pub fn new(held: Vec<DeploymentCatalog>) -> Catalogs {
        Catalogs { held }
    }

    /// Reads one catalog file as §8.6 reads it: a refusal's words end with
    /// its token.
    ///
    /// # Errors
    /// The file unreadable, or the document refused.
    pub fn load(path: &Path) -> Result<DeploymentCatalog, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        DeploymentCatalog::parse_bytes(&bytes)
            .map_err(|e| format!("{}: refused — {e}", path.display()))
    }

    /// Every catalog held, in the order given.
    pub fn held(&self) -> &[DeploymentCatalog] {
        &self.held
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The catalog an owner is given: one at most, since an owner writes
    /// under one (§8.6).
    ///
    /// # Errors
    /// Several held.
    pub fn one(&self) -> Result<Option<&DeploymentCatalog>, String> {
        match self.held.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            several => Err(format!(
                "{} catalogs given, and an owner writes under one (core §8.6)",
                several.len()
            )),
        }
    }

    /// The epoch a reader counts the `time` fields of an owner that
    /// `stated` its catalog from (§8.6).
    pub fn epoch(&self, stated: Stated) -> Epoch {
        match stated {
            Stated::Names(None) => Epoch::Stated(EPOCH_WITHOUT_CATALOG),
            Stated::Names(Some(id)) => zenkey_model::catalog::through(&self.held, id)
                .map_or(Epoch::Unknown(id), |t| Epoch::Stated(t.epoch())),
            Stated::Disagree => Epoch::Ambiguous,
            Stated::Unread => {
                let epochs: BTreeSet<i64> =
                    self.held.iter().map(DeploymentCatalog::epoch).collect();
                let mut it = epochs.iter();
                match (it.next(), it.next()) {
                    (None, _) => Epoch::Assumed,
                    (Some(e), None) => Epoch::Deployment(*e),
                    (Some(_), Some(_)) => Epoch::Ambiguous,
                }
            }
        }
    }

    /// Where a revision sits in each held catalog.
    pub fn placed(&self, iface: &IfaceId, contract: &Fingerprint) -> Vec<Placed> {
        self.held
            .iter()
            .map(|c| Placed {
                catalog: c.id(),
                index: c.index_of(iface, contract),
            })
            .collect()
    }

    /// The catalog a running revision is judged against (the doctor's
    /// `catalog-missing`): the one its owner names, read through what is
    /// held, the count of its entries the named one had; every held one
    /// when it names none.
    pub fn lacks(&self, named: Option<CatalogId>, iface: &IfaceId, contract: &Fingerprint) -> Lack {
        match named {
            Some(id) => match zenkey_model::catalog::through(&self.held, id) {
                Ok(t) => match t.catalog().index_of(iface, contract) {
                    Some(i) if i < t.count() => Lack::Held,
                    _ => Lack::Lacked(vec![id]),
                },
                Err(_) => Lack::NotHeld(id),
            },
            None => {
                if self
                    .held
                    .iter()
                    .any(|c| c.index_of(iface, contract).is_some())
                {
                    Lack::Held
                } else {
                    Lack::Lacked(self.held.iter().map(DeploymentCatalog::id).collect())
                }
            }
        }
    }
}

impl Catalogs {
    /// Whether a held catalog lists the revision: across a face, step 1's
    /// filter of the revision of a key (observer §7.5).
    pub fn lists(&self, iface: &IfaceId, contract: &Fingerprint) -> bool {
        self.held
            .iter()
            .any(|c| c.index_of(iface, contract).is_some())
    }

    /// The revision's index, the order step 3 reads (observer §7.5): the
    /// one every held catalog that lists it gives, since a successor keeps
    /// its predecessor's indices (core §8.6). `None` where none lists it,
    /// or two give it different indices, which orders nothing.
    pub fn order(&self, iface: &IfaceId, contract: &Fingerprint) -> Option<u16> {
        let mut at = self.held.iter().filter_map(|c| c.index_of(iface, contract));
        let first = at.next()?;
        at.all(|i| i == first).then_some(first)
    }
}

/// Whether a running revision is in the catalog it is judged against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lack {
    Held,
    /// Each of these catalogs lacks it.
    Lacked(Vec<CatalogId>),
    /// Its owner names a catalog no held one is or succeeds.
    NotHeld(CatalogId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modem() -> DeploymentCatalog {
        let p =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/zenoh-modem/catalog.json");
        Catalogs::load(&p).unwrap()
    }

    #[test]
    fn the_epoch_is_the_owners_word_then_the_deployments_then_assumed() {
        let c = modem();
        let id = c.id();
        let none = Catalogs::default();
        let held = Catalogs::new(vec![c.clone()]);
        assert_eq!(
            held.epoch(Stated::Names(Some(id))),
            Epoch::Stated(c.epoch())
        );
        assert_eq!(held.epoch(Stated::Names(None)), Epoch::Stated(0));
        assert_eq!(held.epoch(Stated::Unread), Epoch::Deployment(c.epoch()));
        assert_eq!(none.epoch(Stated::Unread), Epoch::Assumed);
        assert_eq!(none.epoch(Stated::Names(None)), Epoch::Stated(0));
        assert_eq!(none.epoch(Stated::Names(Some(id))), Epoch::Unknown(id));
        assert!(
            Epoch::Unknown(id)
                .refusal()
                .unwrap()
                .contains("unknown_catalog")
        );
        assert_eq!(held.epoch(Stated::Disagree), Epoch::Ambiguous);
        // A successor reads what its predecessor named.
        let mut next = c.successor();
        next.push(
            "timing.v1".parse().unwrap(),
            Fingerprint::parse(&format!("sha256:{}", "a".repeat(64))).unwrap(),
        )
        .unwrap();
        let later = Catalogs::new(vec![next]);
        assert_eq!(
            later.epoch(Stated::Names(Some(id))),
            Epoch::Stated(c.epoch())
        );
    }

    #[test]
    fn a_revision_is_judged_against_the_catalog_its_owner_names() {
        let c = modem();
        let e = &c.entries()[0];
        let held = Catalogs::new(vec![c.clone()]);
        assert_eq!(held.lacks(Some(c.id()), &e.iface, &e.contract), Lack::Held);
        assert_eq!(held.lacks(None, &e.iface, &e.contract), Lack::Held);
        let other = Fingerprint::parse(&format!("sha256:{}", "b".repeat(64))).unwrap();
        assert_eq!(
            held.lacks(None, &e.iface, &other),
            Lack::Lacked(vec![c.id()])
        );
        // Appended after the catalog the owner names: not its to index.
        let mut next = c.successor();
        next.push(e.iface.clone(), other.clone()).unwrap();
        let later = Catalogs::new(vec![next]);
        assert_eq!(
            later.lacks(Some(c.id()), &e.iface, &other),
            Lack::Lacked(vec![c.id()])
        );
        let stranger = CatalogId::from_hex("00000000").unwrap();
        assert_eq!(
            held.lacks(Some(stranger), &e.iface, &e.contract),
            Lack::NotHeld(stranger)
        );
    }
}
