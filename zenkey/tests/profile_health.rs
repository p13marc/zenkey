//! `spec/profiles/health/scenarios.md` §1–§5 (`health.v1`, #721), one test
//! per section, named after it. §6–§8 are a tool's, and run in
//! `zenctl/tests/live_zk2.rs`.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default, and without `timestamping.drop_future_timestamp`
//! except where a step measures it. Owners implement `health.v1` through
//! [`ServiceBuilder::health`], on the standard contract, as clients of R1.
//! S records every delivery under the owner's `health.v1/**` (arrival on
//! this host's monotonic clock, key, kind, payload, stamp), with a
//! consumer's subscription on the status for its freshness and its clock
//! measurement; G GETs the owner's `health.v1/state/**` as core S4 says
//! (target `All`, consolidation `Latest`). Every verdict is
//! `zenkey_model::health::judge`'s.
//!
//! The horizon is the contract's 60 s: §2 and §4 wait it out (about 70 s
//! and 100 s), §1 waits one confirmation interval (35 s).

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::{T, addr, client, config, eventually, imp, router, router_with};
use zenkey::consumer::{Consumer, Subscription};
use zenkey::health::{self, Health, Level, Read, v1};
use zenkey::model::freshness::{ClockTrust, DEFAULT_DELTA, Observation, StampAge};
use zenkey::model::grammar::{IfaceId, ZkKey, parse};
use zenkey::model::health::{self as hm, Judged, Listing, Presence, Reading, Reason, Verdict};
use zenkey::model::template::Bindings;
use zenkey::prost::Message as _;
use zenkey::state::Current;
use zenkey::{ServiceBuilder, ServiceConfig, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

/// The owner of `scenarios.md`'s conventions.
const SVC: &str = "lab/svc";

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
    fn is_status(&self) -> bool {
        self.key.ends_with("/health.v1/state/status")
    }

    fn check(&self) -> Option<&str> {
        self.key.split_once("/health.v1/state/checks/").map(|x| x.1)
    }

    fn is_fault(&self) -> bool {
        self.key.ends_with("/health.v1/stream/faults")
    }

    fn status(&self) -> v1::Status {
        v1::Status::decode(self.payload.as_slice()).expect("a health.v1.Status")
    }

    fn check_value(&self) -> v1::Check {
        v1::Check::decode(self.payload.as_slice()).expect("a health.v1.Check")
    }

    fn fault(&self) -> v1::Fault {
        v1::Fault::decode(self.payload.as_slice()).expect("a health.v1.Fault")
    }

    fn stamp_id(&self) -> String {
        self.stamp.expect("stamped").get_id().to_string()
    }
}

type Log = Arc<Mutex<Vec<Rec>>>;

fn snapshot(log: &Log) -> Vec<Rec> {
    log.lock().unwrap().clone()
}

/// S's recording subscriber on `selector`.
async fn record(s: &zenoh::Session, selector: &str) -> (zenoh::pubsub::Subscriber<()>, Log) {
    let log: Log = Arc::default();
    let w = Arc::clone(&log);
    let sub = s
        .declare_subscriber(selector.to_owned())
        .callback(move |x: Sample| {
            w.lock().unwrap().push(Rec {
                at: Instant::now(),
                key: x.key_expr().as_str().to_owned(),
                delete: x.kind() == SampleKind::Delete,
                payload: x.payload().to_bytes().into_owned(),
                stamp: x.timestamp().copied(),
            });
        })
        .await
        .unwrap();
    (sub, log)
}

/// Waits until the log holds at least `n` deliveries; returns them.
async fn at_least(log: &Log, n: usize) -> Vec<Rec> {
    eventually(&format!("{n} deliveries"), || async {
        log.lock().unwrap().len() >= n
    })
    .await;
    snapshot(log)
}

/// A tool's consumer of `address`'s `health.v1`, on the standard contract.
fn reader(s: &zenoh::Session, address: &str) -> Consumer {
    Consumer::for_tool(
        s,
        health::implementation().shared_contract(),
        &[address],
        &Default::default(),
    )
    .unwrap()
}

/// S's subscription on the status: its freshness and its clock (`freshness.v1`
/// §2.5, §2.6).
async fn watch_status(c: &Consumer) -> Subscription {
    c.subscribe(health::STATUS, |_| {}).await.unwrap()
}

/// Waits until `w`'s publisher sees a subscriber.
async fn matched(w: &zenkey::writer::Writer) {
    eventually("the subscriber is matched", || async {
        w.matching().await.unwrap()
    })
    .await;
}

/// An owner at `cfg`'s address implementing `health.v1`, not started.
async fn owner(s: &zenoh::Session, cfg: ServiceConfig) -> (ServiceBuilder, Health) {
    let mut b = ServiceBuilder::new(s, cfg);
    let h = b.health().await.unwrap();
    (b, h)
}

/// G's GET of `address`'s `health.v1/state/**` (core S2, S4).
async fn get_state(s: &zenoh::Session, address: &str) -> Vec<Current> {
    let rx = s
        .get(format!("zk2/{address}/health.v1/state/**"))
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        let Ok(sample) = r.into_result() else {
            continue;
        };
        let key = sample.key_expr().as_str().to_owned();
        out.push(match sample.kind() {
            SampleKind::Delete => Current::Deleted {
                key,
                timestamp: sample.timestamp().copied(),
            },
            SampleKind::Put => Current::Value {
                key,
                sample: Box::new(sample),
            },
        });
    }
    out
}

/// What G's reply holds: the status, and each check (`None`: a
/// `reply_del`).
struct Got {
    status: Option<(Current, v1::Status)>,
    checks: BTreeMap<String, Option<v1::Check>>,
}

fn split(replies: Vec<Current>) -> Got {
    let mut got = Got {
        status: None,
        checks: BTreeMap::new(),
    };
    for c in replies {
        let key = c.key().to_owned();
        let bytes = match &c {
            Current::Value { sample, .. } => Some(sample.payload().to_bytes().into_owned()),
            Current::Deleted { .. } => None,
        };
        if key.ends_with("/state/status") {
            let s =
                v1::Status::decode(bytes.expect("the status is never deleted").as_slice()).unwrap();
            got.status = Some((c, s));
        } else if let Some((_, name)) = key.split_once("/state/checks/") {
            let v = bytes.map(|b| v1::Check::decode(b.as_slice()).unwrap());
            got.checks.insert(name.to_owned(), v);
        }
    }
    got
}

impl Got {
    /// The levels of the checks the owner answered with a value (§2.4).
    fn check_levels(&self) -> Vec<Read> {
        self.checks
            .values()
            .flatten()
            .map(v1::Check::read)
            .collect()
    }

    /// G judges this reply (v1.md §2.11), its clock as `trust`.
    fn judge(&self, presence: Presence, trust: ClockTrust) -> Judged {
        let (current, status) = self.status.as_ref().expect("a status in the reply");
        let obs = [current.freshness(trust)];
        let checks = self.check_levels();
        hm::judge(&Reading {
            presence,
            status: &obs,
            level: Some(status.read()),
            checks: Some(&checks),
        })
    }
}

/// S judges from its deliveries: the status's freshness on its
/// subscription, the latest status's level, and the latest of each check.
fn judge_s(sub: &Subscription, log: &[Rec], presence: Presence) -> Judged {
    let status: Vec<&Rec> = log.iter().filter(|r| r.is_status()).collect();
    let key = &status.last().expect("a status delivery").key;
    let obs: [Observation; 1] = [sub.freshness(key)];
    let mut checks = BTreeMap::new();
    for r in log {
        if let Some(name) = r.check() {
            checks.insert(name.to_owned(), (!r.delete).then(|| r.check_value().read()));
        }
    }
    let checks: Vec<Read> = checks.into_values().flatten().collect();
    hm::judge(&Reading {
        presence,
        status: &obs,
        level: status.last().map(|r| r.status().read()),
        checks: Some(&checks),
    })
}

fn verdict(j: Judged) -> (Verdict, Reason, Option<Level>) {
    (j.verdict, j.reason, j.level)
}

/// A present owner whose descriptor lists `health.v1`, as a tool reads it
/// (§2.7): through its instance token and descriptor.
async fn presence_of(tool: &zenoh::Session, address: &str) -> Presence {
    let tokens = presence::tokens(tool, &format!("zk2/{address}/@zk/**"), T)
        .await
        .unwrap();
    let Some(instance) = tokens.iter().find_map(|k| match k {
        ZkKey::Instance { instance, .. } => Some(instance.clone()),
        _ => None,
    }) else {
        return Presence::Absent;
    };
    let d = presence::descriptor(tool, &addr(address), &instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    Presence::Present(
        d.interfaces
            .iter()
            .find(|e| e.iface == "health.v1")
            .map_or(Listing::NotListed, |e| Listing::Listed { token: e.token }),
    )
}

/// The tokens `address` holds.
async fn tokens_of(tool: &zenoh::Session, address: &str) -> Vec<ZkKey> {
    presence::tokens(tool, &format!("zk2/{address}/@zk/**"), T)
        .await
        .unwrap()
}

fn is_health_token(k: &ZkKey) -> bool {
    matches!(k, ZkKey::Alive { iface, .. } if *iface == health::iface())
}

/// v1.md §5's last question, "is this service's clock ahead?", from S's
/// log: yes on a `clock_ahead` fault with no confirmation of the status
/// heard since; no when the status was confirmed after the last one, or
/// with none heard.
fn clock_ahead(log: &[Rec]) -> bool {
    let last_fault = log
        .iter()
        .rposition(|r| r.is_fault() && r.fault().code == health::CLOCK_AHEAD);
    let last_status = log.iter().rposition(|r| r.is_status() && !r.delete);
    match (last_fault, last_status) {
        (Some(f), Some(s)) => f > s,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// §1: the status answers a GET made at the instance token and at the
/// interface token, or at the instance token for a tokenless `health.v1`;
/// it is confirmed every 30 s, unchanged, and never deleted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_bring_up() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let qs = client(&ep).await;
    let ts = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let (_s, log) = record(&ts, "zk2/lab/svc/health.v1/**").await;
    let tokens = ts
        .liveliness()
        .declare_subscriber("zk2/lab/*/@zk/**")
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let next_token = || async {
        tokio::time::timeout(common::SETTLE, tokens.recv_async())
            .await
            .expect("a token")
            .unwrap()
    };

    // 1. The owner starts with nothing set: UNSPECIFIED, "starting".
    let (b, h) = owner(&os, config(SVC)).await;
    matched(h.status_writer().writer()).await;
    let svc = b.start().await.unwrap();
    let mut at = Vec::new();
    while at.len() < 2 {
        let s = next_token().await;
        if s.kind() != SampleKind::Put {
            continue;
        }
        let k = parse(s.key_expr().as_str()).unwrap();
        match &k {
            ZkKey::Instance { addr: a, .. } if *a == addr(SVC) => {
                at.push(("instance", get_state(&ts, SVC).await));
            }
            ZkKey::Alive { addr: a, .. } if *a == addr(SVC) && is_health_token(&k) => {
                at.push(("alive/health.v1", get_state(&ts, SVC).await));
            }
            _ => {}
        }
    }
    let mut first_since = 0;
    for (when, replies) in at {
        let got = split(replies);
        let (current, status) = got.status.expect(when);
        assert_eq!(
            (status.read(), status.reason.as_str()),
            (Read::Unspecified, health::STARTING),
            "at the {when} token"
        );
        assert_eq!(
            current.clock().as_deref(),
            Some(owner_zid.as_str()),
            "stamped by the owner's session (S1)"
        );
        assert!(got.checks.is_empty());
        first_since = status.since_ns;
    }
    let started = h.status().unwrap();
    assert_eq!(
        (started.level, started.declared, started.since_ns),
        (None, None, first_since)
    );

    // 2. lab/quiet: health.v1 in its tokenless set, OK before it starts.
    let quiet_cfg = config("lab/quiet").tokenless(health::iface());
    let (qb, qh) = owner(&qs, quiet_cfg).await;
    qh.set_status(Level::Ok, "serving").await.unwrap();
    let quiet = qb.start().await.unwrap();
    let instance = loop {
        let s = next_token().await;
        match parse(s.key_expr().as_str()).unwrap() {
            ZkKey::Instance { addr: a, instance } if a == addr("lab/quiet") => break instance,
            k => assert!(
                !(is_health_token(&k)
                    && matches!(&k, ZkKey::Alive { addr: a, .. } if *a == addr("lab/quiet"))),
                "no health.v1 token: {k:?}"
            ),
        }
    };
    let d = presence::descriptor(&ts, &addr("lab/quiet"), &instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    let got = split(get_state(&ts, "lab/quiet").await);
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "health.v1")
        .expect("listed");
    assert!(!e.token, "\"token\": false");
    assert_eq!(got.status.unwrap().1.read(), Read::Level(Level::Ok));
    let quiet_tokens = tokens_of(&ts, "lab/quiet").await;
    assert_eq!(
        quiet_tokens.len(),
        1,
        "the instance token only: {quiet_tokens:?}"
    );
    assert!(matches!(quiet_tokens[0], ZkKey::Instance { .. }));

    // 3. OK, "serving": since_ns moves with the level.
    let put = h.set_status(Level::Ok, "serving").await.unwrap();
    assert_eq!(
        (put.level, put.declared, put.raised_by.as_deref()),
        (Some(Level::Ok), Some(Level::Ok), None)
    );
    let got = split(get_state(&ts, SVC).await);
    let (_, serving) = got.status.unwrap();
    assert_eq!(
        (serving.read(), serving.reason.as_str()),
        (Read::Level(Level::Ok), "serving")
    );
    assert!(
        serving.since_ns > first_since,
        "since_ns later than the first"
    );
    eventually("S has the OK", || async {
        snapshot(&log)
            .iter()
            .any(|r| r.is_status() && r.status().reason == "serving")
    })
    .await;
    let ok_at = Instant::now();

    // 4. Nothing for 35 s: the status is confirmed, unchanged.
    tokio::time::sleep_until((ok_at + Duration::from_secs(35)).into()).await;
    let confirmed: Vec<Rec> = snapshot(&log)
        .into_iter()
        .filter(|r| r.is_status() && !r.delete && r.status().reason == "serving")
        .collect();
    assert!(confirmed.len() >= 2, "the put and a re-put: {confirmed:#?}");
    for w in confirmed.windows(2) {
        assert!(
            w[1].at - w[0].at <= Duration::from_secs(30) + JITTER,
            "no two deliveries more than 30 s apart: {:?}",
            w[1].at - w[0].at
        );
        assert_eq!(w[1].payload, w[0].payload, "the same payload, since_ns too");
        assert!(w[1].stamp.unwrap() > w[0].stamp.unwrap(), "a later stamp");
        assert_eq!(w[1].stamp_id(), owner_zid, "of the same id");
    }
    assert!(
        ok_at + Duration::from_secs(35) - confirmed.last().unwrap().at
            <= Duration::from_secs(30) + JITTER,
        "and none more than 30 s before the window ends"
    );

    // 5. The owner closes; S listens 3 s more.
    svc.close().await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !snapshot(&log).iter().any(|r| r.is_status() && r.delete),
        "no delete of the status, before or after the close"
    );
    quiet.close().await.unwrap();
}

/// §2: a status no longer confirmed, its owner up, reads stale at 60 s for
/// S and for G, never unhealthy and never its last level as current.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_stale_status() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, SVC);
    let sub = watch_status(&c).await;
    let (_s, log) = record(&rs, "zk2/lab/svc/health.v1/**").await;

    // 1. OK, confirmed; S and G judge.
    let (b, h) = owner(&os, config(SVC)).await;
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    let svc = b.start().await.unwrap();
    assert!(
        h.is_confirming(),
        "the contract's horizon turns the re-puts on"
    );
    at_least(&log, 1).await;
    eventually("S's subscription has it", || async {
        !sub.members().is_empty()
    })
    .await;
    let trust = sub.clock_trust(&owner_zid, DEFAULT_DELTA);
    assert!(matches!(trust, ClockTrust::Trusted { .. }), "{trust:?}");
    let present = presence_of(&rs, SVC).await;
    assert_eq!(present, Presence::Present(Listing::Listed { token: true }));
    assert_eq!(
        verdict(judge_s(&sub, &snapshot(&log), present)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "S"
    );
    assert_eq!(
        verdict(split(get_state(&rs, SVC).await).judge(present, trust)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "G"
    );

    // 2. The owner stops confirming its status; its service stays up.
    h.set_confirming(false);
    assert!(!h.is_confirming());
    let last = snapshot(&log).last().unwrap().at;

    // 3. 65 s after S's last delivery.
    tokio::time::sleep_until((last + Duration::from_secs(65)).into()).await;
    assert_eq!(
        snapshot(&log).len(),
        1,
        "no re-put once the owner stopped confirming"
    );
    let tokens = tokens_of(&rs, SVC).await;
    assert!(
        tokens.iter().any(|k| matches!(k, ZkKey::Instance { .. }))
            && tokens.iter().any(is_health_token),
        "the instance and interface tokens are present: {tokens:?}"
    );
    let present = presence_of(&rs, SVC).await;
    let s = judge_s(&sub, &snapshot(&log), present);
    assert_eq!(
        verdict(s),
        (
            Verdict::Stale,
            Reason::Freshness(zenkey::model::freshness::Reason::BeyondHorizon),
            None
        ),
        "S: stale, never unhealthy, never OK as current"
    );
    let got = split(get_state(&rs, SVC).await);
    let (current, _) = got.status.as_ref().unwrap();
    let age = StampAge::between(
        current.timestamp().unwrap().get_time().to_system_time(),
        SystemTime::now(),
    );
    assert!(age.as_secs_f64() > 60.5, "the reply's stamp is {age:?} old");
    let g = got.judge(present, trust);
    assert_eq!(
        verdict(g),
        (
            Verdict::Stale,
            Reason::Freshness(zenkey::model::freshness::Reason::BeyondHorizon),
            None
        ),
        "G"
    );
    svc.close().await.unwrap();
}

/// §3: the status never better than its worst current check, with the puts
/// ordered so that no delivery breaks it; a retired check deleted; no
/// check re-put.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_aggregation() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, SVC);
    let sub = watch_status(&c).await;
    let (_s, log) = record(&rs, "zk2/lab/svc/health.v1/**").await;

    // 1. Status OK, disk OK, net OK.
    let (b, h) = owner(&os, config(SVC)).await;
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    h.set_check("disk", Level::Ok, "70% used").await.unwrap();
    h.set_check("net", Level::Ok, "up").await.unwrap();
    let svc = b.start().await.unwrap();
    at_least(&log, 3).await;
    eventually("S's subscription has it", || async {
        !sub.members().is_empty()
    })
    .await;
    let trust = sub.clock_trust(&owner_zid, DEFAULT_DELTA);
    let present = presence_of(&rs, SVC).await;
    assert_eq!(
        verdict(split(get_state(&rs, SVC).await).judge(present, trust)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );
    let unchanged = h.set_check("net", Level::Ok, "up").await.unwrap();
    assert_eq!(
        unchanged.level,
        Some(Level::Ok),
        "a check put on change only"
    );

    // 2. disk FAILED: the status first, raised to the check.
    let raised = h.set_check("disk", Level::Failed, "full").await.unwrap();
    assert_eq!(
        (raised.level, raised.declared, raised.raised_by.as_deref()),
        (Some(Level::Failed), Some(Level::Ok), Some("disk")),
        "the caller learns that its status was raised"
    );
    assert_eq!(raised.reason, "check disk is FAILED (declared OK: serving)");
    let got = at_least(&log, 5).await;
    assert!(got[3].is_status() && got[3].status().read() == Read::Level(Level::Failed));
    assert_eq!(got[4].check(), Some("disk"));
    assert_eq!(got[4].check_value().read(), Read::Level(Level::Failed));
    let reply = split(get_state(&rs, SVC).await);
    assert_eq!(
        reply.status.as_ref().unwrap().1.read(),
        Read::Level(Level::Failed)
    );
    assert_eq!(
        reply.checks["disk"].as_ref().unwrap().read(),
        Read::Level(Level::Failed)
    );
    assert_eq!(
        reply.checks["net"].as_ref().unwrap().read(),
        Read::Level(Level::Ok)
    );
    assert_eq!(
        verdict(reply.judge(present, trust)),
        (Verdict::Unhealthy, Reason::Failed, Some(Level::Failed))
    );

    // 3. disk OK: the check first, then the status improves.
    let back = h.set_check("disk", Level::Ok, "70% used").await.unwrap();
    assert_eq!((back.level, back.raised_by), (Some(Level::Ok), None));
    let got = at_least(&log, 7).await;
    assert_eq!(got[5].check(), Some("disk"));
    assert_eq!(got[5].check_value().read(), Read::Level(Level::Ok));
    assert!(got[6].is_status() && got[6].status().read() == Read::Level(Level::Ok));
    assert_eq!(got[6].status().reason, "serving");
    assert_eq!(
        verdict(split(get_state(&rs, SVC).await).judge(present, trust)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );

    // 4. net retired: deleted, and no status put for it.
    h.retire_check("net").await.unwrap();
    let got = at_least(&log, 8).await;
    assert!(got[7].delete && got[7].check() == Some("net"));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(snapshot(&log).len(), 8, "no status put for the retire");
    let reply = split(get_state(&rs, SVC).await);
    assert!(reply.status.is_some());
    assert!(reply.checks["disk"].is_some());
    assert_eq!(
        reply.checks.get("net"),
        Some(&None),
        "a reply_del for checks/net"
    );

    // Throughout: no delivery's latest status better than its latest
    // check, and each check delivered only when it changed.
    let log = snapshot(&log);
    let (mut status, mut checks) = (None, BTreeMap::<String, Option<Level>>::new());
    for r in &log {
        if r.is_status() {
            status = r.status().read().level();
        } else if let Some(name) = r.check() {
            let l = (!r.delete)
                .then(|| r.check_value().read().level())
                .flatten();
            checks.insert(name.to_owned(), l);
        }
        for (name, l) in &checks {
            if let (Some(s), Some(l)) = (status, l) {
                assert!(s >= *l, "{name} {l} under a status {s}: {log:#?}");
            }
        }
    }
    let of = |name: &str| -> Vec<Option<Level>> {
        log.iter()
            .filter(|r| r.check() == Some(name))
            .map(|r| {
                (!r.delete)
                    .then(|| r.check_value().read().level())
                    .flatten()
            })
            .collect()
    };
    assert_eq!(
        of("disk"),
        [Some(Level::Ok), Some(Level::Failed), Some(Level::Ok)]
    );
    assert_eq!(of("net"), [Some(Level::Ok), None]);
    svc.close().await.unwrap();
}

/// §4: a clock 2 s ahead: `clock_ahead` at the first hold, re-stamped by
/// R1, again within a status interval; the status stale, never FAILED;
/// after the correction, the status re-put and no more faults.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_clock_ahead() {
    let (r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let hb = client(&ep).await;
    let owner_zid = os.zid().to_string();
    let c = reader(&rs, "lab/ahead");
    let sub = watch_status(&c).await;
    let (_s, log) = record(&rs, "zk2/lab/ahead/health.v1/**").await;

    let mut cfg = config("lab/ahead");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let (b, h) = owner(&os, cfg).await;
    b.minter().simulate_offset(2000);
    matched(h.status_writer().writer()).await;
    matched(h.faults_writer()).await;

    // 1. The status, before any heartbeat.
    h.set_status(Level::Ok, "serving").await.unwrap();
    let svc = b.start().await.unwrap();
    at_least(&log, 1).await;
    assert!(snapshot(&log)[0].is_status());

    // 2. The heartbeat until the owner reports itself ahead.
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        h.is_clock_ahead()
    })
    .await;
    let detected = Instant::now();
    eventually("the fault arrives", || async {
        snapshot(&log).iter().any(Rec::is_fault)
    })
    .await;
    let first = snapshot(&log).into_iter().find(Rec::is_fault).unwrap();
    assert!(
        first.at <= detected + Duration::from_secs(1),
        "within 1 s of the detection"
    );
    let f = first.fault();
    assert_eq!(
        (f.code.as_str(), f.read()),
        (health::CLOCK_AHEAD, Read::Level(Level::Failed))
    );
    assert!(f.detail.contains(" ms ahead"), "{}", f.detail);
    assert_eq!(
        first.stamp_id(),
        r1.zid().to_string(),
        "R1 re-stamped the future-dated fault (core §4.1)"
    );
    assert_ne!(first.stamp_id(), owner_zid);
    assert!(set_status_refused(&h).await, "the guard holds state writes");
    tokio::time::sleep_until((detected + Duration::from_secs(65)).into()).await;
    let held = snapshot(&log);
    assert!(
        !held.iter().any(|r| r.is_status() && r.at > detected),
        "no status put from the detection on"
    );
    let faults: Vec<&Rec> = held.iter().filter(|r| r.is_fault()).collect();
    assert!(faults.len() >= 2, "repeated while held: {faults:#?}");
    for w in faults.windows(2) {
        assert!(
            w[1].at - w[0].at <= Duration::from_secs(31),
            "again within 31 s: {:?}",
            w[1].at - w[0].at
        );
        assert_eq!(w[1].fault().code, health::CLOCK_AHEAD);
    }
    let tokens = tokens_of(&rs, "lab/ahead").await;
    assert!(
        tokens.iter().any(|k| matches!(k, ZkKey::Instance { .. }))
            && tokens.iter().any(is_health_token),
        "the owner's tokens stay present: {tokens:?}"
    );
    let present = presence_of(&rs, "lab/ahead").await;
    assert_eq!(
        verdict(judge_s(&sub, &held, present)),
        (
            Verdict::Stale,
            Reason::Freshness(zenkey::model::freshness::Reason::BeyondHorizon),
            None
        ),
        "stale at 65 s, never FAILED"
    );
    assert!(clock_ahead(&held), "is its clock ahead? yes");

    // 3. The clock set right, the heartbeat on: the status re-put, and no
    // clock_ahead after it.
    svc.minter().simulate_offset(0);
    eventually("the guard releases", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        !h.is_clock_ahead()
    })
    .await;
    let released = Instant::now();
    eventually("the status is re-put", || async {
        snapshot(&log)
            .iter()
            .any(|r| r.is_status() && r.at > released)
    })
    .await;
    let reput = snapshot(&log)
        .into_iter()
        .find(|r| r.is_status() && r.at > released)
        .unwrap();
    assert!(
        reput.at <= released + Duration::from_secs(3),
        "re-put within a few seconds of the correction"
    );
    assert_eq!(reput.status().read(), Read::Level(Level::Ok));
    tokio::time::sleep_until((released + Duration::from_secs(35)).into()).await;
    let after = snapshot(&log);
    assert!(
        !after.iter().any(|r| r.is_fault() && r.at > reput.at),
        "no clock_ahead after the re-put"
    );
    assert_eq!(
        verdict(judge_s(&sub, &after, present)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );
    assert!(!clock_ahead(&after), "is its clock ahead? no");
    svc.close().await.unwrap();
}

/// Whether the guard refuses a status put (core §4.3).
async fn set_status_refused(h: &Health) -> bool {
    matches!(
        h.set_status(Level::Degraded, "probe").await,
        Err(zenkey::Error::ClockAhead)
    )
}

/// §4, what the text leaves to zenoh, measured. A fault carries the owner's
/// clock as its stamp (v1.md §2.5): a router with
/// `timestamping.drop_future_timestamp` drops a fault from a clock beyond
/// its delta, and the status puts made before the guard tripped too, so S
/// hears nothing while the owner's GET still answers its status. An owner
/// whose own session runs its HLC, as the owner example's `--connect` does,
/// has its session re-stamp the simulated offset before R1 sees it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_the_faults_stamp_measured() {
    // drop_future_timestamp: nothing of the owner ahead reaches S.
    let (r2, ep) = router_with(None, &[("timestamping/drop_future_timestamp", "true")]).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let hb = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/ahead/health.v1/**").await;
    let mut cfg = config("lab/ahead");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let (b, h) = owner(&os, cfg).await;
    b.minter().simulate_offset(2000);
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    let svc = b.start().await.unwrap();
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        h.is_clock_ahead()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        snapshot(&log).is_empty(),
        "R2 drops the fault and the future-dated status: {:#?}",
        snapshot(&log)
    );
    let got = split(get_state(&rs, "lab/ahead").await);
    let (current, status) = got.status.expect("the owner's GET answers it");
    assert_eq!(status.read(), Read::Level(Level::Ok));
    assert_eq!(
        current.clock().as_deref(),
        Some(os.zid().to_string().as_str()),
        "a GET reply is never re-stamped (core §4.1)"
    );
    svc.close().await.unwrap();
    drop(r2);

    // An owner session with its HLC on, through R1 without the setting.
    let (r1, ep) = router(None).await;
    let mut z = zenoh::Config::default();
    z.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    z.insert_json5("mode", "\"client\"").unwrap();
    z.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    z.insert_json5("timestamping/enabled", "true").unwrap();
    let os = zenoh::open(z).await.unwrap();
    let rs = client(&ep).await;
    let hb = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/ahead/health.v1/**").await;
    let mut cfg = config("lab/ahead");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let (b, h) = owner(&os, cfg).await;
    b.minter().simulate_offset(2000);
    matched(h.faults_writer()).await;
    let svc = b.start().await.unwrap();
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        h.is_clock_ahead()
    })
    .await;
    eventually("the fault arrives", || async {
        snapshot(&log).iter().any(Rec::is_fault)
    })
    .await;
    let fault = snapshot(&log).into_iter().find(Rec::is_fault).unwrap();
    let stamp = fault.stamp.unwrap();
    let age = StampAge::between(stamp.get_time().to_system_time(), SystemTime::now());
    assert_eq!(
        fault.stamp_id(),
        os.zid().to_string(),
        "not R1's ({}): the owner's own session re-stamped it",
        r1.zid()
    );
    assert!(
        age.as_secs_f64().abs() < DEFAULT_DELTA.as_secs_f64(),
        "with its own HLC's time, not the offset clock's: {age:?}"
    );
    svc.close().await.unwrap();
}

fn nav() -> IfaceId {
    "nav.v2".parse().unwrap()
}

/// A `nav.v2` owner (presence.md §1's contract) at `cfg`, its required
/// resources answered and its member template exposed; the queryables are
/// returned to be kept.
async fn nav_builder(
    s: &zenoh::Session,
    cfg: ServiceConfig,
) -> (ServiceBuilder, Vec<zenoh::query::Queryable<()>>) {
    use zenoh::Wait;
    let mut b = ServiceBuilder::new(s, cfg);
    b.implement(imp("nav.v2")).unwrap();
    let mut held = Vec::new();
    for (res, text) in [("state/pose", "here"), ("@op/goto", "ok")] {
        let key = b.key(&nav(), res, &Bindings::new()).unwrap().into_keyexpr();
        held.push(
            b.declare_queryable(&nav(), res, Some(&Bindings::new()), move |q| {
                let _ = q.reply(key.clone(), text).wait();
            })
            .await
            .unwrap(),
        );
    }
    b.expose(&nav(), "state/tracks/{track}").unwrap();
    (b, held)
}

/// §5: 100 services with `health.v1` tokenless: 200 tokens, none of them
/// `health.v1`'s, and every service found by its descriptor and judged
/// healthy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_tokenless_set_of_100() {
    const N: usize = 100;
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let host = client(&ep).await;
    let host_zid = host.zid().to_string();
    let c = reader(&tool, "p5/*");
    let sub = watch_status(&c).await;
    let mut services = Vec::new();
    for i in 0..N {
        let cfg = config(&format!("p5/dev{i}")).tokenless(health::iface());
        let (mut b, held) = nav_builder(&host, cfg).await;
        let h = b.health().await.unwrap();
        h.set_status(Level::Ok, "serving").await.unwrap();
        services.push((b.start().await.unwrap(), held));
    }

    // The tool lists the presence domain, then health.v1's tokens.
    eventually("200 tokens", || async {
        presence::liveliness_keys(&tool, "zk2/p5/*/@zk/**", T)
            .await
            .unwrap()
            .len()
            == 2 * N
    })
    .await;
    let all = presence::tokens(&tool, "zk2/p5/*/@zk/**", T).await.unwrap();
    let count = |f: &dyn Fn(&ZkKey) -> bool| all.iter().filter(|k| f(k)).count();
    assert_eq!(count(&|k| matches!(k, ZkKey::Instance { .. })), N);
    assert_eq!(
        count(&|k| matches!(k, ZkKey::Alive { iface, .. } if *iface == nav())),
        N
    );
    assert_eq!(count(&is_health_token), 0);
    assert!(
        presence::liveliness_keys(&tool, "zk2/p5/*/@zk/alive/health.v1/**", T)
            .await
            .unwrap()
            .is_empty(),
        "the second read finds nothing"
    );

    // Each descriptor, then each listed service's health.v1/state/**.
    eventually("S measured the host's clock", || async {
        !sub.members().is_empty()
    })
    .await;
    let trust = sub.clock_trust(&host_zid, DEFAULT_DELTA);
    let mut healthy = 0;
    for k in &all {
        let ZkKey::Instance { addr: a, instance } = k else {
            continue;
        };
        let d = presence::descriptor(&tool, a, instance, T)
            .await
            .unwrap()
            .into_descriptor()
            .unwrap();
        let e = d
            .interfaces
            .iter()
            .find(|e| e.iface == "health.v1")
            .expect("listed");
        assert!(!e.token, "\"token\": false");
        let present = Presence::Present(Listing::Listed { token: e.token });
        let got = split(get_state(&tool, &a.to_string()).await);
        assert_eq!(
            verdict(got.judge(present, trust)),
            (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
            "{a}"
        );
        healthy += 1;
    }
    assert_eq!(healthy, N, "every provider found and judged");
    for (s, _) in services {
        s.close().await.unwrap();
    }
}
