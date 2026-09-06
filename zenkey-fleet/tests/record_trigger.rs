//! Trigger capture over a real bus (#218; RFC 13 §4.1 version 2): a rule
//! fires, and one file carries the preamble, the pre-roll, the trigger
//! record and the post-roll — in that order, with the kinds counted apart.
//!
//! The fixture: a `state` key that speaks at 2 Hz for a second and a half
//! and then goes quiet, a second `state` key that is only ever *held* (a
//! responder answers GETs for it; nothing publishes it inside the window),
//! and a `silent-for` rule on the first. When the silence fires, the
//! preamble under the default semantics holds exactly the held key — the
//! one the ring cannot tell you about — and under `full` holds both.
//! Ports are ephemeral (`util::peer_pair`).

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::judge::condition::Condition;
use zenkey_fleet::report::{CondState, PreambleSemantics};
use zenkey_fleet::{
    TriggerEvent, TriggerSpec, ZrecItem, ZrecReader, declare_publication, declare_responder,
    record_on,
};

mod util;
use util::peer_pair;

const HEALTH: &str = "v1/h-aaaaaaaaaaaa/state/demo/health";
const CONFIG: &str = "v1/h-aaaaaaaaaaaa/state/demo/config";
const SELECTOR: &str = "v1/h-aaaaaaaaaaaa/state/demo/**";

/// A byte sink the test can read back; the sink owns its writer on the
/// blocking pool (#332), so the bytes are shared rather than handed back.
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer lock").write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Everything one line of the file can be, flattened for assertions.
#[derive(Debug)]
enum Line {
    Preamble(String),
    Sample(String, u64),
    Trigger(String, CondState),
    Dropped(u64),
}

/// Run the fixture once under `semantics` and read the file back.
async fn capture(semantics: PreambleSemantics) -> (zenkey_fleet::RecordReport, Vec<Line>) {
    let (a, b) = peer_pair().await;

    // The held keys: GETs on either are answered from `a`, which is what a
    // storage or a state-serving producer does — nothing publishes `config`
    // inside the window, so only the preamble can carry it.
    let config = declare_responder(&a, CONFIG, br#"{"mode":"x"}"#.to_vec(), None, true)
        .await
        .expect("config responder");
    let health_held = declare_responder(&a, HEALTH, br#"{"ok":false}"#.to_vec(), None, true)
        .await
        .expect("health responder");
    let serving = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(q) = config.next() => { config.answer(q).await; }
                Some(q) = health_held.next() => { health_held.answer(q).await; }
                else => break,
            }
        }
    });

    let publication =
        declare_publication(&a, HEALTH, QosProfile::Transition, Some("application/json"))
            .await
            .expect("declare");
    let matching = publication.matching_events().await.expect("matching");

    let (fired_tx, mut fired_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let buf = SharedBuf::default();
    let recorder = tokio::spawn({
        let b = b.clone();
        let buf = buf.clone();
        async move {
            let fleet = zenkey_fleet::Fleet::new(&b, "");
            let store = zenkey_fleet::SchemaStore::new("", Duration::from_millis(300));
            let spec = TriggerSpec {
                selectors: vec![SELECTOR.into()],
                pre: Duration::from_secs(2),
                post: Duration::from_secs(1),
                rules: vec![Condition::parse(&format!("silent-for {HEALTH} 0.7")).expect("rule")],
                tick: Duration::from_millis(250),
                timeout: Duration::from_secs(1),
                give_up: Some(Duration::from_secs(20)),
                preamble: Some(semantics),
                max_samples: None,
                max_replies: zenkey_fleet::DEFAULT_MAX_REPLIES,
            };
            record_on(
                &fleet,
                None,
                &store,
                &spec,
                || async move { Ok(buf) },
                |ev| {
                    if let TriggerEvent::Fired(_) = ev {
                        let _ = fired_tx.send(());
                    }
                },
            )
            .await
        }
    });

    // The recorder's subscriber raises the badge; then 2 Hz for ~1.5 s.
    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
            .await
            .expect("matching within the settle window")
            .expect("listener alive")
    );
    for i in 0..4u8 {
        publication
            .send(format!(r#"{{"ok":true,"n":{i}}}"#).into_bytes(), None)
            .await
            .expect("send");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // Quiet now. When the silence fires, speak once more: that sample is
    // the post-roll, and must land after the trigger record.
    tokio::time::timeout(util::SETTLE, fired_rx.recv())
        .await
        .expect("the rule fired within the settle window");
    publication
        .send(br#"{"ok":true,"post":true}"#.to_vec(), None)
        .await
        .expect("send post");

    let report = recorder.await.expect("join").expect("the capture ran");
    serving.abort();

    let bytes = buf.0.lock().expect("buffer lock").clone();
    let mut reader = ZrecReader::new(bytes.as_slice()).expect("a .zrec header");
    assert_eq!(reader.header().zrec, 2, "the file is version 2");
    let mut lines = Vec::new();
    while let Some(item) = reader.next() {
        lines.push(match item.expect("a well-formed line") {
            ZrecItem::Preamble { row, .. } => Line::Preamble(row.key),
            ZrecItem::Sample { row, t_us, .. } => Line::Sample(row.key, t_us.expect("t")),
            ZrecItem::Trigger(t) => Line::Trigger(t.rule.clone(), t.to),
            ZrecItem::Dropped(n) => Line::Dropped(n),
        });
    }
    (report, lines)
}

/// The acceptance case: one file, in order — header (version 2, the
/// preamble stated) → preamble row (the held key, `t: 0`) → pre-roll rows
/// (the spoken key, ascending `t`, none marked preamble) → the trigger
/// record (`silent-for`, to `firing`) → the post-roll row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_condition_firing_writes_preamble_pre_trigger_and_post_in_one_file() {
    let (report, lines) = capture(PreambleSemantics::AbsentFromWindow).await;

    let header = &report.header;
    let preamble = header
        .preamble
        .as_ref()
        .expect("the header states the preamble");
    assert_eq!(preamble.semantics, PreambleSemantics::AbsentFromWindow);
    assert_eq!(
        preamble.count, 1,
        "only the key the ring cannot show: {preamble:?}"
    );
    assert_eq!(preamble.selectors, vec![SELECTOR.to_string()]);
    assert!(preamble.failed.is_empty(), "{preamble:?}");
    let pre_roll = header
        .pre_roll
        .as_ref()
        .expect("the header states the pre-roll");
    assert!(
        (pre_roll.asked_s - 2.0).abs() < 1e-9 && pre_roll.covered_s <= 2.0,
        "{pre_roll:?}"
    );
    assert_eq!(pre_roll.watched, vec![SELECTOR.to_string()]);
    assert_eq!(pre_roll.evicted, 0);
    assert_eq!(report.preamble_rows, 1);
    let trigger = report
        .trigger
        .as_ref()
        .expect("the report names the trigger");
    assert!(trigger.rule.starts_with("silent-for"), "{trigger:?}");
    assert_eq!(trigger.to, CondState::Firing);
    assert!(
        report.samples >= 4,
        "pre-roll and post-roll rows: {report:?}"
    );

    // The file order, exactly.
    let mut i = 0;
    assert!(
        matches!(&lines[i], Line::Preamble(k) if k == CONFIG),
        "line 1 is the held key's preamble row: {lines:?}"
    );
    i += 1;
    let mut last_t = 0u64;
    let mut pre_rows = 0;
    while let Line::Sample(key, t) = &lines[i] {
        assert_eq!(key, HEALTH);
        assert!(*t >= last_t, "ascending t: {lines:?}");
        last_t = *t;
        pre_rows += 1;
        i += 1;
    }
    assert!(pre_rows >= 3, "the ring held the 2 Hz burst: {lines:?}");
    assert!(
        matches!(&lines[i], Line::Trigger(rule, CondState::Firing) if rule.starts_with("silent-for")),
        "the trigger record sits after the pre-roll: {lines:?}"
    );
    i += 1;
    assert!(
        lines[i..]
            .iter()
            .any(|l| matches!(l, Line::Sample(k, _) if k == HEALTH)),
        "the post-roll row lands after the trigger: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| matches!(l, Line::Dropped(n) if *n > 0)),
        "a quiet fixture drops nothing: {lines:?}"
    );
}

/// `full` semantics keep every fetched key — the spoken one too, though the
/// ring already holds its story.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_semantics_keep_every_fetched_key() {
    let (report, lines) = capture(PreambleSemantics::Full).await;
    let preamble = report.header.preamble.as_ref().expect("preamble");
    assert_eq!(preamble.semantics, PreambleSemantics::Full);
    assert_eq!(preamble.count, 2, "{preamble:?}");
    let keys: Vec<&str> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Preamble(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        vec![CONFIG, HEALTH],
        "preamble rows come first, by key"
    );
}

/// Nothing fires within `give_up`: no file, no trigger, and a report that
/// says so rather than an empty capture — a rule not firing is not a
/// finding.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rule_that_never_fires_leaves_no_file() {
    let (_a, b) = peer_pair().await;
    let fleet = zenkey_fleet::Fleet::new(&b, "");
    let store = zenkey_fleet::SchemaStore::new("", Duration::from_millis(300));
    let spec = TriggerSpec {
        selectors: vec![SELECTOR.into()],
        pre: Duration::from_secs(2),
        post: Duration::from_secs(1),
        // A silence claim over a span longer than the run can never be
        // established: unobservable throughout, never firing.
        rules: vec![Condition::parse(&format!("silent-for {HEALTH} 60")).expect("rule")],
        tick: Duration::from_millis(200),
        timeout: Duration::from_millis(300),
        give_up: Some(Duration::from_millis(900)),
        preamble: Some(PreambleSemantics::AbsentFromWindow),
        max_samples: None,
        max_replies: zenkey_fleet::DEFAULT_MAX_REPLIES,
    };
    let opened = Arc::new(Mutex::new(false));
    let mut gave_up = false;
    let report = record_on(
        &fleet,
        None,
        &store,
        &spec,
        || async {
            *opened.lock().expect("lock") = true;
            Ok(SharedBuf::default())
        },
        |ev| {
            if let TriggerEvent::GaveUp { .. } = ev {
                gave_up = true;
            }
        },
    )
    .await
    .expect("a run that gives up is still a clean run");
    assert!(gave_up);
    assert!(!*opened.lock().expect("lock"), "no file was opened");
    assert!(report.out.is_none() && report.trigger.is_none());
    assert_eq!(report.samples, 0);
    assert_eq!(report.header.preamble, None);
}
