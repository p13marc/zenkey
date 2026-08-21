//! Capturing the live stream to a `.zrec` file (#74).
//!
//! One function, and it is here rather than in [`super::sweep`] because it is
//! neither a query nor a write: it is a long-lived tap on the monitor that
//! ends when the app says so. The `Notify` is the only way to stop it — a
//! recording that could only be ended by closing the window would lose its
//! trailer.

use std::sync::Arc;

use iced::Task;
use zenkey_fleet::Monitor;

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
