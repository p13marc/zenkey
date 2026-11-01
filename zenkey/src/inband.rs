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
//! profile readers read every data sample through [`read`].

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
}

impl Read {
    /// The in-band stamp, when the sample carried an envelope.
    #[must_use]
    pub fn stamp(&self) -> Option<wire::Stamp> {
        match self {
            Read::Enveloped { stamp, .. } => Some(*stamp),
            Read::Plain(_) | Read::Refused { .. } => None,
        }
    }

    /// CH's tag, when the sample carried an envelope.
    #[must_use]
    pub fn tag(&self) -> Option<Tag> {
        match self {
            Read::Enveloped { tag, .. } => Some(*tag),
            Read::Plain(_) | Read::Refused { .. } => None,
        }
    }

    /// The refusal, when the sample's envelope was refused.
    #[must_use]
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Read::Refused { refusal, .. } => Some(*refusal),
            Read::Plain(_) | Read::Enveloped { .. } => None,
        }
    }

    /// The sample: an enveloped put's with its record as its payload.
    #[must_use]
    pub fn sample(&self) -> &Sample {
        match self {
            Read::Plain(sample) | Read::Enveloped { sample, .. } | Read::Refused { sample, .. } => {
                sample
            }
        }
    }

    /// [`Read::sample`], owned.
    #[must_use]
    pub fn into_sample(self) -> Sample {
        match self {
            Read::Plain(sample) | Read::Enveloped { sample, .. } | Read::Refused { sample, .. } => {
                sample
            }
        }
    }

    /// What orders it: its in-band stamp, else its Zenoh timestamp. A
    /// refused envelope has none: it is never ordered.
    #[must_use]
    pub fn mark(&self) -> Option<Mark> {
        match self {
            Read::Refused { .. } => None,
            Read::Plain(s) => Mark::of(None, s.timestamp().map(stamp_of)),
            Read::Enveloped { sample, stamp, .. } => {
                Mark::of(Some(*stamp), sample.timestamp().map(stamp_of))
            }
        }
    }
}

/// Reads a received sample: see the module doc.
#[must_use]
pub fn read(sample: Sample) -> Read {
    let stamp = match envelope(&sample) {
        None => return Read::Plain(sample),
        Some(Err(refusal)) => return Read::Refused { sample, refusal },
        Some(Ok(stamp)) => stamp,
    };
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
    Read::Enveloped { sample, stamp, tag }
}

/// Why a received data sample is undecodable, never delivered nor ordered:
/// its envelope refused (core §7.5), or its packed record refused by the
/// layout of the type its reader holds (§7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undecodable {
    Envelope(Refusal),
    Record(ReadRefusal),
}

impl Undecodable {
    /// The fixtures' token: `envelope/vectors.json`'s, or
    /// `packed/vectors.json`'s.
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            Undecodable::Envelope(r) => r.token(),
            Undecodable::Record(r) => r.token(),
        }
    }

    /// The reason, in words, naming what refused it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Undecodable::Envelope(r) => r.reason(),
            Undecodable::Record(r) => format!("a packed record refused by its layout, {r}"),
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
    if let Some(r) = read.refusal() {
        return Err(Undecodable::Envelope(r));
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

/// What a sample's envelope says: `None` for a sample that carries none,
/// else its stamp (a put's record is every byte after it) or the refusal.
fn envelope(sample: &Sample) -> Option<Result<wire::Stamp, Refusal>> {
    let key = sample.key_expr().as_str();
    match sample.kind() {
        SampleKind::Delete => {
            let attachment = sample.attachment()?.to_bytes();
            wire::read_delete(key, Some(&attachment)).map(Ok)
        }
        SampleKind::Put => {
            if !wire::carries(key, &sample.encoding().to_string()) {
                return None;
            }
            let bytes = sample.payload().to_bytes();
            Some(match wire::read_put(key, wire::ENCODING, &bytes) {
                Reading::Enveloped(e) => Ok(e.stamp),
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
/// when it carried one ([`Read::stamp`]).
#[must_use]
pub fn stamp(sample: &Sample) -> Option<wire::Stamp> {
    envelope(sample)?.ok()
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
