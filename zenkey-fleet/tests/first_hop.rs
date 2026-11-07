//! `spec/scenarios/constrained.md` §10 from the tools' side (#982 H3;
//! observer §3.11): a device on a first hop, told to the reader by the
//! face policy that lists it on a device face (`link.v2` §2.5).
//!
//! The device is an owner of `ping.v1` started with
//! `ServiceConfig::first_hop`, a client of the router through a TCP proxy
//! the test cuts and heals, so its session loses its transport and opens a
//! new one under the same instance id, as a radio's does several times a
//! day. On the near side the reader under test: a `PresenceFeed` over a
//! `Monitor` on a watch session of its own, a `LensFeed` following it, a
//! one-shot presence read, and the doctor.
//!
//! - **Link closed, never down**: a complete read showing none of the
//!   device's instance tokens reads its link closed, where an owner the
//!   policy does not list reads down.
//! - **A descriptor kept** across the token's return (observer §3.11's
//!   MAY): the lens reads it again with no GET, and reads the descriptor
//!   again when a put states another run than the one kept (#1074).
//! - **`first-hop-token`**: a device left at its defaults holds interface
//!   tokens and contract queryables, a warning; one started for its first
//!   hop holds neither.
//!
//! The router runs no access control: the face's carve (`link.v2` §2.6)
//! would hide the defaults' tokens from the near side, which is the
//! doctor's clean pole, and `zenkey/tests/constrained.rs` runs it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

mod util;
use util::zk2::{T, addr, client, config, eventually, router, watch_of};

use zenkey::{Implementation, Service, ServiceBuilder};
use zenkey_fleet::report::{CheckId, DoctorSeverity, Judgement};
use zenkey_fleet::{
    AddressStatus, BundleStore, Catalog, DoctorBus, DoctorSpec, FirstHop, LensFeed, LivePresence,
    Monitor, MonitorSpec, PresenceChange, PresenceFeed, PresenceScope, PresenceTransition,
    SampleView, run_doctor,
};
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::grammar::Addr;
use zenkey_model::template::Bindings;
use zenkey_model::wire::Run;

/// The device's address: the RF medium's radio, attached as a device.
const DEVICE: &str = "lab/radio-a";

/// One of the runtime's scenario contracts, by file name.
fn scenario(name: &str) -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts")
        .join(format!("{name}.toml"));
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// `ping.v1`, from the runtime's scenario contracts: one replicated
/// operation, so an owner at its defaults holds an instance token, one
/// interface token and one contract queryable.
fn ping() -> Contract {
    scenario("ping.v1")
}

/// The RF medium's policy (`examples/zk2/link/media/rf.face.toml`) with
/// `lab/radio-a` attached as a device on face `radio`, as
/// `tests/acl_plans.rs` attaches it (#982 H1).
fn device_policy() -> zenkey_model::link::FacePolicy {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/link/media/rf.face.toml");
    let rf = std::fs::read_to_string(path).expect("the RF policy");
    let text = rf
        .replace("services = [\"radio-a\"]", "services = []")
        .replace(
            "far              = \"ground\"\nattach           = \"client\"\n",
            "far              = \"radio-a\"\nattach           = \"device\"\n\
         devices          = [\"lab/radio-a\"]\n",
        );
    assert_ne!(text, rf, "the edit applies");
    let parsed = zenkey_model::link::parse(&text);
    parsed
        .policy
        .unwrap_or_else(|| panic!("the policy reads: {:?}", parsed.findings))
}

/// An owner of `ping.v1` at `address`, through the runtime's own builder,
/// on its first hop or at its defaults.
async fn owner(session: &zenoh::Session, address: &str, first_hop: bool) -> Service {
    owner_of(session, address, first_hop, &[ping()]).await
}

/// An owner of `contracts` at `address`, every resource exposed, on its
/// first hop or at its defaults.
async fn owner_of(
    session: &zenoh::Session,
    address: &str,
    first_hop: bool,
    contracts: &[Contract],
) -> Service {
    let mut cfg = config(address);
    if first_hop {
        cfg = cfg.first_hop();
    }
    let mut b = ServiceBuilder::new(session, cfg);
    for c in contracts {
        b.implement(Implementation::new(c.clone()))
            .expect("implement");
        for r in &c.resources {
            let _ = b.expose(&c.iface, &zenkey::implementation::resource_name(r));
        }
    }
    b.start().await.expect("start")
}

/// The device's token keys `session`'s router shows, by one complete read
/// of its whole `@zk` subtree; `None` when the read was not complete. A
/// token sits below the prefix a caller names (`…/@zk/alive/<iface>/
/// <instance>/<fp16>`), so the selector is the subtree, never the prefix.
async fn device_keys(session: &zenoh::Session) -> Option<Vec<String>> {
    let read = zenkey::presence::liveliness_read(session, &format!("zk2/{DEVICE}/@zk/**"), T)
        .await
        .ok()?;
    read.complete.then_some(read.keys)
}

/// Whether one complete read shows a token of the device's under `prefix`.
async fn reads(session: &zenoh::Session, prefix: &str) -> bool {
    device_keys(session)
        .await
        .is_some_and(|keys| keys.iter().any(|k| k.starts_with(prefix)))
}

/// Whether one complete read shows no token of the device's under
/// `prefix`: an incomplete read says nothing of absence.
async fn reads_none(session: &zenoh::Session, prefix: &str) -> bool {
    device_keys(session)
        .await
        .is_some_and(|keys| !keys.iter().any(|k| k.starts_with(prefix)))
}

/// Folds the feed into `seen`, and each batch into `lens`, until `want`
/// holds of the feed, failing after [`util::SETTLE`] with everything seen.
async fn until(
    feed: &mut PresenceFeed,
    lens: &LensFeed,
    seen: &mut Vec<PresenceTransition>,
    what: &str,
    want: impl Fn(&PresenceFeed) -> bool,
) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !want(feed) {
        match tokio::time::timeout_at(deadline, feed.next()).await {
            Ok(Some(moved)) => {
                lens.on_presence(&moved).await;
                seen.extend(moved);
            }
            Ok(None) => panic!("the monitor's broadcast closed"),
            Err(_) => panic!("never: {what}; saw {seen:#?}"),
        }
    }
}

/// Folds the feed as [`until`] does until `want` holds of the lens's
/// catalog: what its counters are read after, so a status settled one
/// batch before the lens folded the token's return is never what they say
/// (review-bot on #1073).
async fn until_lens(
    feed: &mut PresenceFeed,
    lens: &LensFeed,
    seen: &mut Vec<PresenceTransition>,
    what: &str,
    want: impl Fn(&Catalog) -> bool,
) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !lens.catalog().is_some_and(|c| want(&c)) {
        match tokio::time::timeout_at(deadline, feed.next()).await {
            Ok(Some(moved)) => {
                lens.on_presence(&moved).await;
                seen.extend(moved);
            }
            Ok(None) => panic!("the monitor's broadcast closed"),
            Err(_) => panic!("never: {what}; saw {seen:#?}"),
        }
    }
}

/// `address`'s status moves in `seen`, each `(from, to)`.
fn statuses(seen: &[PresenceTransition], address: &str) -> Vec<(AddressStatus, AddressStatus)> {
    seen.iter()
        .filter(|t| t.address.to_string() == address)
        .filter_map(|t| match t.change {
            PresenceChange::Status { from, to } => Some((from, to)),
            _ => None,
        })
        .collect()
}

// ─── a link the test can cut and heal ───────────────────────────────────────

/// A TCP proxy to a router, as `tests/presence_scenarios.rs`'s: cutting
/// closes every forwarded connection and refuses new ones until healed, so
/// zenoh sees the link drop at once.
struct Link {
    endpoint: String,
    cut: Arc<AtomicBool>,
    conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
    _accept: tokio::task::JoinHandle<()>,
}

impl Link {
    async fn to(upstream: &str) -> Link {
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
        let cut = Arc::new(AtomicBool::new(false));
        let conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>> = Arc::default();
        let (c, cs) = (Arc::clone(&cut), Arc::clone(&conns));
        let accept = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                if c.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let (down_rd, down_wr) = down.into_split();
                let (up_rd, up_wr) = up.into_split();
                let h = tokio::spawn(async move {
                    tokio::join!(pump(down_rd, up_wr), pump(up_rd, down_wr));
                });
                cs.lock().unwrap().push(h.abort_handle());
            }
        });
        Link {
            endpoint,
            cut,
            conns,
            _accept: accept,
        }
    }

    fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
    }
}

/// Forwards `from` to `to` until either side closes.
async fn pump(mut from: tokio::net::tcp::OwnedReadHalf, mut to: tokio::net::tcp::OwnedWriteHalf) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// A client of `endpoint` that retries fast after losing it: the device's
/// session, opening a new transport as soon as the link heals.
async fn retrying_client(endpoint: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .unwrap();
    zenoh::open(c).await.expect("a client")
}

// ─── link closed, never down; a descriptor kept ─────────────────────────────

/// §10 steps 1 and 2, the tools' expected (observer §3.11): the reader
/// told of the device face lists the device from its descriptor, reads its
/// link closed while the proxy is cut — a complete read showing no
/// instance token of it — and never down, and reads it up again at the new
/// transport with the descriptor it kept, no GET made (the MAY). An owner
/// the policy does not list, closed beside it, reads down: the control.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_device_on_a_device_face_is_link_closed_never_down() {
    let (_router, ep) = router(None).await;
    let link = Link::to(&ep).await;
    let device_session = retrying_client(&link.endpoint).await;
    let tool = client(&ep).await;
    let watching = client(&ep).await;
    let hop = FirstHop::of_policy(&device_policy());
    let device = addr(DEVICE);
    assert_eq!(hop.face(&device), Some("radio"));

    let watch = watch_of(&watching, "");
    let spec = MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&PresenceScope::all());
    let monitor = Monitor::start_watch(&watch, spec).await.expect("a monitor");
    let live =
        LivePresence::new(PresenceScope::all()).with_first_hop(hop.clone(), SystemTime::now());
    let (mut feed, seeded) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("a feed");
    // Before the device starts, a complete seed shows none of it: tracked
    // from the policy, its link is closed, not down.
    assert_eq!(feed.presence().status(&device), AddressStatus::LinkClosed);
    let store = BundleStore::new(T);
    let (lens, error) = LensFeed::open(&tool, &store, T).await;
    assert!(error.is_none(), "{error:?}");
    lens.on_presence(&seeded).await;

    // Step 1: the device starts for its first hop, and an owner the policy
    // does not list beside it.
    let ground_session = client(&ep).await;
    let ground = owner(&ground_session, "lab/ground", false).await;
    let svc = owner(&device_session, DEVICE, true).await;
    let mut seen = Vec::new();
    until(
        &mut feed,
        &lens,
        &mut seen,
        "the device and the ground up",
        |f| {
            f.presence().status(&device) == AddressStatus::Up
                && f.presence().status(&addr("lab/ground")) == AddressStatus::Up
        },
    )
    .await;
    eventually("the lens lists the device's interface", || {
        let lens = lens.clone();
        async move {
            lens.catalog()
                .is_some_and(|c| c.provided(&addr(DEVICE)).contains(&ping().iface))
        }
    })
    .await;
    let described = lens.descriptors_read();
    assert!(
        described >= 1,
        "the lens described the device on its way up"
    );

    // Step 2: the link is cut. The device's instance token is withdrawn,
    // the read it owes shows none, and its link is closed.
    link.cut();
    until(
        &mut feed,
        &lens,
        &mut seen,
        "the device's link closed",
        |f| f.presence().status(&device) == AddressStatus::LinkClosed,
    )
    .await;
    let moves = statuses(&seen, DEVICE);
    assert!(
        !moves.iter().any(|(_, to)| *to == AddressStatus::Down),
        "a first-hop device is never down (observer §3.11): {moves:?}"
    );
    // A one-shot read says the same of it, as `service list` and `service
    // show` given the policy print it.
    let observed = zenkey_fleet::observe_presence(&tool, &PresenceScope::all(), T)
        .await
        .expect("a presence read");
    let catalog = Catalog::new(&observed);
    let listed = catalog.first_hop(&hop, &PresenceScope::all());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].address, DEVICE);
    assert_eq!(listed[0].face, "radio");
    assert_eq!(listed[0].status, AddressStatus::LinkClosed);
    let shown = catalog.service_given(&device, &hop);
    assert!(shown.instances.is_empty());
    assert_eq!(
        shown.first_hop.map(|f| f.status),
        Some(AddressStatus::LinkClosed)
    );

    // The new transport: the same instance's token returns, and the lens
    // reads the descriptor it kept, with no GET across the hop.
    link.heal();
    until(&mut feed, &lens, &mut seen, "the device up again", |f| {
        f.presence().status(&device) == AddressStatus::Up
    })
    .await;
    until_lens(
        &mut feed,
        &lens,
        &mut seen,
        "the lens lists the device's interface again",
        |c| c.provided(&device).contains(&ping().iface),
    )
    .await;
    assert_eq!(
        lens.descriptors_read(),
        described,
        "the token's return under the same instance costs no descriptor GET"
    );
    assert_eq!(lens.descriptors_kept().await, 1);
    let catalog = lens.catalog().expect("a catalog");
    assert!(
        catalog.provided(&device).contains(&ping().iface),
        "the kept descriptor lists the device's interface"
    );

    // The control: an owner the policy does not list goes, and reads down.
    ground.close().await.expect("close");
    until(&mut feed, &lens, &mut seen, "the ground down", |f| {
        f.presence().status(&addr("lab/ground")) == AddressStatus::Down
    })
    .await;
    assert_eq!(feed.presence().status(&device), AddressStatus::Up);
    svc.close().await.expect("close");
}

// ─── a descriptor kept by (instance, run) ───────────────────────────────────

/// The run the lens's catalog reads in the device's descriptor.
fn run_listed(lens: &LensFeed) -> Option<String> {
    lens.catalog()?
        .descriptors()
        .find(|d| d.service == DEVICE)?
        .run
        .clone()
}

/// The runs the lens reads in the device's next put on `level`
/// ([`LensFeed::runs_in`]): put until the listener hears one, since a
/// put before the router declares the listener to the device's new
/// transport is dropped at its source.
async fn runs_heard(
    lens: &LensFeed,
    level: &zenkey::writer::Writer,
    heard: &zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>,
) -> Vec<(Addr, Run)> {
    while let Ok(Some(_)) = heard.try_recv() {}
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        level.put("1").await.expect("a put");
        if let Ok(Ok(s)) =
            tokio::time::timeout(Duration::from_millis(200), heard.recv_async()).await
        {
            return lens.runs_in("", &[Arc::new(SampleView::of(&s))]);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the device's put never crossed"
        );
    }
}

/// #1074: the descriptor kept is by (instance, run). A run moves within an
/// instance with no token moving (core §4.3's MAY, `Service::new_run`), and
/// the descriptor put stating it does not cross while the link is closed:
/// the lens reuses what it kept at the token's return, on trust, and reads
/// the descriptor again at the first put stating the new run, once. A put
/// under the run kept costs no GET, and is read once (#1079): the next
/// put under it hands nothing on. Before any token went nothing is read of
/// a put at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_run_moved_while_the_link_was_closed_is_read_again() {
    let (_router, ep) = router(None).await;
    let link = Link::to(&ep).await;
    let device_session = retrying_client(&link.endpoint).await;
    let tool = client(&ep).await;
    let watching = client(&ep).await;
    let listening = client(&ep).await;
    let device = addr(DEVICE);

    let watch = watch_of(&watching, "");
    let spec = MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&PresenceScope::all());
    let monitor = Monitor::start_watch(&watch, spec).await.expect("a monitor");
    let live = LivePresence::new(PresenceScope::all())
        .with_first_hop(FirstHop::of_policy(&device_policy()), SystemTime::now());
    let (mut feed, seeded) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("a feed");
    let store = BundleStore::new(T);
    let (lens, error) = LensFeed::open(&tool, &store, T).await;
    assert!(error.is_none(), "{error:?}");
    lens.on_presence(&seeded).await;
    let heard = listening
        .declare_subscriber(format!("zk2/{DEVICE}/**"))
        .await
        .expect("a subscriber");

    // The device starts for its first hop, and the lens describes it.
    let hop = scenario("hop.v1");
    let svc = owner_of(&device_session, DEVICE, true, std::slice::from_ref(&hop)).await;
    let level = svc
        .writer(&hop.iface, "stream/level", &Bindings::new())
        .await
        .expect("a writer");
    let mut seen = Vec::new();
    until(&mut feed, &lens, &mut seen, "the device up", |f| {
        f.presence().status(&device) == AddressStatus::Up
    })
    .await;
    eventually("the lens reads the device's run", || {
        let lens = lens.clone();
        async move { run_listed(&lens).is_some() }
    })
    .await;
    let first = run_listed(&lens).expect("the descriptor states its run");
    assert!(
        runs_heard(&lens, &level, &heard).await.is_empty(),
        "a descriptor a GET read stands: no put is read against it"
    );
    let described = lens.descriptors_read();

    // The link closes and opens again under the same run: the kept
    // descriptor is reused, and a put under its run reads nothing again.
    link.cut();
    until(&mut feed, &lens, &mut seen, "the link closed", |f| {
        f.presence().status(&device) == AddressStatus::LinkClosed
    })
    .await;
    link.heal();
    until(&mut feed, &lens, &mut seen, "the device up again", |f| {
        f.presence().status(&device) == AddressStatus::Up
    })
    .await;
    until_lens(
        &mut feed,
        &lens,
        &mut seen,
        "the device described again",
        |c| c.descriptors().any(|d| d.service == DEVICE),
    )
    .await;
    assert_eq!(lens.descriptors_kept().await, 1);
    let runs = runs_heard(&lens, &level, &heard).await;
    assert_eq!(
        runs,
        [(device.clone(), Run::from_hex(&first).expect("a run"))]
    );
    assert!(
        !lens.on_runs(&runs).await,
        "its own run reads nothing again"
    );
    assert_eq!(lens.descriptors_read(), described);
    assert!(
        runs_heard(&lens, &level, &heard).await.is_empty(),
        "a run read once is not read again (#1079)"
    );

    // Again, and the run moves while the link is closed: its descriptor
    // put does not cross, the token's return reuses what was kept, and
    // the first put under the new run reads the descriptor again.
    link.cut();
    until(&mut feed, &lens, &mut seen, "the link closed again", |f| {
        f.presence().status(&device) == AddressStatus::LinkClosed
    })
    .await;
    let moved = svc.new_run();
    assert_ne!(moved.hex(), first);
    link.heal();
    until(
        &mut feed,
        &lens,
        &mut seen,
        "the device up a third time",
        |f| f.presence().status(&device) == AddressStatus::Up,
    )
    .await;
    until_lens(
        &mut feed,
        &lens,
        &mut seen,
        "the device described a third time",
        |c| c.descriptors().any(|d| d.service == DEVICE),
    )
    .await;
    assert_eq!(lens.descriptors_kept().await, 2);
    assert_eq!(
        run_listed(&lens),
        Some(first.clone()),
        "reused on trust: nothing crossed to say the run moved"
    );
    let runs = runs_heard(&lens, &level, &heard).await;
    assert_eq!(runs, [(device.clone(), moved)]);
    assert!(
        lens.on_runs(&runs).await,
        "the new run's descriptor is read"
    );
    assert_eq!(lens.descriptors_read(), described + 1, "one GET");
    assert_eq!(run_listed(&lens), Some(moved.hex()));
    assert!(
        runs_heard(&lens, &level, &heard).await.is_empty(),
        "read once: no put is read against it again"
    );
    svc.close().await.expect("close");
}

// ─── first-hop-token ────────────────────────────────────────────────────────

/// The doctor's `first-hop-token` (#982 H3; core §8.1, §8.4): only asked
/// with a face policy; a device left at its defaults is a warning, naming
/// its interface token and its contract queryable; one started for its
/// first hop is clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_hop_token_finds_a_device_at_its_defaults() {
    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let raw = client(&ep).await;
    let bus = DoctorBus {
        session: tool.clone(),
        raw,
        namespace: String::new(),
        watch: None,
    };
    let mut spec = DoctorSpec::new(T);
    spec.grace = Duration::from_millis(300);
    spec.checks = [CheckId::FirstHopToken].into_iter().collect();

    let at_defaults_session = client(&ep).await;
    let svc = owner(&at_defaults_session, DEVICE, false).await;
    eventually("the device's interface token", || {
        reads(&tool, "zk2/lab/radio-a/@zk/alive/ping.v1")
    })
    .await;

    // No face policy: not asked.
    let r = run_doctor(&bus, &BundleStore::new(T), &spec).await;
    let check = r.check(CheckId::FirstHopToken).expect("reported");
    assert!(matches!(check.verdict, Judgement::NotAsked), "{check:?}");

    // The device at its defaults: an interface token and a contract
    // queryable, a warning.
    spec.first_hop = FirstHop::of_policy(&device_policy());
    let r = run_doctor(&bus, &BundleStore::new(T), &spec).await;
    let check = r.check(CheckId::FirstHopToken).expect("reported");
    assert_eq!(check.findings.len(), 1, "{check:?}");
    let f = &check.findings[0];
    assert_eq!(f.severity, DoctorSeverity::Warning);
    assert!(f.subject.starts_with("lab/radio-a@"), "{}", f.subject);
    for needle in [
        "interface token(s) of ping.v1",
        "a contract queryable for ping.v1 sha256:",
        "\"radio\"",
        "first_hop",
    ] {
        assert!(
            f.evidence.contains(needle),
            "{needle:?} not in {}",
            f.evidence
        );
    }
    svc.close().await.expect("close");

    // Started for its first hop: its instance token alone, and clean.
    let first_hop_session = client(&ep).await;
    let svc = owner(&first_hop_session, DEVICE, true).await;
    eventually("the device's instance token", || {
        reads(&tool, "zk2/lab/radio-a/@zk/instance/")
    })
    .await;
    eventually("the defaults' interface token gone", || {
        reads_none(&tool, "zk2/lab/radio-a/@zk/alive/")
    })
    .await;
    let r = run_doctor(&bus, &BundleStore::new(T), &spec).await;
    let check = r.check(CheckId::FirstHopToken).expect("reported");
    assert!(
        matches!(check.verdict, Judgement::NotEstablished { .. }),
        "{check:?}"
    );
    let Judgement::NotEstablished { reason } = &check.verdict else {
        unreachable!()
    };
    assert!(reason.contains("1 device(s)"), "{reason}");
    svc.close().await.expect("close");
}
