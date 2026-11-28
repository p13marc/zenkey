//! The compact descriptor (core §3.3, "The compact form"; #980 C5a).
//!
//! A descriptor's second form, in bytes: §7.5's mutation form on the
//! instance key under `zenoh/bytes`, its record a body that names each
//! revision by its index in the deployment catalog (§8.6) and states
//! capabilities as bits. [`decode`] gives today's [`Descriptor`], so
//! [`super::check_descriptor`] reads it as it reads JSON; [`encode`] writes
//! one back. No rule adopts the form yet: no owner writes it and no reader
//! is required to read it (C5b, C5c).
//!
//! The reading takes §3.3's three steps: [`Head::parse`] with no catalog and
//! no contract, [`Head::pairs`] through the catalogs held, then the rest
//! with each entry's revision. Every [`Refusal`] is the first, in §3.3's
//! order, field by field. `spec/conformance/descriptors/compact/` pins
//! them.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::{Cause, Descriptor, FORMAT, InterfaceEntry, Unavailable};
use crate::canonical::Fingerprint;
use crate::catalog::{self, Catalog, CatalogId, ReadRefusal};
use crate::collection::{IntegerRefusal, read_integer, write_integer};
use crate::contract::Contract;
use crate::grammar::{Addr, IfaceId, InstanceId};
use crate::packed::BitWriter;
use crate::wire::{Form, Header, Run, Seq, Stamp, Tag};

/// The most entries a compact descriptor lists: its count is 4 bits.
pub const MAX_ENTRIES: usize = 15;

/// The most items an unavailable or a cardinality list holds: its count
/// is 8 bits.
pub const MAX_ITEMS: usize = 255;

/// An entry's bits: its index (10) and its four flags.
pub const ENTRY_BITS: u32 = 14;

/// An entry's index: 10 bits, as an owner heartbeat's group's (§7.5).
pub const INDEX_BITS: u32 = 10;

/// The zid: 128 bits.
pub const ZID_BITS: u32 = 128;

/// The profile a set `minted` bit lists: today's one derivation-only
/// profile (core §10).
pub const MINTED_PROFILE: &str = "hostid.v1";

/// A compact descriptor's value: the descriptor it decodes to, its
/// envelope's seq (the instance's as-of mark when it put it, core §5.1) and
/// its `minted` bit. The bytes write back from these three (§3.3,
/// "Writing").
#[derive(Debug, Clone, PartialEq)]
pub struct Compact {
    pub descriptor: Descriptor,
    pub seq: Seq,
    /// The instance's system is minted, so it follows `hostid.v1`: its
    /// decoded `profiles` list it. A revision that lists `hostid.v1` in
    /// `uses` puts it there either way, so only the bit says which.
    pub minted: bool,
}

/// One entry as the head writes it: the revision's index and the four bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryBits {
    pub index: u16,
    pub token: bool,
    pub standby: bool,
    /// The entry's unavailable list follows.
    pub has_unav: bool,
    /// The entry's cardinality list follows.
    pub has_card: bool,
}

/// What a compact descriptor's head holds (§3.3 "Reading", step 1): what
/// a reader reads with no catalog and no contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The envelope's run and seq.
    pub stamp: Stamp,
    /// The catalog the instance writes under.
    pub catalog: CatalogId,
    pub minted: bool,
    /// The zid follows.
    pub has_zid: bool,
    /// From 1 to 15, ordered by index, an index possibly twice.
    pub entries: Vec<EntryBits>,
}

/// Why a reader refuses a compact descriptor (core §3.3, "Reading"), in
/// the text's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Refusal {
    /// No byte, or the bytes end before a field.
    #[error("short: the bytes end before a field")]
    Short,
    /// CH's form is not 0, or its tag is not 0.
    #[error("form: CH is not a mutation's with the tag 0")]
    Form,
    /// A count of 0: of entries, or of a list's items.
    #[error("count: a count of 0")]
    Count,
    /// An extension bit is 1.
    #[error("extension: an extension bit is 1")]
    Extension,
    /// An entry's index below the one before it, or a list's resource not
    /// above the one before it.
    #[error("order: an index or a resource out of order")]
    Order,
    /// No held catalog is the one named, or succeeds it (core §8.6).
    #[error("unknown_catalog: no held catalog is or succeeds the one named")]
    UnknownCatalog,
    /// An index at or above the count read through (core §8.6).
    #[error("unknown_index: an index at or above the count read through")]
    UnknownIndex,
    /// No held revision of the pair an index names.
    #[error("unknown_revision: no held revision of the pair an index names")]
    UnknownRevision,
    /// A capability's bit not the one an earlier bitmap holds for it.
    #[error("capability: two bitmaps disagree on a capability")]
    Capability,
    /// A resource at or above its revision's count of resources.
    #[error("resource: a resource past its revision's")]
    Resource,
    /// A cause of 0.
    #[error("cause: a cause of 0")]
    Cause,
    /// A bound refused as a snapshot's integer is (core §7.5).
    #[error("leb128: a bound that is not LEB128 in its shortest form, at most 2^32 − 1")]
    Leb128,
    /// A byte after the pad.
    #[error("trailing: a byte after the pad")]
    Trailing,
    /// A pad bit is 1.
    #[error("pad: a pad bit is 1")]
    Pad,
}

impl Refusal {
    /// The fixtures' token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Form => "form",
            Self::Count => "count",
            Self::Extension => "extension",
            Self::Order => "order",
            Self::UnknownCatalog => "unknown_catalog",
            Self::UnknownIndex => "unknown_index",
            Self::UnknownRevision => "unknown_revision",
            Self::Capability => "capability",
            Self::Resource => "resource",
            Self::Cause => "cause",
            Self::Leb128 => "leb128",
            Self::Trailing => "trailing",
            Self::Pad => "pad",
        }
    }
}

impl From<ReadRefusal> for Refusal {
    fn from(r: ReadRefusal) -> Self {
        match r {
            ReadRefusal::UnknownCatalog => Self::UnknownCatalog,
            ReadRefusal::UnknownIndex => Self::UnknownIndex,
        }
    }
}

/// Why a descriptor has no compact form (core §3.3, "Which descriptors
/// have one"): its owner writes it as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncodeError {
    /// It states no run, or misspells it (D002).
    #[error("no run: the envelope needs one")]
    Run,
    /// It names no catalog, or misspells its id (D002).
    #[error("no catalog: an index needs one")]
    Catalog,
    /// No catalog held is the one it names, or succeeds it.
    #[error("the catalog it names is not held: {0}")]
    Unheld(ReadRefusal),
    /// It lists no interface, or more than [`MAX_ENTRIES`].
    #[error("{0} interfaces: a compact descriptor lists from 1 to 15")]
    Count(usize),
    /// It lists a requirement.
    #[error("a requirement: no catalog indexes a binding")]
    Requires,
    /// An entry's revision is not among those the catalog lists.
    #[error("{iface} {contract}: not in the catalog it names")]
    Uncatalogued { iface: String, contract: String },
    /// No revision given is the entry's.
    #[error("{iface} {contract}: its revision is not held")]
    Revision { iface: String, contract: String },
    /// An unavailable resource or a cardinality key that names no resource
    /// of the entry's revision.
    #[error("{iface}: {resource:?} is not one of its resources")]
    Resource { iface: String, resource: String },
    /// One entry's unavailable list names a resource twice.
    #[error("{iface}: {resource:?} is listed unavailable twice")]
    Twice { iface: String, resource: String },
    /// A list of more than [`MAX_ITEMS`].
    #[error("{iface}: a list of {items} items, above 255")]
    Items { iface: String, items: usize },
    /// A bound above 2^32 − 1.
    #[error("{iface}: the bound {bound} is above 2^32 − 1")]
    Bound { iface: String, bound: u64 },
    /// A `meta.zid` that is not 1 to 32 lowercase hex digits.
    #[error("meta.zid {0} is not 1 to 32 lowercase hex digits")]
    Zid(String),
}

/// The capabilities a revision gates on (core §3.3, "The bitmaps"): each
/// `<n>` of a `capability:<n>` gate among its resources, once, sorted as
/// strings, bytewise.
#[must_use]
pub fn gated(revision: &Contract) -> Vec<String> {
    let caps: BTreeSet<&str> = revision
        .resources
        .iter()
        .flat_map(|r| r.gate.iter())
        .filter_map(|g| g.strip_prefix("capability:"))
        .collect();
    caps.into_iter().map(str::to_owned).collect()
}

/// The bits a revision writes a resource in (core §3.3, "A resource"):
/// ⌈log₂ n⌉ for n resources, none where n is 0 or 1.
#[must_use]
pub fn resource_bits(revision: &Contract) -> u32 {
    let n = revision.resources.len();
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

/// A revision's resources by name, `<kind token>/<template>`, in the
/// canonical form's order (core §9.5): an item's resource is its place here.
#[must_use]
pub fn resource_names(revision: &Contract) -> Vec<String> {
    revision
        .resources
        .iter()
        .map(|r| format!("{}/{}", r.token.as_str(), r.template))
        .collect()
}

/// The code §7.5's error byte gives a cause: 1 build, 2 config, 3
/// capability.
fn cause_code(c: Cause) -> u64 {
    match c {
        Cause::Build => 1,
        Cause::Config => 2,
        Cause::Capability => 3,
    }
}

fn cause_of(code: u64) -> Option<Cause> {
    match code {
        1 => Some(Cause::Build),
        2 => Some(Cause::Config),
        3 => Some(Cause::Capability),
        _ => None,
    }
}

/// The revisions held, each with its fingerprint, computed once.
struct Held<'c>(Vec<(Fingerprint, &'c Contract)>);

impl<'c> Held<'c> {
    fn new(revisions: &[&'c Contract]) -> Self {
        Held(revisions.iter().map(|c| (Fingerprint::of(c), *c)).collect())
    }

    fn of(&self, e: &catalog::Entry) -> Option<&'c Contract> {
        self.0
            .iter()
            .find(|(fp, c)| *fp == e.contract && c.iface == e.iface)
            .map(|(_, c)| *c)
    }
}

/// Bits read in order, `short` where they end.
struct Bits<'a> {
    bytes: &'a [u8],
    pos: u64,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Bits { bytes, pos: 0 }
    }

    fn left(&self) -> u64 {
        self.bytes.len() as u64 * 8 - self.pos
    }

    fn take(&mut self, width: u32) -> Result<u64, Refusal> {
        debug_assert!(width <= 64);
        if u64::from(width) > self.left() {
            return Err(Refusal::Short);
        }
        let mut v = 0u64;
        for _ in 0..width {
            let b = (self.bytes[(self.pos / 8) as usize] >> (7 - self.pos % 8)) & 1;
            v = (v << 1) | u64::from(b);
            self.pos += 1;
        }
        Ok(v)
    }

    fn bit(&mut self) -> Result<bool, Refusal> {
        self.take(1).map(|b| b == 1)
    }

    /// §7.5's integer, each byte the next 8 bits wherever they fall.
    fn integer(&mut self) -> Result<u32, Refusal> {
        read_integer(|| self.take(8).ok().map(|b| b as u8)).map_err(|e| match e {
            IntegerRefusal::Short => Refusal::Short,
            IntegerRefusal::Leb128 => Refusal::Leb128,
        })
    }

    /// No byte after the pad (`trailing`), then every pad bit 0 (`pad`).
    fn end(mut self) -> Result<(), Refusal> {
        if self.bytes.len() as u64 != self.pos.div_ceil(8) {
            return Err(Refusal::Trailing);
        }
        while self.left() > 0 {
            if self.bit()? {
                return Err(Refusal::Pad);
            }
        }
        Ok(())
    }
}

impl Head {
    /// The head (§3.3 "Reading", step 1), with no catalog and no contract:
    /// the refusals `short`, `form`, `count`, `extension` and `order`, the
    /// first that applies.
    ///
    /// # Errors
    /// The refusal.
    pub fn parse(bytes: &[u8]) -> Result<Head, Refusal> {
        Self::read(bytes).map(|(h, _)| h)
    }

    /// The head and the bits after it.
    fn read(bytes: &[u8]) -> Result<(Head, Bits<'_>), Refusal> {
        let ch = *bytes.first().ok_or(Refusal::Short)?;
        let header = Header::from_byte(ch);
        if header.form != Form::Mutation || header.tag != Tag::NONE {
            return Err(Refusal::Form);
        }
        let run_bytes = if header.wide { 4 } else { 2 };
        // CH, the run, the seq, the catalog id and the flags.
        if bytes.len() < 1 + run_bytes + 3 + 4 + 1 {
            return Err(Refusal::Short);
        }
        let run = if header.wide {
            Run::U32(u32::from_be_bytes(bytes[1..5].try_into().expect("4 bytes")))
        } else {
            Run::U16(u16::from_be_bytes(bytes[1..3].try_into().expect("2 bytes")))
        };
        let at = 1 + run_bytes;
        let seq = Seq::new(
            (u32::from(bytes[at]) << 16)
                | (u32::from(bytes[at + 1]) << 8)
                | u32::from(bytes[at + 2]),
        )
        .expect("24 bits");
        let mut r = Bits::new(&bytes[at + 3..]);
        let id = u32::try_from(r.take(32)?).expect("32 bits");
        let catalog = CatalogId::from_hex(&format!("{id:08x}")).expect("8 hex digits");
        let count = r.take(4)?;
        if count == 0 {
            return Err(Refusal::Count);
        }
        let minted = r.bit()?;
        let has_zid = r.bit()?;
        if r.take(2)? != 0 {
            return Err(Refusal::Extension);
        }
        let mut entries: Vec<EntryBits> = Vec::new();
        for _ in 0..count {
            if r.left() < u64::from(ENTRY_BITS) {
                return Err(Refusal::Short);
            }
            let index = u16::try_from(r.take(INDEX_BITS)?).expect("10 bits");
            if entries.last().is_some_and(|e| index < e.index) {
                return Err(Refusal::Order);
            }
            entries.push(EntryBits {
                index,
                token: r.bit()?,
                standby: r.bit()?,
                has_unav: r.bit()?,
                has_card: r.bit()?,
            });
        }
        Ok((
            Head {
                stamp: Stamp { run, seq },
                catalog,
                minted,
                has_zid,
                entries,
            },
            r,
        ))
    }

    /// The pair each entry's index names, read through the catalogs held
    /// (§3.3 "Reading", step 2, without the revisions): what a reader
    /// retrieves before it reads the rest (core §8.4).
    ///
    /// # Errors
    /// `unknown_catalog`, then `unknown_index` for the first entry whose
    /// index the catalog read through does not hold.
    pub fn pairs<'h>(&self, held: &'h [Catalog]) -> Result<Vec<&'h catalog::Entry>, Refusal> {
        let through = catalog::through(held, self.catalog)?;
        self.entries
            .iter()
            .map(|e| through.entry(e.index).map_err(Refusal::from))
            .collect()
    }
}

/// Decodes a compact descriptor (core §3.3, "Decoding" and "Reading") that
/// came on the instance key of `addr` and `instance`, with the catalogs
/// and the revisions the reader holds.
///
/// # Errors
/// The first refusal in §3.3's order.
pub fn decode(
    bytes: &[u8],
    addr: &Addr,
    instance: &InstanceId,
    held: &[Catalog],
    revisions: &[&Contract],
) -> Result<Compact, Refusal> {
    // 1. The head.
    let (head, mut r) = Head::read(bytes)?;
    // 2. The catalog: the id, then each entry's index and revision.
    let through = catalog::through(held, head.catalog)?;
    let held_revisions = Held::new(revisions);
    let mut revs: Vec<(&catalog::Entry, &Contract)> = Vec::with_capacity(head.entries.len());
    for e in &head.entries {
        let pair = through.entry(e.index)?;
        let rev = held_revisions.of(pair).ok_or(Refusal::UnknownRevision)?;
        revs.push((pair, rev));
    }
    // 3. The rest, field by field.
    let mut bits: BTreeMap<String, bool> = BTreeMap::new();
    for (_, rev) in &revs {
        for cap in gated(rev) {
            let b = r.bit()?;
            if bits.get(&cap).is_some_and(|held| *held != b) {
                return Err(Refusal::Capability);
            }
            bits.insert(cap, b);
        }
    }
    let mut unavailable: Vec<Vec<Unavailable>> = vec![Vec::new(); revs.len()];
    for (i, (e, (_, rev))) in head.entries.iter().zip(&revs).enumerate() {
        if !e.has_unav {
            continue;
        }
        let names = resource_names(rev);
        let mut last = None;
        for _ in 0..list_count(&mut r)? {
            let k = item_resource(&mut r, rev, &mut last)?;
            let cause = cause_of(r.take(2)?).ok_or(Refusal::Cause)?;
            unavailable[i].push(Unavailable {
                resource: names[k].clone(),
                cause,
                reason: None,
            });
        }
    }
    let mut cardinality: Vec<BTreeMap<String, u64>> = vec![BTreeMap::new(); revs.len()];
    for (i, (e, (_, rev))) in head.entries.iter().zip(&revs).enumerate() {
        if !e.has_card {
            continue;
        }
        let names = resource_names(rev);
        let mut last = None;
        for _ in 0..list_count(&mut r)? {
            let k = item_resource(&mut r, rev, &mut last)?;
            let bound = r.integer()?;
            cardinality[i].insert(names[k].clone(), u64::from(bound));
        }
    }
    let zid = if head.has_zid {
        let hi = u128::from(r.take(64)?);
        let lo = u128::from(r.take(64)?);
        Some((hi << 64) | lo)
    } else {
        None
    };
    r.end()?;

    // The descriptor.
    let interfaces = head
        .entries
        .iter()
        .zip(&revs)
        .zip(unavailable.into_iter().zip(cardinality))
        .map(
            |((e, (pair, _)), (unavailable, cardinality))| InterfaceEntry {
                iface: pair.iface.to_string(),
                contract: pair.contract.to_string(),
                minor: None,
                standby: e.standby,
                token: e.token,
                unavailable,
                cardinality,
            },
        )
        .collect();
    let capabilities = bits
        .into_iter()
        .filter(|(_, held)| *held)
        .map(|(cap, _)| cap)
        .collect();
    let mut profiles: BTreeSet<IfaceId> = revs
        .iter()
        .flat_map(|(_, rev)| rev.uses.iter().cloned())
        .collect();
    if head.minted {
        profiles.insert(MINTED_PROFILE.parse().expect("an interface id"));
    }
    let mut meta = BTreeMap::new();
    if let Some(zid) = zid {
        meta.insert(
            "zid".to_owned(),
            serde_json::Value::String(format!("{zid:x}")),
        );
    }
    Ok(Compact {
        descriptor: Descriptor {
            format: FORMAT.to_owned(),
            service: addr.to_string(),
            instance: instance.to_string(),
            run: Some(head.stamp.run.hex()),
            catalog: Some(head.catalog.hex()),
            interfaces,
            capabilities,
            requires: Vec::new(),
            profiles: profiles.iter().map(ToString::to_string).collect(),
            meta,
        },
        seq: head.stamp.seq,
        minted: head.minted,
    })
}

/// A list's count of items: `count` at 0.
fn list_count(r: &mut Bits<'_>) -> Result<u64, Refusal> {
    match r.take(8)? {
        0 => Err(Refusal::Count),
        n => Ok(n),
    }
}

/// An item's resource: `resource` past its revision's, then `order` where
/// not above the one before it in its list.
fn item_resource(
    r: &mut Bits<'_>,
    rev: &Contract,
    last: &mut Option<u64>,
) -> Result<usize, Refusal> {
    let k = r.take(resource_bits(rev))?;
    if k >= rev.resources.len() as u64 {
        return Err(Refusal::Resource);
    }
    if last.is_some_and(|l| k <= l) {
        return Err(Refusal::Order);
    }
    *last = Some(k);
    Ok(usize::try_from(k).expect("below the count of resources"))
}

/// Writes a descriptor's compact form (core §3.3, "The layout" and
/// "Writing"), with the catalogs its owner holds and the revisions it
/// implements: the entries ordered by index (an index twice in the order
/// given), each list's items by resource, every bit of one capability
/// alike. Its `minor`, `profiles`, each `reason`, `meta` but its `zid`,
/// and a capability no listed revision gates on are left out.
///
/// # Errors
/// Why the descriptor has no compact form.
pub fn encode(
    c: &Compact,
    held: &[Catalog],
    revisions: &[&Contract],
) -> Result<Vec<u8>, EncodeError> {
    let d = &c.descriptor;
    let run = d.stated_run().ok_or(EncodeError::Run)?;
    let id = d.stated_catalog().ok_or(EncodeError::Catalog)?;
    let through = catalog::through(held, id).map_err(EncodeError::Unheld)?;
    if d.interfaces.is_empty() || d.interfaces.len() > MAX_ENTRIES {
        return Err(EncodeError::Count(d.interfaces.len()));
    }
    if !d.requires.is_empty() {
        return Err(EncodeError::Requires);
    }
    let zid = match d.meta.get("zid") {
        None => None,
        Some(v) => {
            let text = v.as_str().unwrap_or_default();
            let ok = (1..=32).contains(&text.len())
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !ok {
                return Err(EncodeError::Zid(v.to_string()));
            }
            Some(u128::from_str_radix(text, 16).expect("hex digits"))
        }
    };
    let held_revisions = Held::new(revisions);
    // Each entry's index and revision, then the entries by index.
    let mut entries = Vec::with_capacity(d.interfaces.len());
    for e in &d.interfaces {
        let iface: Option<IfaceId> = e.iface.parse().ok();
        let contract = Fingerprint::parse(&e.contract).ok();
        let index = iface.zip(contract).and_then(|(iface, contract)| {
            through.catalog().entries()[..usize::from(through.count())]
                .iter()
                .position(|x| x.iface == iface && x.contract == contract)
        });
        let Some(index) = index else {
            return Err(EncodeError::Uncatalogued {
                iface: e.iface.clone(),
                contract: e.contract.clone(),
            });
        };
        let pair = &through.catalog().entries()[index];
        let rev = held_revisions
            .of(pair)
            .ok_or_else(|| EncodeError::Revision {
                iface: e.iface.clone(),
                contract: e.contract.clone(),
            })?;
        entries.push((u16::try_from(index).expect("at most 1,024 entries"), e, rev));
    }
    entries.sort_by_key(|(index, _, _)| *index);

    let mut w = BitWriter::new();
    w.push(u64::from(u32::from_be_bytes(id.bytes())), 32);
    w.push(entries.len() as u64, 4);
    w.bit(c.minted);
    w.bit(zid.is_some());
    w.push(0, 2);
    for (index, e, _) in &entries {
        w.push(u64::from(*index), INDEX_BITS);
        w.bit(e.token);
        w.bit(e.standby);
        w.bit(!e.unavailable.is_empty());
        w.bit(!e.cardinality.is_empty());
    }
    for (_, _, rev) in &entries {
        for cap in gated(rev) {
            w.bit(d.capabilities.contains(&cap));
        }
    }
    for (_, e, rev) in &entries {
        if e.unavailable.is_empty() {
            continue;
        }
        let names = resource_names(rev);
        let mut items = Vec::with_capacity(e.unavailable.len());
        for u in &e.unavailable {
            let k = place(&names, &e.iface, &u.resource)?;
            items.push((k, cause_code(u.cause)));
        }
        items.sort_unstable();
        if let Some(w) = items.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(EncodeError::Twice {
                iface: e.iface.clone(),
                resource: names[w[0].0].clone(),
            });
        }
        list(&mut w, &e.iface, items.len())?;
        for (k, cause) in items {
            w.push(k as u64, resource_bits(rev));
            w.push(cause, 2);
        }
    }
    for (_, e, rev) in &entries {
        if e.cardinality.is_empty() {
            continue;
        }
        let names = resource_names(rev);
        let mut items = Vec::with_capacity(e.cardinality.len());
        for (name, bound) in &e.cardinality {
            let k = place(&names, &e.iface, name)?;
            let bound = u32::try_from(*bound).map_err(|_| EncodeError::Bound {
                iface: e.iface.clone(),
                bound: *bound,
            })?;
            items.push((k, bound));
        }
        items.sort_unstable();
        list(&mut w, &e.iface, items.len())?;
        for (k, bound) in items {
            w.push(k as u64, resource_bits(rev));
            let mut leb = Vec::with_capacity(5);
            write_integer(&mut leb, bound);
            for b in leb {
                w.push(u64::from(b), 8);
            }
        }
    }
    if let Some(zid) = zid {
        w.push((zid >> 64) as u64, 64);
        w.push(zid as u64, 64);
    }
    let body = w.finish();
    let stamp = Stamp { run, seq: c.seq };
    let header = Header {
        form: Form::Mutation,
        wide: run.is_wide(),
        tag: Tag::NONE,
    };
    let mut out = Vec::with_capacity(stamp.envelope_bytes() + body.bytes.len());
    out.push(header.byte());
    stamp.write_to(&mut out);
    out.extend_from_slice(&body.bytes);
    Ok(out)
}

/// A resource's place in its revision.
fn place(names: &[String], iface: &str, resource: &str) -> Result<usize, EncodeError> {
    names
        .iter()
        .position(|n| n == resource)
        .ok_or_else(|| EncodeError::Resource {
            iface: iface.to_owned(),
            resource: resource.to_owned(),
        })
}

/// A list's count, at most [`MAX_ITEMS`].
fn list(w: &mut BitWriter, iface: &str, items: usize) -> Result<(), EncodeError> {
    if items > MAX_ITEMS {
        return Err(EncodeError::Items {
            iface: iface.to_owned(),
            items,
        });
    }
    w.push(items as u64, 8);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(toml: &str) -> Contract {
        let l = crate::contract::load_str(toml, std::path::Path::new("."), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// ⌈log₂ n⌉ bits for n resources, none for one or none.
    #[test]
    fn a_resource_takes_the_fewest_bits_that_hold_every_index() {
        let res = |n: usize| -> String {
            (0..n)
                .map(|i| {
                    format!(
                        "[resources.r{i}]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n"
                    )
                })
                .collect()
        };
        for (n, bits) in [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
            (8, 3),
            (9, 4),
            (108, 7),
            (128, 7),
            (129, 8),
        ] {
            let c = contract(&format!(
                "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n{}",
                res(n)
            ));
            assert_eq!(resource_bits(&c), bits, "{n} resources");
        }
    }

    /// The writer's own refusals: no run, no catalog, a requirement, no
    /// interface, an uncatalogued revision, a resource that is none of the
    /// revision's, a bound above 2^32 − 1 and a malformed zid.
    #[test]
    fn a_descriptor_without_a_compact_form_is_refused_by_the_writer() {
        let c = contract(
            "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
             [resources.\"a/{x}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { x = \"string\" }\ncardinality = 9\n",
        );
        let mut catalog = Catalog::new(0).unwrap();
        catalog.push(c.iface.clone(), Fingerprint::of(&c)).unwrap();
        let held = [catalog];
        let fp = Fingerprint::of(&c).to_string();
        let base = Compact {
            descriptor: serde_json::from_str(&format!(
                r#"{{"format": "zk2-descriptor/0.1", "service": "s/t", "instance": "000000000000000a",
                    "run": "0001", "catalog": "{}", "interfaces": [{{"iface": "t.v1", "contract": "{fp}"}}]}}"#,
                held[0].id()
            ))
            .unwrap(),
            seq: Seq::default(),
            minted: false,
        };
        let bytes = encode(&base, &held, &[&c]).unwrap();
        let addr = Addr::new("s", "t").unwrap();
        let instance: InstanceId = "000000000000000a".parse().unwrap();
        assert_eq!(
            decode(&bytes, &addr, &instance, &held, &[&c]).unwrap(),
            base
        );

        let refused = |f: &dyn Fn(&mut Compact)| {
            let mut v = base.clone();
            f(&mut v);
            encode(&v, &held, &[&c]).unwrap_err()
        };
        assert_eq!(refused(&|v| v.descriptor.run = None), EncodeError::Run);
        assert_eq!(
            refused(&|v| v.descriptor.catalog = None),
            EncodeError::Catalog
        );
        assert!(matches!(
            refused(&|v| v.descriptor.catalog = Some("00000000".into())),
            EncodeError::Unheld(ReadRefusal::UnknownCatalog)
        ));
        assert_eq!(
            refused(&|v| v.descriptor.interfaces.clear()),
            EncodeError::Count(0)
        );
        assert_eq!(
            refused(&|v| {
                let e = v.descriptor.interfaces[0].clone();
                v.descriptor.interfaces = vec![e; 16];
            }),
            EncodeError::Count(16)
        );
        assert_eq!(
            refused(&|v| v.descriptor.requires =
                serde_json::from_str(r#"[{"role": "r", "interface": "u.v1", "bindings": []}]"#)
                    .unwrap()),
            EncodeError::Requires
        );
        assert!(matches!(
            refused(&|v| v.descriptor.interfaces[0].iface = "u.v1".into()),
            EncodeError::Uncatalogued { .. }
        ));
        assert!(matches!(
            encode(&base, &held, &[]).unwrap_err(),
            EncodeError::Revision { .. }
        ));
        assert!(matches!(
            refused(&|v| {
                v.descriptor.interfaces[0]
                    .cardinality
                    .insert("state/b/{x}".into(), 1);
            }),
            EncodeError::Resource { .. }
        ));
        assert!(matches!(
            refused(&|v| {
                v.descriptor.interfaces[0]
                    .cardinality
                    .insert("state/a/{x}".into(), 1 << 32);
            }),
            EncodeError::Bound { .. }
        ));
        for zid in ["", "0A", &"1".repeat(33), "x"] {
            assert!(matches!(
                refused(&|v| {
                    v.descriptor.meta.insert("zid".into(), zid.into());
                }),
                EncodeError::Zid(_)
            ));
        }
    }
}
