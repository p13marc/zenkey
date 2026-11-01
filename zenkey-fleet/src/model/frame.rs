//! Frames, read for a tool (core §7.7, observer §7.6; #981's K2).
//!
//! ```text
//! a frame (core §7.5's form 5) on a frame resource's key
//!     → its body parsed with the resource's layout, under CH's tag
//!     → applied by the key's reader: each record's seq, time and field
//!       states (known, within its delta, absent, unknown with why)
//!     → Rendered::Value, the reading `spec/conformance/frame/apply.json`
//!       spells ([`frame_check`], [`applied_rendering`])
//! a frame whose body its layout refuses     → undecodable, the refusal named
//! a frame on a resource with no `frame` table → undecodable ([`FRAME_AS_RECORD`])
//! a mutation on a frame resource            → undecodable ([`MUTATION_ON_FRAME`])
//! a frame with no declaration in hand       → its envelope: run, seq, size
//!                                             ([`unread_rendering`]), never sniffed
//! ```
//!
//! A reader is per key ([`Readers`]): a window that reads a key's frames in
//! the order they arrived (`zenctl echo`, a `.zrec` replayed through it)
//! keeps one, so a lost frame turns the fields its successors omit
//! `unknown` until each is included again. A frame read with nothing before
//! it (one reply of a GET, one row read alone) is read as a reader's first
//! frame (observer §7.6): every field its records omit is `unknown`, never
//! a value. A consumer's subscription keeps its own readers in the runtime
//! (`zenkey::consumer::Delivery::frame`), which [`applied_rendering`]
//! renders.

use std::collections::BTreeMap;

use zenkey_model::frame::{Applied, Frame, Layout, Reader};
use zenkey_model::wire;

use crate::model::deployment::Epoch;
use crate::report::{Conformance, PayloadRendering, Rendered, ResolvedResource, Unresolved};

pub use zenkey::inband::{FRAME_AS_RECORD, MUTATION_ON_FRAME};

/// The most keys a window's [`Readers`] holds; a key forgotten past it
/// reads its next frame as a first, its omitted fields `unknown`.
pub const MAX_KEYS: usize = 4096;

/// One window's readers, one per frame key (core §7.7).
#[derive(Debug, Clone, Default)]
pub struct Readers {
    keys: BTreeMap<String, Reader>,
}

impl Readers {
    /// Readers that hold nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `key`'s reader, made when it holds none. Past [`MAX_KEYS`] it
    /// forgets the first key in byte order.
    pub fn reader(&mut self, key: &str) -> &mut Reader {
        if !self.keys.contains_key(key) && self.keys.len() >= MAX_KEYS {
            self.keys.pop_first();
        }
        self.keys.entry(key.to_owned()).or_default()
    }

    /// How many keys it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// What a frame is declared as: its form, and its record type.
#[must_use]
pub fn declared(record_type: &str) -> String {
    format!("zk2c frame of {record_type}")
}

/// Whether the frame's record type holds a `time` field (core §7.6),
/// which an epoch dates. A record's own `at` is from the deployment epoch
/// as the frame carries it, and is never read as a wall time here.
fn holds_time(layout: &Layout) -> bool {
    layout
        .fields()
        .iter()
        .any(|f| matches!(f.kind, zenkey_model::packed::FieldKind::Time { .. }))
}

/// A frame its reader read, rendered: its reading as the value, valid,
/// its record type's `time` fields under `epoch`, the epoch in force for
/// its owner (core §8.6), as a packed record's are
/// (`crate::model::render::decode_under`): assumed and said so where none
/// is given, and undecodable where the reader refuses it. A `time` field
/// past an `i64` of milliseconds is undecodable.
#[must_use]
pub fn applied_rendering(
    key: &str,
    resource: Option<ResolvedResource>,
    declared: String,
    layout: &Layout,
    applied: &Applied,
    size: usize,
    epoch: Epoch,
) -> (PayloadRendering, Conformance) {
    let timed = holds_time(layout);
    if timed && let Some(reason) = epoch.refusal() {
        return undecodable(key, resource, declared, size, reason);
    }
    let given = epoch.given();
    let assumed = (timed && given.is_none()).then_some(zenkey_model::packed::DEFAULT_EPOCH);
    let under = given.unwrap_or(zenkey_model::packed::DEFAULT_EPOCH);
    match applied.reading(layout, Some(under)) {
        Ok(value) => (
            PayloadRendering {
                key: key.to_owned(),
                size,
                resource,
                rendered: Rendered::Value {
                    declared,
                    value,
                    epoch_assumed: assumed,
                },
            },
            Conformance::Valid,
        ),
        Err(r) => undecodable(key, resource, declared, size, r.reason().to_owned()),
    }
}

/// A frame's body read with its layout under CH's `tag`, then applied by
/// `reader`: its reading under `epoch` ([`applied_rendering`]), or
/// undecodable with its refusal (core §7.7's first, by token). A refused
/// body never reaches the reader, so its records read as lost at the key's
/// next frame.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn frame_check(
    key: &str,
    resource: Option<ResolvedResource>,
    declared: String,
    layout: &Layout,
    stamp: wire::Stamp,
    tag: wire::Tag,
    body: &[u8],
    reader: &mut Reader,
    epoch: Epoch,
) -> (PayloadRendering, Conformance) {
    match Frame::parse(tag, body, layout) {
        Ok(frame) => {
            let applied = reader.apply(layout, stamp, &frame);
            applied_rendering(key, resource, declared, layout, &applied, body.len(), epoch)
        }
        Err(r) => undecodable(
            key,
            resource,
            declared,
            body.len(),
            format!("a zk2c frame refused by its layout, {r}"),
        ),
    }
}

/// Undecodable, with why.
#[must_use]
pub fn undecodable(
    key: &str,
    resource: Option<ResolvedResource>,
    declared: String,
    size: usize,
    reason: String,
) -> (PayloadRendering, Conformance) {
    (
        PayloadRendering {
            key: key.to_owned(),
            size,
            resource,
            rendered: Rendered::Undecodable {
                declared: declared.clone(),
                reason: reason.clone(),
            },
        },
        Conformance::Undecodable { declared, reason },
    )
}

/// The structural text of a frame read with no declaration in hand: its
/// form, run, seq and size, its body never sniffed (observer §7.6).
#[must_use]
pub fn unread_text(stamp: wire::Stamp, size: usize) -> String {
    format!(
        "<zk2c frame, run {}, seq {}, {size} bytes>",
        stamp.run.hex(),
        stamp.seq.get()
    )
}

/// A frame rendered with no declaration in hand: its envelope and size,
/// with where resolution stopped (observer §7.6).
#[must_use]
pub fn unread_rendering(
    key: &str,
    why: Unresolved,
    stamp: wire::Stamp,
    size: usize,
) -> PayloadRendering {
    PayloadRendering {
        key: key.to_owned(),
        size,
        resource: None,
        rendered: Rendered::Structural {
            why,
            value: None,
            text: unread_text(stamp, size),
        },
    }
}

/// Whether a rendering is a frame's reading ([`declared`]'s spelling).
#[must_use]
pub fn is_frame_declared(declared: &str) -> bool {
    declared.starts_with("zk2c frame of ")
}

/// A frame's reading as a person reads it (observer §7.6), one line a
/// record after a head: `None` for a value that is no frame reading.
///
/// ```text
/// run 0007, seq 13, 1 record lost before it
/// #13 t=280.000 measured: dropped unknown (records lost) · queue_depth 31 ±4 (within) · mode absent
/// #31 t=7210.000 scheduled ±500 ms: temperature "22"
/// ```
///
/// A scheduled time says so with its jitter, never as a measured one; a
/// `within` value carries its delta; an `unknown` or `absent` field shows
/// no value; `t` is from the deployment epoch, `+n ms` an offset from
/// record 0 when its time is unknown.
#[must_use]
pub fn reading_lines(value: &serde_json::Value) -> Option<Vec<String>> {
    let records = value.get("records")?.as_array()?;
    let follows = match value.get("follows")?.as_str()? {
        "first" => ", the first frame read".to_owned(),
        "continues" => String::new(),
        "gap" => {
            let n = value
                .get("lost")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            format!(
                ", {n} record{} lost before it",
                if n == 1 { "" } else { "s" }
            )
        }
        "rewind" => ", a rewind: its run's seq went back".to_owned(),
        "run" => ", a run not held before".to_owned(),
        other => format!(", {other}"),
    };
    let mut out = vec![format!(
        "run {}, seq {}{follows}",
        value.get("run")?.as_str()?,
        value.get("seq")?
    )];
    for r in records {
        let at = r.get("at").map(|a| {
            format!(
                "{}.{:03}",
                a["s"].as_u64().unwrap_or(0),
                a["ms"].as_u64().unwrap_or(0)
            )
        });
        let when = match (r.get("time")?.as_str()?, at, r.get("after_ms")) {
            ("unknown", _, _) => "t unknown (the owner's clock)".to_owned(),
            ("scheduled", at, after) => format!(
                "t={} scheduled ±{} ms",
                at.unwrap_or_else(|| format!("+{} ms", after.cloned().unwrap_or_default())),
                r.get("jitter_ms").cloned().unwrap_or_default()
            ),
            (_, Some(at), _) => format!("t={at} measured"),
            (_, None, after) => format!("t=+{} ms measured", after.cloned().unwrap_or_default()),
        };
        let fields: Vec<String> = r
            .get("fields")?
            .as_object()?
            .iter()
            .map(|(name, f)| match f["state"].as_str() {
                Some("known") => format!("{name} {}", f["value"]),
                Some("within") => format!(
                    "{name} {} ±{} (within)",
                    f["value"],
                    f["delta"].as_str().unwrap_or("?")
                ),
                Some("absent") => format!("{name} absent"),
                _ => format!(
                    "{name} unknown ({})",
                    match f["why"].as_str() {
                        Some("first") => "not included since the first frame read",
                        Some("run") => "a run not held",
                        Some("gap") => "records lost",
                        Some("rewind") => "a rewind",
                        Some("deadband") => "max_records passed",
                        Some("no_value") => "no value to hold within",
                        _ => "unknown",
                    }
                ),
            })
            .collect();
        out.push(format!("#{} {when}: {}", r.get("seq")?, fields.join(" · ")));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zenkey_model::authoring::FrameSpec;
    use zenkey_model::frame::FrameParams;
    use zenkey_model::wire::{Run, Seq, Stamp};

    fn link(spec: FrameSpec) -> Layout {
        let doc = json!({"types": {"Link": {"fields": [
            {"name": "dropped", "type": "counter", "width": 32, "optional": true},
            {"name": "queue_depth", "type": "int", "minimum": 0, "maximum": 1022, "optional": true}
        ]}}});
        let record = zenkey_model::packed::layout_in(&doc, "Link").unwrap();
        Layout::of(&record, &FrameParams::resolve(&spec).unwrap()).unwrap()
    }

    fn body(l: &Layout, v: serde_json::Value) -> Vec<u8> {
        Frame::from_json(&v, l, None)
            .unwrap()
            .write(l)
            .unwrap()
            .bytes
    }

    /// A key's reader turns a lost frame's omitted fields unknown; a fresh
    /// reader reads a delta frame as a first, never showing a value it
    /// does not hold; another tag is undecodable, named.
    #[test]
    fn a_keys_frames_read_in_order_and_alone() {
        let l = link(FrameSpec::default());
        let stamp = |seq| Stamp {
            run: Run::U16(1),
            seq: Seq::new(seq).unwrap(),
        };
        let key = "zk2/lab/modem/m.v1/stream/frames/link";
        let full = body(
            &l,
            json!({"time": {"s": 1, "ms": 0}, "records": [{"full": true,
            "values": {"dropped": 3, "queue_depth": 9}}]}),
        );
        let delta = body(
            &l,
            json!({"time": {"s": 2, "ms": 0}, "records": [{"full": false,
            "values": {"queue_depth": 10}}]}),
        );
        let mut readers = Readers::new();
        let check = |readers: &mut Readers, seq, b: &[u8]| {
            frame_check(
                key,
                None,
                declared("packed:Link"),
                &l,
                stamp(seq),
                l.tag(),
                b,
                readers.reader(key),
                Epoch::Assumed,
            )
        };
        let value = |c: (PayloadRendering, Conformance)| match c.0.rendered {
            Rendered::Value { value, .. } => value,
            r => panic!("{r:?}"),
        };
        let v = value(check(&mut readers, 5, &full));
        assert_eq!(v["follows"], "first");
        let v = value(check(&mut readers, 7, &delta));
        assert_eq!(
            (v["follows"].clone(), v["lost"].clone()),
            (json!("gap"), json!(1))
        );
        assert_eq!(
            v["records"][0]["fields"]["dropped"],
            json!({"state": "unknown", "why": "gap"})
        );
        let v = value(check(&mut Readers::new(), 8, &delta));
        assert_eq!(
            v["records"][0]["fields"]["dropped"],
            json!({"state": "unknown", "why": "first"})
        );
        let other = wire::Tag::new((l.tag().get() + 1) % 16).unwrap();
        let (r, c) = frame_check(
            key,
            None,
            declared("packed:Link"),
            &l,
            stamp(9),
            other,
            &full,
            readers.reader(key),
            Epoch::Assumed,
        );
        assert!(matches!(r.rendered, Rendered::Undecodable { .. }), "{r:?}");
        assert!(
            matches!(c, Conformance::Undecodable { reason, .. } if reason.ends_with("tag: the envelope's tag is not the frame's"))
        );
        assert_eq!(
            unread_text(stamp(9), 11),
            "<zk2c frame, run 0001, seq 9, 11 bytes>"
        );
        assert_eq!(full.len(), 11, "42 + 1 + 33 + 11 bits");
        let lines = reading_lines(&v).unwrap();
        assert_eq!(
            lines,
            [
                "run 0001, seq 8, the first frame read",
                "#8 t=2.000 measured: dropped unknown (not included since the first frame read) · queue_depth 10",
            ]
        );
    }
}
