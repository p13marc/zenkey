//! `spec/profiles/health/scenarios-v2.md` §1–§5 and §9 (`health.v2`, #986's
//! Q2), one test per section, named after it. §6–§8 are a tool's, and run
//! in `zenctl/tests/live_zk2.rs`.
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default, and without `timestamping.drop_future_timestamp`
//! except where a step measures it. Owners implement `health.v2` through
//! [`ServiceBuilder::health_v2`], on the standard contract, as clients of
//! R1, with no catalog: the deployment epoch is 0. S records every delivery
//! under the owner's `health.v2/**` as it arrived (arrival on this host's
//! monotonic clock, key, kind, the put's whole payload, the Zenoh stamp),
//! with a consumer's subscription on the status for its freshness, and
//! reads each heartbeat covering `health.v2` raw. G GETs the owner's
//! `health.v2/state/**` as core S4 says (target `All`, consolidation
//! `None`) and reads its two snapshots as `zenkey_model::health::v2::Reading`
//! does. G and T take the deployment's word for their clock. Every verdict
//! is `zenkey_model::health::judge`'s, and every answer to the clock
//! question `zenkey_model::health::v2::clock_ahead`'s.
//!
//! **A 16-bit run.** §1's Expected gives a status put 12 bytes and a
//! heartbeat 16 under a 16-bit run (v2.md §3, Appendix B): `lab/svc` and
//! `lab/ahead` take their run from a run file. Every other owner runs on
//! the runtime's default, a random 32-bit run, whose put is 14 bytes under
//! CH `0x13`, which §9's test holds.
//!
//! **§9's tools.** T and T2 subscribe before step 1, as the scenario's
//! setup says: step 2 replaces step 1's status, and a fault is a stream
//! sample no later read answers. Each shows a word by `v2::read_word` over
//! the tables of the revisions the owner's descriptor names among those it
//! holds; the fleet's and zenctl's rendering is a later chunk's.
//!
//! **Without root** (§4's setup): the owner's clock is offset in its
//! runtime (`Minter::simulate_offset`). No expectation rests on a fault's
//! stamp, since a fault of this major carries none of its owner's.
//!
//! **The horizon.** The status is never put again unchanged: its owner's
//! heartbeat confirms it within the contract's horizon (`freshness.v2`
//! §2.4). §1, §2 and §4 wait the horizon out, so they run on a test's copy
//! of the standard contract whose `freshness.ttl_s` is 6, made in memory
//! and never published, as the scenarios' conventions allow (#1151): its
//! heartbeat every 2.85 s, under the copy's tag, and every reader here, S's
//! subscription, H, L and G's verdict, holds it. At the standard's 60 s
//! they wait about 70 s, 135 s and 140 s, in the slow tier, `#[ignore]`d
//! (`…_at_60_s`).

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::{Beats, T, addr, client, config, eventually, imp, router, router_with};
use zenkey::consumer::{Consumer, Subscription};
use zenkey::health::{self, Health, Level, Major, Read, v2};
use zenkey::model::collection::Confirmation;
use zenkey::model::descriptor::Descriptor;
use zenkey::model::freshness::{self, ClockTrust, DEFAULT_DELTA, Horizon, Observation};
use zenkey::model::grammar::{IfaceId, InstanceId, ZkKey, parse};
use zenkey::model::health::v2::{self as m2, ClockHeard, Packed as _, StatusEntry};
use zenkey::model::health::{
    self as hm, Answer, ClockReason, Judged, Listing, Presence, Reason, Verdict,
};
use zenkey::model::heartbeat::Time;
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Form, Run, Stamp};
use zenkey::prost::Message as _;
use zenkey::run::FileRunStore;
use zenkey::state::Current;
use zenkey::{ServiceBuilder, ServiceConfig, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

/// The deployment's word for G's clock (`freshness.v1` §2.6, ground 1).
const TRUSTED: ClockTrust = ClockTrust::Trusted {
    delta: DEFAULT_DELTA,
};

/// The scenarios' deployment epoch: no owner is given a catalog.
const EPOCH: i64 = 0;

/// The owner of `scenarios-v2.md`'s conventions.
const SVC: &str = "lab/svc";

/// The revision a section runs on, and its horizon (the conventions' "The
/// horizon"): the standard contract's 60 s, or a test's copy's 6 s, which
/// every reader of the section holds.
#[derive(Clone)]
struct On {
    imp: zenkey::Implementation,
    horizon: Duration,
}

impl On {
    /// The standard contract, revision 2.0: the slow tier.
    fn standard() -> On {
        On {
            imp: v2::implementation(),
            horizon: Duration::from_secs(60),
        }
    }

    /// The copy whose `freshness.ttl_s` is 6: a heartbeat every 2.85 s.
    fn short() -> On {
        On {
            imp: common::short_horizon("health/health.v2.toml", 60, 6),
            horizon: Duration::from_secs(6),
        }
    }

    /// The scenarios' "65 s": past the horizon by 5 s.
    fn past(&self) -> Duration {
        self.horizon + Duration::from_secs(5)
    }

    /// The heartbeat confirms the status at least every half horizon, the
    /// standard's 30 s (§2.3).
    fn beat(&self) -> Duration {
        self.horizon / 2
    }

    /// The scenarios' "35 s": half the horizon and 5 s.
    fn half_past(&self) -> Duration {
        self.beat() + Duration::from_secs(5)
    }

    /// The revision, as a heartbeat covering it is read.
    fn layout(&self) -> zenkey::model::heartbeat::Layout {
        zenkey::model::heartbeat::Layout::of(self.imp.contract())
    }

    /// The status's horizon, as `freshness.v1` reads it.
    fn within(&self) -> Horizon {
        Horizon::Within(self.horizon)
    }
}

/// The key of `address`'s heartbeat covering `health.v2`.
fn heartbeat_key(address: &str) -> String {
    format!("zk2/{address}/freshness.v2/stream/heartbeat/health.v2")
}

/// `address`'s heartbeats covering `health.v2`, read raw with `on`'s
/// revision.
async fn heartbeats(
    s: &zenoh::Session,
    address: &str,
    on: &On,
) -> (zenoh::pubsub::Subscriber<()>, Beats) {
    common::heartbeats(s, &heartbeat_key(address), on.layout()).await
}

/// Whether `b` vouches for the status: `health.v2`'s one collection with
/// a horizon.
fn vouches(b: &common::Beat) -> bool {
    matches!(
        b.hb.sections.first(),
        Some(Some(zenkey::model::heartbeat::Section {
            members: zenkey::model::heartbeat::Members::One { vouched: true },
            ..
        }))
    )
}

/// The heartbeats heard after `from`.
fn beats_after(beats: &Beats, from: Instant) -> Vec<common::Beat> {
    beats
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.at > from)
        .cloned()
        .collect()
}

/// One delivery, as S records it: the put's payload whole, its envelope
/// included.
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
        self.key.ends_with("/health.v2/state/status")
    }

    fn check(&self) -> Option<&str> {
        self.key.split_once("/health.v2/state/checks/").map(|x| x.1)
    }

    fn is_fault(&self) -> bool {
        self.key.ends_with("/health.v2/stream/faults")
    }

    /// The put's envelope (core §7.5): a mutation's, CH, the run, the seq,
    /// then the record.
    fn envelope(&self) -> Envelope<'_> {
        let e = Envelope::parse(&self.payload).expect("a zk2c envelope");
        assert_eq!(e.form, Form::Mutation, "a put of a record: {self:?}");
        e
    }

    /// Its in-band stamp: the owner's run and seq.
    fn inband(&self) -> Stamp {
        self.envelope().stamp
    }

    fn status(&self) -> v2::Status {
        let e = self.envelope();
        v2::Status::read(e.tag.get(), e.record, EPOCH).expect("a health.v2 Status")
    }

    fn check_value(&self) -> v2::Check {
        let e = self.envelope();
        v2::Check::read(e.tag.get(), e.record, EPOCH).expect("a health.v2 Check")
    }

    fn fault(&self) -> v2::Fault {
        let e = self.envelope();
        v2::Fault::read(e.tag.get(), e.record, EPOCH).expect("a health.v2 Fault")
    }

    /// The level a reader reads of a status's or a check's put (§2.6).
    fn level(&self) -> Read {
        let e = self.envelope();
        m2::level_read(v2::Status::read(e.tag.get(), e.record, EPOCH).map(|s| s.level))
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

/// A tool's consumer of `address`'s `health.v2`, on `on`'s revision.
fn reader(s: &zenoh::Session, address: &str, on: &On) -> Consumer {
    Consumer::for_tool(s, on.imp.shared_contract(), &[address], &Default::default()).unwrap()
}

/// S's subscription on the status: its freshness, confirmed by the
/// owner's heartbeats (`freshness.v2` §2.5).
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

/// A run file of this test's own, for an owner on a 16-bit run.
fn run_file(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("zk2-health-v2-{name}-{}.run", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

/// An owner at `cfg`'s address implementing `health.v2`, not started; on a
/// 16-bit run from `runs` when given.
async fn owner(
    s: &zenoh::Session,
    cfg: ServiceConfig,
    runs: Option<&PathBuf>,
) -> (ServiceBuilder, Health) {
    owner_on(s, cfg, runs, &On::standard()).await
}

/// [`owner`] on `on`'s revision.
async fn owner_on(
    s: &zenoh::Session,
    cfg: ServiceConfig,
    runs: Option<&PathBuf>,
    on: &On,
) -> (ServiceBuilder, Health) {
    let mut b = ServiceBuilder::new(s, cfg);
    if let Some(p) = runs {
        b.run_store(Arc::new(FileRunStore::new(p))).unwrap();
    }
    let h = b.health_v2_with(on.imp.clone()).await.unwrap();
    assert_eq!(h.major(), Major::V2);
    (b, h)
}

/// What a GET over `…/health.v2/state/**` answered: its replies as they
/// arrived.
struct Got {
    samples: Vec<Sample>,
}

/// G's GET of `selector` (core S2, S4).
async fn get(s: &zenoh::Session, selector: &str) -> Got {
    let rx = s
        .get(selector.to_owned())
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut samples = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        if let Ok(sample) = r.into_result() {
            samples.push(sample);
        }
    }
    Got { samples }
}

/// G's GET of `address`'s `health.v2/state/**`.
async fn get_state(s: &zenoh::Session, address: &str) -> Got {
    get(s, &format!("zk2/{address}/health.v2/state/**")).await
}

impl Got {
    /// The replies under `address`'s keys.
    fn of(&self, address: &str) -> Vec<&Sample> {
        let prefix = format!("zk2/{address}/");
        self.samples
            .iter()
            .filter(|s| s.key_expr().as_str().starts_with(&prefix))
            .collect()
    }

    /// `address`'s two snapshots, read (§2.4, core §4.2).
    fn reading(&self, address: &str) -> m2::Reading {
        let replies: Vec<(String, Vec<u8>)> = self
            .of(address)
            .into_iter()
            .map(|s| {
                (
                    s.key_expr().as_str().to_owned(),
                    s.payload().to_bytes().into_owned(),
                )
            })
            .collect();
        m2::Reading::of(
            replies.iter().map(|(k, p)| (k.as_str(), p.as_slice())),
            EPOCH,
        )
    }

    /// The status's entry, live.
    fn status(&self, address: &str) -> m2::Entry<v2::Status> {
        match self
            .reading(address)
            .status
            .expect("the status's snapshot")
            .entry
        {
            StatusEntry::Live(e) => e,
            other => panic!("no live status: {other:?}"),
        }
    }

    /// The status's record.
    fn status_record(&self, address: &str) -> v2::Status {
        self.status(address).record.expect("a record that reads")
    }

    /// The status's entry as a reply stamped with its confirmation, as
    /// the runtime's consumer stamps it (`freshness.v2` §2.6): what G ages.
    fn status_current(&self, address: &str) -> Current {
        let key = format!("zk2/{address}/health.v2/state/status");
        self.of(address)
            .into_iter()
            .flat_map(|s| common::replies_aged(s.clone()))
            .find(|s| s.key_expr().as_str() == key && s.kind() == SampleKind::Put)
            .map(|sample| Current::Value {
                key,
                sample: Box::new(common::strip(sample)),
            })
            .expect("the status in the reply")
    }

    /// G judges `address` from this reply (v2.md §2.11), its clock as
    /// `trust`.
    fn judge(&self, address: &str, presence: Presence, trust: ClockTrust) -> Judged {
        self.judge_on(address, presence, trust, &On::standard())
    }

    /// [`Got::judge`] at `on`'s horizon, the one G holds.
    fn judge_on(&self, address: &str, presence: Presence, trust: ClockTrust, on: &On) -> Judged {
        let reading = self.reading(address);
        let obs = [self.status_current(address).freshness(trust)];
        let checks = reading.check_levels();
        hm::judge_status(
            presence,
            freshness::judge_all(&on.within(), &obs),
            reading.status_level(),
            checks.as_deref(),
        )
    }
}

/// S judges from its deliveries at `on`'s horizon: the status's freshness
/// on its subscription, the latest status's level, and the latest of each
/// check.
fn judge_s(sub: &Subscription, log: &[Rec], presence: Presence, on: &On) -> Judged {
    let status: Vec<&Rec> = log.iter().filter(|r| r.is_status()).collect();
    let key = &status.last().expect("a status delivery").key;
    let obs: [Observation; 1] = [sub.freshness(key)];
    let mut checks = BTreeMap::new();
    for r in log {
        if let Some(name) = r.check() {
            checks.insert(name.to_owned(), (!r.delete).then(|| r.level()));
        }
    }
    let checks: Vec<Read> = checks.into_values().flatten().collect();
    hm::judge_status(
        presence,
        freshness::judge_all(&on.within(), &obs),
        status.last().map(|r| r.level()),
        Some(&checks),
    )
}

fn verdict(j: Judged) -> (Verdict, Reason, Option<Level>) {
    (j.verdict, j.reason, j.level)
}

/// The descriptor of `address`'s one instance. Every caller expects the
/// owner present, so the read waits for its instance token to reach the
/// tool: a start returns before its declarations reach R1.
async fn descriptor_of(tool: &zenoh::Session, address: &str) -> Descriptor {
    let instance = || async {
        presence::tokens(tool, &format!("zk2/{address}/@zk/**"), T)
            .await
            .unwrap()
            .into_iter()
            .find_map(|k| match k {
                ZkKey::Instance { instance, .. } => Some(instance),
                _ => None,
            })
    };
    eventually("the owner's instance token", || async {
        instance().await.is_some()
    })
    .await;
    let instance = instance().await.expect("an instance token");
    described(tool, address, &instance).await
}

async fn described(tool: &zenoh::Session, address: &str, instance: &InstanceId) -> Descriptor {
    presence::descriptor(tool, &addr(address), instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap()
}

/// The major a descriptor is read by (§2.11's step 2, §2.12), and the
/// presence its listing gives.
fn listed(d: &Descriptor) -> (Option<Major>, Presence) {
    let major = Major::of(d.interfaces.iter().map(|e| e.iface.as_str()));
    let listing = major.map_or(Listing::NotListed, |m| {
        let e = d
            .interfaces
            .iter()
            .find(|e| e.iface == m.iface())
            .expect("listed");
        Listing::Listed { token: e.token }
    });
    (major, Presence::Present(listing))
}

/// A present owner whose descriptor lists `health.v2`, as a tool reads it
/// (§2.7): through its instance token and descriptor.
async fn presence_of(tool: &zenoh::Session, address: &str) -> Presence {
    let (major, presence) = listed(&descriptor_of(tool, address).await);
    assert_eq!(major, Some(Major::V2), "{address} is read by health.v2");
    presence
}

/// The tokens `address` holds.
async fn tokens_of(tool: &zenoh::Session, address: &str) -> Vec<ZkKey> {
    presence::tokens(tool, &format!("zk2/{address}/@zk/**"), T)
        .await
        .unwrap()
}

fn is_health_token(k: &ZkKey) -> bool {
    matches!(k, ZkKey::Alive { iface, .. } if *iface == v2::iface())
}

fn is_freshness_token(k: &ZkKey) -> bool {
    matches!(k, ZkKey::Alive { iface, .. } if *iface == zenkey::heartbeat::iface())
}

const PRESENT: Presence = Presence::Present(Listing::Listed { token: true });

/// v2.md §5's last question, "is this service's clock ahead?", from what a
/// reader heard, in arrival order (§2.5, its fixture `v2/clock.json`): the
/// last heartbeat read answers, and nothing else does.
fn clock(log: &[Rec], beats: &[common::Beat]) -> (Answer, ClockReason) {
    let mut heard: Vec<(Instant, ClockHeard)> = log
        .iter()
        .filter_map(|r| {
            let h = if r.is_status() {
                if r.delete {
                    ClockHeard::StatusDeleted
                } else {
                    ClockHeard::StatusPut
                }
            } else if r.is_fault() {
                if r.fault().is_clock_ahead() {
                    ClockHeard::ClockAhead
                } else {
                    ClockHeard::Fault
                }
            } else {
                return None;
            };
            Some((r.at, h))
        })
        .collect();
    heard.extend(
        beats
            .iter()
            .map(|b| (b.at, ClockHeard::Heartbeat(b.hb.time))),
    );
    heard.sort_by_key(|(at, _)| *at);
    let heard: Vec<ClockHeard> = heard.into_iter().map(|(_, h)| h).collect();
    let c = m2::clock_ahead(PRESENT, &heard);
    (c.answer, c.reason)
}

fn all_beats(beats: &Beats) -> Vec<common::Beat> {
    beats.lock().unwrap().clone()
}

/// §2.3, "One major per instance": an owner that implements one major of
/// health is refused the other, through the profile's own calls and
/// through `implement`, in either order; nothing of the second is
/// declared, and the first stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_an_owner_implements_one_major() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let both = |e: zenkey::Error| match e {
        zenkey::Error::Contract(why) => {
            assert!(why.contains("never both (health.v2 §2.3)"), "{why}");
        }
        other => panic!("{other}"),
    };

    let mut b = ServiceBuilder::new(&os, config("lab/two"));
    let h = b.health_v2().await.unwrap();
    both(b.health().await.err().expect("health.v1 beside health.v2"));
    both(
        b.implement(health::implementation())
            .err()
            .expect("health.v1's contract beside health.v2"),
    );
    assert!(matches!(
        b.health_v2().await,
        Err(zenkey::Error::Duplicate(i)) if i == v2::iface()
    ));
    assert!(b.implements(&v2::iface()) && !b.implements(&health::iface()));
    assert_eq!(h.iface(), &v2::iface());

    let mut b = ServiceBuilder::new(&os, config("lab/one"));
    let h = b.health().await.unwrap();
    assert_eq!(h.major(), Major::V1);
    both(
        b.health_v2()
            .await
            .err()
            .expect("health.v2 beside health.v1"),
    );
    both(
        b.implement(v2::implementation())
            .err()
            .expect("health.v2's contract beside health.v1"),
    );
    assert!(b.implements(&health::iface()) && !b.implements(&v2::iface()));
}

/// §1: the status answers a GET made at the instance token and at the
/// interface token, or at the instance token for a tokenless `health.v2`;
/// its put is 12 bytes; its heartbeat confirms it every half horizon in 16
/// bytes, it is never put again, S judges it fresh past the horizon, a tool
/// reads each service by the major its descriptor lists, and the status is
/// never deleted. On the copy's 6 s horizon (the module doc).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_bring_up_and_both_majors() {
    bring_up_and_both_majors(On::short()).await;
}

/// §1 at the standard contract: 65 s of nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §1 waits 65 s at the contract's 60 s horizon"]
async fn s1_bring_up_and_both_majors_at_60_s() {
    bring_up_and_both_majors(On::standard()).await;
}

/// §1 on `on`'s revision, `lab/svc`'s and `lab/quiet`'s.
async fn bring_up_and_both_majors(on: On) {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let qs = client(&ep).await;
    let old = client(&ep).await;
    let ts = client(&ep).await;
    let (_s, log) = record(&ts, "zk2/lab/svc/health.v2/**").await;
    let (_h, beats) = heartbeats(&ts, SVC, &on).await;
    let c = reader(&ts, SVC, &on);
    let sub = watch_status(&c).await;
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

    // 1. The owner starts with nothing set: unknown, `starting`. T GETs at
    // the instance token, and again at the interface token.
    let runs = run_file("s1");
    let (b, h) = owner_on(&os, config(SVC), Some(&runs), &on).await;
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
                at.push(("alive/health.v2", get_state(&ts, SVC).await));
            }
            _ => {}
        }
    }
    let stated = described(&ts, SVC, svc.instance()).await.run;
    assert!(stated.is_some(), "the owner's descriptor states its run");
    let mut first_since = 0;
    for (when, got) in &at {
        let reading = got.reading(SVC);
        let snapshot = reading.status.as_ref().expect(when);
        let entry = got.status(SVC);
        let status = entry.record.expect("a record that reads");
        assert_eq!(
            (status.level, status.reason.code()),
            (None, 1),
            "at the {when} token: unknown, with the reason 1, `starting`"
        );
        assert_eq!(
            Some(snapshot.heard.run.to_string()),
            stated,
            "at the {when} token: a snapshot under the descriptor's run"
        );
        assert!(snapshot.heard.complete, "one page, whole");
        assert_eq!(
            entry.confirmation,
            Confirmation::AtTime,
            "at the {when} token: with its confirmation (freshness.v2 §2.4)"
        );
        assert_eq!(
            reading.checks.as_ref().map(|c| c.live.len()),
            Some(0),
            "at the {when} token: the checks' snapshot, of no entry"
        );
        first_since = status.since.expect("a clock the epoch codes");
    }
    // The put, as S heard it: before the tokens, 12 bytes under a 16-bit
    // run: CH 0x03, the run, the seq, and a record of 6 bytes.
    let first = at_least(&log, 1).await.remove(0);
    assert!(first.is_status() && !first.delete);
    assert_eq!(first.payload.len(), 12, "CH, run 2, seq 3, record 6");
    assert_eq!(first.payload[0], 0x03, "form 0, a 16-bit run, tag 3");
    let e = first.envelope();
    assert!(matches!(e.stamp.run, Run::U16(_)));
    assert_eq!(Some(e.stamp.run.to_string()), stated);
    assert_eq!(e.record.len(), 6);
    assert_eq!(first.status(), v2::Status::starting(Some(first_since)));
    let started = h.status().unwrap();
    assert_eq!(
        (
            started.level,
            started.declared,
            started.word.map(v2::Word::code)
        ),
        (None, None, Some(1))
    );

    // 2. lab/quiet: health.v2 and freshness.v2 in its tokenless set, ok
    // before it starts.
    let quiet_cfg = config("lab/quiet")
        .tokenless(v2::iface())
        .tokenless(zenkey::heartbeat::iface());
    let (qb, qh) = owner_on(&qs, quiet_cfg, None, &on).await;
    qh.set_status(Level::Ok, "serving").await.unwrap();
    let quiet = qb.start().await.unwrap();
    let instance = loop {
        let s = next_token().await;
        match parse(s.key_expr().as_str()).unwrap() {
            ZkKey::Instance { addr: a, instance } if a == addr("lab/quiet") => break instance,
            k => assert!(
                !((is_health_token(&k) || is_freshness_token(&k))
                    && matches!(&k, ZkKey::Alive { addr: a, .. } if *a == addr("lab/quiet"))),
                "no health.v2 or freshness.v2 token: {k:?}"
            ),
        }
    };
    let d = described(&ts, "lab/quiet", &instance).await;
    let got = get_state(&ts, "lab/quiet").await;
    let e = d
        .interfaces
        .iter()
        .find(|e| e.iface == "health.v2")
        .expect("listed");
    assert!(!e.token, "\"token\": false");
    assert_eq!(got.status_record("lab/quiet").level, Some(Level::Ok));
    // The instance token reaches the tool's read a little after its
    // subscriber heard it, so the read is asked until it holds one, and
    // that read is the one checked: a second read beside a loaded suite
    // could end at its timeout with nothing (#1151, the flake check).
    let deadline = Instant::now() + common::SETTLE;
    let quiet_tokens = loop {
        let read = tokens_of(&ts, "lab/quiet").await;
        if !read.is_empty() {
            break read;
        }
        assert!(Instant::now() < deadline, "never: the instance token read");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        quiet_tokens.len(),
        1,
        "the instance token only: {quiet_tokens:?}"
    );
    assert!(matches!(quiet_tokens[0], ZkKey::Instance { .. }));

    // 3. ok, `serving`: the reason 2, its `since` not before the first's.
    let put = h.set_status(Level::Ok, "serving").await.unwrap();
    assert_eq!(
        (put.level, put.declared, put.word.map(v2::Word::code)),
        (Some(Level::Ok), Some(Level::Ok), Some(2))
    );
    let serving = get_state(&ts, SVC).await.status_record(SVC);
    assert_eq!((serving.level, serving.reason.code()), (Some(Level::Ok), 2));
    assert!(
        serving.since.expect("a time") >= first_since,
        "its since not before the first status's"
    );
    eventually("S has the ok", || async {
        snapshot(&log)
            .iter()
            .any(|r| r.is_status() && r.status().level == Some(Level::Ok))
    })
    .await;
    let ok_at = Instant::now();

    // 5's owner, started now so that step 4's wait covers it: lab/old
    // implements health.v1, its standard contract's revision 1.1.
    let mut ob = ServiceBuilder::new(&old, config("lab/old"));
    let oh = ob.health().await.unwrap();
    oh.set_status(Level::Ok, "serving").await.unwrap();
    let old_svc = ob.start().await.unwrap();

    // 4. Nothing for 65 s, the horizon and 5 s: S receives no put of the
    // status, a heartbeat at least every half horizon, of form 6 and the
    // revision's tag (1, the standard's), 16 bytes under a 16-bit run, each
    // vouching for the status; and judges it healthy past the horizon.
    let end = ok_at + on.past();
    tokio::time::sleep_until(end.into()).await;
    let puts: Vec<Rec> = snapshot(&log)
        .into_iter()
        .filter(|r| r.is_status() && !r.delete)
        .collect();
    assert_eq!(
        puts.len(),
        2,
        "`starting`, then ok: never put again: {puts:#?}"
    );
    let seen = common::assert_cadence(&beats, ok_at, end, on.beat(), JITTER, on.layout().tag);
    assert_eq!(
        On::standard().layout().tag.get(),
        1,
        "health.v2's revision 2.0"
    );
    assert!(seen.iter().all(vouches), "each vouches for the status");
    assert!(
        seen.iter().all(|b| b.len == 16),
        "16 bytes under a 16-bit run: {:?}",
        seen.iter().map(|b| b.len).collect::<Vec<_>>()
    );
    assert_eq!(
        verdict(judge_s(&sub, &snapshot(&log), PRESENT, &on)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "S, past the horizon after the put: the heartbeats confirmed it"
    );

    // 5. T reads the deployment: one GET over each major's keys, and each
    // instance's descriptor; it judges each service by the major its
    // descriptor lists.
    let v2_got = get(&ts, "zk2/lab/*/health.v2/state/**").await;
    let v1_got = get(&ts, "zk2/lab/*/health.v1/state/**").await;
    let mut judged = Vec::new();
    for (address, expect) in [
        (SVC, Major::V2),
        ("lab/quiet", Major::V2),
        ("lab/old", Major::V1),
    ] {
        let d = descriptor_of(&ts, address).await;
        let (major, present) = listed(&d);
        assert_eq!(major, Some(expect), "{address}");
        let lists = |iface: &str| d.interfaces.iter().any(|e| e.iface == iface);
        assert_eq!(
            (lists("health.v2"), lists("health.v1")),
            (expect == Major::V2, expect == Major::V1),
            "{address}: one major, never both (§2.3)"
        );
        let j = match expect {
            Major::V2 => v2_got.judge_on(address, present, TRUSTED, &on),
            Major::V1 => judge_v1(&v1_got, address, present),
        };
        assert_eq!(
            verdict(j),
            (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
            "{address}, under {}",
            expect.iface()
        );
        judged.push(j);
    }
    assert!(
        v2_got.of("lab/old").is_empty() && v1_got.of(SVC).is_empty(),
        "each owner answers its own major's GET alone"
    );
    let rollup = hm::rollup(judged);
    assert_eq!(
        (rollup.healthy, rollup.worst),
        (3, Some(Level::Ok)),
        "one roll-up over both majors"
    );

    // 6. The owner closes; S listens 3 s more.
    svc.close().await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !snapshot(&log).iter().any(|r| r.is_status() && r.delete),
        "no delete of the status, before or after the close"
    );
    quiet.close().await.unwrap();
    old_svc.close().await.unwrap();
    let _ = std::fs::remove_file(runs);
}

/// T judges `address` under `health.v1`'s text from its GET's replies.
fn judge_v1(got: &Got, address: &str, presence: Presence) -> Judged {
    let key = format!("zk2/{address}/health.v1/state/status");
    let sample = got
        .of(address)
        .into_iter()
        .flat_map(|s| common::replies_aged(s.clone()))
        .find(|s| s.key_expr().as_str() == key)
        .expect("health.v1's status in the reply");
    let current = Current::Value {
        key,
        sample: Box::new(common::strip(sample)),
    };
    let Current::Value { sample, .. } = &current else {
        unreachable!()
    };
    let status = health::v1::Status::decode(sample.payload().to_bytes().as_ref())
        .expect("a health.v1.Status");
    let obs = [current.freshness(TRUSTED)];
    hm::judge(&hm::Reading {
        presence,
        status: &obs,
        level: Some(status.read()),
        checks: Some(&[]),
    })
}

/// §2: a status its heartbeat confirms reads fresh past the horizon, for S
/// and for G, with no put again and confirmation 1; no longer vouched for,
/// its owner up, it reads stale past the horizon for both, G's entry
/// carrying confirmation 2 and a last confirmation more than the horizon
/// and 0.5 s old; never unhealthy and never its last level as current. On
/// the copy's 6 s horizon (the module doc).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_stale_status() {
    stale_status(On::short()).await;
}

/// §2 at the standard contract: 65 s confirmed, then 65 s more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §2 waits 135 s at the contract's 60 s horizon"]
async fn s2_a_stale_status_at_60_s() {
    stale_status(On::standard()).await;
}

/// §2 on `on`'s revision.
async fn stale_status(on: On) {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let c = reader(&rs, SVC, &on);
    let sub = watch_status(&c).await;
    let (_s, log) = record(&rs, "zk2/lab/svc/health.v2/**").await;
    let (_h, beats) = heartbeats(&rs, SVC, &on).await;

    // 1. ok, confirmed past the horizon by the heartbeat; S and G judge.
    let (b, h) = owner_on(&os, config(SVC), None, &on).await;
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    let svc = b.start().await.unwrap();
    assert!(
        h.is_confirming(),
        "the contract's horizon confirms it, by the heartbeat"
    );
    at_least(&log, 1).await;
    eventually("S's subscription has it", || async {
        !sub.members().is_empty()
    })
    .await;
    let put_at = snapshot(&log)[0].at;
    tokio::time::sleep_until((put_at + on.past()).into()).await;
    assert_eq!(
        snapshot(&log).len(),
        1,
        "no put of the status since step 1's"
    );
    let present = presence_of(&rs, SVC).await;
    assert_eq!(present, PRESENT);
    assert_eq!(
        verdict(judge_s(&sub, &snapshot(&log), present, &on)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "S"
    );
    let got = get_state(&rs, SVC).await;
    assert_eq!(
        got.status(SVC).confirmation,
        Confirmation::AtTime,
        "G's entry: confirmation 1, vouched for at the snapshot's time"
    );
    assert_eq!(
        verdict(got.judge_on(SVC, present, TRUSTED, &on)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "G"
    );

    // 2. The owner stops vouching for its status; its service stays up.
    h.set_confirming(false);
    assert!(!h.is_confirming());
    let cleared = Instant::now();

    // 3. Past the horizon (65 s at the standard's) after the last heartbeat
    // that vouched for it, which came before the bit was cleared.
    let last = all_beats(&beats)
        .iter()
        .filter(|b| vouches(b) && b.at <= cleared)
        .map(|b| b.at)
        .max()
        .expect("a heartbeat vouched for it");
    tokio::time::sleep_until((last + on.past()).into()).await;
    let after = beats_after(&beats, cleared + JITTER);
    assert!(!after.is_empty(), "its heartbeats go on");
    assert!(
        !after.iter().any(vouches),
        "each with the status's bit clear"
    );
    assert_eq!(snapshot(&log).len(), 1, "and no put");
    let tokens = tokens_of(&rs, SVC).await;
    assert!(
        tokens.iter().any(|k| matches!(k, ZkKey::Instance { .. }))
            && tokens.iter().any(is_health_token),
        "the owner's tokens are present: {tokens:?}"
    );
    let present = presence_of(&rs, SVC).await;
    let stale = (
        Verdict::Stale,
        Reason::Freshness(zenkey::model::freshness::Reason::BeyondHorizon),
        None,
    );
    assert_eq!(
        verdict(judge_s(&sub, &snapshot(&log), present, &on)),
        stale,
        "S: stale, never unhealthy, never ok as current"
    );
    let got = get_state(&rs, SVC).await;
    let entry = got.status(SVC);
    let Confirmation::At(conf) = entry.confirmation else {
        panic!("confirmation 2, with its last: {:?}", entry.confirmation)
    };
    let conf = common::instant(conf, EPOCH).expect("a last confirmation that is known");
    let age = SystemTime::now()
        .duration_since(conf)
        .expect("a confirmation in the past");
    assert!(
        age.as_secs_f64() > on.horizon.as_secs_f64() + 0.5,
        "a last confirmation more than the horizon and 0.5 s old: {age:?}"
    );
    assert_eq!(
        got.status_record(SVC).level,
        Some(Level::Ok),
        "the record still reads ok, which G does not report as current"
    );
    assert_eq!(
        verdict(got.judge_on(SVC, present, TRUSTED, &on)),
        stale,
        "G"
    );
    svc.close().await.unwrap();
}

/// §3: the status never better than its worst current check at any seq:
/// the status first, its seq the lower, when a check worsens; the check
/// first when it improves; one GET's two snapshots one instant; a retired
/// check deleted, and listed as a tombstone; no check put again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_aggregation_by_seq() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/svc/health.v2/**").await;

    // 1. Status ok, disk ok, net ok.
    let (b, h) = owner(&os, config(SVC), None).await;
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    h.set_check("disk", Level::Ok, "ok").await.unwrap();
    h.set_check("net", Level::Ok, "ok").await.unwrap();
    let svc = b.start().await.unwrap();
    at_least(&log, 3).await;
    let present = presence_of(&rs, SVC).await;
    assert_eq!(
        verdict(get_state(&rs, SVC).await.judge(SVC, present, TRUSTED)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );
    let unchanged = h.set_check("net", Level::Ok, "ok").await.unwrap();
    assert_eq!(unchanged.level, Some(Level::Ok));

    // 2. disk failed, `timeout`: the status first, raised to the check,
    // with the reason `check`, and its seq the lower.
    let raised = h.set_check("disk", Level::Failed, "timeout").await.unwrap();
    assert_eq!(
        (raised.level, raised.declared, raised.raised_by.as_deref()),
        (Some(Level::Failed), Some(Level::Ok), Some("disk")),
        "the caller learns that its status was raised"
    );
    assert_eq!(raised.reason, v2::CHECK);
    let got = at_least(&log, 5).await;
    let (status, disk) = (&got[3], &got[4]);
    assert!(status.is_status());
    assert_eq!(
        (status.status().level, status.status().reason.code()),
        (Some(Level::Failed), 4),
        "failed, with the reason `check`"
    );
    assert_eq!(disk.check(), Some("disk"));
    assert_eq!(
        (disk.check_value().level, disk.check_value().detail.code()),
        (Some(Level::Failed), 3),
        "failed, with the detail `timeout`"
    );
    assert_eq!(
        status.inband().run,
        disk.inband().run,
        "both puts are of one run"
    );
    assert!(
        status.inband().seq.get() < disk.inband().seq.get(),
        "the status's seq is the lower: {:?} then {:?}",
        status.inband(),
        disk.inband()
    );
    let reply = get_state(&rs, SVC).await;
    let reading = reply.reading(SVC);
    let (s, c) = (
        reading.status.as_ref().unwrap(),
        reading.checks.as_ref().unwrap(),
    );
    assert_eq!(
        (s.heard.run, s.heard.asof),
        (c.heard.run, c.heard.asof),
        "two snapshots under one run and one asof"
    );
    assert!(reading.is_one_instant(), "one instant: {reading:#?}");
    assert_eq!(
        reading.status_better_than_a_check(),
        Some(false),
        "no break of §2.2"
    );
    assert_eq!(reply.status_record(SVC).level, Some(Level::Failed));
    let levels: BTreeMap<&str, Option<Level>> = c
        .live
        .iter()
        .map(|e| (e.id.as_str(), e.record.unwrap().level))
        .collect();
    assert_eq!(
        levels,
        BTreeMap::from([("disk", Some(Level::Failed)), ("net", Some(Level::Ok))])
    );
    assert_eq!(
        verdict(reply.judge(SVC, present, TRUSTED)),
        (Verdict::Unhealthy, Reason::Failed, Some(Level::Failed))
    );

    // 3. disk ok: the check first, its seq the lower, then the status.
    let back = h.set_check("disk", Level::Ok, "ok").await.unwrap();
    assert_eq!((back.level, back.raised_by), (Some(Level::Ok), None));
    let got = at_least(&log, 7).await;
    let (disk, status) = (&got[5], &got[6]);
    assert_eq!(disk.check(), Some("disk"));
    assert_eq!(disk.check_value().level, Some(Level::Ok));
    assert!(status.is_status());
    assert_eq!(
        (status.status().level, status.status().reason.code()),
        (Some(Level::Ok), 2),
        "ok, `serving` again"
    );
    assert!(
        disk.inband().seq.get() < status.inband().seq.get(),
        "the check's seq is the lower"
    );
    let reply = get_state(&rs, SVC).await;
    assert!(reply.reading(SVC).is_one_instant());
    assert_eq!(
        verdict(reply.judge(SVC, present, TRUSTED)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );

    // 4. net retired: deleted, and no status put for it. G's answer holds
    // the status and disk, and net as a tombstone.
    h.retire_check("net").await.unwrap();
    let got = at_least(&log, 8).await;
    assert!(got[7].delete && got[7].check() == Some("net"));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(snapshot(&log).len(), 8, "no status put for the retire");
    let reply = get_state(&rs, SVC).await;
    let reading = reply.reading(SVC);
    let c = reading.checks.as_ref().unwrap();
    assert_eq!(reply.status_record(SVC).level, Some(Level::Ok));
    assert_eq!(
        c.live.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["disk"]
    );
    assert_eq!(
        c.retired
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["net"],
        "a tombstone in the checks' snapshot"
    );
    assert!(reading.is_one_instant());

    // Throughout: at no delivery is S's latest status better than its
    // latest check, and each check was delivered only when it changed.
    let log = snapshot(&log);
    let (mut status, mut checks) = (None, BTreeMap::<String, Option<Level>>::new());
    for r in &log {
        if r.is_status() {
            status = r.status().level;
        } else if let Some(name) = r.check() {
            let l = (!r.delete).then(|| r.check_value().level).flatten();
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
            .map(|r| (!r.delete).then(|| r.check_value().level).flatten())
            .collect()
    };
    assert_eq!(
        of("disk"),
        [Some(Level::Ok), Some(Level::Failed), Some(Level::Ok)]
    );
    assert_eq!(of("net"), [Some(Level::Ok), None]);
    // Every put of state is 12 or 14 bytes, a record of 6: no text.
    for r in log.iter().filter(|r| !r.delete) {
        assert_eq!(r.envelope().record.len(), 6, "{r:?}");
    }
    svc.close().await.unwrap();
}

/// §3's rule beside it (§2.3): at most the contract's 64 checks, or the
/// bound the descriptor lowers it to; a check it cannot run is put
/// `unknown` with `cannot_run`, and bounds the status no more; a check put
/// in other words under the same number is not put again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_the_checks_bound_and_a_check_it_cannot_run() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/few/health.v2/**").await;

    let (mut b, h) = owner(&os, config("lab/few"), None).await;
    assert_eq!(h.max_checks(), 64);
    b.cardinality(&v2::iface(), health::CHECKS, 2).unwrap();
    assert_eq!(h.max_checks(), 2);
    matched(h.status_writer().writer()).await;
    h.set_status(Level::Ok, "serving").await.unwrap();
    h.set_check("a", Level::Failed, "refused").await.unwrap();
    h.set_check("b", Level::Ok, "ok").await.unwrap();
    assert!(matches!(
        h.set_check("c", Level::Ok, "ok").await,
        Err(zenkey::Error::Contract(_))
    ));
    let svc = b.start().await.unwrap();
    let unknown = h.set_check_unknown("a", v2::CANNOT_RUN).await.unwrap();
    assert_eq!(
        (unknown.level, unknown.raised_by, unknown.checks["a"]),
        (Some(Level::Ok), None, None),
        "unknown bounds nothing"
    );
    eventually("the owner answers", || async {
        get_state(&rs, "lab/few")
            .await
            .reading("lab/few")
            .status
            .is_some()
    })
    .await;
    let reading = get_state(&rs, "lab/few").await.reading("lab/few");
    let a = reading
        .checks
        .as_ref()
        .unwrap()
        .live
        .iter()
        .find(|e| e.id == "a")
        .unwrap()
        .record
        .unwrap();
    assert_eq!(
        (a.level, a.detail.code()),
        (None, 5),
        "unknown, `cannot_run`"
    );
    assert_eq!(reading.status_level(), Some(Read::Level(Level::Ok)));
    // Other words no table names: `out_of_range` once, a change of the
    // record, then no put for other words under the same number (§2.3).
    let b_puts = || {
        snapshot(&log)
            .iter()
            .filter(|r| r.check() == Some("b"))
            .map(|r| r.check_value().detail.code())
            .collect::<Vec<_>>()
    };
    h.set_check("b", Level::Ok, "all good").await.unwrap();
    eventually("S hears the check's out_of_range", || async {
        b_puts() == [1, 63]
    })
    .await;
    h.set_check("b", Level::Ok, "better still").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        b_puts(),
        [1, 63],
        "out_of_range again is no change of the record"
    );
    h.retire_check("a").await.unwrap();
    h.set_check("c", Level::Ok, "ok").await.unwrap();
    svc.close().await.unwrap();
}

/// What §4's steps 1 and 2 share, on the router at `ep`: `lab/ahead`, its
/// clock 2 s ahead, started with status ok, `serving`, before any put of
/// its reference.
struct Ahead {
    owner: zenoh::Session,
    hb: zenoh::Session,
    rs: zenoh::Session,
    svc: zenkey::Service,
    h: Health,
    sub: Subscription,
    log: Log,
    beats: Beats,
    _held: (zenoh::pubsub::Subscriber<()>, zenoh::pubsub::Subscriber<()>),
    runs: PathBuf,
}

impl Ahead {
    async fn start(ep: &str, name: &str, on: &On) -> Ahead {
        let owner_session = client(ep).await;
        let rs = client(ep).await;
        let hb = client(ep).await;
        let c = reader(&rs, "lab/ahead", on);
        let sub = watch_status(&c).await;
        let (s, log) = record(&rs, "zk2/lab/ahead/health.v2/**").await;
        let (hsub, beats) = heartbeats(&rs, "lab/ahead", on).await;
        let mut cfg = config("lab/ahead");
        cfg.clock_reference = Some("clock/heartbeat".to_owned());
        let runs = run_file(name);
        let (b, h) = owner_on(&owner_session, cfg, Some(&runs), on).await;
        b.minter().simulate_offset(2000);
        matched(h.status_writer().writer()).await;
        matched(h.faults_writer()).await;
        // 1. The status, before any put of the reference.
        h.set_status(Level::Ok, "serving").await.unwrap();
        let svc = b.start().await.unwrap();
        Ahead {
            owner: owner_session,
            hb,
            rs,
            svc,
            h,
            sub,
            log,
            beats,
            _held: (s, hsub),
            runs,
        }
    }

    /// Puts the reference until the owner's guard is as `ahead`; returns
    /// the instant it was seen so.
    async fn reference_until(&self, ahead: bool) -> Instant {
        eventually("the guard moves", || async {
            self.hb.put("clock/heartbeat", "tick").await.unwrap();
            self.h.is_clock_ahead() == ahead
        })
        .await;
        Instant::now()
    }

    fn faults(&self) -> Vec<Rec> {
        snapshot(&self.log)
            .into_iter()
            .filter(Rec::is_fault)
            .collect()
    }

    /// Step 2's first expectations: within 1 s of the detection, one
    /// `clock_ahead` fault, failed, its argument the offset rounded up,
    /// with no Zenoh timestamp of the owner's.
    async fn the_fault_of_the_hold(&self, detected: Instant, nth: usize) -> Rec {
        eventually("the fault arrives", || async { self.faults().len() >= nth }).await;
        let fault = self.faults().remove(nth - 1);
        assert!(
            fault.at <= detected + Duration::from_secs(1),
            "within 1 s of the detection"
        );
        let f = fault.fault();
        assert!(f.is_clock_ahead());
        assert_eq!((f.code.code(), f.level), (1, Some(Level::Failed)));
        assert!(
            matches!(f.argument, v2::Argument::Number(2 | 3)),
            "the offset measured, rounded up: {:?}",
            f.argument
        );
        assert!(f.at.is_some(), "its at, on the owner's clock");
        assert_eq!(fault.envelope().record.len(), 8, "a record of 8 bytes");
        assert_eq!(fault.payload.len(), 14, "14 bytes under a 16-bit run");
        assert_eq!(fault.payload[0], 0x0b, "form 0, a 16-bit run, tag 11");
        assert!(
            fault
                .stamp
                .is_none_or(|t| t.get_id().to_string() != self.owner.zid().to_string()),
            "its put carries no Zenoh timestamp of the owner's: {:?}",
            fault.stamp
        );
        fault
    }
}

/// §4, steps 1 to 5: a clock 2 s ahead. One `clock_ahead` fault at the
/// detection and no second past the horizon; the status not held; every
/// heartbeat from the detection on carries the ahead time and vouches for
/// the status, so S answers yes and judges healthy past the horizon; a
/// late joiner on the heartbeat key alone answers yes from its first
/// heartbeat; after the correction a heartbeat carries an instant and S
/// answers no; a second hold has a fault of its own. On the copy's 6 s
/// horizon (the module doc).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_clock_ahead_answered_by_a_heartbeat() {
    clock_ahead_answered(On::short()).await;
}

/// §4, steps 1 to 5, at the standard contract: 65 s, 35 s and 35 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §4 waits 135 s at the contract's 60 s horizon"]
async fn s4_clock_ahead_answered_by_a_heartbeat_at_60_s() {
    clock_ahead_answered(On::standard()).await;
}

/// §4, steps 1 to 5, on `on`'s revision.
async fn clock_ahead_answered(on: On) {
    let (_r1, ep) = router(None).await;
    let a = Ahead::start(&ep, "s4", &on).await;
    let ls = client(&ep).await;

    // 1. S receives the status. Each heartbeat it reads carries an
    // instant, and it answers no, `confirmed`.
    at_least(&a.log, 1).await;
    assert!(snapshot(&a.log)[0].is_status());
    eventually("the first heartbeat", || async {
        !all_beats(&a.beats).is_empty()
    })
    .await;
    let before = all_beats(&a.beats);
    assert!(before.iter().all(|b| b.hb.time.is_instant()));
    assert_eq!(
        clock(&snapshot(&a.log), &before),
        (Answer::No, ClockReason::Confirmed)
    );

    // 2. The reference until the owner reports itself ahead; then status
    // ok with the reason `fallback`; S listens past the horizon (65 s at
    // the standard's) from the detection.
    let detected = a.reference_until(true).await;
    a.the_fault_of_the_hold(detected, 1).await;
    let put =
        a.h.set_status(Level::Ok, "fallback")
            .await
            .expect("the guard does not hold the status");
    assert_eq!(put.word.map(v2::Word::code), Some(6));
    eventually("S hears the status put while held", || async {
        snapshot(&a.log)
            .iter()
            .any(|r| r.is_status() && r.at > detected && r.status().reason.code() == 6)
    })
    .await;

    // 3. L subscribes half a horizon and 5 s (35 s at the standard's)
    // after the detection, to the heartbeat key alone, and listens half a
    // horizon.
    tokio::time::sleep_until((detected + on.half_past()).into()).await;
    let (_l, late) = heartbeats(&ls, "lab/ahead", &on).await;
    let joined = Instant::now();

    tokio::time::sleep_until((detected + on.past()).into()).await;
    let held = snapshot(&a.log);
    assert_eq!(
        a.faults().len(),
        1,
        "no second clock_ahead fault past the horizon (§2.5)"
    );
    let during = beats_after(&a.beats, detected + JITTER);
    assert!(during.len() >= 2, "heartbeats go on: {}", during.len());
    assert!(
        during
            .iter()
            .all(|b| vouches(b) && b.hb.time == Time::Ahead),
        "each carries the ahead time, never all ones, and vouches for the status: {:?}",
        during.iter().map(|b| b.hb.time).collect::<Vec<_>>()
    );
    let first_ahead = beats_after(&a.beats, detected)
        .into_iter()
        .find(|b| b.hb.time == Time::Ahead)
        .expect("a heartbeat with the ahead time");
    assert!(
        first_ahead.at <= detected + on.beat() + JITTER,
        "its first within half a horizon of the detection"
    );
    assert_eq!(
        clock(&held, &all_beats(&a.beats)),
        (Answer::Yes, ClockReason::Ahead)
    );
    let tokens = tokens_of(&a.rs, "lab/ahead").await;
    assert!(
        tokens.iter().any(|k| matches!(k, ZkKey::Instance { .. }))
            && tokens.iter().any(is_health_token),
        "the owner's tokens stay present: {tokens:?}"
    );
    let present = presence_of(&a.rs, "lab/ahead").await;
    assert_eq!(
        verdict(judge_s(&a.sub, &held, present, &on)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok)),
        "the heartbeats confirmed the status past the horizon"
    );
    // L: no fault, published before it joined; yes from its first
    // heartbeat, within half a horizon.
    let heard = all_beats(&late);
    let first = heard.first().expect("L read a heartbeat");
    assert!(
        first.at <= joined + on.beat() + JITTER,
        "within half a horizon"
    );
    assert_eq!(
        clock(&[], &heard[..1]),
        (Answer::Yes, ClockReason::Ahead),
        "a late joiner, from the first heartbeat it reads"
    );

    // 4. The clock set right, the reference on: within half a horizon a
    // heartbeat whose time is an instant again, and no clock_ahead after
    // step 2's.
    a.svc.minter().simulate_offset(0);
    let released = a.reference_until(false).await;
    tokio::time::sleep_until((released + on.half_past()).into()).await;
    let again = beats_after(&a.beats, released)
        .into_iter()
        .find(|b| b.hb.time.is_instant())
        .expect("a heartbeat whose time is an instant again");
    assert!(
        again.at <= released + on.beat() + JITTER,
        "within half a horizon of the correction"
    );
    assert_eq!(a.faults().len(), 1, "no clock_ahead fault after step 2's");
    let after = snapshot(&a.log);
    assert_eq!(
        verdict(judge_s(&a.sub, &after, present, &on)),
        (Verdict::Healthy, Reason::Ok, Some(Level::Ok))
    );
    assert_eq!(
        clock(&after, &all_beats(&a.beats)),
        (Answer::No, ClockReason::Confirmed)
    );

    // 5. 2 s ahead again: a new hold, with a fault of its own, and the
    // heartbeats carry the ahead time again.
    a.svc.minter().simulate_offset(2000);
    let detected = a.reference_until(true).await;
    a.the_fault_of_the_hold(detected, 2).await;
    tokio::time::sleep_until((detected + on.half_past()).into()).await;
    assert_eq!(a.faults().len(), 2, "one more, and one only");
    let during = beats_after(&a.beats, detected + JITTER);
    assert!(
        !during.is_empty() && during.iter().all(|b| b.hb.time == Time::Ahead),
        "its heartbeats carry the ahead time again"
    );
    a.svc.close().await.unwrap();
    let _ = std::fs::remove_file(&a.runs);
}

/// §4, step 6: steps 1 and 2 with R1 under
/// `timestamping.drop_future_timestamp`. The fault carries no Zenoh
/// timestamp of the owner's, so R1 has no future-dated put to drop: S
/// receives it, the status put while ahead and the heartbeats, and answers
/// yes. On the copy's 6 s horizon (the module doc), so S listens 8 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_router_that_drops_future_stamps_drops_nothing() {
    let on = On::short();
    let (_r2, ep) = router_with(None, &[("timestamping/drop_future_timestamp", "true")]).await;
    let a = Ahead::start(&ep, "s4-drop", &on).await;
    at_least(&a.log, 1).await;
    let detected = a.reference_until(true).await;
    a.the_fault_of_the_hold(detected, 1).await;
    a.h.set_status(Level::Ok, "fallback")
        .await
        .expect("the guard does not hold the status");
    tokio::time::sleep_until((detected + on.half_past()).into()).await;
    let heard = snapshot(&a.log);
    assert!(
        heard
            .iter()
            .any(|r| { r.is_status() && r.at > detected && r.status().reason.code() == 6 }),
        "S receives the status put while the guard holds: {heard:#?}"
    );
    assert_eq!(a.faults().len(), 1, "the fault, once");
    let during = beats_after(&a.beats, detected + JITTER);
    assert!(
        !during.is_empty()
            && during
                .iter()
                .all(|b| vouches(b) && b.hb.time == Time::Ahead),
        "the heartbeats as in step 2"
    );
    assert_eq!(
        clock(&heard, &all_beats(&a.beats)),
        (Answer::Yes, ClockReason::Ahead)
    );
    a.svc.close().await.unwrap();
    let _ = std::fs::remove_file(&a.runs);
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

/// §5: 100 services with `health.v2` and `freshness.v2` tokenless: 200
/// tokens, none of them `health.v2`'s or `freshness.v2`'s, and every
/// service found by its descriptor and judged healthy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_tokenless_set_of_100() {
    const N: usize = 100;
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let host = client(&ep).await;
    let mut services = Vec::new();
    for i in 0..N {
        let cfg = config(&format!("p5/dev{i}"))
            .tokenless(v2::iface())
            .tokenless(zenkey::heartbeat::iface());
        let (mut b, held) = nav_builder(&host, cfg).await;
        let h = b.health_v2().await.unwrap();
        h.set_status(Level::Ok, "serving").await.unwrap();
        services.push((b.start().await.unwrap(), held));
    }

    // The tool lists the presence domain, then health.v2's tokens.
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
    assert_eq!(count(&is_freshness_token), 0);
    assert!(
        presence::liveliness_keys(&tool, "zk2/p5/*/@zk/alive/health.v2/**", T)
            .await
            .unwrap()
            .is_empty(),
        "the second read finds nothing"
    );

    // Each descriptor, then each listed service's health.v2/state/**.
    let mut healthy = 0;
    for k in &all {
        let ZkKey::Instance { addr: a, instance } = k else {
            continue;
        };
        let address = a.to_string();
        let d = described(&tool, &address, instance).await;
        let (major, present) = listed(&d);
        assert_eq!(major, Some(Major::V2), "{a}");
        assert_eq!(
            present,
            Presence::Present(Listing::Listed { token: false }),
            "{a}: \"token\": false"
        );
        let got = get_state(&tool, &address).await;
        assert_eq!(
            verdict(got.judge(&address, present, TRUSTED)),
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

/// The owners' log, as §9 reads it: every line `zenkey::health` writes.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buffer {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn logs() -> &'static Buffer {
    static LOGS: std::sync::OnceLock<Buffer> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Buffer::default();
        let w = buffer.clone();
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(move || w.clone())
                .with_ansi(false)
                .without_time()
                .with_env_filter(tracing_subscriber::EnvFilter::new("zenkey::health=trace"))
                .finish(),
        );
        buffer
    })
}

/// A tool's tables for an owner (§2.10): the tables of each revision the
/// owner's descriptor names, among those the tool holds. A revision it
/// holds that the descriptor does not name adds none.
fn tables_of(d: &Descriptor, held: &[zenkey::Implementation]) -> v2::InstanceTables {
    let named: Vec<v2::Tables> = held
        .iter()
        .filter(|i| {
            let fp = format!("sha256:{}", i.fingerprint().hex());
            d.interfaces
                .iter()
                .any(|e| e.iface == i.iface().to_string() && e.contract == fp)
        })
        .map(|i| v2::Tables::of(i.contract()))
        .collect();
    v2::Tables::of_instance(&named)
}

/// §9: words by table. An owner writes its own reason and its own code as
/// 32, with no byte of text, and a reason its table does not hold as
/// `out_of_range`, the words in its log; an owner with no table writes
/// `out_of_range` too. A tool holding the revision the owner's descriptor
/// names shows the names; one holding another revision shows the number,
/// never the name its own revision has there. A word plays no part in the
/// verdict.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_words_by_table() {
    let logs = logs();
    let (_r1, ep) = router(None).await;
    let tc_session = client(&ep).await;
    let plain_session = client(&ep).await;
    let ts = client(&ep).await;
    let (_s, log) = record(&ts, "zk2/lab/*/health.v2/**").await;
    let tc_v1 = imp("health/tc.v1");
    let earlier = imp("health/tc.v1-earlier");
    assert_ne!(tc_v1.fingerprint().hex(), earlier.fingerprint().hex());

    // lab/tc implements health.v2, then a contract of its own that lists
    // health.v2 and declares two tables on [defaults].
    let (mut b, tc) = owner(&tc_session, config("lab/tc"), None).await;
    assert_eq!(tc.tables(), v2::InstanceTables::default(), "none yet");
    b.implement(tc_v1.clone()).unwrap();
    b.expose(&tc_v1.iface().clone(), "state/qdisc").unwrap();
    let own = tc.tables();
    assert_eq!(
        own.own(v2::Kind::Reason),
        Some(&["tc_netns_gone".to_owned(), "tc_qdisc_missing".to_owned()][..])
    );
    assert_eq!(own.own(v2::Kind::Detail), None);
    assert_eq!(
        own.own(v2::Kind::Code),
        Some(&["tc_netns_restarted".to_owned()][..])
    );
    matched(tc.status_writer().writer()).await;
    matched(tc.faults_writer()).await;
    let tc_svc = b.start().await.unwrap();
    let (pb, plain) = owner(&plain_session, config("lab/plain"), None).await;
    matched(plain.status_writer().writer()).await;
    let plain_svc = pb.start().await.unwrap();
    let of = |address: &str| {
        let prefix = format!("zk2/{address}/");
        snapshot(&log)
            .into_iter()
            .filter(|r| r.key.starts_with(&prefix))
            .collect::<Vec<_>>()
    };

    // 1. Its own reason, then a fault with its own code and the argument 3.
    let put = tc
        .set_status(Level::Degraded, "tc_netns_gone")
        .await
        .unwrap();
    assert_eq!(put.word.map(v2::Word::code), Some(32));
    tc.fault_argument("tc_netns_restarted", Level::Degraded, Some(3))
        .await
        .unwrap();
    eventually("S hears both", || async {
        let heard = of("lab/tc");
        heard.iter().any(Rec::is_fault)
            && heard
                .iter()
                .any(|r| r.is_status() && r.status().level == Some(Level::Degraded))
    })
    .await;
    let heard = of("lab/tc");
    let own_status = heard
        .iter()
        .rfind(|r| r.is_status())
        .cloned()
        .expect("the status");
    let own_fault = heard
        .iter()
        .find(|r| r.is_fault())
        .cloned()
        .expect("the fault");
    assert_eq!(own_status.status().reason.code(), 32, "the reason 32");
    assert_eq!(
        (own_fault.fault().code.code(), own_fault.fault().argument),
        (32, v2::Argument::Number(3)),
        "the code 32 and the argument 3"
    );
    // Neither payload holds a byte of text: CH, a 32-bit run, the seq, and
    // a record of 6 or 8 bytes.
    assert_eq!(own_status.payload.len(), 14);
    assert_eq!(own_status.payload[0], 0x13, "form 0, a 32-bit run, tag 3");
    assert_eq!(own_fault.payload.len(), 16);
    assert_eq!(own_fault.payload[0], 0x1b, "form 0, a 32-bit run, tag 11");
    for r in [&own_status, &own_fault] {
        assert!(
            !r.payload.windows(3).any(|w| w == b"tc_"),
            "no byte of text: {:02x?}",
            r.payload
        );
    }

    // 2. A reason its table does not hold: `out_of_range`, the code 63,
    // and the words in its log.
    let words = "kernel too old for this qdisc";
    let put = tc.set_status(Level::Degraded, words).await.unwrap();
    assert_eq!(put.word, Some(v2::Word::OUT_OF_RANGE));
    assert_eq!(put.reason, words, "the owner holds its words");
    eventually("S hears it", || async {
        of("lab/tc")
            .iter()
            .any(|r| r.is_status() && r.status().reason.is_out_of_range())
    })
    .await;
    let unnamed = of("lab/tc")
        .into_iter()
        .rfind(Rec::is_status)
        .expect("the status");
    assert_eq!(unnamed.status().reason.code(), 63);
    assert_eq!(unnamed.payload.len(), 14, "no byte of text here either");
    let text = logs.text();
    assert!(
        text.contains(words) && text.contains("lab/tc"),
        "the words are in lab/tc's log:\n{text}"
    );

    // 3. lab/plain has no table: the same reason is `out_of_range` too.
    assert_eq!(plain.tables(), v2::InstanceTables::default());
    let put = plain
        .set_status(Level::Degraded, "tc_netns_gone")
        .await
        .unwrap();
    assert_eq!(put.word, Some(v2::Word::OUT_OF_RANGE));
    eventually("S hears lab/plain", || async {
        of("lab/plain")
            .iter()
            .any(|r| r.is_status() && r.status().level == Some(Level::Degraded))
    })
    .await;
    let plain_status = of("lab/plain")
        .into_iter()
        .rfind(Rec::is_status)
        .expect("the status");
    assert!(plain_status.status().reason.is_out_of_range());

    // 4. T holds the revision lab/tc's descriptor names; T2 an earlier
    // one alone. Both read both owners.
    let tc_d = descriptor_of(&ts, "lab/tc").await;
    let plain_d = descriptor_of(&ts, "lab/plain").await;
    let t = [tc_v1.clone()];
    let t2 = [earlier.clone()];
    let reason = |d: &Descriptor, held: &[zenkey::Implementation], word: v2::Word| {
        let tables = tables_of(d, held);
        match v2::read_word(v2::Kind::Reason, word, tables.own(v2::Kind::Reason)) {
            v2::Named::None => "(none)".to_owned(),
            v2::Named::Profile(n) => n.to_owned(),
            v2::Named::Service(n) => n.to_owned(),
            v2::Named::Number(n) => n.to_string(),
            v2::Named::OutOfRange => "(out of range)".to_owned(),
        }
    };
    let code = |d: &Descriptor, held: &[zenkey::Implementation], word: v2::Word| {
        let tables = tables_of(d, held);
        match v2::read_word(v2::Kind::Code, word, tables.own(v2::Kind::Code)) {
            v2::Named::Service(n) => n.to_owned(),
            v2::Named::Number(n) => n.to_string(),
            other => format!("{other:?}"),
        }
    };
    let (r1, f1, r2) = (
        own_status.status().reason,
        own_fault.fault(),
        unnamed.status().reason,
    );
    // T.
    assert_eq!(reason(&tc_d, &t, r1), "tc_netns_gone");
    assert_eq!(code(&tc_d, &t, f1.code), "tc_netns_restarted");
    assert_eq!(f1.argument, v2::Argument::Number(3));
    assert_eq!(
        reason(&tc_d, &t, r2),
        "(out of range)",
        "a word no table names: never a name, and never no word"
    );
    // T2 does not hold the revision the descriptor names: the number,
    // never `tc_qdisc_missing`, the name its own revision has at index 0.
    assert_eq!(
        v2::Tables::of(earlier.contract()).reasons.as_deref(),
        Some(&["tc_qdisc_missing".to_owned(), "tc_netns_gone".to_owned()][..])
    );
    assert_eq!(reason(&tc_d, &t2, r1), "32");
    assert_eq!(code(&tc_d, &t2, f1.code), "32");
    assert_eq!(reason(&tc_d, &t2, r2), "(out of range)");
    // Both show lab/plain's reason as a word no table names.
    for held in [&t[..], &t2[..]] {
        assert_eq!(
            reason(&plain_d, held, plain_status.status().reason),
            "(out of range)"
        );
    }
    // Each judges both services unhealthy, degraded: by GET, the level
    // alone.
    for address in ["lab/tc", "lab/plain"] {
        let present = presence_of(&ts, address).await;
        let got = get_state(&ts, address).await;
        assert_eq!(
            verdict(got.judge(address, present, TRUSTED)),
            (Verdict::Unhealthy, Reason::Degraded, Some(Level::Degraded)),
            "{address}: a word plays no part in the verdict"
        );
    }
    // The runtime refuses the profile's own code from an application, and
    // an argument that is not_available's code.
    assert!(matches!(
        tc.fault_argument(health::CLOCK_AHEAD, Level::Failed, Some(2))
            .await,
        Err(zenkey::Error::Contract(_))
    ));
    assert!(matches!(
        tc.fault_argument("tc_netns_restarted", Level::Failed, Some(u16::MAX))
            .await,
        Err(zenkey::Error::Contract(_))
    ));
    tc_svc.close().await.unwrap();
    plain_svc.close().await.unwrap();
}

/// §2.10 beside §9: a contract implemented after a status was set brings
/// the table that names its reason, and `start` puts the status again
/// under its number; two contracts whose tables differ leave the instance
/// none, and its owner writes no word from 32 to 62.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_an_instances_table_is_its_contracts_one_table() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let ds = client(&ep).await;
    let rs = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/*/health.v2/state/status").await;
    let tc_v1 = imp("health/tc.v1");

    // The status first, the table after: `start` puts it again.
    let (mut b, h) = owner(&os, config("lab/late"), None).await;
    matched(h.status_writer().writer()).await;
    let early = h
        .set_status(Level::Degraded, "tc_qdisc_missing")
        .await
        .unwrap();
    assert_eq!(early.word, Some(v2::Word::OUT_OF_RANGE), "no table yet");
    b.implement(tc_v1.clone()).unwrap();
    b.expose(&tc_v1.iface().clone(), "state/qdisc").unwrap();
    let svc = b.start().await.unwrap();
    assert_eq!(h.status().unwrap().word.map(v2::Word::code), Some(33));
    let heard = at_least(&log, 2).await;
    assert_eq!(
        heard
            .iter()
            .map(|r| r.status().reason.code())
            .collect::<Vec<_>>(),
        [63, 33],
        "out_of_range, then the word its table names"
    );
    assert_eq!(
        heard[0].status().since,
        heard[1].status().since,
        "a change of reason alone keeps since (§2.3)"
    );
    svc.close().await.unwrap();

    // Two revisions' worth of tables that differ: none.
    let mut text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/contracts/health/tc.v1-earlier.toml"),
    )
    .unwrap();
    text = text.replace("name  = \"tc\"", "name  = \"shaper\"");
    let dir = std::env::temp_dir().join(format!("zk2-health-v2-differ-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shaper.v1.toml");
    std::fs::write(&path, text).unwrap();
    let l = zenkey::model::contract::load_path(&path);
    let shaper = zenkey::Implementation::new(
        l.contract
            .unwrap_or_else(|| panic!("shaper.v1 does not load:\n{}", l.report)),
    );
    let (mut b, h) = owner(&ds, config("lab/differ"), None).await;
    b.implement(tc_v1.clone()).unwrap();
    assert!(h.tables().own(v2::Kind::Reason).is_some());
    b.implement(shaper).unwrap();
    assert_eq!(h.tables().reasons, v2::Instance::Differ);
    assert_eq!(h.tables().own(v2::Kind::Reason), None);
    assert!(
        h.tables().own(v2::Kind::Code).is_some(),
        "the codes' table is one contract's alone"
    );
    let put = h
        .set_status(Level::Degraded, "tc_netns_gone")
        .await
        .unwrap();
    assert_eq!(
        put.word,
        Some(v2::Word::OUT_OF_RANGE),
        "no word from 32 to 62 where two tables differ"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// No section of `scenarios-v2.md` runs an owner under a catalog (zk2py's
/// round 120 mend): its conventions give none, so every record's times
/// count from 0, and an owner that wrote them from 0 under a catalog would
/// pass §1 to §9. Here `lab/cat` runs under a catalog whose epoch is
/// 2026-01-01 (core §8.6). Its status's `since`, its check's `at` and its
/// fault's `at` are whole seconds from that epoch (§2.3, core §7.6): read
/// under it, each is this host's clock to the second; read under 0, each
/// is that epoch early, in 1970. Its snapshots' time counts from it too,
/// and its descriptor names the catalog, so a reader knows which epoch to
/// read by.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_a_catalog_every_time_counts_from_its_epoch() {
    use zenkey::model::catalog::Catalog;
    const CAT: &str = "lab/cat";
    const E: i64 = 1_767_225_600;
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let (_s, log) = record(&rs, "zk2/lab/cat/health.v2/**").await;
    let catalog = Catalog::new(E).unwrap();
    let id = catalog.id();
    let mut b = ServiceBuilder::new(&os, config(CAT));
    b.catalog(catalog).unwrap();
    let h = b.health_v2().await.unwrap();
    assert_eq!(b.deployment_epoch(), E);
    matched(h.status_writer().writer()).await;
    matched(h.faults_writer()).await;
    let before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    h.set_status(Level::Ok, "serving").await.unwrap();
    h.set_check("disk", Level::Ok, "ok").await.unwrap();
    let svc = b.start().await.unwrap();
    h.fault_argument("disk_full", Level::Degraded, Some(7))
        .await
        .unwrap();
    let got = at_least(&log, 3).await;
    let after = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let now = |t: Option<i64>, what: &str| {
        let t = t.unwrap_or_else(|| panic!("{what}: a time the catalog's epoch codes"));
        assert!(
            (before..=after).contains(&t),
            "{what}: {t} is this host's clock, {before} to {after}"
        );
    };
    let envelope = |r: &Rec| {
        let e = r.envelope();
        (e.tag.get(), e.record.to_vec())
    };
    let status = got.iter().find(|r| r.is_status()).expect("the status");
    let (tag, record) = envelope(status);
    now(
        v2::Status::read(tag, &record, E).unwrap().since,
        "the status's since",
    );
    assert_eq!(
        v2::Status::read(tag, &record, 0).unwrap().since,
        v2::Status::read(tag, &record, E)
            .unwrap()
            .since
            .map(|t| t - E),
        "read under 0, the catalog's epoch early"
    );
    let disk = got
        .iter()
        .find(|r| r.check() == Some("disk"))
        .expect("disk");
    let (tag, record) = envelope(disk);
    now(
        v2::Check::read(tag, &record, E).unwrap().at,
        "the check's at",
    );
    let fault = got.iter().find(|r| r.is_fault()).expect("the fault");
    let (tag, record) = envelope(fault);
    let f = v2::Fault::read(tag, &record, E).unwrap();
    assert_eq!(
        (f.code, f.argument),
        (v2::Word::OUT_OF_RANGE, v2::Argument::Number(7))
    );
    now(f.at, "the fault's at");

    // A late joiner's GET: the snapshots' time, and the entries' records,
    // under the epoch the descriptor names.
    let d = descriptor_of(&rs, CAT).await;
    assert_eq!(
        d.stated_catalog(),
        Some(id),
        "its descriptor names the catalog"
    );
    let answer = get_state(&rs, CAT).await;
    let replies: Vec<(String, Vec<u8>)> = answer
        .of(CAT)
        .into_iter()
        .map(|s| {
            (
                s.key_expr().as_str().to_owned(),
                s.payload().to_bytes().into_owned(),
            )
        })
        .collect();
    let reading = m2::Reading::of(replies.iter().map(|(k, p)| (k.as_str(), p.as_slice())), E);
    let snapshot = reading.status.as_ref().expect("the status's snapshot");
    let ms = snapshot
        .heard
        .time
        .unix_ms(E)
        .expect("a snapshot's time, known") as i64;
    now(Some(ms / 1000), "the snapshot's time");
    match &snapshot.entry {
        StatusEntry::Live(e) => now(
            e.record.as_ref().expect("a record").since,
            "the snapshot's status",
        ),
        other => panic!("no live status: {other:?}"),
    }
    assert!(reading.is_one_instant());
    svc.close().await.unwrap();
}
