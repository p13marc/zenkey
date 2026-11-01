//! The history pane (#63).

use super::*;

// ── History pane (#63) ───────────────────────────────────────────────────

/// Build a recorder over a scripted sample stream.
fn recording(key: &str, max_entries: usize, samples: &[(&[u8], bool)]) -> HistoryRecorder {
    let mut rec = HistoryRecorder::new(key, max_entries);
    for (payload, is_delete) in samples {
        rec.observe(&zenkey_fleet::SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "application/json".to_string(),
            kind: if *is_delete {
                zenoh::sample::SampleKind::Delete
            } else {
                zenoh::sample::SampleKind::Put
            },
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: Instant::now(),
        });
    }
    rec
}

/// The pane's first job is saying whether anything is being recorded at all.
/// "Nothing yet" for a key nobody watches is a verdict never obtained (O4).
#[test]
fn the_history_pane_says_why_it_is_empty() {
    use zengui::view::history::{HistoryData, section};

    // Nothing selected.
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: None,
        recorder: None,
        watched: false,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(ui.find("Nothing selected").is_ok());

    // Selected, but no watch covers it — the two must not read alike.
    let rec = recording(REGISTERED, 8, &[]);
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: false,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(
        ui.find("Not watched — nothing is being recorded").is_ok(),
        "an unwatched key must not render as a quiet one"
    );
    assert!(
        ui.find("watch this key").is_ok(),
        "and the pane offers the watch rather than only complaining"
    );

    // Watched and genuinely quiet: a different sentence, and not a verdict.
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(ui.find("No samples yet").is_ok());
    assert!(
        ui.find(
            "The watch is active and nothing has arrived on this key since it \
             was selected. That is not a statement about the bus (observer §2)."
        )
        .is_ok(),
        "silence stays a non-verdict"
    );
}

/// The diff names the field that moved, and says which clock stamped the row.
#[test]
fn the_history_pane_diffs_consecutive_payloads() {
    use zengui::view::history::{HistoryData, section};

    let rec = recording(
        REGISTERED,
        8,
        &[
            (br#"{"value":41.0,"unit":"percent"}"#, false),
            (br#"{"value":42.0,"unit":"percent","inodes":1188}"#, false),
        ],
    );
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(
        ui.find("~ value  41.0 → 42.0").is_ok(),
        "the changed field is named with both sides"
    );
    assert!(
        ui.find("+ inodes  1188").is_ok(),
        "an added field is not a change"
    );
    assert!(
        ui.find(
            "times are the publisher's HLC where one was stamped and this observer's \
             arrival otherwise — every row says which"
        )
        .is_ok(),
        "the clock is labelled, never implied"
    );
}

/// A tombstone is authoritative retirement (core §4.2), and the put that
/// follows one starts a new value rather than "changing" the deleted one.
#[test]
fn the_history_pane_marks_a_tombstone_as_retirement() {
    use zengui::view::history::{HistoryData, section};

    let script: &[(&[u8], bool)] = &[
        (br#"{"status":"ok"}"#, false),
        (b"", true),
        (br#"{"status":"ok"}"#, false),
    ];

    // Focused on the delete itself.
    let mut rec = recording(REGISTERED, 8, script);
    rec.selected = Some(1);
    {
        let mut ui = simulator::<Message, _, _>(section(HistoryData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            key: Some(REGISTERED),
            recorder: Some(&rec),
            watched: true,
            scroll: Default::default(),
            snapshot: None,
            compare_snapshot: false,
        }));
        assert!(ui.find("▸ t-1").is_ok(), "the focused row is marked");
        assert!(
            ui.find("DELETE (tombstone)").is_ok(),
            "a delete must be visibly distinct from an empty put"
        );
        assert!(
            ui.find("retired — an authoritative delete (core §4.2), not an empty value")
                .is_ok()
        );
    }

    // Focused on the put after it: a new value, not a change.
    rec.selected = Some(2);
    {
        let mut ui = simulator::<Message, _, _>(section(HistoryData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            key: Some(REGISTERED),
            recorder: Some(&rec),
            watched: true,
            scroll: Default::default(),
            snapshot: None,
            compare_snapshot: false,
        }));
        assert!(
            ui.find("new value after retirement — not a change to the previous value")
                .is_ok()
        );
    }
}

/// A payload with no structural form degrades to a byte comparison that says
/// so, instead of inventing field names for bytes it cannot read.
#[test]
fn the_history_pane_falls_back_to_bytes_and_admits_it() {
    use zengui::view::history::{HistoryData, section};

    let rec = recording(
        FOREIGN,
        8,
        &[
            (b"just a plain string", false),
            (b"just a plain STRING", false),
        ],
    );
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(FOREIGN),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(
        ui.find("neither sample has a structural form — compared as bytes, not as fields")
            .is_ok()
    );
}

/// Under core 0.73's wire (#976 A4): each row shows its owner's run and seq
/// beside its clock, and the diff compares records, never envelopes — a
/// re-put of the same record says it is one, with both stamps, and two
/// records with no structural form are compared as their own bytes.
#[test]
fn the_history_pane_shows_in_band_stamps_and_diffs_records() {
    use zengui::view::history::{HistoryData, section};
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};

    let key = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let sealed = |seq: u32, record: &[u8]| {
        Envelope {
            form: zenkey_model::wire::Form::Mutation,
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(3),
                seq: Seq::new(seq).unwrap(),
            },
            record,
        }
        .write()
    };
    let recording = |script: &[(u32, &[u8])]| {
        let mut rec = HistoryRecorder::new(key, 8);
        for (seq, record) in script {
            rec.observe(&zenkey_fleet::SampleView {
                key: key.to_string(),
                payload: zenoh::bytes::ZBytes::from(sealed(*seq, record)),
                encoding: "zenoh/bytes".to_string(),
                kind: zenoh::sample::SampleKind::Put,
                timestamp: None,
                stamped_by: None,
                attachment: None,
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: Instant::now(),
            });
        }
        rec
    };
    fn pane<'a>(key: &'a str, rec: &'a HistoryRecorder) -> iced_test::Simulator<'a, Message> {
        simulator::<Message, _, _>(section(HistoryData {
            slot: zengui::message::SlotId::FOLLOW,
            sp: sp(),
            key: Some(key),
            recorder: Some(rec),
            watched: true,
            scroll: Default::default(),
            snapshot: None,
            compare_snapshot: false,
        }))
    }

    let rec = recording(&[(4, br#"{"rx":1}"#), (9, br#"{"rx":1}"#)]);
    let mut ui = pane(key, &rec);
    assert!(
        ui.find("0003·9 (in band)").is_ok(),
        "the newest row's stamp"
    );
    assert!(ui.find("0003·4 (in band)").is_ok(), "the one before it");
    assert!(
        ui.find("re-stamped in band (0003·4 → 0003·9), same value: a re-put (core §4.2)")
            .is_ok(),
        "the same record under a new seq is a re-put, never a change"
    );

    let rec = recording(&[(10, b"hello world"), (11, b"hello there")]);
    let mut ui = pane(key, &rec);
    assert!(
        ui.find("neither sample has a structural form — compared as bytes, not as fields")
            .is_ok()
    );
    assert!(
        ui.find("11 B → 11 B · first 6 B and last 0 B in common · differs over 6..11 → 6..11")
            .is_ok(),
        "the records' bytes, not the envelopes' (whose seq differs at byte 5)"
    );
}

/// The bound is on screen, not implied by a short list (tooling guide O6).
#[test]
fn the_history_pane_counts_what_it_evicted() {
    use zengui::view::history::{HistoryData, section};

    let script: Vec<(Vec<u8>, bool)> = (0..10)
        .map(|i| (format!(r#"{{"value":{i}}}"#).into_bytes(), false))
        .collect();
    let borrowed: Vec<(&[u8], bool)> = script.iter().map(|(p, d)| (p.as_slice(), *d)).collect();
    let rec = recording(REGISTERED, 3, &borrowed);

    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(REGISTERED),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: None,
        compare_snapshot: false,
    }));
    assert!(
        ui.find("recording since selection · 3 retained · 7 evicted (ring full)")
            .is_ok(),
        "what the bound cost is a fact on screen"
    );
}

/// The `.zsnap` v2 compare (#219; #614, FL8b): the newest sample against
/// the snapshot's row for the key, captioned with the snapshot's moment and
/// span, and with what the snapshot established of its row — the key's
/// identity at collection, the payload against its declared type, whose
/// clock stamped it, and who held it: evidence, not inference.
#[test]
fn the_history_pane_compares_against_a_v2_snapshot_row() {
    use zengui::view::history::{HistoryData, section};
    use zenkey_fleet::report::{
        AnsweredBy, Conformance, Holder, KeyGroup, KeyIdentity, StamperWire,
    };

    let key = common::ETH0;
    let rec = recording(
        key,
        8,
        &[(br#"{"name":"eth0","index":2,"is_up":false}"#, false)],
    );
    let header = zenkey_fleet::ZsnapHeader {
        zsnap: zenkey_fleet::ZSNAP_VERSION,
        selectors: vec!["zk2/*/*/*/state/**".into()],
        base: String::new(),
        collected_at: "2026-10-05T03:00:00Z".into(),
        collection_span_s: 0.42,
        asked: 1,
        answered: 1,
        elided: 0,
        errors: 0,
        discarded: 0,
        superseded: 0,
        presence: None,
    };
    let row = zenkey_fleet::SnapshotRow {
        key: key.into(),
        identity: KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: "state".into(),
                resource: Some("state/interfaces/{ns}/{iface}".into()),
            },
            values: Default::default(),
            unresolved: None,
        },
        delete: false,
        // {"name":"eth0","index":2,"is_up":true}
        bytes: Some("eyJuYW1lIjoiZXRoMCIsImluZGV4IjoyLCJpc191cCI6dHJ1ZX0=".into()),
        encoding: Some("application/json".into()),
        timestamp: Some("2026-10-05T02:59:59Z/a1b2".into()),
        stamper: Some(StamperWire::Owner { id: "a1b2".into() }),
        inband: None,
        source_zid: Some("a1b2".into()),
        conformance: Conformance::Valid,
        holder: Holder::Live {
            address: "host-a/tc".into(),
            answered_by: AnsweredBy::Owner,
        },
    };
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(key),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: Some((&row, &header)),
        compare_snapshot: true,
    }));
    assert!(
        ui.find("snapshot 2026-10-05T03:00:00Z (over 0.42s) → newest sample")
            .is_ok(),
        "a snapshot is collected over a span, never at an instant"
    );
    assert!(
        ui.find("resolved").is_ok(),
        "the row's identity at collection"
    );
    assert!(ui.find("valid").is_ok(), "its payload against its type");
    assert!(
        ui.find(
            "held by host-a/tc, alive at collection — answered by the owner's own session (S4)"
        )
        .is_ok()
    );
    assert!(ui.find("stamped by the owner's clock (a1b2, S1)").is_ok());
    assert!(
        ui.find(
            "stamped in band 0003·12 — the owner's run and seq: its order, never a time (core \
             §4.3)"
        )
        .is_err(),
        "a row with no envelope says nothing of one"
    );
    assert!(
        ui.find("~ is_up  true → false").is_ok(),
        "the snapshot's value against the newest sample, field by field"
    );

    // A value no instance held at collection: said, never attributed.
    let gone = zenkey_fleet::SnapshotRow {
        holder: Holder::NoInstance {
            address: "host-a/tc".into(),
        },
        ..row.clone()
    };
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(key),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: Some((&gone, &header)),
        compare_snapshot: true,
    }));
    assert!(
        ui.find(
            "a value answered and no instance of host-a/tc was present at collection — an \
             owner gone, or a store answering on its keys, which S4 forbids"
        )
        .is_ok()
    );

    // A row the file kept an in-band stamp for (#976 A4): its run and seq,
    // never a time (observer §8.3), beside whose clock set its timestamp.
    let stamped = zenkey_fleet::SnapshotRow {
        inband: Some(zenkey_fleet::report::InbandStamp {
            run: "0003".into(),
            seq: 12,
        }),
        ..row.clone()
    };
    let mut ui = simulator::<Message, _, _>(section(HistoryData {
        slot: zengui::message::SlotId::FOLLOW,
        sp: sp(),
        key: Some(key),
        recorder: Some(&rec),
        watched: true,
        scroll: Default::default(),
        snapshot: Some((&stamped, &header)),
        compare_snapshot: true,
    }));
    assert!(
        ui.find(
            "stamped in band 0003·12 — the owner's run and seq: its order, never a time (core \
             §4.3)"
        )
        .is_ok()
    );
    assert!(ui.find("stamped by the owner's clock (a1b2, S1)").is_ok());
}
