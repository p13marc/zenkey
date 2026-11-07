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
        let ms = u16::try_from(since % 1000).expect("below 1000");
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
        integer_bytes(self.id.len() as u32)
            + self.id.len()
            + 1
            + self.run.map_or(0, Run::width_bytes)
            + 3
            + 6 * usize::from(matches!(self.confirmation, Confirmation::At(_)))
            + integer_bytes(self.record.len() as u32)
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
        integer_bytes(self.id.len() as u32)
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
        + integer_bytes(live as u32)
        + live_bytes
        + integer_bytes(tombs as u32)
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
