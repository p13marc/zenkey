//! Capturing the live stream to a `.zrec` file (#74).
//!
//! One function, and it is here rather than in [`super::sweep`] because it is
//! neither a query nor a write: it is a long-lived tap on the monitor that
//! ends when the app says so. The `Notify` is the only way to stop it — a
//! recording that could only be ended by closing the window would lose its
//! trailer.

use std::sync::Arc;
use std::time::Instant;

use iced::Task;
use zenkey_fleet::{Monitor, SampleView};

use crate::message::{Message, WorkspaceMsg};
use crate::view::replay::ReplayMsg;

/// Record every watched selector until `stop` fires.
///
/// The header names the selectors as the monitor knows them, so a replay of
/// this file says what was actually being observed, not what the scope preset
/// claimed.
pub fn start(
    monitor: Arc<Monitor>,
    path: String,
    base: String,
    stop: Arc<tokio::sync::Notify>,
) -> Task<Message> {
    Task::perform(
        async move {
            let selectors: Vec<String> = monitor
                .watched()
                .await
                .into_iter()
                .map(|(_, s)| s)
                .collect();
            let header = zenkey_fleet::ZrecHeader {
                zrec: zenkey_fleet::ZREC_VERSION,
                selectors,
                base,
                captured_at: zenkey_fleet::record::rfc3339_now(),
            };
            let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            let mut writer = zenkey_fleet::ZrecWriter::new(std::io::BufWriter::new(file), &header)
                .map_err(|e| e.to_string())?;
            let mut events = monitor.events();
            let recording = zenkey_fleet::record(
                &mut events,
                &mut writer,
                zenkey_fleet::RecordBounds::default(),
                |_, _| {},
            );
            tokio::select! {
                r = recording => r.map_err(|e| e.to_string())?,
                _ = stop.notified() => {}
            }
            let (samples, dropped) = writer.counts();
            writer.finish().map_err(|e| e.to_string())?;
            Ok((samples, dropped, path))
        },
        |r| Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::RecordFinished(r))),
    )
}

/// Write a retained window through the ordinary `.zrec` writer (#217): the
/// header names the watches the ring was fed under (O4/O5), and the epoch
/// is the window's own start, so every row keeps the arrival offset the
/// ring preserved ([`zenkey_fleet::ZrecWriter::new_at`]). The result is
/// indistinguishable from a file recorded deliberately at that moment.
///
/// Split from [`save_window`] so the round trip — write a window, load it
/// back, get the same rows — is testable without a task runtime.
pub(crate) fn write_window(
    rows: &[Arc<SampleView>],
    epoch: Instant,
    selectors: Vec<String>,
    base: String,
    out: impl std::io::Write,
) -> Result<u64, String> {
    let header = zenkey_fleet::ZrecHeader {
        zrec: zenkey_fleet::ZREC_VERSION,
        selectors,
        base,
        captured_at: zenkey_fleet::record::rfc3339_now(),
    };
    let mut writer =
        zenkey_fleet::ZrecWriter::new_at(out, &header, epoch).map_err(|e| e.to_string())?;
    for view in rows {
        writer.write_sample(view).map_err(|e| e.to_string())?;
    }
    let (samples, _) = writer.counts();
    writer.finish().map_err(|e| e.to_string())?;
    Ok(samples)
}

/// Save the retained window to `path`. Lands on the same message a live
/// capture lands on — the dock's capture line reports it either way. The
/// drop count is zero by construction: the ring sits on the ingest path,
/// upstream of the broadcast's lag, and its own eviction ledger stays in
/// the banner as its own number (O6), not in the file as a fake gap.
pub fn save_window(
    rows: Vec<Arc<SampleView>>,
    epoch: Instant,
    selectors: Vec<String>,
    base: String,
    path: String,
) -> Task<Message> {
    Task::perform(
        async move {
            let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
            let samples =
                write_window(&rows, epoch, selectors, base, std::io::BufWriter::new(file))?;
            Ok((samples, 0, path))
        },
        |r| Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::RecordFinished(r))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The save path round-trips (#217): a window written through
    /// `write_window` loads back with the same rows at the same offsets —
    /// which is the half of "indistinguishable from a deliberate recording"
    /// that a unit test can pin (the byte-identical panes live in
    /// `app_tests`).
    #[test]
    fn a_saved_window_loads_back_row_for_row() {
        let epoch = Instant::now();
        let view = |t_ms: u64, key: &str, payload: &[u8]| {
            Arc::new(SampleView {
                key: key.to_string(),
                payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
                encoding: String::new(),
                kind: zenoh::sample::SampleKind::Put,
                timestamp: None,
                stamped_by: None,
                attachment: None,
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: epoch + Duration::from_millis(t_ms),
            })
        };
        let rows = vec![
            view(0, "v1/h-0123456789ab/state/p/a", b"1"),
            view(1500, "v1/h-0123456789ab/state/p/b", b"2"),
        ];
        let mut bytes = Vec::new();
        let written = write_window(
            &rows,
            epoch,
            vec!["v1/**".to_string()],
            String::new(),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(written, 2);

        let state = crate::replay::ReplayState::load("w.zrec", bytes.as_slice()).unwrap();
        assert_eq!(state.rows.len(), 2);
        assert_eq!(state.rows[0].t_us, 0);
        assert_eq!(
            state.rows[1].t_us, 1_500_000,
            "the ring's pacing survives the file"
        );
        assert_eq!(state.rows[1].view.key, "v1/h-0123456789ab/state/p/b");
        assert_eq!(state.malformed, 0);
        assert_eq!(state.capture_dropped, 0);
        let crate::replay::ReplaySource::File { header, .. } = &state.source else {
            panic!("a loaded file is a file");
        };
        assert_eq!(header.selectors, vec!["v1/**".to_string()]);
    }
}
