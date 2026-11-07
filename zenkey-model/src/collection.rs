//! A collection's snapshot (core §7.5's form 7, 0.97; #977's E1): one
//! `state` or `@state` resource's members, their values and tombstones, in
//! the pages of a GET's reply. Defined, and adopted by no rule yet: E2
//! reads it, E3 has owners write it.
//!
//! ```text
//! page        CH · run · asof · body          a GET reply's payload, CH's tag the collection's type's
//! body        time · page · coll · live · tombs
//! time        t32 · ms16: when the owner built the snapshot
//! page        an integer: the page's index × 2, plus 1 when a later page follows
//! coll        an integer: the collection's index
//! live        an integer, their count · that many live entries
//! tombs       an integer, their count · that many tombstones
//! live entry  id · flags · [run] · seq · [conf] · length · record
//! tombstone   id · flags · [run] · seq
//! id          an integer, its length in bytes · that many bytes
//! flags       bit 7 own run · bits 6–5 confirmation · bit 4 run width · bits 3–0 zero
//! ```
//!
//! An integer is unsigned LEB128, at most 5 bytes and at most 2^32 − 1, in
//! its shortest form. The fixed fields are big-endian. An id is empty, or
//! plain chunks (§1.2) joined by `/`, below the reply's key.
//!
//! The envelope is [`crate::wire`]'s: a snapshot is a put's to the parser
//! ([`crate::wire::SampleKind::Put`]), its asof in the seq's place, its body
//! unread there. [`Page::parse`] reads a page's bytes, envelope and body,
//! and [`Page::read`] a body after its envelope; both refuse for the first
//! [`BodyRefusal`] that applies, reading the fields in order, as
//! `spec/conformance/snapshot/bodies.json` pins. [`Page::write`] writes
//! nothing that reading refuses, and every page accepted writes back to its
//! bytes. [`Snapshot::paginate`] cuts a snapshot into pages under a page
//! size, and [`Snapshot::complete`] assembles a complete set of pages.

use thiserror::Error;

use crate::chunk::is_plain_chunk;
use crate::contract::{Contract, Resource};
use crate::grammar::{KindToken, ZkKey, parse};
use crate::template::{Segment, Template};
use crate::wire::{self, Envelope, Form, Header, Run, Seq, Stamp, Tag};

/// The greatest integer the body holds: 2^32 − 1, in at most 5 bytes.
pub const INTEGER_MAX: u32 = u32::MAX;

/// A time, `t32 · ms16`: seconds and milliseconds since the deployment
/// epoch E (core §7.6), or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Time {
    /// t32 all ones with ms16 0: the owner's clock is unknown.
    Unknown,
    /// t32 seconds and ms16 milliseconds since the deployment epoch. A
    /// writer refuses an ms16 above 999, and a t32 all ones, which reads as
    /// unknown with ms16 0 and is unused with any other (`time`).
    Known { s: u32, ms: u16 },
}

impl Time {
    /// The time of an instant `t`, in milliseconds since the Unix epoch,
    /// under the deployment epoch `epoch` in seconds: t32 = ⌊(t − 1000 ×
    /// E) / 1000⌋ and ms16 = (t − 1000 × E) modulo 1000. `None` before the
    /// epoch, and from t32 all ones on, which no known time takes.
    #[must_use]
    pub fn from_unix_ms(t: i128, epoch: i64) -> Option<Time> {
        let since = t - i128::from(epoch) * 1000;
        if since < 0 {
            return None;
        }
        let s = u32::try_from(since / 1000)
            .ok()
            .filter(|&s| s != u32::MAX)?;
        // From 0 to 999: `since` is not negative here.
        let ms = (since % 1000) as u16;
        Some(Time::Known { s, ms })
    }

    /// The instant, in milliseconds since the Unix epoch, under the
    /// deployment epoch `epoch` in seconds; `None` when unknown.
    #[must_use]
    pub fn unix_ms(self, epoch: i64) -> Option<i128> {
        match self {
            Time::Unknown => None,
            Time::Known { s, ms } => {
                Some(i128::from(epoch) * 1000 + i128::from(s) * 1000 + i128::from(ms))
            }
        }
    }

    /// Its two fields, as a writer writes them, or `time` for one the
    /// reading would refuse or read as another.
    fn fields(self) -> Result<(u32, u16), BodyRefusal> {
        match self {
            Time::Unknown => Ok((u32::MAX, 0)),
            Time::Known { s, ms } if s != u32::MAX && ms <= 999 => Ok((s, ms)),
            Time::Known { .. } => Err(BodyRefusal::Time),
        }
    }

    /// The two fields read: unknown, known, or `time` for an unused ms16
    /// (above 999, or any but 0 with t32 all ones).
    fn of_fields(s: u32, ms: u16) -> Result<Time, BodyRefusal> {
        match (s, ms) {
            (u32::MAX, 0) => Ok(Time::Unknown),
            (u32::MAX, _) => Err(BodyRefusal::Time),
            (_, ms) if ms > 999 => Err(BodyRefusal::Time),
            (s, ms) => Ok(Time::Known { s, ms }),
        }
    }
}

/// When the owner last vouched for a live entry: the flags' bits 6–5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confirmation {
    /// 0: the collection has no horizon. A tombstone's is always 0.
    None,
    /// 1: the owner vouches for the member as it builds the snapshot, at
    /// the snapshot's time, as a heartbeat sent then would.
    AtTime,
    /// 2: the owner does not vouch for it now; `conf` follows, its last
    /// confirmation, the later of its last mutation and the last heartbeat
    /// that vouched for it. Unknown when the owner's clock was.
    At(Time),
}

impl Confirmation {
    /// Its code, 0, 1 or 2. 3 is unused.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Confirmation::None => 0,
            Confirmation::AtTime => 1,
            Confirmation::At(_) => 2,
        }
    }
}

/// A live entry: a member whose last mutation as of asof is a put.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    /// Below the reply's key: empty for the key itself, else plain chunks
    /// joined by `/`.
    pub id: String,
    /// Its own run, when its last mutation was under a run other than the
    /// header's; `None` for the header's.
    pub run: Option<Run>,
    /// The put's seq.
    pub seq: Seq,
    pub confirmation: Confirmation,
    /// The put's record, without its envelope; a packed record whole, its
    /// pad included.
    pub record: Vec<u8>,
}

/// A tombstone: a member the owner deleted within the window (S3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tombstone {
    /// As an [`Entry`]'s.
    pub id: String,
    /// As an [`Entry`]'s.
    pub run: Option<Run>,
    /// The delete's seq.
    pub seq: Seq,
}

/// One page of a snapshot: a GET reply's payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Page {
    /// CH's tag: the collection's type's, as a mutation's is.
    pub tag: Tag,
    /// The owner's run when it built the snapshot.
    pub run: Run,
    /// A seq of that run, in the seq's place: the snapshot reflects every
    /// state mutation of its run at or below it.
    pub asof: Seq,
    /// When the owner built the snapshot; no confirmation.
    pub time: Time,
    /// The page's index, from 0.
    pub index: u32,
    /// Whether a later page of the snapshot follows.
    pub more: bool,
    /// The collection's index among the interface's resources of the reply
    /// key's kind token, in the canonical form's order (§9.5).
    pub coll: u32,
    /// In the byte order of their ids.
    pub live: Vec<Entry>,
    /// In the byte order of their ids, none of them live.
    pub tombs: Vec<Tombstone>,
}

/// A snapshot: a page without its index and more bit, every entry of the
/// collection as of asof.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Snapshot {
    pub tag: Tag,
    pub run: Run,
    pub asof: Seq,
    pub time: Time,
    pub coll: u32,
    pub live: Vec<Entry>,
    pub tombs: Vec<Tombstone>,
}

/// Why a page's body is refused, by its reading or by a writer, which
/// writes nothing the reading refuses (core §7.5, "Reading a snapshot's
/// body"). The variants are in §7.5's order, the first that applies
/// reading the fields in order, and for one field the first in this
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum BodyRefusal {
    /// The body ends before a byte a field needs, an integer's last byte
    /// included.
    #[error("the body ends before a byte a field needs")]
    Short,
    /// An integer whose fifth byte has its high bit set, one above 2^32 −
    /// 1, or one not in its shortest form.
    #[error("an integer that is not LEB128 in its shortest form, at most 2^32 − 1")]
    Leb128,
    /// A time or a conf whose ms16 is unused: above 999, or any but 0 with
    /// t32 all ones.
    #[error("a time whose ms16 is unused")]
    Time,
    /// An id that is neither empty nor plain chunks joined by `/`.
    #[error("an id that is neither empty nor plain chunks joined by /")]
    Id,
    /// An id not above the one before it in its list, or a tombstone's id
    /// that a live entry of the body has.
    #[error("an id out of order, or both live and a tombstone")]
    Order,
    /// A flags byte with a bit from 3 to 0 set, bit 4 without bit 7,
    /// confirmation 3, or a tombstone's confirmation other than 0.
    #[error("a flags byte this version leaves unused")]
    Flags,
    /// An entry's own run that is the header's, in width and value.
    #[error("an entry's own run is the header's")]
    Run,
    /// A byte after the last tombstone.
    #[error("bytes after the last tombstone")]
    Trailing,
}

impl BodyRefusal {
    /// The token `spec/conformance/snapshot/bodies.json` pins: `short`,
    /// `leb128`, `time`, `id`, `order`, `flags`, `run` or `trailing`.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            BodyRefusal::Short => "short",
            BodyRefusal::Leb128 => "leb128",
            BodyRefusal::Time => "time",
            BodyRefusal::Id => "id",
            BodyRefusal::Order => "order",
            BodyRefusal::Flags => "flags",
            BodyRefusal::Run => "run",
            BodyRefusal::Trailing => "trailing",
        }
    }
}

/// Why a page's bytes are not read: its envelope refused by §7.5's parse
/// on a put, an envelope of another form, or its body refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PageRefusal {
    #[error("its envelope is refused: {0}")]
    Envelope(wire::Refusal),
    #[error("its envelope is a {0}, not a snapshot")]
    NotSnapshot(Form),
    #[error("its body is refused: {0}")]
    Body(BodyRefusal),
}

const RUN_FOLLOWS: u8 = 0x80;
const CONFIRMATION_SHIFT: u8 = 5;
const WIDE_RUN: u8 = 0x10;
const UNUSED_FLAGS: u8 = 0x0f;

/// The bytes an integer takes in LEB128's shortest form: 1 to 5.
#[must_use]
pub const fn integer_bytes(v: u32) -> usize {
    match v {
        0..=0x7f => 1,
        0x80..=0x3fff => 2,
        0x4000..=0x1f_ffff => 3,
        0x20_0000..=0x0fff_ffff => 4,
        _ => 5,
    }
}

fn write_integer(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A length as an integer, `leb128` above 2^32 − 1.
fn length(n: usize) -> Result<u32, BodyRefusal> {
    u32::try_from(n).map_err(|_| BodyRefusal::Leb128)
}

/// A length or a count as an integer for its size alone, 2^32 − 1 above
/// it: LEB128 takes 5 bytes there either way, and a writer refuses such a
/// length ([`length`]) before any size is relied on.
fn saturating(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Whether an id is empty, or plain chunks joined by `/`.
fn id_valid(id: &[u8]) -> bool {
    id.is_empty() || std::str::from_utf8(id).is_ok_and(|s| s.split('/').all(is_plain_chunk))
}

/// The flags byte of an entry: its own run, its confirmation's code.
fn flags(run: Option<Run>, confirmation: u8) -> u8 {
    let run_bits = match run {
        None => 0,
        Some(r) if r.is_wide() => RUN_FOLLOWS | WIDE_RUN,
        Some(_) => RUN_FOLLOWS,
    };
    run_bits | (confirmation << CONFIRMATION_SHIFT)
}

fn write_run(out: &mut Vec<u8>, run: Run) {
    match run {
        Run::U16(v) => out.extend_from_slice(&v.to_be_bytes()),
        Run::U32(v) => out.extend_from_slice(&v.to_be_bytes()),
    }
}

fn write_seq(out: &mut Vec<u8>, seq: Seq) {
    out.extend_from_slice(&seq.get().to_be_bytes()[1..]);
}

fn write_time(out: &mut Vec<u8>, (s, ms): (u32, u16)) {
    out.extend_from_slice(&s.to_be_bytes());
    out.extend_from_slice(&ms.to_be_bytes());
}

/// A writer's checks on an entry's id, as the reading makes them: `id`,
/// then `order` against `before`, the id before it in its list, and for a
/// tombstone against `live`, the body's live entries in order.
fn check_id(id: &str, before: Option<&str>, live: Option<&[Entry]>) -> Result<(), BodyRefusal> {
    if !id_valid(id.as_bytes()) {
        return Err(BodyRefusal::Id);
    }
    if before.is_some_and(|b| id.as_bytes() <= b.as_bytes()) {
        return Err(BodyRefusal::Order);
    }
    if live.is_some_and(|l| {
        l.binary_search_by(|e| e.id.as_bytes().cmp(id.as_bytes()))
            .is_ok()
    }) {
        return Err(BodyRefusal::Order);
    }
    Ok(())
}

fn check_run(own: Option<Run>, header: Run) -> Result<(), BodyRefusal> {
    if own == Some(header) {
        return Err(BodyRefusal::Run);
    }
    Ok(())
}

impl Entry {
    /// Its bytes in a body: id, flags, its own run, seq, conf, length and
    /// record.
    fn write_to(&self, out: &mut Vec<u8>) -> Result<(), BodyRefusal> {
        write_integer(out, length(self.id.len())?);
        out.extend_from_slice(self.id.as_bytes());
        out.push(flags(self.run, self.confirmation.code()));
        if let Some(r) = self.run {
            write_run(out, r);
        }
        write_seq(out, self.seq);
        if let Confirmation::At(t) = self.confirmation {
            write_time(out, t.fields()?);
        }
        write_integer(out, length(self.record.len())?);
        out.extend_from_slice(&self.record);
        Ok(())
    }

    /// Its bytes in a body, computed without writing it.
    fn size(&self) -> usize {
        integer_bytes(saturating(self.id.len()))
            + self.id.len()
            + 1
            + self.run.map_or(0, Run::width_bytes)
            + 3
            + 6 * usize::from(matches!(self.confirmation, Confirmation::At(_)))
            + integer_bytes(saturating(self.record.len()))
            + self.record.len()
    }
}

impl Tombstone {
    fn write_to(&self, out: &mut Vec<u8>) -> Result<(), BodyRefusal> {
        write_integer(out, length(self.id.len())?);
        out.extend_from_slice(self.id.as_bytes());
        out.push(flags(self.run, 0));
        if let Some(r) = self.run {
            write_run(out, r);
        }
        write_seq(out, self.seq);
        Ok(())
    }

    fn size(&self) -> usize {
        integer_bytes(saturating(self.id.len()))
            + self.id.len()
            + 1
            + self.run.map_or(0, Run::width_bytes)
            + 3
    }
}

/// A body's bytes, read a field at a time.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], BodyRefusal> {
        if self.bytes.len() < n {
            return Err(BodyRefusal::Short);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, BodyRefusal> {
        Ok(self.take(1)?[0])
    }

    /// An integer: a byte at a time, stopping at the fifth, whose high bit
    /// set is `leb128` whatever follows it.
    fn integer(&mut self) -> Result<u32, BodyRefusal> {
        let mut v: u64 = 0;
        for i in 0..5 {
            let b = self.byte()?;
            v |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                if b == 0 && i > 0 {
                    return Err(BodyRefusal::Leb128);
                }
                return u32::try_from(v).map_err(|_| BodyRefusal::Leb128);
            }
        }
        Err(BodyRefusal::Leb128)
    }

    fn time(&mut self) -> Result<Time, BodyRefusal> {
        let t = self.take(6)?;
        Time::of_fields(
            u32::from_be_bytes([t[0], t[1], t[2], t[3]]),
            u16::from_be_bytes([t[4], t[5]]),
        )
    }

    fn id(&mut self) -> Result<&'a [u8], BodyRefusal> {
        let n = self.integer()?;
        let id = self.take(n as usize)?;
        if !id_valid(id) {
            return Err(BodyRefusal::Id);
        }
        Ok(id)
    }

    /// An entry's flags, then its own run: `(run, confirmation)`.
    fn flags_and_run(
        &mut self,
        header: Run,
        tombstone: bool,
    ) -> Result<(Option<Run>, u8), BodyRefusal> {
        let f = self.byte()?;
        let confirmation = (f >> CONFIRMATION_SHIFT) & 0x03;
        let follows = f & RUN_FOLLOWS != 0;
        let wide = f & WIDE_RUN != 0;
        if f & UNUSED_FLAGS != 0
            || (wide && !follows)
            || confirmation == 3
            || (tombstone && confirmation != 0)
        {
            return Err(BodyRefusal::Flags);
        }
        let run = if !follows {
            None
        } else if wide {
            let r = self.take(4)?;
            Some(Run::U32(u32::from_be_bytes([r[0], r[1], r[2], r[3]])))
        } else {
            let r = self.take(2)?;
            Some(Run::U16(u16::from_be_bytes([r[0], r[1]])))
        };
        check_run(run, header)?;
        Ok((run, confirmation))
    }

    fn seq(&mut self) -> Result<Seq, BodyRefusal> {
        let s = self.take(3)?;
        Ok(Seq::new(u32::from_be_bytes([0, s[0], s[1], s[2]])).expect("24 bits"))
    }
}

/// An id read as text: [`Reader::id`] has checked it is ASCII.
fn id_text(id: &[u8]) -> String {
    std::str::from_utf8(id)
        .expect("plain chunks are ASCII")
        .to_owned()
}

impl Page {
    /// A page's bytes, CH to its last tombstone: the envelope by §7.5's
    /// parse on a put, which must be a snapshot's, then its body
    /// ([`Page::read`]). The key's kind is not read here:
    /// [`crate::wire::read_put`] refuses a snapshot off a state key.
    ///
    /// # Errors
    ///
    /// The envelope's refusal, a form other than a snapshot, or the
    /// body's first refusal.
    pub fn parse(bytes: &[u8]) -> Result<Page, PageRefusal> {
        let e = Envelope::parse(bytes).map_err(PageRefusal::Envelope)?;
        Page::read(&e)
    }

    /// A snapshot's envelope with its body read: the body's fields in
    /// order, with the header's run, refused for the first that applies
    /// (core §7.5, "Reading a snapshot's body").
    ///
    /// # Errors
    ///
    /// [`PageRefusal::NotSnapshot`] for an envelope of another form, and
    /// [`PageRefusal::Body`] with the first [`BodyRefusal`] that applies.
    pub fn read(envelope: &Envelope<'_>) -> Result<Page, PageRefusal> {
        if envelope.form != Form::Snapshot {
            return Err(PageRefusal::NotSnapshot(envelope.form));
        }
        Page::read_body(envelope.tag, envelope.stamp, envelope.record).map_err(PageRefusal::Body)
    }

    fn read_body(tag: Tag, stamp: Stamp, body: &[u8]) -> Result<Page, BodyRefusal> {
        let header = stamp.run;
        let mut r = Reader { bytes: body };
        let time = r.time()?;
        let page = r.integer()?;
        let coll = r.integer()?;
        let live_count = r.integer()?;
        // Never `with_capacity` from a count: it comes from the wire, up to
        // 2^32 − 1, and only the entries that follow it bound it. The
        // vectors grow with the entries read, so a short body is refused
        // before it holds more than its bytes.
        let mut live: Vec<Entry> = Vec::new();
        for _ in 0..live_count {
            let id = r.id()?;
            if live.last().is_some_and(|b| id <= b.id.as_bytes()) {
                return Err(BodyRefusal::Order);
            }
            let (run, c) = r.flags_and_run(header, false)?;
            let seq = r.seq()?;
            let confirmation = match c {
                0 => Confirmation::None,
                1 => Confirmation::AtTime,
                _ => Confirmation::At(r.time()?),
            };
            let n = r.integer()?;
            let record = r.take(n as usize)?.to_vec();
            live.push(Entry {
                id: id_text(id),
                run,
                seq,
                confirmation,
                record,
            });
        }
        let tomb_count = r.integer()?;
        let mut tombs: Vec<Tombstone> = Vec::new();
        for _ in 0..tomb_count {
            let id = r.id()?;
            if tombs.last().is_some_and(|b| id <= b.id.as_bytes())
                || live.binary_search_by(|e| e.id.as_bytes().cmp(id)).is_ok()
            {
                return Err(BodyRefusal::Order);
            }
            let (run, _) = r.flags_and_run(header, true)?;
            let seq = r.seq()?;
            tombs.push(Tombstone {
                id: id_text(id),
                run,
                seq,
            });
        }
        if !r.bytes.is_empty() {
            return Err(BodyRefusal::Trailing);
        }
        Ok(Page {
            tag,
            run: header,
            asof: stamp.seq,
            time,
            index: page >> 1,
            more: page & 1 == 1,
            coll,
            live,
            tombs,
        })
    }

    /// Its CH: a snapshot, the width the run takes, and its tag.
    #[must_use]
    pub const fn header(&self) -> Header {
        Header {
            form: Form::Snapshot,
            wide: self.run.is_wide(),
            tag: self.tag,
        }
    }

    /// The page's bytes, CH to its last tombstone, or the refusal its
    /// reading would give, the first in reading order: a writer writes no
    /// page the reading refuses (core §7.5).
    ///
    /// # Errors
    ///
    /// `time`, `id`, `order` or `run` for a field the reading would
    /// refuse, and `leb128` for an index, a count or a length above 2^32 −
    /// 1 as an integer.
    pub fn write(&self) -> Result<Vec<u8>, BodyRefusal> {
        let page = self
            .index
            .checked_mul(2)
            .and_then(|p| p.checked_add(u32::from(self.more)))
            .ok_or(BodyRefusal::Leb128)?;
        let mut out = Vec::with_capacity(self.size());
        out.push(self.header().byte());
        Stamp {
            run: self.run,
            seq: self.asof,
        }
        .write_to(&mut out);
        write_time(&mut out, self.time.fields()?);
        write_integer(&mut out, page);
        write_integer(&mut out, self.coll);
        write_integer(&mut out, length(self.live.len())?);
        let mut before: Option<&str> = None;
        for e in &self.live {
            check_id(&e.id, before, None)?;
            check_run(e.run, self.run)?;
            e.write_to(&mut out)?;
            before = Some(&e.id);
        }
        write_integer(&mut out, length(self.tombs.len())?);
        let mut before: Option<&str> = None;
        for t in &self.tombs {
            check_id(&t.id, before, Some(&self.live))?;
            check_run(t.run, self.run)?;
            t.write_to(&mut out)?;
            before = Some(&t.id);
        }
        Ok(out)
    }

    /// Its bytes' count, CH to its last tombstone, as [`Page::write`]
    /// writes them.
    #[must_use]
    pub fn size(&self) -> usize {
        page_size(
            self.run,
            self.index,
            self.coll,
            self.live.len(),
            self.live.iter().map(Entry::size).sum(),
            self.tombs.len(),
            self.tombs.iter().map(Tombstone::size).sum(),
        )
    }
}

/// A page's bytes: CH, the run, asof, the time, the page integer (whose
/// more bit never changes its size), coll, the two counts and the entries.
fn page_size(
    run: Run,
    index: u32,
    coll: u32,
    live: usize,
    live_bytes: usize,
    tombs: usize,
    tomb_bytes: usize,
) -> usize {
    1 + run.width_bytes()
        + 3
        + 6
        + integer_bytes(index.saturating_mul(2))
        + integer_bytes(coll)
        + integer_bytes(saturating(live))
        + live_bytes
        + integer_bytes(saturating(tombs))
        + tomb_bytes
}

/// One of a snapshot's entries, live or a tombstone, as the pages take
/// them: together, in the byte order of their ids.
#[derive(Clone, Copy)]
enum Item<'a> {
    Live(&'a Entry),
    Tomb(&'a Tombstone),
}

impl<'a> Item<'a> {
    fn id(self) -> &'a str {
        match self {
            Item::Live(e) => &e.id,
            Item::Tomb(t) => &t.id,
        }
    }
}

impl Snapshot {
    /// The snapshot as one page, index 0 and the last: what its pages
    /// write, checked whole.
    fn as_page(&self) -> Page {
        Page {
            tag: self.tag,
            run: self.run,
            asof: self.asof,
            time: self.time,
            index: 0,
            more: false,
            coll: self.coll,
            live: self.live.clone(),
            tombs: self.tombs.clone(),
        }
    }

    /// The snapshot cut into pages under a page size of `page_bytes`
    /// (core §7.5, "Pages"): its live entries and tombstones together, in
    /// the byte order of their ids, fill page 0, then page 1. A page takes
    /// the next entry when it holds none yet, or when its bytes with that
    /// entry, CH to its last tombstone, are within the page size. The last
    /// page's more bit is 0, and a snapshot with no entry is one page.
    ///
    /// # Errors
    ///
    /// The refusal [`Page::write`] gives the snapshot as one page: a
    /// writer cuts no snapshot it could not write.
    pub fn paginate(&self, page_bytes: usize) -> Result<Vec<Page>, BodyRefusal> {
        self.as_page().write()?;
        let mut items: Vec<Item<'_>> = self
            .live
            .iter()
            .map(Item::Live)
            .chain(self.tombs.iter().map(Item::Tomb))
            .collect();
        items.sort_by_key(|i| i.id().as_bytes());
        let empty = |index: u32| Page {
            tag: self.tag,
            run: self.run,
            asof: self.asof,
            time: self.time,
            index,
            more: false,
            coll: self.coll,
            live: Vec::new(),
            tombs: Vec::new(),
        };
        let mut pages = vec![empty(0)];
        let (mut live_bytes, mut tomb_bytes) = (0usize, 0usize);
        for item in items {
            let (dl, dt, size) = match item {
                Item::Live(e) => (1, 0, e.size()),
                Item::Tomb(t) => (0, 1, t.size()),
            };
            let page = pages.last().expect("a page");
            let held = page.live.len() + page.tombs.len();
            let with = page_size(
                self.run,
                page.index,
                self.coll,
                page.live.len() + dl,
                live_bytes + dl * size,
                page.tombs.len() + dt,
                tomb_bytes + dt * size,
            );
            if held > 0 && with > page_bytes {
                let index = page.index.checked_add(1).ok_or(BodyRefusal::Leb128)?;
                pages.last_mut().expect("a page").more = true;
                pages.push(empty(index));
                live_bytes = 0;
                tomb_bytes = 0;
            }
            let page = pages.last_mut().expect("a page");
            match item {
                Item::Live(e) => {
                    page.live.push(e.clone());
                    live_bytes += size;
                }
                Item::Tomb(t) => {
                    page.tombs.push(t.clone());
                    tomb_bytes += size;
                }
            }
        }
        Ok(pages)
    }

    /// The snapshot a complete set of pages holds (core §7.5,
    /// "Complete"): one page of each index from 0 to a page whose more bit
    /// is 0, all with one CH, run, asof, time and coll, in any order.
    /// `None` when they are not complete.
    #[must_use]
    pub fn complete(pages: &[Page]) -> Option<Snapshot> {
        let mut sorted: Vec<&Page> = pages.iter().collect();
        sorted.sort_by_key(|p| p.index);
        let first = sorted.first()?;
        let last = sorted.last()?;
        let same = |p: &Page| {
            (p.tag, p.run, p.asof, p.time, p.coll)
                == (first.tag, first.run, first.asof, first.time, first.coll)
        };
        let whole = sorted.iter().enumerate().all(|(i, p)| {
            u32::try_from(i).is_ok_and(|i| p.index == i)
                && p.more == (p.index != last.index)
                && same(p)
        });
        if !whole {
            return None;
        }
        Some(Snapshot {
            tag: first.tag,
            run: first.run,
            asof: first.asof,
            time: first.time,
            coll: first.coll,
            live: sorted.iter().flat_map(|p| p.live.iter().cloned()).collect(),
            tombs: sorted
                .iter()
                .flat_map(|p| p.tombs.iter().cloned())
                .collect(),
        })
    }
}

/// A member's key in a page heard on `key`: `key`, then `/` and the id;
/// `key` itself for the empty id (core §7.5).
#[must_use]
pub fn member_key(key: &str, id: &str) -> String {
    if id.is_empty() {
        key.to_owned()
    } else {
        format!("{key}/{id}")
    }
}

/// One entry of a page as the per-key reply it stands for (core 0.97
/// §4.2's "what a profile reads", read by #977's E2): on its member's key,
/// with its stamp, a live entry the put of its record and a tombstone the
/// delete. A reader that reads per-key replies reads these the same way:
/// [`Member::payload`] is the bytes the mutation put, and
/// [`Member::attachment`] the delete's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Member {
    /// The page's key, then `/` and the id ([`member_key`]).
    pub key: String,
    /// Its own run, else the page's, and its seq.
    pub stamp: Stamp,
    /// CH's tag: the page's, the collection's type's, as the mutation's.
    pub tag: Tag,
    /// A live entry's confirmation; a tombstone's is
    /// [`Confirmation::None`].
    pub confirmation: Confirmation,
    /// A live entry's record; `None` for a tombstone.
    pub record: Option<Vec<u8>>,
}

impl Member {
    /// Whether it is a tombstone: a delete within the window (S3).
    #[must_use]
    pub const fn is_tombstone(&self) -> bool {
        self.record.is_none()
    }

    /// A live entry's put, as the owner's per-key reply carries it:
    /// `CH · run · seq · record`, CH a mutation's with the page's tag and
    /// the width its run takes. `None` for a tombstone.
    #[must_use]
    pub fn payload(&self) -> Option<Vec<u8>> {
        let record = self.record.as_deref()?;
        Some(Envelope::mutation(self.tag, self.stamp, record).write())
    }

    /// A tombstone's delete, as the owner's `reply_del` carries it in its
    /// attachment: `CH · run · seq`. `None` for a live entry.
    #[must_use]
    pub fn attachment(&self) -> Option<Vec<u8>> {
        self.is_tombstone()
            .then(|| wire::DeleteStamp { stamp: self.stamp }.write())
    }
}

impl Page {
    /// Its entries as the per-key replies they stand for, live entries and
    /// tombstones together in the byte order of their ids, each on its
    /// member's key below `key`, the key the page was heard on.
    #[must_use]
    pub fn members(&self, key: &str) -> Vec<Member> {
        let stamp = |run: Option<Run>, seq| Stamp {
            run: run.unwrap_or(self.run),
            seq,
        };
        let mut out: Vec<Member> = self
            .live
            .iter()
            .map(|e| Member {
                key: member_key(key, &e.id),
                stamp: stamp(e.run, e.seq),
                tag: self.tag,
                confirmation: e.confirmation,
                record: Some(e.record.clone()),
            })
            .chain(self.tombs.iter().map(|t| Member {
                key: member_key(key, &t.id),
                stamp: stamp(t.run, t.seq),
                tag: self.tag,
                confirmation: Confirmation::None,
                record: None,
            }))
            .collect();
        out.sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));
        out
    }
}

/// One snapshot as a reader holds it, from the pages it heard (core §7.5,
/// "Complete"): the pages of one key, CH's tag, run, asof, time and coll,
/// and which indices it holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heard {
    /// The key its pages were heard on: the snapshot key of a collection
    /// read, a member's key of a member read.
    pub key: String,
    pub tag: Tag,
    pub run: Run,
    pub asof: Seq,
    pub time: Time,
    pub coll: u32,
    /// The indices held, each once however often it was heard.
    pub pages: std::collections::BTreeSet<u32>,
    /// The indices held whose more bit is 0.
    pub last: std::collections::BTreeSet<u32>,
}

impl Heard {
    /// The snapshot `page`, heard on `key`, is a page of, holding none of
    /// its pages yet.
    #[must_use]
    pub fn of(key: &str, page: &Page) -> Heard {
        Heard {
            key: key.to_owned(),
            tag: page.tag,
            run: page.run,
            asof: page.asof,
            time: page.time,
            coll: page.coll,
            pages: std::collections::BTreeSet::new(),
            last: std::collections::BTreeSet::new(),
        }
    }

    /// Whether `page`, heard on `key`, is one of its pages: one key, CH's
    /// tag, run (its width included), asof, time and coll.
    #[must_use]
    pub fn holds(&self, key: &str, page: &Page) -> bool {
        self.key == key
            && (self.tag, self.run, self.asof, self.time, self.coll)
                == (page.tag, page.run, page.asof, page.time, page.coll)
    }

    /// Holds `page`, one of its pages; whether it held none of its index
    /// before.
    pub fn add(&mut self, page: &Page) -> bool {
        if !page.more {
            self.last.insert(page.index);
        }
        self.pages.insert(page.index)
    }

    /// Whether it is complete (core §7.5): one page of each index from 0
    /// to a page whose more bit is 0, and no other, so that it holds every
    /// entry the snapshot has.
    #[must_use]
    pub fn complete(&self) -> bool {
        let Some(&last) = self.last.first() else {
            return false;
        };
        self.last.len() == 1
            && self.pages.last() == Some(&last)
            && u64::try_from(self.pages.len()).is_ok_and(|n| n == u64::from(last) + 1)
    }
}

/// Adds `page`, heard on `key`, to the snapshot of `heard` it is a page of,
/// or to a new one at the end: one GET's snapshots, in the order of their
/// first page. Returns that snapshot's place, and whether this page made it
/// complete: complete now, and not before.
pub fn hear(heard: &mut Vec<Heard>, key: &str, page: &Page) -> (usize, bool) {
    let at = match heard.iter().position(|h| h.holds(key, page)) {
        Some(at) => at,
        None => {
            heard.push(Heard::of(key, page));
            heard.len() - 1
        }
    };
    let h = &mut heard[at];
    let was = h.complete();
    h.add(page);
    (at, !was && h.complete())
}

/// The collection's index of `resource`, a `state` or `@state` resource of
/// `contract`: its place, from 0, among the contract's resources of its
/// kind token, in the canonical form's order (core §7.5, §9.5), the order
/// [`Contract::resources`] keeps. `None` for any other kind, and for a
/// resource the contract does not hold.
#[must_use]
pub fn coll_of(contract: &Contract, resource: &Resource) -> Option<u32> {
    if !matches!(resource.token, KindToken::State | KindToken::ExplicitState) {
        return None;
    }
    let at = contract
        .resources
        .iter()
        .filter(|r| r.token == resource.token)
        .position(|r| r.template == resource.template)?;
    u32::try_from(at).ok()
}

/// The resource a snapshot's coll names, heard on a key whose kind token
/// is `kind` (`state` or `@state`): [`coll_of`]'s inverse. `None` when the
/// contract has no such collection, which a reader without the contract
/// cannot tell either (core §7.5, "Why no contract").
#[must_use]
pub fn collection_of(contract: &Contract, kind: KindToken, coll: u32) -> Option<&Resource> {
    if !matches!(kind, KindToken::State | KindToken::ExplicitState) {
        return None;
    }
    contract
        .resources
        .iter()
        .filter(|r| r.token == kind)
        .nth(usize::try_from(coll).ok()?)
}

/// A collection's snapshot key below its kind token (core 0.97 §4.2,
/// "Answering a state GET", E3's text): its template's chunks before its
/// first parameter, the whole template when it has none. `None` for a
/// template whose first chunk is a parameter, which has no snapshot key.
#[must_use]
pub fn snapshot_prefix(template: &Template) -> Option<Vec<&str>> {
    let prefix: Vec<&str> = template
        .segments()
        .iter()
        .map_while(|s| match s {
            Segment::Literal(l) => Some(l.as_str()),
            Segment::Param(_) | Segment::Rest(_) => None,
        })
        .collect();
    (!prefix.is_empty()).then_some(prefix)
}

/// Whether `key` is a member's key of `resource`'s collection: a zk2 data
/// key of the resource's kind token whose resource chunks its template
/// matches, each parameter one chunk. A key with a namespace in front is
/// read from its first `zk2` chunk from which the grammar parses a data
/// key, as [`wire::enveloped_kind`] reads one. The address and interface
/// are not read: the caller asks of keys at or under one page's key.
#[must_use]
pub fn is_member(resource: &Resource, key: &str) -> bool {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(ZkKey::Data {
                kind,
                resource: chunks,
                ..
            }) = parse(k)
        {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            return kind == resource.token && resource.template.matches(&refs).is_some();
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_round_trip_in_their_shortest_form() {
        for v in [
            0,
            1,
            127,
            128,
            16_383,
            16_384,
            0x0fff_ffff,
            0x1000_0000,
            u32::MAX,
        ] {
            let mut out = Vec::new();
            write_integer(&mut out, v);
            assert_eq!(out.len(), integer_bytes(v), "{v}");
            let mut r = Reader { bytes: &out };
            assert_eq!(r.integer(), Ok(v));
            assert!(r.bytes.is_empty());
        }
        let refused = |b: &[u8]| Reader { bytes: b }.integer();
        assert_eq!(refused(&[0x80, 0x00][..]), Err(BodyRefusal::Leb128));
        assert_eq!(
            refused(&[0xff, 0xff, 0xff, 0xff, 0x10][..]),
            Err(BodyRefusal::Leb128)
        );
        assert_eq!(
            refused(&[0x80, 0x80, 0x80, 0x80, 0x80][..]),
            Err(BodyRefusal::Leb128)
        );
        assert_eq!(
            refused(&[0x80, 0x80, 0x80, 0x80][..]),
            Err(BodyRefusal::Short)
        );
    }

    #[test]
    fn a_time_from_the_unix_clock() {
        let epoch = 1_767_225_600;
        let t = i128::from(epoch) * 1000 + 3_600_250;
        assert_eq!(
            Time::from_unix_ms(t, epoch),
            Some(Time::Known { s: 3600, ms: 250 })
        );
        assert_eq!(Time::Known { s: 3600, ms: 250 }.unix_ms(epoch), Some(t));
        assert_eq!(Time::from_unix_ms(t - 3_600_251, epoch), None);
        assert_eq!(Time::Unknown.unix_ms(epoch), None);
    }

    /// A page's entries are the per-key replies they stand for: a live
    /// entry the bytes its mutation put, which §7.5's parse reads back to
    /// its stamp and record, a tombstone a delete's attachment; an entry's
    /// own run beside the header's.
    #[test]
    fn a_pages_entries_are_their_members_replies() {
        let page = Page {
            tag: Tag::new(3).unwrap(),
            run: Run::U16(5),
            asof: Seq::new(9).unwrap(),
            time: Time::Unknown,
            index: 0,
            more: false,
            coll: 1,
            live: vec![
                Entry {
                    id: "a".into(),
                    run: Some(Run::U32(7)),
                    seq: Seq::new(2).unwrap(),
                    confirmation: Confirmation::AtTime,
                    record: b"r".to_vec(),
                },
                Entry {
                    id: "c".into(),
                    run: None,
                    seq: Seq::new(4).unwrap(),
                    confirmation: Confirmation::None,
                    record: Vec::new(),
                },
            ],
            tombs: vec![Tombstone {
                id: "b".into(),
                run: None,
                seq: Seq::new(3).unwrap(),
            }],
        };
        let m = page.members("zk2/h/s/i.v1/state/x");
        let keys: Vec<&str> = m.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "zk2/h/s/i.v1/state/x/a",
                "zk2/h/s/i.v1/state/x/b",
                "zk2/h/s/i.v1/state/x/c"
            ]
        );
        let bytes = m[0].payload().unwrap();
        let put = Envelope::parse(&bytes).unwrap();
        assert_eq!(
            (put.form, put.tag, put.record),
            (Form::Mutation, page.tag, &b"r"[..])
        );
        assert_eq!(put.stamp.run, Run::U32(7), "its own run");
        assert!(m[1].is_tombstone() && m[1].payload().is_none());
        let del = wire::DeleteStamp::parse(&m[1].attachment().unwrap()).unwrap();
        assert_eq!(
            del.stamp,
            Stamp {
                run: Run::U16(5),
                seq: Seq::new(3).unwrap()
            }
        );
        assert_eq!(m[2].attachment(), None);
        assert_eq!(member_key("k", ""), "k");
    }

    /// Pages are held by snapshot, and one is complete at one page of each
    /// index to a last, and no other: out of order, twice, a page lost, two
    /// last pages, or two times apart.
    #[test]
    fn a_snapshot_is_complete_at_every_index_to_its_last() {
        let page = |index, more| Page {
            tag: Tag::NONE,
            run: Run::U16(1),
            asof: Seq::new(9).unwrap(),
            time: Time::Known { s: 1, ms: 2 },
            index,
            more,
            coll: 0,
            live: Vec::new(),
            tombs: Vec::new(),
        };
        let mut heard = Vec::new();
        assert_eq!(hear(&mut heard, "k", &page(2, false)), (0, false));
        assert_eq!(hear(&mut heard, "k", &page(0, true)), (0, false));
        assert_eq!(hear(&mut heard, "k", &page(0, true)), (0, false), "twice");
        assert_eq!(
            hear(&mut heard, "k", &page(1, true)),
            (0, true),
            "complete now"
        );
        assert_eq!(
            hear(&mut heard, "k", &page(1, true)),
            (0, false),
            "and not newly"
        );
        assert!(heard[0].complete());
        let mut other = page(0, false);
        other.time = Time::Unknown;
        assert_eq!(
            hear(&mut heard, "k", &other),
            (1, true),
            "another time, another snapshot"
        );
        assert_eq!(
            hear(&mut heard, "j", &page(0, false)),
            (2, true),
            "another key"
        );
        let mut lost = Vec::new();
        hear(&mut lost, "k", &page(0, true));
        hear(&mut lost, "k", &page(2, false));
        assert!(!lost[0].complete(), "page 1 lost");
        let mut two = Vec::new();
        hear(&mut two, "k", &page(0, false));
        hear(&mut two, "k", &page(1, false));
        assert!(!two[0].complete(), "two last pages");
    }

    /// A collection's index is its place among its kind token's resources
    /// in canonical order; its snapshot key the template's chunks before its
    /// first parameter; and its members the keys its template matches.
    #[test]
    fn a_collections_index_snapshot_key_and_members() {
        let l = crate::contract::load_str(
            r#"[interface]
name = "c"
major = 1
minor = 0

[resources."b/{x}"]
kind = "state"
type = { raw = "text/plain" }
params = { x = "string" }
cardinality = 4

[resources.a]
kind = "state"
type = { raw = "text/plain" }

[resources."{y}/z"]
kind = "state"
explicit = true
type = { raw = "text/plain" }
params = { y = "string" }
cardinality = 4

[resources.s]
kind = "stream"
type = { raw = "text/plain" }
"#,
            std::path::Path::new("."),
            None,
        );
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        let by = |t: &str| {
            c.resources
                .iter()
                .find(|r| r.template.as_str() == t)
                .unwrap()
        };
        assert_eq!(coll_of(&c, by("a")), Some(0));
        assert_eq!(coll_of(&c, by("b/{x}")), Some(1));
        assert_eq!(coll_of(&c, by("{y}/z")), Some(0), "@state counts apart");
        assert_eq!(coll_of(&c, by("s")), None, "a stream is no collection");
        let named = |k, i| collection_of(&c, k, i).map(|r| r.template.as_str());
        assert_eq!(named(KindToken::State, 1), Some("b/{x}"));
        assert_eq!(named(KindToken::ExplicitState, 0), Some("{y}/z"));
        assert_eq!(named(KindToken::State, 2), None);
        assert_eq!(named(KindToken::Stream, 0), None);
        assert_eq!(snapshot_prefix(&by("b/{x}").template), Some(vec!["b"]));
        assert_eq!(snapshot_prefix(&by("a").template), Some(vec!["a"]));
        assert_eq!(snapshot_prefix(&by("{y}/z").template), None);
        assert!(is_member(by("b/{x}"), "zk2/h/s/c.v1/state/b/one"));
        assert!(
            is_member(by("b/{x}"), "prod/zk2/h/s/c.v1/state/b/one"),
            "namespaced"
        );
        assert!(
            !is_member(by("b/{x}"), "zk2/h/s/c.v1/state/b"),
            "the snapshot key"
        );
        assert!(!is_member(by("b/{x}"), "zk2/h/s/c.v1/state/b/one/two"));
        assert!(
            !is_member(by("b/{x}"), "zk2/h/s/c.v1/@state/b/one"),
            "another kind"
        );
        assert!(is_member(by("a"), "zk2/h/s/c.v1/state/a"));
    }

    #[test]
    fn an_empty_snapshot_is_one_sixteen_byte_page() {
        let s = Snapshot {
            tag: Tag::NONE,
            run: Run::U16(1),
            asof: Seq::new(9).unwrap(),
            time: Time::Known { s: 3600, ms: 250 },
            coll: 0,
            live: Vec::new(),
            tombs: Vec::new(),
        };
        let pages = s.paginate(1).unwrap();
        assert_eq!(pages.len(), 1);
        let bytes = pages[0].write().unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(pages[0].size(), 16);
        assert_eq!(Page::parse(&bytes).as_ref(), Ok(&pages[0]));
        assert_eq!(Snapshot::complete(&pages), Some(s));
    }
}
