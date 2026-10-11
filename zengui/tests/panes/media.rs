//! The media pane (#69).

use super::*;

/// The media pane (issue #69) discovers declared streams off the loaded
/// slices (#77), and is honest twice over: a rung nobody can decode is
/// listed as metadata only, and no registry means "not asked" (O4).
#[test]
fn the_media_pane_lists_declared_streams_honestly() {
    use zengui::view::media::{MediaState, section};

    let state = MediaState::default();

    // Not asked ≠ nothing declared.
    let mut ui = simulator::<Message, _, _>(section(&state, None, sp()));
    assert!(
        ui.find(
            "no registry loaded — declared streams unknown (not asked, O4); \
             the bus serves them via introspect (RFC 08 §6, v1.16)"
        )
        .is_ok()
    );

    let slice = zenkey_v1::slice::parse_slice(
        r#"
[registry]
version = "1.3"
app = "spray"
convention = 1
[producer]
name = "parallax"
[[media]]
path = "{stream}/preview/png"
encoding = "image/png"
[[media]]
path = "{stream}/video/{codec}/{tier}"
encoding = "video/*"
"#,
    )
    .unwrap();
    let slices = SliceSet::from_slices(vec![slice]);
    let mut ui = simulator::<Message, _, _>(section(&state, Some(&slices), sp()));
    assert!(ui.find("parallax declares:").is_ok());
    assert!(
        ui.find("  {stream}/preview/png (image/png)").is_ok(),
        "the decodable rung is offered"
    );
    assert!(
        ui.find(
            "  {stream}/video/{codec}/{tier} (video/*) — metadata only, no \
             decode story yet"
        )
        .is_ok(),
        "the video rung is listed, never pretended"
    );
}

/// A viewing measures its own facts client-side and says which clock; a
/// frame in a codec we cannot decode is reported, not rendered.
#[test]
fn a_media_viewing_reports_what_it_cannot_render() {
    use zengui::view::media::{MediaState, Viewing, section};

    let mut viewing = Viewing::new("v1/h-0123456789ab/@media/parallax/cam0/video/h264/hi".into());
    let sample = zenkey_fleet::SampleView {
        key: viewing.key.clone(),
        payload: zenoh::bytes::ZBytes::from(vec![0u8; 4096]),
        encoding: "video/h264".into(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: Some(zenoh::bytes::ZBytes::from(
            br#"{"seq":7,"keyframe":true}"#.to_vec(),
        )),
        priority: zenoh::qos::Priority::InteractiveHigh,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: true,
        source: None,
        received: Instant::now(),
    };
    viewing.on_frame(&sample);
    assert_eq!(viewing.frames, 1);
    assert!(viewing.fps().is_none(), "one frame is not a rate");

    let state = MediaState {
        viewing: Some(viewing),
        ..MediaState::default()
    };
    let mut ui = simulator::<Message, _, _>(section(&state, None, sp()));
    assert!(
        ui.find(
            "frame arrived: 4096 B as video/h264 — no decode story for this \
             codec; shown as metadata only (RFC 07 §1: the codec is the \
             wire Encoding, and pretending to render it would be a lie)"
        )
        .is_ok()
    );
    assert!(
        ui.find(r#"frame meta (attachment): {"keyframe":true,"seq":7}"#)
            .is_ok(),
        "the attachment metadata reaches the screen"
    );
}
