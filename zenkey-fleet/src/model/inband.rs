//! A payload read beside today's wire (core §7.5, observer §7.3's "the
//! envelope first"; #976's A2): the fleet's one place that tells a sample
//! carrying a zk2c envelope, strips it, says what orders it, and says what
//! a refused one renders as.
//!
//! ```text
//! a put on a stream, state or event key, under zenoh/bytes
//!     → its record: the payload every later step reads (Payload::Record)
//!     → or refused: undecodable, its refusal named, never sniffed (Payload::Refused)
//! a delete on such a key whose attachment parses as a delete's envelope
//!     → its in-band stamp (delete_stamp)
//! every other sample
//!     → read as today (Payload::Plain)
//! ```
//!
//! The decision is `zenkey_model::wire`'s, by the key and the Encoding
//! alone (a namespace in front or none), and the order of two marks
//! `zenkey_model::seed`'s. A sample keeps its bytes as they arrived
//! wherever it is carried ([`crate::SampleView`], [`crate::FleetAnswer`],
//! a `.zrec` row, a `.zsnap` row), so a recording of enveloped samples
//! replays through the same lens: the strip happens where the bytes are
//! read, here. A raw zenoh sample in a bus reader is read by [`read`], the
//! runtime's `zenkey::inband::read`, which makes the same decisions.
//!
//! A sample's Zenoh timestamp stays its time where a profile reads one
//! (freshness's age, timing's lifespan, desired's author order), and its
//! wire size what the traffic statistics count. Its *order* is its in-band
//! stamp ([`mark`]).

use std::borrow::Cow;

use zenkey_model::seed::Mark;
use zenkey_model::wire::{self, Reading, Refusal};

use crate::model::structural;
use crate::report::{Conformance, PayloadRendering, Rendered, ResolvedResource};

pub use zenkey::inband::{Read, read, refusal, stamp_of};

/// What a sample's zk2c envelope says: `None` for a sample that carries
/// none, else its in-band stamp or why its envelope was refused.
pub type Envelope = Option<Result<wire::Stamp, Refusal>>;

/// A raw sample's envelope, as [`read`] read it.
pub fn envelope_of(read: &Read) -> Envelope {
    match read {
        Read::Plain(_) => None,
        Read::Enveloped { stamp, .. } => Some(Ok(*stamp)),
        Read::Refused { refusal, .. } => Some(Err(*refusal)),
    }
}

/// What orders a sample whose envelope says `envelope`: its in-band stamp,
/// else its Zenoh timestamp; `None` for a refused envelope, never ordered.
pub fn mark_of(envelope: Envelope, zenoh: Option<&zenoh::time::Timestamp>) -> Option<Mark> {
    match envelope {
        Some(Err(_)) => None,
        e => mark(e.and_then(Result::ok), zenoh),
    }
}

/// What a sample's bytes read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload<'a> {
    /// No envelope: the bytes as they arrived.
    Plain(&'a [u8]),
    /// An envelope: its record, and its run and seq.
    Record {
        record: &'a [u8],
        stamp: wire::Stamp,
    },
    /// A refused envelope: undecodable.
    Refused(Refusal),
}

impl<'a> Payload<'a> {
    /// The bytes every later step reads, `None` for a refused envelope.
    pub fn bytes(&self) -> Option<&'a [u8]> {
        match *self {
            Payload::Plain(b) | Payload::Record { record: b, .. } => Some(b),
            Payload::Refused(_) => None,
        }
    }

    /// The in-band stamp, when the bytes carried an envelope.
    pub fn stamp(&self) -> Option<wire::Stamp> {
        match self {
            Payload::Record { stamp, .. } => Some(*stamp),
            Payload::Plain(_) | Payload::Refused(_) => None,
        }
    }

    /// The refusal, when the envelope was refused.
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Payload::Refused(r) => Some(*r),
            Payload::Plain(_) | Payload::Record { .. } => None,
        }
    }

    /// The Encoding the later steps read the bytes on: the sample's for
    /// plain bytes, none for a record, whose wire is its contract's
    /// `encoding` (observer §7.3: `zenoh/bytes` picks no wire).
    pub fn encoding<'e>(&self, sample: Option<&'e str>) -> Option<&'e str> {
        match self {
            Payload::Plain(_) => sample,
            Payload::Record { .. } | Payload::Refused(_) => None,
        }
    }
}

/// A put's bytes on `key` under `encoding` (zenoh's spelling; `None` when
/// the caller does not know it, which no envelope is read under), read:
/// see the module doc. A delete carries its stamp in its attachment, never
/// in these bytes ([`delete_stamp`]).
pub fn payload<'a>(key: &str, encoding: Option<&str>, bytes: &'a [u8]) -> Payload<'a> {
    let Some(encoding) = encoding else {
        return Payload::Plain(bytes);
    };
    match wire::read_put(key, encoding, bytes) {
        Reading::Plain => Payload::Plain(bytes),
        Reading::Enveloped(e) => Payload::Record {
            record: e.record,
            stamp: e.stamp,
        },
        Reading::Refused(r) => Payload::Refused(r),
    }
}

/// A delete's in-band stamp, from its attachment.
pub fn delete_stamp(key: &str, attachment: Option<&[u8]>) -> Option<wire::Stamp> {
    wire::read_delete(key, attachment)
}

/// A sample's in-band stamp, a put's from its bytes and a delete's from its
/// attachment.
pub fn stamp(
    key: &str,
    delete: bool,
    encoding: Option<&str>,
    bytes: &[u8],
    attachment: Option<&[u8]>,
) -> Option<wire::Stamp> {
    if delete {
        delete_stamp(key, attachment)
    } else {
        payload(key, encoding, bytes).stamp()
    }
}

/// What orders a sample: its in-band stamp, else its Zenoh timestamp
/// (`zenkey_model::seed::Mark::of`).
pub fn mark(inband: Option<wire::Stamp>, zenoh: Option<&zenoh::time::Timestamp>) -> Option<Mark> {
    Mark::of(inband, zenoh.map(stamp_of))
}

/// Replies of one read, newest first, as indices into `marks` (each
/// reply's [`mark`], in arrival order): within one run of in-band stamps by
/// seq, among Zenoh stamps by zenoh's order, each run and the Zenoh-stamped
/// a group ranked by its first arrival, since nothing orders two of them
/// (core §7.5), and the unstamped (a refused envelope among them) last, in
/// arrival order. A total order, so a sort by it never breaks.
pub fn newest_first(marks: &[Option<Mark>]) -> Vec<usize> {
    use std::cmp::Ordering;
    // `Some(None)`: the Zenoh-stamped; `Some(Some(run))`: one run.
    let group = |m: &Option<Mark>| match m {
        Some(Mark::InBand(s)) => Some(Some(s.run)),
        Some(Mark::Zenoh(_)) => Some(None),
        None => None,
    };
    let mut firsts = Vec::new();
    for g in marks.iter().filter_map(group) {
        if !firsts.contains(&g) {
            firsts.push(g);
        }
    }
    let rank = |g| firsts.iter().position(|f| *f == g);
    let mut order: Vec<usize> = (0..marks.len()).collect();
    order.sort_by(|&i, &j| match (group(&marks[i]), group(&marks[j])) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(gi), Some(gj)) if gi != gj => rank(gi).cmp(&rank(gj)),
        _ => match (marks[i], marks[j]) {
            (Some(Mark::InBand(a)), Some(Mark::InBand(b))) => b.seq.cmp(&a.seq),
            (Some(Mark::Zenoh(a)), Some(Mark::Zenoh(b))) => b.cmp(&a),
            _ => Ordering::Equal,
        },
    });
    order
}

/// `bytes` without their first `from`, borrowed where they were.
pub fn after(bytes: Cow<'_, [u8]>, from: usize) -> Cow<'_, [u8]> {
    match bytes {
        Cow::Borrowed(b) => Cow::Borrowed(&b[from..]),
        Cow::Owned(mut v) => {
            v.drain(..from);
            Cow::Owned(v)
        }
    }
}

/// A put's bytes as the later steps read them, owned or borrowed as they
/// were handed in: the record of an envelope, the bytes of any other
/// sample, the refusal of a refused envelope.
pub fn record<'a>(
    key: &str,
    encoding: Option<&str>,
    bytes: Cow<'a, [u8]>,
) -> Result<Cow<'a, [u8]>, Refusal> {
    let from = match payload(key, encoding, &bytes) {
        Payload::Plain(_) => return Ok(bytes),
        Payload::Refused(r) => return Err(r),
        Payload::Record { stamp, .. } => stamp.envelope_bytes(),
    };
    Ok(after(bytes, from))
}

/// A sample in hand, read (a [`crate::SampleView`], a history entry, a
/// replayed row): the payload every later step reads, borrowed or owned
/// as `bytes` were, and what its envelope says. A put's from its bytes
/// under `encoding` (its record then the payload; a refused one's bytes
/// left as they came, which no step may read), a delete's from its
/// attachment (its payload unchanged).
pub fn read_parts<'a>(
    key: &str,
    delete: bool,
    encoding: Option<&str>,
    bytes: Cow<'a, [u8]>,
    attachment: Option<&[u8]>,
) -> (Cow<'a, [u8]>, Envelope) {
    if delete {
        return (bytes, delete_stamp(key, attachment).map(Ok));
    }
    match payload(key, encoding, &bytes) {
        Payload::Plain(_) => (bytes, None),
        Payload::Refused(r) => (bytes, Some(Err(r))),
        Payload::Record { stamp, .. } => (after(bytes, stamp.envelope_bytes()), Some(Ok(stamp))),
    }
}

/// A raw sample's payload as the later steps read it ([`record`]): a zk2c
/// envelope's record, any other sample's bytes, or the refusal. A
/// delete's is its (empty) payload.
pub fn record_of(sample: &zenoh::sample::Sample) -> Result<Cow<'_, [u8]>, Refusal> {
    let bytes = sample.payload().to_bytes();
    if sample.kind() == zenoh::sample::SampleKind::Delete {
        return Ok(bytes);
    }
    record(
        sample.key_expr().as_str(),
        Some(&sample.encoding().to_string()),
        bytes,
    )
}

/// The Encoding to hand a rendering of a sample the runtime already read
/// through `zenkey::inband` (a consumer's `Delivery`, a `StateAnswer`'s
/// value, an archive's `LastKnown`), so its bytes are never stripped
/// twice: none for `zenoh/bytes`, the sample's own otherwise. The runtime
/// keeps a stripped sample's Encoding, `zenoh/bytes`, and delivers no
/// refused envelope, so a value it read under `zenoh/bytes` is a record,
/// read on its contract's `encoding`; on a key no envelope is read on,
/// `zenoh/bytes` names no wire either (observer §7.3).
pub fn delivered_encoding(encoding: &str) -> Option<&str> {
    (encoding != wire::ENCODING).then_some(encoding)
}

/// What a refused envelope's sample is declared as when no contract types
/// it: the envelope, which its key and Encoding declared.
pub const ENVELOPE: &str = "zk2c envelope";

/// A refused envelope's rendering: undecodable, as `declared` (the
/// resource's type, else [`ENVELOPE`]), with the refusal's reason.
pub fn refused_rendering(
    key: &str,
    size: usize,
    resource: Option<ResolvedResource>,
    declared: Option<String>,
    refusal: Refusal,
) -> PayloadRendering {
    PayloadRendering {
        key: key.to_owned(),
        size,
        resource,
        rendered: Rendered::Undecodable {
            declared: declared.unwrap_or_else(|| ENVELOPE.to_owned()),
            reason: refusal.reason(),
        },
    }
}

/// A refused envelope's conformance: undecodable, declared as its
/// rendering says ([`refused_rendering`]), with the refusal's reason.
pub fn refused_conformance(rendering: &PayloadRendering, refusal: Refusal) -> Conformance {
    let declared = match &rendering.rendered {
        Rendered::Undecodable { declared, .. } => declared.clone(),
        _ => ENVELOPE.to_owned(),
    };
    Conformance::Undecodable {
        declared,
        reason: refusal.reason(),
    }
}

/// The structural ladder over a put's bytes, its envelope stripped first
/// (observer §7.3: strip, then sniff): the record's document, text or byte
/// count, or a refused envelope's reason, never its bytes sniffed.
pub fn structural_text(key: &str, encoding: Option<&str>, bytes: &[u8]) -> String {
    match payload(key, encoding, bytes) {
        Payload::Refused(r) => format!("<undecodable: {}>", r.reason()),
        Payload::Plain(b) | Payload::Record { record: b, .. } => structural::structural(b),
    }
}

/// [`structural::structural_value`] over a put's bytes, its envelope
/// stripped first; `None` for a refused envelope.
pub fn structural_value(
    key: &str,
    encoding: Option<&str>,
    bytes: &[u8],
) -> Option<serde_json::Value> {
    payload(key, encoding, bytes)
        .bytes()
        .and_then(structural::structural_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

    fn stamp(run: u16, seq: u32) -> Stamp {
        Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        }
    }

    #[test]
    fn a_namespaced_key_strips_and_a_foreign_one_does_not() {
        let st = stamp(3, 4);
        let bytes = Envelope {
            tag: Tag::NONE,
            stamp: st,
            record: b"up",
        }
        .write();
        let p = payload("prod/zk2/h/s/i.v1/state/x", Some("zenoh/bytes"), &bytes);
        assert_eq!(
            p,
            Payload::Record {
                record: b"up",
                stamp: st
            }
        );
        assert_eq!(p.encoding(Some("zenoh/bytes")), None);
        // An Encoding the caller does not know marks no envelope.
        assert_eq!(
            payload("zk2/h/s/i.v1/state/x", None, &bytes),
            Payload::Plain(&bytes)
        );
        assert_eq!(
            payload("prod/demo/x", Some("zenoh/bytes"), &bytes),
            Payload::Plain(&bytes)
        );
        assert_eq!(
            payload("zk2/h/s/i.v1/state/x", Some("text/plain"), &bytes),
            Payload::Plain(&bytes)
        );
        // An operation's key keeps its wire.
        assert_eq!(
            payload("zk2/h/s/i.v1/@op/x", Some("zenoh/bytes"), &bytes),
            Payload::Plain(&bytes)
        );
        let refused = payload("zk2/h/s/i.v1/state/x", Some("zenoh/bytes"), b"up");
        assert_eq!(refused.bytes(), None);
        assert!(refused.refusal().is_some());
        assert_eq!(
            after(Cow::Owned(bytes.clone()), 6).as_ref(),
            after(Cow::Borrowed(&bytes), 6).as_ref()
        );
        assert_eq!(
            record(
                "zk2/h/s/i.v1/state/x",
                Some("zenoh/bytes"),
                Cow::Owned(bytes.clone())
            )
            .unwrap()
            .as_ref(),
            b"up"
        );
    }

    #[test]
    fn a_refused_envelope_is_never_sniffed() {
        let key = "zk2/h/s/i.v1/state/x";
        // Plain JSON under zenoh/bytes on a state key: an envelope refused,
        // never the document it would sniff as.
        let text = structural_text(key, Some("zenoh/bytes"), br#"{"a":1}"#);
        assert!(
            text.starts_with("<undecodable: zk2c envelope refused: "),
            "{text}"
        );
        assert_eq!(
            structural_value(key, Some("zenoh/bytes"), br#"{"a":1}"#),
            None
        );
        // Its predefined Encoding reads as today.
        assert_eq!(
            structural_text(key, Some("application/json"), br#"{"a":1}"#),
            r#"{"a":1}"#
        );
        let enveloped = Envelope {
            tag: Tag::NONE,
            stamp: stamp(1, 1),
            record: br#"{"a":1}"#,
        }
        .write();
        assert_eq!(
            structural_value(key, Some("zenoh/bytes"), &enveloped),
            Some(serde_json::json!({"a": 1}))
        );
    }

    #[test]
    fn replies_sort_newest_first_within_a_run_and_by_arrival_across_runs() {
        use zenkey_model::seed::Stamp as Zs;
        let ib = |run, seq| Some(Mark::InBand(stamp(run, seq)));
        let zn = |time| Some(Mark::Zenoh(Zs { time, id: 1 }));
        let marks = [ib(1, 2), zn(5), None, ib(2, 9), ib(1, 7), zn(9), ib(2, 1)];
        // Run 1 first (seq 7, 2), then the Zenoh-stamped (9, 5), then run 2
        // (9, 1), the unstamped last.
        assert_eq!(newest_first(&marks), [4, 0, 5, 1, 3, 6, 2]);
    }

    #[test]
    fn a_delete_carries_its_stamp_in_its_attachment() {
        let key = "zk2/h/s/i.v1/state/x";
        let att = wire::DeleteStamp { stamp: stamp(2, 9) }.write();
        assert_eq!(delete_stamp(key, Some(&att)), Some(stamp(2, 9)));
        assert_eq!(stamp_any(key, &att), Some(stamp(2, 9)));
        // An archive's own attachment is read as today.
        assert_eq!(delete_stamp(key, Some(b"archive")), None);
        assert_eq!(delete_stamp(key, None), None);
    }

    fn stamp_any(key: &str, att: &[u8]) -> Option<Stamp> {
        super::stamp(key, true, Some("zenoh/bytes"), b"", Some(att))
    }
}
