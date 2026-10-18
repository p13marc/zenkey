//! `config.v1` (#768, CC), the reads and the acts: one service's
//! configuration, read as §5's tool reads it and changed through the
//! runtime's client (`spec/profiles/config/v1.md`).
//!
//! **The reading** ([`observe`]), through a session in the deployment's
//! namespace:
//! 1. with a window, first: a subscription to the view, the status and the
//!    change events through the runtime's consumer on the standard contract
//!    (R4), which confirms each document on this host's monotonic clock
//!    (`freshness.v1` §2.5) and measures the owner's clock on live puts
//!    (§2.6, ground 2);
//! 2. presence and the service's descriptors (core §8.1, §3.3): the premise
//!    of every §5 row. Across a constrained face neither crosses, and the
//!    deployment's word stands for them (§2.19, R7);
//! 3. one S4 GET (core §4.2) of the owner's `config.v1/state/**`, or of
//!    `state/status` alone (`--status`), at the window's end: target `All`,
//!    consolidation `Latest`, the owner alone answering. A reply on a
//!    wildcard key is no member (R6);
//! 4. for a change asked about by its token, a replay of the change events
//!    (core §2.6): a wildcard GET a union storage answers, when the
//!    deployment runs one, kept to that token.
//!
//! **The acts** ([`call`]): one operation at one address through the
//! runtime's `Client` (O1: `BestMatching`, consolidation `None`; none is
//! idempotent, so none is retried, O4), with the call metadata the caller
//! claims (O7). The reply is read as its union (§3): `{applied}` or
//! `{pending}` for a `set`, `{view}` or `{status}` for a control operation;
//! a refusal as the core's envelope, its `app` detail a `ConfigError`;
//! silence attributed through presence, or unattributed across a face (R7).
//!
//! Nothing is judged here: [`crate::judge::config`] answers §5's questions
//! from the [`ConfigObservation`] this returns.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value as Json;
use zenkey::config_v1;
use zenkey_model::config::{
    self as cm, ConfigChangeEvent, ConfigError, ConfigStatus, ConfigView, ControlReply, SetReply,
};
use zenkey_model::freshness::{ClockMeasure, Observation};
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId, KindToken, ZkKey};
use zenkey_model::template::Bindings;
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

use crate::bus::health::{Replied, Stamped};
use crate::bus::presence::Scope;
use crate::model::catalog::Observed;
use crate::model::config::{redact_status, redact_view, scrub};
use crate::report::{
    ConfigActAnswer, ConfigActReport, ConfigChecked, ConfigRefusal, ConfigReport, MalformedReply,
    SilenceView,
};

/// A document as read: its value, or why the bytes are not one.
pub type Doc<T> = Result<T, String>;

/// How many change events a window keeps, the latest; past it the oldest
/// go, and the count says so.
pub const EVENT_CAP: usize = 64;

/// `config.v1`'s interface id.
pub fn iface() -> IfaceId {
    IfaceId::from_str(cm::IFACE).expect("config.v1 is an interface id")
}

fn member(addr: &Addr, kind: &str, name: &str) -> String {
    [
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        cm::IFACE,
        kind,
        name,
    ]
    .join("/")
}

/// `zk2/<addr>/config.v1/state/view`.
pub fn view_key(addr: &Addr) -> String {
    member(addr, "state", cm::VIEW)
}

/// `zk2/<addr>/config.v1/state/status`.
pub fn status_key(addr: &Addr) -> String {
    member(addr, "state", cm::STATUS)
}

/// Which services a GET covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigTarget {
    /// Every service: the doctor's.
    All,
    /// One service, `<system>/<service>`.
    One(Addr),
}

impl ConfigTarget {
    /// The selector a GET goes out on, base-relative: the owners'
    /// `config.v1/state/**`, or `state/status` alone.
    pub fn state_selector(&self, status_only: bool) -> String {
        let (sys, svc) = match self {
            ConfigTarget::All => ("*".to_owned(), "*".to_owned()),
            ConfigTarget::One(a) => (a.system.to_string(), a.service.to_string()),
        };
        let last = if status_only { cm::STATUS } else { "**" };
        [GRAMMAR, &sys, &svc, cm::IFACE, "state", last].join("/")
    }
}

/// What one GET answered for one service: each document's reply, `None`
/// when no reply named it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServiceDocs {
    pub view: Option<Replied<Doc<ConfigView>>>,
    pub status: Option<Replied<Doc<ConfigStatus>>>,
}

/// One GET of `config.v1/state/**`, every service's answer by address.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigGet {
    /// The selector, base-relative.
    pub selector: String,
    /// This host's clock as the GET ended: the instant its replies' stamps
    /// are aged at (`freshness.v1` §2.6).
    pub read_at: SystemTime,
    /// Whether the GET ended without an error reply.
    pub complete: bool,
    pub services: BTreeMap<Addr, ServiceDocs>,
}

fn stamped(sample: &Sample) -> Option<Stamped> {
    sample.timestamp().map(|t| Stamped {
        time: t.get_time().to_system_time(),
        clock: t.get_id().to_string(),
    })
}

/// A document's bytes, decoded as its JSON type (§3).
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Doc<T> {
    serde_json::from_slice(bytes).map_err(|e| format!("not the document's type: {e}"))
}

/// A `config.v1` key's owner and resource chunks, when it is one of the kind
/// `kind`.
fn config_key(key: &str, kind: KindToken) -> Option<(Addr, Vec<String>)> {
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

/// One GET (core S4): target `All`, consolidation `Latest`, each reply filed
/// by the owner its key names. A reply on a key that is not a `config.v1`
/// state key, or on a wildcard key, is no member and is left out (R6).
pub async fn get(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<ConfigGet, String> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut out = ConfigGet {
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
        let Some((addr, resource)) = config_key(key, KindToken::State) else {
            continue;
        };
        let stamp = stamped(sample);
        let bytes = (sample.kind() == SampleKind::Put).then(|| sample.payload().to_bytes());
        let docs = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [v] if v == cm::VIEW => {
                docs.view = Some(Replied {
                    value: bytes.map(|b| decode(&b)),
                    stamp,
                });
            }
            [s] if s == cm::STATUS => {
                docs.status = Some(Replied {
                    value: bytes.map(|b| decode(&b)),
                    stamp,
                });
            }
            _ => {}
        }
    }
    out.read_at = SystemTime::now();
    Ok(out)
}

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct ConfigSpec {
    /// Each read's timeout: presence, a descriptor, the GET, a replay.
    pub timeout: Duration,
    /// The window: how long a subscription to the view, the status and the
    /// change events listens before the GET. `None`: none.
    pub window: Option<Duration>,
    /// The operator's word that this host's clock and the owner's agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1).
    pub clocks_synced: bool,
    /// The service sits across a constrained face: no presence and no
    /// descriptor is read, and the deployment's word says it implements
    /// `config.v1` (§2.19, R7).
    pub across_face: bool,
    /// Read the status alone: the view is six frames on a radio, the
    /// status two (§2.19).
    pub status_only: bool,
    /// A change asked about by its token, as the `set` that started it was
    /// answered (§5's second and third rows).
    pub token: Option<String>,
    /// The instance that answered that `set`, when the caller knows it.
    pub instance: Option<String>,
    /// The desired document's revision asked about (§5's last row); `None`:
    /// the last one the owner acted on.
    pub desired_revision: Option<u64>,
}

impl ConfigSpec {
    /// A reading of the view and the status, nothing more asked.
    pub fn new(timeout: Duration) -> ConfigSpec {
        ConfigSpec {
            timeout,
            window: None,
            clocks_synced: false,
            across_face: false,
            status_only: false,
            token: None,
            instance: None,
            desired_revision: None,
        }
    }
}

/// What a window's subscriptions heard, read at its end.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigWindow {
    /// The key expressions subscribed, base-relative.
    pub selectors: Vec<String>,
    /// How long the subscriptions listened.
    pub listened: Duration,
    /// The last view and status put heard.
    pub view: Option<Replied<Doc<ConfigView>>>,
    pub status: Option<Replied<Doc<ConfigStatus>>>,
    /// Each document's `freshness.v1` observation at the window's end
    /// (§2.5), by its key.
    pub seen: BTreeMap<String, Observation>,
    /// The change events heard, the latest [`EVENT_CAP`].
    pub events: Vec<ConfigChangeEvent>,
    /// Events past the cap.
    pub events_dropped: u64,
    /// Events that do not decode as `ConfigChangeEvent`.
    pub events_undecodable: u64,
    /// Per stamping clock, what the window measured of it (§2.6).
    pub clocks: BTreeMap<String, ClockMeasure>,
}

/// A replay of the change events (core §2.6): what a union storage holds,
/// kept to the token asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigReplay {
    pub selector: String,
    pub events: Vec<ConfigChangeEvent>,
    pub undecodable: u64,
}

/// Everything one reading read, as values.
#[derive(Debug, Clone)]
pub struct ConfigObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub address: Addr,
    pub spec: ConfigSpec,
    /// Presence and descriptors; `None` across a face, where neither
    /// crosses.
    pub presence: Option<Result<Observed, String>>,
    pub get: Result<ConfigGet, String>,
    /// The window, when one ran.
    pub window: Option<Result<ConfigWindow, String>>,
    /// The replay, when a token was asked about.
    pub replay: Option<Result<ConfigReplay, String>>,
}

impl ConfigObservation {
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider.
    pub fn asked(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Ok(p)) = &self.presence {
            out.push(p.selector.clone());
        }
        out.push(ConfigTarget::One(self.address.clone()).state_selector(self.spec.status_only));
        if let Some(Ok(w)) = &self.window {
            out.extend(w.selectors.iter().cloned());
        }
        if let Some(Ok(r)) = &self.replay {
            out.push(r.selector.clone());
        }
        let mut seen = BTreeSet::new();
        out.retain(|s| seen.insert(s.clone()));
        out
    }
}

/// One reading: [`observe`], then [`crate::judge::config::judge`].
pub async fn run_config(
    session: &Session,
    namespace: &str,
    address: Addr,
    spec: ConfigSpec,
) -> ConfigReport {
    let obs = observe(session, namespace, address, spec, None).await;
    crate::judge::config::judge(&obs)
}

/// The reading of the module doc. `presence`, when given, is the read a
/// caller already made, and is not made again.
pub async fn observe(
    session: &Session,
    namespace: &str,
    address: Addr,
    spec: ConfigSpec,
    presence: Option<Observed>,
) -> ConfigObservation {
    let t = spec.timeout;
    let declared = Instant::now();
    let window = match spec.window {
        Some(_) => Some(Window::open(session, &address).await),
        None => None,
    };
    let presence = match (spec.across_face, presence) {
        (true, _) => None,
        (false, Some(p)) => Some(Ok(p)),
        (false, None) => Some(
            crate::bus::presence::observe(session, &Scope::service(&address), t)
                .await
                .map_err(|e| crate::one_line(&e)),
        ),
    };
    if let Some(w) = spec.window {
        tokio::time::sleep((declared + w).saturating_duration_since(Instant::now())).await;
    }
    let selector = ConfigTarget::One(address.clone()).state_selector(spec.status_only);
    let get = get(session, &selector, t).await;
    let window = window.map(|w| w.map(|w| w.close(&address)));
    let replay = match &spec.token {
        Some(token) => Some(replay(session, &address, token, t).await),
        None => None,
    };
    ConfigObservation {
        namespace: namespace.to_owned(),
        address,
        spec,
        presence,
        get,
        window,
        replay,
    }
}

/// The last put of one document, as the window keeps it.
type Last<T> = Arc<Mutex<Option<Replied<Doc<T>>>>>;

/// What the window's event subscription keeps.
#[derive(Default)]
struct Events {
    kept: Vec<ConfigChangeEvent>,
    dropped: u64,
    undecodable: u64,
}

/// The window's subscriptions, while they listen.
struct Window {
    view: zenkey::consumer::Subscription,
    status: zenkey::consumer::Subscription,
    events: zenkey::consumer::Subscription,
    last_view: Last<ConfigView>,
    last_status: Last<ConfigStatus>,
    heard: Arc<Mutex<Events>>,
    selectors: Vec<String>,
}

fn keep<T: serde::de::DeserializeOwned>(last: &Last<T>, d: &zenkey::consumer::Delivery) {
    let value =
        (d.sample.kind() == SampleKind::Put).then(|| decode(&d.sample.payload().to_bytes()));
    *last.lock().expect("not poisoned") = Some(Replied {
        value,
        stamp: stamped(&d.sample),
    });
}

impl Window {
    /// Subscribes to the service's view, status and change events through
    /// the runtime's consumer, at once and without presence (R1).
    async fn open(session: &Session, address: &Addr) -> Result<Window, String> {
        let provider = address.to_string();
        let consumer = zenkey::consumer::Consumer::for_tool(
            session,
            config_v1::implementation().shared_contract(),
            &[provider.as_str()],
            &BTreeMap::new(),
        )
        .map_err(|e| e.to_string())?;
        let mut selectors = Vec::new();
        for r in [config_v1::VIEW, config_v1::STATUS, config_v1::CHANGE] {
            selectors.extend(
                consumer
                    .selectors(r)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .map(|k| k.as_str().to_owned()),
            );
        }
        let last_view: Last<ConfigView> = Arc::default();
        let l = Arc::clone(&last_view);
        let view = consumer
            .subscribe(config_v1::VIEW, move |d: zenkey::consumer::Delivery| {
                keep(&l, &d)
            })
            .await
            .map_err(|e| e.to_string())?;
        let last_status: Last<ConfigStatus> = Arc::default();
        let l = Arc::clone(&last_status);
        let status = consumer
            .subscribe(config_v1::STATUS, move |d: zenkey::consumer::Delivery| {
                keep(&l, &d)
            })
            .await
            .map_err(|e| e.to_string())?;
        let heard: Arc<Mutex<Events>> = Arc::default();
        let h = Arc::clone(&heard);
        let events = consumer
            .subscribe(config_v1::CHANGE, move |d: zenkey::consumer::Delivery| {
                let mut e = h.lock().expect("not poisoned");
                match decode::<ConfigChangeEvent>(&d.sample.payload().to_bytes()) {
                    Ok(ev) => {
                        if e.kept.len() >= EVENT_CAP {
                            e.kept.remove(0);
                            e.dropped += 1;
                        }
                        e.kept.push(ev);
                    }
                    Err(_) => e.undecodable += 1,
                }
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(Window {
            view,
            status,
            events,
            last_view,
            last_status,
            heard,
            selectors,
        })
    }

    /// What the window heard, read now: each document's observation (§2.5),
    /// and each clock's measurement across the three subscriptions.
    fn close(self, address: &Addr) -> ConfigWindow {
        let mut clocks = self.view.clock_offsets();
        for sub in [&self.status, &self.events] {
            for (clock, m) in sub.clock_offsets() {
                clocks
                    .entry(clock)
                    .and_modify(|all| *all = all.merge(m))
                    .or_insert(m);
            }
        }
        let seen = BTreeMap::from([
            (view_key(address), self.view.freshness(&view_key(address))),
            (
                status_key(address),
                self.status.freshness(&status_key(address)),
            ),
        ]);
        let heard = std::mem::take(&mut *self.heard.lock().expect("not poisoned"));
        ConfigWindow {
            selectors: self.selectors,
            listened: self.view.listened(),
            view: self.last_view.lock().expect("not poisoned").clone(),
            status: self.last_status.lock().expect("not poisoned").clone(),
            seen,
            events: heard.kept,
            events_dropped: heard.dropped,
            events_undecodable: heard.undecodable,
            clocks,
        }
    }
}

/// The change events a union storage holds for `address`, kept to `token`
/// (core §2.6): a wildcard GET, target `All`, consolidation `None`. A
/// deployment that runs no such storage answers nothing, which is no
/// verdict.
async fn replay(
    session: &Session,
    address: &Addr,
    token: &str,
    timeout: Duration,
) -> Result<ConfigReplay, String> {
    let selector = format!("{}/*", member(address, "events", cm::CHANGE));
    let replies = session
        .get(&selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut out = ConfigReplay {
        selector,
        events: Vec::new(),
        undecodable: 0,
    };
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            continue;
        };
        if sample.kind() != SampleKind::Put || sample.key_expr().as_str().contains('*') {
            continue;
        }
        match decode::<ConfigChangeEvent>(&sample.payload().to_bytes()) {
            Ok(e) if e.token == token => out.events.push(e),
            Ok(_) => {}
            Err(_) => out.undecodable += 1,
        }
    }
    Ok(out)
}

// ─── the acts ───────────────────────────────────────────────────────────────

/// One of `config.v1`'s operations (§2.5, §2.10, §2.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOp {
    Set { group: String },
    Confirm,
    Cancel,
    Extend,
    Persist,
}

impl ConfigOp {
    /// The resource, as the contract names it.
    pub fn resource(&self) -> &'static str {
        match self {
            ConfigOp::Set { .. } => config_v1::SET,
            ConfigOp::Confirm => config_v1::CONFIRM,
            ConfigOp::Cancel => config_v1::CANCEL,
            ConfigOp::Extend => config_v1::EXTEND,
            ConfigOp::Persist => config_v1::PERSIST,
        }
    }

    /// `set/<group>`, or the control operation's name.
    pub fn name(&self) -> String {
        match self {
            ConfigOp::Set { group } => format!("set/{group}"),
            ConfigOp::Confirm => cm::CONFIRM.to_owned(),
            ConfigOp::Cancel => cm::CANCEL.to_owned(),
            ConfigOp::Extend => cm::EXTEND.to_owned(),
            ConfigOp::Persist => cm::PERSIST.to_owned(),
        }
    }

    fn values(&self) -> Bindings {
        match self {
            ConfigOp::Set { group } => [("group".to_owned(), vec![group.clone()])].into(),
            _ => Bindings::new(),
        }
    }
}

/// One act.
#[derive(Debug, Clone)]
pub struct ConfigCall<'a> {
    /// The namespace the session is in, for the report.
    pub namespace: &'a str,
    pub address: &'a Addr,
    pub op: ConfigOp,
    /// The request sent: a `SetRequest` or a `ControlRequest`.
    pub request: Json,
    /// The request as the report shows it, every sensitive value redacted.
    pub shown: Json,
    /// The sensitive values sent: taken out of anything the owner sends back
    /// (§2.15).
    pub secrets: Vec<String>,
    /// The parameters known sensitive, by group and name: shown without a
    /// value in any document the reply carries.
    pub sensitive: BTreeSet<(String, String)>,
    /// The call metadata claimed (core O7).
    pub metadata: Option<zenkey::CallMetadata>,
    pub timeout: Duration,
    /// Across a constrained face: a silence is unattributed (R7).
    pub across_face: bool,
    /// What a `set` read and checked first.
    pub checked: Option<ConfigChecked>,
}

/// A runtime error, in the crate's terms: a contract the call breaks is the
/// caller's input; anything else is the bus.
fn runtime(target: &str, e: zenkey::Error) -> crate::Error {
    match e {
        zenkey::Error::Contract(_) | zenkey::Error::NoResource { .. } | zenkey::Error::Key(_) => {
            crate::Error::unaskable(target, e.to_string())
        }
        other => crate::Error::bus("call", target, other),
    }
}

/// An envelope (§5.2) as the report spells it, its `app` detail decoded as
/// `ConfigError`, every sensitive value sent taken out of its words.
fn refusal(env: &zenkey_model::envelope::Envelope, secrets: &[String]) -> ConfigRefusal {
    let (error, detail_undecodable) = match &env.detail {
        None => (None, None),
        Some(zenkey_model::envelope::Detail::Value(v)) => {
            match serde_json::from_value::<ConfigError>(v.clone()) {
                Ok(mut e) => {
                    e.detail = e.detail.map(|d| scrub(&d, secrets));
                    (Some(e), None)
                }
                Err(err) => (None, Some(format!("not a ConfigError: {err}"))),
            }
        }
        Some(zenkey_model::envelope::Detail::Bytes(b)) => (
            None,
            Some(format!("{} bytes, not a ConfigError (§2.5)", b.len())),
        ),
    };
    ConfigRefusal {
        code: env.code.clone(),
        message: scrub(&env.message, secrets),
        cause: env.cause.clone(),
        error,
        detail_undecodable,
    }
}

/// Makes the act, and reads its reply as its union.
pub async fn call(session: &Session, c: ConfigCall<'_>) -> crate::Result<ConfigActReport> {
    let address = c.address.to_string();
    let mut client = zenkey::Client::new(
        session,
        config_v1::implementation().shared_contract(),
        &[address.as_str()],
    )
    .map_err(|e| runtime(&address, e))?
    .with_timeout(c.timeout);
    if let Some(m) = c.metadata.clone() {
        client = client.with_metadata(m);
    }
    if c.across_face {
        client = client.with_presence(zenkey::consumer::Presence::Unavailable);
    }
    let resource = c.op.resource();
    let values = c.op.values();
    let key = client
        .key(c.address, resource, &values)
        .map_err(|e| runtime(&address, e))?
        .as_str()
        .to_owned();
    let outcome = client
        .call_value(c.address, resource, &values, &c.request)
        .await
        .map_err(|e| runtime(&key, e))?;
    let answer = match outcome {
        zenkey::call::Outcome::Value(a) => {
            let size = a.payload().len();
            match a.value::<Json>() {
                Err(e) => ConfigActAnswer::Undecodable {
                    reason: e.to_string(),
                    size,
                },
                Ok(v) => reply_answer(&c.op, v, size, &c.sensitive),
            }
        }
        zenkey::call::Outcome::Refused(env) => ConfigActAnswer::Refused {
            refusal: refusal(&env, &c.secrets),
        },
        zenkey::call::Outcome::Malformed(m) => ConfigActAnswer::Malformed {
            malformed: MalformedReply {
                encoding: m.encoding.clone(),
                error: m.error.to_string(),
            },
        },
        zenkey::call::Outcome::NoAnswer(s) => ConfigActAnswer::Silent {
            silence: SilenceView {
                attempts: s.attempts,
                transport: s.transport,
                presence: crate::bus::operation::attribution(s.presence),
            },
        },
    };
    let (actor, request_id) = match &c.metadata {
        Some(m) => (m.actor.clone(), m.request_id.clone()),
        None => (None, None),
    };
    Ok(ConfigActReport {
        namespace: c.namespace.to_owned(),
        service: address,
        operation: c.op.name(),
        key,
        request: c.shown,
        actor,
        request_id,
        checked: c.checked,
        across_face: c.across_face,
        timeout_s: c.timeout.as_secs_f64(),
        answer,
    })
}

/// A value reply read as the operation's union (§3): `SetReply` for a
/// `set`, `ControlReply` for the others; anything else is undecodable. A
/// sensitive parameter's value never reaches the report.
fn reply_answer(
    op: &ConfigOp,
    v: Json,
    size: usize,
    sensitive: &BTreeSet<(String, String)>,
) -> ConfigActAnswer {
    let undecodable = |e: serde_json::Error, ty: &str| ConfigActAnswer::Undecodable {
        reason: format!("not a {ty}: {e}"),
        size,
    };
    match op {
        ConfigOp::Set { .. } => match serde_json::from_value::<SetReply>(v) {
            Ok(SetReply::Applied(mut view)) => {
                redact_view(&mut view, sensitive);
                ConfigActAnswer::Applied { view }
            }
            Ok(SetReply::Pending(pending)) => ConfigActAnswer::Pending { pending },
            Err(e) => undecodable(e, "SetReply"),
        },
        _ => match serde_json::from_value::<ControlReply>(v) {
            Ok(ControlReply::View(mut view)) => {
                redact_view(&mut view, sensitive);
                ConfigActAnswer::View { view }
            }
            Ok(ControlReply::Status(mut status)) => {
                redact_status(&mut status, sensitive);
                ConfigActAnswer::Status { status }
            }
            Err(e) => undecodable(e, "ControlReply"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_spells_its_selectors() {
        let a: Addr = "lab/rf0".parse().unwrap();
        assert_eq!(
            ConfigTarget::All.state_selector(false),
            "zk2/*/*/config.v1/state/**"
        );
        assert_eq!(
            ConfigTarget::One(a.clone()).state_selector(true),
            "zk2/lab/rf0/config.v1/state/status"
        );
        assert_eq!(view_key(&a), "zk2/lab/rf0/config.v1/state/view");
        assert_eq!(
            config_key("zk2/lab/rf0/config.v1/state/status", KindToken::State),
            Some((a, vec!["status".to_owned()]))
        );
        assert_eq!(
            config_key("zk2/lab/rf0/health.v1/state/status", KindToken::State),
            None
        );
        assert_eq!(
            ConfigOp::Set {
                group: "duty".into()
            }
            .name(),
            "set/duty"
        );
        assert_eq!(ConfigOp::Persist.resource(), "@op/persist");
    }

    /// A reply is read as its operation's union: a `set`'s `{pending}`, a
    /// control operation's `{status}` with a sensitive value stripped, and
    /// a value that is neither, undecodable.
    #[test]
    fn a_reply_is_read_as_its_union() {
        let set = ConfigOp::Set {
            group: "radio".into(),
        };
        let pending = serde_json::json!({"pending": {"token": "i1-1",
            "apply_at": "2026-10-18T00:00:01.000Z", "deadline": "2026-10-18T00:02:00.000Z"}});
        assert!(matches!(
            reply_answer(&set, pending.clone(), 9, &BTreeSet::new()),
            ConfigActAnswer::Pending { .. }
        ));
        assert!(matches!(
            reply_answer(&ConfigOp::Confirm, pending, 9, &BTreeSet::new()),
            ConfigActAnswer::Undecodable { .. }
        ));
        let status = serde_json::json!({"status": {"instance": "i1", "revision": 1,
            "confirm_floor_s": 10, "values": {"sim": {"pin": "1234"}, "duty": {"f": 1}}}});
        let known = BTreeSet::from([("sim".to_owned(), "pin".to_owned())]);
        match reply_answer(&ConfigOp::Confirm, status, 9, &known) {
            ConfigActAnswer::Status { status } => {
                assert!(!status.values.contains_key("sim"));
                assert!(status.values.contains_key("duty"));
            }
            other => panic!("{other:?}"),
        }
        let env = zenkey_model::envelope::Envelope {
            code: "app".into(),
            message: "pin 1234 refused".into(),
            cause: None,
            detail: Some(zenkey_model::envelope::Detail::Value(
                serde_json::json!({"code": "device-refused", "detail": "bad 1234"}),
            )),
        };
        let r = refusal(&env, &["1234".into()]);
        assert_eq!(r.message, "pin <redacted> refused");
        assert_eq!(r.error.unwrap().detail.as_deref(), Some("bad <redacted>"));
    }
}
