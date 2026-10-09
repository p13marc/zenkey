//! `spec/profiles/hostid/scenarios.md` §1–§5 (`hostid.v1`, #719, PB).
//!
//! Every service runs against a **root** of its own, a temporary directory
//! standing in for `/`, through the runtime's seam
//! ([`HostIdSource::at`]): no test reads the host's `/etc/machine-id` or
//! writes its `/var/lib/zk2`. A scenario's **process** is a
//! [`HostIdMinter`], the state a process mints once into (§2.7); a restart
//! is a new one. Failures a test cannot cause by a mode (it runs as root,
//! or the failure is `link(2)`'s) come through the seam's faults, as the
//! scenarios' conventions allow.
//!
//! §6 is a tool's (a doctor's, reading presence and descriptors), and no
//! tool asks it yet.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};

use common::{T, client, imp, router};
use zenkey::hostid::{
    Attempt, DBUS_MACHINE_ID, ETC_MACHINE_ID, HostIdError, HostIdMinter, HostIdSource, SHARED_FILE,
    Step,
};
use zenkey::model::descriptor::Descriptor;
use zenkey::model::grammar::{IfaceId, Name};
use zenkey::model::hostid as derivation;
use zenkey::model::template::Bindings;
use zenkey::presence;
use zenkey::state::StateWriter;
use zenkey::writer::Writer;
use zenkey::{Error, Service, ServiceBuilder, ServiceConfig};
use zenoh::pubsub::Subscriber;
use zenoh::sample::{Sample, SampleKind};

/// The scenarios' machine ids, with their systems.
const M1: &str = "b642b4217b34b1e8d3bd915fc65c4452";
const S1: &str = "h-bbd1aa1db10b";
const M2: &str = "0123456789abcdef0123456789abcdef";
const S2: &str = "h-3f6d94515669";
const M3: &str = "ffffffffffffffffffffffffffffffff";
const S3: &str = "h-504c6767c349";

fn is_root() -> bool {
    // SAFETY: `geteuid` cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// A directory standing in for `/`, with `var/lib` (the scenarios'
/// conventions).
struct Root(tempfile::TempDir);

impl Root {
    fn new() -> Self {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("var/lib")).unwrap();
        Self(d)
    }

    fn at(&self, abs: &str) -> PathBuf {
        self.0.path().join(abs.trim_start_matches('/'))
    }

    fn put(&self, abs: &str, content: &str) {
        let p = self.at(abs);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// A file "holding" an id: the id and a newline.
    fn holding(&self, abs: &str, id: &str) {
        self.put(abs, &format!("{id}\n"));
    }

    fn link(&self, abs: &str, target: &str) {
        let p = self.at(abs);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, p).unwrap();
    }

    fn read(&self, abs: &str) -> Option<String> {
        fs::read_to_string(self.at(abs)).ok()
    }

    fn source(&self) -> HostIdSource {
        HostIdSource::at(self.0.path())
    }

    /// A process on this root.
    fn process(&self) -> HostIdMinter {
        HostIdMinter::new(self.source())
    }

    /// `var/lib/zk2`, a directory the service cannot write (§3 step 1): by
    /// its mode, or, as root, through the seam.
    fn unwritable(&self) -> HostIdSource {
        let dir = self.at("/var/lib/zk2");
        fs::create_dir_all(&dir).unwrap();
        if is_root() {
            self.source().with_fault(Step::CreateTemp, libc::EACCES)
        } else {
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
            self.source()
        }
    }

    /// `etc/machine-id` holding M1, but unreadable (§3 step 3).
    fn unreadable_m1(&self) -> HostIdSource {
        self.holding(ETC_MACHINE_ID, M1);
        if is_root() {
            self.source()
                .with_fault(Step::Open(ETC_MACHINE_ID), libc::EACCES)
        } else {
            fs::set_permissions(self.at(ETC_MACHINE_ID), fs::Permissions::from_mode(0o000))
                .unwrap();
            self.source()
        }
    }

    /// Every entry under the root, with its content: "nothing is written".
    fn tree(&self) -> BTreeSet<(PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, out: &mut BTreeSet<(PathBuf, Vec<u8>)>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.insert((p.clone(), Vec::new()));
                    walk(&p, out);
                } else {
                    out.insert((p.clone(), fs::read(&p).unwrap_or_default()));
                }
            }
        }
        let mut out = BTreeSet::new();
        walk(self.0.path(), &mut out);
        out
    }
}

impl Drop for Root {
    /// Gives the modes back, so the directory can be removed.
    fn drop(&mut self) {
        fn open_up(p: &Path) {
            if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) {
                let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
                for e in fs::read_dir(p).into_iter().flatten().flatten() {
                    open_up(&e.path());
                }
            } else {
                let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o644));
            }
        }
        open_up(self.0.path());
    }
}

fn sysinfo_iface() -> IfaceId {
    "sysinfo.v1".parse().unwrap()
}

fn name(s: &str) -> Name {
    Name::new("service", s).unwrap()
}

/// `@hostid.v1/<service>`, as the scenarios configure it.
fn minted(service: &str) -> ServiceConfig {
    ServiceConfig::minted(name(service))
}

fn ephemeral(service: &str) -> ServiceConfig {
    ServiceConfig::minted_ephemeral(name(service))
}

/// A running `sysinfo`, with what it writes.
struct Running {
    svc: Service,
    _cpu: Writer,
    _up: StateWriter,
}

/// Brings up `sysinfo` (the scenarios' service): an owner of `sysinfo.v1`,
/// its stream and its state declared and written. A builder whose address
/// could not be resolved declares nothing, and `start` says why.
async fn sysinfo(b: ServiceBuilder) -> Result<Running, Error> {
    let mut b = b;
    b.implement(imp("sysinfo.v1"))?;
    if b.address().is_err() {
        return Err(b.start().await.err().expect("refused"));
    }
    let iface = sysinfo_iface();
    let cpu = b
        .declare_writer(&iface, "stream/cpu", &Bindings::new())
        .await?;
    let up = b
        .declare_state_writer(&iface, "state/uptime", &Bindings::new())
        .await?;
    up.put("1").await?;
    let svc = b.start().await?;
    cpu.put("42").await?;
    Ok(Running {
        svc,
        _cpu: cpu,
        _up: up,
    })
}

/// What a tool watching through R1 received: every key, payload and
/// attachment, kept for §2.10, and the live tokens.
#[derive(Default)]
struct Seen {
    bytes: Vec<Vec<u8>>,
    live: BTreeSet<String>,
    tokens: Vec<String>,
}

/// A tool on R1 (the scenarios' conventions): subscribed to `zk2/**`, to
/// every `@zk` key, and to the liveliness selector `zk2/*/*/@zk/**`.
struct Tool {
    session: zenoh::Session,
    seen: Arc<Mutex<Seen>>,
    _subs: Vec<Subscriber<()>>,
}

impl Tool {
    async fn new(ep: &str) -> Self {
        let session = client(ep).await;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let mut subs = Vec::new();
        for sel in ["zk2/**", "zk2/*/*/@zk/**"] {
            let s = Arc::clone(&seen);
            subs.push(
                session
                    .declare_subscriber(sel)
                    .callback(move |x: Sample| s.lock().unwrap().record(&x))
                    .await
                    .unwrap(),
            );
        }
        let s = Arc::clone(&seen);
        subs.push(
            session
                .liveliness()
                .declare_subscriber("zk2/*/*/@zk/**")
                .history(true)
                .callback(move |x: Sample| {
                    let mut s = s.lock().unwrap();
                    s.record(&x);
                    let k = x.key_expr().as_str().to_owned();
                    match x.kind() {
                        SampleKind::Put => {
                            s.live.insert(k.clone());
                            s.tokens.push(k);
                        }
                        SampleKind::Delete => {
                            s.live.remove(&k);
                        }
                    }
                })
                .await
                .unwrap(),
        );
        Self {
            session,
            seen,
            _subs: subs,
        }
    }

    /// Waits for `svc`'s instance token.
    async fn sees(&self, svc: &Service) {
        let key = svc.instance_key().unwrap().to_string();
        self.sees_key(&key).await;
    }

    async fn sees_key(&self, key: &str) {
        common::eventually(&format!("the token {key}"), || async {
            self.seen.lock().unwrap().live.contains(key)
        })
        .await;
    }

    /// GETs `svc`'s descriptor, keeping its bytes.
    async fn descriptor(&self, svc: &Service) -> Descriptor {
        let found = presence::descriptor(&self.session, svc.address(), svc.instance(), T)
            .await
            .unwrap();
        if let presence::Found::Descriptor(_, bytes) = &found {
            self.seen.lock().unwrap().bytes.push(bytes.clone());
        }
        found.into_descriptor().unwrap()
    }

    fn live(&self) -> BTreeSet<String> {
        self.seen.lock().unwrap().live.clone()
    }

    /// Every token ever seen.
    fn tokens(&self) -> Vec<String> {
        self.seen.lock().unwrap().tokens.clone()
    }

    /// Every `@zk` key a sample was put on (descriptors).
    fn zk_puts(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .bytes
            .iter()
            .filter(|b| b.windows(4).any(|w| w == b"/@zk"))
            .count()
    }
}

impl Seen {
    fn record(&mut self, s: &Sample) {
        self.bytes.push(s.key_expr().as_str().as_bytes().to_vec());
        self.bytes.push(s.payload().to_bytes().into_owned());
        if let Some(a) = s.attachment() {
            self.bytes.push(a.to_bytes().into_owned());
        }
    }
}

/// An id in every form §2.10 names: the hex text in either case, the 16
/// bytes, and a UUID spelling in either case.
fn forms(id: &str) -> Vec<Vec<u8>> {
    let lower = id.to_ascii_lowercase();
    let raw: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&lower[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &lower[..8],
        &lower[8..12],
        &lower[12..16],
        &lower[16..20],
        &lower[20..]
    );
    vec![
        lower.clone().into_bytes(),
        lower.to_ascii_uppercase().into_bytes(),
        raw,
        uuid.clone().into_bytes(),
        uuid.to_ascii_uppercase().into_bytes(),
    ]
}

/// §2.10: nothing the tool received holds any of `ids`, in any form.
fn assert_private(tool: &Tool, ids: &[String]) {
    let seen = tool.seen.lock().unwrap();
    assert!(!seen.bytes.is_empty(), "the tool received something");
    for id in ids {
        for form in forms(id) {
            for b in &seen.bytes {
                assert!(
                    !b.windows(form.len()).any(|w| w == form.as_slice()),
                    "an id reached the bus, in {:?}",
                    String::from_utf8_lossy(b)
                );
            }
        }
    }
}

fn outcomes(trail: &[Attempt]) -> Vec<(&'static str, &'static str)> {
    trail.iter().map(|a| (a.path, a.outcome.label())).collect()
}

/// The `hostid.v1` error behind a refusal.
fn hostid_error(e: &Error) -> &HostIdError {
    match e {
        Error::HostId(h) => h,
        other => panic!("not hostid.v1's refusal: {other}"),
    }
}

/// Starts `sysinfo` on a fresh session, waits for its token, reads its
/// descriptor, and stops it.
async fn run(ep: &str, tool: &Tool, process: &HostIdMinter, config: ServiceConfig) -> Descriptor {
    let s = client(ep).await;
    let r = sysinfo(ServiceBuilder::with_hostid(&s, config, process))
        .await
        .expect("it starts");
    tool.sees(&r.svc).await;
    let d = tool.descriptor(&r.svc).await;
    r.svc.close().await.unwrap();
    s.close().await.unwrap();
    d
}

/// Starts `sysinfo`, which must refuse: the `hostid.v1` error.
async fn refused(ep: &str, process: &HostIdMinter, config: ServiceConfig) -> Arc<HostIdError> {
    let s = client(ep).await;
    let e = sysinfo(ServiceBuilder::with_hostid(&s, config, process))
        .await
        .err()
        .expect("it does not start");
    s.close().await.unwrap();
    match e {
        Error::HostId(h) => h,
        other => panic!("not hostid.v1's refusal: {other}"),
    }
}

fn lists_hostid(d: &Descriptor) -> bool {
    d.profiles.iter().any(|p| p == "hostid.v1")
}

fn no_temp_file(root: &Root) {
    let names: Vec<String> = fs::read_dir(root.at("/var/lib/zk2"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.iter().all(|n| !n.starts_with(".hostid.")),
        "a temporary file remains: {names:?}"
    );
}

/// §1: the inputs in order, the salt, the declaration, and privacy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_the_input_order() {
    let (_r1, ep) = router(None).await;
    let tool = Tool::new(&ep).await;
    let mut ids: Vec<String> = [M1, M2, M3].map(str::to_owned).to_vec();

    // 1. M1 wins over M2; no shared file is created.
    let r = Root::new();
    r.holding(ETC_MACHINE_ID, M1);
    r.holding(DBUS_MACHINE_ID, M2);
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    assert_eq!(d.service, format!("{S1}/sysinfo"));
    assert!(
        tool.tokens()
            .iter()
            .any(|t| t.starts_with(&format!("zk2/{S1}/sysinfo/@zk/instance/")))
    );
    assert!(lists_hostid(&d), "{:?}", d.profiles);
    // §2.13: the host name and the session's zid, for display.
    assert_eq!(
        d.meta.get("host").and_then(|h| h.as_str()),
        zenkey::hostid::hostname().as_deref()
    );
    assert!(d.meta.contains_key("zid"));
    assert!(!r.at("/var/lib/zk2").exists());

    // 2. `uninitialized` is skipped.
    let r = Root::new();
    r.put(ETC_MACHINE_ID, "uninitialized\n");
    r.holding(DBUS_MACHINE_ID, M2);
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    assert_eq!(d.service, format!("{S2}/sysinfo"));

    // 3. The shared file, read and left unchanged.
    let r = Root::new();
    r.put(ETC_MACHINE_ID, "");
    r.holding(SHARED_FILE, M3);
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    assert_eq!(d.service, format!("{S3}/sysinfo"));
    assert_eq!(r.read(SHARED_FILE), Some(format!("{M3}\n")));

    // 4. Neither file yields an id: the shared file is created.
    let r = Root::new();
    r.put(ETC_MACHINE_ID, &"0".repeat(32));
    r.put(DBUS_MACHINE_ID, "not-a-machine-id\n");
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    let content = r.read(SHARED_FILE).expect("created");
    assert_eq!(content.len(), 33);
    assert!(content.ends_with('\n'));
    assert!(
        content[..32]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_eq!(
        fs::metadata(r.at(SHARED_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    no_temp_file(&r);
    assert_eq!(
        d.service,
        format!("{}/sysinfo", derivation::system(&content).unwrap())
    );
    assert!(lists_hostid(&d));
    ids.push(content[..32].to_owned());

    // 5. The control: a literal name in the minted shape is literal.
    let r = Root::new();
    r.holding(ETC_MACHINE_ID, M1);
    r.holding(DBUS_MACHINE_ID, M2);
    let process = r.process();
    let d = run(
        &ep,
        &tool,
        &process,
        ServiceConfig::new(format!("{S1}/sysinfo").parse().unwrap()),
    )
    .await;
    assert_eq!(d.service, format!("{S1}/sysinfo"));
    assert!(!lists_hostid(&d), "{:?}", d.profiles);
    assert!(
        !d.meta.contains_key("host"),
        "only as the deployment states it"
    );
    assert!(process.minted().is_none(), "nothing read");

    // 6. (0.2, F-94) An absolute link resolves in the root: `/etc/machine-id`
    // is absent there, whatever the host's own holds.
    let r = Root::new();
    r.link(DBUS_MACHINE_ID, "/etc/machine-id");
    r.holding(SHARED_FILE, M3);
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    assert_eq!(d.service, format!("{S3}/sysinfo"));

    // 7. (0.2, F-94) And it reaches what the root holds.
    let r = Root::new();
    r.put(ETC_MACHINE_ID, "uninitialized\n");
    r.holding("/srv/machine-id", M2);
    r.link(DBUS_MACHINE_ID, "/srv/machine-id");
    let d = run(&ep, &tool, &r.process(), minted("sysinfo")).await;
    assert_eq!(d.service, format!("{S2}/sysinfo"));

    // Every step: no id on the bus, in any form (§2.10).
    assert_private(&tool, &ids);
}

/// §2: 16 racers, each asking for a minted system with nothing cached
/// between them, 100 rounds without `var/lib/zk2`, then 100 with it
/// present and empty.
#[test]
fn s2_the_shared_file_under_racers() {
    const RACERS: usize = 16;
    const ROUNDS: usize = 100;
    let mut systems = BTreeSet::new();
    for dir_present in [false, true] {
        for round in 0..ROUNDS {
            let r = Root::new();
            if dir_present {
                fs::create_dir(r.at("/var/lib/zk2")).unwrap();
            }
            let barrier = Barrier::new(RACERS);
            let got: Vec<Name> = std::thread::scope(|s| {
                let racers: Vec<_> = (0..RACERS)
                    .map(|i| {
                        let (r, barrier) = (&r, &barrier);
                        s.spawn(move || {
                            // A process of its own: nothing cached.
                            let process = r.process();
                            let config = minted(&format!("svc-{i}"));
                            barrier.wait();
                            config.resolve_with(&process).unwrap().address.system
                        })
                    })
                    .collect();
                racers.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let content = r.read(SHARED_FILE).expect("one shared file");
            assert_eq!(content.len(), 33, "round {round}");
            assert!(content.ends_with('\n'));
            assert!(derivation::normalise(&content).is_some());
            assert_eq!(content[..32].to_ascii_lowercase(), content[..32]);
            let want = derivation::system(&content).unwrap();
            assert!(
                got.iter().all(|s| *s == want),
                "round {round}: every racer has the shared file's system: {got:?}"
            );
            let names: Vec<_> = fs::read_dir(r.at("/var/lib/zk2"))
                .unwrap()
                .flatten()
                .map(|e| e.file_name())
                .collect();
            assert_eq!(names, ["hostid"], "round {round}: no temporary file");
            systems.insert(want);
        }
    }
    assert_eq!(
        systems.len(),
        2 * ROUNDS,
        "rounds on different roots differ"
    );
}

/// §3: fail closed, naming every path; watched through R1, nothing of a
/// refused service appears, where the control's token does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_fail_closed() {
    let (_r1, ep) = router(None).await;
    let tool = Tool::new(&ep).await;

    // 1. No machine-id file, and `var/lib/zk2` not writable.
    let r1 = Root::new();
    let e = refused(&ep, &HostIdMinter::new(r1.unwritable()), minted("sysinfo")).await;
    assert_eq!(
        outcomes(e.trail()),
        [
            (ETC_MACHINE_ID, "absent"),
            (DBUS_MACHINE_ID, "absent"),
            (SHARED_FILE, "not created")
        ]
    );
    assert!(
        e.trail()[2].outcome.error().is_some(),
        "with the OS's error"
    );

    // 2. A shared file without an id: refused, and kept.
    let r = Root::new();
    r.put(SHARED_FILE, "garbage");
    let e = refused(&ep, &r.process(), minted("sysinfo")).await;
    assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "refused"));
    assert_eq!(r.read(SHARED_FILE).as_deref(), Some("garbage"));

    // 3. An unreadable machine id fails closed, and nothing is created.
    let r = Root::new();
    let e = refused(
        &ep,
        &HostIdMinter::new(r.unreadable_m1()),
        minted("sysinfo"),
    )
    .await;
    assert_eq!(outcomes(e.trail()), [(ETC_MACHINE_ID, "unreadable")]);
    assert_eq!(
        e.trail()[0]
            .outcome
            .error()
            .and_then(std::io::Error::raw_os_error),
        Some(libc::EACCES)
    );
    assert!(!r.at("/var/lib/zk2").exists());

    // 4. `link(2)` refused: as 1, and no file of either name.
    let r = Root::new();
    let src = r.source().with_fault(Step::Link, libc::EPERM);
    let e = refused(&ep, &HostIdMinter::new(src), minted("sysinfo")).await;
    assert_eq!(
        outcomes(e.trail()),
        [
            (ETC_MACHINE_ID, "absent"),
            (DBUS_MACHINE_ID, "absent"),
            (SHARED_FILE, "not created")
        ]
    );
    assert_eq!(
        e.trail()[2]
            .outcome
            .error()
            .and_then(std::io::Error::raw_os_error),
        Some(libc::EPERM)
    );
    assert_eq!(fs::read_dir(r.at("/var/lib/zk2")).unwrap().count(), 0);

    // Watched through R1: nothing of the refused services, for the wait.
    tokio::time::sleep(T).await;
    assert!(tool.tokens().is_empty(), "{:?}", tool.tokens());
    assert_eq!(tool.zk_puts(), 0, "no descriptor put");

    // 5. The control: step 1's root with M1. Its token appears.
    r1.holding(ETC_MACHINE_ID, M1);
    let d = run(
        &ep,
        &tool,
        &HostIdMinter::new(r1.unwritable()),
        minted("sysinfo"),
    )
    .await;
    assert_eq!(d.service, format!("{S1}/sysinfo"));
}

/// §4: ephemeral, by opt-in, as a fallback that replaces exactly one
/// refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_ephemeral() {
    let (_r1, ep) = router(None).await;
    let tool = Tool::new(&ep).await;

    // 1. Two runs: both start, each says so, and nothing is written.
    let r = Root::new();
    let src = r.unwritable();
    let before = r.tree();
    let mut systems = Vec::new();
    for _ in 0..2 {
        let process = HostIdMinter::new(src.clone());
        let s = client(&ep).await;
        let (b, log) = logged(|| ServiceBuilder::with_hostid(&s, ephemeral("sysinfo"), &process));
        assert!(log.contains("EPHEMERAL"), "{log}");
        for path in [ETC_MACHINE_ID, DBUS_MACHINE_ID] {
            assert!(log.contains(&format!("{path}: absent")), "{log}");
        }
        assert!(
            log.contains(&format!("{SHARED_FILE}: not created")),
            "{log}"
        );
        let run = sysinfo(b).await.expect("an ephemeral system starts");
        tool.sees(&run.svc).await;
        let d = tool.descriptor(&run.svc).await;
        assert!(lists_hostid(&d));
        let system = run.svc.address().system.clone();
        assert!(derivation::is_minted_shape(system.as_str()));
        assert!(run.svc.minted().unwrap().is_ephemeral());
        systems.push(system);
        run.svc.close().await.unwrap();
        s.close().await.unwrap();
    }
    assert_ne!(systems[0], systems[1], "a restart mints another");
    assert_eq!(r.tree(), before, "nothing written under the root");

    // 2. One process, two ephemeral services: one system.
    let process = HostIdMinter::new(src.clone());
    let s = client(&ep).await;
    let a = sysinfo(ServiceBuilder::with_hostid(&s, ephemeral("a"), &process))
        .await
        .unwrap();
    let b = sysinfo(ServiceBuilder::with_hostid(&s, ephemeral("b"), &process))
        .await
        .unwrap();
    assert_eq!(a.svc.address().system, b.svc.address().system);
    b.svc.close().await.unwrap();

    // 3. Then one that does not set it: a configuration error.
    let tokens = tool.tokens().len();
    let e = sysinfo(ServiceBuilder::with_hostid(&s, minted("c"), &process))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            hostid_error(&e),
            HostIdError::Setting {
                fixed: true,
                asked: false
            }
        ),
        "{e}"
    );
    tokio::time::sleep(T).await;
    assert!(
        tool.tokens()[tokens..]
            .iter()
            .all(|t| !t.contains("/c/@zk/")),
        "c declares nothing"
    );
    a.svc.close().await.unwrap();
    s.close().await.unwrap();

    // 4. An input that yields an id wins over the ephemeral rung.
    let r = Root::new();
    r.holding(ETC_MACHINE_ID, M1);
    r.holding(DBUS_MACHINE_ID, M2);
    let d = run(&ep, &tool, &r.process(), ephemeral("sysinfo")).await;
    assert_eq!(d.service, format!("{S1}/sysinfo"));

    // 5. Unreadable, or a shared file without an id: ephemeral or not.
    let r = Root::new();
    r.put(SHARED_FILE, "garbage");
    let e = refused(&ep, &r.process(), ephemeral("sysinfo")).await;
    assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "refused"));
    let r = Root::new();
    let e = refused(
        &ep,
        &HostIdMinter::new(r.unreadable_m1()),
        ephemeral("sysinfo"),
    )
    .await;
    assert_eq!(outcomes(e.trail()), [(ETC_MACHINE_ID, "unreadable")]);

    // 6. (0.2, F-96) Another racer's file, gone when read: fails closed.
    let r = Root::new();
    let src = r.source().with_fault(Step::Link, libc::EEXIST);
    let e = refused(&ep, &HostIdMinter::new(src), ephemeral("sysinfo")).await;
    assert_eq!(outcomes(e.trail())[2], (SHARED_FILE, "absent"));
    no_temp_file(&r);
}

/// §5: minted once per run, kept across a re-mint, and read again only at
/// the next start.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_minted_once_per_run() {
    let (_r1, ep) = router(None).await;
    let tool = Tool::new(&ep).await;
    let r = Root::new();
    r.holding(ETC_MACHINE_ID, M1);

    // 1. A re-mint: a new instance of the same address, make-before-break.
    let process = r.process();
    let s = client(&ep).await;
    let mut run1 = sysinfo(ServiceBuilder::with_hostid(&s, minted("sysinfo"), &process))
        .await
        .unwrap();
    tool.sees(&run1.svc).await;
    let old = run1.svc.instance_key().unwrap().to_string();
    run1.svc.new_epoch().await.unwrap();
    let new = run1.svc.instance_key().unwrap().to_string();
    assert_ne!(old, new);
    assert!(new.starts_with(&format!("zk2/{S1}/sysinfo/@zk/instance/")));
    tool.sees_key(&new).await;
    common::eventually("the old instance goes", || async {
        !tool.live().contains(&old)
    })
    .await;

    // 2. M2 now, and a re-mint, and a second service: still M1's system.
    r.holding(ETC_MACHINE_ID, M2);
    run1.svc.new_epoch().await.unwrap();
    assert_eq!(run1.svc.address().system.as_str(), S1);
    let mut b = ServiceBuilder::with_hostid(
        &s,
        minted("logger").bind("src", &["self.system/sysinfo"]),
        &process,
    );
    b.require("src", sysinfo_iface(), false);
    let logger = b.start().await.unwrap();
    assert_eq!(logger.address().to_string(), format!("{S1}/logger"));
    // R1 (core 0.20): `self.system` is the minted system. The token first:
    // the descriptor's queryable is declared before it (core §8.2).
    tool.sees(&logger).await;
    let d = tool.descriptor(&logger).await;
    assert_eq!(d.requires[0].bindings, [format!("{S1}/sysinfo")]);
    tool.sees(&run1.svc).await;
    logger.close().await.unwrap();
    run1.svc.close().await.unwrap();
    s.close().await.unwrap();

    // 3. Restarted: M2's system, and nothing left under M1's.
    let process = r.process();
    let s = client(&ep).await;
    let run1 = sysinfo(ServiceBuilder::with_hostid(&s, minted("sysinfo"), &process))
        .await
        .unwrap();
    let logger = ServiceBuilder::with_hostid(&s, minted("logger"), &process)
        .start()
        .await
        .unwrap();
    assert_eq!(run1.svc.address().system.as_str(), S2);
    assert_eq!(logger.address().system.as_str(), S2);
    tool.sees(&logger).await;
    common::eventually("nothing under M1's system", || async {
        tool.live()
            .iter()
            .all(|t| !t.starts_with(&format!("zk2/{S1}/")))
    })
    .await;
    logger.close().await.unwrap();
    run1.svc.close().await.unwrap();
    s.close().await.unwrap();

    // 4. Ephemeral: a re-mint keeps it, a restart mints another.
    let e = Root::new();
    let src = e.unwritable();
    let s = client(&ep).await;
    let process = HostIdMinter::new(src.clone());
    let mut run = sysinfo(ServiceBuilder::with_hostid(
        &s,
        ephemeral("sysinfo"),
        &process,
    ))
    .await
    .unwrap();
    let first = run.svc.address().system.clone();
    run.svc.new_epoch().await.unwrap();
    assert_eq!(run.svc.address().system, first);
    run.svc.close().await.unwrap();
    let process = HostIdMinter::new(src);
    let run = sysinfo(ServiceBuilder::with_hostid(
        &s,
        ephemeral("sysinfo"),
        &process,
    ))
    .await
    .unwrap();
    assert_ne!(run.svc.address().system, first);
    run.svc.close().await.unwrap();

    // 5. A process with only a literal service reads no input.
    let u = Root::new();
    let process = HostIdMinter::new(u.unreadable_m1());
    let run = sysinfo(ServiceBuilder::with_hostid(
        &s,
        ServiceConfig::new("vehicle-01/sysinfo".parse().unwrap()),
        &process,
    ))
    .await
    .expect("a literal service starts");
    assert!(process.minted().is_none());
    run.svc.close().await.unwrap();

    // 6. (0.2, F-95) A failure mints nothing: a later service reads the
    // inputs again. The first ask fixed the setting all the same.
    let f = Root::new();
    let process = HostIdMinter::new(f.unwritable());
    refused(&ep, &process, minted("a")).await;
    f.holding(ETC_MACHINE_ID, M1);
    let b = sysinfo(ServiceBuilder::with_hostid(&s, minted("b"), &process))
        .await
        .expect("the host was fixed");
    assert_eq!(b.svc.address().system.as_str(), S1);
    let e = refused(&ep, &process, ephemeral("c")).await;
    assert!(
        matches!(
            *e,
            HostIdError::Setting {
                fixed: false,
                asked: true
            }
        ),
        "{e}"
    );
    b.svc.close().await.unwrap();
    s.close().await.unwrap();
}

/// What a start logged, through a subscriber of its own on this thread.
fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let w = buf.clone();
    let sub = tracing_subscriber::fmt()
        .with_writer(move || w.clone())
        .with_ansi(false)
        .finish();
    let out = tracing::subscriber::with_default(sub, f);
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    (out, text)
}
