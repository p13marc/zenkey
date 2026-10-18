//! The window's own tests: what a tick does to it, and what a base change
//! does to it (#175).
//!
//! A child module of [`app`](super) rather than a `mod tests` inside it,
//! because these are the only tests that need the whole `Zengui` — and its six
//! fields are private, destructured in exactly two places. A `#[path]` child
//! keeps that true while keeping `app.rs` about the Elm loop.
//!
//! Everything narrower moved with its subject: the placement table to
//! `state/`, the watch-coverage rule to `view/panes.rs`.
//!
//! One module per subject (#614, FL5), each a child of this one, so the
//! chunks that port them in parallel do not meet in one file: `tree`,
//! `subject`, `replay`, `windows`, `slots`, `tuning`, `link` and `send`.
//! The fixtures more than one of them drives stay here.

use std::sync::Arc;

use super::{Zengui, test_app};
use crate::message::{BusTick, Message, Subject, SubjectMsg};
use crate::state::tree::{KeyCounts, shape_held};
use crate::update;

mod config;
mod link;
mod replay;
mod send;
mod slots;
mod subject;
mod tree;
mod tuning;
mod windows;

/// Feed one tick through the five sub-states `apply_tick` names.
fn tick_into(app: &mut Zengui, tick: &BusTick) {
    update::bus::apply_tick(
        &mut app.dep,
        &mut app.obs,
        &mut app.sub,
        &mut app.tree,
        &mut app.work,
        tick,
    );
}

/// A tick that changes nothing about the key set or the watch set.
fn tick(keys: usize, evicted: u64, unwatched: u64, watched: &Arc<[String]>) -> BusTick {
    let stats = zenkey_fleet::StatsTable::new();
    BusTick {
        tree: Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats)),
        samples: Vec::new(),
        lagged: 0,
        coalesced: 0,
        keys,
        keys_evicted: evicted,
        keys_unwatched: unwatched,
        watched: Arc::clone(watched),
        seeded: Vec::new(),
        totals: Default::default(),
    }
}

/// Synthetic traffic for the retained-window tests (#217): ~88 s of
/// watched keys with an LWW update, a tombstone, an attachment and one
/// key that is not this convention at all (O1 — a window curates nothing).
fn traffic(epoch: std::time::Instant) -> Vec<zenkey_fleet::SampleView> {
    use std::time::Duration;
    use zenoh::sample::SampleKind;
    let view =
        |t_s: u64, key: &str, payload: &[u8], encoding: &str, kind, attachment: Option<&[u8]>| {
            zenkey_fleet::SampleView {
                key: key.to_string(),
                payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
                encoding: encoding.to_string(),
                kind,
                timestamp: None,
                stamped_by: None,
                attachment: attachment.map(|a| zenoh::bytes::ZBytes::from(a.to_vec())),
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: epoch + Duration::from_secs(t_s),
            }
        };
    let put = SampleKind::Put;
    vec![
        view(0, "v1/h-0123456789ab/state/p/a", b"1", "", put, None),
        view(
            5,
            "v1/h-0123456789ab/state/p/b",
            br#"{"ok":true}"#,
            "application/json",
            put,
            None,
        ),
        view(
            20,
            "v1/h-0123456789ab/telemetry/x/m",
            b"12345678",
            "",
            put,
            None,
        ),
        view(30, "v1/h-0123456789ab/state/p/a", b"2", "", put, None),
        view(40, "not/this/convention", b"x", "", put, None),
        view(
            55,
            "v1/h-0123456789ab/state/p/b",
            b"",
            "",
            SampleKind::Delete,
            None,
        ),
        view(
            60,
            "v1/h-0123456789ab/telemetry/x/m",
            b"87654321",
            "",
            put,
            Some(b"meta"),
        ),
        view(75, "v1/h-0123456789ab/state/p/a", b"3", "", put, None),
        view(88, "v1/h-0123456789ab/state/p/c", b"zzz", "", put, None),
    ]
}

/// The app with its windows open (#186): what `boot` makes, minus the link —
/// `open_windows` mints real `window::Id`s without a display, because
/// `window::open` only *returns* a task.
fn booted() -> Zengui {
    let mut app = test_app();
    let _ = app.open_windows();
    app
}

/// Run a task to quiescence the way the runtime would: every message it
/// outputs goes back through `update`, and every task *that* returns is run
/// too. Window and widget actions are dropped — a headless test has neither.
async fn drive(app: &mut Zengui, task: iced::Task<Message>) {
    use iced::futures::StreamExt as _;
    use iced_test::runtime::{Action, task::into_stream};

    let mut queue = vec![task];
    let mut steps = 0;
    while let Some(task) = queue.pop() {
        let Some(mut stream) = into_stream(task) else {
            continue;
        };
        while let Some(action) = stream.next().await {
            if let Action::Output(message) = action {
                steps += 1;
                assert!(steps < 100, "the boot tasks did not settle");
                queue.push(app.update(message));
            }
        }
    }
}

/// The boot, with a dead router and `--contracts`, and nothing faked: the
/// real open task, the real failure, the real contracts load (#614, FL6a).
async fn boot_against(connect: &str) -> Zengui {
    let mut settings = super::test_settings();
    settings.connect = vec![connect.to_string()];
    settings.contracts = vec![
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/walkthrough"),
    ];
    let (mut app, open) = Zengui::with_prefs(settings, crate::prefs::Prefs::default(), None);
    tokio::time::timeout(std::time::Duration::from_secs(10), drive(&mut app, open))
        .await
        .expect("a dead router fails the open at once — the boot never hangs");
    app
}
