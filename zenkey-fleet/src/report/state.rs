//! A zk2 state read (spec §4): `zenctl get state`, the owner's answer
//! (S4) or an archive's last-known one (S5).
//!
//! The two are different questions, and the shape says which was asked
//! before it says anything else: `reading` is `current` or `last_known`, in
//! every medium, and an archive's answer is never rendered as current (S6).
//! Silence from the owner is no rows and is never "no value": the verb
//! exits 2 on it.
//!
//! [`Stamp`] is shared with `watch`'s samples: a time is shown with the id
//! of the clock that issued it (§4.1), the owner's own on a resource core
//! §4.3's table lists. [`InbandStamp`] is too: the run and seq the owner
//! wrote in band (§4.3, §7.5), which order its values and carry no time
//! (observer §8.3).
//!
//! A row keeps the reply's bytes beside its rendering (#765), and never
//! serializes them: the shape is the rendering's, and a byte view (a hex
//! dump, the sniff of bytes that did not decode) is a renderer's concern.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::payload::PayloadRendering;

/// One state read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateReport {
    /// Which question was asked: the owner's current state, or an
    /// archive's last-known state.
    pub reading: StateReading,
    /// The owner, `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision the values were decoded against.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The template values given, unslugged; a parameter not given is a
    /// wildcard (current reads only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The key expressions the GET went out on, base-relative: the owner's
    /// keys (S4), or the archive's key for them (§4.4).
    pub selectors: Vec<String>,
    /// The archive asked, `<system>/<service>`: last-known reads only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    /// How long the GET waited for replies, seconds.
    pub timeout_s: f64,
    /// Every key answered, sorted. Empty is silence: no reply within the
    /// timeout, which is not a verdict about the key (S6, O5) — unless
    /// [`StateReport::snapshots`] holds a complete snapshot, the owner's
    /// word that it held no member there.
    pub rows: Vec<StateRow>,
    /// Each collection's snapshot the owner's pages made (core §7.5;
    /// #977's E2), in the order of its first page, its entries among the
    /// rows: current reads only, and absent when no page answered, so a
    /// read of per-key replies keeps its shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<CollectionSnapshot>,
}

/// One collection's snapshot a state read heard (core §7.5's form 7;
/// #977's E2), `get state`'s and a seed's state leg's: the key its pages
/// came on, its collection's index, the owner's run and the snapshot's
/// asof, the pages held, and whether it is complete. Under `Latest`, which
/// the readers keep until E3, a snapshot of several pages arrives as its
/// last page alone, and is held incomplete. Not a `.zsnap`
/// ([`Snapshot`](crate::report::Snapshot)), a tool's capture of state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectionSnapshot {
    pub key: String,
    pub coll: u32,
    /// The owner's run, as a descriptor spells one (core §3.3).
    pub run: String,
    /// A seq of that run: the snapshot reflects every state mutation of
    /// the run at or below it.
    pub asof: u32,
    /// The page indices held.
    pub pages: Vec<u32>,
    pub complete: bool,
}

impl From<&zenkey_model::collection::Heard> for CollectionSnapshot {
    fn from(h: &zenkey_model::collection::Heard) -> Self {
        CollectionSnapshot {
            key: h.key.clone(),
            coll: h.coll,
            run: h.run.hex(),
            asof: h.asof.get(),
            pages: h.pages.iter().copied().collect(),
            complete: h.complete(),
        }
    }
}

/// Which state question a read asked (S6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateReading {
    /// The owner's answer to a GET on its keys (S4): current state.
    Current,
    /// An archive's answer (S5): last-known, never current.
    LastKnown,
}

/// One key of a state read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateRow {
    /// The owner's key, base-relative (for a last-known read, the key the
    /// archive recorded).
    pub key: String,
    /// The value, or the key's deletion. Tagged `state`.
    #[serde(flatten)]
    pub value: StateValue,
    /// The reply's payload as it arrived, refcounted, so a clone is a
    /// refcount bump: the owner's sample (S4), or the bytes the archive
    /// recorded (S5). `None` on a deletion, which carries none.
    ///
    /// Never serialized (#765): `get state --format json` and the pinned
    /// shape are the rendering's. A byte view is a renderer's concern, not
    /// this report's contract. Equality compares the bytes' content.
    #[serde(skip)]
    pub bytes: Option<zenoh::bytes::ZBytes>,
    /// The mutation's in-band stamp, its envelope's run and seq (core
    /// §4.3, S1, S2): what orders it. Absent when the reply carried no
    /// envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inband: Option<InbandStamp>,
    /// The mutation's Zenoh timestamp: on a resource core §4.3's table
    /// lists, the owner's (S1). Absent when the reply carried none, as a
    /// reply off the table does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Stamp>,
    /// A member a `freshness.v2` heartbeat confirms (#978's F3): its last
    /// confirmation, the later of its last mutation and the last heartbeat
    /// that vouched for it, as the owner's reply carries it (core §4.3's
    /// table, S2). Absent elsewhere, and on a deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_confirmation: Option<Stamp>,
    /// Last-known reads only: whether the archive's alignment confirmed
    /// the key (§4.4). `false` is kept, served, and not confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<bool>,
    /// Last-known reads only: the value's type identity as the archive
    /// recorded it, `{"iface", "contract", "type"}` (§7.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<serde_json::Value>,
}

/// A key's state. Tagged `state`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StateValue {
    /// The key's value, rendered through the contract's type.
    Value { payload: Box<PayloadRendering> },
    /// Deleted: a `reply_del` within the owner's tombstone window (S3), or
    /// a tombstone an archive holds. Not an empty value.
    Deleted,
}

/// A Zenoh timestamp, and whose clock issued it (core §4.1, §4.3). It is a
/// time: on a resource §4.3's table lists, the owner's (S1); elsewhere a
/// router's, a reader's own, or none, expected and never a finding
/// (observer §4.4). What orders a value is its [`InbandStamp`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stamp {
    /// The stamp's time, RFC 3339 with nanoseconds, UTC.
    pub time: String,
    /// The id of the clock that issued it: the issuing session's zid. An
    /// owner's own stamp carries its session's (S1); a router's, the
    /// router's.
    pub clock: String,
}

/// An in-band stamp (core §4.3, §7.5): the owner's run and seq. It orders a
/// key's values within one run, and has no time: shown as a run and a seq,
/// never as a time (observer §8.3).
///
/// Read back from a file (a `.zsnap` row), only as core §3.3 spells a run
/// and §7.5 bounds a seq: anything else is refused, never guessed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InbandWire")]
pub struct InbandStamp {
    /// The run, as the owner's descriptor states it (core §3.3): 4
    /// lowercase hex digits for a 16-bit run, 8 for a 32-bit one.
    pub run: String,
    /// The seq within the run.
    pub seq: u32,
}

impl From<zenkey_model::wire::Stamp> for InbandStamp {
    fn from(s: zenkey_model::wire::Stamp) -> Self {
        InbandStamp {
            run: s.run.hex(),
            seq: s.seq.get(),
        }
    }
}

impl InbandStamp {
    /// The stamp it spells, `None` when its run or seq is not one (a value
    /// built by hand; a deserialized one always is).
    pub fn stamp(&self) -> Option<zenkey_model::wire::Stamp> {
        Some(zenkey_model::wire::Stamp {
            run: zenkey_model::wire::Run::from_hex(&self.run)?,
            seq: zenkey_model::wire::Seq::new(self.seq)?,
        })
    }
}

/// [`InbandStamp`] as a file spells it, before its run and seq are checked.
#[derive(Deserialize)]
struct InbandWire {
    run: String,
    seq: u32,
}

impl TryFrom<InbandWire> for InbandStamp {
    type Error = String;

    fn try_from(w: InbandWire) -> Result<Self, String> {
        let run = zenkey_model::wire::Run::from_hex(&w.run).ok_or_else(|| {
            format!(
                "in-band run {:?}: not 4 or 8 lowercase hex digits (core §3.3)",
                w.run
            )
        })?;
        let seq = zenkey_model::wire::Seq::new(w.seq)
            .ok_or_else(|| format!("in-band seq {}: above 16777215 (core §7.5)", w.seq))?;
        Ok(zenkey_model::wire::Stamp { run, seq }.into())
    }
}

impl std::fmt::Display for InbandStamp {
    /// `run 0003, seq 12`: the words every medium shows it in.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "run {}, seq {}", self.run, self.seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Rendered, ResolvedResource};
    use serde_json::json;

    fn payload(key: &str) -> PayloadRendering {
        PayloadRendering {
            key: key.into(),
            size: 2,
            resource: Some(ResolvedResource {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
                resource: "state/namespaces".into(),
                member: "type".into(),
                values: BTreeMap::new(),
            }),
            rendered: Rendered::Value {
                declared: "json:Namespaces".into(),
                value: json!([]),
                epoch_assumed: None,
                dated: None,
                bounds: None,
            },
        }
    }

    /// `reading` leads, so a script never mistakes an archive's answer for
    /// the owner's; a deletion is a `state`, not an empty value. The bytes a
    /// row keeps (#765) reach no serialized shape: the row below carries
    /// them, and its JSON is the rendering's alone.
    #[test]
    fn state_report_json_shape_is_pinned() {
        let key = "zk2/host-a/tc/tc.netif.v1/state/namespaces";
        let r = StateReport {
            snapshots: Vec::new(),
            reading: StateReading::LastKnown,
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: "state/namespaces".into(),
            values: BTreeMap::new(),
            selectors: vec![
                "zk2/ground/archive/archive.v1/@state/host-a/tc/tc.netif.v1/state/namespaces"
                    .into(),
            ],
            archive: Some("ground/archive".into()),
            timeout_s: 5.0,
            rows: vec![StateRow {
                key: key.into(),
                value: StateValue::Value {
                    payload: Box::new(payload(key)),
                },
                bytes: Some(zenoh::bytes::ZBytes::from(&b"[]"[..])),
                inband: Some(InbandStamp {
                    run: "0003".into(),
                    seq: 12,
                }),
                timestamp: Some(Stamp {
                    time: "2026-10-08T12:00:00.000000000Z".into(),
                    clock: "a1b2".into(),
                }),
                last_confirmation: None,
                confirmed: Some(false),
                identity: Some(json!({"iface": "tc.netif.v1"})),
            }],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["reading"], "last_known");
        assert_eq!(v["archive"], "ground/archive");
        assert_eq!(
            v["rows"][0],
            json!({
                "key": key,
                "state": "value",
                "payload": serde_json::to_value(payload(key)).unwrap(),
                "inband": {"run": "0003", "seq": 12},
                "timestamp": {"time": "2026-10-08T12:00:00.000000000Z", "clock": "a1b2"},
                "confirmed": false,
                "identity": {"iface": "tc.netif.v1"},
            })
        );

        assert_eq!(
            r.rows[0].bytes.as_ref().map(|b| b.to_bytes().into_owned()),
            Some(b"[]".to_vec()),
            "kept beside the rendering, for a renderer to show"
        );

        let current = StateRow {
            key: key.into(),
            value: StateValue::Deleted,
            bytes: None,
            inband: None,
            timestamp: None,
            last_confirmation: None,
            confirmed: None,
            identity: None,
        };
        assert_eq!(
            serde_json::to_value(&current).unwrap(),
            json!({"key": key, "state": "deleted"})
        );
        assert_eq!(
            serde_json::to_value(StateReading::Current).unwrap(),
            json!("current")
        );
    }

    /// An in-band stamp reads as the descriptor spells its run, 4 or 8
    /// hex digits, and its seq as a number (core §3.3, §4.3).
    #[test]
    fn an_inband_stamp_spells_its_run_as_the_descriptor_does() {
        use zenkey_model::wire::{Run, Seq, Stamp as Wire};
        let narrow = InbandStamp::from(Wire {
            run: Run::U16(0xa1),
            seq: Seq::new(7).unwrap(),
        });
        assert_eq!(
            serde_json::to_value(&narrow).unwrap(),
            json!({"run": "00a1", "seq": 7})
        );
        assert_eq!(narrow.to_string(), "run 00a1, seq 7");
        let wide = InbandStamp::from(Wire {
            run: Run::U32(0xa1),
            seq: Seq::MAX,
        });
        assert_eq!(
            serde_json::to_value(&wide).unwrap(),
            json!({"run": "000000a1", "seq": 16_777_215})
        );
    }

    /// A stamp read back from a file reads as it was written, and only a
    /// run the descriptor could state and a seq of 24 bits read at all.
    #[test]
    fn an_inband_stamp_reads_back_only_as_core_spells_it() {
        for v in [
            json!({"run": "00a1", "seq": 7}),
            json!({"run": "000000a1", "seq": 16_777_215}),
        ] {
            let back: InbandStamp = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), v);
            assert!(back.stamp().is_some());
        }
        for bad in [
            json!({"run": "00A1", "seq": 7}),
            json!({"run": "a1", "seq": 7}),
            json!({"run": "00a1", "seq": 16_777_216}),
        ] {
            assert!(
                serde_json::from_value::<InbandStamp>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }
}
