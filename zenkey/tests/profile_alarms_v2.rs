//! `spec/profiles/alarms/scenarios-v2.md` §1–§7, §9 and §12 (`alarms.v2`,
//! #984's S2), one test per section, named after it. §8, §10 and §11 are a
//! tool's, and come with the readers (#984's S3).
//!
//! Common setup, the text's: R1 is a router with timestamping on, a
//! router's default. The owner `lab/svc` implements `alarms.v2` through
//! [`ServiceBuilder::alarms_v2`], as a client of R1, beside its own
//! contract `tc.v1` (`tests/contracts/alarms/tc.v1.toml`), which lists
//! `alarms.v2` in `uses` and declares the table T1 of
//! `conformance/v2/tables.json`. No owner is given a catalog: the
//! deployment epoch is 0, so a record's `at` is Unix seconds. S records
//! every delivery under the owner's `alarms.v2/**` as it arrived (arrival on
//! this host's monotonic clock, key, kind, the put's whole payload), with
//! its instance tokens, and reads each heartbeat covering `alarms.v2` raw.
//! G GETs `zk2/lab/svc/alarms.v2/state/alarms/**` as core S4 says (target
//! `All`, consolidation `None`) and reads its snapshot as
//! `zenkey_model::alarms::v2::Seed` does. "Reads" is
//! `zenkey_model::alarms::v2::Track` over S's deliveries of a slot, the
//! alarm named by T1, and every verdict is `zenkey_model::alarms::v2`'s
//! judges'. G and the tools take the deployment's word for their clock.
//!
//! **A 16-bit run.** The Expected sizes are a put's payload under a 16-bit
//! run (`scenarios-v2.md`, "Sizes"): every owner here takes its run from a
//! run file of its own.
//!
//! **The horizon.** §2 and §6 wait 905 s at the contract's 900 s, and §3
//! reads the owner's next heartbeat, 427.5 s apart there: they run in the
//! slow tier, `#[ignore]`d. Their fast variants run the same owner code on
//! a test's copy of the standard contract whose `freshness.ttl_s` is 4,
//! never published, as the scenarios' conventions allow:
//! [`ServiceBuilder::alarms_v2_with`] takes the confirmation period and
//! §2.5's refresh window from it, its heartbeat every 1.9 s and its window
//! 2 s, and S reads that heartbeat with the copy.
//!
//! **§1's and §9's tools.** T and T2 read the owners with the model's
//! judges over what they GET and hear; the fleet's and zenctl's reading is
//! S3's.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{Beats, T, addr, client, config, eventually, eventually_some, imp, router};
use zenkey::alarms::v2::{self as av2, Act, Alarms, Condition, FileRecord, Found, Source, Startup};
use zenkey::alarms::{self as av1, Severity};
use zenkey::consumer::{Consumer, Subscription};
use zenkey::model::alarms::v2::{self as m2, Argument, Record, Rule, RuleIndex};
use zenkey::model::alarms::{
    self as am, Answer, Judged, Listing, Major, Presence, Reason, SetVerdict, State, Transition,
};
use zenkey::model::collection::{Confirmation, Page};
use zenkey::model::descriptor::Descriptor;
use zenkey::model::freshness::{self as fm, ClockTrust, DEFAULT_DELTA, Observation};
use zenkey::model::grammar::{InstanceId, ZkKey};
use zenkey::model::wire::{Envelope, Form, Run, Seq, Stamp, Tag};
use zenkey::run::FileRunStore;
use zenkey::state::Current;
use zenkey::{Implementation, ServiceBuilder, presence};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

/// The scenarios' jitter allowance on loopback.
const JITTER: Duration = Duration::from_millis(200);

/// The deployment's word for a reader's clock (`freshness.v1` §2.6).
const TRUSTED: ClockTrust = ClockTrust::Trusted {
    delta: DEFAULT_DELTA,
};

/// The scenarios' deployment epoch: no owner is given a catalog.
const EPOCH: i64 = 0;

/// The owner of `scenarios-v2.md`'s conventions.
const SVC: &str = "lab/svc";

/// The alarms' alert keys (the conventions' "The alarms").
const A_KEY: &str = "a659f813308ad1da";
const B_KEY: &str = "abb089906038056a";
const C_KEY: &str = "f3ba0819e7b57762";
const H_KEY: &str = "335f24192ff5d0d4";
const D_KEY: &str = "719d159937e54eb8";

/// A: `link_down`, `peer` r2, `port` eth0, rule 1, `labels` 2.
fn alarm_a(severity: Severity) -> Condition {
    Condition::new("link_down", severity, "Link to r2 on eth0 is down")
        .label("peer", "r2")
        .label("port", "eth0")
}

/// B: `door_open`, rule 3, no label.
fn alarm_b(severity: Severity) -> Condition {
    Condition::new("door_open", severity, "Enclosure door open")
}

/// C: `link_down`, `peer` r3, `port` eth1, rule 1, `labels` 5.
fn alarm_c() -> Condition {
    Condition::new("link_down", Severity::Warning, "Link to r3 on eth1 is down")
        .label("peer", "r3")
        .label("port", "eth1")
}

/// H: `hot`, rule 0, with an argument.
fn alarm_h(severity: Severity, reading: &str) -> Condition {
    Condition::new("hot", severity, format!("Temperature {reading} C")).reading(reading)
}

/// D (§5) and W (§9): `disk_full`, `mount` /var, which T1 does not hold.
fn alarm_d() -> Condition {
    Condition::new("disk_full", Severity::Warning, "/var is full").label("mount", "/var")
}

/// `tc.v1` with T1; §5's later revision, T1 with `disk_full` at 6; §9's
/// earlier revision, which T2 holds alone.
fn tc() -> Implementation {
    imp("alarms/tc.v1")
}

fn tc_later() -> Implementation {
    imp("alarms/tc.v1-later")
}

fn tc_earlier() -> Implementation {
    imp("alarms/tc.v1-earlier")
}

fn table_of(i: &Implementation) -> Vec<Rule> {
    m2::table_of(i.contract()).expect("a table of rules")
}

/// A test's copy of the standard contract, its `freshness.ttl_s` set to
/// `ttl_s`: never published, never in `spec/`.
fn fast(ttl_s: u64) -> Implementation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/alarms/alarms.v2.toml");
    let standard = std::fs::read_to_string(&path).unwrap();
    let text = standard.replace(
        "\"freshness.ttl_s\" = 900",
        &format!("\"freshness.ttl_s\" = {ttl_s}"),
    );
    assert_ne!(text, standard, "the horizon is the standard's 900");
    let l = zenkey::model::contract::load_str(&text, path.parent().unwrap(), None);
    Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

/// The horizon of `imp`'s records.
fn horizon(imp: &Implementation) -> fm::Horizon {
    fm::horizon_of(imp.resource(av2::ALARMS).unwrap())
}

/// A run file of this test's own, for an owner on a 16-bit run.
fn run_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.run"))
}

/// An owner at `address` implementing `imp` from `startup`, not started,
/// on a 16-bit run from `runs`, beside `table`'s contract, implemented
/// first so that its table is in force (§4).
async fn owner(
    s: &zenoh::Session,
    address: &str,
    imp: Implementation,
    startup: Startup,
    runs: &Path,
    table: Option<Implementation>,
) -> (ServiceBuilder, Alarms) {
    let mut b = ServiceBuilder::new(s, config(address));
    b.run_store(Arc::new(FileRunStore::new(runs))).unwrap();
    if let Some(t) = table {
        let iface = t.iface().clone();
        b.implement(t).unwrap();
        b.expose(&iface, "state/qdisc").unwrap();
    }
    let a = b.alarms_v2_with(imp, startup).await.unwrap();
    (b, a)
}

/// One delivery, as S records it: the put's payload whole, its envelope
/// included.
#[derive(Debug, Clone)]
struct Rec {
    at: Instant,
    key: String,
    delete: bool,
    payload: Vec<u8>,
}

impl Rec {
    /// The slot of a record's key.
    fn slot(&self) -> Option<u8> {
        m2::slot_of(&self.key).map(|k| k.slot)
    }

    /// The slot of a words' key.
    fn words_slot(&self) -> Option<u8> {
        m2::words_slot_of(&self.key).map(|k| k.slot)
    }

    fn is_instance(&self) -> bool {
        self.key.contains("/@zk/instance/")
    }

    fn envelope(&self) -> Envelope<'_> {
        let e = Envelope::parse(&self.payload).expect("a zk2c envelope");
        assert_eq!(e.form, Form::Mutation, "a put of a record: {self:?}");
        e
    }

    fn stamp(&self) -> Stamp {
        self.envelope().stamp
    }

    fn record(&self) -> Record {
        let e = self.envelope();
        Record::read(e.tag.get(), e.record, EPOCH).expect("an alarms.v2 record")
    }

    fn words(&self) -> m2::Words {
        m2::Words::decode(self.envelope().record).expect("an alarms.v2.Words document")
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
    });
}

/// S's recording subscriber on `address`'s `alarms.v2/**`, and on its
/// instance tokens, into one log in arrival order.
struct Recorder {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Log,
}

async fn record(s: &zenoh::Session, address: &str) -> Recorder {
    record_on(s, address, "alarms.v2").await
}

async fn record_on(s: &zenoh::Session, address: &str, iface: &str) -> Recorder {
    let log: Log = Arc::default();
    let w = Arc::clone(&log);
    let data = s
        .declare_subscriber(format!("zk2/{address}/{iface}/**"))
        .callback(move |x: Sample| push(&w, &x))
        .await
        .unwrap();
    let w = Arc::clone(&log);
    let tokens = s
        .liveliness()
        .declare_subscriber(format!("zk2/{address}/@zk/instance/*"))
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

/// S's deliveries of the record of `slot`.
fn on_slot(log: &[Rec], slot: u8) -> Vec<Rec> {
    log.iter()
        .filter(|r| r.slot() == Some(slot))
        .cloned()
        .collect()
}

/// S's deliveries of records, any slot.
fn records(log: &[Rec]) -> Vec<Rec> {
    log.iter().filter(|r| r.slot().is_some()).cloned().collect()
}

/// S's deliveries of words.
fn words_of(log: &[Rec]) -> Vec<Rec> {
    log.iter()
        .filter(|r| r.words_slot().is_some())
        .cloned()
        .collect()
}

/// Waits until S holds at least `n` deliveries of records; returns them.
async fn at_least(log: &Log, n: usize) -> Vec<Rec> {
    eventually(&format!("{n} records"), || async {
        records(&log.lock().unwrap()).len() >= n
    })
    .await;
    records(&snapshot(log))
}

/// Waits for the moment S sees a new instance token, past the first `n`
/// deliveries in its log, polling every millisecond: "before the new
/// instance token" is observed by a GET made then (§2.9).
async fn at_new_token(log: &Log, n: usize) {
    let deadline = Instant::now() + common::SETTLE;
    while !snapshot(log)[n..].iter().any(Rec::is_instance) {
        assert!(Instant::now() < deadline, "never: the new instance token");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// The record a reader holds of `r` put under `stamp`, named by `table`
/// (§2.6).
fn held(r: &Record, stamp: Stamp, table: Option<&[Rule]>) -> m2::Held {
    m2::Held {
        record: *r,
        run: stamp.run,
        alert_key: r.name(table).alert_key(),
    }
}

/// S reads its deliveries of `slot`, in order, by identity (§2.6).
fn reads(log: &[Rec], slot: u8, table: Option<&[Rule]>) -> Vec<m2::Applied> {
    let mut t = m2::Track::default();
    on_slot(log, slot)
        .iter()
        .map(|r| {
            if r.delete {
                return t.apply(m2::Sample::Delete);
            }
            let e = r.envelope();
            match Record::read(e.tag.get(), e.record, EPOCH) {
                Ok(rec) => t.apply(m2::Sample::Put(held(&rec, e.stamp, table))),
                Err(_) => t.apply(m2::Sample::Unreadable),
            }
        })
        .collect()
}

fn transitions(applied: &[m2::Applied]) -> Vec<Transition> {
    applied.iter().map(|a| a.reads).collect()
}

/// `address`'s heartbeats covering `alarms.v2`, read raw with `imp`.
async fn heartbeats(
    s: &zenoh::Session,
    address: &str,
    imp: &Implementation,
) -> (zenoh::pubsub::Subscriber<()>, Beats) {
    let layout = zenkey::model::heartbeat::Layout::of(imp.contract());
    common::heartbeats(
        s,
        &format!("zk2/{address}/freshness.v2/stream/heartbeat/alarms.v2"),
        layout,
    )
    .await
}

/// A heartbeat's section of the alarms, collection 0: its live count, and
/// the ranks it vouches for.
fn section(b: &common::Beat) -> (u64, Vec<u64>) {
    let s =
        b.hb.sections
            .first()
            .cloned()
            .flatten()
            .expect("the alarms' section");
    (
        s.members.live().expect("a templated collection"),
        s.members.vouched(),
    )
}

/// The first heartbeat after `from` that `f` accepts, waited for up to
/// `within`, the heartbeat's bound: 450 s at the standard contract, where
/// [`eventually`]'s 20 s would give up.
async fn beat_within(
    beats: &Beats,
    from: Instant,
    within: Duration,
    f: impl Fn(&common::Beat) -> bool,
) -> common::Beat {
    let deadline = from + within + JITTER;
    loop {
        let found = beats
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.at > from && f(b))
            .cloned();
        if let Some(b) = found {
            return b;
        }
        assert!(
            Instant::now() < deadline,
            "no such heartbeat within {within:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// What a GET answered: its replies as they arrived.
struct Got {
    samples: Vec<Sample>,
}

/// A GET of `selector` (core S2, S4).
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

/// G's collection read of `address`'s records (§2.12, core S4).
async fn get_alarms(s: &zenoh::Session, address: &str) -> Got {
    let a = addr(address);
    let read = m2::collection_read(a.system.as_str(), a.service.as_str()).unwrap();
    get(s, &read.selector).await
}

/// A read of `address`'s `alarms.v2/state/**`: the records and the words.
async fn get_state(s: &zenoh::Session, address: &str) -> Got {
    get(s, &format!("zk2/{address}/alarms.v2/state/**")).await
}

impl Got {
    /// The replies under `address`'s keys, each its key and its payload.
    fn of(&self, address: &str) -> Vec<(String, Vec<u8>)> {
        let prefix = format!("zk2/{address}/");
        self.samples
            .iter()
            .filter(|s| s.key_expr().as_str().starts_with(&prefix))
            .map(|s| {
                (
                    s.key_expr().as_str().to_owned(),
                    s.payload().to_bytes().into_owned(),
                )
            })
            .collect()
    }

    /// `address`'s snapshots, read (core §4.2, §7.5).
    fn seed(&self, address: &str) -> m2::Seed {
        let replies = self.of(address);
        m2::Seed::of(
            replies.iter().map(|(k, p)| (k.as_str(), p.as_slice())),
            EPOCH,
        )
    }

    /// `address`'s pages, raw.
    fn pages(&self, address: &str) -> Vec<(String, usize, Page)> {
        self.of(address)
            .into_iter()
            .filter_map(|(k, p)| Some((k, p.len(), Page::parse(&p).ok()?)))
            .collect()
    }
}

/// The live entry of `slot` in the alarms' snapshot.
fn entry(seed: &m2::Seed, slot: u8) -> Option<m2::Entry<Result<Record, m2::Undecodable>>> {
    seed.alarms
        .as_ref()
        .expect("the alarms' snapshot")
        .live
        .iter()
        .find(|e| e.slot == Some(slot))
        .cloned()
}

/// The live record of `slot`.
fn record_at(seed: &m2::Seed, slot: u8) -> Record {
    entry(seed, slot)
        .unwrap_or_else(|| panic!("slot {slot} in the snapshot"))
        .record
        .expect("a record that reads")
}

/// The slots the alarms' snapshot lists live.
fn live_slots(seed: &m2::Seed) -> Vec<u8> {
    seed.alarms
        .as_ref()
        .expect("the alarms' snapshot")
        .live
        .iter()
        .filter_map(|e| e.slot)
        .collect()
}

/// The descriptor of `address`'s one instance, `want` where given, once
/// its token reaches the tool.
async fn descriptor_of(
    tool: &zenoh::Session,
    address: &str,
    want: Option<&InstanceId>,
) -> Descriptor {
    let instance = || async {
        presence::tokens(tool, &format!("zk2/{address}/@zk/**"), T)
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
    let instance = eventually_some("the owner's instance token", instance).await;
    presence::descriptor(tool, &addr(address), &instance, T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap()
}

/// The major a descriptor is read by (§2.1, §2.14 step 2), and the
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

/// The runs a present descriptor states (core §3.3).
fn stated(d: &Descriptor) -> Vec<Run> {
    d.stated_run().into_iter().collect()
}

/// A tool's table for an owner (§4): the one table among the revisions its
/// descriptor names that the tool holds.
fn table_for(d: &Descriptor, held: &[Implementation]) -> Option<Vec<Rule>> {
    let named: Vec<Option<Vec<Rule>>> = held
        .iter()
        .filter(|i| {
            let fp = format!("sha256:{}", i.fingerprint().hex());
            d.interfaces
                .iter()
                .any(|e| e.iface == i.iface().to_string() && e.contract == fp)
        })
        .map(|i| m2::table_of(i.contract()))
        .collect();
    m2::instance_table(named.iter().map(Option::as_deref))
        .table()
        .map(<[Rule]>::to_vec)
}

/// G's reading of one slot of a snapshot (§2.14), its freshness by the
/// entry's confirmation under `horizon` (`freshness.v2` §2.6).
fn judge_get(
    seed: &m2::Seed,
    slot: u8,
    presence: Presence,
    stated: &[Run],
    horizon: &fm::Horizon,
) -> (Judged, Judged) {
    let snap = seed.alarms.as_ref().expect("the alarms' snapshot");
    let set = m2::owner_set(
        presence,
        Some(m2::Read::Complete(seed.snapshot(stated))),
        None,
    );
    let e = entry(seed, slot);
    let freshness = e.as_ref().map(|e| {
        let stamp_age = e
            .confirmed_at
            .and_then(|t| common::instant(t, EPOCH))
            .map(|t| fm::StampAge::between(t, SystemTime::now()));
        let obs =
            Observation::of_entry(snap.heard.complete, fm::Entry::Live { stamp_age }, TRUSTED);
        fm::judge_all(horizon, &[obs])
    });
    let r = m2::Reading {
        presence,
        freshness,
        latest: e.as_ref().map(|e| m2::Latest::of(&e.record)),
        lost: false,
        previous_instance: e
            .as_ref()
            .is_some_and(|e| !stated.is_empty() && !stated.contains(&e.stamp.run)),
        held_whole: set.verdict == SetVerdict::Established,
        left_out: e.is_none(),
    };
    (m2::firing(&r), m2::stale_firing(&r))
}

/// A tool's consumer of `lab/svc`'s `alarms.v2` on `imp`'s revision: a
/// heartbeat's tag is its covered revision's.
fn reader(s: &zenoh::Session, imp: &Implementation) -> Consumer {
    Consumer::for_tool(s, imp.shared_contract(), &[SVC], &Default::default()).unwrap()
}

/// S judges the alarm of `slot` from its subscription's age of the key and
/// its latest delivery (§2.14).
fn judge_s(
    c: &Consumer,
    sub: &Subscription,
    log: &[Rec],
    slot: u8,
    presence: Presence,
) -> (Judged, Judged) {
    let key = m2::alarm_key("lab", "svc", slot).unwrap().to_string();
    let last = on_slot(log, slot).last().cloned().expect("a delivery");
    let freshness = c.freshness(av2::ALARMS, &[sub.freshness(&key)]).unwrap();
    let r = m2::Reading {
        presence,
        freshness: Some(freshness),
        latest: Some(if last.delete {
            m2::Latest::Deleted
        } else {
            m2::Latest::of(&Ok::<_, ()>(last.record()))
        }),
        lost: false,
        previous_instance: false,
        held_whole: false,
        left_out: false,
    };
    (m2::firing(&r), m2::stale_firing(&r))
}

fn pair(j: Judged) -> (Answer, Reason) {
    (j.answer, j.reason)
}

/// Whole seconds since the Unix epoch, on this host's clock.
fn now_s() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

/// The conventions' alert keys are the alarms'.
#[test]
fn the_conventions_alert_keys() {
    assert_eq!(alarm_a(Severity::Warning).alert_key(), A_KEY);
    assert_eq!(alarm_b(Severity::Warning).alert_key(), B_KEY);
    assert_eq!(alarm_c().alert_key(), C_KEY);
    assert_eq!(alarm_h(Severity::Warning, "85.5").alert_key(), H_KEY);
    assert_eq!(alarm_d().alert_key(), D_KEY);
    let t1 = table_of(&tc());
    let names: Vec<&str> = t1.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        &names[..4],
        ["hot", "link_down", "sensor_lost", "door_open"]
    );
    let later = table_of(&tc_later());
    assert_eq!(later[6].name, "disk_full", "appended at index 6");
    assert_eq!(&later[..6], &t1[..]);
}

/// §2.1: an owner that implements one major of alarms is refused the
/// other, through either major's call and through `implement`, in either
/// order; nothing of the second is declared, and the first stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_an_owner_implements_one_major() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let both = |e: zenkey::Error| match e {
        zenkey::Error::Contract(why) => {
            assert!(why.contains("never both (alarms.v2 §2.1)"), "{why}");
        }
        other => panic!("{other}"),
    };

    let mut b = ServiceBuilder::new(&os, config("lab/two"));
    let a = b.alarms_v2(Startup::default()).await.unwrap();
    both(
        b.alarms(av1::Startup::default())
            .await
            .err()
            .expect("alarms.v1 beside alarms.v2"),
    );
    both(
        b.implement(av1::implementation())
            .err()
            .expect("alarms.v1's contract beside alarms.v2"),
    );
    assert!(matches!(
        b.alarms_v2(Startup::default()).await,
        Err(zenkey::Error::Duplicate(i)) if i == av2::iface()
    ));
    assert!(b.implements(&av2::iface()) && !b.implements(&av1::iface()));
    assert_eq!(a.iface(), &av2::iface());

    let mut b = ServiceBuilder::new(&os, config("lab/one"));
    b.alarms(av1::Startup::default()).await.unwrap();
    both(
        b.alarms_v2(Startup::default())
            .await
            .err()
            .expect("alarms.v2 beside alarms.v1"),
    );
    both(
        b.implement(av2::implementation())
            .err()
            .expect("alarms.v2's contract beside alarms.v1"),
    );
    assert!(b.implements(&av1::iface()) && !b.implements(&av2::iface()));
}

/// §1: nothing firing is one page of no entry; A raised on the lowest
/// slot, 18 bytes, by the table, and answered by GET; a duplicate
/// observation puts nothing; B on slot 1 in 15 bytes; and a tool reads
/// `lab/svc` by this text and `lab/old` by `alarms.v1`'s, three firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_raise_and_both_majors() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let t1 = table_of(&tc());

    // 1. Nothing firing: G's GET is one page of 16 bytes and no entry,
    // collection 0, under the run the descriptor states.
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    assert_eq!(a.max_alarms().await, 256);
    assert_eq!(a.adoption().source, Source::Nothing);
    assert_eq!(a.table().as_deref(), Some(&t1[..]), "the instance's table");
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let (major, presence) = listed(&d);
    assert_eq!(major, Some(Major::V2));
    let first = get_alarms(&rs, SVC).await;
    let pages = first.pages(SVC);
    assert_eq!(pages.len(), 1, "one page: {pages:?}");
    let (key, len, page) = &pages[0];
    assert_eq!(key, "zk2/lab/svc/alarms.v2/state/alarms");
    assert_eq!(*len, 16, "16 bytes under a 16-bit run");
    assert!(page.live.is_empty() && page.tombs.is_empty());
    assert_eq!(page.coll, m2::ALARMS_COLL);
    assert_eq!(Some(page.run), d.stated_run(), "under the run it states");
    assert!(matches!(page.run, Run::U16(_)), "a 16-bit run");

    // 2. A raised: one put on slot 0, 18 bytes, by the table.
    let before = now_s();
    let done = a.raise(alarm_a(Severity::Warning)).await.unwrap();
    assert_eq!(
        (done.act, done.slot, done.alert_key.as_str()),
        (Act::Raised, 0, A_KEY)
    );
    let recs = at_least(&s.log, 1).await;
    let put = &recs[0];
    assert_eq!(put.slot(), Some(0), "the lowest slot (§2.2)");
    assert_eq!(put.payload.len(), 18, "{:02x?}", put.payload);
    assert_eq!(put.payload[0], 0x0b, "CH: form 0, a 16-bit run, tag 11");
    assert_eq!(put.envelope().record.len(), 12, "a record of 12 bytes");
    let rec = put.record();
    assert_eq!(rec, done.record);
    assert_eq!(
        (
            rec.state,
            rec.severity,
            rec.rule,
            rec.labels,
            rec.argument,
            rec.key
        ),
        (
            State::Firing,
            Some(Severity::Warning),
            RuleIndex::Table(1),
            Some(2),
            Argument::NotAvailable,
            None
        )
    );
    let at = rec.at.expect("an `at`");
    assert!(at >= before - 1 && at <= now_s() + 1, "within 1 s: {at}");
    assert!(words_of(&snapshot(&s.log)).is_empty(), "no words");
    assert_eq!(
        transitions(&reads(&snapshot(&s.log), 0, Some(&t1))),
        [Transition::Raised]
    );
    let got = get_alarms(&rs, SVC).await;
    let seed = got.seed(SVC);
    let e = entry(&seed, 0).expect("slot 0 listed");
    assert_eq!(e.id, "0");
    assert_eq!(e.record, Ok(rec));
    assert_eq!(e.stamp, put.stamp(), "under the put's in-band stamp");
    assert_eq!(e.confirmation, Confirmation::AtTime, "vouched at its time");
    let (firing, _) = judge_get(&seed, 0, presence, &stated(&d), &am::horizon());
    assert_eq!(pair(firing), (Answer::Yes, Reason::Firing));

    // 3. A duplicate observation: nothing put. 4. B, on slot 1.
    let again = a.raise(alarm_a(Severity::Warning)).await.unwrap();
    assert_eq!((again.act, again.slot), (Act::Unchanged, 0));
    let done = a.raise(alarm_b(Severity::Warning)).await.unwrap();
    assert_eq!((done.act, done.slot), (Act::Raised, 1));
    let recs = at_least(&s.log, 2).await;
    assert_eq!(
        recs.iter().map(Rec::slot).collect::<Vec<_>>(),
        [Some(0), Some(1)],
        "nothing more on slot 0 before B"
    );
    let b_put = &recs[1];
    assert_eq!(b_put.payload.len(), 15, "a record of 9 bytes");
    assert_eq!(
        (b_put.record().rule, b_put.record().labels),
        (RuleIndex::Table(3), None)
    );

    // 5. lab/old implements alarms.v1 and holds A firing. T reads both.
    let old_s = client(&ep).await;
    let mut ob = ServiceBuilder::new(&old_s, config("lab/old"));
    let old = ob.alarms(av1::Startup::default()).await.unwrap();
    let old_svc = ob.start().await.unwrap();
    let old_a = av1::Condition::new("link_down", Severity::Warning, "Link to r2 on eth0 is down")
        .label("peer", "r2")
        .label("port", "eth0");
    old.raise(old_a).await.unwrap();
    let ts = client(&ep).await;
    let svc_d = descriptor_of(&ts, SVC, None).await;
    let old_d = descriptor_of(&ts, "lab/old", None).await;
    assert_eq!(listed(&svc_d).0, Some(Major::V2), "lists alarms.v2 alone");
    assert!(svc_d.interfaces.iter().all(|e| e.iface != "alarms.v1"));
    assert_eq!(listed(&old_d).0, Some(Major::V1), "lists alarms.v1 alone");
    // lab/svc by this text: one read over the scope's alarms.v2.
    let v2_read = get(&ts, "zk2/lab/*/alarms.v2/state/alarms/**").await;
    let svc_seed = v2_read.seed(SVC);
    let (_, svc_presence) = listed(&svc_d);
    let svc_set = m2::owner_set(
        svc_presence,
        Some(m2::Read::Complete(svc_seed.snapshot(&stated(&svc_d)))),
        None,
    );
    assert_eq!(
        (svc_set.verdict, svc_set.reason),
        (SetVerdict::Established, Reason::Snapshot)
    );
    let svc_alarms: Vec<(Judged, Option<Severity>)> = live_slots(&svc_seed)
        .into_iter()
        .map(|slot| {
            let (j, _) = judge_get(
                &svc_seed,
                slot,
                svc_presence,
                &stated(&svc_d),
                &am::horizon(),
            );
            (j, record_at(&svc_seed, slot).severity)
        })
        .collect();
    // lab/old by alarms.v1's: its GET's answer.
    let old_c = Consumer::for_tool(
        &ts,
        av1::implementation().shared_contract(),
        &["lab/old"],
        &Default::default(),
    )
    .unwrap();
    let old_answer = eventually_some("lab/old answers A", || async {
        let a = old_c.get_answer(av1::ALARMS, None, T).await.ok()?;
        (!a.current.is_empty()).then_some(a)
    })
    .await;
    let (_, old_presence) = listed(&old_d);
    let old_set = am::owner_set(
        old_presence,
        Some(am::Seed::Complete {
            members: old_answer.current.len(),
        }),
        None,
    );
    let old_alarms: Vec<(Judged, Option<Severity>)> = old_answer
        .current
        .iter()
        .map(|c| {
            let Current::Value { sample, .. } = c else {
                panic!("a value: {c:?}")
            };
            let doc = am::decode(&sample.payload().to_bytes()).unwrap();
            let obs = [c.freshness(TRUSTED)];
            let r = am::Reading {
                presence: old_presence,
                observations: &obs,
                latest: Some(am::Latest::Firing),
                previous_instance: false,
                held_whole: true,
                left_out: false,
            };
            (am::firing(&r), Some(doc.severity))
        })
        .collect();
    let any = am::any_firing(
        Severity::Warning,
        &[
            am::OwnerAlarms {
                set: svc_set,
                alarms: &svc_alarms,
            },
            am::OwnerAlarms {
                set: old_set,
                alarms: &old_alarms,
            },
        ],
        am::ScopeRead::Whole,
    );
    assert_eq!(
        (any.answer, any.reason, any.firing),
        (Answer::Yes, Reason::AnyFiring, 3),
        "A and B of lab/svc, A of lab/old"
    );
    // Two A's: one alert key, two references (§2.12).
    let svc_a = record_at(&svc_seed, 0).name(Some(&t1)).alert_key();
    assert_eq!(svc_a.map(am::key_hex).as_deref(), Some(A_KEY));
    assert_eq!(
        old_answer.current[0].key(),
        "zk2/lab/old/alarms.v1/state/alarms/a659f813308ad1da"
    );
    old_svc.close().await.unwrap();
    svc.close().await.unwrap();
}

/// §2 on `imp`: H escalated, then refreshed with the escalation's `at`
/// kept, a second refresh within the window deferred, then nothing for
/// `quiet`, past the horizon: no put, a heartbeat at most `bound` apart
/// vouching for slot 0, and S judging H firing at the end.
async fn escalate_refresh_confirm(imp: Implementation, quiet: Duration, bound: Duration) {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (_h, beats) = heartbeats(&rs, SVC, &imp).await;
    let tag = zenkey::model::heartbeat::Layout::of(imp.contract()).tag;
    let g = reader(&rs, &imp);
    let sub = g.subscribe(av2::ALARMS, |_| {}).await.unwrap();
    let (b, a) = owner(
        &os,
        SVC,
        imp,
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let (_, presence) = listed(&d);
    let t1 = table_of(&tc());

    // H firing at warning on slot 0, at t0, reading 85.5: argument 1255.
    let raised = a.raise(alarm_h(Severity::Warning, "85.5")).await.unwrap();
    assert_eq!((raised.slot, raised.alert_key.as_str()), (0, H_KEY));
    assert_eq!(raised.record.argument, Argument::Code(1255));
    let t0 = raised.record.at.expect("an `at`");
    at_least(&s.log, 1).await;

    // 1. Escalated to critical: a new `at`, later than t0.
    let done = a.raise(alarm_h(Severity::Critical, "85.5")).await.unwrap();
    assert_eq!(done.act, Act::Escalated);
    let recs = at_least(&s.log, 2).await;
    let escalated = recs[1].record();
    assert_eq!(recs[1].slot(), Some(0));
    assert_eq!(escalated.severity, Some(Severity::Critical));
    assert!(escalated.at.unwrap() > t0, "a new `at`, later than t0");
    assert_eq!(
        transitions(&reads(&recs, 0, Some(&t1))),
        [Transition::Raised, Transition::Escalated]
    );

    // 2. 86.0: refreshed, the escalation's `at` kept, argument 1260.
    let done = a.raise(alarm_h(Severity::Critical, "86.0")).await.unwrap();
    assert_eq!(done.act, Act::Refreshed);
    let recs = at_least(&s.log, 3).await;
    let refreshed = recs[2].record();
    assert_eq!(refreshed.at, escalated.at, "`at` kept");
    assert_eq!(refreshed.argument, Argument::Code(1260));
    assert_eq!(
        transitions(&reads(&recs, 0, Some(&t1)))[2],
        Transition::Refreshed
    );
    let refreshed_at = recs[2].at;

    // 3. 86.5 within the window: deferred, nothing put (§2.5).
    let done = a.raise(alarm_h(Severity::Critical, "86.5")).await.unwrap();
    assert_eq!(done.act, Act::Deferred);
    assert_eq!(done.record, refreshed, "the record on the slot unchanged");

    // 4. Nothing for `quiet`, past the horizon: no put; the heartbeat
    // vouches for slot 0 at most `bound` apart, form 6, under the covered
    // revision's tag, in 17 bytes, with a live count of 1.
    let end = refreshed_at + quiet;
    tokio::time::sleep_until(end.into()).await;
    let recs = records(&snapshot(&s.log));
    assert_eq!(recs.len(), 3, "never put again: {recs:#?}");
    let seen = common::assert_cadence(&beats, refreshed_at, end, bound, JITTER, tag);
    for beat in &seen {
        assert_eq!(beat.len, 17, "a heartbeat of 17 bytes");
        assert_eq!(section(beat), (1, vec![0]), "slot 0's bit set");
    }
    // At the contract's own horizon, the copy's 4 s or the standard's 900 s.
    let (firing, _) = judge_s(&g, &sub, &snapshot(&s.log), 0, presence);
    assert_eq!(
        pair(firing),
        (Answer::Yes, Reason::Firing),
        "the heartbeats confirmed it"
    );
    svc.close().await.unwrap();
}

/// §2, fast: the copy's ttl 4 s, a heartbeat every 1.9 s, a refresh
/// window of 2 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_escalate_refresh_and_confirm() {
    escalate_refresh_confirm(fast(4), Duration::from_secs(7), Duration::from_secs(2)).await;
}

/// §2 at the standard contract: 905 s of nothing, a heartbeat every 450 s,
/// its tag 7.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §2 waits 905 s at the contract's 900 s horizon"]
async fn s2_escalate_refresh_and_confirm_at_900_s() {
    let imp = av2::implementation();
    assert_eq!(
        zenkey::model::heartbeat::Layout::of(imp.contract()).tag,
        Tag::new(7).unwrap()
    );
    escalate_refresh_confirm(imp, Duration::from_secs(905), Duration::from_secs(450)).await;
}

/// §3 on `imp`: a resolve is one put, kept as the slot's value with no
/// delete; G reads it resolved within 1 s and 65 s later; the next
/// heartbeat counts it live with its bit clear.
async fn resolve_one_put(imp: Implementation, bound: Duration) {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (_h, beats) = heartbeats(&rs, SVC, &imp).await;
    let g = reader(&rs, &imp);
    let sub = g.subscribe(av2::ALARMS, |_| {}).await.unwrap();
    let hz = horizon(&imp);
    let (b, a) = owner(
        &os,
        SVC,
        imp,
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let (_, presence) = listed(&d);
    let t1 = table_of(&tc());
    let raised = a.raise(alarm_a(Severity::Warning)).await.unwrap().record;
    at_least(&s.log, 1).await;

    // 1. Resolved: one put on slot 0, and no delete.
    let done = a.resolve(A_KEY).await.unwrap().expect("A held");
    assert_eq!((done.act, done.slot), (Act::Resolved, 0));
    let recs = at_least(&s.log, 2).await;
    let resolved = recs[1].record();
    assert_eq!(recs[1].slot(), Some(0));
    assert_eq!(
        (
            resolved.state,
            resolved.severity,
            resolved.rule,
            resolved.labels
        ),
        (
            State::Resolved,
            Some(Severity::Warning),
            RuleIndex::Table(1),
            Some(2)
        )
    );
    assert!(resolved.at > raised.at, "a new `at`");
    assert_eq!(
        transitions(&reads(&recs, 0, Some(&t1))),
        [Transition::Raised, Transition::Resolved]
    );
    assert!(a.resolve(A_KEY).await.unwrap().is_none(), "not firing");
    let resolved_at = recs[1].at;

    // 2. Within 1 s: slot 0 live, the resolved record, confirmation 2 with
    // its put's time; no tombstone. No, resolved.
    let got = get_alarms(&rs, SVC).await;
    let seed = got.seed(SVC);
    assert!(resolved_at.elapsed() < Duration::from_secs(1) + JITTER);
    let e = entry(&seed, 0).expect("slot 0 live");
    assert_eq!(e.record, Ok(resolved));
    assert!(
        matches!(e.confirmation, Confirmation::At(_)),
        "its last confirmation: {:?}",
        e.confirmation
    );
    assert!(
        seed.alarms.as_ref().unwrap().retired.is_empty(),
        "no tombstone"
    );
    let (firing, _) = judge_get(&seed, 0, presence, &stated(&d), &hz);
    assert_eq!(pair(firing), (Answer::No, Reason::Resolved));

    // 3. 65 s later: the same, still the slot's value.
    tokio::time::sleep_until((resolved_at + Duration::from_secs(65)).into()).await;
    let got = get_alarms(&rs, SVC).await;
    let seed = got.seed(SVC);
    assert_eq!(record_at(&seed, 0), resolved);
    let (firing, _) = judge_get(&seed, 0, presence, &stated(&d), &hz);
    assert_eq!(pair(firing), (Answer::No, Reason::Resolved));
    assert!(
        snapshot(&s.log).iter().all(|r| !r.delete),
        "no delete, within 60 s or after it"
    );

    // 4. The owner's next heartbeat: a live count of 1, slot 0's bit
    // clear, in step with S's view. S judges A resolved, never stale.
    let beat = beat_within(&beats, Instant::now(), bound, |_| true).await;
    assert_eq!(section(&beat), (1, vec![]), "live, its bit clear");
    let (firing, _) = judge_s(&g, &sub, &snapshot(&s.log), 0, presence);
    assert_eq!(pair(firing), (Answer::No, Reason::Resolved));
    assert!(a.held().await[&0].record.state == State::Resolved);
    svc.close().await.unwrap();
}

/// §3, fast: the copy's ttl 4 s, so its next heartbeat within 1.9 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_resolve_is_one_put_and_no_delete() {
    resolve_one_put(fast(4), Duration::from_secs(2)).await;
}

/// §3 at the standard contract: its next heartbeat within 427.5 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §3 reads a heartbeat up to 427.5 s away at the contract's 900 s horizon"]
async fn s3_a_resolve_is_one_put_and_no_delete_at_900_s() {
    resolve_one_put(av2::implementation(), Duration::from_secs(450)).await;
}

/// §4: B takes A's resolved slot 0, which S reads as `replaced` and a
/// reader that held A firing as `rebound`; A comes back on slot 1, later
/// than its resolve; B's re-raise keeps slot 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_re_raise_and_a_resolved_slot_retaken() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let (_, presence) = listed(&d);
    let t1 = table_of(&tc());

    // Setup: A firing on slot 0, which L GETs; then §3, A resolved.
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    at_least(&s.log, 1).await;
    let l_first = get_alarms(&rs, SVC).await.seed(SVC);
    let mut l = m2::Track::default();
    let l_entry = entry(&l_first, 0).expect("A on slot 0");
    l.apply(m2::Sample::Put(held(
        l_entry.record.as_ref().unwrap(),
        l_entry.stamp,
        Some(&t1),
    )));
    let a_resolved = a.resolve(A_KEY).await.unwrap().unwrap().record;
    at_least(&s.log, 2).await;

    // 1. B on slot 0: S reads A's record replaced, then B raised.
    let done = a.raise(alarm_b(Severity::Warning)).await.unwrap();
    assert_eq!((done.act, done.slot), (Act::Raised, 0));
    let recs = at_least(&s.log, 3).await;
    assert_eq!(recs[2].slot(), Some(0));
    assert_eq!(recs[2].record().rule, RuleIndex::Table(3), "B's record");
    let applied = reads(&recs, 0, Some(&t1));
    let third = &applied[2];
    assert_eq!(third.displaced.map(|x| x.0), Some(m2::Displaced::Replaced));
    assert_eq!(third.reads, Transition::Raised);
    assert!(
        third
            .reports
            .iter()
            .all(|r| r.alert_key.map(am::key_hex).as_deref() == Some(B_KEY)),
        "nothing reported of A: {:?}",
        third.reports
    );

    // 2. L GETs: slot 0 holds B under the run of the record L holds, so
    // L reads A resolved by `rebound`, then B raised.
    let l_now = get_alarms(&rs, SVC).await.seed(SVC);
    let e = entry(&l_now, 0).expect("slot 0");
    assert_eq!(e.stamp.run, l_entry.stamp.run, "under the same run");
    let got = l.apply(m2::Sample::Put(held(
        e.record.as_ref().unwrap(),
        e.stamp,
        Some(&t1),
    )));
    assert_eq!(got.displaced.map(|x| x.0), Some(m2::Displaced::Rebound));
    assert_eq!(got.reads, Transition::Raised);
    let rebound = m2::Reading {
        presence,
        freshness: None,
        latest: Some(m2::Latest::Rebound),
        lost: false,
        previous_instance: false,
        held_whole: true,
        left_out: false,
    };
    assert_eq!(pair(m2::firing(&rebound)), (Answer::No, Reason::Rebound));

    // 3. A again: slot 1, the lowest free, `at` later than its resolve's.
    let done = a.raise(alarm_a(Severity::Warning)).await.unwrap();
    assert_eq!((done.act, done.slot), (Act::Raised, 1));
    let recs = at_least(&s.log, 4).await;
    assert_eq!(recs[3].slot(), Some(1));
    assert!(
        recs[3].record().at > a_resolved.at,
        "later than its resolve"
    );
    assert_eq!(
        transitions(&reads(&recs, 1, Some(&t1))),
        [Transition::Raised]
    );

    // 4. B resolved on slot 0, then B firing on slot 0.
    assert_eq!(a.resolve(B_KEY).await.unwrap().unwrap().slot, 0);
    let again = a.raise(alarm_b(Severity::Warning)).await.unwrap();
    assert_eq!((again.act, again.slot), (Act::Raised, 0));
    let recs = at_least(&s.log, 6).await;
    assert_eq!(
        recs.iter()
            .map(|r| (r.slot().unwrap(), r.record().state, r.record().rule))
            .collect::<Vec<_>>(),
        [
            (0, State::Firing, RuleIndex::Table(1)),
            (0, State::Resolved, RuleIndex::Table(1)),
            (0, State::Firing, RuleIndex::Table(3)),
            (1, State::Firing, RuleIndex::Table(1)),
            (0, State::Resolved, RuleIndex::Table(3)),
            (0, State::Firing, RuleIndex::Table(3)),
        ]
    );
    // No identity on two slots at any step (§2.2).
    let seen: Vec<m2::Seen> = live_slots(&get_alarms(&rs, SVC).await.seed(SVC))
        .into_iter()
        .map(|slot| {
            let (_, rec) = (slot, on_slot(&recs, slot).last().unwrap().clone());
            m2::Seen {
                slot,
                stamp: rec.stamp(),
                alert_key: rec.record().name(Some(&t1)).alert_key(),
            }
        })
        .collect();
    assert!(m2::taken(&seen, d.stated_run()).double.is_empty());
    svc.close().await.unwrap();
}

/// §5: a restart adopts from its record by slot. The kept alarms are put
/// again by the new instance before its token, D by the later table now
/// that it names its words; nothing is put for a resolved entry or one
/// whose words do not derive its key; B, cleared while down, resolves at
/// the first evaluation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_restart_mid_incident() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let path = dir.path().join("alarms.json");
    let runs = run_file(dir.path(), "svc");
    let startup = || Startup::default().with_record(Arc::new(FileRecord::new(&path)));
    let t1 = table_of(&tc());
    let later = table_of(&tc_later());

    // Setup: A firing at critical on slot 0 since t0, B on slot 1, D by
    // words on slot 2.
    let os = client(&ep).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        startup(),
        &runs,
        Some(tc()),
    )
    .await;
    let svc = b.start().await.unwrap();
    descriptor_of(&rs, SVC, None).await;
    let a_rec = a.raise(alarm_a(Severity::Critical)).await.unwrap().record;
    let t0 = a_rec.at.expect("an `at`");
    assert_eq!(a.raise(alarm_b(Severity::Warning)).await.unwrap().slot, 1);
    let d_done = a.raise(alarm_d()).await.unwrap();
    assert_eq!((d_done.slot, d_done.alert_key.as_str()), (2, D_KEY));
    assert_eq!(
        d_done.record.rule,
        RuleIndex::NotAvailable,
        "T1 does not name it"
    );
    at_least(&s.log, 3).await;

    // 1. The process ends without closing; S listens 3 s.
    drop(a);
    drop(svc);
    os.close().await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let before = snapshot(&s.log);
    assert!(before.iter().all(|r| !r.delete), "no delete");
    assert_eq!(records(&before).len(), 3, "no put (§2.9)");

    // The record holds A, B and D by slot; a test writes R and P into it.
    let Ok(Found::V2(kept)) = FileRecord::parse(&std::fs::read_to_string(&path).unwrap()) else {
        panic!("an alarms.v2 record")
    };
    assert_eq!(kept.iter().map(|k| k.slot).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(kept[2].alert_key, D_KEY);
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let r_words =
        Condition::new("sensor_lost", Severity::Warning, "Sensor 7 lost").label("sensor", "7");
    let r_fields = m2::write_alarm(Some(&t1), "sensor_lost", [("sensor", "7")], None);
    assert_eq!(r_fields.rule, RuleIndex::Table(2));
    let r_rec = Record::new(State::Resolved, Severity::Warning, r_fields, Some(t0 - 60));
    file["slots"]["3"] = serde_json::json!({
        "record": r_rec.to_json(),
        "words": r_words.words().to_json(),
        "alert_key": r_words.alert_key(),
    });
    file["slots"]["4"] = serde_json::json!({
        "record": a_rec.to_json(),
        "words": alarm_a(Severity::Critical).words().to_json(),
        "alert_key": "0000000000000001",
    });
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

    // 2. B's condition clears while it is down. 3. It restarts, a new
    // process, so a new session, with the later revision of tc.v1.
    let os = client(&ep).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        startup(),
        &runs,
        Some(tc_later()),
    )
    .await;
    let adoption = a.adoption();
    assert_eq!(adoption.source, Source::Record);
    assert_eq!(adoption.slots(m2::Adoption::Adopted), [0, 1, 2]);
    assert_eq!(
        adoption.slots(m2::Adoption::Free),
        [3, 4],
        "R resolved, P a phantom"
    );
    assert!(adoption.failed.is_empty());
    let svc = b.start().await.unwrap();
    let instance = svc.instance().clone();
    let new_run = svc.minter().run();

    // G GETs the moment S sees the new instance token: a snapshot under
    // the new run, slots 0 to 2 put again under it, neither 3 nor 4.
    at_new_token(&s.log, before.len()).await;
    let at_token = get_alarms(&rs, SVC).await.seed(SVC);
    let snap = at_token.alarms.as_ref().expect("the alarms' snapshot");
    assert!(snap.heard.complete);
    assert_eq!(snap.heard.run, new_run, "under the new instance's run");
    assert_eq!(live_slots(&at_token), [0, 1, 2]);
    for slot in [0, 1, 2] {
        assert_eq!(
            entry(&at_token, slot).unwrap().stamp.run,
            new_run,
            "slot {slot} put again under the new run before the GET"
        );
    }
    let a_again = record_at(&at_token, 0);
    assert_eq!(a_again, a_rec, "A firing, t0 kept");
    assert_eq!(a_again.at, Some(t0));
    assert_eq!(record_at(&at_token, 1).state, State::Firing, "B firing");
    let d_again = record_at(&at_token, 2);
    assert_eq!(
        (d_again.state, d_again.rule, d_again.labels, d_again.key),
        (State::Firing, RuleIndex::Table(6), Some(0), None),
        "the later table names D's words (§2.9, by name)"
    );
    assert_eq!(
        d_again
            .name(Some(&later))
            .alert_key()
            .map(am::key_hex)
            .as_deref(),
        Some(D_KEY),
        "the same alert key"
    );
    assert_eq!(d_again.at, d_done.record.at, "D's `at` kept");

    // S: A confirmed, B confirmed, D refreshed, in no order against the
    // token; B resolved at the first evaluation.
    assert_eq!(
        a.raise(alarm_a(Severity::Critical)).await.unwrap().act,
        Act::Unchanged
    );
    assert_eq!(a.raise(alarm_d()).await.unwrap().act, Act::Unchanged);
    let resolved = a.resolve(B_KEY).await.unwrap().unwrap();
    assert_eq!((resolved.act, resolved.slot), (Act::Resolved, 1));
    let all = at_least(&s.log, 7).await;
    let table = |slot: u8, rec: &Rec| {
        if rec.stamp().run == new_run || slot != 2 {
            Some(later.clone())
        } else {
            Some(t1.clone())
        }
    };
    let read_slot = |slot: u8| {
        let mut t = m2::Track::default();
        on_slot(&all, slot)
            .iter()
            .map(|r| {
                let tb = table(slot, r);
                t.apply(m2::Sample::Put(held(&r.record(), r.stamp(), tb.as_deref())))
                    .reads
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(read_slot(0), [Transition::Raised, Transition::Confirmed]);
    assert_eq!(
        read_slot(1),
        [
            Transition::Raised,
            Transition::Confirmed,
            Transition::Resolved
        ]
    );
    assert_eq!(read_slot(2), [Transition::Raised, Transition::Refreshed]);
    assert!(on_slot(&all, 3).is_empty() && on_slot(&all, 4).is_empty());
    assert!(
        words_of(&all[before.len()..]).is_empty(),
        "D by the table now: no words"
    );

    // 4. G judges A firing, never previous_instance.
    let d = descriptor_of(&rs, SVC, Some(&instance)).await;
    assert_eq!(d.stated_run(), Some(new_run));
    let (_, presence) = listed(&d);
    let seed = get_alarms(&rs, SVC).await.seed(SVC);
    let (firing, _) = judge_get(&seed, 0, presence, &stated(&d), &am::horizon());
    assert_eq!(pair(firing), (Answer::Yes, Reason::Firing));
    // The record holds what the instance holds, B resolved among them.
    let Ok(Found::V2(kept)) = FileRecord::parse(&std::fs::read_to_string(&path).unwrap()) else {
        panic!("an alarms.v2 record")
    };
    assert_eq!(kept.iter().map(|k| k.slot).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(kept[1].record["state"], "resolved");
    svc.close().await.unwrap();
}

/// §6 on `imp`: the owner stops confirming A and keeps its tokens; each
/// heartbeat counts A and B's resolved record live with both bits clear;
/// past the horizon A is stale and B still resolved.
async fn stops_confirming(imp: Implementation, quiet: Duration, bound: Duration) {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (_h, beats) = heartbeats(&rs, SVC, &imp).await;
    let g = reader(&rs, &imp);
    let sub = g.subscribe(av2::ALARMS, |_| {}).await.unwrap();
    let hz = horizon(&imp);
    let (b, a) = owner(
        &os,
        SVC,
        imp,
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let (_, presence) = listed(&d);
    a.raise(alarm_a(Severity::Warning)).await.unwrap();
    a.raise(alarm_b(Severity::Warning)).await.unwrap();
    a.resolve(B_KEY).await.unwrap().unwrap();
    at_least(&s.log, 3).await;
    // A vouched for by a heartbeat first.
    beat_within(&beats, Instant::now(), bound, |b| {
        section(b) == (2, vec![0])
    })
    .await;

    // 1. A's rule no longer evaluated: its bit cleared; tokens, heartbeats
    // and GETs go on.
    assert!(a.set_confirming(A_KEY, false).await);
    assert_eq!(a.is_confirming(A_KEY).await, Some(false));
    let cleared = Instant::now();
    let first_clear = beat_within(&beats, cleared, bound, |b| section(b) == (2, vec![])).await;
    let last_vouched = beats
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.at < first_clear.at && section(b) == (2, vec![0]))
        .map(|b| b.at)
        .next_back()
        .expect("a heartbeat vouching for A before the first clear one");

    // 2. `quiet` after the last heartbeat that vouched for A.
    tokio::time::sleep_until((last_vouched + quiet).into()).await;
    for beat in beats
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.at > cleared + JITTER)
    {
        assert_eq!(
            section(beat),
            (2, vec![]),
            "a live count of 2, both bits clear"
        );
    }
    assert_eq!(records(&snapshot(&s.log)).len(), 3, "no put of A");
    assert!(
        descriptor_of(&rs, SVC, None).await.run.is_some(),
        "its token present"
    );
    let log = snapshot(&s.log);
    let (firing, stale) = judge_s(&g, &sub, &log, 0, presence);
    assert_eq!(pair(firing), (Answer::Unobservable, Reason::Stale));
    assert_eq!(pair(stale), (Answer::Yes, Reason::Stale));
    let (firing, _) = judge_s(&g, &sub, &log, 1, presence);
    assert_eq!(
        pair(firing),
        (Answer::No, Reason::Resolved),
        "B: never stale"
    );
    let seed = get_alarms(&rs, SVC).await.seed(SVC);
    let a_entry = entry(&seed, 0).unwrap();
    assert!(
        matches!(a_entry.confirmation, Confirmation::At(_)),
        "A with its last confirmation"
    );
    let (firing, stale) = judge_get(&seed, 0, presence, &stated(&d), &hz);
    assert_eq!(pair(firing), (Answer::Unobservable, Reason::Stale));
    assert_eq!(pair(stale), (Answer::Yes, Reason::Stale));
    let (firing, _) = judge_get(&seed, 1, presence, &stated(&d), &hz);
    assert_eq!(pair(firing), (Answer::No, Reason::Resolved));
    svc.close().await.unwrap();
}

/// §6, fast: the copy's ttl 4 s; 5 s after the last heartbeat that vouched
/// for A.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_hung_owner_goes_stale() {
    stops_confirming(fast(4), Duration::from_secs(5), Duration::from_secs(2)).await;
}

/// §6 at the standard contract: 905 s after the last heartbeat that
/// vouched for A.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "slow tier: §6 waits 905 s at the contract's 900 s horizon"]
async fn s6_a_hung_owner_goes_stale_at_900_s() {
    stops_confirming(
        av2::implementation(),
        Duration::from_secs(905),
        Duration::from_secs(450),
    )
    .await;
}

/// §7: at the descriptor's bound of 2, a raise with both slots firing is
/// refused and its writer told, nothing put or resolved; a resolved slot
/// takes no room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_at_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        Startup::default().with_cardinality(2),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    assert_eq!(a.max_alarms().await, 2);
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, None).await;
    let entry_of = |d: &Descriptor| {
        d.interfaces
            .iter()
            .find(|e| e.iface == "alarms.v2")
            .unwrap()
            .cardinality
            .get(av2::ALARMS)
            .copied()
    };
    assert_eq!(entry_of(&d), Some(2), "the descriptor states 2");
    let t1 = table_of(&tc());

    // 1. A and B on slots 0 and 1.
    assert_eq!(a.raise(alarm_a(Severity::Warning)).await.unwrap().slot, 0);
    assert_eq!(a.raise(alarm_b(Severity::Warning)).await.unwrap().slot, 1);

    // 2. C refused: its writer told; nothing put or resolved.
    let refused = a.raise(alarm_c()).await;
    let Err(zenkey::Error::Budget(r)) = refused else {
        panic!("refused at the bound: {refused:?}")
    };
    assert_eq!(r.resource, av2::ALARMS);

    // 3. B resolved, then C on slot 1: a resolved record takes no room.
    assert_eq!(a.resolve(B_KEY).await.unwrap().unwrap().slot, 1);
    let c = a.raise(alarm_c()).await.unwrap();
    assert_eq!(
        (c.act, c.slot, c.alert_key.as_str()),
        (Act::Raised, 1, C_KEY)
    );
    assert_eq!(c.record.labels, Some(5), "C: labels 5");
    let recs = at_least(&s.log, 4).await;
    assert_eq!(
        recs.iter()
            .map(|r| (r.slot().unwrap(), r.record().state))
            .collect::<Vec<_>>(),
        [
            (0, State::Firing),
            (1, State::Firing),
            (1, State::Resolved),
            (1, State::Firing)
        ],
        "no put of C, no resolve of A or B, nothing on slot 0 or 1 at step 2"
    );
    let applied = reads(&recs, 1, Some(&t1));
    assert_eq!(
        applied[2].displaced.map(|x| x.0),
        Some(m2::Displaced::Replaced)
    );

    // 4. B again: refused, both slots fire.
    assert!(matches!(
        a.raise(alarm_b(Severity::Warning)).await,
        Err(zenkey::Error::Budget(_))
    ));
    let seed = get_alarms(&rs, SVC).await.seed(SVC);
    assert_eq!(live_slots(&seed), [0, 1]);
    assert_eq!(record_at(&seed, 0).labels, Some(2), "A on slot 0");
    assert_eq!(record_at(&seed, 1).labels, Some(5), "C on slot 1");
    assert_eq!(records(&snapshot(&s.log)).len(), 4, "nothing put at step 4");
    svc.close().await.unwrap();
}

/// §9: words. An owner with no table writes A by words, its words first;
/// a refresh by words is its words alone; an owner with a table writes A by
/// numbers and W by words. A tool holding the named revision shows names,
/// one holding another revision numbers, and words need no table. A slot
/// whose words do not derive its key is a mismatch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_words() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let svc_s = client(&ep).await;
    let plain_s = client(&ep).await;
    let ts = client(&ep).await;
    let plain = record(&ts, "lab/plain").await;
    let svcr = record(&ts, SVC).await;
    let t1 = table_of(&tc());
    let (pb, pa) = owner(
        &plain_s,
        "lab/plain",
        av2::implementation(),
        Startup::default(),
        &run_file(dir.path(), "plain"),
        None,
    )
    .await;
    assert_eq!(pa.table(), None, "no table");
    let plain_svc = pb.start().await.unwrap();
    let (sb, sa) = owner(
        &svc_s,
        SVC,
        av2::implementation(),
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let svc = sb.start().await.unwrap();
    descriptor_of(&ts, "lab/plain", None).await;
    descriptor_of(&ts, SVC, None).await;

    // 1. lab/plain raises A: its words on words/0, then its record on
    // alarms/0, 23 bytes, by words.
    let a_plain = Condition::new("link_down", Severity::Warning, "Link to r2 is down")
        .label("peer", "r2")
        .label("port", "eth0");
    let done = pa.raise(a_plain.clone()).await.unwrap();
    assert_eq!(done.slot, 0);
    eventually("the words and the record", || async {
        let log = snapshot(&plain.log);
        !words_of(&log).is_empty() && !records(&log).is_empty()
    })
    .await;
    let log = snapshot(&plain.log);
    let w = words_of(&log)[0].clone();
    let r = records(&log)[0].clone();
    assert_eq!(w.words_slot(), Some(0));
    assert_eq!(w.payload[0], 0x00, "a JSON type has no tag");
    assert_eq!(w.words(), a_plain.words());
    assert_eq!(r.slot(), Some(0));
    assert_eq!(r.payload.len(), 23, "{:02x?}", r.payload);
    let rec = r.record();
    assert_eq!(
        (rec.rule, rec.labels, rec.key),
        (RuleIndex::NotAvailable, None, Some(0xa659_f813_308a_d1da))
    );
    assert!(w.stamp().seq.get() < r.stamp().seq.get(), "the words first");

    // 2. Its summary changes: the words alone, which derive the key.
    let flaps = Condition {
        summary: "Link to r2 flaps".into(),
        ..a_plain.clone()
    };
    assert_eq!(pa.raise(flaps.clone()).await.unwrap().act, Act::Refreshed);
    eventually("the new words", || async {
        words_of(&snapshot(&plain.log)).len() >= 2
    })
    .await;
    let plain_state = get_state(&ts, "lab/plain").await.seed("lab/plain");
    let log = snapshot(&plain.log);
    assert_eq!(records(&log).len(), 1, "no put on alarms/0");
    let held_rec = record_at(&plain_state, 0);
    assert_eq!(held_rec, rec, "A's state and `at` do not move");
    let new_words = words_of(&log)[1].words();
    assert_eq!(new_words.summary, "Link to r2 flaps");
    assert_eq!(
        m2::words_taken(Some(&Ok::<_, ()>(held_rec)), Some(&Ok::<_, ()>(new_words))),
        Ok(m2::How::Strict)
    );

    // 3. lab/svc raises A by its table, then W by words on slot 1.
    sa.raise(alarm_a(Severity::Warning)).await.unwrap();
    let w_done = sa.raise(alarm_d()).await.unwrap();
    assert_eq!(
        (w_done.slot, w_done.record.rule),
        (1, RuleIndex::NotAvailable)
    );
    eventually("lab/svc's puts", || async {
        let log = snapshot(&svcr.log);
        records(&log).len() >= 2 && !words_of(&log).is_empty()
    })
    .await;
    let log = snapshot(&svcr.log);
    let recs = records(&log);
    assert_eq!(recs[0].payload.len(), 18);
    assert_eq!(
        (
            recs[0].slot(),
            recs[0].record().rule,
            recs[0].record().labels
        ),
        (Some(0), RuleIndex::Table(1), Some(2))
    );
    let ws = words_of(&log);
    assert_eq!(ws.len(), 1, "no words for A");
    assert_eq!(ws[0].words_slot(), Some(1));
    assert!(
        ws[0].stamp().seq.get() < recs[1].stamp().seq.get(),
        "W's words first"
    );

    // 4. T holds the revision lab/svc's descriptor names, T2 an earlier one
    // alone. Both read both owners.
    let t_held = [tc()];
    let t2_held = [tc_earlier()];
    let svc_d = descriptor_of(&ts, SVC, None).await;
    let plain_d = descriptor_of(&ts, "lab/plain", None).await;
    let svc_state = get_state(&ts, SVC).await.seed(SVC);
    let plain_state = get_state(&ts, "lab/plain").await.seed("lab/plain");
    let words_at = |seed: &m2::Seed, slot: u8| {
        seed.words
            .as_ref()
            .and_then(|w| w.live.iter().find(|e| e.slot == Some(slot)))
            .map(|e| e.record.clone())
    };
    // What a tool shows of a slot: the rule's name and its summary, or the
    // words it takes, or the numbers.
    let shows = |seed: &m2::Seed, slot: u8, table: Option<&[Rule]>| -> String {
        let rec = record_at(seed, slot);
        match rec.name(table) {
            m2::Naming::Named(n) => format!("{} {:?}: {}", n.rule.name, n.labels, n.summary),
            m2::Naming::ByWords { alert_key } => {
                let words = words_at(seed, slot);
                match m2::words_taken(Some(&Ok::<_, ()>(rec)), words.as_ref()) {
                    Ok(_) => words.unwrap().unwrap().summary,
                    Err(_) => am::key_hex(alert_key),
                }
            }
            m2::Naming::Unnamed(why) => format!(
                "rule {}, labels {} ({})",
                match rec.rule {
                    RuleIndex::Table(i) => i.to_string(),
                    RuleIndex::NotAvailable => "?".into(),
                },
                rec.labels.map_or("-".into(), |l| l.to_string()),
                why.as_str()
            ),
        }
    };
    let t_svc = table_for(&svc_d, &t_held);
    let t2_svc = table_for(&svc_d, &t2_held);
    assert_eq!(t_svc.as_deref(), Some(&t1[..]));
    assert_eq!(t2_svc, None, "T2 does not hold the revision named");
    assert_eq!(
        shows(&svc_state, 0, t_svc.as_deref()),
        r#"link_down [("peer", "r2"), ("port", "eth0")]: Link to r2 on eth0 is down"#
    );
    assert_eq!(
        shows(&plain_state, 0, table_for(&plain_d, &t_held).as_deref()),
        "Link to r2 flaps"
    );
    for (seed, table) in [(&svc_state, t_svc.as_deref()), (&plain_state, None)] {
        assert_eq!(
            record_at(seed, 0)
                .name(table)
                .alert_key()
                .map(am::key_hex)
                .as_deref(),
            Some(A_KEY),
            "both A's have one alert key"
        );
    }
    assert_eq!(
        shows(&svc_state, 0, t2_svc.as_deref()),
        "rule 1, labels 2 (no_table)",
        "unnamed, never door_open"
    );
    assert_eq!(
        table_of(&tc_earlier())[1].name,
        "door_open",
        "the name T2's own revision has at index 1"
    );
    assert_eq!(
        shows(&plain_state, 0, None),
        "Link to r2 flaps",
        "words need no table"
    );
    assert_eq!(shows(&svc_state, 1, t2_svc.as_deref()), "/var is full");
    for (address, seed, d) in [
        (SVC, &svc_state, &svc_d),
        ("lab/plain", &plain_state, &plain_d),
    ] {
        let (_, presence) = listed(d);
        for slot in live_slots(seed) {
            let (firing, _) = judge_get(seed, slot, presence, &stated(d), &am::horizon());
            assert_eq!(
                pair(firing),
                (Answer::Yes, Reason::Firing),
                "{address} slot {slot}: a name plays no part"
            );
        }
    }

    // 5. lab/liar puts a record by words with A's key on slot 0, and
    // disk_full's words on words/0: no, mismatch.
    let liar = client(&ep).await;
    let heard = record(&ts, "lab/liar").await;
    let stamp = |seq| Stamp {
        run: Run::U16(9),
        seq: Seq::new(seq).unwrap(),
    };
    let lie_words = alarm_d().words();
    let lie_rec = Record::new(
        State::Firing,
        Severity::Warning,
        m2::Fields {
            rule: RuleIndex::NotAvailable,
            labels: None,
            argument: Argument::NotAvailable,
            key: Some(0xa659_f813_308a_d1da),
        },
        Some(now_s()),
    );
    let words_json = serde_json::to_vec(&lie_words.to_json()).unwrap();
    let words_put = Envelope::mutation(Tag::NONE, stamp(1), &words_json).write();
    let rec_put = lie_rec.put(stamp(2), EPOCH).unwrap();
    let wk = m2::words_key("lab", "liar", 0).unwrap().to_string();
    let rk = m2::alarm_key("lab", "liar", 0).unwrap().to_string();
    eventually("T's subscription reaches the liar", || async {
        let p = liar.declare_publisher(rk.clone()).await.unwrap();
        p.matching_status().await.unwrap().matching()
    })
    .await;
    liar.put(wk, words_put).await.unwrap();
    liar.put(rk, rec_put).await.unwrap();
    eventually("T hears the lie", || async {
        let log = snapshot(&heard.log);
        !words_of(&log).is_empty() && !records(&log).is_empty()
    })
    .await;
    let log = snapshot(&heard.log);
    let lie = records(&log)[0].record();
    let taken = m2::words_taken(
        Some(&Ok::<_, ()>(lie)),
        Some(&Ok::<_, ()>(words_of(&log)[0].words())),
    );
    assert_eq!(
        pair(m2::words_derive(taken)),
        (Answer::No, Reason::Mismatch)
    );
    assert_eq!(
        lie.name(None),
        m2::Naming::ByWords {
            alert_key: 0xa659_f813_308a_d1da
        },
        "shown by its alert key, with no words"
    );
    plain_svc.close().await.unwrap();
    svc.close().await.unwrap();
}

/// A notifier that follows this text (§2.16): what it told, by reference.
#[derive(Default)]
struct Notifier {
    told: std::collections::BTreeMap<(String, String, String), Severity>,
    reports: Vec<String>,
}

impl Notifier {
    /// The alarm of `reference` reads firing at `severity`: told, unless
    /// that is what it told.
    fn firing(&mut self, reference: (&str, &str, &str), severity: Severity) {
        let k = (
            reference.0.to_owned(),
            reference.1.to_owned(),
            reference.2.to_owned(),
        );
        if self.told.get(&k) != Some(&severity) {
            self.reports
                .push(format!("{k:?} firing at {}", severity.as_str()));
            self.told.insert(k, severity);
        }
    }
}

/// §12: a service moves from `alarms.v1`, adopting A from its v1 record
/// onto slot 0, its `at` the `timestamp`'s second; nothing moves on the
/// `alarms.v1` key; an acknowledgement made against v1 still applies, and
/// a notifier tells nothing again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s12_a_service_moves_from_alarms_v1() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let v1_log = record_on(&rs, SVC, "alarms.v1").await;
    let path = dir.path().join("alarms.json");

    // Setup: lab/svc on alarms.v1 holds A firing at critical, its
    // `timestamp` t0 123 ms past its second, in its record.
    let t0 = u64::try_from(now_s() - 30).unwrap() * 1000 + 123;
    let doc = serde_json::json!({
        "timestamp": t0, "rule": "link_down", "severity": "critical", "state": "firing",
        "summary": "Link to r2 on eth0 is down", "labels": {"peer": "r2", "port": "eth0"},
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"profile": "alarms.v1", "alarms": {A_KEY: doc}}))
            .unwrap(),
    )
    .unwrap();
    let os = client(&ep).await;
    let mut ob = ServiceBuilder::new(&os, config(SVC));
    let tc_iface = tc().iface().clone();
    ob.implement(tc()).unwrap();
    ob.expose(&tc_iface, "state/qdisc").unwrap();
    let old = ob
        .alarms(av1::Startup::default().with_record(Arc::new(av1::FileRecord::new(&path))))
        .await
        .unwrap();
    assert_eq!(old.adoption().adopted, [A_KEY.to_owned()]);
    let old_svc = ob.start().await.unwrap();
    // N reported A firing, by its reference.
    let mut n = Notifier::default();
    let v1c = Consumer::for_tool(
        &rs,
        av1::implementation().shared_contract(),
        &[SVC],
        &Default::default(),
    )
    .unwrap();
    let answer = eventually_some("A under alarms.v1", || async {
        let a = v1c.get_answer(av1::ALARMS, None, T).await.ok()?;
        (!a.current.is_empty()).then_some(a)
    })
    .await;
    let Current::Value { sample, .. } = &answer.current[0] else {
        panic!("a value")
    };
    let told = am::decode(&sample.payload().to_bytes()).unwrap();
    assert_eq!(told.timestamp, t0);
    n.firing(("lab", "svc", A_KEY), told.severity);
    assert_eq!(n.reports.len(), 1);
    // The acknowledgement, made against t0.
    let acked = Some(i128::from(t0));

    // 1. The service restarts as an alarms.v2 instance, adopting from its
    // record.
    drop(old);
    drop(old_svc);
    os.close().await.unwrap();
    let v1_before = snapshot(&v1_log.log).len();
    let os = client(&ep).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        Startup::default().with_record(Arc::new(FileRecord::new(&path))),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let adoption = a.adoption();
    assert_eq!(adoption.source, Source::RecordV1);
    assert_eq!(adoption.slots(m2::Adoption::Adopted), [0]);
    let svc = b.start().await.unwrap();
    let d = descriptor_of(&rs, SVC, Some(svc.instance())).await;
    assert_eq!(listed(&d).0, Some(Major::V2));
    assert!(
        d.interfaces.iter().all(|e| e.iface != "alarms.v1"),
        "not alarms.v1"
    );
    let recs = at_least(&s.log, 1).await;
    let put = recs[0].record();
    assert_eq!(recs[0].slot(), Some(0), "…/alarms.v2/state/alarms/0");
    assert_eq!(
        (put.state, put.severity, put.rule, put.labels),
        (
            State::Firing,
            Some(Severity::Critical),
            RuleIndex::Table(1),
            Some(2)
        )
    );
    assert_eq!(
        put.at,
        Some(i64::try_from(t0 / 1000).unwrap()),
        "t0's second"
    );

    // 2. T judges A, by its reference: firing; the acknowledgement still
    // applies; N tells nothing.
    let (_, presence) = listed(&d);
    let seed = get_alarms(&rs, SVC).await.seed(SVC);
    let t1 = table_of(&tc());
    let slot = live_slots(&seed)
        .into_iter()
        .find(|s| {
            record_at(&seed, *s)
                .name(Some(&t1))
                .alert_key()
                .map(am::key_hex)
                .as_deref()
                == Some(A_KEY)
        })
        .expect("the alarm whose record derives A's key");
    let (firing, _) = judge_get(&seed, slot, presence, &stated(&d), &am::horizon());
    assert_eq!(pair(firing), (Answer::Yes, Reason::Firing));
    assert!(
        m2::acknowledged(acked, record_at(&seed, slot).at),
        "at ≤ t0"
    );
    let applied = reads(&snapshot(&s.log), 0, Some(&t1));
    for report in &applied[0].reports {
        assert_eq!(report.change, m2::Change::Firing);
        let key = am::key_hex(report.alert_key.expect("named"));
        n.firing(("lab", "svc", &key), put.severity.unwrap());
    }
    assert_eq!(n.reports.len(), 1, "N reports nothing: {:?}", n.reports);
    // The new instance put and deleted nothing on an alarms.v1 key.
    a.raise(alarm_a(Severity::Critical)).await.unwrap();
    get_alarms(&rs, SVC).await;
    let v1_after = snapshot(&v1_log.log);
    assert!(
        v1_after[v1_before..].iter().all(Rec::is_instance),
        "nothing on alarms.v1: {:?}",
        &v1_after[v1_before..]
    );
    let Ok(Found::V2(kept)) = FileRecord::parse(&std::fs::read_to_string(&path).unwrap()) else {
        panic!("the record is the alarms.v2 owner's now")
    };
    assert_eq!(kept.len(), 1);
    svc.close().await.unwrap();
}

/// §2.9 at a re-mint: each firing record put again on its slot under the
/// new run, `at` kept, before the new instance's token; a resolved record
/// is the earlier instance's, left out of the new instance's answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_epoch_puts_each_firing_record_again() {
    let dir = tempfile::tempdir().unwrap();
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let s = record(&rs, SVC).await;
    let (b, a) = owner(
        &os,
        SVC,
        av2::implementation(),
        Startup::default(),
        &run_file(dir.path(), "svc"),
        Some(tc()),
    )
    .await;
    let mut svc = b.start().await.unwrap();
    descriptor_of(&rs, SVC, None).await;
    let a_rec = a.raise(alarm_a(Severity::Warning)).await.unwrap().record;
    a.raise(alarm_b(Severity::Warning)).await.unwrap();
    a.resolve(B_KEY).await.unwrap().unwrap();
    at_least(&s.log, 3).await;
    let old_run = svc.minter().run();
    let n = snapshot(&s.log).len();

    let instance = svc.new_epoch().await.unwrap();
    let new_run = svc.minter().run();
    assert_ne!(new_run, old_run);
    at_new_token(&s.log, n).await;
    let seed = get_alarms(&rs, SVC).await.seed(SVC);
    assert_eq!(live_slots(&seed), [0], "B's resolved record left out");
    let e = entry(&seed, 0).unwrap();
    assert_eq!(e.stamp.run, new_run, "put again under the new run");
    assert_eq!(e.record, Ok(a_rec), "unchanged, `at` kept");
    let held = a.held().await;
    assert_eq!(held.keys().copied().collect::<Vec<_>>(), [0]);
    // Slot 1 is free: B comes back on it, the lowest free.
    assert_eq!(a.raise(alarm_b(Severity::Warning)).await.unwrap().slot, 1);
    let d = descriptor_of(&rs, SVC, Some(&instance)).await;
    assert_eq!(d.stated_run(), Some(new_run));
    svc.close().await.unwrap();
}
