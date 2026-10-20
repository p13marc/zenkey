//! `spec/profiles/alarms/scenarios.md` §1–§7 (`alarms.v1`, #800, AB), one
//! test per section, named after it. §8–§10 are a tool's (#800, AC).
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default. The owner `lab/svc` implements `alarms.v1` through
//! [`ServiceBuilder::alarms`], as a client of R1. S records every delivery
//! under `zk2/lab/svc/alarms.v1/**` (arrival on this host's monotonic
//! clock, key, kind, payload, stamp), with a consumer's subscription on the
//! alarms for their freshness; G GETs `zk2/lab/svc/alarms.v1/state/alarms/*`
//! as core S4 says, through a tool's consumer, its clock on the
//! deployment's word (every session here runs on one host). "Reads" is
//! `zenkey_model::alarms::Track` over S's deliveries of a key, and every
//! verdict is `zenkey_model::alarms`'s judges'.
//!
//! **The horizon.** §2 waits 455 s and §6 about 910 s at the contract's
//! 900 s: they run in the slow tier, `#[ignore]`d. Their fast variants run
//! the same owner code on a test's copy of the standard contract whose
//! `freshness.ttl_s` is 4, never published: [`ServiceBuilder::alarms_with`]
//! takes the confirmation period from it (1.9 s), so the fast variants
//! check what the owner puts, and the slow ones the verdicts at 900 s.

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{T, addr, client, config, eventually, eventually_some, router};
use zenkey::alarms::{
    self, Act, Alarms, Condition, FileRecord, Retired, Severity, Source, Startup,
};
use zenkey::consumer::{Consumer, Subscription};
use zenkey::model::alarms::{
    self as am, Alarm, Answer, Judged, Latest, Listing, Presence, Reading, Reason, Seed,
    SetVerdict, Track, Transition,
};
use zenkey::model::freshness::{ClockTrust, DEFAULT_DELTA, Observation};
use zenkey::model::grammar::{InstanceId, ZkKey};
use zenkey::state::{Current, StateAnswer};
use zenkey::{Implementation, ServiceBuilder, ServiceConfig, presence};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

/// The owner of `scenarios.md`'s conventions.
const SVC: &str = "lab/svc";

/// A's alert key (§2.2).
const A_KEY: &str = "a659f813308ad1da";

/// The alarms' resource, as a consumer names it.
const RES: &str = alarms::ALARMS;

/// The alarm A at `severity`: rule `link_down`, labels `{peer: r2, port:
/// eth0}`.
fn alarm_a(severity: Severity) -> Condition {
    Condition::new("link_down", severity, "link to r2 down on eth0")
        .label("peer", "r2")
        .label("port", "eth0")
}

/// The alarm B: rule `disk_full`, labels `{mount: /var}`.
fn alarm_b() -> Condition {
    Condition::new("disk_full", Severity::Warning, "/var is 97% full").label("mount", "/var")
}

/// The key of the alarm `alert_key` of `lab/svc`.
fn key_of(alert_key: &str) -> String {
    format!("zk2/{SVC}/alarms.v1/state/alarms/{alert_key}")
}

/// A test's copy of the standard contract, its `freshness.ttl_s` set to
/// `ttl_s`: never published, never in `spec/`.
fn fast(ttl_s: u64) -> Implementation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/alarms/alarms.v1.toml");
    let standard = std::fs::read_to_string(&path).unwrap();
    let text = standard.replace(
        "\"freshness.ttl_s\" = 900",
        &format!("\"freshness.ttl_s\" = {ttl_s}"),
    );
    assert_ne!(text, standard, "the horizon is the standard's 900");
    let l = zenkey::model::contract::load_str(&text, path.parent().unwrap(), None);
    Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

/// One delivery, as S records it.
#[derive(Debug, Clone)]
struct Rec {
    at: Instant,
    key: String,
    delete: bool,
    payload: Vec<u8>,
    stamp: Option<Timestamp>,
}

impl Rec {
    fn alert_key(&self) -> Option<&str> {
        self.key.split_once("/state/alarms/").map(|x| x.1)
    }

    fn is_instance(&self) -> bool {
        self.key.contains("/@zk/instance/")
    }

    fn doc(&self) -> Alarm {
        am::decode(&self.payload).expect("an alarms.v1 document")
    }

    fn stamp_id(&self) -> String {
        self.stamp.expect("stamped").get_id().to_string()
    }
}

type Log = Arc<Mutex<Vec<Rec>>>;

fn snapshot(log: &Log) -> Vec<Rec> {
    log.lock().unwrap().clone()
}

fn push(log: &Log, x: &Sample) {
    log.lock().unwrap().push(Rec {
        at: Instant::now(),
        key: x.key_expr().as_str().to_owned(),
        delete: x.kind() == SampleKind::Delete,
        payload: x.payload().to_bytes().into_owned(),
        stamp: x.timestamp().copied(),
    });
}

/// S's recording subscriber on the owner's alarms, and on its instance
/// tokens, into one log in arrival order.
struct Recorder {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Log,
}

async fn record(s: &zenoh::Session) -> Recorder {
    let log: Log = Arc::default();
    let w = Arc::clone(&log);
    let data = s
        .declare_subscriber(format!("zk2/{SVC}/alarms.v1/**"))
        .callback(move |x: Sample| push(&w, &x))
        .await
        .unwrap();
    let w = Arc::clone(&log);
    let tokens = s
        .liveliness()
        .declare_subscriber(format!("zk2/{SVC}/@zk/instance/*"))
        .callback(move |x: Sample| {
            if x.kind() == SampleKind::Put {
                push(&w, &x);
            }
        })
        .await
        .unwrap();
    Recorder {
        _data: data,
        _tokens: tokens,
        log,
    }
}

/// S's deliveries of the alarm `alert_key`.
fn of(log: &[Rec], alert_key: &str) -> Vec<Rec> {
    log.iter()
        .filter(|r| r.alert_key() == Some(alert_key))
        .cloned()
        .collect()
}

/// S reads its deliveries of one key, in order (§2.6).
fn reads(recs: &[Rec]) -> Vec<Transition> {
    let mut t = Track::default();
    recs.iter()
        .map(|r| {
            if r.delete {
                t.apply(am::Sample::Delete)
            } else {
                match am::decode(&r.payload) {
                    Ok(a) => t.apply(am::Sample::Put(&a)),
                    Err(_) => t.apply(am::Sample::Unreadable),
                }
            }
        })
        .collect()
}

/// Waits until S holds at least `n` deliveries of `alert_key`; returns them.
async fn deliveries(log: &Log, alert_key: &str, n: usize) -> Vec<Rec> {
    eventually(&format!("{n} deliveries of {alert_key}"), || async {
        of(&log.lock().unwrap(), alert_key).len() >= n
    })
    .await;
    of(&snapshot(log), alert_key)
}

/// S received `after[put]` before the new instance token `after[token]`,
/// within [`JITTER`]. The owner puts first (§2.9), but S hears the two on
/// two subscriptions, a data one and a liveliness one, and a token's
/// declaration and a put ride the transport on different lanes: under
/// load the token can overtake a put it followed by a few milliseconds.
/// What S can bound is the inversion, never the owner's own order.
#[track_caller]
fn before_token(after: &[Rec], put: usize, token: usize, what: &str) {
    assert!(
        put < token || after[put].at <= after[token].at + JITTER,
        "{what} before the new instance token, within {JITTER:?}: the put came {:?} after it",
        after[put].at.saturating_duration_since(after[token].at)
    );
}

/// A tool's consumer of `lab/svc`'s `alarms.v1`, on the standard contract.
fn reader(s: &zenoh::Session) -> Consumer {
    Consumer::for_tool(
        s,
        alarms::implementation().shared_contract(),
        &[SVC],
        &Default::default(),
    )
    .unwrap()
}

/// Waits until the owner's session sees S's subscription, so that its
/// first put of an alarm, on a publisher declared at the raise, is not
/// filtered out before it (zenoh's write-side filtering).
async fn seen_by(owner: &zenoh::Session) {
    let p = owner
        .declare_publisher(key_of("0000000000000000"))
        .await
        .unwrap();
    eventually("S's subscription reaches the owner", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
}

/// An owner at `cfg`'s address implementing `imp` from `startup`, not
/// started.
async fn owner(
    s: &zenoh::Session,
    cfg: ServiceConfig,
    imp: Implementation,
    startup: Startup,
) -> (ServiceBuilder, Alarms) {
    let mut b = ServiceBuilder::new(s, cfg);
    let a = b.alarms_with(imp, startup).await.unwrap();
    (b, a)
}

/// The owner's presence as a tool reads it (§2.13): its instance token and
/// descriptor, waiting for the token (a start returns before its
/// declarations reach R1), and the descriptor's `meta.zid`.
async fn presence_of(tool: &zenoh::Session, want: Option<&InstanceId>) -> (Presence, String) {
    let instance = || async {
        presence::tokens(tool, &format!("zk2/{SVC}/@zk/**"), T)
            .await
            .unwrap()
            .into_iter()
            .find_map(|k| match k {
                ZkKey::Instance { instance, .. } if want.is_none_or(|w| *w == instance) => {
                    Some(instance)
                }
                _ => None,
            })
    };
    // The instance the poll found, never a second read: under load a
    // second liveliness read can come back without the token.
    let instance = eventually_some("the owner's instance token", instance).await;
    let d = presence::descriptor(tool, &addr(SVC), &instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let zid = d.meta["zid"].as_str().unwrap().to_owned();
    let p = Presence::Present(
        d.interfaces
            .iter()
            .find(|e| e.iface == "alarms.v1")
            .map_or(Listing::NotListed, |e| Listing::Listed { token: e.token }),
    );
    (p, zid)
}

/// G's S4 GET of the owner's alarms.
async fn get(c: &Consumer) -> StateAnswer {
    c.get_answer(RES, None, T).await.unwrap()
}

/// G's reply for the key of `alert_key`.
fn reply<'a>(a: &'a StateAnswer, alert_key: &str) -> Option<&'a Current> {
    a.current.iter().find(|c| c.key() == key_of(alert_key))
}

/// The latest sample of a reply, as §2.14 reads it.
fn latest_of(c: &Current) -> Latest {
    match c {
        Current::Deleted { .. } => Latest::Deleted,
        Current::Value { sample, .. } => match am::decode(&sample.payload().to_bytes()) {
            Ok(a) if a.state == am::State::Firing => Latest::Firing,
            Ok(_) => Latest::Resolved,
            Err(_) => Latest::Undecodable,
        },
    }
}

/// The deployment's word for G's clock (`freshness.v1` §2.6, ground 1).
const TRUSTED: ClockTrust = ClockTrust::Trusted {
    delta: DEFAULT_DELTA,
};

/// G judges its reply for `alert_key` (§2.14): §5's first question and
/// its last.
fn judge_g(a: &StateAnswer, alert_key: &str, presence: Presence, zid: &str) -> (Judged, Judged) {
    let c = reply(a, alert_key).expect("a reply for the key");
    let obs = [c.freshness(TRUSTED)];
    let r = Reading {
        presence,
        observations: &obs,
        latest: Some(latest_of(c)),
        previous_instance: c.clock().is_some_and(|id| id != zid),
        held_whole: false,
    };
    (am::firing(&r), am::stale_firing(&r))
}

/// S judges its deliveries of `alert_key` (§2.14): its subscription's age
/// of the key, and its latest delivery.
fn judge_s(
    sub: &Subscription,
    log: &[Rec],
    alert_key: &str,
    presence: Presence,
    zid: &str,
) -> (Judged, Judged) {
    let recs = of(log, alert_key);
    let last = recs.last().expect("a delivery of the key");
    let obs: [Observation; 1] = [sub.freshness(&key_of(alert_key))];
    let latest = if last.delete {
        Latest::Deleted
    } else {
        match am::decode(&last.payload) {
            Ok(a) if a.state == am::State::Firing => Latest::Firing,
            Ok(_) => Latest::Resolved,
            Err(_) => Latest::Undecodable,
        }
    };
    let r = Reading {
        presence,
        observations: &obs,
        latest: Some(latest),
        previous_instance: !last.delete && last.stamp_id() != zid,
        held_whole: false,
    };
    (am::firing(&r), am::stale_firing(&r))
}

fn pair(j: Judged) -> (Answer, Reason) {
    (j.answer, j.reason)
}

/// Milliseconds since the Unix epoch, on this host's clock.
fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

/// §1: nothing firing, then A raised, put whole and answered by GET; a
/// duplicate observation puts nothing, and no second key appears.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_raise() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let s = record(&rs).await;
    let g = reader(&rs);

    // 1. Nothing firing: G's GET ends at its final reply with no value.
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default(),
    )
    .await;
    assert_eq!(a.max_alarms(), 256);
    assert_eq!(a.adoption().source, Source::Nothing);
    let svc = b.start().await.unwrap();
    let (presence, zid) = presence_of(&rs, None).await;
    assert_eq!(zid, owner_zid);
    assert_eq!(presence, Presence::Present(Listing::Listed { token: true }));
    let first = get(&g).await;
    assert!(first.complete, "at its final reply (core S2)");
    assert!(first.current.is_empty(), "no value: {:?}", first.current);
    let set = am::owner_set(presence, Some(Seed::Complete { members: 0 }), None);
    assert_eq!(
        (set.verdict, set.reason),
        (SetVerdict::Unobservable, Reason::NoMember),
        "an empty reply set alone is no verdict (core O5)"
    );

    // 2. A raised.
    seen_by(&os).await;
    let before = now_ms();
    let done = a.raise(alarm_a(Severity::Warning)).await.unwrap();
    assert_eq!((done.alert_key.as_str(), done.act), (A_KEY, Act::Raised));
    let recs = deliveries(&s.log, A_KEY, 1).await;
    let put = &recs[0];
    assert!(!put.delete);
    assert_eq!(put.stamp_id(), owner_zid, "stamped by the owner (core S1)");
    let doc = put.doc();
    assert_eq!(doc, done.document);
    assert_eq!(
        (doc.state, doc.severity, doc.observed_at_ms),
        (am::State::Firing, Severity::Warning, None)
    );
    assert!(
        doc.timestamp >= before.saturating_sub(1000) && doc.timestamp <= now_ms() + 1000,
        "within 1 s of the step: {} against {before}",
        doc.timestamp
    );
    assert_eq!(reads(&recs), [Transition::Raised]);
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).expect("G's reply holds A");
    let Current::Value { sample, .. } = c else {
        panic!("a value: {c:?}")
    };
    assert_eq!(am::decode(&sample.payload().to_bytes()).unwrap(), doc);
    assert_eq!(c.timestamp(), put.stamp, "under the same stamp (core S2)");
    let (firing, stale) = judge_g(&answer, A_KEY, presence, &zid);
    assert_eq!(pair(firing), (Answer::Yes, Reason::Firing));
    assert_eq!(pair(stale), (Answer::No, Reason::Fresh));

    // 3. The same condition again, its labels given in another order.
    let again = Condition::new("link_down", Severity::Warning, "link to r2 down on eth0")
        .label("port", "eth0")
        .label("peer", "r2");
    let done = a.raise(again).await.unwrap();
    assert_eq!((done.alert_key.as_str(), done.act), (A_KEY, Act::Unchanged));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let log = snapshot(&s.log);
    let recs = of(&log, A_KEY);
    assert!(
        reads(&recs)[1..]
            .iter()
            .all(|t| *t == Transition::Confirmed),
        "nothing, or the same document: {recs:?}"
    );
    let keys: std::collections::BTreeSet<&str> = log.iter().filter_map(Rec::alert_key).collect();
    assert_eq!(
        keys.into_iter().collect::<Vec<_>>(),
        [A_KEY],
        "no second key"
    );
    assert_eq!(a.held().await.len(), 1);
    svc.close().await.unwrap();
}

/// §2 on `imp`: an escalation, a refresh, then nothing for `quiet`, with no
/// two deliveries more than `bound` apart.
async fn escalate_refresh_confirm(imp: Implementation, quiet: Duration, bound: Duration) {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let s = record(&rs).await;
    let (b, a) = owner(&os, config(SVC), imp, Startup::default()).await;
    let svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    seen_by(&os).await;
    let t0 = a
        .raise(alarm_a(Severity::Warning))
        .await
        .unwrap()
        .document
        .timestamp;
    deliveries(&s.log, A_KEY, 1).await;

    // 1. Escalated to critical.
    let done = a.raise(alarm_a(Severity::Critical)).await.unwrap();
    assert_eq!(done.act, Act::Escalated);
    let recs = deliveries(&s.log, A_KEY, 2).await;
    let escalated = recs[1].doc();
    assert_eq!(escalated.severity, Severity::Critical);
    assert!(escalated.timestamp > t0, "a new timestamp, later than t0");
    assert_eq!(escalated.observed_at_ms, None);
    assert_eq!(reads(&recs), [Transition::Raised, Transition::Escalated]);

    // 2. Refreshed: the summary changes, and a host-scoped label comes.
    let refresh = Condition::new(
        "link_down",
        Severity::Critical,
        "link to r2 down on eth0, 3 retries",
    )
    .label("peer", "r2")
    .label("port", "eth0")
    .label("host.id", "h-3fa9c2d41b7e");
    assert_eq!(refresh.alert_key(), A_KEY, "host.id is host-scoped (§2.2)");
    let done = a.raise(refresh).await.unwrap();
    assert_eq!((done.alert_key.as_str(), done.act), (A_KEY, Act::Refreshed));
    let recs = deliveries(&s.log, A_KEY, 3).await;
    let refreshed = recs[2].doc();
    assert_eq!(
        refreshed.timestamp, escalated.timestamp,
        "the timestamp kept"
    );
    assert!(
        refreshed
            .observed_at_ms
            .is_some_and(|o| o >= refreshed.timestamp)
    );
    assert_eq!(reads(&recs)[2], Transition::Refreshed);
    let refreshed_at = recs[2].at;

    // 3. Nothing for a while: confirmed, unchanged, observed_at_ms too.
    tokio::time::sleep_until((refreshed_at + quiet).into()).await;
    let recs = of(&snapshot(&s.log), A_KEY);
    let confirmations = &recs[2..];
    assert!(
        confirmations.len() >= 2,
        "the refresh and a confirmation: {recs:#?}"
    );
    for w in confirmations.windows(2) {
        assert!(
            w[1].at - w[0].at <= bound + JITTER,
            "no two deliveries more than {bound:?} apart: {:?}",
            w[1].at - w[0].at
        );
        assert_eq!(
            w[1].payload, w[0].payload,
            "unchanged, observed_at_ms included"
        );
        assert!(
            w[1].stamp.unwrap() > w[0].stamp.unwrap(),
            "under a later stamp"
        );
        assert_eq!(w[1].stamp_id(), owner_zid, "of the same id");
    }
    assert!(
        refreshed_at + quiet - recs.last().unwrap().at <= bound + JITTER,
        "and none more than {bound:?} before the window ends"
    );
    assert!(
        reads(&recs)[3..]
            .iter()
            .all(|t| *t == Transition::Confirmed),
        "each read confirmed"
    );
    svc.close().await.unwrap();
}

/// §2, fast: the contract's ttl 4 s, so confirmed every 1.9 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_escalate_refresh_and_confirm() {
    escalate_refresh_confirm(fast(4), Duration::from_secs(7), Duration::from_secs(2)).await;
}

/// §2 at the standard contract: 455 s of nothing, confirmed every 450 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §2 waits 455 s at the contract's 900 s horizon"]
async fn s2_escalate_refresh_and_confirm_at_900_s() {
    escalate_refresh_confirm(
        alarms::implementation(),
        Duration::from_secs(455),
        Duration::from_secs(450),
    )
    .await;
}

/// §3: a resolve is a resolved put, then a delete within 1 s; G reads the
/// delete as a `reply_del` within the window, and nothing after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_resolve_then_delete() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let g = reader(&rs);
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default(),
    )
    .await;
    let svc = b.start().await.unwrap();
    let (presence, zid) = presence_of(&rs, None).await;
    seen_by(&os).await;
    let raised = a.raise(alarm_a(Severity::Warning)).await.unwrap().document;
    deliveries(&s.log, A_KEY, 1).await;

    // 1. Resolved.
    let done = a.resolve(A_KEY).await.unwrap().expect("A held");
    assert_eq!(done.act, Act::Resolved);
    let recs = deliveries(&s.log, A_KEY, 3).await;
    let resolved = recs[1].doc();
    assert!(!recs[1].delete && recs[2].delete);
    assert_eq!(resolved.state, am::State::Resolved);
    assert!(resolved.timestamp > raised.timestamp, "a new timestamp");
    assert_eq!(resolved.observed_at_ms, None);
    assert!(
        recs[2].at - recs[1].at <= Duration::from_secs(1),
        "the delete within 1 s: {:?}",
        recs[2].at - recs[1].at
    );
    assert_eq!(
        reads(&recs),
        [
            Transition::Raised,
            Transition::Resolved,
            Transition::Retired
        ]
    );
    assert!(a.held().await.is_empty());
    assert!(a.resolve(A_KEY).await.unwrap().is_none(), "nothing held");

    // 2. Within 1 s of the delete: a reply_del, with the delete's stamp.
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).expect("a reply for A");
    assert!(matches!(c, Current::Deleted { .. }), "{c:?}");
    assert_eq!(c.timestamp(), recs[2].stamp, "the delete's stamp (core S3)");
    let (firing, _) = judge_g(&answer, A_KEY, presence, &zid);
    assert_eq!(pair(firing), (Answer::No, Reason::Deleted));

    // 3. 65 s after it, with the 60 s window: nothing for the key.
    tokio::time::sleep_until((recs[2].at + Duration::from_secs(65)).into()).await;
    let answer = get(&g).await;
    assert!(answer.complete);
    assert!(reply(&answer, A_KEY).is_none(), "{:?}", answer.current);
    svc.close().await.unwrap();
}

/// §4: a re-raise after a resolve, on the same key, later than the
/// resolve.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_re_raise() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default(),
    )
    .await;
    let svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    seen_by(&os).await;
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    let resolved = a.resolve(A_KEY).await.unwrap().unwrap().document;
    deliveries(&s.log, A_KEY, 3).await;

    let done = a.raise(alarm_a(Severity::Warning)).await.unwrap();
    assert_eq!(done.act, Act::Raised);
    let recs = deliveries(&s.log, A_KEY, 4).await;
    let again = recs[3].doc();
    assert_eq!(again.state, am::State::Firing);
    assert!(
        again.timestamp > resolved.timestamp,
        "later than the resolve's"
    );
    assert_eq!(
        reads(&recs),
        [
            Transition::Raised,
            Transition::Resolved,
            Transition::Retired,
            Transition::Raised
        ]
    );
    svc.close().await.unwrap();
}

/// §5: a restart adopts from its record. The kept alarms are put again by
/// the new instance before its instance token and before any GET answers
/// them; a resolved document and a phantom are deleted; an alarm whose
/// condition cleared while the owner was down is resolved at its first
/// evaluation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_restart_mid_incident() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let g = reader(&rs);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alarms.json");
    let startup = || Startup::default().with_record(Arc::new(FileRecord::new(&path)));

    // Setup: A firing at critical since t0, and B firing.
    let os = client(&ep).await;
    let (b, a) = owner(&os, config(SVC), alarms::implementation(), startup()).await;
    let svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    seen_by(&os).await;
    let a_doc = a.raise(alarm_a(Severity::Critical)).await.unwrap().document;
    let b_key = a.raise(alarm_b()).await.unwrap().alert_key;
    deliveries(&s.log, A_KEY, 1).await;
    deliveries(&s.log, &b_key, 1).await;

    // 1. The process ends without closing; S listens 3 s.
    drop(a);
    drop(svc);
    os.close().await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let before = snapshot(&s.log);
    assert!(
        before
            .iter()
            .filter(|r| r.alert_key().is_some())
            .all(|r| !r.delete && r.doc().state == am::State::Firing),
        "no delete and no resolved put (§2.9)"
    );

    // The record, and the two documents a test writes into it.
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut held: Vec<&str> = file["alarms"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    held.sort_unstable();
    let mut want = vec![A_KEY, b_key.as_str()];
    want.sort_unstable();
    assert_eq!(held, want, "the record holds A and B");
    let r_doc = serde_json::json!({
        "timestamp": a_doc.timestamp - 60_000, "rule": "fan_failed", "severity": "warning",
        "state": "resolved", "summary": "fan 2 stopped"
    });
    let r_key = am::derive("fan_failed", []).key;
    let p_key = "0000000000000001";
    file["alarms"][&r_key] = r_doc;
    file["alarms"][p_key] = serde_json::to_value(&a_doc).unwrap();
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

    // 2. B's condition clears while the owner is down. 3. It restarts, in
    // a new process, so a new session, and evaluates its rules once.
    let os = client(&ep).await;
    let new_zid = os.zid().to_string();
    let (b, a) = owner(&os, config(SVC), alarms::implementation(), startup()).await;
    let adoption = a.adoption();
    assert_eq!(adoption.source, Source::Record);
    let mut adopted = adoption.adopted.clone();
    adopted.sort();
    let mut want = vec![A_KEY.to_owned(), b_key.clone()];
    want.sort();
    assert_eq!(adopted, want);
    let mut retired = adoption.retired.clone();
    retired.sort_by(|x, y| x.0.cmp(&y.0));
    let mut want_retired = vec![
        (p_key.to_owned(), Retired::Phantom),
        (r_key.clone(), Retired::Resolved),
    ];
    want_retired.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(retired, want_retired);
    assert!(adoption.resolved.is_empty() && adoption.over_bound.is_empty());
    let svc = b.start().await.unwrap();
    let instance = svc.instance().clone();
    // The first evaluation: A still holds, B's condition is clear.
    assert_eq!(
        a.raise(alarm_a(Severity::Critical)).await.unwrap().act,
        Act::Unchanged
    );
    assert_eq!(a.resolve(&b_key).await.unwrap().unwrap().act, Act::Resolved);

    // What S received: before the new instance token, A unchanged under
    // the new instance's stamp id, the deletes of R's and P's keys, and
    // B's put; then B's resolved put and delete.
    eventually("the new instance token reaches S", || async {
        snapshot(&s.log)[before.len()..]
            .iter()
            .any(Rec::is_instance)
    })
    .await;
    deliveries(&s.log, &b_key, 4).await;
    let after = snapshot(&s.log)[before.len()..].to_vec();
    let token_at = after.iter().position(Rec::is_instance).unwrap();
    let a_put = after
        .iter()
        .position(|r| r.alert_key() == Some(A_KEY))
        .expect("A put again");
    assert_eq!(after[a_put].doc(), a_doc, "unchanged, t0 kept");
    assert_eq!(
        after[a_put].stamp_id(),
        new_zid,
        "under the new instance's stamp id"
    );
    for k in [&r_key, &p_key.to_owned(), &b_key] {
        let at = after
            .iter()
            .position(|r| r.alert_key() == Some(k.as_str()))
            .unwrap_or_else(|| panic!("{k} delivered"));
        before_token(&after, at, token_at, &format!("{k} (a SHOULD)"));
    }
    before_token(&after, a_put, token_at, "A (a SHOULD)");
    let all = snapshot(&s.log);
    assert_eq!(
        reads(&of(&all, A_KEY)),
        [Transition::Raised, Transition::Confirmed]
    );
    assert_eq!(reads(&of(&all, &r_key)), [Transition::Retired]);
    assert_eq!(reads(&of(&all, p_key)), [Transition::Retired]);
    assert_eq!(
        reads(&of(&all, &b_key)),
        [
            Transition::Raised,
            Transition::Confirmed,
            Transition::Resolved,
            Transition::Retired
        ]
    );

    // 4. G: A stamped by the new instance, whose meta.zid is the stamp's id.
    let (presence, zid) = presence_of(&rs, Some(&instance)).await;
    assert_eq!(zid, new_zid);
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).expect("A answered");
    assert_eq!(c.clock().as_deref(), Some(new_zid.as_str()));
    let (firing, _) = judge_g(&answer, A_KEY, presence, &zid);
    assert_eq!(
        pair(firing),
        (Answer::Yes, Reason::Firing),
        "never previous_instance"
    );
    let record = FileRecord::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let kept: Vec<&str> = record.iter().map(|k| k.alert_key.as_str()).collect();
    assert_eq!(kept, [A_KEY], "the record holds what the instance holds");
    svc.close().await.unwrap();
}

/// §6 on `imp`: the owner stops confirming A and keeps its tokens; S
/// receives nothing more of it for `quiet`, and G's GET still answers it,
/// under the last stamp. Returns S's subscription and log, G's consumer,
/// the owner's presence and zid, and what keeps the bus up.
async fn stops_confirming(
    imp: Implementation,
    quiet: Duration,
) -> (
    impl Sized,
    Subscription,
    Vec<Rec>,
    Consumer,
    Presence,
    String,
    Alarms,
) {
    let (r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let g = reader(&rs);
    let sub = g.subscribe(RES, |_| {}).await.unwrap();
    let (b, a) = owner(&os, config(SVC), imp, Startup::default()).await;
    let svc = b.start().await.unwrap();
    let (presence, zid) = presence_of(&rs, None).await;
    seen_by(&os).await;
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    deliveries(&s.log, A_KEY, 1).await;
    assert_eq!(a.is_confirming(A_KEY).await, Some(true));

    // 1. Its evaluation of A's rule stops, and so do its re-puts of A.
    assert!(a.set_confirming(A_KEY, false).await);
    assert_eq!(a.is_confirming(A_KEY).await, Some(false));
    let before = of(&snapshot(&s.log), A_KEY);
    let last = before.last().unwrap().clone();
    tokio::time::sleep_until((last.at + quiet).into()).await;
    let log = snapshot(&s.log);
    assert_eq!(of(&log, A_KEY).len(), before.len(), "nothing more of A");
    let tokens = presence::tokens(&rs, &format!("zk2/{SVC}/@zk/instance/*"), T)
        .await
        .unwrap();
    assert_eq!(tokens.len(), 1, "its instance token held");
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).expect("its GETs still answer A");
    assert_eq!(
        c.timestamp(),
        last.stamp,
        "under the last stamp, not a newer one"
    );
    ((r1, os, rs, s, svc), sub, log, g, presence, zid, a)
}

/// §6, fast: the contract's ttl 4 s; nothing more of A for six of its
/// confirmation periods, and its confirmation back on re-puts it at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_hung_owner_goes_stale() {
    let (bus, _sub, log, g, _presence, _zid, a) =
        stops_confirming(fast(4), Duration::from_secs(12)).await;
    // Its evaluation back: confirmed at once, its interval run out.
    assert!(a.set_confirming(A_KEY, true).await);
    let start = Instant::now();
    eventually("A confirmed again", || async {
        get(&g).await.current.iter().any(|c| {
            c.key() == key_of(A_KEY) && c.timestamp() > of(&log, A_KEY).last().unwrap().stamp
        })
    })
    .await;
    assert!(
        start.elapsed() < Duration::from_secs(2) + JITTER,
        "at once: {:?}",
        start.elapsed()
    );
    drop(bus);
}

/// §6 at the standard contract: 905 s after S's last delivery of A, S and
/// G judge it stale, never firing now and never resolved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §6 waits 905 s at the contract's 900 s horizon"]
async fn s6_a_hung_owner_goes_stale_at_900_s() {
    let (bus, sub, log, g, presence, zid, _a) =
        stops_confirming(alarms::implementation(), Duration::from_secs(905)).await;
    // 2. S judges.
    let (firing, stale) = judge_s(&sub, &log, A_KEY, presence, &zid);
    assert_eq!(pair(firing), (Answer::Unobservable, Reason::Stale));
    assert_eq!(pair(stale), (Answer::Yes, Reason::Stale));
    // G GETs and judges: A firing, under a stamp more than 900.5 s old.
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).unwrap();
    let age = SystemTime::now()
        .duration_since(c.timestamp().unwrap().get_time().to_system_time())
        .unwrap();
    assert!(age > Duration::from_millis(900_500), "{age:?}");
    assert_eq!(latest_of(c), Latest::Firing);
    let (firing, stale) = judge_g(&answer, A_KEY, presence, &zid);
    assert_eq!(pair(firing), (Answer::Unobservable, Reason::Stale));
    assert_eq!(pair(stale), (Answer::Yes, Reason::Stale));
    drop(bus);
}

/// §7: at the descriptor's bound of 2, a third raise is refused and its
/// writer told; nothing held is resolved or deleted to make room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_at_the_bound() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default().with_cardinality(2),
    )
    .await;
    assert_eq!(a.max_alarms(), 2);
    let svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    let d = presence::descriptor(&rs, &addr(SVC), svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "alarms.v1")
        .unwrap();
    assert_eq!(
        e.cardinality.get(alarms::ALARMS),
        Some(&2),
        "{:?}",
        e.cardinality
    );
    seen_by(&os).await;

    // 1. A and B.
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    let b_key = a.raise(alarm_b()).await.unwrap().alert_key;
    deliveries(&s.log, A_KEY, 1).await;
    deliveries(&s.log, &b_key, 1).await;

    // 2. C refused, its writer told.
    let c = Condition::new("link_down", Severity::Warning, "link to r3 down on eth1")
        .label("peer", "r3")
        .label("port", "eth1");
    let c_key = c.alert_key();
    let refused = a.raise(c.clone()).await.unwrap_err();
    let zenkey::Error::Budget(r) = &refused else {
        panic!("a budget refusal: {refused}")
    };
    assert_eq!(
        r.breach,
        zenkey::budget::Breach::Bound {
            bound: 2,
            source: zenkey::budget::Source::Descriptor,
            live: 2
        }
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    let log = snapshot(&s.log);
    assert!(of(&log, &c_key).is_empty(), "no put of C");
    assert_eq!(of(&log, A_KEY).len(), 1, "no resolve or delete of A");
    assert_eq!(of(&log, &b_key).len(), 1, "nor of B");
    assert_eq!(a.held().await.len(), 2);

    // 3. B resolved, then C raised.
    a.resolve(&b_key).await.unwrap().unwrap();
    assert_eq!(a.raise(c).await.unwrap().act, Act::Raised);
    let recs = deliveries(&s.log, &c_key, 1).await;
    assert_eq!(recs[0].doc().state, am::State::Firing);
    let b_recs = deliveries(&s.log, &b_key, 3).await;
    assert_eq!(
        reads(&b_recs),
        [
            Transition::Raised,
            Transition::Resolved,
            Transition::Retired
        ]
    );
    let log = snapshot(&s.log);
    let b_deleted = log
        .iter()
        .position(|r| r.alert_key() == Some(b_key.as_str()) && r.delete)
        .unwrap();
    let c_put = log
        .iter()
        .position(|r| r.alert_key() == Some(c_key.as_str()))
        .unwrap();
    assert!(b_deleted < c_put, "B resolved and deleted, then C");
    svc.close().await.unwrap();
}

/// A re-mint in one session (§2.9): every alarm is put again before the
/// new instance's token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_re_mint_puts_every_alarm_again() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs).await;
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default(),
    )
    .await;
    let mut svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    seen_by(&os).await;
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    deliveries(&s.log, A_KEY, 1).await;
    let n = snapshot(&s.log).len();
    svc.new_epoch().await.unwrap();
    eventually("the new instance token", || async {
        snapshot(&s.log)[n..].iter().any(Rec::is_instance)
    })
    .await;
    let after = snapshot(&s.log)[n..].to_vec();
    let token = after.iter().position(Rec::is_instance).unwrap();
    let put = after
        .iter()
        .position(|r| r.alert_key() == Some(A_KEY))
        .expect("A put again");
    before_token(&after, put, token, "A");
    assert_eq!(
        reads(&of(&snapshot(&s.log), A_KEY)),
        [Transition::Raised, Transition::Confirmed]
    );
    assert!(svc.alarms().is_some());
    svc.close().await.unwrap();
}

/// §2.9's second source: with no record, a restart adopts from an explicit
/// read of an archive on its own side, last-known, never from a GET of its
/// own keys; and puts what it adopts again under its new session's stamps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_adopts_from_an_archive_on_its_own_side() {
    use zenkey::archive::{Archive, ArchiveConfig, Recorded};

    let (_r1, ep) = router(None).await;
    let xs = client(&ep).await;
    let rs = client(&ep).await;
    let g = reader(&rs);
    let arch = Archive::start(
        &xs,
        ArchiveConfig {
            service: config("lab/archive"),
            records: vec![Recorded {
                owner: addr(SVC),
                selector: format!("zk2/{SVC}/alarms.v1/state/alarms/*"),
                implementation: alarms::implementation(),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
        },
    )
    .await
    .unwrap();

    let os = client(&ep).await;
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default(),
    )
    .await;
    let svc = b.start().await.unwrap();
    presence_of(&rs, None).await;
    let raised = a.raise(alarm_a(Severity::Critical)).await.unwrap().document;
    eventually("the archive records A", || async {
        arch.confirmed(&key_of(A_KEY)).is_some()
    })
    .await;
    svc.close().await.unwrap();
    drop(a);
    os.close().await.unwrap();

    let os = client(&ep).await;
    let new_zid = os.zid().to_string();
    let (b, a) = owner(
        &os,
        config(SVC),
        alarms::implementation(),
        Startup::default().with_archive(addr("lab/archive"), T),
    )
    .await;
    let adoption = a.adoption();
    assert_eq!(adoption.source, Source::Archive);
    assert_eq!(adoption.adopted, [A_KEY]);
    assert_eq!(
        a.get(A_KEY).await,
        Some(raised.clone()),
        "its timestamp kept"
    );
    let svc = b.start().await.unwrap();
    let (presence, zid) = presence_of(&rs, Some(svc.instance())).await;
    assert_eq!(zid, new_zid);
    let answer = get(&g).await;
    let c = reply(&answer, A_KEY).expect("A answered");
    assert_eq!(
        c.clock().as_deref(),
        Some(new_zid.as_str()),
        "under the new stamps"
    );
    let (firing, _) = judge_g(&answer, A_KEY, presence, &zid);
    assert_eq!(pair(firing), (Answer::Yes, Reason::Firing));
    svc.close().await.unwrap();
}
