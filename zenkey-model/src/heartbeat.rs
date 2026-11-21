//! The heartbeat (core §7.5, form 6; #978's F1, epic #975): one put per
//! interface an owner serves that vouches, by a bit for each member, for
//! the state it holds, without putting the values again.
//!
//! ```text
//! heartbeat, a put's payload   CH · run · seq · body      CH: form 6 · run width · the revision's tag
//! body       time · map · a section for each map bit set, in map order · pad
//! time       42 bits: t32 · ms10 from the deployment epoch; all ones unknown
//! map        1 bit for each collection of the covered revision, in its order
//! section    digest32 · members
//! members    one member: vouched1
//!            templated:  live (⌈log₂(c + 1)⌉) · sel1 · (bitmap | count · ranks)
//! ```
//!
//! **The collections** ([`Layout::of`]) of a revision are its `state` and
//! `@state` resources whose resolved `freshness.ttl_s` is above 0, in the
//! canonical resource order (by kind token, then template). The map's
//! width and each templated section's widths come from them, so a body is
//! read with the covered revision ([`Heartbeat::parse`]); CH's tag, the
//! low 4 bits of the revision's fingerprint ([`Layout::tag`]), is the
//! check that the reader holds that revision.
//!
//! **The digest** ([`Digest`]) of a collection is the XOR over its live
//! members of the first 4 bytes of the sha256 of the key, a zero byte, the
//! run's width bit, the run and the seq. It is free of order, so an owner
//! keeps it at each mutation and a reader computes it over its own view.
//!
//! `freshness.v2` (#978's F3, core 0.82) adopts it: an owner of a
//! contract that uses it sends one heartbeat per interface it serves that
//! has a collection, on [`key`], at most half the shortest horizon apart
//! (v2 §2.4), and a reader confirms by it as [`crate::freshness::View`]
//! reads it (v2 §2.5, `spec/profiles/freshness/conformance/apply.json`).
//! The refusals' tokens are `spec/conformance/heartbeat/bodies.json`'s.
//!
//! **The owner heartbeat** (core 0.108; #1110's H1, adopted by no rule
//! yet) is form 6's second layout: one put for several interfaces an
//! owner serves, a [`Group`] for each.
//!
//! ```text
//! owner heartbeat, a put's payload   CH · run · seq · body      CH: form 6 · run width · tag 0
//! body       time · groups · that many groups · pad
//! groups     4 bits: the count of groups, from 1 to 15
//! group      index10 · tag4 · size · sections
//! size       unsigned LEB128, each byte the body's next 8 bits: the sections' count of bits
//! sections   map · a section for each map bit set, as a heartbeat's body between its time and its pad
//! ```
//!
//! It is read in two steps. [`OwnerHeartbeat::parse`] reads the groups
//! with no contract, each one's sections taken unread, and refuses the
//! whole put ([`OwnerRefusal`]). [`Group::entry`] then reads a group's
//! index through the catalog its owner names (core §8.6) and
//! [`Group::read`] its sections with that revision's [`Layout`], each
//! refusing that group alone ([`GroupRefusal`]). Nothing in the bytes
//! tells the two layouts apart: the caller says which one a key carries,
//! and calls [`Heartbeat::parse`] or [`OwnerHeartbeat::parse`]. The
//! tokens are `spec/conformance/heartbeat/owner.json`'s.

use std::time::Duration;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::canonical::Fingerprint;
use crate::catalog::{self, Catalog, CatalogId};
use crate::collection::{IntegerRefusal, read_integer, write_integer};
use crate::contract::Contract;
use crate::freshness::{Horizon, horizon_of};
use crate::grammar::{IfaceId, KindToken, ZkKey, parse};
use crate::packed::{BitReader, BitWriter, ReadRefusal, Record};
use crate::template::Template;
use crate::wire::{self, Run, Stamp, Tag};

/// The profile that adopts the heartbeat (#978's F3), as its key's
/// interface chunk spells it.
pub const PROFILE: &str = "freshness.v2";

/// The resource `freshness.v2`'s standard contract declares, its
/// parameter the covered interface's id (v2 §3).
pub const RESOURCE: &str = "heartbeat/{iface}";

/// The width of a heartbeat's time: 32 bits of seconds, then 10 of
/// milliseconds, as core §7.6's `time` field with `ms`.
pub const TIME_BITS: u32 = 42;

/// The base-relative key a heartbeat covering `iface` travels on from the
/// service at `system/service`: `zk2/<system>/<service>/freshness.v2/stream/heartbeat/<iface>`
/// (`freshness.v2` §3 declares it). An interface id
/// is a plain chunk, so it needs no slug.
#[must_use]
pub fn key(system: &str, service: &str, iface: &IfaceId) -> String {
    format!("zk2/{system}/{service}/{PROFILE}/stream/heartbeat/{iface}")
}

/// The covered interface a heartbeat key names, read from a key with a
/// namespace in front or none: `None` for any other key.
#[must_use]
pub fn covered(key: &str) -> Option<(crate::grammar::Addr, IfaceId)> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(ZkKey::Data {
                addr,
                iface,
                kind: KindToken::Stream,
                resource,
            }) = parse(k)
        {
            if iface.to_string() != PROFILE {
                return None;
            }
            let [h, covered] = resource.as_slice() else {
                return None;
            };
            if h != "heartbeat" {
                return None;
            }
            let (name, major) = covered.rsplit_once(".v")?;
            return Some((addr, IfaceId::new(name, major.parse().ok()?).ok()?));
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}

/// ⌈log₂ n⌉ for n ≥ 1, 0 for n ≤ 1: the fewest bits that hold n codes.
fn bits_for(n: u128) -> u32 {
    if n <= 1 {
        0
    } else {
        128 - (n - 1).leading_zeros()
    }
}

/// One collection of a revision: a `state` or `@state` resource whose
/// horizon is above 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub token: KindToken,
    pub template: Template,
    /// Its cardinality when its template has a parameter; `None` for one
    /// member.
    pub cardinality: Option<u64>,
    /// Its horizon, `freshness.ttl_s`.
    pub ttl: Duration,
}

impl Collection {
    /// `<kind token>/<template>`, as the fixtures name it.
    #[must_use]
    pub fn resource(&self) -> String {
        format!("{}/{}", self.token.as_str(), self.template.as_str())
    }

    /// Whether its template has a parameter.
    #[must_use]
    pub fn is_templated(&self) -> bool {
        self.cardinality.is_some()
    }

    /// The width of its live count, ⌈log₂(c + 1)⌉; `None` with one member.
    #[must_use]
    pub fn live_bits(&self) -> Option<u32> {
        self.cardinality.map(|c| bits_for(u128::from(c) + 1))
    }
}

/// A revision's collections, its fingerprint and the heartbeat's tag: what
/// a heartbeat covering it is read and written by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub iface: IfaceId,
    pub fingerprint: Fingerprint,
    /// The low 4 bits of the fingerprint's last byte.
    pub tag: Tag,
    /// In map order.
    pub collections: Vec<Collection>,
    /// Every `state` and `@state` template of the revision, each with its
    /// collection's index, so a key resolves among them all (§2.2).
    states: Vec<(KindToken, Template, Option<usize>)>,
}

impl Layout {
    /// The layout of a revision.
    #[must_use]
    pub fn of(c: &Contract) -> Layout {
        let fingerprint = Fingerprint::of(c);
        let mut collections = Vec::new();
        let mut states = Vec::new();
        for r in &c.resources {
            if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
                continue;
            }
            let index = match horizon_of(r) {
                Horizon::Within(ttl) => {
                    collections.push(Collection {
                        token: r.token,
                        template: r.template.clone(),
                        // A templated resource always states one (E013);
                        // none is read as no ceiling.
                        cardinality: r
                            .template
                            .has_params()
                            .then(|| r.cardinality.unwrap_or(u64::from(u32::MAX))),
                        ttl,
                    });
                    Some(collections.len() - 1)
                }
                _ => None,
            };
            states.push((r.token, r.template.clone(), index));
        }
        Layout {
            iface: c.iface.clone(),
            tag: Self::tag_of(&fingerprint),
            fingerprint,
            collections,
            states,
        }
    }

    /// A heartbeat's tag for a revision: the low 4 bits of its
    /// fingerprint's last byte, its last hex digit.
    #[must_use]
    pub fn tag_of(fp: &Fingerprint) -> Tag {
        let last = fp.hex().as_str().as_bytes()[63];
        let v = (last as char)
            .to_digit(16)
            .expect("a fingerprint is lowercase hex");
        Tag::new(v as u8).expect("one hex digit is a tag")
    }

    /// The shortest horizon among the collections, `None` with none.
    #[must_use]
    pub fn horizon(&self) -> Option<Duration> {
        self.collections.iter().map(|c| c.ttl).min()
    }

    /// The collection named `<kind token>/<template>`.
    #[must_use]
    pub fn index_of(&self, resource: &str) -> Option<usize> {
        self.collections
            .iter()
            .position(|c| c.resource() == resource)
    }

    /// The collection a member key belongs to: a base-relative state key
    /// (or one with a namespace in front) of this interface whose winning
    /// template (§2.2) is a collection's. `None` for any other key.
    #[must_use]
    pub fn collection_of(&self, key: &str) -> Option<usize> {
        let (token, chunks) = self.state_chunks(key)?;
        let candidates: Vec<&(KindToken, Template, Option<usize>)> =
            self.states.iter().filter(|(t, _, _)| *t == token).collect();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (i, _) = crate::template::resolve(candidates.iter().map(|(_, t, _)| t), &refs)?;
        candidates[i].2
    }

    fn state_chunks(&self, key: &str) -> Option<(KindToken, Vec<String>)> {
        let k = base_relative(key)?;
        match parse(k) {
            Ok(ZkKey::Data {
                iface,
                kind,
                resource,
                ..
            }) if iface == self.iface
                && matches!(kind, KindToken::State | KindToken::ExplicitState) =>
            {
                Some((kind, resource))
            }
            _ => None,
        }
    }
}

/// A key from its first `zk2/` chunk from which the grammar parses a zk2
/// key: a session in no namespace hears the namespace in front.
#[must_use]
pub fn base_relative(key: &str) -> Option<&str> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/") && parse(k).is_ok() {
            return Some(k);
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}

/// A collection's digest: the XOR of each live member's value, 0 with none
/// live. Toggling a member's value in or out keeps it in O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Digest(u32);

impl Digest {
    /// No member live.
    pub const EMPTY: Digest = Digest(0);

    /// A digest as a body carries it.
    #[must_use]
    pub const fn from_u32(v: u32) -> Digest {
        Digest(v)
    }

    /// Its value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// A live member's value: the first 4 bytes, big-endian, of the sha256
    /// of its base-relative key, a zero byte, its run's width bit (0 for a
    /// 16-bit run, 1 for a 32-bit run), then its run and seq as an envelope
    /// writes them.
    #[must_use]
    pub fn member(key: &str, stamp: Stamp) -> u32 {
        let mut h = Sha256::new();
        h.update(key.as_bytes());
        h.update([0, u8::from(stamp.run.is_wide())]);
        match stamp.run {
            Run::U16(v) => h.update(v.to_be_bytes()),
            Run::U32(v) => h.update(v.to_be_bytes()),
        }
        h.update(&stamp.seq.get().to_be_bytes()[1..]);
        let d = h.finalize();
        u32::from_be_bytes([d[0], d[1], d[2], d[3]])
    }

    /// The digest of these live members.
    #[must_use]
    pub fn of<'a>(members: impl IntoIterator<Item = (&'a str, Stamp)>) -> Digest {
        let mut d = Digest::EMPTY;
        for (k, s) in members {
            d.toggle(k, s);
        }
        d
    }

    /// A member's value in or out: a put adds the new stamp's and removes
    /// the old one's, a delete removes it.
    pub fn toggle(&mut self, key: &str, stamp: Stamp) {
        self.0 ^= Digest::member(key, stamp);
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

/// The owner's clock when it wrote the heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Time {
    /// All 42 bits set: the owner's clock is unknown.
    Unknown,
    /// Seconds from the deployment epoch, then milliseconds below 1000.
    At { secs: u32, ms: u16 },
}

impl Time {
    /// Its 42-bit code.
    #[must_use]
    pub fn code(self) -> u64 {
        match self {
            Time::Unknown => (1 << TIME_BITS) - 1,
            Time::At { secs, ms } => (u64::from(secs) << 10) | u64::from(ms),
        }
    }

    /// A 42-bit code read: `code` for a millisecond code from 1000 up but
    /// the unknown time's.
    ///
    /// # Errors
    /// [`BodyRefusal::Code`].
    pub fn from_code(code: u64) -> Result<Time, BodyRefusal> {
        if code == (1 << TIME_BITS) - 1 {
            return Ok(Time::Unknown);
        }
        let ms = (code & 0x3ff) as u16;
        if ms >= 1000 {
            return Err(BodyRefusal::Code);
        }
        Ok(Time::At {
            secs: (code >> 10) as u32,
            ms,
        })
    }

    /// An instant in milliseconds since the Unix epoch, under the
    /// deployment epoch `epoch` in seconds (core §7.6): `None` outside the
    /// epoch's 2^32 seconds.
    #[must_use]
    pub fn from_unix_ms(t: i128, epoch: i64) -> Option<Time> {
        let code = crate::packed::time_code(t, Some(epoch), true).ok()?;
        Time::from_code(code).ok()
    }

    /// The instant in milliseconds since the Unix epoch, `None` when
    /// unknown.
    #[must_use]
    pub fn unix_ms(self, epoch: i64) -> Option<i128> {
        match self {
            Time::Unknown => None,
            Time::At { .. } => crate::packed::time_value(self.code(), Some(epoch), true).ok(),
        }
    }
}

/// A section's members.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Members {
    /// A collection of one member: whether it is vouched for.
    One { vouched: bool },
    /// A templated collection's bitmap: one bit for each live member, in
    /// key order.
    Bitmap(Vec<bool>),
    /// A templated collection's list: its live count, and the ranks vouched
    /// for, ascending.
    List { live: u64, ranks: Vec<u64> },
}

impl Members {
    /// A templated collection's members as a writer writes them: the list
    /// only where it is shorter than the bitmap (core §7.5's MUST). The
    /// ranks are taken in any order, each once, below `live`.
    #[must_use]
    pub fn templated(live: u64, vouched: impl IntoIterator<Item = u64>) -> Members {
        let mut ranks: Vec<u64> = vouched.into_iter().filter(|r| *r < live).collect();
        ranks.sort_unstable();
        ranks.dedup();
        let list = u128::from(bits_for(u128::from(live) + 1))
            + ranks.len() as u128 * u128::from(bits_for(u128::from(live)));
        if list < u128::from(live) {
            Members::List { live, ranks }
        } else {
            let mut bits = vec![false; usize::try_from(live).unwrap_or(usize::MAX)];
            for r in ranks {
                bits[r as usize] = true;
            }
            Members::Bitmap(bits)
        }
    }

    /// The live count; `None` with one member.
    #[must_use]
    pub fn live(&self) -> Option<u64> {
        match self {
            Members::One { .. } => None,
            Members::Bitmap(b) => Some(b.len() as u64),
            Members::List { live, .. } => Some(*live),
        }
    }

    /// The ranks vouched for, ascending; `[0]` for one member vouched for.
    #[must_use]
    pub fn vouched(&self) -> Vec<u64> {
        match self {
            Members::One { vouched } => {
                if *vouched {
                    vec![0]
                } else {
                    vec![]
                }
            }
            Members::Bitmap(b) => (0..b.len() as u64).filter(|i| b[*i as usize]).collect(),
            Members::List { ranks, .. } => ranks.clone(),
        }
    }

    /// `bitmap` or `list` for a templated collection, as the fixtures
    /// name the form read; `None` with one member.
    #[must_use]
    pub fn form(&self) -> Option<&'static str> {
        match self {
            Members::One { .. } => None,
            Members::Bitmap(_) => Some("bitmap"),
            Members::List { .. } => Some("list"),
        }
    }
}

/// One collection's section.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Section {
    pub digest: Digest,
    pub members: Members,
}

/// A heartbeat's body, read: its time, and a section for each collection
/// whose map bit is set (`None` where it is 0), in map order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heartbeat {
    pub time: Time,
    pub sections: Vec<Option<Section>>,
}

/// Why a body is not a heartbeat of its layout, in core §7.5's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum BodyRefusal {
    /// The body ends before a bit its fields need.
    #[error("short: the body ends before a bit its fields need")]
    Short,
    /// A code is unused: a millisecond code from 1000 up but the unknown
    /// time's, a live count above the cardinality, a count above the live
    /// count, or a rank not below the live count or not above the one
    /// before it.
    #[error("code: an unused code")]
    Code,
    /// A byte after the pad.
    #[error("trailing: a byte after the pad")]
    Trailing,
    /// A bit of the pad is 1.
    #[error("pad: a bit of the pad is 1")]
    Pad,
}

impl BodyRefusal {
    /// The token `spec/conformance/heartbeat/bodies.json` pins.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            BodyRefusal::Short => "short",
            BodyRefusal::Code => "code",
            BodyRefusal::Trailing => "trailing",
            BodyRefusal::Pad => "pad",
        }
    }
}

/// Why a heartbeat value cannot be written under a layout.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WriteError {
    #[error("{have} sections for {need} collections")]
    Sections { need: usize, have: usize },
    #[error("collection {0}: members of the wrong shape for its template")]
    Shape(usize),
    #[error("collection {0}: a live count above its cardinality")]
    Live(usize),
    #[error("collection {0}: ranks not ascending below the live count")]
    Ranks(usize),
}

/// Bits read in order, `short` where they end: a whole body's, or the
/// bits of one group's sections, which end where its size says and not at
/// a byte.
struct Bits<'a> {
    r: BitReader<'a>,
    left: u64,
}

impl<'a> Bits<'a> {
    /// Every bit of `bytes`.
    fn whole(bytes: &'a [u8]) -> Self {
        Bits {
            r: BitReader::new(bytes),
            left: bytes.len() as u64 * 8,
        }
    }

    /// A record's bits, its pad left out.
    fn of(record: &'a Record) -> Self {
        Bits {
            r: BitReader::new(&record.bytes),
            left: record.bits.min(record.bytes.len() as u64 * 8),
        }
    }

    fn take(&mut self, width: u32) -> Result<u64, BodyRefusal> {
        if u64::from(width) > self.left {
            return Err(BodyRefusal::Short);
        }
        self.left -= u64::from(width);
        self.r.take(width).map_err(|_| BodyRefusal::Short)
    }

    /// The whole body's end: no byte after its pad, then every pad bit 0.
    fn end(self) -> Result<(), BodyRefusal> {
        self.r.end().map_err(|e| match e {
            ReadRefusal::Pad => BodyRefusal::Pad,
            _ => BodyRefusal::Trailing,
        })
    }
}

/// A body's map and sections under `layout`, field by field: what a
/// heartbeat's body holds between its time and its pad, and what a
/// group's sections are.
fn read_sections(r: &mut Bits<'_>, layout: &Layout) -> Result<Vec<Option<Section>>, BodyRefusal> {
    let mut map = Vec::with_capacity(layout.collections.len());
    for _ in &layout.collections {
        map.push(r.take(1)? == 1);
    }
    let mut sections = Vec::with_capacity(map.len());
    for (c, on) in layout.collections.iter().zip(map) {
        if !on {
            sections.push(None);
            continue;
        }
        let digest = Digest(r.take(32)? as u32);
        let members = match (c.cardinality, c.live_bits()) {
            (Some(card), Some(w)) => {
                let live = r.take(w)?;
                if live > card {
                    return Err(BodyRefusal::Code);
                }
                if r.take(1)? == 0 {
                    let mut bits = Vec::new();
                    for _ in 0..live {
                        bits.push(r.take(1)? == 1);
                    }
                    Members::Bitmap(bits)
                } else {
                    let count = r.take(bits_for(u128::from(live) + 1))?;
                    if count > live {
                        return Err(BodyRefusal::Code);
                    }
                    let rw = bits_for(u128::from(live));
                    let mut ranks: Vec<u64> = Vec::new();
                    for _ in 0..count {
                        let rank = r.take(rw)?;
                        if rank >= live || ranks.last().is_some_and(|p| rank <= *p) {
                            return Err(BodyRefusal::Code);
                        }
                        ranks.push(rank);
                    }
                    Members::List { live, ranks }
                }
            }
            _ => Members::One {
                vouched: r.take(1)? == 1,
            },
        };
        sections.push(Some(Section { digest, members }));
    }
    Ok(sections)
}

/// The map and sections under `layout`, written: [`read_sections`]'s
/// inverse.
fn write_sections(
    w: &mut BitWriter,
    layout: &Layout,
    sections: &[Option<Section>],
) -> Result<(), WriteError> {
    if sections.len() != layout.collections.len() {
        return Err(WriteError::Sections {
            need: layout.collections.len(),
            have: sections.len(),
        });
    }
    for s in sections {
        w.bit(s.is_some());
    }
    for (i, (c, s)) in layout.collections.iter().zip(sections).enumerate() {
        let Some(s) = s else { continue };
        w.push(u64::from(s.digest.0), 32);
        match (&s.members, c.cardinality, c.live_bits()) {
            (Members::One { vouched }, None, _) => w.bit(*vouched),
            (Members::Bitmap(bits), Some(card), Some(lw)) => {
                let live = bits.len() as u64;
                if live > card {
                    return Err(WriteError::Live(i));
                }
                w.push(live, lw);
                w.bit(false);
                for b in bits {
                    w.bit(*b);
                }
            }
            (Members::List { live, ranks }, Some(card), Some(lw)) => {
                if *live > card {
                    return Err(WriteError::Live(i));
                }
                if ranks.windows(2).any(|p| p[0] >= p[1]) || ranks.iter().any(|r| r >= live) {
                    return Err(WriteError::Ranks(i));
                }
                w.push(*live, lw);
                w.bit(true);
                w.push(ranks.len() as u64, bits_for(u128::from(*live) + 1));
                let rw = bits_for(u128::from(*live));
                for r in ranks {
                    w.push(*r, rw);
                }
            }
            _ => return Err(WriteError::Shape(i)),
        }
    }
    Ok(())
}

impl Heartbeat {
    /// A body read with its covered revision's layout, or the first
    /// refusal that applies (core §7.5): the fields in order (`short`,
    /// `code`), then a byte after the pad (`trailing`), then a pad bit set
    /// (`pad`). CH's tag is the caller's to compare with
    /// [`Layout::tag`].
    ///
    /// # Errors
    /// The [`BodyRefusal`].
    pub fn parse(body: &[u8], layout: &Layout) -> Result<Heartbeat, BodyRefusal> {
        let mut r = Bits::whole(body);
        let time = Time::from_code(r.take(TIME_BITS)?)?;
        let sections = read_sections(&mut r, layout)?;
        r.end()?;
        Ok(Heartbeat { time, sections })
    }

    /// The body's bits and pad under `layout` ([`crate::packed::Record`]).
    ///
    /// # Errors
    /// A [`WriteError`]: a section per collection, each of its
    /// collection's shape, a live count within the cardinality, and ranks
    /// ascending below it.
    pub fn record(&self, layout: &Layout) -> Result<crate::packed::Record, WriteError> {
        let mut w = BitWriter::new();
        w.push(self.time.code(), TIME_BITS);
        write_sections(&mut w, layout, &self.sections)?;
        Ok(w.finish())
    }

    /// The body's bytes under `layout`, its pad included.
    ///
    /// # Errors
    /// As [`Heartbeat::record`].
    pub fn write(&self, layout: &Layout) -> Result<Vec<u8>, WriteError> {
        self.record(layout).map(|r| r.bytes)
    }

    /// The heartbeat as a put's payload: CH (form 6, the run's width, the
    /// layout's tag), the stamp, then the body.
    ///
    /// # Errors
    /// As [`Heartbeat::record`].
    pub fn envelope(&self, layout: &Layout, stamp: Stamp) -> Result<Vec<u8>, WriteError> {
        let body = self.write(layout)?;
        Ok(wire::Envelope {
            form: wire::Form::Heartbeat,
            tag: layout.tag,
            stamp,
            record: &body,
        }
        .write())
    }
}

/// The width of an owner heartbeat's count of groups.
pub const GROUPS_BITS: u32 = 4;

/// The most groups an owner heartbeat holds: its count is 4 bits, and 0 is
/// unused.
pub const MAX_GROUPS: usize = 15;

/// The width of a group's index: a deployment catalog holds at most 1,024
/// entries (core §8.6).
pub const INDEX_BITS: u32 = 10;

/// The highest index a group names.
pub const MAX_INDEX: u16 = (1 << INDEX_BITS) - 1;

/// One covered interface's part of an owner heartbeat (core §7.5): the
/// covered revision's index in the catalog its owner names, the tag the
/// owner wrote with, and the group's sections, unread. Their count of bits
/// is the group's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The covered revision's index in the deployment catalog, at most
    /// [`MAX_INDEX`].
    pub index: u16,
    /// The covered revision's tag, as a heartbeat's CH carries it.
    pub tag: Tag,
    /// The sections' bits: a map bit for each collection of the covered
    /// revision, then a section for each map bit set. A group has no pad
    /// of its own, so `sections.bits` is its size whatever the bytes hold
    /// after them.
    pub sections: Record,
}

/// Why one group of an owner heartbeat is not read, in core §7.5's order.
/// It refuses that group alone: every other group reads as it would with
/// this one left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum GroupRefusal {
    /// The reader holds neither the catalog its owner names nor one whose
    /// lineage holds it, or the owner names none (core §8.6).
    #[error("unknown_catalog: no catalog held is or succeeds the one its owner names")]
    UnknownCatalog,
    /// The index is at or above the count it is read through (core §8.6).
    #[error("unknown_index: the index is at or above the count it is read through")]
    UnknownIndex,
    /// The sections end before a bit their fields need.
    #[error("short: the sections end before a bit their fields need")]
    Short,
    /// A code is unused: a live count above the cardinality, a count above
    /// the live count, or a rank not below the live count or not above the
    /// one before it.
    #[error("code: an unused code")]
    Code,
    /// A bit after the sections' last field.
    #[error("trailing: a bit after the sections' last field")]
    Trailing,
}

impl GroupRefusal {
    /// The token `spec/conformance/heartbeat/owner.json` pins.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            GroupRefusal::UnknownCatalog => "unknown_catalog",
            GroupRefusal::UnknownIndex => "unknown_index",
            GroupRefusal::Short => "short",
            GroupRefusal::Code => "code",
            GroupRefusal::Trailing => "trailing",
        }
    }
}

impl From<catalog::ReadRefusal> for GroupRefusal {
    fn from(r: catalog::ReadRefusal) -> Self {
        match r {
            catalog::ReadRefusal::UnknownCatalog => GroupRefusal::UnknownCatalog,
            catalog::ReadRefusal::UnknownIndex => GroupRefusal::UnknownIndex,
        }
    }
}

impl Group {
    /// A group covering `layout`'s revision at `index`: its tag the
    /// layout's, its sections these, one for each collection (`None` for a
    /// map bit of 0).
    ///
    /// # Errors
    /// A [`WriteError`], as [`Heartbeat::record`].
    pub fn of(
        index: u16,
        layout: &Layout,
        sections: &[Option<Section>],
    ) -> Result<Group, WriteError> {
        let mut w = BitWriter::new();
        write_sections(&mut w, layout, sections)?;
        Ok(Group {
            index,
            tag: layout.tag,
            sections: w.finish(),
        })
    }

    /// The group's size: the count of bits in its sections.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.sections.bits
    }

    /// The revision the group's index names, read through the catalog its
    /// owner names among those the reader holds (core §8.6,
    /// [`catalog::through`]). A reader then reads the sections with that
    /// revision's layout ([`Group::read`]), or skips the group where it
    /// does not hold the revision.
    ///
    /// # Errors
    /// [`GroupRefusal::UnknownCatalog`] when the owner names no catalog,
    /// or no catalog held is or succeeds the one it names;
    /// [`GroupRefusal::UnknownIndex`] at or above the count read through.
    pub fn entry<'c>(
        &self,
        held: &'c [Catalog],
        named: Option<CatalogId>,
    ) -> Result<&'c catalog::Entry, GroupRefusal> {
        let named = named.ok_or(GroupRefusal::UnknownCatalog)?;
        Ok(catalog::through(held, named)?.entry(self.index)?)
    }

    /// Whether the group's tag is `layout`'s. It is a check, as a
    /// heartbeat's is: another tag refuses nothing here, the sections read
    /// the same, and what a reader concludes is `freshness.v2`'s to say.
    #[must_use]
    pub fn tag_is(&self, layout: &Layout) -> bool {
        self.tag == layout.tag
    }

    /// The group's sections read with its covered revision's layout, one
    /// for each collection (`None` for a map bit of 0), or the first
    /// refusal that applies (core §7.5): the fields in order (`short`,
    /// `code`), then a bit after the last field (`trailing`). The index is
    /// read first, by [`Group::entry`].
    ///
    /// # Errors
    /// [`GroupRefusal::Short`], [`GroupRefusal::Code`] or
    /// [`GroupRefusal::Trailing`].
    pub fn read(&self, layout: &Layout) -> Result<Vec<Option<Section>>, GroupRefusal> {
        let mut r = Bits::of(&self.sections);
        // The fields refuse `short` or `code` alone: a group's bits have no
        // pad, and what follows their last field is judged below.
        let sections = read_sections(&mut r, layout).map_err(|e| match e {
            BodyRefusal::Code => GroupRefusal::Code,
            _ => GroupRefusal::Short,
        })?;
        if r.left > 0 {
            return Err(GroupRefusal::Trailing);
        }
        Ok(sections)
    }
}

/// An owner heartbeat's body, read without any contract: its time and its
/// groups, in ascending order of their indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerHeartbeat {
    pub time: Time,
    pub groups: Vec<Group>,
}

/// Why a put on an owner heartbeat's key is not an owner heartbeat, in
/// core §7.5's order. It refuses the whole put, every group with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum OwnerRefusal {
    /// CH's tag is not 0.
    #[error("tag: CH's tag is not 0")]
    Tag,
    /// The body ends before a bit its fields need, a size's or a group's
    /// sections' among them.
    #[error("short: the body ends before a bit its fields need")]
    Short,
    /// A code is unused: a millisecond code from 1000 up but the unknown
    /// time's, or a count of 0 groups.
    #[error("code: an unused code")]
    Code,
    /// A group's index is not above the index of the group before it.
    #[error("order: a group's index is not above the one before it")]
    Order,
    /// A size's fifth byte has its high bit set, it is above 2^32 − 1, or
    /// it is not in its shortest form.
    #[error("leb128: a size that is not LEB128 in its shortest form, at most 2^32 − 1")]
    Leb128,
    /// A byte after the pad.
    #[error("trailing: a byte after the pad")]
    Trailing,
    /// A bit of the pad is 1.
    #[error("pad: a bit of the pad is 1")]
    Pad,
}

impl OwnerRefusal {
    /// The token `spec/conformance/heartbeat/owner.json` pins.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            OwnerRefusal::Tag => "tag",
            OwnerRefusal::Short => "short",
            OwnerRefusal::Code => "code",
            OwnerRefusal::Order => "order",
            OwnerRefusal::Leb128 => "leb128",
            OwnerRefusal::Trailing => "trailing",
            OwnerRefusal::Pad => "pad",
        }
    }
}

impl From<BodyRefusal> for OwnerRefusal {
    fn from(r: BodyRefusal) -> Self {
        match r {
            BodyRefusal::Short => OwnerRefusal::Short,
            BodyRefusal::Code => OwnerRefusal::Code,
            BodyRefusal::Trailing => OwnerRefusal::Trailing,
            BodyRefusal::Pad => OwnerRefusal::Pad,
        }
    }
}

/// Why an owner heartbeat cannot be written (core §7.5's writer's rule).
/// A group is named by its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OwnerWriteError {
    /// No group, or more than [`MAX_GROUPS`].
    #[error("{0} groups: an owner heartbeat holds from 1 to 15")]
    Groups(usize),
    /// A group's index is above [`MAX_INDEX`].
    #[error("group {0}: an index above 1023")]
    Index(usize),
    /// A group's index is not above the index of the group before it.
    #[error("group {0}: its index is not above the one before it")]
    Order(usize),
    /// A group's sections are more bits than their bytes hold, or more
    /// than 2^32 − 1.
    #[error("group {0}: its sections are not a size's count of bits")]
    Size(usize),
}

impl OwnerHeartbeat {
    /// A put's body on an owner heartbeat's key, after the envelope's
    /// parse gave CH's `tag` and the stamp: the groups read with no
    /// contract and no catalog, each one's sections taken unread, or the
    /// first refusal that applies (core §7.5): CH's tag not 0 (`tag`),
    /// then the fields in order, each read whole and then judged (`short`,
    /// `code`, `order`, `leb128`), then a byte after the pad (`trailing`),
    /// then a pad bit set (`pad`).
    ///
    /// The caller knows the key carries this layout: a heartbeat's body
    /// can read as an owner heartbeat's, and the reverse.
    ///
    /// # Errors
    /// The [`OwnerRefusal`].
    pub fn parse(tag: Tag, body: &[u8]) -> Result<OwnerHeartbeat, OwnerRefusal> {
        if tag != Tag::NONE {
            return Err(OwnerRefusal::Tag);
        }
        let mut r = Bits::whole(body);
        let time = Time::from_code(r.take(TIME_BITS)?)?;
        let count = r.take(GROUPS_BITS)?;
        if count == 0 {
            return Err(OwnerRefusal::Code);
        }
        let mut groups: Vec<Group> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let index = r.take(INDEX_BITS)? as u16;
            if groups.last().is_some_and(|g| index <= g.index) {
                return Err(OwnerRefusal::Order);
            }
            let tag = Tag::new(r.take(4)? as u8).expect("four bits are a tag");
            let size = read_integer(|| r.take(8).ok().map(|b| b as u8)).map_err(|e| match e {
                IntegerRefusal::Short => OwnerRefusal::Short,
                IntegerRefusal::Leb128 => OwnerRefusal::Leb128,
            })?;
            // Never allocated from the size: the bits are copied as the
            // body gives them, and it ends where it ends.
            let mut sections = BitWriter::new();
            let mut left = u64::from(size);
            while left > 0 {
                let n = left.min(64) as u32;
                sections.push(r.take(n)?, n);
                left -= u64::from(n);
            }
            groups.push(Group {
                index,
                tag,
                sections: sections.finish(),
            });
        }
        r.end()?;
        Ok(OwnerHeartbeat { time, groups })
    }

    /// The group of an index, `None` where the owner heartbeat holds none:
    /// how a reader bound to one interface finds its own behind the
    /// others'.
    #[must_use]
    pub fn group(&self, index: u16) -> Option<&Group> {
        self.groups.iter().find(|g| g.index == index)
    }

    /// The body's bits and pad ([`Record`]).
    ///
    /// # Errors
    /// An [`OwnerWriteError`]: from 1 to 15 groups, in ascending order of
    /// their indices, none above 1023, each one's size the count of bits
    /// in its sections.
    pub fn record(&self) -> Result<Record, OwnerWriteError> {
        if self.groups.is_empty() || self.groups.len() > MAX_GROUPS {
            return Err(OwnerWriteError::Groups(self.groups.len()));
        }
        let mut w = BitWriter::new();
        w.push(self.time.code(), TIME_BITS);
        w.push(self.groups.len() as u64, GROUPS_BITS);
        let mut size = Vec::with_capacity(5);
        for (i, g) in self.groups.iter().enumerate() {
            if g.index > MAX_INDEX {
                return Err(OwnerWriteError::Index(i));
            }
            if i > 0 && g.index <= self.groups[i - 1].index {
                return Err(OwnerWriteError::Order(i));
            }
            let bits = u32::try_from(g.sections.bits).map_err(|_| OwnerWriteError::Size(i))?;
            let mut r = Bits::of(&g.sections);
            if r.left != g.sections.bits {
                return Err(OwnerWriteError::Size(i));
            }
            w.push(u64::from(g.index), INDEX_BITS);
            w.push(u64::from(g.tag.get()), 4);
            size.clear();
            write_integer(&mut size, bits);
            for b in &size {
                w.push(u64::from(*b), 8);
            }
            while r.left > 0 {
                let n = r.left.min(64) as u32;
                let v = r.take(n).map_err(|_| OwnerWriteError::Size(i))?;
                w.push(v, n);
            }
        }
        Ok(w.finish())
    }

    /// The body's bytes, its pad included.
    ///
    /// # Errors
    /// As [`OwnerHeartbeat::record`].
    pub fn write(&self) -> Result<Vec<u8>, OwnerWriteError> {
        self.record().map(|r| r.bytes)
    }

    /// The owner heartbeat as a put's payload: CH (form 6, the run's
    /// width, tag 0), the stamp, then the body.
    ///
    /// # Errors
    /// As [`OwnerHeartbeat::record`].
    pub fn envelope(&self, stamp: Stamp) -> Result<Vec<u8>, OwnerWriteError> {
        let body = self.write()?;
        Ok(wire::Envelope {
            form: wire::Form::Heartbeat,
            tag: Tag::NONE,
            stamp,
            record: &body,
        }
        .write())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Seq;

    fn stamp(run: u16, seq: u32) -> Stamp {
        Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        }
    }

    #[test]
    fn widths() {
        assert_eq!(bits_for(1), 0);
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(3), 2);
        assert_eq!(bits_for(6), 3);
        assert_eq!(bits_for(1 << 32), 32);
        assert_eq!(bits_for((1 << 32) + 1), 33);
    }

    #[test]
    fn the_writer_writes_the_list_only_where_it_is_shorter() {
        assert!(matches!(Members::templated(3, [0, 2]), Members::Bitmap(_)));
        assert!(matches!(Members::templated(5, []), Members::List { .. }));
        // A tie keeps the bitmap: 1 bit against 1 + 0.
        assert!(matches!(Members::templated(1, [0]), Members::Bitmap(_)));
        assert_eq!(Members::templated(40, [17, 17, 99]).vouched(), vec![17]);
    }

    #[test]
    fn the_digest_is_free_of_order_and_toggles() {
        let a = ("zk2/s/v/i.v1/state/p/a", stamp(1, 1));
        let b = ("zk2/s/v/i.v1/state/p/b", stamp(1, 2));
        assert_eq!(Digest::of([a, b]), Digest::of([b, a]));
        let mut d = Digest::of([a, b]);
        d.toggle(b.0, b.1);
        assert_eq!(d, Digest::of([a]));
        d.toggle(a.0, a.1);
        assert_eq!(d, Digest::EMPTY);
    }

    #[test]
    fn a_heartbeat_key_names_its_covered_interface() {
        let iface = IfaceId::new("modem", 3).unwrap();
        let k = key("h-1", "rf0", &iface);
        assert_eq!(k, "zk2/h-1/rf0/freshness.v2/stream/heartbeat/modem.v3");
        let (addr, got) = covered(&k).unwrap();
        assert_eq!(
            (addr.to_string(), got),
            ("h-1/rf0".to_owned(), iface.clone())
        );
        assert_eq!(covered(&format!("prod/{k}")).map(|c| c.1), Some(iface));
        assert_eq!(covered("zk2/h/s/health.v1/stream/heartbeat/x.v1"), None);
        assert_eq!(covered("zk2/h/s/freshness.v2/state/heartbeat/x.v1"), None);
        assert_eq!(covered("demo/tick"), None);
    }

    /// A string of `0` and `1` as a record's bits.
    fn bits(s: &str) -> Record {
        let mut w = BitWriter::new();
        for c in s.chars() {
            w.bit(c == '1');
        }
        w.finish()
    }

    fn group(index: u16, sections: &str) -> Group {
        Group {
            index,
            tag: Tag::new((index % 16) as u8).unwrap(),
            sections: bits(sections),
        }
    }

    fn owner(groups: Vec<Group>) -> OwnerHeartbeat {
        OwnerHeartbeat {
            time: Time::At {
                secs: 3600,
                ms: 250,
            },
            groups,
        }
    }

    /// A revision with a templated collection of at most 5 and a
    /// collection of one member.
    fn layout() -> Layout {
        let text = r#"
[interface]
name  = "pair"
major = 1
minor = 0
uses  = ["freshness.v2"]

[resources."peer/{peer}"]
kind        = "state"
type        = { raw = "text/plain" }
params      = { peer = "string" }
cardinality = 5
annotations = { "freshness.ttl_s" = 60 }

[resources.status]
kind        = "state"
type        = { raw = "text/plain" }
annotations = { "freshness.ttl_s" = 60 }
"#;
        let loaded = crate::contract::load_str(text, std::path::Path::new("."), None);
        Layout::of(&loaded.contract.expect("the contract loads"))
    }

    #[test]
    fn an_owner_heartbeat_holds_from_1_to_15_groups() {
        assert_eq!(owner(vec![]).record(), Err(OwnerWriteError::Groups(0)));
        let many = |n: u16| owner((0..n).map(|i| group(i, "1")).collect());
        assert!(many(1).record().is_ok());
        assert!(many(15).record().is_ok());
        assert_eq!(many(16).record(), Err(OwnerWriteError::Groups(16)));
        assert_eq!(
            many(16).envelope(stamp(7, 1)),
            Err(OwnerWriteError::Groups(16))
        );
    }

    #[test]
    fn a_writer_refuses_indices_that_do_not_ascend() {
        let of = |indices: &[u16]| owner(indices.iter().map(|i| group(*i, "10")).collect());
        assert!(of(&[0, 2, 1023]).record().is_ok());
        // Descending, and the same index twice: the group at fault named
        // by its position.
        assert_eq!(of(&[4, 2]).record(), Err(OwnerWriteError::Order(1)));
        assert_eq!(of(&[2, 4, 4]).record(), Err(OwnerWriteError::Order(2)));
        assert_eq!(of(&[0, 9, 3, 12]).record(), Err(OwnerWriteError::Order(2)));
        // An index 10 bits cannot hold is refused, never written modulo
        // 1,024.
        assert_eq!(of(&[1024]).record(), Err(OwnerWriteError::Index(0)));
        assert_eq!(of(&[5, 2000]).record(), Err(OwnerWriteError::Index(1)));
    }

    #[test]
    fn a_writer_refuses_sections_that_are_not_their_bytes_bits() {
        let mut g = group(3, "1011");
        g.sections.bits = 9;
        assert_eq!(owner(vec![g]).record(), Err(OwnerWriteError::Size(0)));
        // Bits set after the sections' count are no part of the group.
        let mut g = group(3, "1011");
        g.sections.bytes[0] |= 0x0f;
        let written = owner(vec![g]).write().unwrap();
        assert_eq!(written, owner(vec![group(3, "1011")]).write().unwrap());
    }

    #[test]
    fn a_size_is_the_sections_count_of_bits_wherever_it_falls() {
        // 46 bits of time and count, 14 of index and tag: the size starts
        // at bit 60, on no byte boundary.
        let body = owner(vec![group(4, &"1".repeat(128))]).write().unwrap();
        // 128 is 0x80 0x01, the low seven bits first.
        assert_eq!(body[7] & 0x0f, 0x8);
        assert_eq!(body[8], 0x00);
        assert_eq!(body[9] >> 4, 0x1);
        let read = OwnerHeartbeat::parse(Tag::NONE, &body).unwrap();
        assert_eq!(read.groups[0].size(), 128);
        assert_eq!(read.group(4), Some(&group(4, &"1".repeat(128))));
        assert_eq!(read.group(5), None);
    }

    /// A xorshift generator: the same owner heartbeats on every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn every_owner_heartbeat_written_reads_back_and_no_prefix_of_it_does() {
        let mut rng = Rng(0x1110_0108_2026_1121);
        for round in 0..300 {
            let count = 1 + rng.below(15);
            let mut indices: Vec<u16> = (0..count).map(|_| rng.below(1024) as u16).collect();
            indices.sort_unstable();
            indices.dedup();
            let groups: Vec<Group> = indices
                .iter()
                .map(|i| {
                    // Sizes on both sides of each LEB128 length: 0, 127,
                    // 128 and 16,384 among them.
                    let len = match rng.below(8) {
                        0 => 0,
                        1 => 127 + rng.below(3),
                        2 => 16_383 + rng.below(3),
                        _ => rng.below(200),
                    };
                    let s: String = (0..len)
                        .map(|_| if rng.below(2) == 1 { '1' } else { '0' })
                        .collect();
                    Group {
                        index: *i,
                        tag: Tag::new(rng.below(16) as u8).unwrap(),
                        sections: bits(&s),
                    }
                })
                .collect();
            let time = if rng.below(10) == 0 {
                Time::Unknown
            } else {
                Time::At {
                    secs: rng.next() as u32,
                    ms: rng.below(1000) as u16,
                }
            };
            let hb = OwnerHeartbeat { time, groups };
            let record = hb.record().unwrap();
            let expected: u64 = 46
                + hb.groups
                    .iter()
                    .map(|g| {
                        let size = crate::collection::integer_bytes(g.size() as u32) as u64;
                        14 + 8 * size + g.size()
                    })
                    .sum::<u64>();
            assert_eq!(record.bits, expected, "round {round}: the body's bits");
            assert!(record.pad() < 8);
            assert_eq!(
                OwnerHeartbeat::parse(Tag::NONE, &record.bytes),
                Ok(hb.clone()),
                "round {round}: reads back"
            );
            assert_eq!(
                OwnerHeartbeat::parse(Tag::new(1).unwrap(), &record.bytes),
                Err(OwnerRefusal::Tag)
            );
            // A body cut anywhere lacks a bit a field needs: the pad is
            // under a byte, so the last byte holds one.
            for cut in [0, 1, 5, 6, record.bytes.len() / 2, record.bytes.len() - 1] {
                assert_eq!(
                    OwnerHeartbeat::parse(Tag::NONE, &record.bytes[..cut]),
                    Err(OwnerRefusal::Short),
                    "round {round}: cut at {cut}"
                );
            }
            let payload = hb.envelope(stamp(7, round)).unwrap();
            let env = wire::Envelope::parse(&payload).unwrap();
            assert!(env.is_heartbeat());
            assert_eq!((env.tag, env.stamp), (Tag::NONE, stamp(7, round)));
            assert_eq!(OwnerHeartbeat::parse(env.tag, env.record), Ok(hb));
        }
    }

    #[test]
    fn a_group_reads_and_writes_its_sections_by_its_revisions_layout() {
        let l = layout();
        let sections = vec![
            Some(Section {
                digest: Digest::from_u32(0xdead_beef),
                members: Members::templated(3, [0, 2]),
            }),
            Some(Section {
                digest: Digest::from_u32(0x0bad_cafe),
                members: Members::One { vouched: true },
            }),
        ];
        let g = Group::of(9, &l, &sections).unwrap();
        // The map's 2 bits; 32 + 3 + 1 + 3 for the bitmap; 32 + 1.
        assert_eq!(g.size(), 2 + 39 + 33);
        assert!(g.tag_is(&l));
        assert_eq!(g.read(&l), Ok(sections.clone()));
        // A heartbeat's body between its time and its pad is the same
        // bits.
        let body = Heartbeat {
            time: Time::Unknown,
            sections: sections.clone(),
        }
        .record(&l)
        .unwrap();
        assert_eq!(
            body.bit_string()[TIME_BITS as usize..],
            g.sections.bit_string()
        );
        // Every map bit 0 is the map alone, and a section the layout has
        // no collection for is the writer's refusal.
        let none = Group::of(9, &l, &[None, None]).unwrap();
        assert_eq!((none.size(), none.read(&l)), (2, Ok(vec![None, None])));
        assert_eq!(
            Group::of(9, &l, &sections[..1]),
            Err(WriteError::Sections { need: 2, have: 1 })
        );
    }

    #[test]
    fn a_refused_group_leaves_the_others_read() {
        let l = layout();
        let one = [
            None,
            Some(Section {
                digest: Digest::from_u32(1),
                members: Members::One { vouched: true },
            }),
        ];
        let good = |index| Group::of(index, &l, &one).unwrap();
        let mut long = good(5);
        long.sections = bits(&(long.sections.bit_string() + "0"));
        let mut cut = good(6);
        cut.sections = bits(&cut.sections.bit_string()[..20]);
        // Six live where the cardinality is 5.
        let over = Group {
            index: 7,
            tag: l.tag,
            sections: bits(&format!("10{}110", "0".repeat(32))),
        };
        let hb = owner(vec![good(2), long, cut, over, good(8)]);
        let read = OwnerHeartbeat::parse(Tag::NONE, &hb.write().unwrap()).unwrap();
        let got: Vec<_> = read.groups.iter().map(|g| g.read(&l)).collect();
        assert_eq!(
            got,
            [
                Ok(one.to_vec()),
                Err(GroupRefusal::Trailing),
                Err(GroupRefusal::Short),
                Err(GroupRefusal::Code),
                Ok(one.to_vec()),
            ]
        );
    }

    #[test]
    fn a_group_reads_its_index_through_the_catalog_its_owner_names() {
        let l = layout();
        let mut first = Catalog::new(1_767_225_600).unwrap();
        first.push(l.iface.clone(), l.fingerprint.clone()).unwrap();
        let named = Some(first.id());
        let held = [first];
        let g = Group::of(0, &l, &[None, None]).unwrap();
        let entry = g.entry(&held, named).unwrap();
        assert_eq!((&entry.iface, &entry.contract), (&l.iface, &l.fingerprint));
        let mut past = g.clone();
        past.index = 1;
        assert_eq!(past.entry(&held, named), Err(GroupRefusal::UnknownIndex));
        // No catalog named, none held, and another's id: never a guess.
        assert_eq!(g.entry(&held, None), Err(GroupRefusal::UnknownCatalog));
        assert_eq!(g.entry(&[], named), Err(GroupRefusal::UnknownCatalog));
        let other = CatalogId::from_hex("00000000");
        assert!(other.is_some());
        assert_eq!(g.entry(&held, other), Err(GroupRefusal::UnknownCatalog));
    }
}
