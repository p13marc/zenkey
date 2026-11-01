//! The zk2c envelope (core §7.5, 0.72; #976, epic #975): the bytes that
//! carry an owner's run and sequence number with a sample, before its
//! record.
//!
//! ```text
//! mutation, a put's payload        CH · run · seq · record
//! delete, a delete's attachment    CH · run · seq
//! heartbeat, a put's payload       CH · run · seq · body        (core 0.81, #978's F1)
//! frame, a put's payload           CH · run · seq · body        (core 0.83, #981's K1)
//! CH                               form (bits 7–5) · run width (bit 4) · tag (bits 3–0)
//! run                              16 or 32 bits unsigned, big-endian, as CH's width bit says
//! seq                              24 bits unsigned, big-endian
//! record                           every byte after the seq, read as the resource's type says
//! ```
//!
//! **Adopted by core §4.3** (0.73, #976's flip): an owner writes the
//! envelope on every put of a `stream`, `state`, `@state` or `events` key
//! and every delete of a state member, an `@stream` sample's excepted. What
//! a run names and how a seq counts are §4.3's, and the order of two
//! in-band stamps [`crate::seed`]'s; this module fixes the bytes, a
//! descriptor's spelling of a run ([`Run::hex`]), and which samples carry
//! an envelope.
//!
//! **Parsing reads no contract,** as parsing a key does (§1.1): it knows
//! the sample's kind, put ([`Envelope::parse`]) or delete
//! ([`DeleteStamp::parse`]), and nothing more. So a mutation's tag is
//! reported, never checked against a type; `packed` (#979) is the first
//! type kind to define one. A put carries a mutation, a heartbeat or a
//! frame ([`Envelope::form`]); a heartbeat's body is read with its covered
//! revision ([`crate::heartbeat`]), its tag that revision's, and a frame's
//! with its declaration ([`crate::frame`]), its tag that declaration's.
//! [`read_put`] refuses a frame on a key whose kind token is not `stream`
//! (`key_kind`), which [`Envelope::parse_on`] reads. The first refusal that
//! applies is reported, in §7.5's order, with the token
//! `spec/conformance/envelope/` pins ([`Refusal::token`]). Every envelope
//! accepted writes back to its bytes.
//!
//! Not [`crate::envelope`], the error envelope of a failed call (§5.2).
//!
//! **Reading** (observer §7.3, "the envelope first"): a reader tells a
//! sample that carries an envelope by its key and its Encoding alone
//! ([`carries`]), reads a put's payload through [`read_put`] and a
//! delete's attachment through [`read_delete`], and reads every other
//! sample as one with no envelope: an `@stream` sample, a foreign key's
//! (§1.7), or one whose owner broke §4.3. The order of two in-band stamps
//! is [`crate::seed`]'s, by the key's kind ([`enveloped_kind`]), and a
//! sample with no envelope is ordered by its Zenoh timestamp (core §4.2).

use thiserror::Error;

use crate::grammar::{KindToken, ZkKey, parse};

/// A form: CH's three high bits. This version defines a mutation, a
/// delete, a frame and a heartbeat, and reserves the other four for later
/// versions, each refused.
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
    /// Code 5: a stream's records under one header, a put's payload on a
    /// plain stream's key (core 0.83 §7.7, [`crate::frame`]).
    Frame,
    /// Code 6: an owner's confirmation of the state it holds for one
    /// interface, a put's payload (core 0.81, [`crate::heartbeat`]).
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
        matches!(
            self,
            Form::Mutation | Form::Delete | Form::Frame | Form::Heartbeat
        )
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
    /// one, which this version gives no sample. A frame and a heartbeat are
    /// a put's payload, so each is refused on a delete's attachment.
    #[must_use]
    pub const fn sample_kind(self) -> Option<SampleKind> {
        match self {
            Form::Mutation | Form::Frame | Form::Heartbeat => Some(SampleKind::Put),
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
/// says. An instance's ordering epoch (core §4.3): a 16-bit run is a
/// counter in the owner's record, a 32-bit run a random draw. Two runs of
/// different widths are two runs, whatever their values, and nothing
/// orders two runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Run {
    U16(u16),
    U32(u32),
}

impl std::fmt::Display for Run {
    /// Its descriptor spelling ([`Run::hex`]).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Run::U16(v) => write!(f, "{v:04x}"),
            Run::U32(v) => write!(f, "{v:08x}"),
        }
    }
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

    /// The run as a descriptor states it (core §3.3): its bytes in
    /// lowercase hexadecimal, 4 digits for a 16-bit run and 8 for a 32-bit
    /// one. So `00a1` and `000000a1` are two runs.
    #[must_use]
    pub fn hex(self) -> String {
        self.to_string()
    }

    /// A descriptor's `run` read back: exactly 4 or 8 lowercase hex
    /// digits, the width from their count. `None` for any other spelling,
    /// which a descriptor checker reports as D002.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Run> {
        let lower_hex = text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !lower_hex {
            return None;
        }
        match text.len() {
            4 => u16::from_str_radix(text, 16).ok().map(Run::U16),
            8 => u32::from_str_radix(text, 16).ok().map(Run::U32),
            _ => None,
        }
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

/// An envelope's stamp, the sample's in-band stamp (core §4.3): its run
/// and its seq, which order a key's values within one run.
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
    /// A frame on a key whose kind token is not `stream` (core 0.83).
    #[error("a frame on a {0} key, which is no plain stream's")]
    KeyKind(KindToken),
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
    /// The reason token `spec/conformance/envelope/vectors.json` and
    /// `carries.json` pin: `short`, `form`, `sample_kind`, `key_kind`,
    /// `tag` or `trailing`.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            Refusal::Empty | Refusal::Short { .. } => "short",
            Refusal::Reserved(_) => "form",
            Refusal::WrongSample { .. } => "sample_kind",
            Refusal::KeyKind(_) => "key_kind",
            Refusal::DeleteTag(_) => "tag",
            Refusal::Trailing(_) => "trailing",
        }
    }
}

/// §7.5's parse, in its order: CH, its form, the sample's kind, a frame's
/// key kind (when `kind` is known), a delete's tag, then the stamp. Returns
/// CH, the stamp and every byte after it.
fn read(
    bytes: &[u8],
    sample: SampleKind,
    kind: Option<KindToken>,
) -> Result<(Header, Stamp, &[u8]), Refusal> {
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
    if header.form == Form::Frame
        && let Some(k) = kind
        && k != KindToken::Stream
    {
        return Err(Refusal::KeyKind(k));
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

/// A put's payload under the envelope. This version defines two forms for
/// a put: the mutation, `CH · run · seq · record`, the frame,
/// `CH · run · seq · body` (core 0.83), and the heartbeat,
/// `CH · run · seq · body` (core 0.81).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Envelope<'a> {
    /// [`Form::Mutation`], [`Form::Frame`] or [`Form::Heartbeat`].
    pub form: Form,
    /// A mutation's: 0 unless the record's type kind defines a tag. A
    /// frame's: its declaration's ([`crate::frame::Layout::tag`]). A
    /// heartbeat's: its covered revision's.
    pub tag: Tag,
    pub stamp: Stamp,
    /// Every byte after the seq, possibly none: a mutation's record, read
    /// as the resource's type says (§7.1), or a frame's or a heartbeat's
    /// body.
    pub record: &'a [u8],
}

impl<'a> Envelope<'a> {
    /// A mutation: a put's record under its stamp.
    #[must_use]
    pub const fn mutation(tag: Tag, stamp: Stamp, record: &'a [u8]) -> Envelope<'a> {
        Envelope {
            form: Form::Mutation,
            tag,
            stamp,
            record,
        }
    }

    /// A frame: a body under the stamp of its record 0 (core §7.7).
    #[must_use]
    pub const fn frame(tag: Tag, stamp: Stamp, body: &'a [u8]) -> Envelope<'a> {
        Envelope {
            form: Form::Frame,
            tag,
            stamp,
            record: body,
        }
    }

    /// Whether it is a frame's: its bytes after the seq are a body, never a
    /// record.
    #[must_use]
    pub const fn is_frame(&self) -> bool {
        matches!(self.form, Form::Frame)
    }

    /// Whether it is a heartbeat's: its bytes after the seq are a body,
    /// never a record.
    #[must_use]
    pub const fn is_heartbeat(&self) -> bool {
        matches!(self.form, Form::Heartbeat)
    }

    /// A put's payload read as an envelope, or the first refusal that
    /// applies. The key is not read, so a frame is accepted on any key:
    /// [`Envelope::parse_on`] reads its key's kind.
    ///
    /// # Errors
    ///
    /// [`Refusal::Empty`], [`Refusal::Reserved`], [`Refusal::WrongSample`]
    /// for a delete's form, or [`Refusal::Short`], in that order.
    pub fn parse(bytes: &'a [u8]) -> Result<Envelope<'a>, Refusal> {
        Self::read_with(bytes, None)
    }

    /// [`Envelope::parse`] for a put on a key of the kind `kind`: a frame on
    /// a key whose kind is not `stream` is [`Refusal::KeyKind`], after the
    /// sample's kind and before the stamp (core §7.5).
    ///
    /// # Errors
    ///
    /// [`Envelope::parse`]'s, and [`Refusal::KeyKind`] after
    /// [`Refusal::WrongSample`].
    pub fn parse_on(bytes: &'a [u8], kind: KindToken) -> Result<Envelope<'a>, Refusal> {
        Self::read_with(bytes, Some(kind))
    }

    fn read_with(bytes: &'a [u8], kind: Option<KindToken>) -> Result<Envelope<'a>, Refusal> {
        let (header, stamp, record) = read(bytes, SampleKind::Put, kind)?;
        Ok(Envelope {
            form: header.form,
            tag: header.tag,
            stamp,
            record,
        })
    }

    /// Its CH: its form, the width the run takes, and its tag.
    #[must_use]
    pub const fn header(&self) -> Header {
        Header {
            form: self.form,
            wide: self.stamp.run.is_wide(),
            tag: self.tag,
        }
    }

    /// Its bytes: 6 over the record or the body, or 8 with a 32-bit run.
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
        let (_, stamp, rest) = read(bytes, SampleKind::Delete, None)?;
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

/// The Encoding a sample that carries an envelope has (§7.5), as zenoh
/// writes it: its default, with no schema. A raw media type zenoh does not
/// predefine travels as its custom encoding, which zenoh writes as the
/// media type itself (§7.2), so it is never this.
pub const ENCODING: &str = "zenoh/bytes";

/// Whether `key` is a zk2 data key whose kind is a stream, state or an
/// event (§1.1, by the grammar alone, with no contract): the keys whose
/// samples may carry an envelope ([`enveloped_kind`]). An operation's
/// (`@op`) and every control key (`@zk`) keep their wire.
#[must_use]
pub fn enveloped_key(key: &str) -> bool {
    enveloped_kind(key).is_some()
}

/// The kind token of a key whose samples may carry an envelope: a zk2 data
/// key whose kind is a stream, state or an event, by the grammar alone.
/// `None` for an operation's key, every control key (`@zk`), a foreign key
/// and a key the grammar refuses. The kind picks how two seqs of one run
/// compare ([`crate::seed::Order::of_kind`]).
///
/// A session in a namespace hears a key base-relative, and one in none
/// hears it with the namespace in front (§1.6), which is any chunks: the
/// key is read from its first `zk2` chunk from which the grammar parses a
/// zk2 key, so another deployment's key reads as its own.
#[must_use]
pub fn enveloped_kind(key: &str) -> Option<KindToken> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/") {
            match parse(k) {
                Ok(ZkKey::Data { kind, .. }) => {
                    return (kind != KindToken::Op).then_some(kind);
                }
                Ok(_) => return None,
                Err(_) => {}
            }
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}

/// Whether a sample carries an envelope: its key is a zk2 data key whose
/// kind is a stream, state or an event ([`enveloped_key`]), and its
/// Encoding is `zenoh/bytes` ([`ENCODING`]). An owner sends `zenoh/bytes`
/// on such a key only with an envelope (core §7.2 asks the predefined
/// Encoding of every other sample, and a raw type keeps its media type), so
/// the test tells the two wires apart. Every other sample is read as one
/// with no envelope (observer §7.3, "Every other sample").
#[must_use]
pub fn carries(key: &str, encoding: &str) -> bool {
    encoding == ENCODING && enveloped_key(key)
}

/// What a put's payload reads as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading<'a> {
    /// The sample carries no envelope ([`carries`]): its payload is read as
    /// it arrived, by observer §7.3's decode order.
    Plain,
    /// A mutation's envelope: its stamp orders the sample, and its record is
    /// the payload every later step reads (decode, a schema check, a
    /// rendering, a profile's reader).
    Enveloped(Envelope<'a>),
    /// A frame (core 0.83 §7.7, form 5) on a plain stream's key: its body
    /// is read with its frame resource's declaration
    /// ([`crate::frame::Frame::parse`]), never decoded as a record of the
    /// key's type. Its stamp is its record 0's.
    Framed(Envelope<'a>),
    /// A heartbeat (core 0.81, form 6): its body is read with its covered
    /// revision ([`crate::heartbeat::Heartbeat::parse`]), never decoded as
    /// a record of the key's type. Its stamp is its key's.
    Heartbeat(Envelope<'a>),
    /// An envelope refused: the sample is undecodable, with the refusal's
    /// token as its reason ([`Refusal::reason`]). It is counted and shown
    /// as such, never decoded or sniffed as a payload with no envelope
    /// (observer §7.3's MUST NOT), and never ordered.
    Refused(Refusal),
}

/// A put's payload read as its key and Encoding say ([`carries`]).
#[must_use]
pub fn read_put<'a>(key: &str, encoding: &str, payload: &'a [u8]) -> Reading<'a> {
    if !carries(key, encoding) {
        return Reading::Plain;
    }
    let parsed = match enveloped_kind(key) {
        Some(kind) => Envelope::parse_on(payload, kind),
        None => Envelope::parse(payload),
    };
    match parsed {
        Ok(e) if e.is_frame() => Reading::Framed(e),
        Ok(e) if e.is_heartbeat() => Reading::Heartbeat(e),
        Ok(e) => Reading::Enveloped(e),
        Err(r) => Reading::Refused(r),
    }
}

/// A delete's in-band stamp: on a key whose samples may carry an envelope
/// ([`enveloped_key`]), an attachment that parses as a delete's envelope.
/// A delete whose attachment is absent or does not parse is read as one
/// with no envelope, by its Zenoh timestamp (core §4.2): an archive's or a
/// profile's attachment (`telemetry.v1`'s Epoch) is never refused here.
#[must_use]
pub fn read_delete(key: &str, attachment: Option<&[u8]>) -> Option<Stamp> {
    let a = attachment?;
    if !enveloped_key(key) {
        return None;
    }
    DeleteStamp::parse(a).ok().map(|d| d.stamp)
}

impl Refusal {
    /// Why a refused envelope's sample is undecodable, for a person and a
    /// script alike: the refusal's token after a fixed prefix,
    /// `zk2c envelope refused: short`.
    #[must_use]
    pub fn reason(&self) -> String {
        format!("zk2c envelope refused: {}", self.token())
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
                form: Form::Mutation,
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

    /// A run as a descriptor states it (core §3.3): 4 lowercase hex digits
    /// for a 16-bit run, 8 for a 32-bit one, and nothing else read back.
    #[test]
    fn a_run_as_a_descriptor_states_it() {
        assert_eq!(Run::U16(3).hex(), "0003");
        assert_eq!(Run::U16(0xa1).hex(), "00a1");
        assert_eq!(Run::U32(0xa1).hex(), "000000a1");
        assert_eq!(Run::U32(u32::MAX).hex(), "ffffffff");
        for r in [
            Run::U16(0),
            Run::U16(u16::MAX),
            Run::U32(0),
            Run::U32(0xdead_beef),
        ] {
            assert_eq!(Run::from_hex(&r.hex()), Some(r));
        }
        assert_eq!(Run::from_hex("00a1"), Some(Run::U16(0xa1)));
        assert_eq!(Run::from_hex("000000a1"), Some(Run::U32(0xa1)));
        for bad in [
            "",
            "1a2b3",
            "ABCD",
            "00A1",
            "abc",
            "+0a1",
            "-001",
            "000000a1f",
            " 0a1",
            "g000",
        ] {
            assert_eq!(Run::from_hex(bad), None, "{bad:?}");
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
            form: Form::Mutation,
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

    /// Rule 1 (#976's A2): a stream, state or event key of the grammar,
    /// under `zenoh/bytes`, and nothing else.
    #[test]
    fn a_sample_carries_an_envelope_by_its_key_kind_and_its_encoding() {
        for key in [
            "zk2/h/s/i.v1/stream/pose",
            "zk2/h/s/i.v1/@stream/frames/f1",
            "zk2/h/s/i.v1/state/mode",
            "zk2/h/s/i.v1/@state/tracks/t1",
            "zk2/h/s/i.v1/events/fault/01arz3ndektsv4rrffq69g5fav",
            // A session in no namespace hears the namespace in front.
            "prod/zk2/h/s/i.v1/state/mode",
            "site/a/zk2/h/s/i.v1/stream/pose",
            // A namespace whose own chunk is `zk2`: the first suffix the
            // grammar parses decides.
            "zk2/zk2/h/s/i.v1/state/mode",
        ] {
            assert!(enveloped_key(key), "{key}");
            assert!(enveloped_kind(key).is_some(), "{key}");
            assert!(carries(key, ENCODING), "{key}");
            for other in [
                "application/json",
                "application/cbor",
                "application/protobuf",
                "zenoh/bytes;x",
                "image/x-portable-anymap",
                "",
            ] {
                assert!(!carries(key, other), "{key} under {other:?}");
            }
        }
        for key in [
            "zk2/h/s/i.v1/@op/reset",
            "zk2/h/s/@zk/instance/0a1b2c3d4e5f6071",
            "zk2/@zk/contract/i.v1/0000000000000000000000000000000000000000000000000000000000000000",
            "demo/tick",
            "seedtest/state/health",
            "zk2/h/s/i.v1",
            "zk2",
            "",
        ] {
            assert!(!enveloped_key(key), "{key}");
            assert_eq!(enveloped_kind(key), None, "{key}");
            assert!(!carries(key, ENCODING), "{key}");
        }
        assert_eq!(
            enveloped_kind("prod/zk2/h/s/i.v1/@stream/frames/f1"),
            Some(KindToken::ExplicitStream)
        );
        assert_eq!(
            enveloped_kind("zk2/h/s/i.v1/events/fault/01arz3ndektsv4rrffq69g5fav"),
            Some(KindToken::Events)
        );
    }

    #[test]
    fn a_put_reads_plain_enveloped_or_refused() {
        let key = "zk2/h/s/i.v1/state/mode";
        let stamp = Stamp {
            run: Run::U16(7),
            seq: Seq::new(3).unwrap(),
        };
        let bytes = Envelope {
            form: Form::Mutation,
            tag: Tag::NONE,
            stamp,
            record: br#"{"up":true}"#,
        }
        .write();
        // Today's wire: the predefined Encoding, read as it is.
        assert_eq!(read_put(key, "application/json", &bytes), Reading::Plain);
        let Reading::Enveloped(e) = read_put(key, ENCODING, &bytes) else {
            panic!("an envelope")
        };
        assert_eq!((e.stamp, e.record), (stamp, &br#"{"up":true}"#[..]));
        // A legacy payload under `zenoh/bytes` is refused, never read: `{`
        // is form 3, reserved.
        let refused = read_put(key, ENCODING, br#"{"up":true}"#);
        assert_eq!(refused, Reading::Refused(Refusal::Reserved(Form::Error)));
        let Reading::Refused(r) = refused else {
            unreachable!()
        };
        assert_eq!(r.reason(), "zk2c envelope refused: form");
        assert_eq!(
            read_put(key, ENCODING, &[]),
            Reading::Refused(Refusal::Empty)
        );
        // A foreign key under `zenoh/bytes` is its own bytes.
        assert_eq!(read_put("demo/tick", ENCODING, &bytes), Reading::Plain);
    }

    /// Rule 3: a delete's attachment that parses carries its stamp; any
    /// other is read as one with no envelope.
    #[test]
    fn a_delete_carries_its_stamp_in_its_attachment_or_none() {
        let key = "zk2/h/s/i.v1/state/mode";
        let stamp = Stamp {
            run: Run::U32(0xdead_beef),
            seq: Seq::new(9).unwrap(),
        };
        let att = DeleteStamp { stamp }.write();
        assert_eq!(read_delete(key, Some(&att)), Some(stamp));
        assert_eq!(read_delete(key, None), None);
        // An archive's JSON, or a mutation's bytes, is no delete stamp.
        assert_eq!(read_delete(key, Some(br#"{"confirmed":true}"#)), None);
        assert_eq!(read_delete(key, Some(&[0x00, 0, 7, 0, 0, 9])), None);
        assert_eq!(read_delete("demo/tick", Some(&att)), None);
        assert_eq!(read_delete("zk2/h/s/i.v1/@op/reset", Some(&att)), None);
    }
}
