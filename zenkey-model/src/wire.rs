//! The zk2c envelope (core §7.5, 0.72; #976, epic #975): the bytes that
//! carry an owner's run and sequence number with a sample, before its
//! record.
//!
//! ```text
//! mutation, a put's payload        CH · run · seq · record
//! delete, a delete's attachment    CH · run · seq
//! CH                               form (bits 7–5) · run width (bit 4) · tag (bits 3–0)
//! run                              16 or 32 bits unsigned, big-endian, as CH's width bit says
//! seq                              24 bits unsigned, big-endian
//! record                           every byte after the seq, read as the resource's type says
//! ```
//!
//! **Defined ahead of its use.** No rule adopts the envelope yet: no owner
//! writes one and no consumer has to read one, so nothing in the runtime,
//! the fleet or the tools calls this module. What a run names and how a
//! seq counts are for the rule that adopts it (#976's flip); this module
//! fixes the bytes alone.
//!
//! **Parsing reads no contract,** as parsing a key does (§1.1): it knows
//! the sample's kind, put ([`Envelope::parse`]) or delete
//! ([`DeleteStamp::parse`]), and nothing more. So a mutation's tag is
//! reported, never checked against a type; `packed` (#979) is the first
//! type kind to define one. The first refusal that applies is reported, in
//! §7.5's order, with the token `spec/conformance/envelope/` pins
//! ([`Refusal::token`]). Every envelope accepted writes back to its bytes.
//!
//! Not [`crate::envelope`], the error envelope of a failed call (§5.2).

use thiserror::Error;

/// A form: CH's three high bits. This version defines a mutation and a
/// delete, and reserves the other six for later versions, each refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Form {
    /// Code 0: a put's run and seq, then its record.
    Mutation,
    /// Code 1: a delete's run and seq, as its attachment, and no record.
    Delete,
    /// Code 2, reserved: an operation's reply value.
    Value,
    /// Code 3, reserved: an operation's error reply.
    Error,
    /// Code 4, reserved: an event's occurrence.
    Event,
    /// Code 5, reserved: a stream's records under one header.
    Frame,
    /// Code 6, reserved: an owner's periodic confirmation of what it holds.
    Heartbeat,
    /// Code 7, reserved: a collection's state in one reply.
    Snapshot,
}

impl Form {
    /// Every form, in code order.
    pub const ALL: [Form; 8] = [
        Form::Mutation,
        Form::Delete,
        Form::Value,
        Form::Error,
        Form::Event,
        Form::Frame,
        Form::Heartbeat,
        Form::Snapshot,
    ];

    /// The form's code, from 0 to 7.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Form::Mutation => 0,
            Form::Delete => 1,
            Form::Value => 2,
            Form::Error => 3,
            Form::Event => 4,
            Form::Frame => 5,
            Form::Heartbeat => 6,
            Form::Snapshot => 7,
        }
    }

    /// The form of a code, `None` above 7.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Form> {
        if code < 8 {
            Some(Form::ALL[code as usize])
        } else {
            None
        }
    }

    /// Whether this version defines the form; a reserved one is refused.
    #[must_use]
    pub const fn is_defined(self) -> bool {
        matches!(self, Form::Mutation | Form::Delete)
    }

    /// The form's name, as §7.5's table and the fixtures spell it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Form::Mutation => "mutation",
            Form::Delete => "delete",
            Form::Value => "value",
            Form::Error => "error",
            Form::Event => "event",
            Form::Frame => "frame",
            Form::Heartbeat => "heartbeat",
            Form::Snapshot => "snapshot",
        }
    }

    /// The kind of sample a defined form travels on; `None` for a reserved
    /// one, which this version gives no sample.
    const fn sample_kind(self) -> Option<SampleKind> {
        match self {
            Form::Mutation => Some(SampleKind::Put),
            Form::Delete => Some(SampleKind::Delete),
            _ => None,
        }
    }
}

impl std::fmt::Display for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind of the sample that carries an envelope, as zenoh's
/// `SampleKind`: a put carries it as its payload, a delete as its
/// attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleKind {
    Put,
    Delete,
}

impl SampleKind {
    /// `put` or `delete`, as the fixtures spell it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SampleKind::Put => "put",
            SampleKind::Delete => "delete",
        }
    }
}

impl std::fmt::Display for SampleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// CH's four low bits: 0 unless the record's type kind defines a tag. A
/// value from 0 to 15, by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Tag(u8);

impl Tag {
    /// Tag 0: every record whose type kind defines no tag, and every delete.
    pub const NONE: Tag = Tag(0);

    /// A tag, `None` above 15.
    #[must_use]
    pub const fn new(value: u8) -> Option<Tag> {
        if value <= 0x0f {
            Some(Tag(value))
        } else {
            None
        }
    }

    /// The tag's value, from 0 to 15.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// CH, the header byte, read as its three fields. Every byte reads as a
/// header, a reserved form's included, and writes back to itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Header {
    /// Bits 7–5.
    pub form: Form,
    /// Bit 4: a 32-bit run when set, a 16-bit run when clear.
    pub wide: bool,
    /// Bits 3–0.
    pub tag: Tag,
}

impl Header {
    /// CH's fields: the form's code × 32, plus the width bit × 16, plus the
    /// tag. `0x1b` is `000 1 1011`: a mutation, a 32-bit run, tag 11.
    #[must_use]
    pub const fn from_byte(ch: u8) -> Header {
        Header {
            form: Form::ALL[(ch >> 5) as usize],
            wide: ch & 0x10 != 0,
            tag: Tag(ch & 0x0f),
        }
    }

    /// The byte.
    #[must_use]
    pub const fn byte(self) -> u8 {
        (self.form.code() << 5) | ((self.wide as u8) << 4) | self.tag.0
    }
}

/// A run: 16 or 32 bits unsigned, written big-endian, as CH's width bit
/// says. What a run names is for the rule that adopts the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Run {
    U16(u16),
    U32(u32),
}

impl Run {
    /// Whether CH's width bit is set for this run.
    #[must_use]
    pub const fn is_wide(self) -> bool {
        matches!(self, Run::U32(_))
    }

    /// The run's value.
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Run::U16(v) => v as u32,
            Run::U32(v) => v,
        }
    }

    /// Its bytes on the wire: 2 or 4.
    #[must_use]
    pub const fn width_bytes(self) -> usize {
        if self.is_wide() { 4 } else { 2 }
    }
}

/// A seq: 24 bits unsigned, written big-endian. A value from 0 to
/// 16,777,215, by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Seq(u32);

impl Seq {
    /// 16,777,215, the greatest seq.
    pub const MAX: Seq = Seq(0x00ff_ffff);

    /// A seq, `None` above [`Seq::MAX`].
    #[must_use]
    pub const fn new(value: u32) -> Option<Seq> {
        if value <= Seq::MAX.0 {
            Some(Seq(value))
        } else {
            None
        }
    }

    /// The seq's value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An envelope's stamp: its run and its seq. Not §4.3's stamp, which still
/// orders a key's values until a rule adopts the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub run: Run,
    pub seq: Seq,
}

impl Stamp {
    /// CH and the stamp: 6 bytes with a 16-bit run, 8 with a 32-bit one.
    #[must_use]
    pub const fn envelope_bytes(self) -> usize {
        1 + self.run.width_bytes() + 3
    }

    fn write_to(self, out: &mut Vec<u8>) {
        match self.run {
            Run::U16(v) => out.extend_from_slice(&v.to_be_bytes()),
            Run::U32(v) => out.extend_from_slice(&v.to_be_bytes()),
        }
        out.extend_from_slice(&self.seq.0.to_be_bytes()[1..]);
    }
}

/// Why bytes are not an envelope of their sample's kind. The variants are
/// in §7.5's order, which is the order they are found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Refusal {
    /// No byte at all, so no CH.
    #[error("no byte, so no header byte")]
    Empty,
    /// A form this version reserves.
    #[error("the form {0} is reserved")]
    Reserved(Form),
    /// A form the sample's kind does not carry.
    #[error("a {form} envelope on a {sample}")]
    WrongSample { form: Form, sample: SampleKind },
    /// A delete whose tag is not 0.
    #[error("a delete's tag is {}, not 0", .0.get())]
    DeleteTag(Tag),
    /// Fewer bytes than CH, the run and the seq need.
    #[error("{have} bytes, short of the {need} that the header, the run and the seq need")]
    Short { need: usize, have: usize },
    /// Bytes after a delete's seq.
    #[error("{0} bytes after a delete's seq")]
    Trailing(usize),
}

impl Refusal {
    /// The reason token `spec/conformance/envelope/vectors.json` pins:
    /// `short`, `form`, `sample_kind`, `tag` or `trailing`.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            Refusal::Empty | Refusal::Short { .. } => "short",
            Refusal::Reserved(_) => "form",
            Refusal::WrongSample { .. } => "sample_kind",
            Refusal::DeleteTag(_) => "tag",
            Refusal::Trailing(_) => "trailing",
        }
    }
}

/// §7.5's parse, in its order: CH, its form, the sample's kind, a delete's
/// tag, then the stamp. Returns CH, the stamp and every byte after it.
fn read(bytes: &[u8], sample: SampleKind) -> Result<(Header, Stamp, &[u8]), Refusal> {
    let (&ch, rest) = bytes.split_first().ok_or(Refusal::Empty)?;
    let header = Header::from_byte(ch);
    if !header.form.is_defined() {
        return Err(Refusal::Reserved(header.form));
    }
    if header.form.sample_kind() != Some(sample) {
        return Err(Refusal::WrongSample {
            form: header.form,
            sample,
        });
    }
    if header.form == Form::Delete && header.tag != Tag::NONE {
        return Err(Refusal::DeleteTag(header.tag));
    }
    let run_bytes = if header.wide { 4 } else { 2 };
    let need = 1 + run_bytes + 3;
    if bytes.len() < need {
        return Err(Refusal::Short {
            need,
            have: bytes.len(),
        });
    }
    let (run, rest) = rest.split_at(run_bytes);
    let run = match *run {
        [a, b] => Run::U16(u16::from_be_bytes([a, b])),
        [a, b, c, d] => Run::U32(u32::from_be_bytes([a, b, c, d])),
        _ => unreachable!("a run is 2 or 4 bytes"),
    };
    let (seq, rest) = rest.split_at(3);
    let seq = Seq(u32::from_be_bytes([0, seq[0], seq[1], seq[2]]));
    Ok((header, Stamp { run, seq }, rest))
}

/// A put's payload under the envelope. This version defines one form for a
/// put, the mutation: `CH · run · seq · record`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Envelope<'a> {
    /// 0 unless the record's type kind defines a tag.
    pub tag: Tag,
    pub stamp: Stamp,
    /// Every byte after the seq, possibly none, read as the resource's
    /// type says (§7.1).
    pub record: &'a [u8],
}

impl<'a> Envelope<'a> {
    /// A put's payload read as an envelope, or the first refusal that
    /// applies.
    ///
    /// # Errors
    ///
    /// [`Refusal::Empty`], [`Refusal::Reserved`], [`Refusal::WrongSample`]
    /// for a delete's form, or [`Refusal::Short`], in that order.
    pub fn parse(bytes: &'a [u8]) -> Result<Envelope<'a>, Refusal> {
        let (header, stamp, record) = read(bytes, SampleKind::Put)?;
        Ok(Envelope {
            tag: header.tag,
            stamp,
            record,
        })
    }

    /// Its CH: a mutation, the width the run takes, and its tag.
    #[must_use]
    pub const fn header(&self) -> Header {
        Header {
            form: Form::Mutation,
            wide: self.stamp.run.is_wide(),
            tag: self.tag,
        }
    }

    /// Its bytes: 6 over the record, or 8 with a 32-bit run.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.stamp.envelope_bytes() + self.record.len());
        out.push(self.header().byte());
        self.stamp.write_to(&mut out);
        out.extend_from_slice(self.record);
        out
    }
}

/// A delete's attachment under the envelope: `CH · run · seq`, with tag 0
/// and no record. It keeps CH, which states the run's width and lets a
/// reader refuse an attachment that is not an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeleteStamp {
    pub stamp: Stamp,
}

impl DeleteStamp {
    /// A delete's attachment read as an envelope, or the first refusal
    /// that applies.
    ///
    /// # Errors
    ///
    /// [`Refusal::Empty`], [`Refusal::Reserved`], [`Refusal::WrongSample`]
    /// for a mutation's form, [`Refusal::DeleteTag`], [`Refusal::Short`] or
    /// [`Refusal::Trailing`], in that order.
    pub fn parse(bytes: &[u8]) -> Result<DeleteStamp, Refusal> {
        let (_, stamp, rest) = read(bytes, SampleKind::Delete)?;
        if !rest.is_empty() {
            return Err(Refusal::Trailing(rest.len()));
        }
        Ok(DeleteStamp { stamp })
    }

    /// Its CH: a delete, the width the run takes, and tag 0.
    #[must_use]
    pub const fn header(&self) -> Header {
        Header {
            form: Form::Delete,
            wide: self.stamp.run.is_wide(),
            tag: Tag::NONE,
        }
    }

    /// Its bytes: 6, or 8 with a 32-bit run.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.stamp.envelope_bytes());
        out.push(self.header().byte());
        self.stamp.write_to(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_is_a_header_and_writes_back() {
        for ch in 0..=u8::MAX {
            let h = Header::from_byte(ch);
            assert_eq!(h.byte(), ch);
            assert_eq!(h.form.code(), ch >> 5);
            assert_eq!(Form::from_code(h.form.code()), Some(h.form));
        }
        assert_eq!(Form::from_code(8), None);
    }

    #[test]
    fn the_worked_bytes() {
        let h = Header::from_byte(0x1b);
        assert_eq!((h.form, h.wide, h.tag.get()), (Form::Mutation, true, 11));
        assert_eq!(Header::from_byte(0x20).form, Form::Delete);
        assert!(!Header::from_byte(0x20).wide);
    }

    #[test]
    fn bounds_by_construction() {
        assert_eq!(Tag::new(15).map(Tag::get), Some(15));
        assert_eq!(Tag::new(16), None);
        assert_eq!(Seq::new(0x00ff_ffff), Some(Seq::MAX));
        assert_eq!(Seq::new(0x0100_0000), None);
    }

    #[test]
    fn sizes() {
        let narrow = Stamp {
            run: Run::U16(7),
            seq: Seq::MAX,
        };
        let wide = Stamp {
            run: Run::U32(7),
            seq: Seq::MAX,
        };
        let record = [1, 2, 3];
        for (stamp, over) in [(narrow, 6), (wide, 8)] {
            let e = Envelope {
                tag: Tag::NONE,
                stamp,
                record: &record,
            };
            let bytes = e.write();
            assert_eq!(bytes.len(), over + record.len());
            assert_eq!(Envelope::parse(&bytes), Ok(e));
            let d = DeleteStamp { stamp };
            let bytes = d.write();
            assert_eq!(bytes.len(), over);
            assert_eq!(DeleteStamp::parse(&bytes), Ok(d));
        }
    }

    #[test]
    fn each_kind_refuses_the_others_form() {
        let d = DeleteStamp {
            stamp: Stamp {
                run: Run::U16(1),
                seq: Seq::default(),
            },
        }
        .write();
        assert_eq!(
            Envelope::parse(&d),
            Err(Refusal::WrongSample {
                form: Form::Delete,
                sample: SampleKind::Put
            })
        );
        let m = Envelope {
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(1),
                seq: Seq::default(),
            },
            record: &[],
        }
        .write();
        assert_eq!(
            DeleteStamp::parse(&m).map_err(|r| r.token()),
            Err("sample_kind")
        );
    }
}
