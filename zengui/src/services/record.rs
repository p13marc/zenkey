//! The `.zrec` file, both directions (#74): capturing the live stream to
//! one, and loading one back (#255).
//!
//! Here rather than in [`super::sweep`] because none of it is a query or a
//! bus write: the capture is a long-lived tap on the monitor that ends when
//! the app says so — the stop signal is the only way to end it, and a
//! recording that could only be ended by closing the window would lose its
//! trailer — and the load is that tap read back.
//!
//! ## The stop signal is a `oneshot`, not a `Notify` (#335)
//!
//! A `Notify` stores no permit for `notify_waiters`, and this task does real
//! work before it can wait on anything: iced has to poll it at all, then
//! `monitor.watched()` awaits. Toggling Record off in that window fired into
//! nothing, the toggle had already taken the handle out of the state, and the
//! capture then held an `EventStream` and an open file for the rest of the
//! process's life with nothing left to stop it with. A `oneshot` **stores**
//! its value: fired before the receiver is ever polled, it is still there when
//! the receiver is polled. Dropping the handle ends the capture too — a
//! sender that goes away resolves the receiver — so no path leaks the task.

use std::sync::Arc;
use std::time::Instant;

use iced::Task;
use zenkey_fleet::{Monitor, MonitorCore, SampleView};

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
    stop: tokio::sync::oneshot::Receiver<()>,
) -> Task<Message> {
    Task::perform(
        async move {
            let selectors: Vec<String> = monitor
                .watched()
                .await
                .into_iter()
                .map(|(_, s)| s)
                .collect();
            capture(monitor.core(), selectors, path, base, stop).await
        },
        |r| Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::RecordFinished(r))),
    )
}

/// The capture proper: open the file, tap the core, write until `stop`.
///
/// Split from [`start`] on the one seam that matters for the defect — it takes
/// a [`MonitorCore`], which is constructible without a session, so the
/// stop-during-setup window is testable offline.
async fn capture(
    core: &Arc<MonitorCore>,
    selectors: Vec<String>,
    path: String,
    base: String,
    stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<(u64, u64, String), String> {
    let header = zenkey_fleet::ZrecHeader {
        zrec: zenkey_fleet::ZREC_VERSION,
        selectors,
        base,
        captured_at: zenkey_fleet::tape::record::rfc3339_now(),
    };
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut writer = zenkey_fleet::ZrecWriter::new(std::io::BufWriter::new(file), &header)
        .map_err(|e| e.to_string())?;
    let mut events = core.events();
    let recording = zenkey_fleet::record(
        &mut events,
        &mut writer,
        zenkey_fleet::RecordBounds::default(),
        |_, _| {},
    );
    tokio::select! {
        // Biased on the stop: a capture told to end does not get one more
        // sample in first, however much traffic is queued behind it.
        biased;
        _ = stop => {}
        r = recording => r.map_err(|e| e.to_string())?,
    }
    let (samples, dropped) = writer.counts();
    writer.finish().map_err(|e| e.to_string())?;
    Ok((samples, dropped, path))
}

/// Load a `.zrec` for replay, off the update thread (#255): the parse is
/// bounded only by the file, so on the render thread it froze the window for
/// as long as a capture took to read, with no upper limit and no sign that
/// anything was happening. Lands on [`ReplayMsg::Loaded`] — whose handler
/// clears the loading state the `Open` handler raised (RFC 09 §5.1 O4:
/// "loading" is not "empty", and the Activity replay tab says which).
pub fn load(path: String) -> Task<Message> {
    Task::perform(
        async move {
            let parsed = std::fs::File::open(&path)
                .map_err(|e| e.to_string())
                .and_then(|f| crate::replay::ReplayState::load(&path, std::io::BufReader::new(f)));
            (path, parsed.map(crate::replay::LoadedReplay::new))
        },
        |(path, r)| Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::Loaded(path, r))),
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
        captured_at: zenkey_fleet::tape::record::rfc3339_now(),
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

    /// #335: the stop lands *inside* the setup window — before the capture is
    /// polled at all, which is the widest form of it (iced has to schedule the
    /// task, and `monitor.watched()` awaits before this is reached). The old
    /// `Notify::notify_waiters` stored nothing, so with no waiter registered
    /// the toggle was a no-op: the capture ran for the process lifetime
    /// holding an `EventStream` and an open file, and the handler had already
    /// dropped the only handle. A `oneshot` keeps the value, so the capture
    /// finds it the first time it looks.
    #[tokio::test]
    async fn a_stop_fired_before_the_capture_is_polled_still_ends_it() {
        let dir = std::env::temp_dir().join(format!("zengui-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stop-window.zrec");

        let core = zenkey_fleet::MonitorCore::new(64);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        // Toggle off before the capture future exists, let alone runs.
        stop.send(()).unwrap();

        let done = tokio::time::timeout(
            Duration::from_secs(5),
            capture(
                &core,
                vec!["v1/**".to_string()],
                path.display().to_string(),
                String::new(),
                stopped,
            ),
        )
        .await
        .expect("the capture ignored a stop fired during its setup window (#335)")
        .expect("the capture failed");

        assert_eq!(done.0, 0, "nothing was published, so nothing was captured");
        // Stopped, not orphaned: the trailer is on disk, so the file is a
        // finished recording rather than a handle nobody holds.
        let state = crate::replay::ReplayState::load(
            &done.2,
            std::io::BufReader::new(std::fs::File::open(&path).unwrap()),
        )
        .unwrap();
        assert_eq!(state.rows.len(), 0);
        let _ = std::fs::remove_file(&path);
    }

    /// The other half of "cannot be lost": the toggle handler takes the handle
    /// out of the state, so a handle dropped without being fired — a closed
    /// window, a panicking update — must end the capture too.
    #[tokio::test]
    async fn a_dropped_handle_ends_the_capture_as_well() {
        let dir = std::env::temp_dir().join(format!("zengui-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dropped-handle.zrec");

        let core = zenkey_fleet::MonitorCore::new(64);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        drop(stop);

        tokio::time::timeout(
            Duration::from_secs(5),
            capture(
                &core,
                Vec::new(),
                path.display().to_string(),
                String::new(),
                stopped,
            ),
        )
        .await
        .expect("a dropped stop handle left the capture running (#335)")
        .expect("the capture failed");
        let _ = std::fs::remove_file(&path);
    }

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
