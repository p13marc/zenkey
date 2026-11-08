//! `link.v1`: what crosses a constrained face
//! (`spec/profiles/link/v1.md`, #769), and `link.v2`: the transport it runs
//! (`spec/profiles/link/v2.md`, #988 L1).
//!
//! The profile's session-free half: the exposure a contract gives a
//! resource (§2.2), the face policy a deployment writes and its lints
//! (§2.1), the plan a generator derives from both (§2.3–§2.12), every
//! revision of an interface read together, how a generator keeps every
//! other principal's subject off a face (§2.4), and the checks of a
//! router's block (§2.15), whether its access control is enabled among
//! them (0.7, F-124). A far principal two faces name carries both plans'
//! denies on each, which the linter warns of (§2.1, §2.4, 0.7, F-125).
//!
//! ```text
//! exposure(kind, annotations)       ─▶ host │ link │ invalid │ ignored
//! crossing(token, templated, …)     ─▶ downsampled │ whole │ denied │ none, + reason
//! parse(face policy TOML)           ─▶ the policy, or the findings that refuse it
//! plan(policy, deployment)          ─▶ per face: denies, downsampling, attachment,
//!                                      window floors, liveness evidence, warnings
//! narrowed(policy, is_far)          ─▶ the link protocols a principal's subject keeps
//! check_windows / check_block /
//!   check_subjects /
//!   check_narrowing                 ─▶ findings
//! ```
//!
//! There is no session here and no router: a generator writes the plan
//! into a router's configuration (`zenkey-fleet`'s `acl gen`), and a tool
//! reads a block somebody says the router runs. The deployment's services
//! and the contracts come in as values.
//!
//! **`link.v2`** (#988 L1) is read by the same [`parse`]: a policy whose
//! `profile` is `"link.v2"` adds the router's [`Transport`] (v2 §2.16), the
//! [`Binding`] it runs, which names its link protocols (v2 §2.4) and its
//! defaults (v2 §2.19), the catalog's id, and per face the loss, the ARQ
//! and the reply timeout (v2 §2.1); one without `profile` is a `link.v1`
//! policy, read as it was. A v2 face's `batch_bytes` and `lease_ms` are
//! its `[transport]`'s, so the plan and §2.10 read both profiles alike.
//!
//! **link.v2's contract side** (#988 L3, v2 0.3): a contract that lists
//! `link.v2` types every resource it exposes as packed ([`contract_findings`],
//! `packed_required`, v2 §2.17); a face's `frames` lists the frame resources
//! that cross it, each at its cadence, and its `horizon_s` is the shortest
//! horizon it affords (v2 §2.1, §2.7, §2.12); and under a link.v2 policy the
//! plan denies a timed stream rather than downsample it (v2 §2.8), refuses a
//! crossing revision the deployment's catalog lacks (v2 §2.18), and weighs
//! what crosses against the face (`not_packed`, `frame_unlisted`,
//! `horizon_under_face`, `block_over_batch`, `keep_alive_not_suppressed`,
//! `timed_downsampled`).
//!
//! **A device face** (#982 H1, v2 0.7): `attach = "device"` with `devices`,
//! the owners on a first hop by address (v2 §2.1, §2.5). Each device is a
//! listed owner on every face ([`FacePolicy::owners`]), but for
//! downsampling on its own face (v2 §2.8); the plan refuses one that
//! implements `archive.v1` or a governed interface (`presence_required`),
//! warns of its `epoch` templates (`epoch_on_device`), and carves `zk` into
//! three denies, so that an instance key crosses on core §11.2's Own row's
//! pairs alone ([`OWN_PAIRS`], v2 §2.6); [`check_block`] reads that carve
//! on a device face (v2 §2.15).
//!
//! ```text
//! crossing_v2(token, shape, …)      ─▶ link.v2's reading of one resource
//! contract_findings(contract)       ─▶ packed_required, per exposed resource
//! ```
//!
//! ```text
//! Transport::value(key)             ─▶ the value a generator writes at zenoh's key
//! StatedTransport::read(config)     ─▶ the keys a router's configuration states
//! check_transport(policy, router,
//!                 peer)             ─▶ transport_differs, wait_under_lease, peer_differs,
//!                                      face_timestamping
//! check_catalog(policy, held)       ─▶ catalog_mismatch
//! ```

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::Value;
use zenoh_keyexpr::keyexpr;

use crate::authoring::Kind;
use crate::chunk::is_plain_chunk;
use crate::contract::{Contract, Resource};
use crate::freshness::{self, Horizon};
use crate::grammar::KindToken;
use crate::template::{Segment, Template};

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "link.v1";

/// `archive.v1`'s interface id: a device implementing it is
/// `presence_required` (v2 §2.5).
const ARCHIVE: &str = "archive.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "link";

/// The one annotation key (§4).
pub const EXPOSURE: &str = "link.exposure";

/// The published vocabulary (§4): the keys under `link.`, which W105 reads
/// in place of core Appendix D's interim row (core 0.29).
pub const VOCABULARY: &[&str] = &["exposure"];

/// `link.v2`'s id, as a face policy's `profile` names it (v2 §2.1), and as
/// a contract lists it in `uses` (v2 §2.17).
pub const PROFILE_V2: &str = "link.v2";

/// Whether a contract lists `link.v2` in `uses`: it then types every
/// resource it exposes as packed (v2 §2.17).
#[must_use]
pub fn uses_v2(c: &Contract) -> bool {
    c.uses.iter().any(|u| u.to_string() == PROFILE_V2)
}

/// The bytes of the envelope a packed record or a frame travels in, with a
/// 32-bit run (core §7.5): CH, the run and the seq.
pub const ENVELOPE_BYTES: u64 = 8;

/// zenoh's transport lease, `transport.link.tx.lease`, when a face policy
/// gives none (§2.1).
pub const DEFAULT_LEASE_MS: u64 = 10_000;

/// The core's tombstone window when a deployment configures none (core S3).
pub const DEFAULT_WINDOW_S: u64 = 60;

/// zenoh 1.10.1's link protocol names (`InterceptorLink`, kebab-case), the
/// values `select.link_protocols` takes (§2.1); `zm-1.10.2-r1` adds
/// `unixsock-dgram` after `unixsock-stream` (v2 §2.4).
const ZENOH_1_10_1_PROTOCOLS: &[&str] = &[
    "tcp",
    "udp",
    "tls",
    "quic",
    "serial",
    "unixpipe",
    "unixsock-stream",
    "vsock",
    "ws",
];

/// The fork's `InterceptorLink` (`commons/zenoh-config/src/lib.rs` at
/// `zm-1.10.2-r1`, lines 320–331), in its order.
const ZM_1_10_2_R1_PROTOCOLS: &[&str] = &[
    "tcp",
    "udp",
    "tls",
    "quic",
    "serial",
    "unixpipe",
    "unixsock-stream",
    "unixsock-dgram",
    "vsock",
    "ws",
];

/// The nine access-control messages, in the order a plan lists them (§0).
pub const MESSAGES: [&str; 9] = [
    "put",
    "delete",
    "declare_subscriber",
    "query",
    "declare_queryable",
    "reply",
    "liveliness_token",
    "declare_liveliness_subscriber",
    "liveliness_query",
];

/// The push messages, which an exposure deny names (§0, §2.7).
pub const PUSH_MESSAGES: [&str; 3] = ["put", "delete", "declare_subscriber"];

/// Both flows, in the order a plan lists them (§0).
pub const FLOWS: [&str; 2] = ["egress", "ingress"];

/// The `@zk` deny's key expression, base-relative (§2.6).
pub const ZK_DENY: &str = "zk2/**/@zk/**";

/// The `@adv` deny's key expression, base-relative (§2.6).
pub const ADV_DENY: &str = "zk2/**/@adv/**";

/// The admin space's deny, never namespaced (§2.6).
pub const ADMIN_DENY: &str = "@/**";

/// The kind tokens whose pushes a plan denies, in its order (§2.7).
pub const PUSH_TOKENS: [KindToken; 5] = [
    KindToken::Stream,
    KindToken::ExplicitStream,
    KindToken::State,
    KindToken::ExplicitState,
    KindToken::Events,
];

const MAX_U32: u64 = u32::MAX as u64;
const MAX_BATCH: u64 = 65_535;
const MAX_REGION: usize = 32;
/// `keep_alive`'s bound (v2 §2.1): it keeps `keep_alive_flaps`'s exact
/// arithmetic small, and no medium runs more.
const MAX_KEEP_ALIVE: u64 = 255;
const MAX_LOSS_PPM: u64 = 1_000_000;

// ── link.v2: the profile, the binding, the transport (v2 §2.1, §2.4, §2.16) ─

/// Which text a face policy is read by (v2 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// No `profile`: `link.v1`, read as its §2.1 says.
    #[default]
    V1,
    /// `profile = "link.v2"`.
    V2,
}

impl Profile {
    /// The profile's id.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => PROFILE,
            Self::V2 => PROFILE_V2,
        }
    }
}

/// The zenoh a near router runs (v2 §2.4): it names the router's link
/// protocols, and the defaults a configuration that omits a transport key
/// runs with (v2 §2.19). A `link.v1` policy reads as `zenoh-1.10.1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Binding {
    /// Stock zenoh 1.10.1.
    #[default]
    Zenoh1_10_1,
    /// `zm-1.10.2-r1`, the fork zenoh-modem builds on.
    Zm1_10_2R1,
}

impl Binding {
    /// Every binding, in the order v2 §2.4 lists them.
    pub const ALL: [Binding; 2] = [Binding::Zenoh1_10_1, Binding::Zm1_10_2R1];

    /// Its name in a policy.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zenoh1_10_1 => "zenoh-1.10.1",
            Self::Zm1_10_2R1 => "zm-1.10.2-r1",
        }
    }

    /// The binding a policy's `binding` names, if any.
    #[must_use]
    pub fn parse(s: &str) -> Option<Binding> {
        Self::ALL.into_iter().find(|b| b.as_str() == s)
    }

    /// The closed list of link protocols `select.link_protocols` takes and
    /// §2.4's narrowing reads, in its order (v2 §2.4).
    #[must_use]
    pub fn protocols(self) -> &'static [&'static str] {
        match self {
            Self::Zenoh1_10_1 => ZENOH_1_10_1_PROTOCOLS,
            Self::Zm1_10_2R1 => ZM_1_10_2_R1_PROTOCOLS,
        }
    }

    /// What a router of this binding runs for a transport key its
    /// configuration omits (v2 §2.19; `zenoh-config`'s `defaults.rs`): the
    /// fork's `wait_before_close` is its lease, 10 s, where 1.10.1's is 5 s.
    #[must_use]
    pub fn defaults(self) -> Transport {
        Transport {
            lease_ms: 10_000,
            keep_alive: 4,
            wait_before_close_ms: match self {
                Self::Zenoh1_10_1 => 5_000,
                Self::Zm1_10_2R1 => 10_000,
            },
            sequence: Sequence::Bit32,
            batch_bytes: MAX_BATCH,
            open_timeout_ms: DEFAULT_TIMEOUT_MS,
            accept_timeout_ms: DEFAULT_TIMEOUT_MS,
            interests_timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Binding {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// zenoh 1.10.1's `open_timeout`, `accept_timeout` and interests timeout,
/// in ms: an optional `[transport]` key's default (v2 §2.1).
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// The frame sequence number's resolution (v2 §2.16), as zenoh's `Bits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sequence {
    Bit8,
    Bit16,
    Bit32,
    Bit64,
}

impl Sequence {
    /// Every resolution.
    pub const ALL: [Sequence; 4] = [
        Sequence::Bit8,
        Sequence::Bit16,
        Sequence::Bit32,
        Sequence::Bit64,
    ];

    /// zenoh's spelling, `"16bit"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bit8 => "8bit",
            Self::Bit16 => "16bit",
            Self::Bit32 => "32bit",
            Self::Bit64 => "64bit",
        }
    }

    /// The resolution a spelling names, if any.
    #[must_use]
    pub fn parse(s: &str) -> Option<Sequence> {
        Self::ALL.into_iter().find(|q| q.as_str() == s)
    }

    /// The window (v2 §2.16): a frame half the mask or more past the last
    /// one received is dropped as old. zenoh 1.10.1 masks a frame's sequence
    /// number to 7, 14, 28 and 32 bits (`common/seq_num.rs`, its
    /// `TransportSn` being 32 bits), so the window is 64, 8,192, 2^27 and
    /// 2^31 frames.
    #[must_use]
    pub fn window(self) -> u64 {
        match self {
            Self::Bit8 => 1 << 6,
            Self::Bit16 => 1 << 13,
            Self::Bit32 => 1 << 27,
            Self::Bit64 => 1 << 31,
        }
    }
}

impl Serialize for Sequence {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// A near router's transport, `[transport]` (v2 §2.16): zenoh 1.10.1 reads
/// it once per router, for every link. Every key resolved: an optional
/// one the policy omits is at its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Transport {
    pub lease_ms: u64,
    pub keep_alive: u64,
    pub wait_before_close_ms: u64,
    pub sequence: Sequence,
    pub batch_bytes: u64,
    pub open_timeout_ms: u64,
    pub accept_timeout_ms: u64,
    pub interests_timeout_ms: u64,
}

impl Transport {
    /// The value a generator writes at `key`'s zenoh key (v2 §2.18), in
    /// zenoh's units: `wait_before_close` in µs, the rest as the policy
    /// states them.
    #[must_use]
    pub fn value(&self, key: TransportKey) -> Value {
        match key {
            TransportKey::Lease => Value::from(self.lease_ms),
            TransportKey::KeepAlive => Value::from(self.keep_alive),
            TransportKey::WaitBeforeClose => Value::from(self.wait_before_close_ms * 1_000),
            TransportKey::Sequence => Value::from(self.sequence.as_str()),
            TransportKey::BatchBytes => Value::from(self.batch_bytes),
            TransportKey::OpenTimeout => Value::from(self.open_timeout_ms),
            TransportKey::AcceptTimeout => Value::from(self.accept_timeout_ms),
            TransportKey::InterestsTimeout => Value::from(self.interests_timeout_ms),
        }
    }

    /// The keep-alive interval, `lease_ms` / `keep_alive`, in whole ms,
    /// rounded down.
    #[must_use]
    pub fn keep_alive_interval_ms(&self) -> u64 {
        self.lease_ms / self.keep_alive.max(1)
    }

    /// The keep-alive interval in words: `"60000 ms"`, or `"about 666 ms"`
    /// where the lease is no multiple of `keep_alive`.
    #[must_use]
    pub fn keep_alive_interval_words(&self) -> String {
        let exact = self.lease_ms.is_multiple_of(self.keep_alive.max(1));
        format!(
            "{}{} ms",
            if exact { "" } else { "about " },
            self.keep_alive_interval_ms()
        )
    }
}

/// One key of `[transport]` (v2 §2.1), with zenoh 1.10.1's configuration
/// key it is written at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransportKey {
    Lease,
    KeepAlive,
    WaitBeforeClose,
    Sequence,
    BatchBytes,
    OpenTimeout,
    AcceptTimeout,
    InterestsTimeout,
}

impl TransportKey {
    /// Every key, in the policy's order (v2 §2.1).
    pub const ALL: [TransportKey; 8] = [
        TransportKey::Lease,
        TransportKey::KeepAlive,
        TransportKey::WaitBeforeClose,
        TransportKey::Sequence,
        TransportKey::BatchBytes,
        TransportKey::OpenTimeout,
        TransportKey::AcceptTimeout,
        TransportKey::InterestsTimeout,
    ];

    /// The key in `[transport]`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Lease => "lease_ms",
            Self::KeepAlive => "keep_alive",
            Self::WaitBeforeClose => "wait_before_close_ms",
            Self::Sequence => "sequence",
            Self::BatchBytes => "batch_bytes",
            Self::OpenTimeout => "open_timeout_ms",
            Self::AcceptTimeout => "accept_timeout_ms",
            Self::InterestsTimeout => "interests_timeout_ms",
        }
    }

    /// A finding's `at` for it, `transport.<key>`.
    #[must_use]
    pub fn at(self) -> String {
        format!("transport.{}", self.key())
    }

    /// zenoh 1.10.1's configuration key, `/`-separated as
    /// `zenoh::Config::insert_json5` takes it (`zenoh-config`'s `lib.rs`).
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::Lease => "transport/link/tx/lease",
            Self::KeepAlive => "transport/link/tx/keep_alive",
            Self::WaitBeforeClose => {
                "transport/link/tx/queue/congestion_control/block/wait_before_close"
            }
            Self::Sequence => "transport/link/tx/sequence_number_resolution",
            Self::BatchBytes => "transport/link/tx/batch_size",
            Self::OpenTimeout => "transport/unicast/open_timeout",
            Self::AcceptTimeout => "transport/unicast/accept_timeout",
            Self::InterestsTimeout => "routing/interests/timeout",
        }
    }

    /// The key whose zenoh key is `path`, if any.
    #[must_use]
    pub fn of_path(path: &str) -> Option<TransportKey> {
        Self::ALL.into_iter().find(|k| k.path() == path)
    }
}

/// What a generator writes beside the transport, at zenoh 1.10.1's
/// configuration key (`zenoh-config`'s `TimestampingConf`, a
/// `ModeDependentValue<bool>`): timestamping off for a router and a peer,
/// so that the near router stamps no put it routes onto the face, whichever
/// mode it runs in (v2 §2.18, core §8.2).
pub const TIMESTAMPING_OFF: [(&str, bool); 2] = [
    ("timestamping/enabled/router", false),
    ("timestamping/enabled/peer", false),
];

/// The keys a router's timestamping is read from (v2 §2.19): its mode, and
/// `timestamping/enabled`, a boolean or a value per mode.
pub const POSTURE_PATHS: [&str; 2] = ["mode", "timestamping/enabled"];

/// The transport keys a router's configuration states, by zenoh key, as
/// the file writes them (v2 §2.19): a key it omits is absent, and read at
/// the binding's default. Its mode and `timestamping/enabled`
/// ([`POSTURE_PATHS`]) beside them, where it states them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatedTransport(pub BTreeMap<String, Value>);

impl StatedTransport {
    /// The keys `config`, a router's configuration as a JSON tree, states:
    /// each of [`TransportKey::ALL`]'s and [`POSTURE_PATHS`]' whose path it
    /// holds, `null` included, since a key written down is the file's
    /// choice.
    #[must_use]
    pub fn read(config: &Value) -> StatedTransport {
        let mut out = BTreeMap::new();
        let paths = TransportKey::ALL
            .into_iter()
            .map(TransportKey::path)
            .chain(POSTURE_PATHS);
        for path in paths {
            let v = path
                .split('/')
                .try_fold(config, |node, part| node.get(part));
            if let Some(v) = v {
                out.insert(path.to_owned(), v.clone());
            }
        }
        StatedTransport(out)
    }

    /// Whether the router stamps the puts it routes (v2 §2.19, core §4.1):
    /// `timestamping/enabled` as a boolean, or its value for the mode the
    /// configuration runs, `router` where it states none, as zenohd runs; a
    /// mode the value leaves out, or no value, at zenoh 1.10.1's default,
    /// on for a router and off for a peer and a client.
    #[must_use]
    pub fn stamps(&self) -> bool {
        let mode = self
            .0
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("router");
        let default = mode == "router";
        match self.0.get("timestamping/enabled") {
            Some(Value::Bool(b)) => *b,
            Some(Value::Object(per_mode)) => per_mode
                .get(mode)
                .and_then(Value::as_bool)
                .unwrap_or(default),
            _ => default,
        }
    }

    /// Whether the configuration states `key`.
    #[must_use]
    pub fn states(&self, key: TransportKey) -> bool {
        self.0.contains_key(key.path())
    }

    /// What the configuration runs for `key`: its value, or `defaults`'s
    /// (a binding's, [`Binding::defaults`]).
    #[must_use]
    pub fn runs(&self, key: TransportKey, defaults: &Transport) -> Value {
        self.0
            .get(key.path())
            .cloned()
            .unwrap_or_else(|| defaults.value(key))
    }
}

// ── Exposure (§2.2) ──────────────────────────────────────────────────────

/// A resource's exposure, read from its resolved annotations (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    /// `"host"`, or no `link.exposure`: it never crosses a constrained face.
    Host,
    /// `"link"`: it may cross a face that exposes.
    Link,
    /// A value that is no exposure, as JSON text: read as `host`.
    Invalid(String),
    /// On an operation, whatever the value: a call crosses by grant.
    Ignored,
}

impl Exposure {
    /// The fixture's token (`exposure.json`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Link => "link",
            Self::Invalid(_) => "invalid",
            Self::Ignored => "ignored",
        }
    }
}

/// Reads the exposure of a resource of `kind` from its resolved annotations
/// (§2.2): `[defaults]`, then `[defaults.<kind>]`, then its own (core §9.3).
#[must_use]
pub fn exposure(kind: Kind, annotations: &BTreeMap<String, Value>) -> Exposure {
    if kind == Kind::Operation {
        return Exposure::Ignored;
    }
    match annotations.get(EXPOSURE) {
        None => Exposure::Host,
        Some(Value::String(s)) if s == "host" => Exposure::Host,
        Some(Value::String(s)) if s == "link" => Exposure::Link,
        Some(v) => Exposure::Invalid(v.to_string()),
    }
}

/// [`exposure`] of a resource of a contract.
#[must_use]
pub fn exposure_of(r: &Resource) -> Exposure {
    exposure(r.kind, &r.annotations)
}

/// What a face's policy lets cross (§2.1 `expose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expose {
    /// Exposed resources cross.
    Link,
    /// No push crosses.
    None,
}

/// What happens to a resource's pushes on a face (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushes {
    /// A stream that crosses, one put per key per interval (§2.8).
    Downsampled,
    /// A state or an event that crosses, every put and delete (§2.7, §2.9).
    Whole,
    /// Denied on the face.
    Denied,
    /// An operation, which has no pushes (§2.9).
    None,
}

impl Pushes {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downsampled => "downsampled",
            Self::Whole => "whole",
            Self::Denied => "denied",
            Self::None => "none",
        }
    }
}

/// Why, the first of these that holds, in this order (`exposure.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// An operation: its calls cross by grant (§2.9).
    RequestsOnly,
    /// A value that is no exposure, read as `host` (§2.2).
    Invalid,
    /// `host`, or no annotation (§2.2).
    Host,
    /// The face exposes none (§2.1).
    FaceClosed,
    /// A stream whose template has a parameter: no rule names its members
    /// (§2.8).
    Templated,
    /// link.v2's: a frame resource on a face whose `frames` does not list it
    /// (v2 §2.7).
    Unlisted,
    /// link.v2's: a stream that would be downsampled, and declares
    /// `timing.deadline_ms` (v2 §2.8).
    Timed,
    /// It crosses.
    Exposed,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestsOnly => "requests_only",
            Self::Invalid => "invalid",
            Self::Host => "host",
            Self::FaceClosed => "face_closed",
            Self::Templated => "templated",
            Self::Unlisted => "unlisted",
            Self::Timed => "timed",
            Self::Exposed => "exposed",
        }
    }
}

/// Whether a resource's pushes cross a face, and why (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    pub pushes: Pushes,
    pub reason: Reason,
}

impl Crossing {
    /// Whether its pushes cross (§0's "crossing").
    #[must_use]
    pub fn crosses(self) -> bool {
        matches!(self.pushes, Pushes::Downsampled | Pushes::Whole)
    }
}

/// Whether the pushes of a resource of `token` cross a face that exposes
/// `expose` (§2.7): `templated` when its template has a parameter.
#[must_use]
pub fn crossing(
    token: KindToken,
    templated: bool,
    exposure: &Exposure,
    expose: Expose,
) -> Crossing {
    let denied = |reason| Crossing {
        pushes: Pushes::Denied,
        reason,
    };
    if token == KindToken::Op {
        return Crossing {
            pushes: Pushes::None,
            reason: Reason::RequestsOnly,
        };
    }
    match exposure {
        Exposure::Ignored => Crossing {
            pushes: Pushes::None,
            reason: Reason::RequestsOnly,
        },
        Exposure::Invalid(_) => denied(Reason::Invalid),
        Exposure::Host => denied(Reason::Host),
        Exposure::Link if expose == Expose::None => denied(Reason::FaceClosed),
        Exposure::Link if is_stream(token) && templated => denied(Reason::Templated),
        Exposure::Link if is_stream(token) => Crossing {
            pushes: Pushes::Downsampled,
            reason: Reason::Exposed,
        },
        Exposure::Link => Crossing {
            pushes: Pushes::Whole,
            reason: Reason::Exposed,
        },
    }
}

/// What link.v2 reads of a resource's own declaration, beside its exposure,
/// to say whether it crosses (v2 §2.7, §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    /// Its template has a parameter.
    pub templated: bool,
    /// It is a frame resource (core §7.7): never downsampled.
    pub frame: bool,
    /// Its resolved annotations hold `timing.deadline_ms`, whatever its
    /// value ([`timed`]).
    pub timed: bool,
}

impl Shape {
    /// The shape of a resource of `token`: `templated` when its template
    /// has a parameter, `frame` when it is a frame resource, and timed when
    /// it is a stream whose `annotations` hold `timing.deadline_ms`.
    #[must_use]
    pub fn of(
        token: KindToken,
        templated: bool,
        frame: bool,
        annotations: &BTreeMap<String, Value>,
    ) -> Self {
        Shape {
            templated,
            frame,
            timed: is_stream(token) && timed(annotations),
        }
    }
}

/// Whether resolved annotations hold `timing.deadline_ms` (v2 §2.8). Its
/// value is not read: an invalid one still says the contract meant a
/// deadline, and the plan fails closed.
#[must_use]
pub fn timed(annotations: &BTreeMap<String, Value>) -> bool {
    annotations.contains_key(crate::timing::DEADLINE_MS)
}

/// link.v2's reading of whether a resource's pushes cross a face (v2 §2.7,
/// §2.8): [`crossing`]'s, and then a frame resource crosses whole, never
/// downsampled, unless the face's `frames` leaves it out (`listed`
/// `Some(false)`; `None` for a face with no `frames`), an exposed event
/// crosses whole unless the face's `events` leaves it out (`listed` alike,
/// from `events`), and a stream that would be downsampled is denied when
/// it is timed.
#[must_use]
pub fn crossing_v2(
    token: KindToken,
    shape: Shape,
    exposure: &Exposure,
    expose: Expose,
    listed: Option<bool>,
) -> Crossing {
    let c = crossing(token, shape.templated, exposure, expose);
    let denied = |reason| Crossing {
        pushes: Pushes::Denied,
        reason,
    };
    if token == KindToken::Events && c.pushes == Pushes::Whole && listed == Some(false) {
        return denied(Reason::Unlisted);
    }
    if c.pushes != Pushes::Downsampled {
        return c;
    }
    if shape.frame {
        if listed == Some(false) {
            return denied(Reason::Unlisted);
        }
        return Crossing {
            pushes: Pushes::Whole,
            reason: Reason::Exposed,
        };
    }
    if shape.timed {
        return denied(Reason::Timed);
    }
    c
}

/// Whether a resource breaks link.v2's packed requirement (v2 §2.17): its
/// contract lists `link.v2`, it is a stream, a state or an event exposed
/// `link`, and its type is not packed.
#[must_use]
pub fn packed_required(uses_v2: bool, token: KindToken, exposure: &Exposure, packed: bool) -> bool {
    uses_v2 && token != KindToken::Op && *exposure == Exposure::Link && !packed
}

/// A contract's findings under link.v2's contract side (v2 §2.17): for a
/// contract that lists `link.v2`, `packed_required` at each resource it
/// exposes `link` whose type is not packed, `at` its name (§0), with no
/// face. None for any other contract.
#[must_use]
pub fn contract_findings(c: &Contract) -> Vec<Finding> {
    let v2 = uses_v2(c);
    let mut out: Vec<Finding> = c
        .resources
        .iter()
        .map(|r| Res::of(c, r))
        .filter(|r| {
            packed_required(
                v2,
                r.token,
                &exposure(kind_of(r.token), &r.annotations),
                r.packed,
            )
        })
        .map(|r| {
            Finding::new(
                Code::PackedRequired,
                None,
                res_name(&c.iface.to_string(), &r),
            )
        })
        .collect();
    sort_findings(&mut out);
    out
}

fn is_stream(token: KindToken) -> bool {
    matches!(token, KindToken::Stream | KindToken::ExplicitStream)
}

fn is_state(token: KindToken) -> bool {
    matches!(token, KindToken::State | KindToken::ExplicitState)
}

/// The kind a token carries (core §1.3).
#[must_use]
pub fn kind_of(token: KindToken) -> Kind {
    match token {
        KindToken::Stream | KindToken::ExplicitStream => Kind::Stream,
        KindToken::State | KindToken::ExplicitState => Kind::State,
        KindToken::Events => Kind::Event,
        KindToken::Op => Kind::Operation,
    }
}

// ── Findings (§2.1, §2.3, §2.11, §2.15) ───────────────────────────────────

/// The closed vocabulary of findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// The file is not TOML 1.0 (§2.1).
    Toml,
    /// A key the format does not define (§2.1).
    UnknownField,
    /// A required key absent (§2.1).
    MissingField,
    /// A value of the wrong type or outside its range (§2.1).
    BadValue,
    /// `attach = "router"` (§2.5).
    RouterAttach,
    /// `select.interfaces` (§2.14).
    SelectInterfaces,
    /// A link protocol zenoh 1.10.1 does not name (§2.1).
    UnknownProtocol,
    /// Two faces with a common link protocol (§2.14).
    SharedProtocol,
    /// A batch over a third of the lease (§2.10).
    BatchOverLease,
    /// Two or more faces naming one far principal, whose policy then
    /// carries every one of their plans' denies on each of them (§2.1,
    /// §2.4, 0.7, F-125).
    FarOfTwoFaces,
    /// A listed service the deployment names no interfaces for (§2.3).
    ServiceUnknown,
    /// An interface whose contract is not given (§2.3).
    ContractUnknown,
    /// An exposed stream with a parameter, denied (§2.8).
    TemplatedExposure,
    /// An exposed resource a deny must include (§2.7).
    CarveOutOverlap,
    /// A crossing stream whose horizon is below twice the interval
    /// (§2.12).
    IntervalOverHorizon,
    /// Revisions of a listed interface that disagree on a resource's
    /// exposure, one `link` and another not: it does not cross (§2.3, 0.3).
    ExposureDisagrees,
    /// A crossing frame resource whose interval is shorter than the
    /// face's `interval_s`: never downsampled, it crosses at its own
    /// cadence (§2.8).
    FrameOverInterval,
    /// A crossing frame resource whose largest frame's payload is above
    /// the face's batch: it is fragmented, and lost whole (§2.8).
    FrameOverBatch,
    /// A crossing frame resource that declares `history`: every put pays
    /// the advanced publisher's source info across the face, and its
    /// history stays home (§2.8).
    HistoryOnFrame,
    /// A listed owner's tombstone window below the outage (§2.11).
    WindowUnderOutage,
    /// A block whose denies leave a pair of `@zk` open (§2.15).
    ZkNotDenied,
    /// A downsampling rule that is not one concrete key (§2.15).
    WildcardDownsampling,
    /// Two downsampling rules that intersect (§2.15).
    IntersectingDownsampling,
    /// A block whose access control is not enabled: zenoh enforces none of
    /// its rules (§2.15, 0.7, F-124).
    BlockDisabled,
    /// A subject on the face bound to another principal than the face's
    /// far one, whatever else it names (§2.4, §2.15, 0.3; widened in 0.4,
    /// F-120).
    NearSubjectOnFace,
    /// Under `allow`, a link protocol of the face that no face subject
    /// naming no interface and no zid selects (§2.4, §2.15, 0.3).
    FaceSubjectMissing,
    /// A face subject that also selects sessions off the face (§2.4,
    /// §2.15, 0.4, F-120).
    FaceSubjectBeyondFace,
    /// A face subject not listed in exactly the far principal's policies
    /// (§2.4, §2.15, 0.4, F-120).
    FaceSubjectMisplaced,
    /// Under `allow`, a name the block binds whose session over a link
    /// protocol it is not kept off matches no subject (§2.4, §2.15, 0.4,
    /// F-120).
    NarrowedTooFar,
    /// A link.v2 policy with no `[transport]` (v2 §2.1).
    TransportMissing,
    /// A face's `lease_ms` or `batch_bytes` in a link.v2 policy: they are
    /// `[transport]`'s (v2 §2.1).
    MovedField,
    /// `wait_before_close` below the lease, in a policy or a router's
    /// configuration (v2 §2.16, §2.19).
    WaitUnderLease,
    /// A face's loss makes more than one false close a day likely (v2
    /// §2.16).
    KeepAliveFlaps,
    /// One lease can carry the sequence window in full batches (v2 §2.16).
    SequenceWindow,
    /// An open or accept timeout below twice the handshake's air (v2
    /// §2.16).
    OpenUnderHandshake,
    /// A face's reply timeout below 16 full batches' air (v2 §2.1).
    ReplyTimeoutShort,
    /// A face whose medium repairs no lost frame (v2 §2.1).
    NoArq,
    /// A router's configuration runs a transport key other than the
    /// policy's (v2 §2.19).
    TransportDiffers,
    /// The far end's lease or keep-alive is not the near router's (v2
    /// §2.19).
    PeerDiffers,
    /// A router's configuration stamps the puts it routes onto the face
    /// (v2 §2.18, §2.19; core §8.2).
    FaceTimestamping,
    /// The catalog the far side holds is not the policy's (v2 §2.19).
    CatalogMismatch,
    /// A contract that lists `link.v2` exposes a resource whose type is not
    /// packed (v2 §2.17).
    PackedRequired,
    /// A crossing resource of a contract that does not list `link.v2`,
    /// whose type is not packed (v2 §2.3, §2.17).
    NotPacked,
    /// A face's `frames` entry that names no frame resource of a listed
    /// interface (v2 §2.7).
    FrameUnknown,
    /// A face's `events` entry that names no event resource of a listed
    /// interface (v2 §2.7). An event that is listed but not exposed is
    /// denied, never this.
    EventUnknown,
    /// An exposed frame resource of a listed interface that a face's
    /// `frames` leaves out, and that crosses no face of the policy (v2
    /// §2.7).
    FrameUnlisted,
    /// A listed frame whose `keyframe_s` its contract's `keyframe_every`
    /// cannot reach at the face's `every_s` (v2 §2.7).
    KeyframeUnreachable,
    /// A crossing heartbeat or frame resource whose horizon is below the
    /// face's `horizon_s` (v2 §2.12).
    HorizonUnderFace,
    /// A crossing `block` resource whose longest put is above the batch
    /// (v2 §2.16).
    BlockOverBatch,
    /// A face whose shortest crossing cadence is above the keep-alive
    /// interval (v2 §2.16).
    KeepAliveNotSuppressed,
    /// A timed stream, denied rather than downsampled (v2 §2.8).
    TimedDownsampled,
    /// A revision of an interface that crosses the face, which the
    /// deployment's catalog does not list (v2 §2.18).
    CatalogMissing,
    /// A device the policy lists elsewhere: among `services`, or in
    /// another face's `devices` (v2 §2.1, §2.5).
    DeviceListed,
    /// A device that implements `archive.v1`, or an interface a revision
    /// given of which uses `redundancy.v1`: each needs presence on its
    /// peers' side of the face (v2 §2.5, core §8.1).
    PresenceRequired,
    /// On a device face, a template of a device's interface that declares
    /// `epoch`: its member tokens cross the face at each transport, and the
    /// face denies them (v2 §2.5).
    EpochOnDevice,
}

impl Code {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::UnknownField => "unknown_field",
            Self::MissingField => "missing_field",
            Self::BadValue => "bad_value",
            Self::RouterAttach => "router_attach",
            Self::SelectInterfaces => "select_interfaces",
            Self::UnknownProtocol => "unknown_protocol",
            Self::SharedProtocol => "shared_protocol",
            Self::BatchOverLease => "batch_over_lease",
            Self::FarOfTwoFaces => "far_of_two_faces",
            Self::ServiceUnknown => "service_unknown",
            Self::ContractUnknown => "contract_unknown",
            Self::TemplatedExposure => "templated_exposure",
            Self::CarveOutOverlap => "carve_out_overlap",
            Self::IntervalOverHorizon => "interval_over_horizon",
            Self::ExposureDisagrees => "exposure_disagrees",
            Self::FrameOverInterval => "frame_over_interval",
            Self::FrameOverBatch => "frame_over_batch",
            Self::HistoryOnFrame => "history_on_frame",
            Self::WindowUnderOutage => "window_under_outage",
            Self::ZkNotDenied => "zk_not_denied",
            Self::WildcardDownsampling => "wildcard_downsampling",
            Self::IntersectingDownsampling => "intersecting_downsampling",
            Self::BlockDisabled => "block_disabled",
            Self::NearSubjectOnFace => "near_subject_on_face",
            Self::FaceSubjectMissing => "face_subject_missing",
            Self::FaceSubjectBeyondFace => "face_subject_beyond_face",
            Self::FaceSubjectMisplaced => "face_subject_misplaced",
            Self::NarrowedTooFar => "narrowed_too_far",
            Self::TransportMissing => "transport_missing",
            Self::MovedField => "moved_field",
            Self::WaitUnderLease => "wait_under_lease",
            Self::KeepAliveFlaps => "keep_alive_flaps",
            Self::SequenceWindow => "sequence_window",
            Self::OpenUnderHandshake => "open_under_handshake",
            Self::ReplyTimeoutShort => "reply_timeout_short",
            Self::NoArq => "no_arq",
            Self::TransportDiffers => "transport_differs",
            Self::PeerDiffers => "peer_differs",
            Self::FaceTimestamping => "face_timestamping",
            Self::CatalogMismatch => "catalog_mismatch",
            Self::PackedRequired => "packed_required",
            Self::NotPacked => "not_packed",
            Self::FrameUnknown => "frame_unknown",
            Self::EventUnknown => "event_unknown",
            Self::FrameUnlisted => "frame_unlisted",
            Self::KeyframeUnreachable => "keyframe_unreachable",
            Self::HorizonUnderFace => "horizon_under_face",
            Self::BlockOverBatch => "block_over_batch",
            Self::KeepAliveNotSuppressed => "keep_alive_not_suppressed",
            Self::TimedDownsampled => "timed_downsampled",
            Self::CatalogMissing => "catalog_missing",
            Self::DeviceListed => "device_listed",
            Self::PresenceRequired => "presence_required",
            Self::EpochOnDevice => "epoch_on_device",
        }
    }

    /// Its severity, fixed per code.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Self::BatchOverLease
            | Self::FarOfTwoFaces
            | Self::TemplatedExposure
            | Self::CarveOutOverlap
            | Self::IntervalOverHorizon
            | Self::ExposureDisagrees
            | Self::FrameOverInterval
            | Self::FrameOverBatch
            | Self::HistoryOnFrame
            | Self::KeepAliveFlaps
            | Self::SequenceWindow
            | Self::OpenUnderHandshake
            | Self::ReplyTimeoutShort
            | Self::NoArq
            | Self::PeerDiffers
            | Self::FaceTimestamping
            | Self::NotPacked
            | Self::FrameUnlisted
            | Self::HorizonUnderFace
            | Self::BlockOverBatch
            | Self::KeepAliveNotSuppressed
            | Self::TimedDownsampled
            | Self::EpochOnDevice => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// An error refuses the policy, or the plan; a warning does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One finding: what, about which face, at which key (§2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Finding {
    pub code: Code,
    pub severity: Severity,
    /// The face's name, or `None` for the top level and the deployment.
    pub face: Option<String>,
    /// The key it is about (§2.1), the service, the interface, the
    /// resource's name, or the rule's key expression.
    pub at: String,
}

impl Finding {
    /// A finding of `code`, with its fixed severity.
    #[must_use]
    pub fn new(code: Code, face: Option<&str>, at: impl Into<String>) -> Self {
        Finding {
            code,
            severity: code.severity(),
            face: face.map(str::to_owned),
            at: at.into(),
        }
    }

    /// Whether it refuses the policy or the plan.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match &self.face {
            Some(face) => write!(f, "{sev} {} [face {face}] {}", self.code, self.at),
            None => write!(f, "{sev} {} {}", self.code, self.at),
        }
    }
}

/// Sorts findings by face (none first), `at`, then code, each by its bytes,
/// and drops the repeated ones: findings are a set (§2.1).
pub fn sort_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        a.face
            .cmp(&b.face)
            .then_with(|| a.at.cmp(&b.at))
            .then_with(|| a.code.as_str().cmp(b.code.as_str()))
    });
    findings.dedup();
}

// ── The face policy (§2.1) ───────────────────────────────────────────────

/// How the far side attaches (§2.5; v2 §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attach {
    /// One far-side session, or a gateway session, as a client.
    Client,
    /// A far router in a south region, named by its `region_name`.
    SouthRegion { region: String },
    /// link.v2's device face (v2 §2.5): the sessions of owners on a first
    /// hop (core §8.1), each a client of the near router, by address
    /// `<system>/<service>`, sorted by their bytes.
    Device { devices: Vec<String> },
}

impl Attach {
    /// The devices a device face attaches; none for any other face.
    #[must_use]
    pub fn devices(&self) -> &[String] {
        match self {
            Attach::Device { devices } => devices,
            _ => &[],
        }
    }

    /// Whether it is a device face (v2 §2.5).
    #[must_use]
    pub fn is_device(&self) -> bool {
        matches!(self, Attach::Device { .. })
    }
}

/// One constrained face of a router (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    /// The far principal: a user name or a certificate CN (core §11.1).
    pub far: String,
    pub attach: Attach,
    /// `select.link_protocols`, in the policy's order.
    pub link_protocols: Vec<String>,
    pub expose: Expose,
    /// Exactly when `expose` is [`Expose::Link`].
    pub interval_s: Option<u64>,
    pub max_outage_s: u64,
    /// The face's `batch_bytes` in a `link.v1` policy; `[transport]`'s in a
    /// link.v2 one (v2 §2.10).
    pub batch_bytes: u64,
    pub link_bps: u64,
    /// The face's `lease_ms` in a `link.v1` policy, [`DEFAULT_LEASE_MS`]
    /// when it gives none; `[transport]`'s in a link.v2 one.
    pub lease_ms: u64,
    /// link.v2's (v2 §2.1): the share of frames the medium loses, in parts
    /// per million. `None` in a `link.v1` policy, as the next two.
    pub loss_ppm: Option<u64>,
    /// link.v2's: whether the medium repairs a lost frame below zenoh.
    pub arq: Option<bool>,
    /// link.v2's: how long a caller across the face waits for a reply, the
    /// deployment's word to every caller.
    pub reply_timeout_ms: Option<u64>,
    /// link.v2's `frames` (v2 §2.7): the frame resources that cross the
    /// face, by their name (§0, `<iface>/stream/<template>`), each at its
    /// cadence. `None` when the face lists none: every exposed frame
    /// resource crosses, as `link.v1` §2.7 says.
    pub frames: Option<BTreeMap<String, FrameCadence>>,
    /// link.v2's `events` (v2 §2.7): the event resources that cross the
    /// face, by their name (§0, `<iface>/events/<template>`). `None` when
    /// the face lists none: every exposed event crosses, as `link.v1` §2.7
    /// says.
    pub events: Option<BTreeSet<String>>,
    /// link.v2's `horizon_s` (v2 §2.12): the shortest horizon the face
    /// affords a crossing heartbeat or frame resource.
    pub horizon_s: Option<u64>,
}

/// A listed frame's cadence on a face (v2 §2.7): the deployment's word to
/// the far side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FrameCadence {
    /// How often its owner puts a frame on this face, in seconds.
    pub every_s: u64,
    /// The longest the face accepts between two full records, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_s: Option<u64>,
}

impl Face {
    /// Whether a batch takes at most a third of the lease to cross (§2.10):
    /// `batch_bytes` × 24,000 ≤ `lease_ms` × `link_bps`, in integers.
    #[must_use]
    pub fn batch_within_lease(&self) -> bool {
        within_lease(self.batch_bytes, self.link_bps, self.lease_ms)
    }

    /// The downsampling frequency, 1/`interval_s` Hz, the quotient as an
    /// IEEE 754 double (§2.8). `interval_s` is at most 2^32−1, which a
    /// double holds exactly.
    #[must_use]
    pub fn freq(&self) -> Option<f64> {
        self.interval_s
            .map(|n| 1.0 / f64::from(u32::try_from(n).unwrap_or(u32::MAX)))
    }
}

fn within_lease(batch_bytes: u64, link_bps: u64, lease_ms: u64) -> bool {
    u128::from(batch_bytes) * 24_000 <= u128::from(lease_ms) * u128::from(link_bps)
}

/// Whether more than one false close a day is expected (v2 §2.16): 2 ×
/// 86,400 × 1,000 × `keep_alive` × `loss_ppm`^(`keep_alive` − 1) >
/// `lease_ms` × 10^(6 × (`keep_alive` − 1)), exactly.
fn flaps(lease_ms: u64, keep_alive: u64, loss_ppm: u64) -> bool {
    use num_bigint::BigUint;
    let n = u32::try_from(keep_alive.saturating_sub(1)).unwrap_or(u32::MAX);
    let lhs = BigUint::from(172_800_000u64) * keep_alive * BigUint::from(loss_ppm).pow(n);
    let rhs = BigUint::from(lease_ms) * BigUint::from(1_000_000u64).pow(n);
    lhs > rhs
}

/// Whether one lease can carry the sequence window in full batches (v2
/// §2.16): window × 8 × `batch_bytes` × 1,000 ≤ `lease_ms` × `link_bps`.
fn window_filled(sequence: Sequence, batch_bytes: u64, lease_ms: u64, link_bps: u64) -> bool {
    u128::from(sequence.window()) * 8 * u128::from(batch_bytes) * 1_000
        <= u128::from(lease_ms) * u128::from(link_bps)
}

/// Whether a timeout is below twice the 384-byte handshake's air at
/// `link_bps` (v2 §2.16): timeout × `link_bps` < 2 × 384 × 8 × 1,000.
fn under_handshake(timeout_ms: u64, link_bps: u64) -> bool {
    u128::from(timeout_ms) * u128::from(link_bps) < 6_144_000
}

/// Whether a reply timeout is below 16 full batches' air (v2 §2.1):
/// `reply_timeout_ms` × `link_bps` < 16 × 8 × 1,000 × `batch_bytes`.
fn reply_short(reply_timeout_ms: u64, link_bps: u64, batch_bytes: u64) -> bool {
    u128::from(reply_timeout_ms) * u128::from(link_bps) < 128_000 * u128::from(batch_bytes)
}

/// A router's face policy (§2.1; v2 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacePolicy {
    /// The text it is read by.
    pub profile: Profile,
    pub system: String,
    /// The listed services, on `system`.
    pub services: Vec<String>,
    /// The faces, by name.
    pub faces: BTreeMap<String, Face>,
    /// The zenoh the near router runs (v2 §2.4); a `link.v1` policy's is
    /// `zenoh-1.10.1`.
    pub binding: Binding,
    /// The near router's transport (v2 §2.16): a link.v2 policy's, `None`
    /// for a `link.v1` one.
    pub transport: Option<Transport>,
    /// The catalog's id the far side decodes with (v2 §2.1), when a
    /// link.v2 policy names one.
    pub catalog: Option<String>,
}

impl FacePolicy {
    /// A listed service's address, `<system>/<service>`.
    #[must_use]
    pub fn address(&self, service: &str) -> String {
        format!("{}/{service}", self.system)
    }

    /// Every device a face of the policy attaches, by address, with its
    /// face's name (v2 §2.5), in the faces' order and each face's.
    pub fn devices(&self) -> impl Iterator<Item = (&str, &str)> {
        self.faces.iter().flat_map(|(name, f)| {
            f.attach
                .devices()
                .iter()
                .map(move |d| (name.as_str(), d.as_str()))
        })
    }

    /// The face that attaches the device at `address`, when one does. A
    /// device sits on one face only (`device_listed`, v2 §2.1): a policy
    /// the linter accepted never lists it twice.
    #[must_use]
    pub fn device_face(&self, address: &str) -> Option<&str> {
        debug_assert!(
            self.devices().filter(|(_, d)| *d == address).count() <= 1,
            "{address} on more than one device face: the linter's device_listed refuses it"
        );
        self.devices()
            .find(|(_, d)| *d == address)
            .map(|(name, _)| name)
    }

    /// Every listed owner the plan reads (v2 §2.5): each listed service,
    /// then each device, sorted by address. Each is `(address, the key the
    /// deployment's `implements` names it by, the face that attaches it
    /// when it is a device)`.
    #[must_use]
    pub fn owners(&self) -> Vec<Owner<'_>> {
        let mut out: Vec<Owner<'_>> = self
            .services
            .iter()
            .map(|s| Owner {
                address: self.address(s),
                key: s.as_str(),
                device_face: None,
            })
            .chain(self.devices().map(|(face, d)| Owner {
                address: d.to_owned(),
                key: d,
                device_face: Some(face),
            }))
            .collect();
        out.sort_by(|a, b| a.address.cmp(&b.address));
        out
    }
}

/// One listed owner of a policy, as the plan reads it: a listed service,
/// or a device (v2 §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner<'a> {
    /// `<system>/<service>`.
    pub address: String,
    /// The key the deployment's `implements` names it by: a listed
    /// service by its bare name (`radio-a`, on the policy's `system`), a
    /// device by its address (`lab/radio-a`), since a device may sit on
    /// any system (v2 §2.5).
    pub key: &'a str,
    /// The face that attaches it, for a device.
    pub device_face: Option<&'a str>,
}

impl FacePolicy {
    /// What a reader or a caller across `face` reads of this policy (v2
    /// §2.20, §2.21): a link.v2 policy's face. A `link.v1` policy states
    /// no reply timeout and no transport, and is refused, as a face it
    /// does not name is.
    pub fn far_side(&self, face: &str) -> Result<FarSide, FarSideRefusal> {
        let (Profile::V2, Some(transport)) = (self.profile, self.transport) else {
            return Err(FarSideRefusal::NotV2);
        };
        let Some(f) = self.faces.get(face) else {
            return Err(FarSideRefusal::NoFace {
                face: face.to_owned(),
                faces: self.faces.keys().cloned().collect(),
            });
        };
        // A link.v2 face that parsed states both (v2 §2.1).
        let (Some(reply_timeout_ms), Some(arq)) = (f.reply_timeout_ms, f.arq) else {
            return Err(FarSideRefusal::NotV2);
        };
        Ok(FarSide {
            face: face.to_owned(),
            reply_timeout_ms,
            arq,
            transport,
        })
    }
}

// ── link.v2: the far side's readers and callers (v2 §2.20, §2.21) ────────

/// How many horizons of silence a subscription across a face waits before
/// it is declared again (core §8.5, v2 §2.21).
pub const SILENT_HORIZONS: u32 = 2;

/// What a reader or a caller on the far side of one face of a link.v2
/// policy takes from the policy (v2 §2.20, §2.21; core §8.1, §8.5): how
/// long a reply may take, whether the medium repairs a lost frame, and the
/// transport its own session runs (v2 §2.16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FarSide {
    /// The face's name in the policy.
    pub face: String,
    /// `reply_timeout_ms`: the deployment's word to every caller across
    /// the face (v2 §2.1).
    pub reply_timeout_ms: u64,
    /// `arq`: whether the medium repairs a lost frame below zenoh.
    pub arq: bool,
    /// The router's `[transport]`, which both ends run (v2 §2.16).
    pub transport: Transport,
}

impl FarSide {
    /// How long a caller across the face waits at least (core §8.1).
    #[must_use]
    pub fn reply_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.reply_timeout_ms)
    }

    /// `timeout`, raised to the face's reply timeout when it is shorter
    /// (core §8.1: at least the face's).
    #[must_use]
    pub fn at_least(&self, timeout: std::time::Duration) -> std::time::Duration {
        timeout.max(self.reply_timeout())
    }

    /// Whether a reader across the face declares a silent subscription
    /// again (core §8.5): where the medium repairs no lost frame. With
    /// ARQ, a frame arrives or the transport closes, and a transport that
    /// opens again declares everything anew.
    #[must_use]
    pub fn redeclares(&self) -> bool {
        !self.arq
    }

    /// The configuration a session across the face runs (v2 §2.16, §2.18):
    /// each `[transport]` key at zenoh's key, in zenoh's units, as the far
    /// end's block writes it: a client of the near router is the gateway
    /// client's session.
    #[must_use]
    pub fn session_keys(&self) -> Vec<(TransportKey, Value)> {
        TransportKey::ALL
            .into_iter()
            .map(|k| (k, self.transport.value(k)))
            .collect()
    }

    /// The silence after which a subscription with `horizon` is declared
    /// again: [`SILENT_HORIZONS`] of it (core §8.5).
    #[must_use]
    pub fn silence(horizon: std::time::Duration) -> std::time::Duration {
        horizon.saturating_mul(SILENT_HORIZONS)
    }
}

/// Why [`FacePolicy::far_side`] gave no far side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FarSideRefusal {
    /// A `link.v1` policy: no reply timeout, no ARQ, no transport.
    NotV2,
    /// The policy has no face of that name; its faces, in order.
    NoFace { face: String, faces: Vec<String> },
}

impl fmt::Display for FarSideRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotV2 => f.write_str(
                "a link.v1 policy states no reply timeout and no transport: a far side reads a \
                 link.v2 policy (link.v2 §2.20)",
            ),
            Self::NoFace { face, faces } => write!(
                f,
                "the policy has no face {face:?}; its faces are {}",
                faces.join(", ")
            ),
        }
    }
}

/// The horizon of a subscription to `r`, a resource of `c`, that a reader
/// across a face declares again after silence (core §8.5): its
/// `freshness.ttl_s` above 0, where its owner puts at least once a
/// horizon. A state of a contract that uses `freshness.v2` has none of its
/// own: its owner re-puts nothing, and its heartbeat's subscription speaks
/// for it ([`redeclare_heartbeat_horizon`]). `None`: never silent this way.
#[must_use]
pub fn redeclare_horizon(c: &Contract, r: &Resource) -> Option<std::time::Duration> {
    if matches!(r.kind, Kind::State) && freshness::uses_v2(c) {
        return None;
    }
    freshness::horizon_of(r).ttl()
}

/// The horizon of a subscription to `c`'s heartbeat (core §8.5,
/// `freshness.v2` §2.4): the shortest of the collections it covers, where
/// `c` uses `freshness.v2` and has one.
#[must_use]
pub fn redeclare_heartbeat_horizon(c: &Contract) -> Option<std::time::Duration> {
    if !freshness::uses_v2(c) {
        return None;
    }
    crate::heartbeat::Layout::of(c).horizon()
}

/// What [`parse`] read: the policy when no error refused it, and every
/// finding, sorted.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub policy: Option<FacePolicy>,
    pub findings: Vec<Finding>,
}

const TOP_KEYS_V1: &[&str] = &["system", "services", "face"];
const TOP_KEYS_V2: &[&str] = &[
    "profile",
    "system",
    "services",
    "binding",
    "catalog",
    "transport",
    "face",
];

/// Reads a face policy, and lints it (§2.1): a `link.v1` one when it has
/// no `profile`, a link.v2 one when its `profile` is `"link.v2"` (v2 §2.1).
#[must_use]
pub fn parse(text: &str) -> Parsed {
    let refuse = |f: Finding| Parsed {
        policy: None,
        findings: vec![f],
    };
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(_) => return refuse(Finding::new(Code::Toml, None, "")),
    };
    if crate::toml10::first_beyond_toml10(text).is_some() {
        return refuse(Finding::new(Code::Toml, None, ""));
    }
    let mut out = Vec::new();
    // Any other `profile` is a wrong value, and the rest is read as a
    // link.v2 policy, whose key it is (v2 §2.1).
    let profile = match table.get("profile") {
        None => Profile::V1,
        Some(toml::Value::String(s)) if s == PROFILE_V2 => Profile::V2,
        Some(_) => {
            out.push(Finding::new(Code::BadValue, None, "profile"));
            Profile::V2
        }
    };
    let top = match profile {
        Profile::V1 => TOP_KEYS_V1,
        Profile::V2 => TOP_KEYS_V2,
    };
    for k in table.keys() {
        if !top.contains(&k.as_str()) {
            out.push(Finding::new(Code::UnknownField, None, k.clone()));
        }
    }
    let system = match table.get("system") {
        None => {
            out.push(Finding::new(Code::MissingField, None, "system"));
            None
        }
        Some(toml::Value::String(s)) if is_plain_chunk(s) => Some(s.clone()),
        Some(_) => {
            out.push(Finding::new(Code::BadValue, None, "system"));
            None
        }
    };
    let services = match table.get("services") {
        None => {
            out.push(Finding::new(Code::MissingField, None, "services"));
            None
        }
        Some(v) => match plain_list(v) {
            Some(l) => Some(l),
            None => {
                out.push(Finding::new(Code::BadValue, None, "services"));
                None
            }
        },
    };
    // link.v2's top level: the binding, the catalog, the transport.
    let (binding, catalog, transport) = match profile {
        Profile::V1 => (Some(Binding::Zenoh1_10_1), None, None),
        Profile::V2 => {
            let binding = match table.get("binding") {
                None => {
                    out.push(Finding::new(Code::MissingField, None, "binding"));
                    None
                }
                Some(toml::Value::String(s)) => {
                    let b = Binding::parse(s);
                    if b.is_none() {
                        out.push(Finding::new(Code::BadValue, None, "binding"));
                    }
                    b
                }
                Some(_) => {
                    out.push(Finding::new(Code::BadValue, None, "binding"));
                    None
                }
            };
            let catalog = match table.get("catalog") {
                None => None,
                Some(toml::Value::String(s)) if is_catalog_id(s) => Some(s.clone()),
                Some(_) => {
                    out.push(Finding::new(Code::BadValue, None, "catalog"));
                    None
                }
            };
            let transport = match table.get("transport") {
                None => {
                    out.push(Finding::new(Code::TransportMissing, None, "transport"));
                    None
                }
                Some(toml::Value::Table(t)) => Some(parse_transport(t, &mut out)),
                Some(_) => {
                    out.push(Finding::new(Code::BadValue, None, "transport"));
                    None
                }
            };
            (binding, catalog, transport)
        }
    };
    let ctx = FaceCtx {
        profile,
        protocols: binding.map(Binding::protocols),
        transport: transport.as_ref(),
    };
    let mut faces = BTreeMap::new();
    match table.get("face") {
        None => out.push(Finding::new(Code::MissingField, None, "face")),
        Some(toml::Value::Table(t)) if t.is_empty() => {
            out.push(Finding::new(Code::BadValue, None, "face"));
        }
        Some(toml::Value::Table(t)) => {
            for (name, v) in t {
                if !is_plain_chunk(name) {
                    out.push(Finding::new(Code::BadValue, Some(name), ""));
                }
                match v {
                    toml::Value::Table(ft) => {
                        if let Some(face) = parse_face(name, ft, &ctx, &mut out) {
                            faces.insert(name.clone(), face);
                        }
                    }
                    _ => out.push(Finding::new(Code::BadValue, Some(name), "")),
                }
            }
        }
        Some(_) => out.push(Finding::new(Code::BadValue, None, "face")),
    }
    if let Some(protocols) = ctx.protocols {
        shared_protocols(&table, protocols, &mut out);
    }
    shared_far(&table, &mut out);
    if profile == Profile::V2 {
        device_listed(&table, system.as_deref(), services.as_deref(), &mut out);
    }
    sort_findings(&mut out);
    let policy = if out.iter().any(Finding::is_error) {
        None
    } else {
        Some(FacePolicy {
            profile,
            system: system.unwrap_or_default(),
            services: services.unwrap_or_default(),
            faces,
            binding: binding.unwrap_or_default(),
            transport: transport.as_ref().and_then(TransportDraft::complete),
            catalog,
        })
    };
    Parsed {
        policy,
        findings: out,
    }
}

/// A catalog's id: 8 lowercase hex digits (v2 §2.1).
fn is_catalog_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `[transport]` as read, each key valid or not (`None`), an optional one
/// the policy omits at its default (v2 §2.1).
#[derive(Debug, Clone, Default)]
struct TransportDraft {
    lease_ms: Option<u64>,
    keep_alive: Option<u64>,
    wait_before_close_ms: Option<u64>,
    sequence: Option<Sequence>,
    batch_bytes: Option<u64>,
    open_timeout_ms: Option<u64>,
    accept_timeout_ms: Option<u64>,
    interests_timeout_ms: Option<u64>,
}

impl TransportDraft {
    fn complete(&self) -> Option<Transport> {
        Some(Transport {
            lease_ms: self.lease_ms?,
            keep_alive: self.keep_alive?,
            wait_before_close_ms: self.wait_before_close_ms?,
            sequence: self.sequence?,
            batch_bytes: self.batch_bytes?,
            open_timeout_ms: self.open_timeout_ms?,
            accept_timeout_ms: self.accept_timeout_ms?,
            interests_timeout_ms: self.interests_timeout_ms?,
        })
    }
}

/// Reads `[transport]` (v2 §2.1), and checks `wait_under_lease` (v2 §2.16)
/// when both keys it reads are valid.
fn parse_transport(t: &toml::Table, out: &mut Vec<Finding>) -> TransportDraft {
    let keys: Vec<&str> = TransportKey::ALL.iter().map(|k| k.key()).collect();
    for k in t.keys() {
        if !keys.contains(&k.as_str()) {
            out.push(Finding::new(
                Code::UnknownField,
                None,
                format!("transport.{k}"),
            ));
        }
    }
    let mut int = |key: TransportKey, hi: u64, default: Option<u64>| match t.get(key.key()) {
        None => {
            if default.is_none() {
                out.push(Finding::new(Code::MissingField, None, key.at()));
            }
            default
        }
        Some(v) => {
            let n = int_in(v, 1, hi);
            if n.is_none() {
                out.push(Finding::new(Code::BadValue, None, key.at()));
            }
            n
        }
    };
    let mut d = TransportDraft {
        lease_ms: int(TransportKey::Lease, MAX_U32, None),
        keep_alive: int(TransportKey::KeepAlive, MAX_KEEP_ALIVE, None),
        wait_before_close_ms: int(TransportKey::WaitBeforeClose, MAX_U32, None),
        sequence: None,
        batch_bytes: int(TransportKey::BatchBytes, MAX_BATCH, Some(MAX_BATCH)),
        open_timeout_ms: int(TransportKey::OpenTimeout, MAX_U32, Some(DEFAULT_TIMEOUT_MS)),
        accept_timeout_ms: int(
            TransportKey::AcceptTimeout,
            MAX_U32,
            Some(DEFAULT_TIMEOUT_MS),
        ),
        interests_timeout_ms: int(
            TransportKey::InterestsTimeout,
            MAX_U32,
            Some(DEFAULT_TIMEOUT_MS),
        ),
    };
    d.sequence = match t.get(TransportKey::Sequence.key()) {
        None => {
            out.push(Finding::new(
                Code::MissingField,
                None,
                TransportKey::Sequence.at(),
            ));
            None
        }
        Some(toml::Value::String(s)) => {
            let q = Sequence::parse(s);
            if q.is_none() {
                out.push(Finding::new(
                    Code::BadValue,
                    None,
                    TransportKey::Sequence.at(),
                ));
            }
            q
        }
        Some(_) => {
            out.push(Finding::new(
                Code::BadValue,
                None,
                TransportKey::Sequence.at(),
            ));
            None
        }
    };
    if let (Some(lease), Some(wbc)) = (d.lease_ms, d.wait_before_close_ms)
        && wbc < lease
    {
        out.push(Finding::new(
            Code::WaitUnderLease,
            None,
            TransportKey::WaitBeforeClose.at(),
        ));
    }
    d
}

/// An array of plain chunks, none repeated.
fn plain_list(v: &toml::Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(a.len());
    for x in a {
        let s = x.as_str().filter(|s| is_plain_chunk(s))?;
        if !seen.insert(s) {
            return None;
        }
        out.push(s.to_owned());
    }
    Some(out)
}

/// An integer of `v` within `lo..=hi`.
fn int_in(v: &toml::Value, lo: u64, hi: u64) -> Option<u64> {
    let i = v.as_integer()?;
    u64::try_from(i).ok().filter(|n| (lo..=hi).contains(n))
}

/// A link protocol list: non-empty strings, none repeated. `None` when it
/// is not one.
fn protocol_list(v: &toml::Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    if a.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(a.len());
    for x in a {
        let s = x.as_str()?;
        if !seen.insert(s) {
            return None;
        }
        out.push(s.to_owned());
    }
    Some(out)
}

const FACE_KEYS: &[&str] = &[
    "far",
    "attach",
    "region",
    "select",
    "class",
    "expose",
    "interval_s",
    "max_outage_s",
    "batch_bytes",
    "link_bps",
    "lease_ms",
];

/// A link.v2 face's keys (v2 §2.1): `link.v1`'s less `batch_bytes` and
/// `lease_ms`, which are `moved_field`, and the medium's three.
const FACE_KEYS_V2: &[&str] = &[
    "far",
    "attach",
    "region",
    "select",
    "class",
    "expose",
    "interval_s",
    "max_outage_s",
    "link_bps",
    "loss_ppm",
    "arq",
    "reply_timeout_ms",
    "horizon_s",
    "frames",
    "events",
    "devices",
];

/// What a face's `attach` names, when it is valid (§2.1; v2 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachKind {
    Client,
    South,
    Device,
}

/// A device face's `devices` (v2 §2.1): a non-empty array of addresses
/// `<system>/<service>`, each two plain chunks (core §1.2), none repeated.
/// `None` when it is not one: `devices` is one value.
fn device_list(v: &toml::Value) -> Option<Vec<String>> {
    let a = v.as_array()?;
    if a.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    for x in a {
        let s = x.as_str()?;
        let (system, service) = s.split_once('/')?;
        if !is_plain_chunk(system) || !is_plain_chunk(service) || !seen.insert(s) {
            return None;
        }
    }
    // Sorted by their bytes (v2 §2.3).
    Some(seen.into_iter().map(str::to_owned).collect())
}

/// A face's `frames` (v2 §2.1): a table of entries, each keyed
/// `<iface>/<template>`, an interface id and a template with no parameter,
/// and valued `{every_s, keyframe_s}`. Each wrong entry is reported once,
/// `at` `frames.<entry>` or a key within it; `None` when any is wrong.
fn parse_frames(
    face: &str,
    v: &toml::Value,
    out: &mut Vec<Finding>,
) -> Option<BTreeMap<String, FrameCadence>> {
    let at = Some(face);
    let toml::Value::Table(t) = v else {
        out.push(Finding::new(Code::BadValue, at, "frames"));
        return None;
    };
    let mut ok = true;
    let mut frames = BTreeMap::new();
    for (k, v) in t {
        let entry = format!("frames.{k}");
        let mut bad = |out: &mut Vec<Finding>, code: Code, key: String| {
            out.push(Finding::new(code, at, key));
            ok = false;
        };
        let name = k.split_once('/').and_then(|(i, rest)| {
            let iface: crate::grammar::IfaceId = i.parse().ok()?;
            let template = Template::parse(rest).ok().filter(|t| !t.has_params())?;
            Some(format!("{iface}/stream/{}", template.as_str()))
        });
        if name.is_none() {
            bad(out, Code::BadValue, entry.clone());
        }
        let toml::Value::Table(e) = v else {
            if name.is_some() {
                bad(out, Code::BadValue, entry);
            }
            continue;
        };
        for key in e.keys() {
            if key != "every_s" && key != "keyframe_s" {
                bad(out, Code::UnknownField, format!("{entry}.{key}"));
            }
        }
        let every_s = match e.get("every_s") {
            None => {
                bad(out, Code::MissingField, format!("{entry}.every_s"));
                None
            }
            Some(v) => {
                let n = int_in(v, 1, MAX_U32);
                if n.is_none() {
                    bad(out, Code::BadValue, format!("{entry}.every_s"));
                }
                n
            }
        };
        let keyframe_s = match e.get("keyframe_s") {
            None => Some(None),
            Some(v) => {
                let n = int_in(v, 1, MAX_U32);
                if n.is_none() {
                    bad(out, Code::BadValue, format!("{entry}.keyframe_s"));
                }
                n.map(Some)
            }
        };
        if let (Some(name), Some(every_s), Some(keyframe_s)) = (name, every_s, keyframe_s) {
            frames.insert(
                name,
                FrameCadence {
                    every_s,
                    keyframe_s,
                },
            );
        }
    }
    ok.then_some(frames)
}

/// A face's `events` (v2 §2.1, §2.7): an array of entries
/// `<iface>/<template>`, an interface id and a template, none repeated,
/// possibly empty; each kept as the event resource's name (§0,
/// `<iface>/events/<template>`). `None` when it is not one: `events` is
/// one value, reported once.
fn parse_events(face: &str, v: &toml::Value, out: &mut Vec<Finding>) -> Option<BTreeSet<String>> {
    let read = || {
        let mut names = BTreeSet::new();
        for x in v.as_array()? {
            let (i, rest) = x.as_str()?.split_once('/')?;
            let iface: crate::grammar::IfaceId = i.parse().ok()?;
            let template = Template::parse(rest).ok()?;
            if !names.insert(format!("{iface}/events/{}", template.as_str())) {
                return None;
            }
        }
        Some(names)
    };
    let events = read();
    if events.is_none() {
        out.push(Finding::new(Code::BadValue, Some(face), "events"));
    }
    events
}

/// What a face is read against: its profile, its binding's link protocols
/// (`None` when the binding is not one: the check is skipped), and a
/// link.v2 policy's `[transport]`.
struct FaceCtx<'a> {
    profile: Profile,
    protocols: Option<&'static [&'static str]>,
    transport: Option<&'a TransportDraft>,
}

fn parse_face(
    name: &str,
    t: &toml::Table,
    ctx: &FaceCtx<'_>,
    out: &mut Vec<Finding>,
) -> Option<Face> {
    let at = Some(name);
    let v2 = ctx.profile == Profile::V2;
    let mut ok = true;
    let mut bad = |out: &mut Vec<Finding>, code: Code, key: &str| {
        out.push(Finding::new(code, at, key));
        ok = false;
    };
    for k in t.keys() {
        if v2 && (k == "batch_bytes" || k == "lease_ms") {
            bad(out, Code::MovedField, k);
        } else if !(if v2 { FACE_KEYS_V2 } else { FACE_KEYS }).contains(&k.as_str()) {
            bad(out, Code::UnknownField, k);
        }
    }
    let far = match t.get("far") {
        None => {
            bad(out, Code::MissingField, "far");
            None
        }
        Some(toml::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => {
            bad(out, Code::BadValue, "far");
            None
        }
    };
    // `attach`, when it is valid: `device` only in a link.v2 policy, which
    // a link.v1 one knows nothing of (v2 §2.1).
    let attach = match t.get("attach") {
        None => {
            bad(out, Code::MissingField, "attach");
            None
        }
        Some(toml::Value::String(s)) if s == "client" => Some(AttachKind::Client),
        Some(toml::Value::String(s)) if s == "south-region" => Some(AttachKind::South),
        Some(toml::Value::String(s)) if v2 && s == "device" => Some(AttachKind::Device),
        Some(toml::Value::String(s)) if s == "router" => {
            bad(out, Code::RouterAttach, "attach");
            None
        }
        Some(_) => {
            bad(out, Code::BadValue, "attach");
            None
        }
    };
    let region = match t.get("region") {
        None => None,
        Some(toml::Value::String(s)) if !s.is_empty() && s.len() <= MAX_REGION => Some(s.clone()),
        Some(_) => {
            bad(out, Code::BadValue, "region");
            None
        }
    };
    match (attach, t.contains_key("region")) {
        (Some(AttachKind::South), false) => bad(out, Code::MissingField, "region"),
        (Some(AttachKind::Client | AttachKind::Device), true) => {
            bad(out, Code::BadValue, "region");
        }
        _ => {}
    }
    // `devices` (v2 §2.1): one value, required with `device` and refused
    // with any other `attach`, each checked against `attach` only when
    // `attach` is valid. A link.v1 face's is `unknown_field`, above.
    let devices = match t.get("devices") {
        Some(v) if v2 => {
            let l = device_list(v);
            if l.is_none() || matches!(attach, Some(AttachKind::Client | AttachKind::South)) {
                bad(out, Code::BadValue, "devices");
            }
            l
        }
        _ => None,
    };
    if attach == Some(AttachKind::Device) && !t.contains_key("devices") {
        bad(out, Code::MissingField, "devices");
    }
    let link_protocols = match t.get("select") {
        None => {
            bad(out, Code::MissingField, "select");
            None
        }
        Some(toml::Value::Table(s)) => {
            for k in s.keys() {
                if k == "interfaces" {
                    bad(out, Code::SelectInterfaces, "select.interfaces");
                } else if k != "link_protocols" {
                    bad(out, Code::UnknownField, &format!("select.{k}"));
                }
            }
            match s.get("link_protocols") {
                None => {
                    bad(out, Code::MissingField, "select.link_protocols");
                    None
                }
                Some(v) => match protocol_list(v) {
                    None => {
                        bad(out, Code::BadValue, "select.link_protocols");
                        None
                    }
                    // Against the binding's list, when the binding is one
                    // (v2 §2.4): each key is checked alone.
                    Some(l)
                        if ctx.protocols.is_some_and(|known| {
                            l.iter().any(|p| !known.contains(&p.as_str()))
                        }) =>
                    {
                        bad(out, Code::UnknownProtocol, "select.link_protocols");
                        None
                    }
                    Some(l) => Some(l),
                },
            }
        }
        Some(_) => {
            bad(out, Code::BadValue, "select");
            None
        }
    };
    match t.get("class") {
        None => bad(out, Code::MissingField, "class"),
        Some(toml::Value::String(s)) if s == "constrained" => {}
        Some(_) => bad(out, Code::BadValue, "class"),
    }
    let expose = match t.get("expose") {
        None => {
            bad(out, Code::MissingField, "expose");
            None
        }
        Some(toml::Value::String(s)) if s == "link" => Some(Expose::Link),
        Some(toml::Value::String(s)) if s == "none" => Some(Expose::None),
        Some(_) => {
            bad(out, Code::BadValue, "expose");
            None
        }
    };
    let interval_s = match t.get("interval_s") {
        None => None,
        Some(v) => match int_in(v, 1, MAX_U32) {
            Some(n) => Some(n),
            None => {
                bad(out, Code::BadValue, "interval_s");
                None
            }
        },
    };
    match (expose, t.contains_key("interval_s")) {
        (Some(Expose::Link), false) => bad(out, Code::MissingField, "interval_s"),
        (Some(Expose::None), true) => bad(out, Code::BadValue, "interval_s"),
        _ => {}
    }
    let mut int = |key: &str, lo: u64, hi: u64, required: bool| match t.get(key) {
        None => {
            if required {
                bad(out, Code::MissingField, key);
            }
            None
        }
        Some(v) => {
            let n = int_in(v, lo, hi);
            if n.is_none() {
                bad(out, Code::BadValue, key);
            }
            n
        }
    };
    let max_outage_s = int("max_outage_s", 1, MAX_U32, true);
    let link_bps = int("link_bps", 1, MAX_U32, true);
    if v2 {
        let loss_ppm = int("loss_ppm", 0, MAX_LOSS_PPM, true);
        let reply_timeout_ms = int("reply_timeout_ms", 1, MAX_U32, true);
        let horizon_s = int("horizon_s", 1, MAX_U32, false);
        let arq = match t.get("arq") {
            None => {
                bad(out, Code::MissingField, "arq");
                None
            }
            Some(toml::Value::Boolean(b)) => Some(*b),
            Some(_) => {
                bad(out, Code::BadValue, "arq");
                None
            }
        };
        let frames = match t.get("frames") {
            None => Some(None),
            Some(v) => {
                let f = parse_frames(name, v, out);
                if f.is_none() {
                    ok = false;
                }
                f.map(Some)
            }
        };
        let events = match t.get("events") {
            None => Some(None),
            Some(v) => {
                let e = parse_events(name, v, out);
                if e.is_none() {
                    ok = false;
                }
                e.map(Some)
            }
        };
        // Like `interval_s`, a face that exposes none lets nothing cross,
        // so it lists no frame and no event, and affords no horizon (v2
        // §2.1).
        if expose == Some(Expose::None) {
            for key in ["frames", "events", "horizon_s"] {
                if t.contains_key(key) {
                    out.push(Finding::new(Code::BadValue, at, key));
                    ok = false;
                }
            }
        }
        face_lints_v2(
            name,
            ctx.transport,
            link_bps,
            loss_ppm,
            reply_timeout_ms,
            arq,
            out,
        );
        if !ok {
            return None;
        }
        // The face is built only with its transport's batch and lease. When
        // either is missing, the policy is refused anyway, by the error that
        // `[transport]` itself pushed (`transport_missing`, or `bad_value` or
        // `missing_field` at its key, in `parse` and `parse_transport`), so
        // no policy is made of this face (`a_bad_transport_refuses_a_good_
        // face` holds it).
        let tr = ctx.transport?;
        return Some(Face {
            far: far?,
            attach: match attach? {
                AttachKind::Client => Attach::Client,
                AttachKind::South => Attach::SouthRegion { region: region? },
                AttachKind::Device => Attach::Device { devices: devices? },
            },
            link_protocols: link_protocols?,
            expose: expose?,
            interval_s,
            max_outage_s: max_outage_s?,
            batch_bytes: tr.batch_bytes?,
            link_bps: link_bps?,
            lease_ms: tr.lease_ms?,
            loss_ppm,
            arq,
            reply_timeout_ms,
            frames: frames?,
            events: events?,
            horizon_s,
        });
    }
    let batch_bytes = int("batch_bytes", 1, MAX_BATCH, true);
    let lease_ms = int("lease_ms", 1, MAX_U32, false);
    // `batch_over_lease` needs these three keys alone, so it is checked
    // whatever else the face holds (§2.1: each key is checked alone).
    let lease = if t.contains_key("lease_ms") {
        lease_ms
    } else {
        Some(DEFAULT_LEASE_MS)
    };
    if let (Some(b), Some(bps), Some(l)) = (batch_bytes, link_bps, lease)
        && !within_lease(b, bps, l)
    {
        out.push(Finding::new(Code::BatchOverLease, at, "batch_bytes"));
    }
    if !ok {
        return None;
    }
    Some(Face {
        far: far?,
        attach: match attach? {
            AttachKind::Client => Attach::Client,
            AttachKind::South => Attach::SouthRegion { region: region? },
            // A link.v1 policy has no device face: `attach = "device"` is
            // its `bad_value`, above.
            AttachKind::Device => return None,
        },
        link_protocols: link_protocols?,
        expose: expose?,
        interval_s,
        max_outage_s: max_outage_s?,
        batch_bytes: batch_bytes?,
        link_bps: link_bps?,
        lease_ms: if t.contains_key("lease_ms") {
            lease_ms?
        } else {
            DEFAULT_LEASE_MS
        },
        loss_ppm: None,
        arq: None,
        reply_timeout_ms: None,
        frames: None,
        events: None,
        horizon_s: None,
    })
}

/// A link.v2 face's checks that read `[transport]` (v2 §2.1, §2.10,
/// §2.16), each made when the keys it reads are valid, whatever else the
/// policy holds.
fn face_lints_v2(
    name: &str,
    tr: Option<&TransportDraft>,
    link_bps: Option<u64>,
    loss_ppm: Option<u64>,
    reply_timeout_ms: Option<u64>,
    arq: Option<bool>,
    out: &mut Vec<Finding>,
) {
    let at = Some(name);
    let tr = tr.cloned().unwrap_or_default();
    let mut warn = |code: Code, key: String| out.push(Finding::new(code, at, key));
    if let (Some(b), Some(bps), Some(l)) = (tr.batch_bytes, link_bps, tr.lease_ms)
        && !within_lease(b, bps, l)
    {
        warn(Code::BatchOverLease, TransportKey::BatchBytes.at());
    }
    if let (Some(l), Some(k), Some(p)) = (tr.lease_ms, tr.keep_alive, loss_ppm)
        && flaps(l, k, p)
    {
        warn(Code::KeepAliveFlaps, TransportKey::KeepAlive.at());
    }
    if let (Some(q), Some(b), Some(l), Some(bps)) =
        (tr.sequence, tr.batch_bytes, tr.lease_ms, link_bps)
        && window_filled(q, b, l, bps)
    {
        warn(Code::SequenceWindow, TransportKey::Sequence.at());
    }
    for (key, timeout) in [
        (TransportKey::OpenTimeout, tr.open_timeout_ms),
        (TransportKey::AcceptTimeout, tr.accept_timeout_ms),
    ] {
        if let (Some(t), Some(bps)) = (timeout, link_bps)
            && under_handshake(t, bps)
        {
            warn(Code::OpenUnderHandshake, key.at());
        }
    }
    if let (Some(r), Some(bps), Some(b)) = (reply_timeout_ms, link_bps, tr.batch_bytes)
        && reply_short(r, bps, b)
    {
        warn(Code::ReplyTimeoutShort, "reply_timeout_ms".to_owned());
    }
    if arq == Some(false) {
        warn(Code::NoArq, "arq".to_owned());
    }
}

/// `shared_protocol` (§2.14), over every face whose protocol list is one
/// and names only link protocols of the binding's list: one naming another
/// is `unknown_protocol`, a wrong value, and checks that need it are
/// skipped (§2.1).
fn shared_protocols(table: &toml::Table, known: &[&str], out: &mut Vec<Finding>) {
    let Some(toml::Value::Table(faces)) = table.get("face") else {
        return;
    };
    let lists: Vec<(&String, Vec<String>)> = faces
        .iter()
        .filter_map(|(name, v)| {
            let l = protocol_list(v.get("select")?.get("link_protocols")?)?;
            l.iter()
                .all(|p| known.contains(&p.as_str()))
                .then_some((name, l))
        })
        .collect();
    for (i, (name, l)) in lists.iter().enumerate() {
        let shared = lists
            .iter()
            .enumerate()
            .any(|(j, (_, m))| i != j && l.iter().any(|p| m.contains(p)));
        if shared {
            out.push(Finding::new(
                Code::SharedProtocol,
                Some(name),
                "select.link_protocols",
            ));
        }
    }
}

/// `far_of_two_faces` (§2.1, §2.4, 0.7, F-125), over every face whose `far`
/// is one, whatever else the policy holds: one warning per far principal
/// two or more faces name, with no face, `at` its name. Its policy carries
/// every one of their plans' denies, and no zenoh rule is limited to a link
/// protocol, so each of its faces carries them all.
fn shared_far(table: &toml::Table, out: &mut Vec<Finding>) {
    let Some(toml::Value::Table(faces)) = table.get("face") else {
        return;
    };
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    for v in faces.values() {
        if let Some(toml::Value::String(far)) = v.get("far")
            && !far.is_empty()
        {
            *named.entry(far.as_str()).or_default() += 1;
        }
    }
    for (far, n) in named {
        if n > 1 {
            out.push(Finding::new(Code::FarOfTwoFaces, None, far));
        }
    }
}

/// `device_listed` (v2 §2.1, §2.5), over every device face whose
/// `devices` is one, whatever else the policy holds: a device listed among
/// `services`, as `<system>/<service>`, or on another face. Each face
/// listing it has the finding, at `devices`.
fn device_listed(
    table: &toml::Table,
    system: Option<&str>,
    services: Option<&[String]>,
    out: &mut Vec<Finding>,
) {
    let Some(toml::Value::Table(faces)) = table.get("face") else {
        return;
    };
    let lists: Vec<(&String, Vec<String>)> = faces
        .iter()
        .filter(|(_, v)| v.get("attach").and_then(toml::Value::as_str) == Some("device"))
        .filter_map(|(name, v)| Some((name, device_list(v.get("devices")?)?)))
        .collect();
    let listed: BTreeSet<String> = match (system, services) {
        (Some(sys), Some(services)) => services.iter().map(|s| format!("{sys}/{s}")).collect(),
        _ => BTreeSet::new(),
    };
    for (i, (name, l)) in lists.iter().enumerate() {
        let elsewhere = l.iter().any(|d| {
            listed.contains(d)
                || lists
                    .iter()
                    .enumerate()
                    .any(|(j, (_, m))| i != j && m.contains(d))
        });
        if elsewhere {
            out.push(Finding::new(Code::DeviceListed, Some(name), "devices"));
        }
    }
}

// ── A router's transport (v2 §2.19) ───────────────────────────────────────

/// A transport value read as an integer: any u64, or a negative i64; `None`
/// for a value of another type.
fn integer(v: &Value) -> Option<i128> {
    v.as_u64()
        .map(i128::from)
        .or_else(|| v.as_i64().map(i128::from))
}

/// Whether two transport values say the same (v2 §2.19): integers by
/// value, anything else as JSON.
#[must_use]
pub fn same_value(a: &Value, b: &Value) -> bool {
    match (a.as_u64(), b.as_u64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Checks a router's configuration against a link.v2 policy's transport,
/// and the far end's against it (v2 §2.19): `router` and `peer` are the
/// keys each configuration states, a key it omits read at the binding's
/// default.
/// - `transport_differs`, each key whose router value is not the policy's;
/// - `wait_under_lease`, the router's `wait_before_close` below its lease,
///   whatever the policy says;
/// - `peer_differs`, the far end's lease or `keep_alive` not the router's;
/// - `face_timestamping`, the router stamping the puts it routes, read at
///   its mode ([`StatedTransport::stamps`]).
///
/// A `link.v1` policy states no transport: nothing to report.
#[must_use]
pub fn check_transport(
    policy: &FacePolicy,
    router: &StatedTransport,
    peer: Option<&StatedTransport>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let Some(planned) = &policy.transport else {
        return out;
    };
    let defaults = policy.binding.defaults();
    for key in TransportKey::ALL {
        if !same_value(&planned.value(key), &router.runs(key, &defaults)) {
            out.push(Finding::new(Code::TransportDiffers, None, key.at()));
        }
    }
    // Both read as integers, a u64's whole range among them; a negative
    // `wait_before_close`, which zenoh's i64 admits, is below any lease. A
    // value of another type (a string, a fraction) is no setting zenoh runs:
    // `transport_differs` has reported it above, since what the policy
    // plans is always an integer there, and this check has nothing to read.
    let lease_us = integer(&router.runs(TransportKey::Lease, &defaults)).map(|ms| ms * 1_000);
    let wbc_us = integer(&router.runs(TransportKey::WaitBeforeClose, &defaults));
    if let (Some(lease), Some(wbc)) = (lease_us, wbc_us)
        && wbc < lease
    {
        out.push(Finding::new(
            Code::WaitUnderLease,
            None,
            TransportKey::WaitBeforeClose.at(),
        ));
    }
    if let Some(peer) = peer {
        for key in [TransportKey::Lease, TransportKey::KeepAlive] {
            if !same_value(&router.runs(key, &defaults), &peer.runs(key, &defaults)) {
                out.push(Finding::new(Code::PeerDiffers, None, key.at()));
            }
        }
    }
    if router.stamps() {
        out.push(Finding::new(Code::FaceTimestamping, None, "timestamping"));
    }
    sort_findings(&mut out);
    out
}

/// The id of the deployment catalog the far side holds (v2 §2.19): the
/// catalog read as core §8.6 reads one, and its id §8.6's, as 8 lowercase
/// hex digits.
///
/// # Errors
/// The first refusal of §8.6's reading: such a file is no catalog, and
/// holds no id to compare.
pub fn catalog_id(text: &str) -> Result<String, crate::catalog::Refusal> {
    crate::catalog::Catalog::parse(text).map(|c| c.id().hex())
}

/// `catalog_mismatch` (v2 §2.19): the id of the catalog the far side holds,
/// `held`, is not the policy's, or the policy names none.
#[must_use]
pub fn check_catalog(policy: &FacePolicy, held: &str) -> Vec<Finding> {
    if policy.catalog.as_deref() == Some(held) {
        Vec::new()
    } else {
        vec![Finding::new(Code::CatalogMismatch, None, "catalog")]
    }
}

// ── The deployment ───────────────────────────────────────────────────────

/// One resource as the plan reads it: its template, its kind token, its
/// resolved annotations, whether it declares `history` (core §2.5), and,
/// for a frame resource (core §7.7), what the plan weighs of its frames;
/// and what link.v2 reads of it (v2 §2.16, §2.17): whether its type is
/// packed, whether its congestion is `block`, its longest put, and whether
/// its contract lists `link.v2`.
#[derive(Debug, Clone)]
pub struct Res {
    pub template: Template,
    pub token: KindToken,
    pub annotations: BTreeMap<String, Value>,
    pub history: bool,
    pub frame: Option<FrameFacts>,
    /// Its type is packed (core §7.6); a frame resource's always is. Read
    /// across revisions, packed only where every one types it packed (v2
    /// §2.3).
    pub packed: bool,
    /// Its congestion is `block` (core §2.4); across revisions, where any
    /// one declares it so.
    pub block: bool,
    /// Its longest put's payload, in bytes: a packed record's longest
    /// (every optional field present) after an [`ENVELOPE_BYTES`] envelope,
    /// or a frame resource's largest frame. `None` for any other type.
    /// Across revisions, the longest of those that declare it `block`, or
    /// of every one where none does.
    pub longest_bytes: Option<u64>,
    /// The revision that declares it lists `link.v2` (v2 §2.17).
    pub link_v2: bool,
    /// The parameter its template declares as its epoch (core §4.3), whose
    /// members hold member tokens: on a device face, `epoch_on_device` (v2
    /// §2.5). Across revisions, any one's.
    pub epoch: Option<String>,
    /// A state whose deletes only a tombstone proves (v2 §2.11, 0.9; core
    /// S3): a per-key resource (core §4.2), or a template whose first chunk
    /// is a parameter, which has no snapshot key. [`tombstone_only`] reads
    /// it; across revisions, where any one is.
    pub tombstone_only: bool,
}

impl Res {
    /// A resource of `token` at `template`, with resolved `annotations`, and
    /// nothing else stated: no history, no frame, a type that is not packed,
    /// `drop`, in a revision that does not list `link.v2` and uses no
    /// profile ([`tombstone_only`]).
    #[must_use]
    pub fn new(template: Template, token: KindToken, annotations: BTreeMap<String, Value>) -> Self {
        Res {
            history: false,
            frame: None,
            packed: false,
            block: false,
            longest_bytes: None,
            link_v2: false,
            epoch: None,
            tombstone_only: tombstone_only(token, &template, &annotations, |_| false),
            template,
            token,
            annotations,
        }
    }

    /// Its shape, as [`crossing_v2`] reads it.
    #[must_use]
    pub fn shape(&self) -> Shape {
        Shape::of(
            self.token,
            self.template.has_params(),
            self.frame.is_some(),
            &self.annotations,
        )
    }
}

/// What the plan reads of a frame resource (§2.8): never downsampled, it
/// crosses at its own cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFacts {
    /// Its interval (core §7.7): its span, else its `timing.period_ms`,
    /// else unstated.
    pub interval_ms: Option<u64>,
    /// The payload of the largest frame its owner writes, in bytes
    /// ([`crate::frame::Layout::largest_payload_bytes`]); `None` when its
    /// record type's layout does not read.
    pub largest_bytes: Option<u64>,
    /// Its `keyframe_every` (core §7.7): a full record at least once every
    /// this many records. Across revisions, the least (v2 §2.7).
    pub keyframe_every: u32,
}

impl Res {
    /// `r` of contract `c`, as the plan reads it.
    #[must_use]
    pub fn of(c: &Contract, r: &Resource) -> Self {
        let data = match &r.body {
            crate::contract::Body::Data(d) => Some(d),
            crate::contract::Body::Operation(_) => None,
        };
        let frame = data.and_then(|d| d.frame.as_ref()).map(|p| FrameFacts {
            interval_ms: p.interval_ms(&r.annotations),
            largest_bytes: crate::frame::Layout::of_resource(c, r)
                .ok()
                .flatten()
                .map(|l| l.largest_payload_bytes()),
            keyframe_every: p.keyframe_every,
        });
        let packed = data.is_some_and(|d| d.type_.is_packed());
        let longest_bytes = match (&frame, data) {
            (Some(f), _) => f.largest_bytes,
            (None, Some(d)) if packed => c
                .packed_layout(&d.type_)
                .ok()
                .flatten()
                .map(|l| ENVELOPE_BYTES + l.max_bits().div_ceil(8)),
            _ => None,
        };
        Res {
            template: r.template.clone(),
            token: r.token,
            annotations: r.annotations.clone(),
            history: data.is_some_and(|d| d.history.is_some()),
            frame,
            packed,
            block: data.is_some_and(|d| d.congestion == crate::authoring::Congestion::Block),
            longest_bytes,
            link_v2: uses_v2(c),
            epoch: r.epoch.clone(),
            tombstone_only: tombstone_only(r.token, &r.template, &r.annotations, |p| {
                c.uses.iter().any(|u| u.to_string() == p)
            }),
        }
    }
}

/// Whether a resource of `token` at `template`, with resolved
/// `annotations`, in a revision whose `uses` lists the profiles `uses`
/// says, is a state whose deletes only a tombstone proves (v2 §2.11, 0.9):
///
/// - a per-key resource (core §4.2,
///   [`crate::collection::per_key_in`]): a state that declares
///   `freshness.ttl_s` above 0, of a revision that does not use
///   `freshness.v2`; one whose annotations hold `desired.target_param`; a
///   metric resource of `telemetry.v1` or `telemetry.v2` (a key of its
///   vocabulary), of a revision that uses one; any state of a revision
///   that uses `redundancy.v1`;
/// - a template whose first chunk is a parameter, which has no snapshot
///   key (core §4.2).
///
/// Every other state is read by snapshot, whose complete answer omits a
/// member deleted beyond the window.
#[must_use]
pub fn tombstone_only(
    token: KindToken,
    template: &Template,
    annotations: &BTreeMap<String, Value>,
    uses: impl Fn(&str) -> bool,
) -> bool {
    is_state(token)
        && (crate::collection::per_key_in(token, annotations, uses)
            || crate::collection::snapshot_prefix(template).is_none())
}

/// What the plan needs of the deployment (§2.3): the interfaces each listed
/// service implements, and the revisions given of each interface.
#[derive(Debug, Clone, Default)]
pub struct Deployment {
    /// A service's name → the interface ids it implements.
    pub implements: BTreeMap<String, Vec<String>>,
    /// An interface id → each revision given of it, as its resources. Two
    /// minors of one major are two revisions of one id: an annotation
    /// changed between them is a review (core §9.8), and two services of
    /// one deployment may run either (core §3.3). The plan reads them
    /// together, failing closed where they disagree (§2.3).
    pub interfaces: BTreeMap<String, Vec<Vec<Res>>>,
    /// The interface ids a revision given of which uses `freshness.v2`:
    /// their owners send a heartbeat covering them (`freshness.v2` §2.4),
    /// which crosses where a collection it covers does (§2.9).
    pub heartbeats: BTreeSet<String>,
    /// An interface id → the fingerprints of the revisions given of it as
    /// contracts, which link.v2's catalog check reads (v2 §2.18). A
    /// revision given as its resources alone has none.
    pub fingerprints: BTreeMap<String, BTreeSet<crate::canonical::Fingerprint>>,
    /// The catalog the far side decodes with (core §8.6), when the
    /// deployment gives it: a link.v2 plan then refuses a crossing revision
    /// it does not list (v2 §2.18).
    pub catalog: Option<crate::catalog::Catalog>,
    /// The interface ids a revision given of which uses `redundancy.v1`:
    /// a device implementing one is `presence_required` (v2 §2.5).
    pub governed: BTreeSet<String>,
}

impl Deployment {
    /// Adds a contract's resources as a revision of its interface id, its
    /// heartbeat when it uses `freshness.v2`, and its fingerprint.
    pub fn add_contract(&mut self, c: &Contract) {
        if freshness::uses_v2(c) {
            self.heartbeats.insert(c.iface.to_string());
        }
        if crate::redundancy::is_governed(c) {
            self.governed.insert(c.iface.to_string());
        }
        self.fingerprints
            .entry(c.iface.to_string())
            .or_default()
            .insert(crate::canonical::Fingerprint::of(c));
        self.add_revision(
            c.iface.to_string(),
            c.resources.iter().map(|r| Res::of(c, r)).collect(),
        );
    }

    /// Adds one revision of `iface`, as its resources.
    pub fn add_revision(&mut self, iface: impl Into<String>, resources: Vec<Res>) {
        self.interfaces
            .entry(iface.into())
            .or_default()
            .push(resources);
    }
}

/// An interface's resources as the plan reads them across its revisions
/// (§2.3): each resource any revision declares, paired by kind token and
/// template, first revision first. A resource several revisions declare is
/// exposed only where every one of them reads `link` (§2.2 fails closed),
/// and has a horizon only where every one gives it a horizon above 0, the
/// least of them (§2.12). What link.v2 weighs is judged revision by
/// revision (v2 §2.3, 0.6, F-469), and so never hangs on their order: a
/// resource is packed only where every revision types it packed, timed
/// where any declares a deadline, and its longest put, for
/// `block_over_batch`, the longest a revision that declares it `block`
/// writes. A frame's largest frame is the largest any revision writes
/// (§2.3). The set holds the indices of the resources the revisions
/// disagree on: one reads `link`, another does not.
fn merge(revisions: &[Vec<Res>]) -> (Vec<Res>, BTreeSet<usize>) {
    let mut out: Vec<Res> = Vec::new();
    let mut readings: Vec<Vec<&Res>> = Vec::new();
    for rev in revisions {
        for r in rev {
            match out
                .iter()
                .position(|x| x.token == r.token && x.template == r.template)
            {
                Some(i) => readings[i].push(r),
                None => {
                    out.push(r.clone());
                    readings.push(vec![r]);
                }
            }
        }
    }
    let mut disagree = BTreeSet::new();
    for (i, rs) in readings.iter().enumerate() {
        let links = rs
            .iter()
            .filter(|r| exposure(kind_of(r.token), &r.annotations) == Exposure::Link)
            .count();
        // A frame's `history` is warned of where any revision declares it
        // (§2.8); its `frame` table is one in every revision of a major,
        // since a change to it is breaking (core §9.8), but for
        // `keyframe_every`, a review, of which the least binds (v2 §2.7).
        out[i].history = rs.iter().any(|r| r.history);
        if out[i].epoch.is_none() {
            out[i].epoch = rs.iter().find_map(|r| r.epoch.clone());
        }
        if let Some(f) = &mut out[i].frame {
            f.keyframe_every = rs
                .iter()
                .filter_map(|r| r.frame.map(|f| f.keyframe_every))
                .min()
                .unwrap_or(f.keyframe_every);
            // Each revision's largest frame, with its own record type and
            // `keyframe_every`: the largest of them.
            f.largest_bytes = rs
                .iter()
                .filter_map(|r| r.frame.and_then(|f| f.largest_bytes))
                .max();
        }
        // Each revision judged alone (v2 §2.3): a raw type in one draws
        // `not_packed`, a deadline in one makes it timed, and a `block`
        // one's longest put draws `block_over_batch`. A packed type grows
        // by a minor (core §7.6), so the longest is any revision's where
        // none is `block`.
        out[i].packed = rs.iter().all(|r| r.packed);
        if !timed(&out[i].annotations)
            && let Some(d) = rs
                .iter()
                .find_map(|r| r.annotations.get(crate::timing::DEADLINE_MS))
        {
            out[i]
                .annotations
                .insert(crate::timing::DEADLINE_MS.to_owned(), d.clone());
        }
        let block = rs.iter().any(|r| r.block);
        out[i].block = block;
        out[i].tombstone_only = rs.iter().any(|r| r.tombstone_only);
        out[i].longest_bytes = rs
            .iter()
            .filter(|r| r.block || !block)
            .filter_map(|r| r.longest_bytes)
            .max();
        if links > 0 && links < rs.len() {
            disagree.insert(i);
            out[i]
                .annotations
                .insert(EXPOSURE.to_owned(), Value::String("host".to_owned()));
        }
        let ttls: Vec<Option<std::time::Duration>> = rs.iter().map(|r| horizon(r).ttl()).collect();
        if ttls.windows(2).any(|w| w[0] != w[1]) {
            match ttls.into_iter().collect::<Option<Vec<_>>>() {
                Some(all) => {
                    let least = all.into_iter().min().unwrap_or_default();
                    out[i]
                        .annotations
                        .insert(freshness::TTL_S.to_owned(), Value::from(least.as_secs()));
                }
                None => {
                    out[i].annotations.remove(freshness::TTL_S);
                }
            }
        }
    }
    (out, disagree)
}

// ── The plan (§2.3–§2.12) ────────────────────────────────────────────────

/// One deny (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deny {
    pub id: String,
    pub flows: Vec<&'static str>,
    pub messages: Vec<&'static str>,
    pub key_exprs: Vec<String>,
}

/// One downsampling rule (§2.8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub key_expr: String,
    pub freq: f64,
}

/// A face's downsampling block (§2.8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Downsampling {
    pub id: String,
    pub link_protocols: Vec<String>,
    pub flows: Vec<&'static str>,
    pub messages: Vec<&'static str>,
    pub rules: Vec<Rule>,
}

/// One `gateway.south` filter (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Filter {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub region_names: Vec<String>,
}

/// One south subregion (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct South {
    pub filters: Vec<Filter>,
}

/// The near router's `gateway` block (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gateway {
    pub south: Vec<South>,
}

/// The attachment, as the plan carries it (§2.5; v2 §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "shape")]
pub enum AttachPlan {
    #[serde(rename = "client")]
    Client,
    #[serde(rename = "south-region")]
    SouthRegion { region: String, gateway: Gateway },
    /// A device face's (v2 §2.3): its devices, sorted by their bytes.
    #[serde(rename = "device")]
    Device { devices: Vec<String> },
}

/// A listed owner of state's tombstone window floor (§2.11): under a
/// link.v2 policy, an owner of state whose deletes only a tombstone proves
/// ([`Res::tombstone_only`], v2 §2.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Window {
    pub service: String,
    pub min_s: u64,
}

/// A listed service's liveness evidence across the face (§2.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Liveness {
    pub service: String,
    pub evidence: Vec<String>,
}

/// One face's plan (§2.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FacePlan {
    pub name: String,
    pub far: String,
    pub attach: AttachPlan,
    pub deny: Vec<Deny>,
    pub downsampling: Option<Downsampling>,
    pub windows: Vec<Window>,
    pub liveness: Vec<Liveness>,
    /// A link.v2 face's reply timeout, the deployment's word to every
    /// caller across it (v2 §2.18); absent for a `link.v1` face.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_timeout_ms: Option<u64>,
    /// A link.v2 face's listed frames that cross it, each at its cadence,
    /// sorted by name: the deployment's word to the far side (v2 §2.7,
    /// §2.18). Absent for a face with no `frames`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<ListedFrame>>,
    pub findings: Vec<Finding>,
    /// What crosses the face, by name (§0): each resource of a listed
    /// interface whose pushes cross, overlapped ones set aside, and each
    /// crossing heartbeat as `freshness.v2/stream/heartbeat/<iface>`
    /// (§2.9). A tool's scope across the face is read from it (observer
    /// §7.5). No router block carries it, so it is never serialized.
    #[serde(skip)]
    pub crossing: BTreeSet<String>,
}

impl FacePlan {
    /// Whether resource `token`/`template` of `iface` crosses the face.
    #[must_use]
    pub fn crosses(&self, iface: &str, token: KindToken, template: &Template) -> bool {
        self.crossing
            .contains(&format!("{iface}/{}/{}", token.as_str(), template.as_str()))
    }

    /// Whether the heartbeat covering `iface` crosses the face (§2.9).
    #[must_use]
    pub fn heartbeat_crosses(&self, iface: &str) -> bool {
        self.crossing.contains(&heartbeat_name(iface))
    }
}

/// A heartbeat's name in a plan, `freshness.v2/stream/heartbeat/<iface>`.
fn heartbeat_name(iface: &str) -> String {
    format!("{}/stream/heartbeat/{iface}", freshness::PROFILE_V2)
}

/// A listed frame that crosses, as the plan carries it (v2 §2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedFrame {
    /// Its name (§0).
    pub resource: String,
    #[serde(flatten)]
    pub cadence: FrameCadence,
}

/// What [`plan_face`] found that the policy's other faces need: the
/// resources crossing it, the exposed frames it leaves unlisted, and the
/// interfaces with something crossing it (v2 §2.7, §2.18).
#[derive(Debug, Default)]
struct FaceFacts {
    crossing: BTreeSet<String>,
    unlisted: BTreeSet<String>,
    ifaces: BTreeSet<String>,
}

/// A resource's name in a plan, `<iface>/<token>/<template>` (§0).
fn res_name(iface: &str, r: &Res) -> String {
    format!("{iface}/{}/{}", r.token.as_str(), r.template.as_str())
}

/// Derives each face's plan (§2.3), faces sorted by name; or the
/// deployment's errors, which refuse it.
pub fn plan(policy: &FacePolicy, deployment: &Deployment) -> Result<Vec<FacePlan>, Vec<Finding>> {
    let mut errors = Vec::new();
    let mut listed: BTreeSet<&str> = BTreeSet::new();
    // The listed services and the devices, each read alike (v2 §2.5): a
    // device is named in the deployment by its address, and a device the
    // deployment names none for is `service_unknown` there (v2 §2.3).
    let services = policy.owners();
    for s in &services {
        match deployment.implements.get(s.key) {
            None => errors.push(Finding::new(Code::ServiceUnknown, None, s.key)),
            Some(ifaces) => {
                for i in ifaces {
                    if deployment
                        .interfaces
                        .get(i)
                        .is_some_and(|revs| !revs.is_empty())
                    {
                        listed.insert(i);
                    } else {
                        errors.push(Finding::new(Code::ContractUnknown, None, i.clone()));
                    }
                    // An owner on a first hop implements neither (v2 §2.5,
                    // core §8.1).
                    if let Some(face) = s.device_face
                        && (i == ARCHIVE || deployment.governed.contains(i))
                    {
                        errors.push(Finding::new(Code::PresenceRequired, Some(face), i.clone()));
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        sort_findings(&mut errors);
        return Err(errors);
    }
    // Each listed interface read once across its revisions (§2.3).
    let view: View = listed
        .iter()
        .map(|i| ((*i).to_owned(), merge(&deployment.interfaces[*i])))
        .collect();
    let v2 = policy.profile == Profile::V2;
    // link.v2's contract side, read per revision given (v2 §2.17).
    if v2 {
        for i in &listed {
            for r in deployment.interfaces[*i].iter().flatten() {
                let e = exposure(kind_of(r.token), &r.annotations);
                if packed_required(r.link_v2, r.token, &e, r.packed) {
                    errors.push(Finding::new(Code::PackedRequired, None, res_name(i, r)));
                }
            }
        }
    }
    let mut faces: Vec<(FacePlan, FaceFacts)> = policy
        .faces
        .iter()
        .map(|(name, face)| {
            plan_face(
                policy,
                deployment,
                &view,
                &services,
                &listed,
                name,
                face,
                &mut errors,
            )
        })
        .collect();
    if v2 {
        // An exposed frame no face lets across (v2 §2.7).
        let crossing: BTreeSet<String> = faces
            .iter()
            .flat_map(|(_, f)| f.crossing.iter().cloned())
            .collect();
        for (p, f) in &mut faces {
            for n in f.unlisted.iter().filter(|n| !crossing.contains(*n)) {
                p.findings
                    .push(Finding::new(Code::FrameUnlisted, Some(&p.name), n.clone()));
            }
            sort_findings(&mut p.findings);
        }
        // Every crossing revision is one the far side can decode (v2 §2.18).
        if let Some(cat) = &deployment.catalog {
            let ifaces: BTreeSet<&String> = faces.iter().flat_map(|(_, f)| &f.ifaces).collect();
            for i in ifaces {
                let Ok(id) = i.parse::<crate::grammar::IfaceId>() else {
                    continue;
                };
                for fp in deployment.fingerprints.get(i).into_iter().flatten() {
                    if cat.index_of(&id, fp).is_none() {
                        errors.push(Finding::new(
                            Code::CatalogMissing,
                            None,
                            format!("{i}@{fp}"),
                        ));
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        sort_findings(&mut errors);
        return Err(errors);
    }
    Ok(faces.into_iter().map(|(p, _)| p).collect())
}

/// Each listed interface's resources across its revisions, and the ones
/// they disagree on ([`merge`]).
type View = BTreeMap<String, (Vec<Res>, BTreeSet<usize>)>;

#[allow(clippy::too_many_arguments)]
fn plan_face(
    policy: &FacePolicy,
    deployment: &Deployment,
    view: &View,
    services: &[Owner<'_>],
    listed: &BTreeSet<&str>,
    name: &str,
    face: &Face,
    errors: &mut Vec<Finding>,
) -> (FacePlan, FaceFacts) {
    let mut findings = Vec::new();
    let mut facts = FaceFacts::default();
    let v2 = policy.profile == Profile::V2;
    let resources_of = |iface: &str| -> &[Res] { &view[iface].0 };
    // Whether each resource of each listed interface crosses (§2.7; v2
    // §2.7, §2.8).
    let crossing_of = |iface: &str, r: &Res| {
        let e = exposure(kind_of(r.token), &r.annotations);
        if v2 {
            let name = res_name(iface, r);
            let listed = if r.token == KindToken::Events {
                face.events.as_ref().map(|l| l.contains(&name))
            } else {
                face.frames.as_ref().map(|f| f.contains_key(&name))
            };
            crossing_v2(r.token, r.shape(), &e, face.expose, listed)
        } else {
            crossing(r.token, r.template.has_params(), &e, face.expose)
        }
    };
    // A listed frame's entry names a frame resource of a listed interface
    // (v2 §2.7).
    for entry in face.frames.iter().flat_map(BTreeMap::keys) {
        let known = listed.iter().any(|i| {
            resources_of(i)
                .iter()
                .any(|r| r.frame.is_some() && &res_name(i, r) == entry)
        });
        if !known {
            errors.push(Finding::new(Code::FrameUnknown, Some(name), entry.clone()));
        }
    }
    // A listed event's entry names an event resource of a listed
    // interface (v2 §2.7).
    for entry in face.events.iter().flatten() {
        let known = listed.iter().any(|i| {
            resources_of(i)
                .iter()
                .any(|r| r.token == KindToken::Events && &res_name(i, r) == entry)
        });
        if !known {
            errors.push(Finding::new(Code::EventUnknown, Some(name), entry.clone()));
        }
    }
    for iface in listed {
        let (resources, disagree) = &view[*iface];
        for (i, r) in resources.iter().enumerate() {
            match crossing_of(iface, r).reason {
                Reason::Templated => findings.push(Finding::new(
                    Code::TemplatedExposure,
                    Some(name),
                    res_name(iface, r),
                )),
                Reason::Timed => findings.push(Finding::new(
                    Code::TimedDownsampled,
                    Some(name),
                    res_name(iface, r),
                )),
                // An unlisted frame is warned of (v2 §2.7); an unlisted
                // event is not.
                Reason::Unlisted if r.frame.is_some() => {
                    facts.unlisted.insert(res_name(iface, r));
                }
                _ => {}
            }
            if face.expose == Expose::Link && disagree.contains(&i) {
                findings.push(Finding::new(
                    Code::ExposureDisagrees,
                    Some(name),
                    res_name(iface, r),
                ));
            }
        }
    }

    // `zk` whole, or a device face's carve (v2 §2.6), then `adv` and
    // `admin`.
    let mut deny = if face.attach.is_device() {
        zk_carve()
    } else {
        vec![plane("zk", ZK_DENY)]
    };
    deny.extend([plane("adv", ADV_DENY), plane("admin", ADMIN_DENY)]);
    // Each template of a device's interface that declares `epoch`, on the
    // device's own face (v2 §2.5).
    for s in services.iter().filter(|s| s.device_face == Some(name)) {
        for iface in &deployment.implements[s.key] {
            for r in resources_of(iface).iter().filter(|r| r.epoch.is_some()) {
                findings.push(Finding::new(
                    Code::EpochOnDevice,
                    Some(name),
                    res_name(iface, r),
                ));
            }
        }
    }
    // The name (§0) of every overlapped resource.
    let mut overlapped: BTreeSet<String> = BTreeSet::new();
    // Each token's denies, computed with every stream last: a heartbeat
    // crosses where a collection it covers crosses, overlapped ones set
    // aside (§2.9), so the state tokens are carved first.
    let mut by_token: BTreeMap<usize, Vec<Deny>> = BTreeMap::new();
    let mut heartbeat_crosses: BTreeMap<String, bool> = BTreeMap::new();
    let order = [2usize, 3, 4, 0, 1];
    for t in order {
        let token = PUSH_TOKENS[t];
        if token == KindToken::Stream {
            heartbeat_crosses = heartbeats_of(deployment, listed, view, &|i: &str, r: &Res| {
                crossing_of(i, r).crosses() && !overlapped.contains(&res_name(i, r))
            });
        }
        // The listed interfaces' templates of the token, and, for
        // streams, each covered interface's heartbeat as a template of
        // `freshness.v2` of its own, in place of the profile's own
        // templated one (§2.9).
        // Each template with its own heartbeat's crossing, paired as one:
        // an interface id is a plain chunk, so every one parses, and one
        // that did not would drop out whole, never shift a pairing.
        let pseudo: Vec<(Template, bool)> = if token == KindToken::Stream {
            heartbeat_crosses
                .iter()
                .filter_map(|(i, crosses)| {
                    Template::parse(&format!("heartbeat/{i}"))
                        .ok()
                        .map(|t| (t, *crosses))
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut groups: BTreeMap<String, Vec<Item<'_>>> = BTreeMap::new();
        for iface in listed {
            if !pseudo.is_empty() && *iface == freshness::PROFILE_V2 {
                continue;
            }
            groups.insert(
                (*iface).to_owned(),
                resources_of(iface)
                    .iter()
                    .filter(|r| r.token == token)
                    .map(|r| Item {
                        template: &r.template,
                        crosses: crossing_of(iface, r).crosses(),
                    })
                    .collect(),
            );
        }
        if !pseudo.is_empty() {
            groups.insert(
                freshness::PROFILE_V2.to_owned(),
                pseudo
                    .iter()
                    .map(|(template, crosses)| Item {
                        template,
                        crosses: *crosses,
                    })
                    .collect(),
            );
        }
        let any = groups.values().flatten().any(|i| i.crosses);
        if !any {
            by_token.insert(
                t,
                vec![push_deny(
                    token.as_str().to_owned(),
                    vec![format!("zk2/*/*/*/{}/**", token.as_str())],
                )],
            );
            continue;
        }
        let mut denies = Vec::new();
        for (iface, items) in &groups {
            let root = format!("zk2/*/*/{iface}/{}", token.as_str());
            let keys = if items.iter().any(|i| i.crosses) {
                let (keys, exact) = carve(&root, items);
                for c in items.iter().filter(|i| i.crosses) {
                    let hit = exact.iter().any(|&t| {
                        let t = items[t].template;
                        t.overlaps(c.template) && c.template.precedence(t) == Ordering::Greater
                    });
                    if hit {
                        let at = format!("{iface}/{}/{}", token.as_str(), c.template.as_str());
                        overlapped.insert(at.clone());
                        findings.push(Finding::new(Code::CarveOutOverlap, Some(name), at));
                    }
                }
                keys
            } else {
                vec![format!("{root}/**")]
            };
            if !keys.is_empty() {
                denies.push(push_deny(format!("{}:{iface}", token.as_str()), keys));
            }
        }
        by_token.insert(t, denies);
    }
    deny.extend(by_token.into_values().flatten());

    // The resources that cross, overlapped ones set aside.
    let crosses = |iface: &str, r: &Res| {
        crossing_of(iface, r).crosses() && !overlapped.contains(&res_name(iface, r))
    };

    // Downsampling (§2.8), the interval against the horizon (§2.12). A
    // device's stream is never downsampled on its own face, where its puts
    // arrive (v2 §2.8).
    let mut rules = Vec::new();
    if let Some(freq) = face.freq() {
        for s in services.iter().filter(|s| s.device_face != Some(name)) {
            for iface in &deployment.implements[s.key] {
                for r in resources_of(iface) {
                    // A frame is never downsampled (§2.8).
                    if is_stream(r.token) && r.frame.is_none() && crosses(iface, r) {
                        rules.push(Rule {
                            key_expr: format!(
                                "zk2/{}/{iface}/{}/{}",
                                s.address,
                                r.token.as_str(),
                                r.template.as_str()
                            ),
                            freq,
                        });
                    }
                }
            }
        }
        rules.sort_by(|a, b| a.key_expr.cmp(&b.key_expr));
        rules.dedup_by(|a, b| a.key_expr == b.key_expr);
        for iface in listed {
            for r in resources_of(iface) {
                if r.frame.is_some() {
                    continue;
                }
                if let (true, Some(ttl), Some(n)) = (
                    is_stream(r.token) && crosses(iface, r),
                    horizon(r).ttl(),
                    face.interval_s,
                ) && u128::from(n) * 2 > u128::from(ttl.as_secs())
                {
                    findings.push(Finding::new(
                        Code::IntervalOverHorizon,
                        Some(name),
                        res_name(iface, r),
                    ));
                }
            }
        }
    }
    // A crossing frame resource (§2.8, §2.12): its own cadence, against
    // the face's interval, its batch and its horizon, and its history. A
    // listed frame's cadence is its entry's `every_s` (v2 §2.8).
    let mut listed_frames = Vec::new();
    let mut cadences: Vec<u64> = Vec::new();
    for iface in listed {
        for r in resources_of(iface) {
            let Some(f) = r.frame.filter(|_| crosses(iface, r)) else {
                continue;
            };
            let at = || res_name(iface, r);
            let entry = face.frames.as_ref().and_then(|l| l.get(&at())).copied();
            let cadence_ms = entry.map_or(f.interval_ms, |e| Some(e.every_s * 1000));
            cadences.extend(cadence_ms);
            if let Some(e) = entry {
                listed_frames.push(ListedFrame {
                    resource: at(),
                    cadence: e,
                });
                // The owner may write full records more often than its contract says,
                // never less (v2 §2.7).
                if let Some(k) = e.keyframe_s
                    && u128::from(k) > u128::from(f.keyframe_every) * u128::from(e.every_s)
                {
                    errors.push(Finding::new(Code::KeyframeUnreachable, Some(name), at()));
                }
            }
            if let (Some(i), Some(n)) = (cadence_ms, face.interval_s)
                && u128::from(i) < u128::from(n) * 1000
            {
                findings.push(Finding::new(Code::FrameOverInterval, Some(name), at()));
            }
            if f.largest_bytes.is_some_and(|b| b > face.batch_bytes) {
                findings.push(Finding::new(Code::FrameOverBatch, Some(name), at()));
            }
            if r.history {
                findings.push(Finding::new(Code::HistoryOnFrame, Some(name), at()));
            }
            if let (Some(ttl), Some(i)) = (horizon(r).ttl(), cadence_ms)
                && u128::from(i) * 2 > ttl.as_millis()
            {
                findings.push(Finding::new(Code::IntervalOverHorizon, Some(name), at()));
            }
            if let (true, Some(ttl), Some(h)) = (v2, horizon(r).ttl(), face.horizon_s)
                && ttl.as_secs() < h
            {
                findings.push(Finding::new(Code::HorizonUnderFace, Some(name), at()));
            }
        }
    }
    // What crosses, by name: a tool's scope across the face (observer
    // §7.5).
    let mut crossing: BTreeSet<String> = listed
        .iter()
        .flat_map(|iface| {
            resources_of(iface)
                .iter()
                .filter(|r| crosses(iface, r))
                .map(|r| res_name(iface, r))
        })
        .collect();
    crossing.extend(
        heartbeat_crosses
            .iter()
            .filter(|(_, c)| **c)
            .map(|(i, _)| heartbeat_name(i)),
    );
    // What else link.v2 weighs of each crossing resource (v2 §2.3, §2.16).
    if v2 {
        for iface in listed {
            for r in resources_of(iface).iter().filter(|r| crosses(iface, r)) {
                facts.crossing.insert(res_name(iface, r));
                facts.ifaces.insert((*iface).to_owned());
                if !r.packed {
                    findings.push(Finding::new(
                        Code::NotPacked,
                        Some(name),
                        res_name(iface, r),
                    ));
                }
                if r.block && r.longest_bytes.is_some_and(|b| b > face.batch_bytes) {
                    findings.push(Finding::new(
                        Code::BlockOverBatch,
                        Some(name),
                        res_name(iface, r),
                    ));
                }
            }
        }
        // A crossing heartbeat: its horizon, the shortest of its
        // collections', and its period, half that (`freshness.v2` §2.4).
        for (iface, crosses) in &heartbeat_crosses {
            if !crosses {
                continue;
            }
            facts.ifaces.insert(iface.clone());
            let Some(ttl) = heartbeat_horizon(view, iface) else {
                continue;
            };
            cadences.push(ttl.as_secs() * 500);
            if face.horizon_s.is_some_and(|h| ttl.as_secs() < h) {
                findings.push(Finding::new(
                    Code::HorizonUnderFace,
                    Some(name),
                    heartbeat_name(iface),
                ));
            }
        }
        // Data more often than the keep-alive interval spares the uplink
        // its keep-alives (v2 §2.16).
        if let (Some(t), Some(c)) = (&policy.transport, cadences.iter().min())
            && u128::from(*c) * u128::from(t.keep_alive) > u128::from(t.lease_ms)
        {
            findings.push(Finding::new(
                Code::KeepAliveNotSuppressed,
                Some(name),
                TransportKey::KeepAlive.at(),
            ));
        }
    }
    listed_frames.sort_by(|a, b| a.resource.cmp(&b.resource));
    let downsampling = (!rules.is_empty()).then(|| Downsampling {
        id: name.to_owned(),
        link_protocols: face.link_protocols.clone(),
        flows: vec!["egress"],
        messages: vec!["put"],
        rules,
    });

    // Windows (§2.11) and liveness (§2.12).
    let mut windows = Vec::new();
    let mut liveness = Vec::new();
    for s in services {
        let ifaces = &deployment.implements[s.key];
        let resources = || {
            ifaces
                .iter()
                .flat_map(|i| resources_of(i).iter().map(move |r| (i, r)))
        };
        // v2 §2.11 (0.9): a link.v2 policy floors only an owner of state
        // whose deletes only a tombstone proves; link.v1 every owner of
        // state.
        if resources().any(|(_, r)| is_state(r.token) && (!v2 || r.tombstone_only)) {
            windows.push(Window {
                service: s.address.clone(),
                min_s: face.max_outage_s,
            });
        }
        let mut evidence: Vec<String> = resources()
            .filter(|(i, r)| crosses(i, r) && horizon(r).ttl().is_some())
            .map(|(i, r)| res_name(i, r))
            .collect();
        // A crossing heartbeat is evidence too (§2.12): its horizon is the
        // shortest of the collections it covers (`freshness.v2` §2.4).
        evidence.extend(
            ifaces
                .iter()
                .filter(|i| heartbeat_crosses.get(i.as_str()) == Some(&true))
                .map(|i| heartbeat_name(i)),
        );
        evidence.sort();
        evidence.dedup();
        liveness.push(Liveness {
            service: s.address.clone(),
            evidence,
        });
    }

    sort_findings(&mut findings);
    let plan = FacePlan {
        name: name.to_owned(),
        far: face.far.clone(),
        attach: match &face.attach {
            Attach::Client => AttachPlan::Client,
            Attach::SouthRegion { region } => AttachPlan::SouthRegion {
                region: region.clone(),
                gateway: gateway(region),
            },
            Attach::Device { devices } => AttachPlan::Device {
                devices: devices.clone(),
            },
        },
        deny,
        downsampling,
        windows,
        liveness,
        reply_timeout_ms: face.reply_timeout_ms,
        frames: face.frames.as_ref().map(|_| listed_frames),
        findings,
        crossing,
    };
    (plan, facts)
}

fn horizon(r: &Res) -> Horizon {
    freshness::horizon(kind_of(r.token), &r.annotations)
}

/// A heartbeat's horizon: the shortest of the collections of `iface` it
/// covers (`freshness.v2` §2.9).
fn heartbeat_horizon(view: &View, iface: &str) -> Option<std::time::Duration> {
    view.get(iface)?
        .0
        .iter()
        .filter(|r| is_state(r.token))
        .filter_map(|r| horizon(r).ttl())
        .min()
}

/// Each listed interface whose owners send a heartbeat covering it
/// (`freshness.v2` §2.4: a revision uses the profile, and it has a
/// collection, a state with a horizon above 0), and whether that heartbeat
/// crosses the face: where a collection it covers crosses (§2.9). Never
/// downsampled (§2.8): no rule is derived for it.
fn heartbeats_of(
    deployment: &Deployment,
    listed: &BTreeSet<&str>,
    view: &View,
    crosses: &dyn Fn(&str, &Res) -> bool,
) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for iface in listed {
        if !deployment.heartbeats.contains(*iface) {
            continue;
        }
        let collections: Vec<&Res> = view[*iface]
            .0
            .iter()
            .filter(|r| is_state(r.token) && horizon(r).ttl().is_some())
            .collect();
        if collections.is_empty() {
            continue;
        }
        out.insert(
            (*iface).to_owned(),
            collections.iter().any(|r| crosses(iface, r)),
        );
    }
    out
}

fn plane(id: &str, key: &str) -> Deny {
    Deny {
        id: id.to_owned(),
        flows: FLOWS.to_vec(),
        messages: MESSAGES.to_vec(),
        key_exprs: vec![key.to_owned()],
    }
}

/// The pairs of core §11.2's Own row, `(flow, message)`: what a far
/// principal's Own grant lets cross a device face on an instance key (v2
/// §2.6). Ingress `put`, `delete`, `declare_queryable`, `reply` and
/// `liveliness_token`; egress `query` and `declare_subscriber`.
pub const OWN_PAIRS: [(&str, &str); 7] = [
    ("ingress", "put"),
    ("ingress", "delete"),
    ("ingress", "declare_queryable"),
    ("ingress", "reply"),
    ("ingress", "liveliness_token"),
    ("egress", "query"),
    ("egress", "declare_subscriber"),
];

/// A device face's `zk` keys (v2 §2.6): every `@zk` form of core §1.1 but
/// the instance subtree, denied on every pair.
pub const ZK_CARVED: [&str; 3] = [
    "zk2/*/*/@zk/alive/**",
    "zk2/*/*/@zk/member/**",
    "zk2/@zk/**",
];

/// The instance subtree a device face lets cross on [`OWN_PAIRS`] alone
/// (v2 §2.6).
pub const ZK_INSTANCE: &str = "zk2/*/*/@zk/instance/**";

/// The messages of `flow` outside core §11.2's Own row, in [`MESSAGES`]'
/// order.
fn outside_own(flow: &str) -> Vec<&'static str> {
    MESSAGES
        .iter()
        .copied()
        .filter(|m| !OWN_PAIRS.contains(&(flow, *m)))
        .collect()
}

/// A device face's three denies in place of `zk` (v2 §2.6): `zk` on the
/// interface tokens, member tokens and contract keys, every pair; and the
/// instance subtree on the eleven pairs outside the Own row, one deny per
/// flow. So an instance key crosses on the Own row's seven pairs alone,
/// and whose instance keys is the far principal's grants' to decide.
#[must_use]
pub fn zk_carve() -> Vec<Deny> {
    let mut out = vec![Deny {
        id: "zk".to_owned(),
        flows: FLOWS.to_vec(),
        messages: MESSAGES.to_vec(),
        key_exprs: ZK_CARVED.iter().map(|k| (*k).to_owned()).collect(),
    }];
    for flow in FLOWS {
        out.push(Deny {
            id: format!("zk:instance:{flow}"),
            flows: vec![flow],
            messages: outside_own(flow),
            key_exprs: vec![ZK_INSTANCE.to_owned()],
        });
    }
    out
}

fn push_deny(id: String, mut key_exprs: Vec<String>) -> Deny {
    key_exprs.sort();
    key_exprs.dedup();
    Deny {
        id,
        flows: FLOWS.to_vec(),
        messages: PUSH_MESSAGES.to_vec(),
        key_exprs,
    }
}

/// The near router's `gateway` block for a far router in `region` (§2.5):
/// its own peers and clients south first, then the region.
#[must_use]
pub fn gateway(region: &str) -> Gateway {
    Gateway {
        south: vec![
            South {
                filters: vec![Filter {
                    modes: vec!["peer".to_owned(), "client".to_owned()],
                    region_names: Vec::new(),
                }],
            },
            South {
                filters: vec![Filter {
                    modes: Vec::new(),
                    region_names: vec![region.to_owned()],
                }],
            },
        ],
    }
}

// ── The carve-out (§2.7) ─────────────────────────────────────────────────

struct Item<'a> {
    template: &'a Template,
    crosses: bool,
}

/// A segment's place in a pattern, and its group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Seg {
    Lit(String),
    Param,
    Rest,
}

impl Seg {
    fn of(s: &Segment) -> Self {
        match s {
            Segment::Literal(l) => Seg::Lit(l.clone()),
            Segment::Param(_) => Seg::Param,
            Segment::Rest(_) => Seg::Rest,
        }
    }

    fn kind(&self) -> u8 {
        match self {
            Seg::Lit(_) => 0,
            Seg::Param => 1,
            Seg::Rest => 2,
        }
    }

    /// A segment's pattern (§0): `{p}` is `*`, `{p...}` is `*/**`.
    fn pattern(&self) -> &str {
        match self {
            Seg::Lit(l) => l,
            Seg::Param => "*",
            Seg::Rest => "*/**",
        }
    }
}

/// `root`, the prefix's pattern, and `suffix`, with a `**` written once
/// before a `/**` that follows it.
fn key(root: &str, prefix: &[Seg], suffix: &str) -> String {
    let mut k = root.to_owned();
    for s in prefix {
        k.push('/');
        k.push_str(s.pattern());
    }
    k.push_str(suffix);
    while k.contains("/**/**") {
        k = k.replace("/**/**", "/**");
    }
    k
}

/// The carve-out of one interface's templates of one token (§2.7): the
/// denies, and the indices of the templates denied by their own pattern in
/// step 3.3.
fn carve(root: &str, items: &[Item<'_>]) -> (Vec<String>, Vec<usize>) {
    let mut keys = Vec::new();
    let mut exact = Vec::new();
    let all: Vec<usize> = (0..items.len()).collect();
    carve_at(root, items, &[], &all, &mut keys, &mut exact);
    keys.sort();
    keys.dedup();
    (keys, exact)
}

fn carve_at(
    root: &str,
    items: &[Item<'_>],
    prefix: &[Seg],
    here: &[usize],
    keys: &mut Vec<String>,
    exact: &mut Vec<usize>,
) {
    let d = prefix.len();
    let crosses = |i: &usize| items[*i].crosses;
    // 1. Nothing crosses: the subtree.
    if !here.iter().any(crosses) {
        keys.push(key(root, prefix, "/**"));
        return;
    }
    // 2. Everything crosses.
    if here.iter().all(crosses) {
        return;
    }
    let segs = |i: usize| items[i].template.segments();
    let (equal, longer): (Vec<usize>, Vec<usize>) = here.iter().partition(|&&i| segs(i).len() == d);
    // 3.1 The template equal to the prefix.
    if equal.iter().any(|i| !crosses(i)) {
        keys.push(key(root, prefix, ""));
    }
    // 3.2 Nothing longer crosses.
    if !longer.iter().any(crosses) {
        keys.push(key(root, prefix, "/*/**"));
        return;
    }
    // 3.3 More than one kind of next segment: template by template.
    let kinds: BTreeSet<u8> = longer
        .iter()
        .map(|&i| Seg::of(&segs(i)[d]).kind())
        .collect();
    if kinds.len() > 1 {
        for &i in &longer {
            if !items[i].crosses {
                let pattern: Vec<Seg> = segs(i).iter().map(Seg::of).collect();
                keys.push(key(root, &pattern, ""));
                exact.push(i);
            }
        }
        return;
    }
    // 3.4 Group by the next segment.
    let mut groups: BTreeMap<Seg, Vec<usize>> = BTreeMap::new();
    for &i in &longer {
        groups.entry(Seg::of(&segs(i)[d])).or_default().push(i);
    }
    for (seg, group) in groups {
        let mut p = prefix.to_vec();
        p.push(seg);
        carve_at(root, items, &p, &group, keys, exact);
    }
}

// ── Windows (§2.11) ──────────────────────────────────────────────────────

/// Each listed owner of state whose configured window (`windows`, by
/// address; [`DEFAULT_WINDOW_S`] when absent) is below a face's outage
/// (§2.11). Under a link.v2 policy, only an owner of state whose deletes
/// only a tombstone proves (v2 §2.11, [`Res::tombstone_only`]).
#[must_use]
pub fn check_windows(
    policy: &FacePolicy,
    deployment: &Deployment,
    windows: &BTreeMap<String, u64>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    // The listed services and the devices (v2 §2.5).
    for s in policy.owners() {
        let Some(ifaces) = deployment.implements.get(s.key) else {
            continue;
        };
        // v2 §2.11 (0.9): under link.v2, only state whose deletes only a
        // tombstone proves, in any revision given.
        let v2 = policy.profile == Profile::V2;
        let owns_state = ifaces.iter().any(|i| {
            deployment.interfaces.get(i).is_some_and(|revs| {
                revs.iter()
                    .flatten()
                    .any(|r| is_state(r.token) && (!v2 || r.tombstone_only))
            })
        });
        if !owns_state {
            continue;
        }
        let addr = s.address;
        let w = windows.get(&addr).copied().unwrap_or(DEFAULT_WINDOW_S);
        for (name, face) in &policy.faces {
            if w < face.max_outage_s {
                out.push(Finding::new(
                    Code::WindowUnderOutage,
                    Some(name),
                    addr.clone(),
                ));
            }
        }
    }
    sort_findings(&mut out);
    out
}

// ── A router's block (§2.15) ─────────────────────────────────────────────

/// One deny of a router's block, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockDeny {
    pub flows: Vec<String>,
    pub messages: Vec<String>,
    pub key_exprs: Vec<String>,
}

/// One downsampling rule of a router's block, as read.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockRule {
    pub key_expr: String,
    pub freq: f64,
}

/// zenoh's `default_permission`, as a router's block runs it (core §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Posture {
    Deny,
    Allow,
}

/// One subject of a router's block, as read (§2.4, §2.15): zenoh 1.10.1's
/// `AclConfigSubjects`. A session matches it when it matches every property
/// the subject names, and one value of each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockSubject {
    pub id: String,
    pub usernames: Vec<String>,
    pub cert_common_names: Vec<String>,
    /// `None` when the subject names no link protocol: it selects every one.
    /// An empty list, which zenoh 1.10.1 refuses to load (§2.15, 0.4), is
    /// read as `None`.
    pub link_protocols: Option<Vec<String>>,
    pub interfaces: Vec<String>,
    pub zids: Vec<String>,
}

impl BlockSubject {
    /// Whether it is bound to a principal: a user name or a CN.
    fn bound(&self) -> bool {
        !self.usernames.is_empty() || !self.cert_common_names.is_empty()
    }

    /// The link protocols it names; `None` for every one, an empty list
    /// included.
    fn protocols(&self) -> Option<&[String]> {
        self.link_protocols.as_deref().filter(|p| !p.is_empty())
    }

    /// Whether it selects the link protocol `p`.
    fn selects(&self, p: &str) -> bool {
        self.protocols().is_none_or(|l| l.iter().any(|x| x == p))
    }

    /// Whether it can match a session over `face` (§2.15): its link
    /// protocols are absent or name one of the face's. Interfaces and zids
    /// keep no session off a face: a zid is unauthenticated (core §11.3),
    /// and a face's link may report an interface.
    fn on_face(&self, face: &Face) -> bool {
        self.protocols()
            .is_none_or(|l| l.iter().any(|x| face.link_protocols.contains(x)))
    }

    /// Whether it is the far principal's (§2.15, 0.4, 0.5): it names `far`
    /// alone in one of its two lists, and nothing in the other. zenoh
    /// matches one value of each property a subject names, so a subject
    /// naming user names and CNs matches only a session presenting one of
    /// each (measured, 0.5, F-121), which the block cannot attribute to one
    /// principal.
    fn far_alone(&self, far: &str) -> bool {
        let only = |v: &[String]| !v.is_empty() && v.iter().all(|x| x == far);
        match (self.usernames.is_empty(), self.cert_common_names.is_empty()) {
            (false, true) => only(&self.usernames),
            (true, false) => only(&self.cert_common_names),
            _ => false,
        }
    }

    /// Whether it answers for the name `name` of the kind `kind` (§2.15,
    /// `narrowed_too_far`, 0.5): it names no interface and no zid, and
    /// either names no user name and no CN, or names `name` in the list of
    /// that kind and nothing in the other. A name no subject binds answers
    /// only to a subject bound to no principal.
    fn answers(&self, name: &str, kind: NameKind) -> bool {
        if !self.interfaces.is_empty() || !self.zids.is_empty() {
            return false;
        }
        if !self.bound() {
            return true;
        }
        let has = |v: &[String]| v.iter().any(|x| x == name);
        match kind {
            NameKind::User => self.cert_common_names.is_empty() && has(&self.usernames),
            NameKind::Cn => self.usernames.is_empty() && has(&self.cert_common_names),
            NameKind::Unbound => false,
        }
    }
}

/// The kind of a name `narrowed_too_far` reads (§2.15, 0.5): a user name
/// or a CN, as the list naming it says, or a face's `far` that no subject
/// names in either list, whose kind the block cannot tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NameKind {
    User,
    Cn,
    Unbound,
}

/// One policy of a router's block, as read (§2.15, 0.4): the ids of the
/// subjects it lists. Its rules are not read here: which policies list a
/// subject is what the subject checks need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockPolicy {
    pub subjects: Vec<String>,
}

/// A block's posture, every subject it defines and every policy's subject
/// list, when they were read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSubjects {
    pub posture: Posture,
    pub subjects: Vec<BlockSubject>,
    pub policies: Vec<BlockPolicy>,
}

impl BlockSubjects {
    /// The indices of the policies that list the subject `id`.
    fn listing(&self, id: &str) -> BTreeSet<usize> {
        self.policies
            .iter()
            .enumerate()
            .filter(|(_, p)| p.subjects.iter().any(|s| s == id))
            .map(|(i, _)| i)
            .collect()
    }
}

/// What a router runs for one face, as somebody read it (§2.15).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block {
    /// The deployment's namespace, empty for none (core §1.6).
    pub namespace: String,
    /// The block's `enabled`, as read: `Some(false)` when access control is
    /// not enabled, which a tool reading a router's configuration also reads
    /// for an `enabled` absent from it, as zenoh 1.10.1 does (§2.15, 0.7,
    /// F-124); `None` when it was not read.
    pub enabled: Option<bool>,
    /// The denies of every policy that lists one subject that can match
    /// the far principal's sessions on the face (§2.15): a tool checks each
    /// such subject's denies alone, since any allow wins across subjects.
    pub deny: Vec<BlockDeny>,
    /// In the block's order.
    pub downsampling: Vec<BlockRule>,
    /// The block's posture, subjects and policies' subject lists, which
    /// [`check_subjects`] and [`check_narrowing`] read; `None` when they
    /// were not read.
    pub subjects: Option<BlockSubjects>,
}

/// Checks a router's block for the face `face` (§2.15): `block_disabled`
/// when its access control is not enabled (0.7, F-124), beside every other
/// finding, which reads the block as written; `zk_not_denied`;
/// `wildcard_downsampling` and `intersecting_downsampling`. On a device
/// face (`device`, v2 §2.15) `zk_not_denied` reads §2.6's carve: a whole
/// `zk` passes, and so does the carve; on any other face the carve fails.
#[must_use]
pub fn check_block(face: &str, device: bool, block: &Block) -> Vec<Finding> {
    let mut out = Vec::new();
    if block.enabled == Some(false) {
        out.push(Finding::new(Code::BlockDisabled, Some(face), "enabled"));
    }
    let ns = block.namespace.trim_matches('/');
    let with_ns = |k: &str| {
        if ns.is_empty() {
            k.to_owned()
        } else {
            format!("{ns}/{k}")
        }
    };
    let denied = if device {
        ZK_CARVED
            .iter()
            .all(|k| zk_denied_on(&with_ns(k), &block.deny, |_, _| true))
            && zk_denied_on(&with_ns(ZK_INSTANCE), &block.deny, |f, m| {
                !OWN_PAIRS.contains(&(f, m))
            })
    } else {
        zk_denied_on(&with_ns(ZK_DENY), &block.deny, |_, _| true)
    };
    if !denied {
        out.push(Finding::new(Code::ZkNotDenied, Some(face), "@zk"));
    }
    let parsed: Vec<Option<&keyexpr>> = block
        .downsampling
        .iter()
        .map(|r| keyexpr::new(r.key_expr.as_str()).ok())
        .collect();
    for (j, r) in block.downsampling.iter().enumerate() {
        // `*`, `**` and `$*` each carry a `*` (zenoh-keyexpr's own test).
        if parsed[j].is_none_or(|k| k.as_str().contains('*')) {
            out.push(Finding::new(
                Code::WildcardDownsampling,
                Some(face),
                r.key_expr.clone(),
            ));
        }
        let hit = (0..j).any(|i| match (parsed[i], parsed[j]) {
            (Some(a), Some(b)) => a.intersects(b),
            _ => false,
        });
        if hit {
            out.push(Finding::new(
                Code::IntersectingDownsampling,
                Some(face),
                r.key_expr.clone(),
            ));
        }
    }
    sort_findings(&mut out);
    out
}

/// Whether every pair of the nine messages and both flows that `pairs`
/// holds is denied on a key expression that includes `target` (§2.15; v2
/// §2.15).
fn zk_denied_on(target: &str, deny: &[BlockDeny], pairs: impl Fn(&str, &str) -> bool) -> bool {
    let Ok(target) = keyexpr::new(target) else {
        return false;
    };
    FLOWS.iter().all(|flow| {
        MESSAGES.iter().filter(|m| pairs(flow, m)).all(|m| {
            deny.iter().any(|d| {
                d.flows.iter().any(|f| f == flow)
                    && d.messages.iter().any(|x| x == m)
                    && d.key_exprs
                        .iter()
                        .any(|k| keyexpr::new(k.as_str()).is_ok_and(|k| k.includes(target)))
            })
        })
    })
}

/// Checks a router's subjects against the face `name` they must keep to
/// its far principal (§2.4, §2.15):
/// - `near_subject_on_face`, each subject on the face bound by a user name
///   or a CN that is not the far principal's, whatever else it names
///   (0.4): zenoh ties no credential to a link, and matches one value of
///   each property, so a session over the face with another name the
///   subject binds matches it. The far principal's names it alone in one
///   list and nothing in the other (0.5): a subject naming user names and
///   CNs matches only a session presenting one of each, whose principal
///   the block cannot tell, and its policies are not the far principal's;
/// - `face_subject_beyond_face`, each face subject (on the face, bound by
///   no user name and no CN) whose link protocols are absent or name one
///   outside the face's: sessions off the face match it too (0.4);
/// - `face_subject_misplaced`, each face subject not listed in exactly the
///   policies that list a subject of the far principal's on the face, at
///   least one: it would grant every session over the face another
///   policy's rules, miss one of the far principal's denies, or, listed in
///   none, get the default (0.4);
/// - `face_subject_missing`, under `allow`, each link protocol of the face
///   that no face subject naming no interface and no zid names in its
///   `link_protocols`: a session over it that matches no other subject
///   gets the default, everything.
#[must_use]
pub fn check_subjects(name: &str, face: &Face, block: &BlockSubjects) -> Vec<Finding> {
    let mut out = Vec::new();
    let on_face: Vec<&BlockSubject> = block.subjects.iter().filter(|s| s.on_face(face)).collect();
    // The far principal's policies: those listing one of its subjects on
    // the face.
    let far_policies: BTreeSet<usize> = on_face
        .iter()
        .filter(|s| s.far_alone(&face.far))
        .flat_map(|s| block.listing(&s.id))
        .collect();
    for s in &on_face {
        let mut found = |code: Code| out.push(Finding::new(code, Some(name), s.id.clone()));
        if s.bound() {
            if !s.far_alone(&face.far) {
                found(Code::NearSubjectOnFace);
            }
            continue;
        }
        if s.protocols()
            .is_none_or(|l| l.iter().any(|x| !face.link_protocols.contains(x)))
        {
            found(Code::FaceSubjectBeyondFace);
        }
        let listed = block.listing(&s.id);
        if listed.is_empty() || listed != far_policies {
            found(Code::FaceSubjectMisplaced);
        }
    }
    if block.posture == Posture::Allow {
        for p in &face.link_protocols {
            let selected = block.subjects.iter().any(|s| {
                !s.bound()
                    && s.interfaces.is_empty()
                    && s.zids.is_empty()
                    && s.protocols().is_some_and(|l| l.contains(p))
            });
            if !selected {
                out.push(Finding::new(
                    Code::FaceSubjectMissing,
                    Some(name),
                    p.clone(),
                ));
            }
        }
    }
    sort_findings(&mut out);
    out
}

/// Checks a router's subjects against every face of `policy` at once
/// (§2.4, §2.15, 0.4, 0.5): under `allow`, `narrowed_too_far` (no face,
/// `at` the name) for each name the block binds, read by kind: each user
/// name a subject names, as a user name; each CN, as a CN; and each face's
/// far principal that no subject names in either list, of a kind the block
/// cannot tell. Such a name is reported with a link protocol of §2.1's
/// list, outside every face whose far principal it is not, that no subject
/// answering for it selects: one naming no interface and no zid, and either
/// naming no user name and no CN, or naming the name in the list of its
/// kind and nothing in the other (0.5, F-121). zenoh matches one value of
/// each property a subject names, so a session with that name alone over
/// that link protocol matches no subject, and is allowed everything (core
/// §11.3): a far principal with no subject at all among them. One finding
/// per name, whichever kinds fail. Under `deny`, such a session gets
/// nothing, and there is nothing to report.
#[must_use]
pub fn check_narrowing(policy: &FacePolicy, block: &BlockSubjects) -> Vec<Finding> {
    let mut out = Vec::new();
    if block.posture != Posture::Allow {
        return out;
    }
    let mut names: BTreeSet<(&str, NameKind)> = BTreeSet::new();
    for s in &block.subjects {
        names.extend(s.usernames.iter().map(|u| (u.as_str(), NameKind::User)));
        names.extend(
            s.cert_common_names
                .iter()
                .map(|c| (c.as_str(), NameKind::Cn)),
        );
    }
    for f in policy.faces.values() {
        // Named in either list, by any subject, interfaces and zids
        // included: it is then read by the kinds those lists give it.
        let bound = block.subjects.iter().any(|s| {
            s.usernames
                .iter()
                .chain(&s.cert_common_names)
                .any(|x| *x == f.far)
        });
        if !bound {
            names.insert((f.far.as_str(), NameKind::Unbound));
        }
    }
    let mut bare_names: BTreeSet<&str> = BTreeSet::new();
    for (name, kind) in names {
        let kept: Vec<String> =
            narrowed(policy, |f| policy.faces[f].far == name).unwrap_or_else(|| {
                policy
                    .binding
                    .protocols()
                    .iter()
                    .map(|p| (*p).to_owned())
                    .collect()
            });
        let bare = kept.iter().any(|p| {
            !block
                .subjects
                .iter()
                .any(|s| s.answers(name, kind) && s.selects(p))
        });
        if bare {
            bare_names.insert(name);
        }
    }
    for name in bare_names {
        out.push(Finding::new(Code::NarrowedTooFar, None, name));
    }
    sort_findings(&mut out);
    out
}

/// The link protocols a subject bound to a principal is selected by, on the
/// router of `policy` (§2.4): every one of the binding's closed list (v2
/// §2.4; §2.1's nine for a `link.v1` policy) outside the faces whose far
/// principal it is not (`is_far`, by the face's name), in the list's order.
/// `None` when no face excludes any: the subject names none. Empty when
/// every protocol is excluded: the principal could connect over none.
#[must_use]
pub fn narrowed(policy: &FacePolicy, is_far: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let excluded: BTreeSet<&str> = policy
        .faces
        .iter()
        .filter(|(name, _)| !is_far(name))
        .flat_map(|(_, f)| f.link_protocols.iter().map(String::as_str))
        .collect();
    (!excluded.is_empty()).then(|| {
        policy
            .binding
            .protocols()
            .iter()
            .filter(|p| !excluded.contains(*p))
            .map(|p| (*p).to_owned())
            .collect()
    })
}

// ── Lint (§2.1, §2.3, §2.11, §2.15) ──────────────────────────────────────

/// What a linter is given: the policy's text, and what else it has.
#[derive(Debug, Clone, Copy, Default)]
pub struct LintInput<'a> {
    pub text: &'a str,
    /// When given, the plan is derived, and its findings included.
    pub deployment: Option<&'a Deployment>,
    /// The windows the deployment configures, by address; needs the
    /// deployment.
    pub windows: Option<&'a BTreeMap<String, u64>>,
    /// A router's block, and the face it guards.
    pub block: Option<(&'a str, &'a Block)>,
}

/// Every finding of a face policy and of what comes with it, sorted. The
/// plan, the windows and the block are checked only when the policy has no
/// error.
#[must_use]
pub fn lint(input: &LintInput<'_>) -> Vec<Finding> {
    let parsed = parse(input.text);
    let mut out = parsed.findings;
    let Some(policy) = parsed.policy else {
        return out;
    };
    if let Some(d) = input.deployment {
        match plan(&policy, d) {
            Ok(plans) => out.extend(plans.into_iter().flat_map(|p| p.findings)),
            Err(f) => out.extend(f),
        }
        if let Some(w) = input.windows {
            out.extend(check_windows(&policy, d, w));
        }
    }
    if let Some((face, block)) = input.block {
        let device = policy.faces.get(face).is_some_and(|f| f.attach.is_device());
        out.extend(check_block(face, device, block));
        if let Some(s) = &block.subjects {
            if let Some(f) = policy.faces.get(face) {
                out.extend(check_subjects(face, f, s));
            }
            out.extend(check_narrowing(&policy, s));
        }
    }
    sort_findings(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ke(s: &str) -> &keyexpr {
        keyexpr::new(s).unwrap()
    }

    /// The zenoh facts the plan rests on (§0, §2.6, §2.7).
    #[test]
    fn key_expression_facts() {
        // `**` matches zero chunks: a subtree deny includes its prefix, and
        // `zk2/**/@zk/**` the contract keys.
        assert!(ke("zk2/*/*/m.v1/stream/a/**").includes(ke("zk2/h/s/m.v1/stream/a")));
        assert!(ke(ZK_DENY).includes(ke("zk2/@zk/contract/m.v1/abc")));
        assert!(ke(ZK_DENY).includes(ke("zk2/h/s/@zk/instance/0123456789abcdef")));
        assert!(ke("**/@zk/**").includes(ke(ZK_DENY)));
        // A rest parameter takes one or more chunks: `*/**` leaves its prefix.
        assert!(!ke("zk2/*/*/m.v1/events/logs/*/**").includes(ke("zk2/h/s/m.v1/events/logs")));
        assert!(ke("zk2/*/*/m.v1/events/logs/*/**").includes(ke("zk2/h/s/m.v1/events/logs/a/b")));
        // `*` never matches a verbatim chunk, so a wholesale deny spells it.
        assert!(!ke("zk2/*/*/*/stream/**").includes(ke("zk2/h/s/m.v1/@stream/x")));
        assert!(ke("zk2/*/*/*/@stream/**").includes(ke("zk2/h/s/m.v1/@stream/x")));
        assert!(ke(ADV_DENY).includes(ke("zk2/h/s/m.v1/stream/x/@adv/pub/abc")));
    }

    /// A device face's carve (v2 §2.6): an instance key crosses on the Own
    /// row's seven pairs and no other, and every other `@zk` key on none.
    #[test]
    fn the_carve_lets_an_instance_key_cross_on_the_own_pairs_alone() {
        let carve = zk_carve();
        let denied = |key: &str, flow: &str, m: &str| {
            carve.iter().any(|d| {
                d.flows.contains(&flow)
                    && d.messages.contains(&m)
                    && d.key_exprs.iter().any(|k| ke(k).includes(ke(key)))
            })
        };
        let instance = "zk2/lab/probe-a/@zk/instance/0123456789abcdef";
        let mut open = Vec::new();
        for flow in FLOWS {
            for m in MESSAGES {
                for key in [
                    "zk2/lab/probe-a/@zk/alive/probe.v1",
                    "zk2/lab/probe-a/@zk/member/0123456789abcdef/x",
                    "zk2/@zk/contract/probe.v1/abc",
                ] {
                    assert!(denied(key, flow, m), "{key} {flow} {m}");
                }
                if !denied(instance, flow, m) {
                    open.push((flow, m));
                }
            }
        }
        open.sort_unstable();
        let mut own = OWN_PAIRS.to_vec();
        own.sort_unstable();
        assert_eq!(open, own);
        // The carve names no address: whose instance keys is the grants'.
        assert!(
            carve
                .iter()
                .flat_map(|d| &d.key_exprs)
                .all(|k| k.starts_with("zk2/*/*/") || k == "zk2/@zk/**")
        );
    }

    #[test]
    fn keys_collapse_a_trailing_rest() {
        assert_eq!(
            key("r", &[Seg::Lit("x".into()), Seg::Rest], "/**"),
            "r/x/*/**"
        );
        assert_eq!(key("r", &[], "/**"), "r/**");
        assert_eq!(key("r", &[Seg::Lit("a".into())], "/*/**"), "r/a/*/**");
    }

    fn res(template: &str, annotations: Value) -> Res {
        Res::new(
            Template::parse(template).unwrap(),
            KindToken::Stream,
            serde_json::from_value(annotations).unwrap(),
        )
    }

    /// Two revisions read together (§2.3, 0.3): exposed only where every
    /// one that declares a resource exposes it, a horizon only where every
    /// one gives one, the least; a resource one revision adds is its own.
    #[test]
    fn revisions_are_read_together_failing_closed() {
        let a = vec![
            res(
                "x",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 10}),
            ),
            res(
                "y",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 10}),
            ),
            res("w", serde_json::json!({"link.exposure": "LINK"})),
        ];
        let b = vec![
            res("x", serde_json::json!({"freshness.ttl_s": 4})),
            res(
                "y",
                serde_json::json!({"link.exposure": "link", "freshness.ttl_s": 0}),
            ),
            res("w", serde_json::json!({})),
            res("z", serde_json::json!({"link.exposure": "link"})),
        ];
        let (m, disagree) = merge(&[a, b]);
        let read = |t: &str| {
            let r = m.iter().find(|r| r.template.as_str() == t).unwrap();
            (exposure_of_res(r), horizon(r).ttl().map(|d| d.as_secs()))
        };
        assert_eq!(read("x"), (Exposure::Host, Some(4)));
        assert_eq!(read("y"), (Exposure::Link, None));
        assert_eq!(read("z"), (Exposure::Link, None));
        // Two readings of host, one invalid: no disagreement to report.
        assert_eq!(disagree, BTreeSet::from([0]));
    }

    /// What link.v2 weighs, each revision judged alone, in either order
    /// (v2 §2.3, 0.6, F-469): one raw revision makes it not packed, one
    /// deadline timed, and the longest put is a `block` revision's.
    #[test]
    fn link_v2_judges_each_revision_alone() {
        let raw = res("s", serde_json::json!({"link.exposure": "link"}));
        let mut packed = raw.clone();
        packed.packed = true;
        packed.block = true;
        packed.longest_bytes = Some(300);
        let mut timed = raw.clone();
        timed.packed = true;
        timed.longest_bytes = Some(400);
        timed
            .annotations
            .insert(crate::timing::DEADLINE_MS.to_owned(), Value::from(2000));
        for revs in [
            [vec![raw.clone()], vec![packed.clone()], vec![timed.clone()]],
            [vec![timed.clone()], vec![packed.clone()], vec![raw.clone()]],
        ] {
            let (m, _) = merge(&revs);
            let r = &m[0];
            assert!(!r.packed, "one revision's raw type");
            assert!(r.shape().timed, "one revision's deadline");
            assert!(r.block);
            assert_eq!(r.longest_bytes, Some(300), "the block revision's");
        }
        let (m, _) = merge(&[vec![packed.clone()], vec![timed.clone()]]);
        assert!(m[0].packed, "packed in every revision");
        let (m, _) = merge(&[vec![timed.clone()], vec![raw]]);
        assert_eq!(m[0].longest_bytes, Some(400), "none block: the longest");
    }

    fn exposure_of_res(r: &Res) -> Exposure {
        match exposure(kind_of(r.token), &r.annotations) {
            Exposure::Invalid(_) => Exposure::Host,
            e => e,
        }
    }

    fn face(far: &str, protocols: &[&str]) -> Face {
        Face {
            far: far.to_owned(),
            attach: Attach::Client,
            link_protocols: protocols.iter().map(|p| (*p).to_owned()).collect(),
            expose: Expose::Link,
            interval_s: Some(1),
            max_outage_s: 1,
            batch_bytes: 1,
            link_bps: 1,
            lease_ms: DEFAULT_LEASE_MS,
            loss_ppm: None,
            arq: None,
            reply_timeout_ms: None,
            frames: None,
            events: None,
            horizon_s: None,
        }
    }

    /// A subject keeps the closed list outside every face whose far
    /// principal it is not (§2.4, 0.3).
    #[test]
    fn a_subject_is_narrowed_off_the_faces_it_is_not_far_of() {
        let policy = |faces: Vec<(&str, Face)>| FacePolicy {
            profile: Profile::V1,
            system: "lab".into(),
            services: vec![],
            faces: faces.into_iter().map(|(n, f)| (n.to_owned(), f)).collect(),
            binding: Binding::Zenoh1_10_1,
            transport: None,
            catalog: None,
        };
        let two = policy(vec![
            ("radio", face("ground", &["unixsock-stream"])),
            ("sat", face("sat", &["serial", "unixpipe"])),
        ]);
        let of = |p: &str| narrowed(&two, |name| two.faces[name].far == p);
        assert_eq!(
            of("local").unwrap(),
            ["tcp", "udp", "tls", "quic", "vsock", "ws"]
        );
        assert_eq!(
            of("ground").unwrap(),
            [
                "tcp",
                "udp",
                "tls",
                "quic",
                "unixsock-stream",
                "vsock",
                "ws"
            ]
        );
        let one = policy(vec![("radio", face("ground", &["unixsock-stream"]))]);
        assert_eq!(narrowed(&one, |_| true), None);
        let all = policy(vec![("x", face("x", Binding::Zenoh1_10_1.protocols()))]);
        assert_eq!(narrowed(&all, |_| false), Some(Vec::new()));
        // The fork's list keeps `unixsock-dgram`, after `unixsock-stream`
        // (v2 §2.4).
        let fork = FacePolicy {
            binding: Binding::Zm1_10_2R1,
            ..policy(vec![("radio", face("ground", &["unixsock-stream"]))])
        };
        assert_eq!(
            narrowed(&fork, |_| false).unwrap(),
            [
                "tcp",
                "udp",
                "tls",
                "quic",
                "serial",
                "unixpipe",
                "unixsock-dgram",
                "vsock",
                "ws"
            ]
        );
    }

    /// `keep_alive_flaps` over v2's Appendix B (v2 §2.16): at 5 % loss,
    /// 240 s with 4 keep-alives expects 0.36 false closes a day, 120 s with
    /// 2 expects 144; at no loss 2 never flaps, and 1 always does.
    #[test]
    fn the_keep_alive_lint_is_exact() {
        assert!(!flaps(240_000, 4, 50_000));
        assert!(!flaps(120_000, 4, 50_000));
        assert!(!flaps(1_080_000, 4, 50_000));
        assert!(flaps(120_000, 2, 50_000));
        assert!(flaps(60_000, 2, 50_000));
        assert!(!flaps(3_600_000, 2, 0));
        assert!(flaps(3_600_000, 1, 0));
        // 2 × 86,400 × 1,000 × 4 × 0.05^3 = lease_ms exactly: not above.
        assert!(!flaps(86_400, 4, 50_000));
        assert!(flaps(86_399, 4, 50_000));
        // The bounds, exactly: 255 keep-alives at 99.9999 % delivery.
        assert!(flaps(MAX_U32, MAX_KEEP_ALIVE, MAX_LOSS_PPM - 1));
        assert!(!flaps(MAX_U32, MAX_KEEP_ALIVE, 0));
    }

    /// A good face beside a `[transport]` that is not one never makes a
    /// policy (v2 §2.1): the face takes its batch and lease from it, and the
    /// error `[transport]` pushed refuses the whole.
    #[test]
    fn a_bad_transport_refuses_a_good_face() {
        let face = "[face.rf]\nfar = \"ground\"\nattach = \"client\"\n\
                    select = { link_protocols = [\"unixsock-stream\"] }\n\
                    class = \"constrained\"\nexpose = \"none\"\nmax_outage_s = 3600\n\
                    link_bps = 2400\nloss_ppm = 0\narq = true\nreply_timeout_ms = 15000\n";
        let head = "profile = \"link.v2\"\nsystem = \"lab\"\nservices = []\n\
                    binding = \"zenoh-1.10.1\"\n";
        let good = "[transport]\nlease_ms = 10000\nkeep_alive = 4\n\
                    wait_before_close_ms = 10000\nsequence = \"16bit\"\n";
        let ok = parse(&format!("{head}{good}{face}"));
        assert!(ok.policy.is_some(), "{:?}", ok.findings);
        for (transport, code, at) in [
            ("", Code::TransportMissing, "transport"),
            ("transport = 1\n", Code::BadValue, "transport"),
            (
                &good.replace("lease_ms = 10000", "lease_ms = 0"),
                Code::BadValue,
                "transport.lease_ms",
            ),
            (
                &good.replace("lease_ms = 10000\n", ""),
                Code::MissingField,
                "transport.lease_ms",
            ),
            (
                &format!("{good}batch_bytes = 65536\n"),
                Code::BadValue,
                "transport.batch_bytes",
            ),
        ] {
            let p = parse(&format!("{head}{transport}{face}"));
            assert!(p.policy.is_none(), "{transport:?}");
            assert!(
                p.findings
                    .iter()
                    .any(|f| f.code == code && f.at == at && f.face.is_none()),
                "{transport:?}: {:?}",
                p.findings
            );
        }
    }

    /// `wait_under_lease` reads both values as integers (v2 §2.19): a u64
    /// above i64::MAX and a negative i64 among them; a value of another type
    /// is `transport_differs`, which the check of every key reports.
    #[test]
    fn wait_under_lease_reads_integers_and_a_wrong_type_differs() {
        let p = parse(
            "profile = \"link.v2\"\nsystem = \"lab\"\nservices = []\n\
             binding = \"zenoh-1.10.1\"\n[transport]\nlease_ms = 10000\nkeep_alive = 4\n\
             wait_before_close_ms = 10000\nsequence = \"32bit\"\n[face.rf]\n\
             far = \"ground\"\nattach = \"client\"\n\
             select = { link_protocols = [\"unixsock-stream\"] }\nclass = \"constrained\"\n\
             expose = \"none\"\nmax_outage_s = 3600\nlink_bps = 1000000\nloss_ppm = 0\n\
             arq = true\nreply_timeout_ms = 15000\n",
        )
        .policy
        .expect("a policy");
        let wbc = TransportKey::WaitBeforeClose.path();
        let lease = TransportKey::Lease.path();
        let found = |pairs: &[(&str, Value)]| -> Vec<(&'static str, String)> {
            // A router that stamps nothing, as the generator writes it.
            let stated = StatedTransport(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .chain([("timestamping/enabled".to_owned(), Value::from(false))])
                    .collect(),
            );
            check_transport(&p, &stated, None)
                .into_iter()
                .map(|f| (f.code.as_str(), f.at))
                .collect()
        };
        let at = || "transport.wait_before_close_ms".to_owned();
        let differs = || ("transport_differs", at());
        let under = || ("wait_under_lease", at());
        assert_eq!(found(&[(wbc, Value::from(10_000_000u64))]), []);
        assert_eq!(found(&[(wbc, Value::from(u64::MAX))]), [differs()]);
        assert_eq!(found(&[(wbc, Value::from(-1i64))]), [differs(), under()]);
        assert_eq!(found(&[(wbc, Value::from("10000000"))]), [differs()]);
        assert_eq!(found(&[(wbc, Value::from(1.0e7))]), [differs()]);
        assert_eq!(
            found(&[
                (lease, Value::from("10000")),
                (wbc, Value::from(10_000_000u64))
            ]),
            [("transport_differs", "transport.lease_ms".to_owned())]
        );
    }

    /// A router's timestamping read at its mode (v2 §2.19, core §4.1):
    /// a boolean whatever the mode, a value per mode, or zenoh 1.10.1's
    /// default where neither is stated, on for a router only; no mode is
    /// zenohd's `router`.
    #[test]
    fn a_routers_timestamping_is_read_at_its_mode() {
        let stamps = |mode: Option<&str>, enabled: Option<Value>| {
            let mut m = BTreeMap::new();
            if let Some(mode) = mode {
                m.insert("mode".to_owned(), Value::from(mode));
            }
            if let Some(e) = enabled {
                m.insert("timestamping/enabled".to_owned(), e);
            }
            StatedTransport(m).stamps()
        };
        assert!(stamps(None, None), "zenohd's router stamps by default");
        assert!(stamps(Some("router"), None));
        assert!(!stamps(Some("peer"), None));
        assert!(!stamps(Some("client"), None));
        assert!(stamps(Some("peer"), Some(Value::from(true))));
        assert!(!stamps(Some("router"), Some(Value::from(false))));
        let off = serde_json::json!({"router": false, "peer": false});
        assert!(!stamps(None, Some(off.clone())));
        assert!(!stamps(Some("peer"), Some(off)));
        // A per-mode value that leaves the router's mode out is its default.
        assert!(stamps(
            Some("router"),
            Some(serde_json::json!({"peer": false}))
        ));
        assert!(stamps(
            Some("peer"),
            Some(serde_json::json!({"peer": true}))
        ));
        // The generator's posture, read back: a router that stamps nothing.
        let written = StatedTransport::read(&serde_json::json!({
            "mode": "router",
            "timestamping": {"enabled": {"router": false, "peer": false}},
        }));
        assert!(!written.stamps());
        assert_eq!(TIMESTAMPING_OFF.len(), 2);
    }

    /// The keep-alive interval in words: exact, or "about" where the lease
    /// is no multiple of `keep_alive`.
    #[test]
    fn the_keep_alive_interval_says_when_it_rounds() {
        let t = |lease_ms, keep_alive| Transport {
            lease_ms,
            keep_alive,
            ..Binding::Zenoh1_10_1.defaults()
        };
        assert_eq!(t(240_000, 4).keep_alive_interval_words(), "60000 ms");
        assert_eq!(t(2_000, 3).keep_alive_interval_words(), "about 666 ms");
    }

    /// The window and the handshake's air (v2 §2.16): RF's 324 full
    /// batches a lease fit 16 bits' window and not 8 bits'.
    #[test]
    fn the_sequence_window_and_the_handshake_are_integer_arithmetic() {
        assert!(!window_filled(Sequence::Bit16, 222, 240_000, 2_400));
        assert!(window_filled(Sequence::Bit8, 222, 240_000, 2_400));
        assert!(under_handshake(10_000, 293));
        assert!(!under_handshake(40_000, 293));
        assert!(!under_handshake(2_560, 2_400));
        assert!(under_handshake(2_559, 2_400));
        assert!(!reply_short(15_000, 2_400, 222));
        assert!(reply_short(11_000, 2_400, 222));
    }

    /// An empty `link_protocols`, which zenoh 1.10.1 refuses to load, is
    /// read as absent: the subject selects every link protocol (§2.15,
    /// 0.4).
    #[test]
    fn an_empty_link_protocol_list_reads_as_absent() {
        let radio = face("ground", &["unixsock-stream"]);
        let block = |links: Option<Vec<String>>| BlockSubjects {
            posture: Posture::Deny,
            subjects: vec![BlockSubject {
                id: "local".into(),
                usernames: vec!["local".into()],
                link_protocols: links,
                ..BlockSubject::default()
            }],
            policies: vec![BlockPolicy {
                subjects: vec!["local".into()],
            }],
        };
        let found = |links| check_subjects("radio", &radio, &block(links));
        assert_eq!(found(Some(Vec::new())), found(None));
        assert_eq!(found(None).len(), 1);
    }

    #[test]
    fn the_batch_against_the_lease_is_integer_arithmetic() {
        assert!(within_lease(1000, 2400, 10_000));
        assert!(!within_lease(1024, 2400, 10_000));
        assert!(within_lease(65_535, MAX_U32, MAX_U32));
    }

    /// The far side of a link.v2 face (v2 §2.20, §2.21): its reply timeout
    /// raises a shorter one, never a longer one; a face without ARQ
    /// re-declares; its session runs the eight `[transport]` keys; a
    /// `link.v1` policy and a face the policy does not name are refused.
    #[test]
    fn a_far_side_reads_its_face_of_a_link_v2_policy() {
        use std::time::Duration;
        let text = "profile = \"link.v2\"\nsystem = \"lab\"\nservices = []\n\
                    binding = \"zenoh-1.10.1\"\n\
                    [transport]\nlease_ms = 240000\nkeep_alive = 4\n\
                    wait_before_close_ms = 240000\nsequence = \"16bit\"\nbatch_bytes = 222\n\
                    [face.rf]\nfar = \"ground\"\nattach = \"client\"\n\
                    select = { link_protocols = [\"unixsock-stream\"] }\n\
                    class = \"constrained\"\nexpose = \"none\"\nmax_outage_s = 3600\n\
                    link_bps = 2400\nloss_ppm = 50000\narq = false\nreply_timeout_ms = 15000\n";
        let parsed = parse(text);
        let policy = parsed.policy.expect("a policy");
        let far = policy.far_side("rf").expect("its face");
        assert_eq!(far.reply_timeout(), Duration::from_secs(15));
        assert_eq!(
            far.at_least(Duration::from_secs(1)),
            Duration::from_secs(15)
        );
        assert_eq!(
            far.at_least(Duration::from_secs(30)),
            Duration::from_secs(30)
        );
        assert!(
            far.redeclares(),
            "no ARQ: a silent subscription is declared again"
        );
        assert_eq!(
            FarSide::silence(Duration::from_secs(300)),
            Duration::from_secs(600)
        );
        let keys = far.session_keys();
        assert_eq!(keys.len(), 8);
        assert_eq!(
            keys[2],
            (TransportKey::WaitBeforeClose, Value::from(240_000_000u64)),
            "wait_before_close in µs"
        );
        assert!(matches!(
            policy.far_side("sbd"),
            Err(FarSideRefusal::NoFace { faces, .. }) if faces == ["rf"]
        ));
        let v1 = FacePolicy {
            profile: Profile::V1,
            transport: None,
            ..policy
        };
        assert_eq!(v1.far_side("rf"), Err(FarSideRefusal::NotV2));
    }
}
