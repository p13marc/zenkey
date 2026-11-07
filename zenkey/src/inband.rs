//! A received sample read as core §4.3 writes it (core §7.5, observer
//! §7.3's "the envelope first"; #976): the one place the runtime tells a
//! sample that carries a zk2c envelope, strips it, and reads what orders
//! it.
//!
//! ```text
//! a put on a stream, state or event key, under zenoh/bytes
//!     → its envelope parsed: the record becomes the payload, the run and
//!       seq its in-band stamp (Read::Enveloped)
//!     → or refused: undecodable, never read as a payload with no envelope
//!       (Read::Refused)
//! a delete on such a key whose attachment parses as a delete's envelope
//!     → its in-band stamp (Read::Enveloped, the sample unchanged)
//! a put whose envelope is a heartbeat (core §7.5's form 6, 0.81)
//!     → its body becomes the payload, its run and seq its key's stamp
//!       (Read::Heartbeat): never a record of the key's type
//! a put whose envelope is a frame (core §7.5's form 5, §7.7, 0.83)
//!     → its body becomes the payload, its run and seq record 0's
//!       (Read::Framed): read with its frame resource's declaration
//!       ([`admit_frame`]), never as one record of the key's type
//! a put whose envelope is a snapshot's page (core §7.5's form 7, 0.97)
//!     → its body read (Read::Snapshot): a GET's reader takes its entries
//!       as the per-key replies they stand for ([`replies`]); anywhere
//!       else it is no record (#977's E2)
//! every other sample
//!     → read with no envelope (Read::Plain)
//! ```
//!
//! A sample with no envelope is one core §4.3 writes so: an `@stream`
//! sample, an operation's, a descriptor's or a bundle's, and a foreign
//! key's (§1.7). Any other was put by an owner that broke §4.3's in-band
//! rule (S1, for state), and core §4.2 orders it among samples with no
//! envelope by its Zenoh timestamp ([`Read::mark`]); beside an in-band
//! stamp it is unordered.
//!
//! A reader that knows the resource's type then admits the record
//! ([`admit`]): a packed type's (core §7.6) by its layout, under the tag
//! CH carried, refused by name ([`Undecodable::Record`]) as the model's
//! `Layout::check` refuses it; a packed record that came in no envelope has
//! no tag, and is refused as `tag`.
//!
//! The decision is `zenkey_model::wire`'s, by the key and the Encoding
//! alone, and the order of two marks `zenkey_model::seed`'s. A sample's
//! Zenoh timestamp stays on it: a profile that reads *time* on a resource
//! core §4.3's table lists (freshness's age, timing's lifespan, desired's
//! author order, redundancy's fencing) keeps reading it, and an enveloped
//! sample without one has no time. Its *order* is its in-band stamp
//! ([`Read::mark`]).
//!
//! The consumer ([`crate::consumer`]), its state GET, the archive and the
//! profile readers read every data sample through [`read`]. A heartbeat is
//! read with its covered revision by `zenkey_model::freshness::View`; a
//! reader of a resource's records refuses one ([`admit`],
//! [`Undecodable::Heartbeat`]). A frame (#981's K2) is read by
//! [`admit_frame`] on a frame resource, its body parsed with its layout,
//! and refused anywhere else ([`Undecodable::Framed`]); a mutation on a
//! frame resource has no reading ([`Undecodable::Mutation`]).
//!
//! **A snapshot's page** (#977's E2): only a GET's reply carries one (core
//! §7.5), and the parser cannot tell how a put came. So [`read`] reads its
//! body either way, and a reader of a GET's replies — the consumer's state
//! GET and re-read, an archive's alignment — reads each entry as the
//! per-key reply it stands for ([`replies`]: a put of the entry's record
//! under its stamp on its member's key, or a tombstone's delete). Every
//! other reader admits no record from it ([`Undecodable::Snapshot`]): a
//! page heard on a subscription is no owner's answer. The GET readers keep
//! `Latest` until E3, so a snapshot of several pages arrives there as its
//! last page alone, which its index shows incomplete.

use zenkey_model::collection::{BodyRefusal, Page};
use zenkey_model::frame::{self, Frame};
use zenkey_model::packed::{Layout, ReadRefusal};
use zenkey_model::seed::{self, Mark};
use zenkey_model::wire::{self, Reading, Refusal, Tag};
use zenoh::sample::{Sample, SampleBuilder, SampleBuilderPut, SampleKind};
use zenoh::time::Timestamp;

/// A received sample, read.
#[derive(Debug, Clone)]
pub enum Read {
    /// A sample with no envelope: read as it arrived.
    Plain(Sample),
    /// An envelope's sample: a put's with its record as its payload, a
    /// delete's unchanged, the envelope's run and seq, and CH's tag (a
    /// packed record's type's, core §7.6; 0 otherwise, and for a delete).
    Enveloped {
        sample: Sample,
        stamp: wire::Stamp,
        tag: Tag,
    },
    /// A put whose envelope was refused: undecodable, with the refusal
    /// ([`Refusal::reason`]). The sample is as it arrived, for a caller
    /// that shows it; it is never decoded, nor ordered.
    Refused { sample: Sample, refusal: Refusal },
    /// A heartbeat (core §7.5's form 6): the put with its body as its
    /// payload, its key's run and seq, and CH's tag, its covered
    /// revision's. Never a record of the key's type.
    Heartbeat {
        sample: Sample,
        stamp: wire::Stamp,
        tag: Tag,
    },
    /// A frame (core §7.5's form 5, §7.7): the put with its body as its
    /// payload, its record 0's run and seq, and CH's tag, its frame
    /// form's. Read with its frame resource's declaration
    /// ([`admit_frame`]), never as one record of the key's type.
    Framed {
        sample: Sample,
        stamp: wire::Stamp,
        tag: Tag,
    },
    /// A snapshot's page (core §7.5's form 7): the put as it arrived, and
    /// its body read, or the body's first refusal. A GET's reader reads
    /// its entries ([`replies`]); it is never a record of the key's type,
    /// and has no stamp of its own to order a value by.
    Snapshot {
        sample: Sample,
        page: Result<Box<Page>, BodyRefusal>,
    },
}

impl Read {
    /// The in-band stamp, when the sample carried an envelope.
    #[must_use]
    pub fn stamp(&self) -> Option<wire::Stamp> {
        match self {
            Read::Enveloped { stamp, .. }
            | Read::Heartbeat { stamp, .. }
            | Read::Framed { stamp, .. } => Some(*stamp),
            Read::Plain(_) | Read::Refused { .. } | Read::Snapshot { .. } => None,
        }
    }

    /// CH's tag, when the sample carried an envelope.
    #[must_use]
    pub fn tag(&self) -> Option<Tag> {
        match self {
            Read::Enveloped { tag, .. }
            | Read::Heartbeat { tag, .. }
            | Read::Framed { tag, .. } => Some(*tag),
            Read::Snapshot { page, .. } => page.as_ref().ok().map(|p| p.tag),
            Read::Plain(_) | Read::Refused { .. } => None,
        }
    }

    /// The refusal, when the sample's envelope was refused.
    #[must_use]
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Read::Refused { refusal, .. } => Some(*refusal),
            Read::Plain(_)
            | Read::Enveloped { .. }
            | Read::Heartbeat { .. }
            | Read::Framed { .. }
            | Read::Snapshot { .. } => None,
        }
    }

    /// A snapshot's page, its body read; `None` for any other sample and
    /// for a page whose body is refused.
    #[must_use]
    pub fn page(&self) -> Option<&Page> {
        match self {
            Read::Snapshot { page: Ok(p), .. } => Some(p),
            _ => None,
        }
    }

    /// Whether it is a heartbeat's: its payload is a body, never a record.
    #[must_use]
    pub fn is_heartbeat(&self) -> bool {
        matches!(self, Read::Heartbeat { .. })
    }

    /// Whether it is a frame's: its payload is a body, never one record.
    #[must_use]
    pub fn is_frame(&self) -> bool {
        matches!(self, Read::Framed { .. })
    }

    /// Why it is no record of its key's type: its envelope refused, a
    /// heartbeat's or a frame's. `None` for a record, a delete, or a
    /// sample with no envelope. A frame resource's reader reads a frame
    /// by [`admit_frame`] instead.
    #[must_use]
    pub fn undecodable(&self) -> Option<Undecodable> {
        match self {
            Read::Refused { refusal, .. } => Some(Undecodable::Envelope(*refusal)),
            Read::Heartbeat { .. } => Some(Undecodable::Heartbeat),
            Read::Framed { .. } => Some(Undecodable::Framed),
            Read::Snapshot { page, .. } => {
                Some(Undecodable::Snapshot(page.as_ref().err().copied()))
            }
            Read::Plain(_) | Read::Enveloped { .. } => None,
        }
    }

    /// The sample: an enveloped put's with its record as its payload.
    #[must_use]
    pub fn sample(&self) -> &Sample {
        match self {
            Read::Plain(sample)
            | Read::Enveloped { sample, .. }
            | Read::Refused { sample, .. }
            | Read::Heartbeat { sample, .. }
            | Read::Framed { sample, .. }
            | Read::Snapshot { sample, .. } => sample,
        }
    }

    /// [`Read::sample`], owned.
    #[must_use]
    pub fn into_sample(self) -> Sample {
        match self {
            Read::Plain(sample)
            | Read::Enveloped { sample, .. }
            | Read::Refused { sample, .. }
            | Read::Heartbeat { sample, .. }
            | Read::Framed { sample, .. }
            | Read::Snapshot { sample, .. } => sample,
        }
    }

    /// What orders it: its in-band stamp, else its Zenoh timestamp. A
    /// refused envelope has none: it is never ordered.
    #[must_use]
    pub fn mark(&self) -> Option<Mark> {
        match self {
            Read::Refused { .. } | Read::Snapshot { .. } => None,
            Read::Plain(s) => Mark::of(None, s.timestamp().map(stamp_of)),
            Read::Enveloped { sample, stamp, .. }
            | Read::Heartbeat { sample, stamp, .. }
            | Read::Framed { sample, stamp, .. } => {
                Mark::of(Some(*stamp), sample.timestamp().map(stamp_of))
            }
        }
    }
}

/// Reads a received sample: see the module doc.
#[must_use]
pub fn read(sample: Sample) -> Read {
    let (stamp, form) = match envelope(&sample) {
        None => return Read::Plain(sample),
        Some(Err(refusal)) => return Read::Refused { sample, refusal },
        Some(Ok(read)) => read,
    };
    if form == wire::Form::Snapshot {
        use zenkey_model::collection::PageRefusal;
        // `envelope` parsed this envelope as a snapshot's; a refusal of it
        // here is the envelope's, never a panic on network input
        // (review-bot on #1078).
        let page = Page::parse(&sample.payload().to_bytes());
        let page = match page {
            Ok(p) => Ok(Box::new(p)),
            Err(PageRefusal::Body(b)) => Err(b),
            Err(PageRefusal::Envelope(refusal)) => return Read::Refused { sample, refusal },
            Err(PageRefusal::NotSnapshot(f)) => {
                return Read::Refused {
                    sample,
                    refusal: Refusal::Reserved(f),
                };
            }
        };
        return Read::Snapshot { sample, page };
    }
    if sample.kind() == SampleKind::Delete {
        return Read::Enveloped {
            sample,
            stamp,
            tag: Tag::NONE,
        };
    }
    // `envelope` returned a put's stamp only from `Envelope::parse` of these
    // bytes, which refuses an empty or short payload (`short`): they hold
    // CH and the stamp, `stamp.envelope_bytes()` of them.
    let bytes = sample.payload().to_bytes();
    let tag = wire::Header::from_byte(bytes[0]).tag;
    let record = bytes[stamp.envelope_bytes()..].to_vec();
    let sample = SampleBuilder::<SampleBuilderPut>::try_from(sample)
        .expect("a put")
        .payload(record)
        .into();
    match form {
        wire::Form::Heartbeat => Read::Heartbeat { sample, stamp, tag },
        wire::Form::Frame => Read::Framed { sample, stamp, tag },
        _ => Read::Enveloped { sample, stamp, tag },
    }
}

/// Why a received data sample is undecodable, never delivered nor ordered:
/// its envelope refused (core §7.5), or its packed record refused by the
/// layout of the type its reader holds (§7.6), or a frame's body by its
/// frame resource's (§7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undecodable {
    Envelope(Refusal),
    Record(ReadRefusal),
    /// A heartbeat (core §7.5's form 6) where a record of the resource's
    /// type was read: a body, never a value.
    Heartbeat,
    /// A frame's body refused by its frame resource's layout (core §7.7).
    Frame(frame::ReadRefusal),
    /// A frame on a resource that declares no `frame` table: no reading
    /// (core §7.7).
    Framed,
    /// A mutation, or a sample with no envelope, on a frame resource: no
    /// reading (core §7.7).
    Mutation,
    /// A snapshot's page (core §7.5's form 7) where a record was read: on
    /// a subscription, where it is no owner's answer, or anywhere a GET's
    /// reader does not read its entries; with its body's refusal when the
    /// body is refused, which a GET's reader reports too.
    Snapshot(Option<BodyRefusal>),
}

impl Undecodable {
    /// The fixtures' token: `envelope/vectors.json`'s, or
    /// `packed/vectors.json`'s.
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            Undecodable::Envelope(r) => r.token(),
            Undecodable::Record(r) => r.token(),
            Undecodable::Frame(r) => r.token(),
            Undecodable::Heartbeat => "heartbeat",
            Undecodable::Framed => "frame",
            Undecodable::Mutation => "mutation",
            Undecodable::Snapshot(None) => "snapshot",
            Undecodable::Snapshot(Some(r)) => r.token(),
        }
    }

    /// The reason, in words, naming what refused it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Undecodable::Envelope(r) => r.reason(),
            Undecodable::Record(r) => format!("a packed record refused by its layout, {r}"),
            Undecodable::Heartbeat => {
                "a heartbeat's body where a record of the resource's type was read".to_owned()
            }
            Undecodable::Frame(r) => format!("a frame refused by its layout, {r}"),
            Undecodable::Framed => FRAME_AS_RECORD.to_owned(),
            Undecodable::Mutation => MUTATION_ON_FRAME.to_owned(),
            Undecodable::Snapshot(None) => SNAPSHOT_AS_RECORD.to_owned(),
            Undecodable::Snapshot(Some(r)) => format!("a zk2c snapshot's body refused: {r}"),
        }
    }
}

impl std::fmt::Display for Undecodable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason())
    }
}

/// Admits a read sample as its resource's type reads it, `layout` that
/// type's when it is packed (core §7.6): a refused envelope is refused;
/// a packed put's record is checked under the tag its CH carried, by the
/// layout's checks in §7.6's order (`tag`, `length`, `code`, `extension`,
/// `pad`); a packed put that came in no envelope has no tag, so `tag`. A
/// delete carries no record, and every other type's sample is admitted.
///
/// # Errors
/// The [`Undecodable`] refusal.
pub fn admit(read: &Read, layout: Option<&Layout>) -> Result<(), Undecodable> {
    if let Some(u) = read.undecodable() {
        return Err(u);
    }
    let Some(layout) = layout else {
        return Ok(());
    };
    let sample = read.sample();
    if sample.kind() == SampleKind::Delete {
        return Ok(());
    }
    let tag = read.tag().ok_or(Undecodable::Record(ReadRefusal::Tag))?;
    layout
        .check(tag.get(), &sample.payload().to_bytes())
        .map_err(Undecodable::Record)
}

/// Why a frame on a resource with no `frame` table is undecodable (core
/// §7.7, "No reading").
pub const FRAME_AS_RECORD: &str =
    "a zk2c frame on a resource that declares no `frame` table: it has no reading";

/// Why a snapshot's page where a record was read is undecodable (core
/// §7.5): only a GET's reply carries one, and its entries are read by the
/// GET's reader ([`replies`]), never as one record.
pub const SNAPSHOT_AS_RECORD: &str =
    "a zk2c snapshot's page where a record was read: only a GET's reader reads its entries";

/// Why a mutation on a frame resource is undecodable (core §7.7, "No
/// reading"): its samples travel as frames.
pub const MUTATION_ON_FRAME: &str =
    "a record that came in no frame, on a frame resource: it has no reading";

/// Admits a read sample of a frame resource, `layout` its frame layout
/// (core §7.7): a frame's body parsed under the tag its CH carried, for
/// the first of §7.7's refusals; a refused envelope refused; a mutation,
/// or a put with no envelope, refused as having no reading. `Ok(None)` for
/// a delete, which carries no frame.
///
/// # Errors
/// The [`Undecodable`] refusal.
pub fn admit_frame(read: &Read, layout: &frame::Layout) -> Result<Option<Frame>, Undecodable> {
    match read {
        Read::Framed { sample, tag, .. } => {
            Frame::parse(*tag, &sample.payload().to_bytes(), layout)
                .map(Some)
                .map_err(Undecodable::Frame)
        }
        Read::Refused { refusal, .. } => Err(Undecodable::Envelope(*refusal)),
        Read::Heartbeat { .. } => Err(Undecodable::Heartbeat),
        Read::Snapshot { page, .. } => Err(Undecodable::Snapshot(page.as_ref().err().copied())),
        r if r.sample().kind() == SampleKind::Delete => Ok(None),
        Read::Enveloped { .. } | Read::Plain(_) => Err(Undecodable::Mutation),
    }
}

/// What a sample's envelope says: `None` for a sample that carries none,
/// else its stamp and form (a put's record or body is every byte after
/// it) or the refusal.
fn envelope(sample: &Sample) -> Option<Result<(wire::Stamp, wire::Form), Refusal>> {
    let key = sample.key_expr().as_str();
    match sample.kind() {
        SampleKind::Delete => {
            let attachment = sample.attachment()?.to_bytes();
            wire::read_delete(key, Some(&attachment)).map(|s| Ok((s, wire::Form::Delete)))
        }
        SampleKind::Put => {
            if !wire::carries(key, &sample.encoding().to_string()) {
                return None;
            }
            let bytes = sample.payload().to_bytes();
            Some(match wire::read_put(key, wire::ENCODING, &bytes) {
                Reading::Enveloped(e)
                | Reading::Heartbeat(e)
                | Reading::Framed(e)
                | Reading::Snapshot(e) => Ok((e.stamp, e.form)),
                Reading::Refused(refusal) => Err(refusal),
                Reading::Plain => return None,
            })
        }
    }
}

/// A zenoh timestamp as the model orders it: its NTP64 time, and its id's
/// 128-bit value (`zenkey_model::seed::Stamp`, whose order is zenoh's).
#[must_use]
pub fn stamp_of(ts: &Timestamp) -> seed::Stamp {
    seed::Stamp {
        time: ts.get_time().as_u64(),
        id: u128::from_le_bytes(ts.get_id().to_le_bytes()),
    }
}

/// A sample's in-band stamp, read without stripping it: its envelope's
/// when it carried one ([`Read::stamp`]). A snapshot's page has none: its
/// run and asof order no value.
#[must_use]
pub fn stamp(sample: &Sample) -> Option<wire::Stamp> {
    envelope(sample)?
        .ok()
        .filter(|(_, f)| *f != wire::Form::Snapshot)
        .map(|(s, _)| s)
}

/// Whether a sample is a snapshot's page (core §7.5's form 7), read
/// without stripping it: no mutation, whatever it carries.
#[must_use]
pub fn is_snapshot(sample: &Sample) -> bool {
    matches!(envelope(sample), Some(Ok((_, wire::Form::Snapshot))))
}

/// The per-key replies a snapshot's page stands for (#977's E2; core 0.97
/// §4.2's "what a profile reads", E3's text): one per entry, on its
/// member's key below the page's key, as the owner would have answered it
/// per key — a live entry a put of `CH · run · seq · record` under
/// `zenoh/bytes`, CH a mutation's with the page's tag, and a tombstone a
/// delete whose attachment is its envelope. [`read`] reads each as the
/// reply it stands for. They carry no Zenoh timestamp, since a snapshot
/// carries none, and the page's QoS.
#[must_use]
pub fn replies(sample: &Sample, page: &Page) -> Vec<Sample> {
    use zenoh::bytes::Encoding;
    use zenoh::key_expr::KeyExpr;
    page.members(sample.key_expr().as_str())
        .into_iter()
        .filter_map(|m| {
            // Plain chunks below a key make a key: never refused here.
            let key = KeyExpr::try_from(m.key.clone()).ok()?;
            Some(match (m.payload(), m.attachment()) {
                (Some(payload), _) => SampleBuilder::put(key, payload)
                    .encoding(Encoding::ZENOH_BYTES)
                    .into(),
                (None, attachment) => SampleBuilder::delete(key).attachment(attachment).into(),
            })
        })
        .collect()
}

/// [`replies`] for a reader that ages what it reads (`freshness.v2` §2.6,
/// text 0.6; #977's E3): a live entry with a confirmation whose time is
/// known (1 at the snapshot's time, 2 at its conf; core §7.5) carries it
/// as its Zenoh timestamp, counted from the deployment epoch `epoch` in
/// seconds, so the reader ages it as it aged a reply from its stamp. Its id
/// names the owner's clock, which built the snapshot, by the owner's
/// address ([`owner_clock`]): a snapshot carries no zid. Every other entry
/// carries none.
#[must_use]
pub fn replies_confirmed(sample: &Sample, page: &Page, epoch: i64) -> Vec<Sample> {
    use zenkey_model::collection::Confirmation;
    use zenoh::bytes::Encoding;
    use zenoh::key_expr::KeyExpr;
    let clock = owner_clock(sample.key_expr().as_str());
    page.members(sample.key_expr().as_str())
        .into_iter()
        .filter_map(|m| {
            let key = KeyExpr::try_from(m.key.clone()).ok()?;
            Some(match (m.payload(), m.attachment()) {
                (Some(payload), _) => {
                    let time = match m.confirmation {
                        Confirmation::None => None,
                        Confirmation::AtTime => Some(page.time),
                        Confirmation::At(t) => Some(t),
                    };
                    let ts = time
                        .and_then(|t| t.unix_ms(epoch))
                        .and_then(|ms| u64::try_from(ms).ok())
                        .map(|ms| {
                            Timestamp::new(
                                zenoh::time::NTP64::from(std::time::Duration::from_millis(ms)),
                                clock,
                            )
                        });
                    SampleBuilder::put(key, payload)
                        .encoding(Encoding::ZENOH_BYTES)
                        .timestamp(ts)
                        .into()
                }
                (None, attachment) => SampleBuilder::delete(key).attachment(attachment).into(),
            })
        })
        .collect()
}

/// The id a snapshot's confirmations are stamped under
/// ([`replies_confirmed`]): the owner's clock, named by the owner's
/// address in `key`, a namespace in front skipped (or the key itself),
/// mixed into 16 bytes. A snapshot
/// names no zid, so no measurement of the owner's session matches it; the
/// deployment's word trusts it as any other (`freshness.v2` §2.6).
#[must_use]
pub fn owner_clock(key: &str) -> zenoh::time::TimestampId {
    // The data key from its first `zk2` chunk on, a namespace in front
    // skipped, so a key read in one and the same key read in none agree.
    let mut owner = key.to_owned();
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(zenkey_model::grammar::ZkKey::Data { addr, .. }) =
                zenkey_model::grammar::parse(k)
        {
            owner = addr.to_string();
            break;
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    let mix = |seed: u64| {
        owner.bytes().fold(seed, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    };
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&mix(0xcbf2_9ce4_8422_2325).to_le_bytes());
    id[8..].copy_from_slice(&mix(0x8422_2325_cbf2_9ce4).to_le_bytes());
    id[15] |= 1;
    zenoh::time::TimestampId::try_from(id).expect("16 bytes, not all zero")
}

/// Why a sample's envelope was refused, read without stripping it
/// ([`Read::refusal`]).
#[must_use]
pub fn refusal(sample: &Sample) -> Option<Refusal> {
    envelope(sample)?.err()
}

#[cfg(test)]
mod tests {
    use super::{Read, Undecodable, admit, read};
    use zenoh::sample::{Sample, SampleBuilder};

    /// An empty or short put on an enveloped key (zenoh's default Encoding,
    /// `zenoh/bytes`) is refused `short` before anything indexes its bytes,
    /// and `admit` refuses it whatever layout the reader holds.
    #[test]
    fn a_short_put_is_refused_never_read() {
        let layout = zenkey_model::packed::layout_in(
            &serde_json::json!({"types": {"T": {"fields": [{"name": "b", "type": "bool"}]}}}),
            "T",
        )
        .unwrap();
        for payload in [vec![], vec![0x00, 0x01, 0x02]] {
            let key = zenoh::key_expr::KeyExpr::try_from("zk2/lab/a/packed.v1/stream/x").unwrap();
            let s: Sample = SampleBuilder::put(key, payload.clone()).into();
            let r = read(s);
            assert!(matches!(r, Read::Refused { .. }), "{payload:?}: {r:?}");
            assert_eq!(r.refusal().map(|r| r.token()), Some("short"), "{payload:?}");
            for l in [None, Some(&layout)] {
                assert!(
                    matches!(admit(&r, l), Err(Undecodable::Envelope(_))),
                    "{payload:?}"
                );
            }
        }
    }
}
