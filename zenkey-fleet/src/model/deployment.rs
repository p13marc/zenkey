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

/// The catalogs a tool was given, in the order given, and the catalogs it
/// holds only as a `.zrec` header recorded them (the tooling guide's §5,
/// version 4): an id, an epoch and a lineage, no entries.
#[derive(Debug, Clone, Default)]
pub struct Catalogs {
    held: Vec<DeploymentCatalog>,
    recorded: Vec<RecordedCatalog>,
}

/// A catalog as a `.zrec` header records it (the tooling guide's §5): its
/// id, its epoch and its lineage, enough to read an owner's epoch through
/// it (core §8.6), and no entry to list a revision by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCatalog {
    pub id: CatalogId,
    pub epoch: i64,
    pub lineage: Vec<zenkey_model::catalog::Ancestor>,
}

impl RecordedCatalog {
    /// Whether it is `id`, or succeeds it.
    pub fn reads(&self, id: CatalogId) -> bool {
        self.id == id || self.lineage.iter().any(|a| a.id == id)
    }
}

/// The epoch in force and the catalog it is read from: what a date says
/// beside a time (the tooling guide's O8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dating {
    pub epoch: Epoch,
    /// The catalog the owner names, or with no descriptor the one held;
    /// `None` for an owner naming none, and where nothing states it.
    pub catalog: Option<CatalogId>,
}

impl From<Epoch> for Dating {
    fn from(epoch: Epoch) -> Dating {
        let catalog = match epoch {
            Epoch::Unknown(id) => Some(id),
            _ => None,
        };
        Dating { epoch, catalog }
    }
}

impl Dating {
    /// The date of a wire number counted from the deployment epoch, `ms`
    /// milliseconds after it (a frame record's `at`, a heartbeat's time):
    /// dated under a stated or a deployment epoch, named alone under an
    /// assumed or a refused one (O8).
    pub fn dated_from(self, ms: i128) -> crate::report::Dated {
        let utc = self
            .epoch
            .given()
            .and_then(|e| rfc3339_ms(i128::from(e) * 1000 + ms, true));
        self.dated(utc)
    }

    /// The date of a value already read under this epoch as milliseconds
    /// since the Unix epoch (a packed `time` field, core §7.6's reading),
    /// at seconds where the field carries no milliseconds.
    pub fn dated_unix(self, unix_ms: i128, with_ms: bool) -> crate::report::Dated {
        let utc = self
            .epoch
            .given()
            .and_then(|_| rfc3339_ms(unix_ms, with_ms));
        self.dated(utc)
    }

    fn dated(self, utc: Option<String>) -> crate::report::Dated {
        use crate::report::DatedEpoch as E;
        let (epoch, catalog) = match self.epoch {
            Epoch::Stated(_) => (E::Stated, self.catalog),
            Epoch::Deployment(_) => (E::Deployment, self.catalog),
            Epoch::Assumed => (E::Assumed, None),
            Epoch::Unknown(id) => (E::UnknownCatalog, Some(id)),
            Epoch::Ambiguous => (E::Ambiguous, None),
        };
        let utc = match epoch {
            E::Stated | E::Deployment => utc,
            _ => None,
        };
        crate::report::Dated {
            utc,
            epoch,
            catalog: catalog.map(CatalogId::hex),
        }
    }
}

/// RFC 3339 UTC of `unix_ms` milliseconds since the Unix epoch, at
/// milliseconds or at seconds: `2026-11-07T14:49:16.523Z`. `None` past
/// year 9999 or before year 0.
pub fn rfc3339_ms(unix_ms: i128, with_ms: bool) -> Option<String> {
    let secs = unix_ms.div_euclid(1000);
    let ms = unix_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i128::from(m <= 2);
    if !(0..=9999).contains(&y) {
        return None;
    }
    let (h, mi, s) = (tod / 3600, tod % 3600 / 60, tod % 60);
    Some(if with_ms {
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
    } else {
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    })
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
        Catalogs {
            held,
            recorded: Vec::new(),
        }
    }

    /// Catalogs held only as a `.zrec` header recorded them (§5, version
    /// 4): an owner's epoch reads through them, and they list no revision.
    pub fn recorded(recorded: Vec<RecordedCatalog>) -> Catalogs {
        Catalogs {
            held: Vec::new(),
            recorded,
        }
    }

    /// Whether a catalog with its entries is held: what lists a revision,
    /// across a face (observer §7.5's step 1).
    pub fn lists_entries(&self) -> bool {
        !self.held.is_empty()
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
        self.held.is_empty() && self.recorded.is_empty()
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
        self.dating(stated).epoch
    }

    /// [`Catalogs::epoch`], with the catalog it is read from (the tooling
    /// guide's O8): the one the owner names, or with no descriptor the one
    /// held.
    pub fn dating(&self, stated: Stated) -> Dating {
        match stated {
            Stated::Names(None) => Dating {
                epoch: Epoch::Stated(EPOCH_WITHOUT_CATALOG),
                catalog: None,
            },
            Stated::Names(Some(id)) => {
                let epoch = match zenkey_model::catalog::through(&self.held, id) {
                    Ok(t) => Epoch::Stated(t.epoch()),
                    Err(_) => self
                        .recorded
                        .iter()
                        .find(|r| r.reads(id))
                        .map_or(Epoch::Unknown(id), |r| Epoch::Stated(r.epoch)),
                };
                Dating {
                    epoch,
                    catalog: Some(id),
                }
            }
            Stated::Disagree => Dating {
                epoch: Epoch::Ambiguous,
                catalog: None,
            },
            Stated::Unread => {
                let all: Vec<(i64, CatalogId)> = self
                    .held
                    .iter()
                    .map(|c| (c.epoch(), c.id()))
                    .chain(self.recorded.iter().map(|r| (r.epoch, r.id)))
                    .collect();
                let epochs: BTreeSet<i64> = all.iter().map(|(e, _)| *e).collect();
                let mut it = epochs.iter();
                match (it.next(), it.next()) {
                    (None, _) => Dating {
                        epoch: Epoch::Assumed,
                        catalog: None,
                    },
                    (Some(e), None) => Dating {
                        epoch: Epoch::Deployment(*e),
                        catalog: all.first().map(|(_, id)| *id),
                    },
                    (Some(_), Some(_)) => Dating {
                        epoch: Epoch::Ambiguous,
                        catalog: None,
                    },
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

    /// O8: a date at the wire's precision, from the epoch in force, named;
    /// an assumed or a refused epoch dates nothing.
    #[test]
    fn a_time_is_dated_under_the_epoch_in_force_and_named() {
        use crate::report::DatedEpoch;
        let c = modem();
        let id = c.id();
        let held = Catalogs::new(vec![c.clone()]);
        let stated = held.dating(Stated::Names(Some(id)));
        let d = stated.dated_from(26_837_356_523);
        assert_eq!(d.utc.as_deref(), Some("2026-11-07T14:49:16.523Z"));
        assert_eq!((d.epoch, d.catalog), (DatedEpoch::Stated, Some(id.hex())));
        let deployment = held.dating(Stated::Unread).dated_from(0);
        assert_eq!(deployment.epoch, DatedEpoch::Deployment);
        assert_eq!(deployment.utc.as_deref(), Some("2026-01-01T00:00:00.000Z"));
        let zero = held.dating(Stated::Names(None)).dated_unix(1000, false);
        assert_eq!(
            (zero.utc.as_deref(), zero.catalog),
            (Some("1970-01-01T00:00:01Z"), None)
        );
        let assumed = Catalogs::default().dating(Stated::Unread).dated_from(5);
        assert_eq!((assumed.epoch, assumed.utc), (DatedEpoch::Assumed, None));
        let unknown = Catalogs::default()
            .dating(Stated::Names(Some(id)))
            .dated_from(5);
        assert_eq!(
            (unknown.epoch, unknown.utc, unknown.catalog),
            (DatedEpoch::UnknownCatalog, None, Some(id.hex()))
        );
        // A header's catalog: read through by id and lineage, no entries.
        let rec = Catalogs::recorded(vec![RecordedCatalog {
            id,
            epoch: c.epoch(),
            lineage: c.lineage().to_vec(),
        }]);
        assert_eq!(rec.epoch(Stated::Names(Some(id))), Epoch::Stated(c.epoch()));
        assert_eq!(
            rec.epoch(Stated::Names(Some(c.lineage()[0].id))),
            Epoch::Stated(c.epoch())
        );
        assert!(!rec.lists_entries());
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
