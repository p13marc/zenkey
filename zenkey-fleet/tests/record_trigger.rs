//! Trigger capture over a real bus (#218; `.zrec` version 3 since #612,
//! FJ8a): a rule fires, and one file carries the preamble, the pre-roll,
//! the trigger record and the post-roll — in that order, with the kinds
//! counted apart.
//!
//! The fixture is a zk2 owner (`tc.netif.v1` at `host-a/tc`): one state
//! member that speaks at 2 Hz for a second and a half and then goes quiet,
//! a second state resource that is only ever *held* (put once before the
//! window, answered by the owner's state queryable), and a `silent-for`
//! rule on the first. When the silence fires, the preamble — the owner's
//! own S4 answer — holds exactly the held key under the default semantics,
//! the one the ring cannot tell you about, and both under `full`.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::judge::condition::Condition;
use zenkey_fleet::model::catalog::Revision;
use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::{CondState, ContractSource, PreambleSemantics};
use zenkey_fleet::{Synth, TriggerEvent, TriggerSpec, ZrecItem, ZrecReader, record_on};
use zenkey_model::authoring::Kind;
use zenkey_model::template::Bindings;

mod util;
use util::zk2::{client, config, example, iface, router};

const HEALTH: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
const CONFIG: &str = "zk2/host-a/tc/tc.netif.v1/state/namespaces";
const SELECTOR: &str = "zk2/host-a/tc/tc.netif.v1/state/**";

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

fn spec(rule: &str, semantics: Option<PreambleSemantics>, give_up: Duration) -> TriggerSpec {
    TriggerSpec {
        selectors: vec![SELECTOR.into()],
        pre: Duration::from_secs(2),
        post: Duration::from_secs(1),
        rules: vec![Condition::parse(rule).expect("rule")],
        tick: Duration::from_millis(250),
        timeout: Duration::from_secs(1),
        clocks_synced: false,
        deployment: None,
        contracts: zenkey_fleet::ContractSet::new(),
        give_up: Some(give_up),
        preamble: semantics,
        max_samples: None,
        max_replies: zenkey_fleet::DEFAULT_MAX_REPLIES,
    }
}

/// Run the fixture once under `semantics` and read the file back.
async fn capture(semantics: PreambleSemantics) -> (zenkey_fleet::RecordReport, Vec<Line>) {
    let (_router, ep) = router(None).await;
    let (owners, recorder_session) = (client(&ep).await, client(&ep).await);
    let rev = Revision::from_contract(example("tcgui/tc.netif.v1"), ContractSource::File);
    let netif = iface("tc.netif.v1");
    let synth = Synth::new(5);
    let ns = rev.resource("namespaces", &[Kind::State]).unwrap().clone();
    let ifc = rev
        .resource("interfaces/{ns}/{iface}", &[Kind::State])
        .unwrap()
        .clone();

    // The owner: `namespaces` held (one put before start, nothing in the
    // window), `interfaces/default/eth0` spoken below.
    let mut b = zenkey::ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(zenkey::Implementation::new(example("tcgui/tc.netif.v1")))
        .expect("implement");
    for r in &example("tcgui/tc.netif.v1").resources {
        let _ = b.expose(&netif, &zenkey::implementation::resource_name(r));
    }
    let held = b
        .declare_state_writer(&netif, "state/namespaces", &Bindings::new())
        .await
        .expect("held");
    held.put(synth.sample(&rev, &ns, Member::Type, 0).unwrap().bytes)
        .await
        .expect("held value");
    let member: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let spoken = b
        .declare_state_writer(&netif, "state/interfaces/{ns}/{iface}", &member)
        .await
        .expect("spoken");
    let _service = b.start().await.expect("the owner");

    let (fired_tx, mut fired_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let buf = SharedBuf::default();
    let recorder = tokio::spawn({
        let buf = buf.clone();
        async move {
            record_on(
                &recorder_session,
                "",
                &spec(
                    &format!("silent-for {HEALTH} 0.7"),
                    Some(semantics),
                    Duration::from_secs(20),
                ),
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

    // The recorder's subscriber matches; then 2 Hz for ~1.5 s.
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !spoken.writer().matching().await.unwrap_or(false) {
        assert!(tokio::time::Instant::now() < deadline, "never matched");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for i in 0..4u64 {
        spoken
            .put(synth.sample(&rev, &ifc, Member::Type, i).unwrap().bytes)
            .await
            .expect("put");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // Quiet now. When the silence fires, speak once more: that sample is
    // the post-roll, and must land after the trigger record.
    tokio::time::timeout(util::SETTLE, fired_rx.recv())
        .await
        .expect("the rule fired within the settle window");
    spoken
        .put(synth.sample(&rev, &ifc, Member::Type, 9).unwrap().bytes)
        .await
        .expect("put post");

    let report = recorder.await.expect("join").expect("the capture ran");

    let bytes = buf.0.lock().expect("buffer lock").clone();
    let mut reader = ZrecReader::new(bytes.as_slice()).expect("a .zrec header");
    assert_eq!(reader.header().zrec, 3, "the file is version 3");
    let mut lines = Vec::new();
    while let Some(item) = reader.next() {
        lines.push(match item.expect("a well-formed line") {
            ZrecItem::Preamble { row, .. } => Line::Preamble(row.key),
            ZrecItem::Sample { row, t_us, .. } => {
                assert!(row.qos_axes.is_some(), "version 3 rows carry their axes");
                Line::Sample(row.key, t_us.expect("t"))
            }
            ZrecItem::Trigger(t) => Line::Trigger(t.rule.clone(), t.to),
            ZrecItem::Dropped(n) => Line::Dropped(n),
        });
    }
    (report, lines)
}

/// The acceptance case: one file, in order — header (version 3, the
/// preamble and what the selectors exclude stated) → preamble row (the
/// held key, `t: 0`) → pre-roll rows (the spoken key, ascending `t`, none
/// marked preamble) → the trigger record (`silent-for`, to `firing`) → the
/// post-roll row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_condition_firing_writes_preamble_pre_trigger_and_post_in_one_file() {
    let (report, lines) = capture(PreambleSemantics::AbsentFromWindow).await;

    let header = &report.header;
    assert_eq!(
        header.excluded.as_deref(),
        Some(
            ["@stream", "@state", "@op", "@zk", "@adv"]
                .map(String::from)
                .as_slice()
        ),
        "a plain `state/**` capture names no verbatim chunk (O5)"
    );
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
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_semantics_keep_every_fetched_key() {
    let (report, lines) = capture(PreambleSemantics::Full).await;
    let preamble = report.header.preamble.as_ref().expect("preamble");
    assert_eq!(preamble.semantics, PreambleSemantics::Full);
    assert_eq!(preamble.count, 2, "{preamble:?}");
    let mut keys: Vec<&str> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Preamble(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec![HEALTH, CONFIG]);
}

/// Nothing fires within `give_up`: no file, no trigger, and a report that
/// says so rather than an empty capture — a rule not firing is not a
/// finding. v1's `origin-down` is refused at the parse, naming zk2's
/// `instance-gone`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rule_that_never_fires_leaves_no_file() {
    let (_router, ep) = router(None).await;
    let session = client(&ep).await;
    // A silence claim over a span longer than the run can never be
    // established: unobservable throughout, never firing.
    let never = spec(
        &format!("silent-for {HEALTH} 60"),
        Some(PreambleSemantics::AbsentFromWindow),
        Duration::from_millis(900),
    );
    let opened = Arc::new(Mutex::new(false));
    let mut gave_up = false;
    let report = record_on(
        &session,
        "",
        &never,
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

    let e = Condition::parse("origin-down h-aaaaaaaaaaaa").unwrap_err();
    assert!(e.is_unaskable(), "{e}");
    assert!(e.to_string().contains("instance-gone"), "{e}");
}

/// FJ8b: `invalid-payload` arms a capture again, in its zk2 meaning — a
/// payload that fails its declared type, read through the lens the
/// deployment's session gives — and fires on the owner's first bad put,
/// the good puts before it in the pre-roll.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invalid_payload_fires_a_capture_through_the_lens() {
    let (_router, ep) = router(None).await;
    let (owners, recorder_session) = (client(&ep).await, client(&ep).await);
    let netif = iface("tc.netif.v1");
    let mut b = zenkey::ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(zenkey::Implementation::new(example("tcgui/tc.netif.v1")))
        .expect("implement");
    for r in &example("tcgui/tc.netif.v1").resources {
        let _ = b.expose(&netif, &zenkey::implementation::resource_name(r));
    }
    let member: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let spoken = b
        .declare_state_writer(&netif, "state/interfaces/{ns}/{iface}", &member)
        .await
        .expect("spoken");
    let _service = b.start().await.expect("the owner");

    let (fired_tx, mut fired_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let buf = SharedBuf::default();
    let recorder = tokio::spawn({
        let buf = buf.clone();
        async move {
            let mut s = spec(
                &format!("invalid-payload {SELECTOR}"),
                None,
                Duration::from_secs(20),
            );
            s.deployment = Some(zenkey_fleet::DoctorBus {
                session: recorder_session.clone(),
                raw: recorder_session.clone(),
                namespace: String::new(),
            });
            record_on(
                &recorder_session,
                "",
                &s,
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

    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !spoken.writer().matching().await.unwrap_or(false) {
        assert!(tokio::time::Instant::now() < deadline, "never matched");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Valid values until the rule has judged a clean window, then one that
    // fails its type (`is_up` is a boolean), until it fires.
    let good = br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#.to_vec();
    let bad = br#"{"name":"eth0","index":2,"namespace":"default","is_up":"yes"}"#.to_vec();
    for _ in 0..4 {
        spoken.put(good.clone()).await.expect("put");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let fired = async {
        loop {
            spoken.put(bad.clone()).await.expect("put");
            tokio::select! {
                _ = fired_rx.recv() => break,
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    };
    tokio::time::timeout(util::SETTLE, fired)
        .await
        .expect("the rule fired within the settle window");
    let report = recorder.await.expect("join").expect("the capture ran");
    let trigger = report.trigger.expect("it fired");
    assert_eq!(trigger.to, CondState::Firing);
    assert!(
        trigger.evidence.contains("failed their declared type"),
        "{}",
        trigger.evidence
    );
    assert!(trigger.evidence.contains("/is_up"), "{}", trigger.evidence);
}
