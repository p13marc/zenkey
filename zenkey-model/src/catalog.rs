//! The deployment catalog (core §8.6; #980 C1a).
//!
//! A catalog is a file a deployment provisions to its owners and to the far
//! side of a constrained face. It states the deployment epoch, which core
//! §7.6's `time` fields count from, and gives each contract revision it
//! lists an index, which a later form carries in place of the interface id
//! and the fingerprint. It is never fetched and never served on the bus.
//!
//! - [`Catalog::parse`] is §8.6's reading, refusing a document for the first
//!   [`Reason`] that applies, in the text's order.
//! - [`Catalog::id`] is the first 4 bytes of the sha256 of its JCS bytes.
//! - [`Catalog::succeeds`] is the succession checker, and
//!   [`Catalog::successor`] with [`Catalog::push`] what `zk2 catalog add`
//!   writes.
//! - [`through`] and [`epoch_of`] are a reader's resolution: the catalog an
//!   owner names, or a held one whose lineage holds it, below that item's
//!   count.
//!
//! `spec/catalog.schema.json` is generated from [`Document`] (schemars). The
//! reading states more than JSON Schema can: an index at its position, no
//! pair twice, a lineage's counts. `spec/conformance/catalog/` pins it.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::canonical::Fingerprint;
use crate::chunk::is_lower_hex;
use crate::grammar::IfaceId;
use crate::schema::jcs;

/// The format tag of this version of the document.
pub const FORMAT: &str = "zk2-catalog/1";

/// At most this many entries: a later form carries an index in 10 bits.
pub const MAX_ENTRIES: usize = 1024;

/// The largest epoch: 2^53 − 1 seconds, an integer every JSON reader holds
/// exactly.
pub const MAX_EPOCH: i64 = (1 << 53) - 1;

/// The deployment epoch where no catalog is in force (core §7.6, §8.6): 0,
/// the Unix epoch, so a `time` field's 32 bits are Unix seconds.
pub const EPOCH_WITHOUT_CATALOG: i64 = 0;

/// A catalog's id: the first 4 bytes of the sha256 of its JCS bytes, spelled
/// as 8 lowercase hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogId([u8; 4]);

impl CatalogId {
    /// The id of a document's JCS bytes.
    #[must_use]
    pub fn of_jcs(bytes: &[u8]) -> Self {
        let d = Sha256::digest(bytes);
        Self([d[0], d[1], d[2], d[3]])
    }

    /// Exactly 8 lowercase hex digits, as a descriptor's `catalog` and a
    /// lineage item's `id` spell one; `None` otherwise.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if !is_lower_hex(s, 8) {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Self(v.to_be_bytes()))
    }

    /// The 8 lowercase hex digits.
    #[must_use]
    pub fn hex(self) -> String {
        format!("{:08x}", u32::from_be_bytes(self.0))
    }

    /// The 4 bytes, as a later form carries them.
    #[must_use]
    pub fn bytes(self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl FromStr for CatalogId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Self::from_hex(s).ok_or_else(|| format!("{s:?} is not 8 lowercase hex digits"))
    }
}

/// One contract revision a catalog lists, at its index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    pub iface: IfaceId,
    pub contract: Fingerprint,
}

/// A catalog this one succeeds: its id and its count of entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ancestor {
    pub id: CatalogId,
    pub entries: u16,
}

/// Why a reading refused a document (core §8.6), in the text's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Not strict JSON: RFC 8259, with no member twice in one object.
    Json,
    /// Not an object whose `format` is the string `zk2-catalog/1`.
    Format,
    /// A member missing, or one this version does not name: at the top, in
    /// an entry or in a lineage item.
    Member,
    /// `epoch` is not an integer from 0 to 2^53 − 1.
    Epoch,
    /// `entries` is not an array of objects, or holds more than 1,024.
    Entries,
    /// An entry's `index` is not the integer of its position.
    Index,
    /// An entry's `iface` is not a string holding an interface id.
    Iface,
    /// An entry's `contract` is not a string holding a fingerprint.
    Contract,
    /// An entry's pair is an earlier entry's.
    Duplicate,
    /// `lineage` is not a non-empty array of objects; or an item's `id` is
    /// not 8 lowercase hex digits, or is an earlier item's or the
    /// catalog's own; or its `entries` is not an integer, is below the
    /// item before it's, or above the catalog's count.
    Lineage,
}

impl Reason {
    /// The fixtures' token (`spec/conformance/catalog/documents.json`).
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Format => "format",
            Self::Member => "member",
            Self::Epoch => "epoch",
            Self::Entries => "entries",
            Self::Index => "index",
            Self::Iface => "iface",
            Self::Contract => "contract",
            Self::Duplicate => "duplicate",
            Self::Lineage => "lineage",
        }
    }
}

/// A refused document: the reason, where, and what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: Reason,
    /// `catalog`, `epoch`, `entries[3]`, `lineage[0]`…
    pub at: String,
    pub detail: String,
}

impl Refusal {
    fn new(reason: Reason, at: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            reason,
            at: at.into(),
            detail: detail.into(),
        }
    }

    /// The fixtures' token.
    #[must_use]
    pub fn token(&self) -> &'static str {
        self.reason.token()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.at, self.detail, self.token())
    }
}

impl std::error::Error for Refusal {}

/// Why a catalog does not succeed another (core §8.6), the first in the
/// checker's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Succession {
    /// The successor's epoch is not its predecessor's.
    EpochMoved,
    /// This index, below the predecessor's count, holds another pair.
    Reassigned(u16),
    /// This index of the predecessor is gone.
    Removed(u16),
    /// The successor's lineage is not its predecessor's, followed by the
    /// predecessor's id and count.
    Lineage,
}

impl Succession {
    /// The fixtures' token (`spec/conformance/catalog/succession.json`).
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::EpochMoved => "epoch_moved",
            Self::Reassigned(_) => "reassigned",
            Self::Removed(_) => "removed",
            Self::Lineage => "lineage",
        }
    }
}

impl fmt::Display for Succession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EpochMoved => f.write_str("the epoch moved"),
            Self::Reassigned(i) => write!(f, "index {i} holds another pair"),
            Self::Removed(i) => write!(f, "index {i} is gone"),
            Self::Lineage => {
                f.write_str("the lineage is not the predecessor's, followed by its id and count")
            }
        }?;
        write!(f, " ({})", self.token())
    }
}

/// Why a reader refused an index or an epoch (core §8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRefusal {
    /// It holds neither the catalog named nor one whose lineage holds it;
    /// or an index came with no catalog named.
    UnknownCatalog,
    /// The index is at or above the count read through.
    UnknownIndex,
}

impl ReadRefusal {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::UnknownCatalog => "unknown_catalog",
            Self::UnknownIndex => "unknown_index",
        }
    }
}

impl fmt::Display for ReadRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for ReadRefusal {}

/// A deployment catalog, read and checked (core §8.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    epoch: i64,
    entries: Vec<Entry>,
    lineage: Vec<Ancestor>,
    id: CatalogId,
}

/// What [`Catalog::push`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushed {
    /// The pair is new, at this index.
    Added(u16),
    /// The catalog already held the pair, at this index.
    Held(u16),
}

impl Pushed {
    /// The pair's index, either way.
    #[must_use]
    pub fn index(self) -> u16 {
        match self {
            Self::Added(i) | Self::Held(i) => i,
        }
    }
}

/// An integer as §8.6 reads one: a JSON number written as digits alone,
/// with no sign, fraction or exponent. The strict reader keeps such a
/// number as a `u64`; anything else, `-0` and an integer past `u64` among
/// them, it holds as a negative or a float.
fn digits(v: &Value) -> Option<u64> {
    v.as_u64()
}

const TOP: [&str; 4] = ["format", "epoch", "entries", "lineage"];
const ENTRY: [&str; 3] = ["index", "iface", "contract"];
const ITEM: [&str; 2] = ["id", "entries"];

/// Exactly these members: every `required`, and no member outside `allowed`.
fn members(m: &Map<String, Value>, required: &[&str], allowed: &[&str]) -> bool {
    required.iter().all(|k| m.contains_key(*k)) && m.keys().all(|k| allowed.contains(&k.as_str()))
}

impl Catalog {
    /// An empty first catalog, of this epoch.
    ///
    /// # Errors
    /// `epoch` outside 0 to 2^53 − 1 ([`Reason::Epoch`]).
    pub fn new(epoch: i64) -> Result<Self, Refusal> {
        if !(0..=MAX_EPOCH).contains(&epoch) {
            return Err(Refusal::new(
                Reason::Epoch,
                "epoch",
                format!("{epoch} is not from 0 to 2^53 - 1"),
            ));
        }
        let mut c = Self {
            epoch,
            entries: Vec::new(),
            lineage: Vec::new(),
            id: CatalogId([0; 4]),
        };
        c.id = c.compute_id();
        Ok(c)
    }

    /// Reads a catalog's bytes: UTF-8, then [`Catalog::parse`].
    ///
    /// # Errors
    /// [`Reason::Json`] for bytes that are not UTF-8, else as `parse`.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, Refusal> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Refusal::new(Reason::Json, "catalog", format!("not UTF-8: {e}")))?;
        Self::parse(text)
    }

    /// Reads a catalog as core §8.6 does, refusing it for the first reason
    /// that applies, in the text's order.
    ///
    /// # Errors
    /// The first [`Refusal`].
    pub fn parse(text: &str) -> Result<Self, Refusal> {
        let v = crate::strict::parse_json(text)
            .map_err(|e| Refusal::new(Reason::Json, "catalog", format!("not strict JSON: {e}")))?;
        let top = v
            .as_object()
            .filter(|m| m.get("format").and_then(Value::as_str) == Some(FORMAT))
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Format,
                    "format",
                    format!("not an object whose format is {FORMAT:?}"),
                )
            })?;
        if !members(top, &["epoch", "entries"], &TOP) {
            return Err(Refusal::new(
                Reason::Member,
                "catalog",
                "epoch or entries missing, or a member besides format, epoch, entries and lineage",
            ));
        }
        let epoch = digits(&top["epoch"])
            .and_then(|e| i64::try_from(e).ok())
            .filter(|e| *e <= MAX_EPOCH)
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Epoch,
                    "epoch",
                    format!("{} is not an integer from 0 to 2^53 - 1", top["epoch"]),
                )
            })?;
        let items = top["entries"]
            .as_array()
            .filter(|a| a.len() <= MAX_ENTRIES && a.iter().all(Value::is_object))
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Entries,
                    "entries",
                    format!("not an array of at most {MAX_ENTRIES} objects"),
                )
            })?;
        let mut entries = Vec::with_capacity(items.len());
        let mut seen = BTreeSet::new();
        for (i, item) in items.iter().enumerate() {
            entries.push(Self::read_entry(i, item, &mut seen)?);
        }
        let id = CatalogId::of_jcs(&jcs(&v));
        let lineage = match top.get("lineage") {
            None => Vec::new(),
            Some(l) => Self::read_lineage(l, entries.len(), id)?,
        };
        let c = Self {
            epoch,
            entries,
            lineage,
            id,
        };
        debug_assert_eq!(
            c.id,
            c.compute_id(),
            "the document read writes back to its own bytes"
        );
        Ok(c)
    }

    fn read_entry(
        i: usize,
        item: &Value,
        seen: &mut BTreeSet<(String, String)>,
    ) -> Result<Entry, Refusal> {
        let at = format!("entries[{i}]");
        let m = item.as_object().expect("entries checked as objects");
        if !members(m, &ENTRY, &ENTRY) {
            return Err(Refusal::new(
                Reason::Member,
                at,
                "not exactly index, iface and contract",
            ));
        }
        if digits(&m["index"]) != Some(i as u64) {
            return Err(Refusal::new(
                Reason::Index,
                at,
                format!("index {} is not its position, {i}", m["index"]),
            ));
        }
        let iface = m["iface"]
            .as_str()
            .and_then(|s| s.parse::<IfaceId>().ok())
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Iface,
                    &at,
                    format!("iface {} is not an interface id", m["iface"]),
                )
            })?;
        let contract = m["contract"]
            .as_str()
            .and_then(|s| Fingerprint::parse(s).ok())
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Contract,
                    &at,
                    format!("contract {} is not a fingerprint", m["contract"]),
                )
            })?;
        if !seen.insert((iface.to_string(), contract.to_string())) {
            return Err(Refusal::new(
                Reason::Duplicate,
                at,
                format!("{iface} at {contract} is an earlier entry's pair"),
            ));
        }
        Ok(Entry { iface, contract })
    }

    fn read_lineage(l: &Value, count: usize, own: CatalogId) -> Result<Vec<Ancestor>, Refusal> {
        let items = l
            .as_array()
            .filter(|a| !a.is_empty() && a.iter().all(Value::is_object))
            .ok_or_else(|| {
                Refusal::new(
                    Reason::Lineage,
                    "lineage",
                    "not a non-empty array of objects",
                )
            })?;
        let mut out: Vec<Ancestor> = Vec::with_capacity(items.len());
        for (k, item) in items.iter().enumerate() {
            let at = format!("lineage[{k}]");
            let m = item.as_object().expect("lineage checked as objects");
            if !members(m, &ITEM, &ITEM) {
                return Err(Refusal::new(
                    Reason::Member,
                    at,
                    "not exactly id and entries",
                ));
            }
            let id = m["id"]
                .as_str()
                .and_then(CatalogId::from_hex)
                .ok_or_else(|| {
                    Refusal::new(
                        Reason::Lineage,
                        &at,
                        format!("id {} is not 8 lowercase hex digits", m["id"]),
                    )
                })?;
            if id == own || out.iter().any(|a| a.id == id) {
                return Err(Refusal::new(
                    Reason::Lineage,
                    at,
                    format!("id {id} is an earlier item's or the catalog's own"),
                ));
            }
            let floor = out.last().map_or(0, |a| u64::from(a.entries));
            let entries = digits(&m["entries"])
                .filter(|n| *n >= floor && *n <= count as u64)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| {
                    Refusal::new(
                        Reason::Lineage,
                        &at,
                        format!(
                            "entries {} is not an integer from {floor} to the catalog's {count}",
                            m["entries"]
                        ),
                    )
                })?;
            out.push(Ancestor { id, entries });
        }
        Ok(out)
    }

    /// The deployment epoch, in whole seconds since the Unix epoch.
    #[must_use]
    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    /// The entries, each at its index.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The catalogs this one succeeds, oldest first.
    #[must_use]
    pub fn lineage(&self) -> &[Ancestor] {
        &self.lineage
    }

    /// The catalog's id.
    #[must_use]
    pub fn id(&self) -> CatalogId {
        self.id
    }

    /// The entry at an index, if the catalog holds one there.
    #[must_use]
    pub fn entry(&self, index: u16) -> Option<&Entry> {
        self.entries.get(usize::from(index))
    }

    /// The index of a revision, if the catalog lists it: what an owner
    /// writes for its interface and fingerprint.
    #[must_use]
    pub fn index_of(&self, iface: &IfaceId, contract: &Fingerprint) -> Option<u16> {
        self.entries
            .iter()
            .position(|e| &e.iface == iface && &e.contract == contract)
            .map(|i| u16::try_from(i).expect("at most 1,024 entries"))
    }

    /// The count a reader holding this catalog reads data named `id`
    /// through: its own when `id` is its own, a lineage item's when its
    /// lineage holds `id`, else `None`.
    #[must_use]
    pub fn holds(&self, id: CatalogId) -> Option<u16> {
        if id == self.id {
            return Some(self.count());
        }
        self.lineage.iter().find(|a| a.id == id).map(|a| a.entries)
    }

    fn count(&self) -> u16 {
        u16::try_from(self.entries.len()).expect("at most 1,024 entries")
    }

    /// Whether this catalog succeeds `previous` (core §8.6): its lineage is
    /// `previous`'s followed by `previous`'s id and count, its epoch is
    /// `previous`'s, and `previous`'s entries are its first ones.
    ///
    /// # Errors
    /// The first [`Succession`] refusal, in the checker's order.
    pub fn succeeds(&self, previous: &Catalog) -> Result<(), Succession> {
        if self.epoch != previous.epoch {
            return Err(Succession::EpochMoved);
        }
        for (i, e) in previous.entries.iter().enumerate() {
            let i = u16::try_from(i).expect("at most 1,024 entries");
            match self.entry(i) {
                Some(mine) if mine != e => return Err(Succession::Reassigned(i)),
                Some(_) => {}
                None => return Err(Succession::Removed(i)),
            }
        }
        let mut want = previous.lineage.clone();
        want.push(Ancestor {
            id: previous.id,
            entries: previous.count(),
        });
        if self.lineage != want {
            return Err(Succession::Lineage);
        }
        Ok(())
    }

    /// The catalog that succeeds this one with no entry added: its lineage
    /// extended by this one's id and count. [`Catalog::push`] then appends.
    #[must_use]
    pub fn successor(&self) -> Self {
        let mut c = self.clone();
        c.lineage.push(Ancestor {
            id: self.id,
            entries: self.count(),
        });
        c.id = c.compute_id();
        c
    }

    /// Appends a revision at the next index, or finds it already held.
    ///
    /// # Errors
    /// [`Reason::Entries`] when the catalog holds 1,024 entries;
    /// [`Reason::Lineage`] when the new id would be a lineage item's, a
    /// 32-bit collision. Either way the catalog is unchanged.
    pub fn push(&mut self, iface: IfaceId, contract: Fingerprint) -> Result<Pushed, Refusal> {
        if let Some(i) = self.index_of(&iface, &contract) {
            return Ok(Pushed::Held(i));
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Refusal::new(
                Reason::Entries,
                "entries",
                format!("the catalog holds {MAX_ENTRIES} entries already"),
            ));
        }
        self.entries.push(Entry { iface, contract });
        let id = self.compute_id();
        if self.lineage.iter().any(|a| a.id == id) {
            self.entries.pop();
            return Err(Refusal::new(
                Reason::Lineage,
                "lineage",
                format!("the new id {id} is a lineage item's"),
            ));
        }
        self.id = id;
        Ok(Pushed::Added(self.count() - 1))
    }

    /// The document, as [`Document`] spells it.
    #[must_use]
    pub fn document(&self) -> Document {
        Document {
            format: FORMAT.to_owned(),
            epoch: u64::try_from(self.epoch).expect("an epoch is from 0"),
            entries: self
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| DocumentEntry {
                    index: u16::try_from(i).expect("at most 1,024 entries"),
                    iface: e.iface.to_string(),
                    contract: e.contract.to_string(),
                })
                .collect(),
            lineage: (!self.lineage.is_empty()).then(|| {
                self.lineage
                    .iter()
                    .map(|a| DocumentAncestor {
                        id: a.id.hex(),
                        entries: a.entries,
                    })
                    .collect()
            }),
        }
    }

    /// The document as a JSON value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self.document()).expect("a catalog always serializes")
    }

    /// The document's JCS bytes, which its id hashes and `zk2 catalog`
    /// writes.
    #[must_use]
    pub fn to_jcs(&self) -> Vec<u8> {
        jcs(&self.to_value())
    }

    fn compute_id(&self) -> CatalogId {
        CatalogId::of_jcs(&self.to_jcs())
    }
}

/// What a reader reads an index or a time through (core §8.6): a held
/// catalog, and the count of its entries the named catalog had.
#[derive(Debug, Clone, Copy)]
pub struct Through<'a> {
    catalog: &'a Catalog,
    count: u16,
}

impl<'a> Through<'a> {
    /// The catalog held.
    #[must_use]
    pub fn catalog(&self) -> &'a Catalog {
        self.catalog
    }

    /// How many of its entries data under the named catalog can index.
    #[must_use]
    pub fn count(&self) -> u16 {
        self.count
    }

    /// The epoch to count times from: the held catalog's, which a
    /// successor keeps.
    #[must_use]
    pub fn epoch(&self) -> i64 {
        self.catalog.epoch
    }

    /// The entry at an index.
    ///
    /// # Errors
    /// [`ReadRefusal::UnknownIndex`] at or above the count.
    pub fn entry(&self, index: u16) -> Result<&'a Entry, ReadRefusal> {
        if index >= self.count {
            return Err(ReadRefusal::UnknownIndex);
        }
        self.catalog.entry(index).ok_or(ReadRefusal::UnknownIndex)
    }
}

/// Resolves the catalog an owner names among those a reader holds: the
/// named one itself where it is held, else the first whose lineage holds
/// its id, for the indices below that item's count.
///
/// # Errors
/// [`ReadRefusal::UnknownCatalog`] when no held catalog is or succeeds it.
pub fn through(held: &[Catalog], named: CatalogId) -> Result<Through<'_>, ReadRefusal> {
    held.iter()
        .find(|c| c.id == named)
        .or_else(|| held.iter().find(|c| c.holds(named).is_some()))
        .and_then(|catalog| catalog.holds(named).map(|count| Through { catalog, count }))
        .ok_or(ReadRefusal::UnknownCatalog)
}

/// The deployment epoch a reader counts an owner's times from: the epoch of
/// the catalog it names, read [`through`] what is held, or
/// [`EPOCH_WITHOUT_CATALOG`] when it names none.
///
/// # Errors
/// [`ReadRefusal::UnknownCatalog`] as [`through`].
pub fn epoch_of(held: &[Catalog], named: Option<CatalogId>) -> Result<i64, ReadRefusal> {
    named.map_or(Ok(EPOCH_WITHOUT_CATALOG), |id| {
        through(held, id).map(|t| t.epoch())
    })
}

/// A deployment catalog (core §8.6), as `zk2 catalog` writes it. This
/// schema cannot state all of §8.6's reading: an entry's index at its
/// position, no pair twice, a lineage's ids and counts, and an integer
/// written as digits alone are the reading's alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Catalog")]
pub struct Document {
    /// `zk2-catalog/1`.
    #[schemars(extend("const" = "zk2-catalog/1"))]
    pub format: String,
    /// The deployment epoch: whole seconds since the Unix epoch, which core
    /// §7.6's `time` fields count from.
    #[schemars(range(max = 9_007_199_254_740_991_u64))]
    pub epoch: u64,
    /// The revisions, each at the index of its position: at most 1,024, and
    /// no pair twice. An interface appears once per revision.
    #[schemars(length(max = 1024))]
    pub entries: Vec<DocumentEntry>,
    /// Every catalog this one succeeds, oldest first. Absent for a first
    /// catalog, and never empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<DocumentAncestor>", length(min = 1))]
    pub lineage: Option<Vec<DocumentAncestor>>,
}

/// One revision at its index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentEntry {
    /// Its position in `entries`, from 0.
    #[schemars(range(max = 1023))]
    pub index: u16,
    /// The interface id, `<name>.v<major>`.
    pub iface: String,
    /// The revision's fingerprint, `sha256:` and 64 lowercase hex digits.
    #[schemars(extend("pattern" = "^sha256:[0-9a-f]{64}$"))]
    pub contract: String,
}

/// A catalog this one succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentAncestor {
    /// Its id: 8 lowercase hex digits.
    #[schemars(extend("pattern" = "^[0-9a-f]{8}$"))]
    pub id: String,
    /// Its count of entries: at least the item before it's, and at most this
    /// catalog's.
    #[schemars(range(max = 1024))]
    pub entries: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(n: u32) -> Fingerprint {
        Fingerprint::parse(&format!("sha256:{n:064x}")).unwrap()
    }

    fn iface(s: &str) -> IfaceId {
        s.parse().unwrap()
    }

    /// The bound is 1,024 entries: the largest catalog reads, one more is
    /// refused for the count, and a push past it changes nothing.
    #[test]
    fn a_catalog_holds_at_most_1024_entries() {
        let mut c = Catalog::new(0).unwrap();
        for major in 0..1024 {
            assert_eq!(
                c.push(iface(&format!("a.v{major}")), fp(1)).unwrap(),
                Pushed::Added(u16::try_from(major).unwrap())
            );
        }
        let text = String::from_utf8(c.to_jcs()).unwrap();
        let read = Catalog::parse(&text).unwrap();
        assert_eq!(read, c);
        assert_eq!(read.entry(1023).unwrap().iface, iface("a.v1023"));
        let before = c.clone();
        let r = c.push(iface("b.v1"), fp(1)).unwrap_err();
        assert_eq!(r.token(), "entries");
        assert_eq!(c, before);
        let over = text.replacen("\"entries\":[", "\"entries\":[{},", 1);
        assert_eq!(Catalog::parse(&over).unwrap_err().token(), "entries");
    }

    /// What `zk2 catalog add` writes: a successor that reads back, succeeds
    /// its predecessor, and indexes data under it below its count.
    #[test]
    fn a_successor_reads_back_and_succeeds() {
        let mut a = Catalog::new(1_767_225_600).unwrap();
        a.push(iface("modem.v3"), fp(1)).unwrap();
        a.push(iface("config.v1"), fp(2)).unwrap();
        assert_eq!(a.push(iface("modem.v3"), fp(1)).unwrap(), Pushed::Held(0));
        let mut b = a.successor();
        assert_eq!(b.succeeds(&a), Ok(()));
        assert_eq!(b.push(iface("health.v1"), fp(3)).unwrap(), Pushed::Added(2));
        assert_eq!(b.push(iface("modem.v3"), fp(4)).unwrap(), Pushed::Added(3));
        assert_eq!(b.succeeds(&a), Ok(()));
        assert_eq!(a.succeeds(&b), Err(Succession::Removed(2)));
        assert_eq!(a.succeeds(&a), Err(Succession::Lineage));
        let read = Catalog::parse(std::str::from_utf8(&b.to_jcs()).unwrap()).unwrap();
        assert_eq!(read, b);
        assert_eq!(read.id(), b.id());
        assert_eq!(read.index_of(&iface("modem.v3"), &fp(4)), Some(3));
        assert_eq!(read.index_of(&iface("modem.v3"), &fp(9)), None);

        let held = [b.clone()];
        let t = through(&held, a.id()).unwrap();
        assert_eq!(t.count(), 2);
        assert_eq!(t.entry(1).unwrap().iface, iface("config.v1"));
        assert_eq!(t.entry(2).unwrap_err(), ReadRefusal::UnknownIndex);
        assert_eq!(
            through(&[a.clone()], b.id()).unwrap_err(),
            ReadRefusal::UnknownCatalog
        );
        assert_eq!(epoch_of(&held, Some(a.id())), Ok(1_767_225_600));
        assert_eq!(epoch_of(&held, None), Ok(EPOCH_WITHOUT_CATALOG));
    }

    /// A lineage item may not carry the catalog's own id. A document that
    /// does needs a 32-bit collision, so the check is reached here through
    /// the lineage reader directly.
    #[test]
    fn a_lineage_never_holds_the_catalogs_own_id() {
        let own = CatalogId::from_hex("0a90fd61").unwrap();
        let l = serde_json::json!([{"id": "0a90fd61", "entries": 0}]);
        let r = Catalog::read_lineage(&l, 0, own).unwrap_err();
        assert_eq!(r.token(), "lineage");
        let other = CatalogId::from_hex("0a90fd62").unwrap();
        assert_eq!(
            Catalog::read_lineage(&l, 0, other).unwrap(),
            [Ancestor {
                id: own,
                entries: 0
            }]
        );
    }

    #[test]
    fn an_id_is_8_lowercase_hex_digits() {
        assert_eq!(CatalogId::from_hex("0a90fd61").unwrap().hex(), "0a90fd61");
        assert_eq!(
            CatalogId::from_hex("0a90fd61").unwrap().bytes(),
            [0x0a, 0x90, 0xfd, 0x61]
        );
        for bad in ["0A90FD61", "0a90fd6", "0a90fd611", "+a90fd61", "0x90fd61"] {
            assert!(CatalogId::from_hex(bad).is_none(), "{bad}");
        }
        assert!(Catalog::new(-1).is_err());
        assert!(Catalog::new(MAX_EPOCH + 1).is_err());
        assert!(Catalog::new(MAX_EPOCH).is_ok());
    }
}
