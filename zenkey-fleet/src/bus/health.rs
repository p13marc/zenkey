//! `health.v1` (#721, PF), the reads: how well each service says it serves
//! (`spec/profiles/health/v1.md`), read as §2.11's reader reads it, through
//! a session in the deployment's namespace.
//!
//! What is read, in order:
//! 1. with a window, first: a subscription to every status and to the
//!    faults, through the runtime's consumer on the standard contract (R4:
//!    any revision of the major reads the interface). It confirms a status
//!    on this host's monotonic clock (`freshness.v1` §2.5), measures each
//!    stamping clock on live puts (§2.6, ground 2), and records the order
//!    of status puts and faults that §5's clock question reads;
//! 2. presence and every instance's descriptor (core §8.1, §3.3): a service
//!    implements `health.v1` when its descriptor lists it, token or not
//!    (§2.7). Across a constrained face neither crosses, and the
//!    deployment's word stands for them (§2.8, R7);
//! 3. two readings, a grace apart (or the window apart): each one GET of the
//!    owners' `health.v1/state/**`, target `All`, consolidation `Latest`
//!    (core S4), which answers a status and its checks together (§2.4, core
//!    S2) — so a status is never paired with a check from another instant
//!    (§2.2);
//! 4. for one service named and absent, an archive's form of its status
//!    (core §4.4): last-known, never current (S6).
//!
//! Nothing is judged here: [`crate::judge::health`] decides every verdict
//! from the [`HealthObservation`] this returns. The payloads are decoded
//! here, into the levels and words a verdict needs, with the runtime's own
//! types (`zenkey::health::v1`); a payload that does not decode is
//! [`Read::Undecodable`], never a level (§2.6).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey::health::v1;
use zenkey::prost::Message as _;
use zenkey_model::freshness::{ClockMeasure, Observation};
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId, KindToken, ZkKey};
use zenkey_model::health::{self as hm, Heard, Read};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

use crate::bus::presence::Scope;
use crate::model::catalog::{Catalog, Observed};

/// `archive.v1`'s interface id: its providers are the archives an absent
/// owner's last-known status is asked of.
const ARCHIVE: &str = "archive.v1";

/// How many deliveries of one service a window keeps in order; past it the
/// oldest go. Only the order of the last status put and the last
/// `clock_ahead` fault is read, and repeats collapse, so this is generous.
pub const SEQUENCE_CAP: usize = 64;

/// Which services a reading covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthTarget {
    /// Every service presence shows, and every one that answers.
    All,
    /// One service, `<system>/<service>`.
    One(Addr),
}

impl HealthTarget {
    /// The provider a tool's consumer binds (R1).
    fn provider(&self) -> String {
        match self {
            HealthTarget::All => "*/*".to_owned(),
            HealthTarget::One(a) => a.to_string(),
        }
    }

    fn chunks(&self) -> (String, String) {
        match self {
            HealthTarget::All => ("*".to_owned(), "*".to_owned()),
            HealthTarget::One(a) => (a.system.to_string(), a.service.to_string()),
        }
    }

    /// The selector a reading's GET goes out on: the owners'
    /// `health.v1/state/**`, base-relative (§2.4).
    pub fn state_selector(&self) -> String {
        let (sys, svc) = self.chunks();
        [GRAMMAR, &sys, &svc, hm::IFACE, "state", "**"].join("/")
    }

    fn scope(&self) -> Scope {
        match self {
            HealthTarget::All => Scope::all(),
            HealthTarget::One(a) => Scope::service(a),
        }
    }
}

/// A service across a constrained face (§2.8, core §8.5): presence and
/// descriptors do not cross, and the deployment's word says it implements
/// `health.v1` (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcrossFace {
    /// Whether the face lets `state/status` cross.
    pub status_crosses: bool,
}

/// What a reading asks.
#[derive(Debug, Clone, Copy)]
pub struct HealthSpec {
    /// Each read's timeout: presence, a descriptor, a GET, an archive.
    pub timeout: Duration,
    /// The window: how long a subscription to every status and to the
    /// faults listens, from before the first reading. `None`: no
    /// subscription, and §5's clock question is not asked.
    pub window: Option<Duration>,
    /// How far apart the two readings are when no window separates them.
    pub grace: Duration,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1).
    pub clocks_synced: bool,
    /// The service sits across a constrained face (§2.8).
    pub face: Option<AcrossFace>,
}

/// A stamp's time, and the id of the clock that issued it (core §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub time: SystemTime,
    pub clock: String,
}

impl Stamped {
    fn of(sample: &Sample) -> Option<Stamped> {
        sample.timestamp().map(|t| Stamped {
            time: t.get_time().to_system_time(),
            clock: t.get_id().to_string(),
        })
    }

    /// As the report spells a stamp, as [`crate::bus::consume::stamp`]
    /// does: RFC 3339 with nanoseconds, and the clock's id.
    pub fn report(&self) -> crate::report::Stamp {
        let since = self
            .time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        crate::report::Stamp {
            time: format!("{:#}", zenoh::time::NTP64::from(since)),
            clock: self.clock.clone(),
        }
    }
}

/// A status as decoded (§2.6): its level as read, and the words beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusValue {
    pub level: Read,
    pub reason: String,
    pub since_ns: u64,
}

impl StatusValue {
    /// A status payload: `health.v1.Status`, or [`Read::Undecodable`].
    pub fn decode(bytes: &[u8]) -> StatusValue {
        match v1::Status::decode(bytes) {
            Ok(s) => StatusValue {
                level: s.read(),
                reason: s.reason,
                since_ns: s.since_ns,
            },
            Err(_) => StatusValue {
                level: Read::Undecodable,
                reason: String::new(),
                since_ns: 0,
            },
        }
    }
}

/// A check as decoded (§2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckValue {
    pub level: Read,
    pub detail: String,
}

impl CheckValue {
    /// A check payload: `health.v1.Check`, or [`Read::Undecodable`].
    pub fn decode(bytes: &[u8]) -> CheckValue {
        match v1::Check::decode(bytes) {
            Ok(c) => CheckValue {
                level: c.read(),
                detail: c.detail,
            },
            Err(_) => CheckValue {
                level: Read::Undecodable,
                detail: String::new(),
            },
        }
    }
}

/// A fault as decoded (§2.3, §2.10): an occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultValue {
    pub code: String,
    pub level: Read,
    pub detail: String,
}

/// One state member as a GET answered it: a value, or `None` for a
/// `reply_del` (core S2, S3), with its stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replied<T> {
    pub value: Option<T>,
    pub stamp: Option<Stamped>,
}

/// What one GET of `health.v1/state/**` answered for one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceGot {
    /// The status; `None` when no reply named it.
    pub status: Option<Replied<StatusValue>>,
    /// Each check answered, by the `{check}` chunk of its key.
    pub checks: BTreeMap<String, Replied<CheckValue>>,
}

/// One reading's GET, every service's answer by address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthGet {
    /// The selector, base-relative.
    pub selector: String,
    /// This host's clock as the GET ended: the instant its replies' stamps
    /// are aged at (`freshness.v1` §2.6).
    pub read_at: SystemTime,
    /// Whether the GET ended without an error reply (a timeout among them).
    pub complete: bool,
    pub services: BTreeMap<Addr, ServiceGot>,
}

/// What the window's subscriptions heard of one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceHeard {
    /// The last status put heard, with its stamp.
    pub last: Option<Replied<StatusValue>>,
    /// Status puts and deletes and faults, in arrival order, consecutive
    /// repeats collapsed, at most [`SEQUENCE_CAP`]: what §5's clock
    /// question reads.
    pub sequence: Vec<Heard>,
    /// Faults heard.
    pub faults: u64,
    pub last_fault: Option<FaultValue>,
}

impl ServiceHeard {
    fn push(&mut self, h: Heard) {
        if self.sequence.last() == Some(&h) {
            return;
        }
        if self.sequence.len() >= SEQUENCE_CAP {
            self.sequence.remove(0);
        }
        self.sequence.push(h);
    }
}

/// What the window heard, read at its end.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthWindow {
    /// The key expressions subscribed, base-relative.
    pub selectors: Vec<String>,
    /// How long the subscriptions listened, to the second reading.
    pub listened: Duration,
    /// By address, every service heard.
    pub heard: BTreeMap<Addr, ServiceHeard>,
    /// The status member's `freshness.v1` observation at the second reading
    /// (§2.5), for every service the run read, heard or not.
    pub status: BTreeMap<Addr, Observation>,
    /// Per stamping clock, what the window measured of it (§2.6, ground 2).
    pub clocks: BTreeMap<String, ClockMeasure>,
}

/// An archive's status for an absent owner (core §4.4, S6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedStatus {
    pub archive: Addr,
    pub value: Option<StatusValue>,
    pub stamp: Option<Stamped>,
    pub confirmed: bool,
}

/// Everything one health reading read, as values.
#[derive(Debug, Clone)]
pub struct HealthObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub target: HealthTarget,
    pub spec: HealthSpec,
    /// Presence and descriptors; `None` across a face, where neither
    /// crosses (§2.8).
    pub presence: Option<Result<Observed, String>>,
    /// The two readings' GETs, the first first.
    pub first: Result<HealthGet, String>,
    pub second: Result<HealthGet, String>,
    /// The window, when one ran.
    pub window: Option<Result<HealthWindow, String>>,
    /// For one service named and absent: the first archive presence shows
    /// that holds its status, or none; `None` when not asked.
    pub archived: Option<Result<Option<ArchivedStatus>, String>>,
}

impl HealthObservation {
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider (tooling guide O5).
    pub fn asked(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Ok(p)) = &self.presence {
            out.push(p.selector.clone());
        }
        out.push(self.target.state_selector());
        if let Some(Ok(w)) = &self.window {
            out.extend(w.selectors.iter().cloned());
        }
        let mut seen = BTreeSet::new();
        out.retain(|s| seen.insert(s.clone()));
        out
    }
}

/// The member key of `addr`'s status: `zk2/<addr>/health.v1/state/status`.
pub fn status_key(addr: &Addr) -> String {
    [
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        hm::IFACE,
        "state",
        hm::STATUS,
    ]
    .join("/")
}

/// `health.v1`'s interface id.
fn iface() -> IfaceId {
    IfaceId::from_str(hm::IFACE).expect("health.v1 is an interface id")
}

/// A health reading's key, parsed: its owner and its resource chunks,
/// when it is a `health.v1` key of the kind `kind`.
fn health_key(key: &str, kind: KindToken) -> Option<(Addr, Vec<String>)> {
    match zenkey_model::grammar::parse(key) {
        Ok(ZkKey::Data {
            addr,
            iface: got,
            kind: k,
            resource,
        }) if got == iface() && k == kind => Some((addr, resource)),
        _ => None,
    }
}

/// One health reading: [`observe`], then [`crate::judge::health::judge`].
pub async fn run_health(
    session: &Session,
    namespace: &str,
    target: HealthTarget,
    spec: HealthSpec,
) -> crate::report::HealthReport {
    let obs = observe(session, namespace, target, spec, None).await;
    crate::judge::health::judge(&obs)
}

/// The two readings, the window and the archive, as the module doc says.
/// `presence`, when given, is the read a caller already made (`check
/// conform`), and is not made again.
pub async fn observe(
    session: &Session,
    namespace: &str,
    target: HealthTarget,
    spec: HealthSpec,
    presence: Option<Observed>,
) -> HealthObservation {
    let t = spec.timeout;
    let declared = Instant::now();
    let window = match spec.window {
        Some(_) => Some(Window::open(session, &target).await),
        None => None,
    };
    let presence = match (spec.face, presence) {
        (Some(_), _) => None,
        (None, Some(p)) => Some(Ok(p)),
        (None, None) => Some(
            crate::bus::presence::observe(session, &target.scope(), t)
                .await
                .map_err(|e| crate::one_line(&e)),
        ),
    };
    let selector = target.state_selector();
    let first = get(session, &selector, t).await;
    let wait = match spec.window {
        Some(w) => (declared + w).saturating_duration_since(Instant::now()),
        None => spec.grace,
    };
    tokio::time::sleep(wait).await;
    let second = get(session, &selector, t).await;
    let window = window.map(|w| {
        w.map(|w| {
            let mut addrs: BTreeSet<Addr> = BTreeSet::new();
            if let HealthTarget::One(a) = &target {
                addrs.insert(a.clone());
            }
            if let Some(Ok(p)) = &presence {
                addrs.extend(Catalog::new(p).addresses().cloned());
            }
            for g in [&first, &second].into_iter().flatten() {
                addrs.extend(g.services.keys().cloned());
            }
            w.close(addrs)
        })
    });
    let archived = match (&target, &presence) {
        (HealthTarget::One(a), Some(Ok(p))) if p.complete && !Catalog::new(p).has_instance(a) => {
            Some(archived(session, a, t).await)
        }
        _ => None,
    };
    HealthObservation {
        namespace: namespace.to_owned(),
        target,
        spec,
        presence,
        first,
        second,
        window,
        archived,
    }
}

/// One reading's GET (core S4): target `All`, consolidation `Latest`, every
/// reply grouped by the owner its key names. A reply on a key that is not a
/// `health.v1` state key, or on a wildcard key, is not a member and is left
/// out (R6).
pub async fn get(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<HealthGet, String> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut out = HealthGet {
        selector: selector.to_owned(),
        read_at: SystemTime::now(),
        complete: true,
        services: BTreeMap::new(),
    };
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        let key = sample.key_expr().as_str();
        if key.contains('*') {
            continue;
        }
        let Some((addr, resource)) = health_key(key, KindToken::State) else {
            continue;
        };
        let stamp = Stamped::of(sample);
        let bytes = (sample.kind() == SampleKind::Put).then(|| sample.payload().to_bytes());
        let got = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [s] if s == hm::STATUS => {
                got.status = Some(Replied {
                    value: bytes.map(|b| StatusValue::decode(&b)),
                    stamp,
                });
            }
            [c, name] if c == "checks" => {
                got.checks.insert(
                    name.clone(),
                    Replied {
                        value: bytes.map(|b| CheckValue::decode(&b)),
                        stamp,
                    },
                );
            }
            _ => {}
        }
    }
    out.read_at = SystemTime::now();
    Ok(out)
}

/// The window's subscriptions, while they listen.
struct Window {
    status: zenkey::consumer::Subscription,
    faults: zenkey::consumer::Subscription,
    heard: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>>,
    selectors: Vec<String>,
}

impl Window {
    /// Subscribes to the target's status and faults through the runtime's
    /// consumer, at once and without presence (R1).
    async fn open(session: &Session, target: &HealthTarget) -> Result<Window, String> {
        let provider = target.provider();
        let consumer = zenkey::consumer::Consumer::for_tool(
            session,
            zenkey::health::implementation().shared_contract(),
            &[provider.as_str()],
            &BTreeMap::new(),
        )
        .map_err(|e| e.to_string())?;
        let mut selectors = Vec::new();
        for r in [zenkey::health::STATUS, zenkey::health::FAULTS] {
            selectors.extend(
                consumer
                    .selectors(r)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .map(|k| k.as_str().to_owned()),
            );
        }
        let heard: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>> = Arc::default();
        let h = Arc::clone(&heard);
        let status = consumer
            .subscribe(
                zenkey::health::STATUS,
                move |d: zenkey::consumer::Delivery| {
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    if d.sample.kind() == SampleKind::Delete {
                        s.push(Heard::StatusDeleted);
                        return;
                    }
                    s.push(Heard::Status);
                    s.last = Some(Replied {
                        value: Some(StatusValue::decode(&d.sample.payload().to_bytes())),
                        stamp: Stamped::of(&d.sample),
                    });
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let h = Arc::clone(&heard);
        let faults = consumer
            .subscribe(
                zenkey::health::FAULTS,
                move |d: zenkey::consumer::Delivery| {
                    let fault = match v1::Fault::decode(d.sample.payload().to_bytes().as_ref()) {
                        Ok(f) => FaultValue {
                            level: f.read(),
                            code: f.code,
                            detail: f.detail,
                        },
                        Err(_) => FaultValue {
                            code: String::new(),
                            level: Read::Undecodable,
                            detail: String::new(),
                        },
                    };
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    s.faults += 1;
                    s.push(if fault.code == hm::CLOCK_AHEAD {
                        Heard::ClockAhead
                    } else {
                        Heard::Fault
                    });
                    s.last_fault = Some(fault);
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(Window {
            status,
            faults,
            heard,
            selectors,
        })
    }

    /// What the window heard, read now: every address's status as a
    /// `freshness.v1` observation (§2.5), and each clock's measurement
    /// across both subscriptions, faults included: a stamp ahead of this
    /// host's clock beyond the delta proves the clocks disagree (§2.6).
    fn close(self, addrs: BTreeSet<Addr>) -> HealthWindow {
        let heard = self.heard.lock().expect("not poisoned").clone();
        let mut clocks = self.status.clock_offsets();
        for (clock, m) in self.faults.clock_offsets() {
            clocks
                .entry(clock)
                .and_modify(|all| *all = all.merge(m))
                .or_insert(m);
        }
        let status = addrs
            .into_iter()
            .chain(heard.keys().cloned())
            .map(|a| {
                let o = self.status.freshness(&status_key(&a));
                (a, o)
            })
            .collect();
        HealthWindow {
            selectors: self.selectors,
            listened: self.status.listened(),
            heard,
            status,
            clocks,
        }
    }
}

/// An absent owner's status, from the first archive presence shows that
/// holds it (core §4.4, S5): presence read whole, its `archive.v1`
/// providers by token or by descriptor, each asked in turn.
async fn archived(
    session: &Session,
    owner: &Addr,
    timeout: Duration,
) -> Result<Option<ArchivedStatus>, String> {
    let all = crate::bus::presence::observe(session, &Scope::all(), timeout)
        .await
        .map_err(|e| crate::one_line(&e))?;
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    let catalog = Catalog::new(&all);
    let origin = status_key(owner);
    for a in catalog
        .addresses()
        .filter(|a| catalog.provides(a, &archive))
    {
        let got = zenkey::archive::last_known(session, a, &origin, timeout)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(lk) = got {
            return Ok(Some(ArchivedStatus {
                archive: a.clone(),
                value: lk.value.as_deref().map(StatusValue::decode),
                stamp: lk.timestamp.map(|t| Stamped {
                    time: t.get_time().to_system_time(),
                    clock: t.get_id().to_string(),
                }),
                confirmed: lk.confirmed,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_spells_its_selectors() {
        assert_eq!(
            HealthTarget::All.state_selector(),
            "zk2/*/*/health.v1/state/**"
        );
        let a: Addr = "lab/svc".parse().unwrap();
        assert_eq!(
            HealthTarget::One(a.clone()).state_selector(),
            "zk2/lab/svc/health.v1/state/**"
        );
        assert_eq!(status_key(&a), "zk2/lab/svc/health.v1/state/status");
        assert_eq!(
            health_key("zk2/lab/svc/health.v1/state/checks/disk", KindToken::State),
            Some((a, vec!["checks".to_owned(), "disk".to_owned()]))
        );
        assert_eq!(
            health_key("zk2/lab/svc/nav.v2/state/status", KindToken::State),
            None
        );
    }

    #[test]
    fn a_payload_that_does_not_decode_is_no_level() {
        let s = v1::Status {
            level: 3,
            reason: "down".into(),
            since_ns: 7,
        };
        assert_eq!(
            StatusValue::decode(&s.encode_to_vec()),
            StatusValue {
                level: Read::Level(hm::Level::Failed),
                reason: "down".into(),
                since_ns: 7
            }
        );
        assert_eq!(StatusValue::decode(&[0xff, 0xff]).level, Read::Undecodable);
        let mut h = ServiceHeard::default();
        for x in [
            Heard::Status,
            Heard::Status,
            Heard::ClockAhead,
            Heard::Status,
        ] {
            h.push(x);
        }
        assert_eq!(
            h.sequence,
            [Heard::Status, Heard::ClockAhead, Heard::Status],
            "repeats collapse, the order stays"
        );
    }
}
