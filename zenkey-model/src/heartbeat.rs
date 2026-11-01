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

use std::time::Duration;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::canonical::Fingerprint;
use crate::contract::Contract;
use crate::freshness::{Horizon, horizon_of};
use crate::grammar::{IfaceId, KindToken, ZkKey, parse};
use crate::packed::{BitReader, BitWriter, ReadRefusal};
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

fn take(r: &mut BitReader<'_>, width: u32) -> Result<u64, BodyRefusal> {
    r.take(width).map_err(|_| BodyRefusal::Short)
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
        let mut r = BitReader::new(body);
        let time = Time::from_code(take(&mut r, TIME_BITS)?)?;
        let mut map = Vec::with_capacity(layout.collections.len());
        for _ in &layout.collections {
            map.push(take(&mut r, 1)? == 1);
        }
        let mut sections = Vec::with_capacity(map.len());
        for (c, on) in layout.collections.iter().zip(map) {
            if !on {
                sections.push(None);
                continue;
            }
            let digest = Digest(take(&mut r, 32)? as u32);
            let members = match (c.cardinality, c.live_bits()) {
                (Some(card), Some(w)) => {
                    let live = take(&mut r, w)?;
                    if live > card {
                        return Err(BodyRefusal::Code);
                    }
                    if take(&mut r, 1)? == 0 {
                        let mut bits = Vec::new();
                        for _ in 0..live {
                            bits.push(take(&mut r, 1)? == 1);
                        }
                        Members::Bitmap(bits)
                    } else {
                        let count = take(&mut r, bits_for(u128::from(live) + 1))?;
                        if count > live {
                            return Err(BodyRefusal::Code);
                        }
                        let rw = bits_for(u128::from(live));
                        let mut ranks: Vec<u64> = Vec::new();
                        for _ in 0..count {
                            let rank = take(&mut r, rw)?;
                            if rank >= live || ranks.last().is_some_and(|p| rank <= *p) {
                                return Err(BodyRefusal::Code);
                            }
                            ranks.push(rank);
                        }
                        Members::List { live, ranks }
                    }
                }
                _ => Members::One {
                    vouched: take(&mut r, 1)? == 1,
                },
            };
            sections.push(Some(Section { digest, members }));
        }
        r.end().map_err(|e| match e {
            ReadRefusal::Pad => BodyRefusal::Pad,
            _ => BodyRefusal::Trailing,
        })?;
        Ok(Heartbeat { time, sections })
    }

    /// The body's bits and pad under `layout` ([`crate::packed::Record`]).
    ///
    /// # Errors
    /// A [`WriteError`]: a section per collection, each of its
    /// collection's shape, a live count within the cardinality, and ranks
    /// ascending below it.
    pub fn record(&self, layout: &Layout) -> Result<crate::packed::Record, WriteError> {
        if self.sections.len() != layout.collections.len() {
            return Err(WriteError::Sections {
                need: layout.collections.len(),
                have: self.sections.len(),
            });
        }
        let mut w = BitWriter::new();
        w.push(self.time.code(), TIME_BITS);
        for s in &self.sections {
            w.bit(s.is_some());
        }
        for (i, (c, s)) in layout.collections.iter().zip(&self.sections).enumerate() {
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
}
