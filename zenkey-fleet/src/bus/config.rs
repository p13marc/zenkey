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
//!    consolidation `None`, the owner alone answering. A reply on a
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
//! a refusal as the core's error form, its `app` detail a `ConfigError`;
//! silence attributed through presence, or unattributed across a face (R7).
//!
//! Nothing is judged here: [`crate::judge::config`] answers §5's questions
//! from the [`ConfigObservation`] this returns.
//!
//! **`config.v2`** (#985 G3, `spec/profiles/config/v2.md`). A service
//! whose descriptor lists `config.v1` is read as above. One whose
//! descriptor's `profiles` lists `config.v2` is read through its own config
//! interface: the revision its descriptor names whose contract uses
//! `config.v2`, held offline (`ConfigSpec::contracts`) or retrieved (core
//! §8.4), and refused when its lint has a finding (v2 §2.2). Across a face
//! the deployment's word names it: a revision in hand that uses
//! `config.v2` (`--iface` among several). Then:
//! 1. presence and the descriptors first, since the interface is the
//!    descriptor's (a `config.v1` reading's window opens after them too);
//! 2. with a window, a subscription to the record and the change events
//!    through the runtime's consumer on that contract, seeded by S4's GET,
//!    the record confirmed by the owner's heartbeat (`freshness.v2`);
//! 3. one S4 GET of `<iface>/state/config`, the one record (v2 §2.3), read
//!    by the contract's layout under CH's tag and the deployment epoch the
//!    catalogs give (core §8.6);
//! 4. a replay of the change events, as above.
//!
//! An act writes the group's `Set<T>` or a `Control` record in the request
//! form through the runtime's `Client` on that contract, and reads every
//! value reply as the record, a refusal as the error form with its `Error`
//! detail read by the contract's layout (v2 §2.9).

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

use crate::bus::contracts::BundleStore;
use crate::bus::health::{Replied, Stamped};
use crate::bus::presence::Scope;
use crate::model::catalog::{Catalog, ContractSet, Observed};
use crate::model::config::{
    RecordGot, V2Target, V2Unread, listing, listing_v2, named_revisions, redact_status,
    redact_view, scrub, uses_v2, view_of,
};
use crate::model::deployment::{Catalogs, Stated};
use crate::report::{
    ConfigActAnswer, ConfigActReport, ConfigAppError, ConfigChecked, ConfigRefusal, ConfigReport,
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
    /// `config.v2`'s record (v2 §2.3), read through its contract.
    pub record: Option<Replied<Doc<RecordGot>>>,
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

/// One GET (core S4): target `All`, consolidation `None`, a snapshot's page
/// read as its entries, each entry's confirmation dated under its owner's
/// epoch (`epochs`), every reply merged per key by its stamp, never the
/// last to arrive (`bus::query::merged`), and filed by the owner its key
/// names. A reply on a key that is not a `config.v1` state key, or on a
/// wildcard key, is no member and is left out (R6).
pub async fn get(
    session: &Session,
    selector: &str,
    timeout: Duration,
    epochs: &crate::bus::query::EntryEpochs,
) -> Result<ConfigGet, String> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut out = ConfigGet {
        selector: selector.to_owned(),
        read_at: SystemTime::now(),
        complete: true,
        services: BTreeMap::new(),
    };
    // A snapshot's page is its entries (core §4.2; #977's E3), dated under
    // its owner's epoch, and every reply merged per key by its stamp (S4).
    let mut samples = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        samples.push(sample.clone());
    }
    // A `reply_del` of the view or the status, which its owner answers by
    // snapshot, is no owner's answer: undecodable, `form` (#977's E4).
    let config = zenkey::config_v1::implementation().shared_contract();
    let (samples, strays) = crate::bus::query::merged_without_strays(samples, &[&*config], |s| {
        crate::bus::query::reply_samples_aged(s, epochs)
    });
    let refused = zenkey_model::collection::STRAY_REPLY_DEL.reason();
    for sample in &strays {
        let Some((addr, resource)) = config_key(sample.key_expr().as_str(), KindToken::State)
        else {
            continue;
        };
        let docs = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [v] if v == cm::VIEW => {
                docs.view = Some(Replied {
                    value: Some(Err(refused.clone())),
                    stamp: None,
                });
            }
            [s] if s == cm::STATUS => {
                docs.status = Some(Replied {
                    value: Some(Err(refused.clone())),
                    stamp: None,
                });
            }
            _ => {}
        }
    }
    for sample in &samples {
        let key = sample.key_expr().as_str();
        if key.contains('*') {
            continue;
        }
        let Some((addr, resource)) = config_key(key, KindToken::State) else {
            continue;
        };
        let stamp = stamped(sample);
        // The envelope first (core §7.5): a document is its record, a
        // refused envelope undecodable with its refusal named.
        let bytes = (sample.kind() == SampleKind::Put)
            .then(|| crate::model::inband::record_of(sample).map_err(|r| r.reason()));
        let docs = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [v] if v == cm::VIEW => {
                docs.view = Some(Replied {
                    value: bytes.map(|b| b.and_then(|b| decode(&b))),
                    stamp,
                });
            }
            [s] if s == cm::STATUS => {
                docs.status = Some(Replied {
                    value: bytes.map(|b| b.and_then(|b| decode(&b))),
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
    /// Contracts held offline (`--contracts`): a `config.v2` owner's
    /// config interface is read by one of these before any is retrieved,
    /// and across a face they are the deployment's word for it.
    pub contracts: ContractSet,
    /// The deployment catalogs (`--catalog`): the epoch a `config.v2`
    /// record's times count from (core §8.6).
    pub catalogs: Catalogs,
    /// Across a face, the config interface among several in hand that use
    /// `config.v2`.
    pub iface: Option<IfaceId>,
    /// A store to retrieve contracts through, shared with the caller's;
    /// `None`: one of this reading's own, seeded with `contracts`.
    pub store: Option<BundleStore>,
    /// A `config.v2` owner's config interface the caller already decided
    /// (`check conform`'s): read by it, never looked for again. `None`:
    /// [`observe`] decides.
    pub v2: Option<Result<V2Target, V2Unread>>,
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
            contracts: ContractSet::new(),
            catalogs: Catalogs::default(),
            iface: None,
            store: None,
            v2: None,
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
    /// `config.v2`: the last record put heard.
    pub record: Option<Replied<Doc<RecordGot>>>,
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
    /// `config.v2`'s config interface (#985 G3): `None` for a `config.v1`
    /// reading; else the interface the reading read by, or why it held
    /// none to read by.
    pub v2: Option<Result<V2Target, V2Unread>>,
}

impl ConfigObservation {
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider.
    pub fn asked(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Ok(p)) = &self.presence {
            out.push(p.selector.clone());
        }
        match &self.v2 {
            None => out.push(
                ConfigTarget::One(self.address.clone()).state_selector(self.spec.status_only),
            ),
            Some(Ok(t)) => out.push(t.record_key(&self.address)),
            Some(Err(_)) => {}
        }
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
    let presence = match (spec.across_face, presence) {
        (true, _) => None,
        (false, Some(p)) => Some(Ok(p)),
        (false, None) => Some(
            crate::bus::presence::observe(session, &Scope::service(&address), t)
                .await
                .map_err(|e| crate::one_line(&e)),
        ),
    };
    let v2 = match &spec.v2 {
        Some(decided) => Some(decided.clone()),
        None => find_v2(session, &address, &spec, presence.as_ref()).await,
    };
    let window = match (spec.window, &v2) {
        (None, _) | (Some(_), Some(Err(_))) => None,
        (Some(_), None) => Some(Window::open(session, &address, t).await),
        (Some(_), Some(Ok(target))) => Some(Window::open_v2(session, &address, target, t).await),
    };
    if let Some(w) = spec.window {
        tokio::time::sleep((declared + w).saturating_duration_since(Instant::now())).await;
    }
    let get = match &v2 {
        None => {
            let selector = ConfigTarget::One(address.clone()).state_selector(spec.status_only);
            let epochs = crate::bus::query::EntryEpochs::of_presence(
                &spec.catalogs,
                presence.as_ref().and_then(|p| p.as_ref().ok()),
            );
            get(session, &selector, t, &epochs).await
        }
        Some(Ok(target)) => get_records(session, &[(address.clone(), target.clone())], t).await,
        Some(Err(u)) => Err(u.why.clone()),
    };
    let window = window.map(|w| w.map(|w| w.close(&address)));
    let replay = match (&spec.token, &v2) {
        (Some(token), None) => Some(replay(session, &address, None, token, t).await),
        (Some(token), Some(Ok(target))) => {
            Some(replay(session, &address, Some(target), token, t).await)
        }
        _ => None,
    };
    ConfigObservation {
        namespace: namespace.to_owned(),
        address,
        spec,
        presence,
        get,
        window,
        replay,
        v2,
    }
}

/// Whether an act on `address` is a `config.v2` owner's, and by which
/// config interface (#985 G3), as [`observe`] decides it: presence and the
/// descriptors read first unless across a face, where the contracts in
/// hand are the deployment's word. `None`: `config.v1`'s.
pub async fn resolve(
    session: &Session,
    address: &Addr,
    spec: &ConfigSpec,
) -> Option<Result<V2Target, V2Unread>> {
    let presence = if spec.across_face {
        None
    } else {
        Some(
            crate::bus::presence::observe(session, &Scope::service(address), spec.timeout)
                .await
                .map_err(|e| crate::one_line(&e)),
        )
    };
    find_v2(session, address, spec, presence.as_ref()).await
}

/// Whether `address` is read as a `config.v2` owner, and by which config
/// interface (v2 §2.1): `None` for `config.v1` (its descriptor lists
/// `config.v1`, or neither, or presence was not read, which the premise
/// then says). Across a face, the contracts in hand that use `config.v2`
/// are the deployment's word.
pub async fn find_v2(
    session: &Session,
    address: &Addr,
    spec: &ConfigSpec,
    presence: Option<&Result<Observed, String>>,
) -> Option<Result<V2Target, V2Unread>> {
    let unread = |why: String| V2Unread {
        premise: crate::report::ConfigPremise::ContractUnread,
        why,
    };
    match presence {
        None => {
            let held: Vec<_> = spec
                .contracts
                .iter()
                .filter(|r| uses_v2(r.contract()))
                .filter(|r| spec.iface.as_ref().is_none_or(|i| r.iface() == i))
                .collect();
            match held.as_slice() {
                [] if spec.iface.is_some() => Some(Err(unread(format!(
                    "no revision of {} in hand (--contracts) uses config.v2",
                    spec.iface.as_ref().expect("checked")
                )))),
                [] => None,
                [one] => Some(V2Target::of(
                    one.shared_contract(),
                    one.fingerprint().clone(),
                    spec.catalogs.epoch(Stated::Unread),
                )),
                several => Some(Err(unread(format!(
                    "across a face the deployment's word names one config interface, and \
                     {} revisions in hand use config.v2 ({}): name one with --iface, or give \
                     one revision",
                    several.len(),
                    several
                        .iter()
                        .map(|r| format!(
                            "{} {}",
                            r.iface(),
                            &r.fingerprint().hex().to_string()[..8]
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )))),
            }
        }
        Some(Err(_)) => None,
        Some(Ok(o)) => {
            if listing(o, address) == crate::report::ConfigPremise::Listed
                || listing_v2(o, address) != crate::report::ConfigPremise::Listed
            {
                return None;
            }
            let store = spec.store.clone().unwrap_or_else(|| {
                let s = BundleStore::new(spec.timeout);
                s.seed(&spec.contracts);
                s
            });
            let named = named_revisions(o, address);
            let answers = store.fetch_all(session, &named).await;
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for ((iface, fp), a) in named.iter().zip(answers) {
                match a {
                    Ok(state) => match state.revision() {
                        Some(r) if uses_v2(r.contract()) => found.push(Arc::clone(r)),
                        Some(_) => {}
                        None => missing.push(format!(
                            "{iface} {} ({})",
                            &fp.hex().to_string()[..8],
                            match &state {
                                crate::model::catalog::ContractState::Unreadable { reason } =>
                                    reason.clone(),
                                _ => "unavailable".to_owned(),
                            }
                        )),
                    },
                    Err(e) => missing.push(format!(
                        "{iface} {}: {}",
                        &fp.hex().to_string()[..8],
                        crate::one_line(&e)
                    )),
                }
            }
            let epoch = spec
                .catalogs
                .epoch(Stated::of(&Catalog::new(o).stated_catalogs(address)));
            Some(match found.as_slice() {
                [one] => V2Target::of(one.shared_contract(), one.fingerprint().clone(), epoch),
                [] => Err(unread(format!(
                    "its descriptor lists config.v2, and no revision it names that uses \
                     config.v2 is in hand{}",
                    if missing.is_empty() {
                        String::new()
                    } else {
                        format!(" (not held: {})", missing.join("; "))
                    }
                ))),
                several => Err(unread(format!(
                    "its descriptors name {} revisions that use config.v2, and an instance \
                     implements one config contract (config.v2 §2.1)",
                    several.len()
                ))),
            })
        }
    }
}

/// One S4 GET per `config.v2` owner of its record (v2 §2.3): target
/// `All`, consolidation `None`, a snapshot's entry as its reply, dated
/// under the target's epoch and merged per key by its stamp, each read by
/// its owner's contract under CH's tag. A reply on another key, or a
/// wildcard key, is no member (R6). The GETs run at once.
pub async fn get_records(
    session: &Session,
    owners: &[(Addr, V2Target)],
    timeout: Duration,
) -> Result<ConfigGet, String> {
    let selector = match owners {
        [(a, t)] => t.record_key(a),
        _ => "zk2/*/*/*/state/config".to_owned(),
    };
    let mut asks = Vec::new();
    for (addr, target) in owners {
        let key = target.record_key(addr);
        let replies = session
            .get(&key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(timeout)
            .await
            .map_err(|e| format!("GET {key}: {e}"))?;
        asks.push((addr, target, key, replies));
    }
    let mut out = ConfigGet {
        selector,
        read_at: SystemTime::now(),
        complete: true,
        services: BTreeMap::new(),
    };
    for (addr, target, key, replies) in asks {
        // A snapshot's entry stands for its reply (core §4.2; #977's E3),
        // dated under the target's epoch, every reply merged per key (S4).
        let mut samples = Vec::new();
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else {
                out.complete = false;
                continue;
            };
            samples.push(sample.clone());
        }
        // A `reply_del` of the record, which its owner answers by snapshot,
        // is no owner's answer: undecodable, `form` (#977's E4).
        let (samples, strays) =
            crate::bus::query::merged_without_strays(samples, &[&*target.contract], |s| {
                crate::bus::query::reply_samples_at(s, Some(target.epoch))
            });
        if strays.iter().any(|s| s.key_expr().as_str() == key) {
            out.services.entry(addr.clone()).or_default().record = Some(Replied {
                value: Some(Err(zenkey_model::collection::STRAY_REPLY_DEL.reason())),
                stamp: None,
            });
        }
        for sample in &samples {
            if sample.key_expr().as_str() != key {
                continue;
            }
            let value = (sample.kind() == SampleKind::Put).then(|| record_got(target, sample));
            out.services.entry(addr.clone()).or_default().record = Some(Replied {
                value,
                stamp: stamped(sample),
            });
        }
    }
    out.read_at = SystemTime::now();
    Ok(out)
}

/// A put of the record read: its envelope (core §7.5), then its bytes by
/// the contract's layout under CH's tag (core §7.6, v2 §2.3).
fn record_got(target: &V2Target, sample: &Sample) -> Doc<RecordGot> {
    let bytes = sample.payload().to_bytes();
    let encoding = sample.encoding().to_string();
    match crate::model::inband::payload(sample.key_expr().as_str(), Some(&encoding), &bytes) {
        crate::model::inband::Payload::Record { record, stamp, tag } => Ok(RecordGot {
            state: target.read(tag.get(), record)?,
            inband: Some(stamp),
            size: bytes.len(),
        }),
        crate::model::inband::Payload::Refused(r) => Err(r.reason()),
        _ => Err(format!(
            "not in a zk2c envelope under zenoh/bytes ({encoding}): a record is (core §7.5)"
        )),
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
    /// `config.v1`'s view and status, or `config.v2`'s record alone.
    docs: Docs,
    events: zenkey::consumer::Subscription,
    heard: Arc<Mutex<Events>>,
    selectors: Vec<String>,
}

/// The documents a window follows.
enum Docs {
    V1 {
        view: zenkey::consumer::Subscription,
        status: zenkey::consumer::Subscription,
        last_view: Last<ConfigView>,
        last_status: Last<ConfigStatus>,
    },
    V2 {
        key: String,
        record: zenkey::consumer::Subscription,
        last: Last<RecordGot>,
    },
}

/// Keeps one change event a window heard, the latest [`EVENT_CAP`].
fn keep_event(h: &Mutex<Events>, ev: Doc<ConfigChangeEvent>) {
    let mut e = h.lock().expect("not poisoned");
    match ev {
        Ok(ev) => {
            if e.kept.len() >= EVENT_CAP {
                e.kept.remove(0);
                e.dropped += 1;
            }
            e.kept.push(ev);
        }
        Err(_) => e.undecodable += 1,
    }
}

/// A `config.v2` change event's bytes, as `config.v1`'s is read (v2
/// §2.17).
fn decode_v2_event(bytes: &[u8]) -> Doc<ConfigChangeEvent> {
    decode::<zenkey_model::config::v2::ChangeEvent>(bytes).map(crate::model::config::event_of)
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
    /// the runtime's consumer, at once and without presence (R1), then seeds
    /// the view's and the status's views by S4's GET, as a late joiner does
    /// (`freshness.v2` §2.5; #978's F4): the owner's next heartbeat then
    /// confirms what was put before the window.
    async fn open(session: &Session, address: &Addr, timeout: Duration) -> Result<Window, String> {
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
                keep_event(
                    &h,
                    decode::<ConfigChangeEvent>(&d.sample.payload().to_bytes()),
                );
            })
            .await
            .map_err(|e| e.to_string())?;
        for (r, sub) in [(config_v1::VIEW, &view), (config_v1::STATUS, &status)] {
            let asked = Instant::now();
            if let Ok(answer) = consumer.get_answer(r, None, timeout).await {
                sub.seed(&answer, asked);
            }
        }
        Ok(Window {
            docs: Docs::V1 {
                view,
                status,
                last_view,
                last_status,
            },
            events,
            heard,
            selectors,
        })
    }

    /// A `config.v2` owner's window: its record and its change events
    /// through the runtime's consumer on its own contract, under the
    /// deployment epoch, the record's view seeded by S4's GET, so the
    /// owner's next heartbeat confirms it (`freshness.v2` §2.5).
    async fn open_v2(
        session: &Session,
        address: &Addr,
        target: &V2Target,
        timeout: Duration,
    ) -> Result<Window, String> {
        let provider = address.to_string();
        let consumer = zenkey::consumer::Consumer::for_tool(
            session,
            Arc::clone(&target.contract),
            &[provider.as_str()],
            &BTreeMap::new(),
        )
        .map_err(|e| e.to_string())?
        .with_deployment_epoch(target.epoch);
        let mut selectors = Vec::new();
        for r in [zenkey::config_v2::RECORD, zenkey::config_v2::CHANGE] {
            selectors.extend(
                consumer
                    .selectors(r)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .map(|k| k.as_str().to_owned()),
            );
        }
        let last: Last<RecordGot> = Arc::default();
        let (l, t) = (Arc::clone(&last), target.clone());
        let record = consumer
            .subscribe(
                zenkey::config_v2::RECORD,
                move |d: zenkey::consumer::Delivery| {
                    let bytes = d.sample.payload().to_bytes();
                    let value = (d.sample.kind() == SampleKind::Put).then(|| match d.tag {
                        Some(tag) => t.read(tag.get(), &bytes).map(|state| RecordGot {
                            state,
                            inband: d.inband,
                            size: bytes.len() + d.inband.map_or(0, |s| s.envelope_bytes()),
                        }),
                        None => Err("not in a zk2c envelope: a record is (core §7.5)".to_owned()),
                    });
                    *l.lock().expect("not poisoned") = Some(Replied {
                        value,
                        stamp: stamped(&d.sample),
                    });
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let heard: Arc<Mutex<Events>> = Arc::default();
        let h = Arc::clone(&heard);
        let events = consumer
            .subscribe(
                zenkey::config_v2::CHANGE,
                move |d: zenkey::consumer::Delivery| {
                    keep_event(&h, decode_v2_event(&d.sample.payload().to_bytes()));
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let asked = Instant::now();
        if let Ok(answer) = consumer
            .get_answer(zenkey::config_v2::RECORD, None, timeout)
            .await
        {
            record.seed(&answer, asked);
        }
        Ok(Window {
            docs: Docs::V2 {
                key: target.record_key(address),
                record,
                last,
            },
            events,
            heard,
            selectors,
        })
    }

    /// What the window heard, read now: each document's observation (§2.5),
    /// and each clock's measurement across its subscriptions.
    fn close(self, address: &Addr) -> ConfigWindow {
        let heard = std::mem::take(&mut *self.heard.lock().expect("not poisoned"));
        let merge = |subs: &[&zenkey::consumer::Subscription]| {
            let mut clocks = BTreeMap::new();
            for sub in subs {
                for (clock, m) in sub.clock_offsets() {
                    clocks
                        .entry(clock)
                        .and_modify(|all: &mut ClockMeasure| *all = all.merge(m))
                        .or_insert(m);
                }
            }
            clocks
        };
        let mut out = ConfigWindow {
            selectors: self.selectors,
            listened: Duration::ZERO,
            view: None,
            status: None,
            record: None,
            seen: BTreeMap::new(),
            events: heard.kept,
            events_dropped: heard.dropped,
            events_undecodable: heard.undecodable,
            clocks: BTreeMap::new(),
        };
        match self.docs {
            Docs::V1 {
                view,
                status,
                last_view,
                last_status,
            } => {
                out.clocks = merge(&[&view, &status, &self.events]);
                out.seen = BTreeMap::from([
                    (view_key(address), view.freshness(&view_key(address))),
                    (status_key(address), status.freshness(&status_key(address))),
                ]);
                out.listened = view.listened();
                out.view = last_view.lock().expect("not poisoned").clone();
                out.status = last_status.lock().expect("not poisoned").clone();
            }
            Docs::V2 { key, record, last } => {
                out.clocks = merge(&[&record, &self.events]);
                out.seen = BTreeMap::from([(key.clone(), record.freshness(&key))]);
                out.listened = record.listened();
                out.record = last.lock().expect("not poisoned").clone();
            }
        }
        out
    }
}

/// The change events a union storage holds for `address`, kept to `token`
/// (core §2.6): a wildcard GET, target `All`, consolidation `None`. A
/// deployment that runs no such storage answers nothing, which is no
/// verdict.
async fn replay(
    session: &Session,
    address: &Addr,
    v2: Option<&V2Target>,
    token: &str,
    timeout: Duration,
) -> Result<ConfigReplay, String> {
    let selector = match v2 {
        None => format!("{}/*", member(address, "events", cm::CHANGE)),
        Some(t) => [
            GRAMMAR,
            address.system.as_str(),
            address.service.as_str(),
            &t.iface.to_string(),
            "events",
            cm::CHANGE,
            "*",
        ]
        .join("/"),
    };
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
        let Ok(bytes) = crate::model::inband::record_of(sample) else {
            out.undecodable += 1;
            continue;
        };
        let event = match v2 {
            None => decode::<ConfigChangeEvent>(&bytes),
            Some(_) => decode_v2_event(&bytes),
        };
        match event {
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
    /// A `config.v2` owner's config interface (#985 G3): the request is
    /// then written as its `Set<T>` or `Control` record, and every reply
    /// read as its record. `None`: `config.v1`'s.
    pub v2: Option<V2Target>,
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

/// An owner's error (§5.2), from its error form, as the report spells it,
/// its `app` detail decoded as `ConfigError`
/// ([`zenkey::call::Refusal::detail_as`]), every sensitive value sent taken
/// out of its words.
fn refusal(env: &zenkey::call::Refusal, secrets: &[String]) -> ConfigRefusal {
    let (error, detail_undecodable) = match env.detail_as::<zenkey::codec::Json<ConfigError>>(None)
    {
        None => (None, None),
        Some(Ok(mut e)) => {
            e.detail = e.detail.map(|d| scrub(&d, secrets));
            (Some(ConfigAppError::V1(e)), None)
        }
        Some(Err(err)) => (None, Some(format!("not a ConfigError (§2.5): {err}"))),
    };
    ConfigRefusal {
        code: env.code.clone(),
        message: message(env, secrets),
        cause: env.cause.clone(),
        error,
        detail_undecodable,
        mark: env.form.map(|f| f.stamp.into()),
    }
}

/// A `config.v2` owner's refusal (v2 §2.5, §2.9): core §7.5's error form,
/// its `app` detail `Error` read by the contract's layout under the form's
/// tag, its code the packed enum's own name (`device_refused`), never
/// `config.v1`'s spelling, and the escape shown as the escape (zk2py round
/// 102, F-485). Every sensitive value sent is taken out of its words.
fn refusal_v2(env: &zenkey::call::Refusal, target: &V2Target, secrets: &[String]) -> ConfigRefusal {
    let (error, detail_undecodable) = match (&env.detail, env.form) {
        (None, _) => (None, None),
        (Some(zenkey_model::envelope::Detail::Bytes(b)), Some(form)) => {
            match target
                .interface
                .derived
                .error()
                .read(form.tag.get(), b, None)
            {
                Ok(f) => match &f["code"] {
                    serde_json::Value::String(code) => (
                        Some(ConfigAppError::V2 {
                            code: Some(code.clone()),
                        }),
                        None,
                    ),
                    serde_json::Value::Null => (Some(ConfigAppError::V2 { code: None }), None),
                    other => (
                        None,
                        Some(format!(
                            "an Error whose code is no name: {other} (config.v2 §4)"
                        )),
                    ),
                },
                Err(r) => (
                    None,
                    Some(format!(
                        "not {}'s Error (config.v2 §3): {}",
                        target.iface,
                        r.token()
                    )),
                ),
            }
        }
        (Some(_), _) => (
            None,
            Some("a detail not in the error form's own bytes (core §5.2)".to_owned()),
        ),
    };
    ConfigRefusal {
        code: env.code.clone(),
        message: message(env, secrets),
        cause: env.cause.clone(),
        error,
        detail_undecodable,
        mark: env.form.map(|f| f.stamp.into()),
    }
}

/// A refusal's message as the owner sent it, scrubbed: absent when its
/// error form carried none (core §7.5), never read back as `""`. A refusal
/// made before any wire carries one.
fn message(env: &zenkey::call::Refusal, secrets: &[String]) -> Option<String> {
    env.form
        .is_none_or(|f| f.message)
        .then(|| scrub(&env.message, secrets))
}

/// The act on a `config.v2` owner: the request, `config.v1`'s as the tool
/// typed and checked it, written as the group's `Set<T>` or a `Control`
/// record ([`crate::model::config::set_of`], `control_of`), sent through
/// the runtime's `Client` on the owner's contract in the request form; a
/// value reply read as the record (v2 §2.9). The record is never shown
/// with a sensitive value: it has no bits for one (v2 §2.15).
async fn call_v2(
    session: &Session,
    c: ConfigCall<'_>,
    target: &V2Target,
) -> crate::Result<ConfigActReport> {
    let address = c.address.to_string();
    let unaskable = |why: String| crate::Error::unaskable(&address, why);
    let iface = &target.interface;
    let (resource, record) = match &c.op {
        ConfigOp::Set { group } => {
            let req: zenkey_model::config::SetRequest =
                serde_json::from_value(c.request.clone())
                    .map_err(|e| unaskable(format!("not a set's request: {e}")))?;
            let set = crate::model::config::set_of(iface, group, &req).map_err(unaskable)?;
            let (_, layout) = iface
                .derived
                .set(group)
                .ok_or_else(|| unaskable(format!("group {group:?} has no set (config.v2 §2.4)")))?;
            let rec = layout.write(&set.to_json(), None).map_err(|r| {
                unaskable(format!(
                    "refused here, nothing sent — the request is not {group}'s Set<T>: {} \
                     (core §7.6)",
                    r.token()
                ))
            })?;
            (zenkey::config_v2::set(group), rec.bytes)
        }
        op => {
            let req: zenkey_model::config::ControlRequest =
                serde_json::from_value(c.request.clone())
                    .map_err(|e| unaskable(format!("not a control request: {e}")))?;
            let control = crate::model::config::control_of(&req).map_err(unaskable)?;
            let rec = iface
                .derived
                .control()
                .write(&control.to_json(), None)
                .map_err(|r| {
                    unaskable(format!(
                        "refused here, nothing sent — the request is not a Control: {} (core \
                         §7.6)",
                        r.token()
                    ))
                })?;
            (format!("@op/{}", op.name()), rec.bytes)
        }
    };
    let request_bytes = record.len() + 1;
    let mut client =
        zenkey::Client::new(session, Arc::clone(&target.contract), &[address.as_str()])
            .map_err(|e| runtime(&address, e))?
            .with_timeout(c.timeout)
            .with_deployment_epoch(target.epoch);
    if let Some(m) = c.metadata.clone() {
        client = client.with_metadata(m);
    }
    if c.across_face {
        client = client.with_presence(zenkey::consumer::Presence::Unavailable);
    }
    let values = Bindings::new();
    let key = client
        .key(c.address, &resource, &values)
        .map_err(|e| runtime(&address, e))?
        .as_str()
        .to_owned();
    let outcome = client
        .call(c.address, &resource, &values, record)
        .await
        .map_err(|e| runtime(&key, e))?;
    let answer = match outcome {
        zenkey::call::Outcome::Value(a) => {
            let size = a.payload().len();
            match (a.tag(), a.record()) {
                (Some(tag), Ok(bytes)) => match target.read(tag.get(), &bytes) {
                    Ok(state) => ConfigActAnswer::Record {
                        view: Box::new(view_of(iface, &state, &address)),
                        record: Box::new(state),
                        size,
                        mark: a.mark().map(Into::into),
                    },
                    Err(reason) => ConfigActAnswer::Undecodable { reason, size },
                },
                _ => ConfigActAnswer::Undecodable {
                    reason: "not core §7.5's value form: config.v2 answers every call with its \
                             record in it (config.v2 §2.9)"
                        .to_owned(),
                    size,
                },
            }
        }
        zenkey::call::Outcome::Refused(env) => ConfigActAnswer::Refused {
            refusal: refusal_v2(&env, target, &c.secrets),
        },
        zenkey::call::Outcome::NoAnswer(s) => ConfigActAnswer::Silent {
            silence: SilenceView {
                attempts: s.attempts,
                transport: s.transport.as_ref().map(ToString::to_string),
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
        profile: crate::report::ConfigProfile::V2,
        operation: c.op.name(),
        key,
        request: c.shown,
        request_bytes: Some(request_bytes),
        actor,
        request_id,
        checked: c.checked,
        across_face: c.across_face,
        timeout_s: c.timeout.as_secs_f64(),
        answer,
    })
}

/// Makes the act, and reads its reply as its union.
pub async fn call(session: &Session, c: ConfigCall<'_>) -> crate::Result<ConfigActReport> {
    if let Some(target) = c.v2.clone() {
        return call_v2(session, c, &target).await;
    }
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
        zenkey::call::Outcome::NoAnswer(s) => ConfigActAnswer::Silent {
            silence: SilenceView {
                attempts: s.attempts,
                transport: s.transport.as_ref().map(ToString::to_string),
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
        profile: crate::report::ConfigProfile::V1,
        operation: c.op.name(),
        key,
        request: c.shown,
        request_bytes: None,
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
        // A refusal made before any wire, as a test's `Api` double makes one.
        let made = zenkey::OpError::app(
            "pin 1234 refused",
            &serde_json::json!({"code": "device-refused", "detail": "bad 1234"}),
        );
        let r = refusal(&zenkey::call::Refusal::from(made), &["1234".into()]);
        assert_eq!(r.message.as_deref(), Some("pin <redacted> refused"));
        assert_eq!(r.error.unwrap().detail(), Some("bad <redacted>"));
        // An error form's detail is the ConfigError's own JSON (core §5.2).
        let form = zenkey_model::envelope::ErrorForm {
            message: Some("pin 1234 refused"),
            detail: Some(br#"{"code":"device-refused","detail":"bad 1234"}"#),
            ..zenkey_model::envelope::ErrorForm::new(
                zenkey_model::wire::Tag::NONE,
                zenkey_model::wire::Stamp {
                    run: zenkey_model::wire::Run::U16(1),
                    seq: zenkey_model::wire::Seq::new(4).unwrap(),
                },
                zenkey_model::envelope::Code::App,
                None,
            )
        };
        let r = refusal(&zenkey::call::Refusal::of_form(&form), &["1234".into()]);
        assert_eq!(r.message.as_deref(), Some("pin <redacted> refused"));
        assert_eq!(r.error.unwrap().detail(), Some("bad <redacted>"));
        // A form that carries no message reads as none, never as "" (core
        // §7.5); an empty one is a message, and stays one.
        let none = zenkey_model::envelope::ErrorForm {
            message: None,
            ..form
        };
        let r = refusal(&zenkey::call::Refusal::of_form(&none), &[]);
        assert_eq!(r.message, None);
        let empty = zenkey_model::envelope::ErrorForm {
            message: Some(""),
            ..form
        };
        let r = refusal(&zenkey::call::Refusal::of_form(&empty), &[]);
        assert_eq!(r.message.as_deref(), Some(""));
    }
}
