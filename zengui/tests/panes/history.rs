//! The history pane (#63).

use super::*;

// ── History pane (#63) ───────────────────────────────────────────────────

/// Build a recorder over a scripted sample stream.
fn recording(key: &str, max_entries: usize, samples: &[(&[u8], bool)]) -> HistoryRecorder {
    let mut rec = HistoryRecorder::new(key, max_entries);
    for (payload, is_delete) in samples {
        rec.observe(&zenkey_fleet_v1::SampleView {
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
             was selected. That is not a statement about the bus (RFC 05 §3.1)."
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

/// A tombstone is authoritative retirement (RFC 04 §1.2), and the put that
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
            ui.find("retired — an authoritative delete (RFC 04 §1.2), not an empty value")
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

/// The bound is on screen, not implied by a short list (RFC 09 §5.1 O6).
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
