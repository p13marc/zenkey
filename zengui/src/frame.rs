//! Frames in zengui (core §7.7, observer §7.6; #981's K2): a selected key
//! that resolves to a frame resource shows its newest frame's records, each
//! at its seq and time, every field in its state — **the window's own
//! view** over the slot's history ring.
//!
//! **Session-free, and every state the model's.** [`read`] hands the
//! ring's frames of the key, oldest first, to one
//! [`Reader`], as a tool's reader of the key
//! does, and keeps the newest frame's reading
//! (`zenkey_model::frame::Applied::reading`), which the fleet spells as a
//! person reads it ([`zenkey_fleet::model::frame::reading_lines`]).
//!
//! - **The ring is the window.** Its oldest retained frame is the reader's
//!   first, so a field it omits is `unknown`, never a value; an evicted
//!   ring says so ([`FrameRead::evicted`]).
//! - **A frame lost** between two the ring holds (a seq the pump never
//!   delivered, or a body its layout refused) turns the fields the next one
//!   omits `unknown`, until each is included again. A refused one is named
//!   by the next frame's head (`Reader::refuse`), its records in the gap.
//! - **Time.** A scheduled record reads as scheduled with its jitter, never
//!   as a measured instant; a record's `t` is from the deployment epoch, as
//!   the frame carries it. Its record type's `time` fields read under the
//!   epoch in force for its owner (core §8.6), as a packed record's do: the
//!   fleet's `applied_rendering` assumes 0 and says so where none is given,
//!   and refuses where the catalog its owner names is not held.

use zenkey_fleet::Resolved;
use zenkey_model::frame::{Frame, Layout, Reader};

use crate::history::HistoryRing;

/// The slot key's frames as the window read them.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRead {
    /// `zk2c frame of packed:<type>`.
    pub declared: String,
    /// The frame table, in words: records a frame, the step and jitter.
    pub table: String,
    /// The newest frame's reading, as `spec/conformance/frame/apply.json`
    /// spells one; `None` while the ring holds no frame it could read.
    pub newest: Option<serde_json::Value>,
    /// Why the newest frame read no value: its record type's `time` fields
    /// under an epoch the reader refuses (core §8.6).
    pub undecodable: Option<String>,
    /// What a person reads beside the newest frame whose `time` fields rest
    /// on an epoch assumed (`Rendered::epoch_note`).
    pub epoch_note: Option<String>,
    /// The ring's frames read, and those refused (another tag, a body its
    /// layout refuses, a sample that came in no frame).
    pub frames: usize,
    pub refused: usize,
    /// Whether the ring evicted entries: its oldest kept frame is then the
    /// reader's first.
    pub evicted: bool,
}

impl FrameRead {
    /// The newest frame, a line a record after its head
    /// ([`zenkey_fleet::model::frame::reading_lines`]).
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.newest
            .as_ref()
            .and_then(zenkey_fleet::model::frame::reading_lines)
            .unwrap_or_default()
    }
}

/// The frame table in words.
fn table(l: &Layout) -> String {
    let p = &l.params;
    let mut s = match (p.records, p.step_ms, p.jitter_ms) {
        (1, _, _) => "one record a frame".to_owned(),
        (n, Some(step), Some(j)) => format!("{n} records a frame, every {step} ms ±{j} ms"),
        (n, _, _) => format!("{n} records a frame"),
    };
    s.push_str(&format!(", a full record every {}", p.keyframe_every));
    for (name, d) in &p.deadband {
        s.push_str(&format!(
            ", {name} within ±{} for {} records",
            d.delta, d.max_records
        ));
    }
    s
}

/// The slot key's frames, read over its ring (see the module doc), `key`
/// the wire key the ring records, `epoch` the one in force for its owner
/// (`zenkey_fleet::Lens::epoch_of`); `None` when the key resolves to no
/// frame resource.
#[must_use]
pub fn read(
    resolved: &Resolved,
    key: &str,
    ring: &HistoryRing,
    epoch: zenkey_fleet::DeploymentEpoch,
) -> Option<FrameRead> {
    let contract = resolved.revision.contract();
    let resource = resolved.resource();
    let layout = Layout::of_resource(contract, resource).ok()??;
    let ty = zenkey_model::decode::type_of(
        resolved.revision.bundle(),
        resource.token.as_str(),
        resource.template.as_str(),
        "type",
    );
    let declared = zenkey_fleet::model::frame::declared(
        &ty.map(zenkey_model::decode::declared).unwrap_or_default(),
    );
    let mut reader = Reader::new();
    let mut out = FrameRead {
        declared,
        table: table(&layout),
        newest: None,
        undecodable: None,
        epoch_note: None,
        frames: 0,
        refused: 0,
        evicted: ring.evicted() > 0,
    };
    // The ring iterates newest first: its frames, oldest first.
    for e in ring.iter().rev().filter(|e| !e.is_delete) {
        let bytes = e.payload.to_bytes();
        let encoding = (!e.encoding.is_empty()).then_some(e.encoding.as_str());
        let parsed = match zenkey_fleet::model::inband::payload(key, encoding, &bytes) {
            zenkey_fleet::model::inband::Payload::Frame { body, stamp, tag } => {
                match Frame::parse(tag, body, &layout) {
                    Ok(f) => Some((stamp, f)),
                    // Never applied: the reader notes it, so the next
                    // frame's gap counts and names it (core §7.7).
                    Err(_) => {
                        reader.refuse(stamp);
                        None
                    }
                }
            }
            _ => None,
        };
        match parsed {
            Some((stamp, frame)) => {
                let applied = reader.apply(&layout, stamp, &frame);
                out.frames += 1;
                let (r, _) = zenkey_fleet::model::frame::applied_rendering(
                    key,
                    None,
                    out.declared.clone(),
                    &layout,
                    &applied,
                    bytes.len(),
                    epoch,
                );
                out.epoch_note = r.rendered.epoch_note();
                (out.newest, out.undecodable) = match r.rendered {
                    zenkey_fleet::report::Rendered::Value { value, .. } => (Some(value), None),
                    zenkey_fleet::report::Rendered::Undecodable { reason, .. } => {
                        (None, Some(reason))
                    }
                    _ => (None, None),
                };
            }
            None => out.refused += 1,
        }
    }
    Some(out)
}
