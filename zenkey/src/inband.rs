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

use zenkey_model::seed::{self, Mark};
use zenkey_model::wire::{self, Reading, Refusal};
use zenoh::sample::{Sample, SampleBuilder, SampleBuilderPut, SampleKind};
use zenoh::time::Timestamp;

/// A received sample, read.
#[derive(Debug, Clone)]
pub enum Read {
    /// A sample with no envelope: read as it arrived.
    Plain(Sample),
    /// An envelope's sample: a put's with its record as its payload, a
    /// delete's unchanged, and the envelope's run and seq.
    Enveloped { sample: Sample, stamp: wire::Stamp },
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
            Read::Enveloped { sample, stamp } => {
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
        return Read::Enveloped { sample, stamp };
    }
    let record = sample.payload().to_bytes()[stamp.envelope_bytes()..].to_vec();
    let sample = SampleBuilder::<SampleBuilderPut>::try_from(sample)
        .expect("a put")
        .payload(record)
        .into();
    Read::Enveloped { sample, stamp }
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
