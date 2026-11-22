//! `health.v1` and `health.v2` in the runtime: a service says how well it
//! serves (`spec/profiles/health/v1.md`, #721; `v2.md`, #986's Q2).
//!
//! The owner's half of the profile. The levels, the codes and the reader's
//! judgement are `zenkey-model`'s ([`zenkey_model::health`]); this module
//! puts what the text asks of an owner, on the standard contract, embedded.
//! One handle, [`Health`], serves either major: [`ServiceBuilder::health`]
//! implements `health.v1`, the default, and [`ServiceBuilder::health_v2`]
//! `health.v2`, never both in one instance (`health.v2` §2.3). The table
//! below is `health.v1`'s; what `health.v2` changes follows it.
//!
//! ```text
//! ServiceBuilder::health() ─▶ Health: implements health.v1, declares the status writer,
//!        │                    exposes checks/{check}, declares the faults writer
//!        │  set_status / set_check / set_check_unknown / retire_check / fault
//!        ▼ start()
//!   the status put before step 3 when the caller has not (UNSPECIFIED, "starting")
//!   the clock-ahead watcher, when a clock reference is configured
//!        ▼
//!   Service::health() ─ the same handle
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.2 the status never better than its worst current check | every call puts the declared level raised to the worst check's ([`Effective`]) |
//! | §2.2 the order of the puts | a check worse than the published status: the status first; otherwise the check first, then the status if it moves |
//! | §2.3 the status before §8.2 step 4 | `start()` puts it before step 3, `UNSPECIFIED` with reason `"starting"` unless [`Health::set_status`] came first |
//! | §2.3 confirmed every 30 s, never re-put | the contract uses `freshness.v2`: the heartbeat covering `health.v1` ([`crate::heartbeat`]) vouches for the status every 28.5 s, from its `freshness.ttl_s = 60`, while [`Health::set_confirming`] leaves its bit set (`freshness.v2` §2.4) |
//! | §2.3 never deleted while the service runs | no call deletes it; closing the service leaves it |
//! | §2.3 checks on change, deleted when retired, at most 64 | [`Health::set_check`] puts only a change; [`Health::retire_check`] deletes; the 65th is refused, or past a lowered bound |
//! | §2.3 faults as occurrences | [`Health::fault`]: one sample each, on the stream, with the contract's QoS |
//! | §2.5 `clock_ahead` | on the guard's first hold, again every period of the heartbeat covering `health.v1` (28.5 s, [`crate::heartbeat::period`]) while it holds, none after it releases; the `detail` names the offset |
//! | §2.5 the status not held | the guard holds no call: the status and the checks carry no Zenoh timestamp (core §4.3, "Ahead"; #978's F6), and the heartbeat goes on vouching for the status, its time the ahead time while the guard holds (`freshness.v2` §2.10, core §7.5) |
//! | §2.6 `UNSPECIFIED` | the status only at start; a check only through [`Health::set_check_unknown`] |
//! | §2.10 fault codes | an application's code in the form, or refused; the profile's codes are the runtime's |
//!
//! **`health.v2`** ([`ServiceBuilder::health_v2`], [`v2`]) keeps every row
//! above but these:
//!
//! | Rule (`v2.md`) | Here |
//! |---|---|
//! | §3 packed records | a status and a check are 6 bytes and a fault 8 ([`v2::Status`], [`v2::Check`], [`v2::Fault`]), each sealed under its layout's tag by its writer; `since` and `at` are whole seconds on the owner's clock from the deployment epoch, absent where the epoch does not code them (§2.3) |
//! | §2.10 words | a reason, a detail and a code are given as text and written as a number by the profile's tables and the instance's own ([`Health::tables`]): no text is 0, a name no table holds is `out_of_range`, and its words go to the owner's log (`tracing`, `info`) |
//! | §2.10 an instance's table | the one table among the contracts the service implements, read again at each [`ServiceBuilder::implement`]; `start()` puts again a status or a check whose word a later contract moved |
//! | §2.2 the order of the puts, by seq | the same order as `health.v1`'s; the status and the checks share the owner's one state seq, and a GET's two snapshots are built at one asof, with no entry above it (`Store::answer`) |
//! | §2.2 a status raised to a check | its reason is the profile's `check` |
//! | §2.3 nothing put again unchanged | a status is put when its level or its word changes, a check likewise |
//! | §2.5 `clock_ahead`, once | one fault when the guard begins to hold, its `argument` the offset measured in whole seconds, rounded up; none again until the guard has released and holds anew. The heartbeat's ahead time answers the clock question |
//! | §2.5 no Zenoh timestamp | a fault is put through its writer as any stream sample: its `at` is in the record |
//! | §2.3 a fault's `argument` | [`Health::fault_argument`]; [`Health::fault`]'s `detail` stays in the owner's log |
//!
//! **An unknown status bounds nothing.** Before a level is declared, the
//! status stays `UNSPECIFIED` whatever the checks say: the owner has not
//! established its level, and §2.11 step 5 reads a bad check under an
//! unknown status as unhealthy without the owner claiming a level.
//!
//! **The contract is the published bundle**, revision 1.1
//! ([`FINGERPRINT`]), the one that uses `freshness.v2` (text 0.8, #978's
//! F4), embedded byte for byte from
//! `spec/profiles/.history/health.v1/` and served as it is. The path
//! reaches outside this crate's directory, which publishing it (#606) will
//! have to revisit: the bundle would be copied in, or the profile become a
//! crate of its own.
//!
//! **The payload types** are hand-written prost messages in [`v1`], named
//! as the proto package names them. A test holds them to the bundle's own
//! descriptor set: every field's number and type, and every enum value.
//!
//! **How a caller learns that its status was raised.** Every call that can
//! move the status returns the [`Effective`] status: the level published,
//! the level declared, and, when the first is worse, the check that raised
//! it ([`Effective::raised_by`]). [`Health::status`] reads it at any time.
//! The published reason says it too: `check disk is FAILED (declared OK:
//! serving)`.
//!
//! **Faults carry the owner's clock as their stamp**: [`Minter`]'s clock,
//! the session's `new_timestamp()`, the wall clock on a session without
//! timestamping, as an owner's runs (core §4.3, "Minting"; §8.2), offset
//! by [`Minter::simulate_offset`] in a test. A router
//! re-stamps a fault dated beyond its HLC delta, as a `clock_ahead` fault
//! from a clock ahead is, and drops it under
//! `timestamping.drop_future_timestamp` (§2.5): both are measured in
//! `tests/profile_health.rs`. So is §2.5's "a simulated offset is not a
//! drift" (text 0.2): a session whose HLC runs re-stamps a stamp that far
//! ahead itself, before the put leaves it, so a test's offset reaches a
//! router only from a session without an HLC.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use prost::Message as _;
use zenkey_model::contract::Resource;
use zenkey_model::grammar::{Addr, IfaceId, data_key};
use zenkey_model::health::{self as model, Code};
use zenkey_model::template::Bindings;
use zenoh::time::Timestamp;

pub use zenkey_model::health::{CLOCK_AHEAD, Level, Major, Read};

use crate::error::{Error, Result};
use crate::implementation::Implementation;
use crate::service::ServiceBuilder;
use crate::state::{HLC_DELTA, Minter, StateWriter, Store};
use crate::writer::{PackedType, Writer};

/// `health.v2`'s standard contract and its records
/// (`spec/profiles/health/v2.md` §3): what [`ServiceBuilder::health_v2`]
/// implements, and what a reader of `health.v2` is compiled against. The
/// records, the words and the tables are `zenkey-model`'s
/// ([`zenkey_model::health::v2`]), re-exported here.
pub mod v2 {
    use std::sync::LazyLock;

    use zenkey_model::grammar::IfaceId;
    pub use zenkey_model::health::v2::{
        Argument, CANNOT_RUN, CHECK, Check, Fault, Instance, InstanceTables, Kind, Named, Packed,
        STARTING, Status, Tables, Word, read_word, write_word,
    };

    use crate::implementation::Implementation;

    /// The standard contract's bundle, revision 2.0 as published in
    /// `spec/profiles/.history/health.v2/` (§3), byte for byte, read
    /// through `bundles/` so that the crate's package carries the bytes.
    pub const BUNDLE: &[u8] = include_bytes!(
        "../bundles/health.v2/b447f613bea7bdc24703500280a20bf0cf20fb27cbb38ecd25f5164b0ae005b1.bundle.json"
    );

    /// Its fingerprint (§3).
    pub const FINGERPRINT: &str = zenkey_model::health::v2::FINGERPRINT;

    static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
        Implementation::from_bundle(BUNDLE).expect("the published health.v2 bundle verifies")
    });

    /// The standard contract's interface id, `health.v2`.
    #[must_use]
    pub fn iface() -> IfaceId {
        zenkey_model::health::v2::IFACE
            .parse()
            .expect("health.v2 is an interface id")
    }

    /// The standard contract, implemented from the embedded bundle.
    #[must_use]
    pub fn implementation() -> Implementation {
        IMPLEMENTATION.clone()
    }
}

/// `health.v1`'s payload types (`spec/profiles/health/proto/health/v1/health.proto`),
/// as prost would generate them for the package `health.v1`.
pub mod v1 {
    use zenkey_model::health::{Level as Model, Read};

    /// `health.v1.Level`. The order is `Ok` < `Degraded` < `Failed`
    /// (§2.1); `Unspecified` is no level, and reads unknown (§2.6).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
    #[repr(i32)]
    pub enum Level {
        Unspecified = 0,
        Ok = 1,
        Degraded = 2,
        Failed = 3,
    }

    impl Level {
        /// The value's name in the proto file.
        #[must_use]
        pub fn as_str_name(&self) -> &'static str {
            match self {
                Self::Unspecified => "LEVEL_UNSPECIFIED",
                Self::Ok => "LEVEL_OK",
                Self::Degraded => "LEVEL_DEGRADED",
                Self::Failed => "LEVEL_FAILED",
            }
        }

        /// The value of a name in the proto file.
        #[must_use]
        pub fn from_str_name(name: &str) -> Option<Self> {
            Some(match name {
                "LEVEL_UNSPECIFIED" => Self::Unspecified,
                "LEVEL_OK" => Self::Ok,
                "LEVEL_DEGRADED" => Self::Degraded,
                "LEVEL_FAILED" => Self::Failed,
                _ => return None,
            })
        }
    }

    impl From<Model> for Level {
        fn from(l: Model) -> Self {
            match l {
                Model::Ok => Self::Ok,
                Model::Degraded => Self::Degraded,
                Model::Failed => Self::Failed,
            }
        }
    }

    /// `health.v1.Status`: the service's level (§2.2, §2.3).
    #[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
    pub struct Status {
        #[prost(enumeration = "Level", tag = "1")]
        pub level: i32,
        /// Why the level is what it is, for a person.
        #[prost(string, tag = "2")]
        pub reason: String,
        /// When the level last changed: nanoseconds since the Unix epoch,
        /// on the owner's clock. A re-put keeps it (§2.3).
        #[prost(uint64, tag = "3")]
        pub since_ns: u64,
    }

    /// `health.v1.Check`: one named part of the level (§2.2, §2.3).
    #[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
    pub struct Check {
        #[prost(enumeration = "Level", tag = "1")]
        pub level: i32,
        /// What the check found, for a person.
        #[prost(string, tag = "2")]
        pub detail: String,
        /// When the check found the value put, on the owner's clock.
        /// Informative: a check is not aged by it (§2.3).
        #[prost(uint64, tag = "3")]
        pub checked_at_ns: u64,
    }

    /// `health.v1.Fault`: an occurrence a level would lose (§2.3, §2.5).
    #[derive(Clone, PartialEq, Eq, Hash, prost::Message)]
    pub struct Fault {
        /// A fault code (§2.10).
        #[prost(string, tag = "1")]
        pub code: String,
        #[prost(enumeration = "Level", tag = "2")]
        pub level: i32,
        /// What happened, for a person.
        #[prost(string, tag = "3")]
        pub detail: String,
        /// When it happened: nanoseconds since the Unix epoch, on the
        /// owner's clock.
        #[prost(uint64, tag = "4")]
        pub at_ns: u64,
    }

    impl Status {
        /// The level as a reader reads it: a level, or unknown (§2.6).
        #[must_use]
        pub fn read(&self) -> Read {
            Read::from_wire(self.level)
        }
    }

    impl Check {
        /// The level as a reader reads it (§2.6).
        #[must_use]
        pub fn read(&self) -> Read {
            Read::from_wire(self.level)
        }
    }

    impl Fault {
        /// The level as a reader reads it (§2.6).
        #[must_use]
        pub fn read(&self) -> Read {
            Read::from_wire(self.level)
        }
    }
}

/// The standard contract's bundle, revision 1.1 as published in
/// `spec/profiles/.history/health.v1/` (§3), byte for byte: it uses
/// `freshness.v2`, so its owner's heartbeat confirms the status.
/// The crate reads it through `bundles/`, a link to that file, so its
/// package carries the bytes (#929).
pub const BUNDLE: &[u8] = include_bytes!(
    "../bundles/health.v1/60a4374513172609a9140051bb9b5bd9abc898ed6c22035c303414795940e42f.bundle.json"
);

/// Its fingerprint (§3).
pub const FINGERPRINT: &str = "60a4374513172609a9140051bb9b5bd9abc898ed6c22035c303414795940e42f";

/// The status, as the descriptor names it.
pub const STATUS: &str = "state/status";

/// The checks' template, as the descriptor names it.
pub const CHECKS: &str = "state/checks/{check}";

/// The faults, as the descriptor names it.
pub const FAULTS: &str = "stream/faults";

/// The reason of the status put at start (§2.3).
pub const STARTING: &str = "starting";

static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
    Implementation::from_bundle(BUNDLE).expect("the published health.v1 bundle verifies")
});

/// The standard contract's interface id, `health.v1`.
#[must_use]
pub fn iface() -> IfaceId {
    model::IFACE.parse().expect("health.v1 is an interface id")
}

/// The standard contract, implemented from the embedded bundle: what
/// [`ServiceBuilder::health`] implements, and what a reader of
/// `health.v1` is compiled against (R4).
#[must_use]
pub fn implementation() -> Implementation {
    IMPLEMENTATION.clone()
}

/// The status as published, against what the caller declared (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The level published; `None` for `UNSPECIFIED`, which is put at start
    /// until the caller declares a level (§2.3, §2.6).
    pub level: Option<Level>,
    /// The reason published. Under `health.v2` it is the text the caller
    /// gave, or the raising check's `check`, and [`Effective::word`] is
    /// the number it was written as.
    pub reason: String,
    /// Under `health.v2`, the reason's word as the record carries it
    /// (`health.v2` §2.10): `out_of_range` for a text no table names.
    /// `None` under `health.v1`.
    pub word: Option<v2::Word>,
    /// When the published level last changed, on the owner's clock (§2.3).
    pub since_ns: u64,
    /// The level the caller last declared through [`Health::set_status`].
    pub declared: Option<Level>,
    /// The check the status was raised to, when its level is worse than
    /// the declared one (§2.2): the first by name at the worst level.
    pub raised_by: Option<String>,
    /// Every current check, by name: its level, `None` for `UNSPECIFIED`.
    pub checks: BTreeMap<String, Option<Level>>,
}

impl Effective {
    /// Whether the published level is worse than the declared one.
    #[must_use]
    pub fn is_raised(&self) -> bool {
        self.raised_by.is_some()
    }
}

/// A service's `health.v1` or `health.v2`: the status, its checks and its
/// faults (§2.2, §2.3, §2.5), from [`ServiceBuilder::health`] or
/// [`ServiceBuilder::health_v2`], and again from [`crate::Service::health`]
/// once it runs. [`Health::major`] says which.
///
/// A clone is the same handle. Calls are serialized: each one finishes its
/// puts, in §2.2's order, before the next begins. The clock guard holds
/// none of them (§2.5): the status and the checks carry no Zenoh timestamp
/// (core §4.3), unless the session stamps every put (§8.2), when a call
/// that puts returns [`Error::ClockAhead`]. A call that fails after its
/// first put leaves that put standing, which keeps §2.2's rule;
/// [`Health::status`] says what is published, and the next call puts the
/// status the checks then call for.
#[derive(Clone)]
pub struct Health {
    inner: Arc<Inner>,
}

struct Inner {
    major: Major,
    /// `health.v2` §2.10: the instance's own tables, as the contracts the
    /// service implements hold them ([`Health::retable`]).
    tables: Mutex<v2::InstanceTables>,
    /// The checks' packed type under `health.v2` (core §7.6), which each
    /// check's writer seals and admits by.
    packed: Option<PackedType>,
    /// The deployment epoch a `health.v2` record's times count from, in
    /// seconds (core §7.6, §8.6).
    epoch: i64,
    iface: IfaceId,
    state: tokio::sync::Mutex<State>,
    /// What [`Health::status`] answers.
    seen: Mutex<Option<Effective>>,
    status: StateWriter,
    faults: Arc<Writer>,
    // What a check's writer is made of, while the service runs.
    session: zenoh::Session,
    addr: Addr,
    checks: Resource,
    store: Arc<Store>,
    minter: Arc<Minter>,
    closed: tokio::sync::watch::Receiver<bool>,
    max_checks: AtomicU64,
    clock: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.clock.get_mut().ok().and_then(Option::take) {
            t.abort();
        }
    }
}

#[derive(Default)]
struct State {
    /// What the caller declared, `None` before [`Health::set_status`].
    declared: Option<(Level, String)>,
    /// What is published, `None` before the first status put.
    published: Option<Published>,
    checks: BTreeMap<String, CheckState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Published {
    level: Option<Level>,
    reason: String,
    /// The reason's word under `health.v2`, `None` under `health.v1`.
    word: Option<v2::Word>,
    since_ns: u64,
    raised_by: Option<String>,
}

struct CheckState {
    level: Option<Level>,
    detail: String,
    /// The detail's word under `health.v2`, `None` under `health.v1`.
    word: Option<v2::Word>,
    writer: StateWriter,
}

/// The status §2.2 allows: its level, reason and raising check.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Aggregate {
    level: Option<Level>,
    reason: String,
    raised_by: Option<String>,
}

/// The status §2.2 calls for over `checks`: the declared level, raised to
/// the worst current check's where that is worse, the first by name among
/// equals. A check at an unknown level bounds nothing (§2.2). Before a
/// level is declared the status is `UNSPECIFIED`, and no check bounds it:
/// the owner has not established its level (§2.3), and a reader reads a
/// bad check under an unknown status as §2.11 step 5 says.
///
/// Under `health.v1` a raised status's reason says which check and what
/// was declared, in words. Under `health.v2` it is the profile's `check`:
/// the checks say which (`health.v2` §2.10).
fn aggregate(
    major: Major,
    declared: Option<&(Level, String)>,
    checks: &BTreeMap<&str, Option<Level>>,
) -> Aggregate {
    let Some((d, reason)) = declared else {
        return Aggregate {
            level: None,
            reason: STARTING.to_owned(),
            raised_by: None,
        };
    };
    let worst = checks.iter().filter_map(|(n, l)| l.map(|l| (l, *n))).fold(
        None,
        |best: Option<(Level, &str)>, (l, n)| match best {
            Some((b, _)) if b >= l => best,
            _ => Some((l, n)),
        },
    );
    match worst {
        Some((w, name)) if w > *d => Aggregate {
            level: Some(w),
            reason: match major {
                Major::V2 => v2::CHECK.to_owned(),
                Major::V1 if reason.is_empty() => format!("check {name} is {w} (declared {d})"),
                Major::V1 => format!("check {name} is {w} (declared {d}: {reason})"),
            },
            raised_by: Some(name.to_owned()),
        },
        _ => Aggregate {
            level: Some(*d),
            reason: reason.clone(),
            raised_by: None,
        },
    }
}

fn levels(checks: &BTreeMap<String, CheckState>) -> BTreeMap<&str, Option<Level>> {
    checks.iter().map(|(n, c)| (n.as_str(), c.level)).collect()
}

fn wire(level: Option<Level>) -> i32 {
    level.map_or(v1::Level::Unspecified as i32, Level::wire)
}

/// A stamp's time in nanoseconds since the Unix epoch.
fn ns(t: &Timestamp) -> u64 {
    let since = t
        .get_time()
        .to_system_time()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
}

/// One fault, stamped with the owner's clock (the module doc).
async fn publish_fault(
    w: &Writer,
    minter: &Minter,
    code: &str,
    level: Level,
    detail: String,
) -> Result<()> {
    let stamp = minter.now();
    let f = v1::Fault {
        code: code.to_owned(),
        level: level.wire(),
        detail,
        at_ns: ns(&stamp),
    };
    // health.v1's standard contract is never governed, so this admits it
    // always; every stamped put goes through its writer's gate all the same.
    // Core §4.3's table: the fault carries the Zenoh timestamp the owner
    // sets, beside its envelope.
    let admitted = w.serving().await?;
    w.put_at(&admitted, &f.encode_to_vec(), stamp).await
}

/// §2.5: a `clock_ahead` fault when the guard first holds, again every
/// `period` while it holds, none once it releases, until the service
/// closes. `period` is the heartbeat's covering `health.v1`.
async fn clock_ahead(
    faults: Arc<Writer>,
    minter: Arc<Minter>,
    period: Duration,
    mut closed: tokio::sync::watch::Receiver<bool>,
) {
    let mut held = minter.held();
    loop {
        tokio::select! {
            r = held.wait_for(Option::is_some) => if r.is_err() { return },
            _ = closed.wait_for(|c| *c) => return,
        }
        // Held: the first fault, then one per period until released.
        loop {
            let Some(offset) = *held.borrow_and_update() else {
                break;
            };
            let detail = format!(
                "the clock guard measured this owner {} ms ahead of its router, beyond the HLC \
                 delta of {} ms: its state writes that carry a Zenoh timestamp are held \
                 (core §4.3, health.v1 §2.5)",
                offset.as_millis(),
                HLC_DELTA.as_millis()
            );
            if let Err(e) =
                publish_fault(&faults, &minter, CLOCK_AHEAD, Level::Failed, detail).await
            {
                tracing::warn!("health.v1: the clock_ahead fault was not published: {e}");
            }
            tokio::select! {
                () = tokio::time::sleep(period) => {}
                r = held.changed() => if r.is_err() { return },
                _ = closed.wait_for(|c| *c) => return,
            }
        }
    }
}

/// A stamp's time as a `health.v2` record's `since` or `at` (§2.3): whole
/// seconds since the Unix epoch, absent where `epoch` does not code it.
fn seconds(ns: u64, epoch: i64) -> Option<i64> {
    model::v2::time_at(i128::from(ns / 1_000_000), epoch)
}

/// A `health.v2` record's bytes, as its writer seals them.
fn record<R: v2::Packed>(r: &R, epoch: i64) -> Result<Vec<u8>> {
    r.write(epoch)
        .map(|rec| rec.bytes)
        .map_err(|e| Error::Payload(format!("health.v2 {}: {e}", R::TYPE.name())))
}

/// One `health.v2` fault (§2.3, §2.5): a stream sample as any other, with
/// no Zenoh timestamp of the owner's, its `at` in the record.
async fn publish_fault_v2(w: &Writer, fault: &v2::Fault, epoch: i64) -> Result<()> {
    w.put(record(fault, epoch)?).await
}

/// `health.v2` §2.5: one `clock_ahead` fault when the guard begins to
/// hold, none again while it holds, and one more at each later hold, until
/// the service closes. The heartbeat's ahead time repeats the word each
/// period (`freshness.v2` §2.10).
async fn clock_ahead_once(
    faults: Arc<Writer>,
    minter: Arc<Minter>,
    epoch: i64,
    mut closed: tokio::sync::watch::Receiver<bool>,
) {
    let mut held = minter.held();
    loop {
        let offset = tokio::select! {
            r = held.wait_for(Option::is_some) => match r {
                Ok(offset) => *offset,
                Err(_) => return,
            },
            _ = closed.wait_for(|c| *c) => return,
        };
        let fault = v2::Fault::clock_ahead(offset, seconds(ns(&minter.now()), epoch));
        if let Err(e) = publish_fault_v2(&faults, &fault, epoch).await {
            tracing::warn!("health.v2: the clock_ahead fault was not published: {e}");
        }
        tokio::select! {
            r = held.wait_for(Option::is_none) => if r.is_err() { return },
            _ = closed.wait_for(|c| *c) => return,
        }
    }
}

impl Health {
    /// Implements `major`'s standard contract on `b`: the status's writer,
    /// the checks' template exposed, the faults' writer (§2.3).
    pub(crate) async fn declare(b: &mut ServiceBuilder, major: Major) -> Result<Self> {
        let addr = b.address()?.clone();
        let imp = match major {
            Major::V1 => implementation(),
            Major::V2 => v2::implementation(),
        };
        let iface = imp.iface().clone();
        let checks = imp.resource(CHECKS)?.clone();
        let epoch = b.deployment_epoch();
        let packed = PackedType::of(imp.contract(), &checks, epoch)?;
        b.implement(imp)?;
        let none = Bindings::new();
        let status = b.declare_state_writer(&iface, STATUS, &none).await?;
        b.expose(&iface, CHECKS)?;
        let faults = b.declare_writer(&iface, FAULTS, &none).await?;
        let (session, store, minter, closed) = b.state_parts();
        let h = Self {
            inner: Arc::new(Inner {
                major,
                tables: Mutex::new(b.health_tables()),
                packed,
                epoch,
                max_checks: AtomicU64::new(checks.cardinality.unwrap_or(model::MAX_CHECKS)),
                iface,
                state: tokio::sync::Mutex::new(State::default()),
                seen: Mutex::new(None),
                status,
                faults: Arc::new(faults),
                session,
                addr,
                checks,
                store,
                minter,
                closed,
                clock: Mutex::new(None),
            }),
        };
        b.health = Some(h.clone());
        Ok(h)
    }

    /// At start, before step 3: the status, `UNSPECIFIED` and `"starting"`,
    /// when nothing put one yet (§2.3). Under `health.v2`, a status or a
    /// check whose word the instance's tables now write otherwise is put
    /// again: a contract implemented after the call may have brought the
    /// table that names it, or one that differs, which leaves none
    /// (`health.v2` §2.10).
    pub(crate) async fn bring_up(&self) -> Result<()> {
        let mut st = self.inner.state.lock().await;
        if self.inner.major == Major::V2 {
            let moved: Vec<String> = st
                .checks
                .iter()
                .filter(|(_, c)| c.word != Some(self.word(v2::Kind::Detail, &c.detail, false)))
                .map(|(n, _)| n.clone())
                .collect();
            for name in moved {
                let c = &st.checks[&name];
                let (payload, word) = self.check_payload(c.level, &c.detail)?;
                c.writer.put(payload).await?;
                st.checks.get_mut(&name).expect("a current check").word = word;
            }
        }
        if self.inner.major == Major::V2 || st.published.is_none() {
            let next = self.aggregate(&st);
            self.put_status(&mut st, next).await?;
            self.remember(&st);
        }
        Ok(())
    }

    /// `health.v2` §2.10: the instance's own tables are `tables`, read
    /// from the contracts the service implements so far.
    pub(crate) fn retable(&self, tables: v2::InstanceTables) {
        *self.inner.tables.lock().expect("not poisoned") = tables;
    }

    /// The major this handle serves: `health.v1` from
    /// [`ServiceBuilder::health`], `health.v2` from
    /// [`ServiceBuilder::health_v2`].
    #[must_use]
    pub fn major(&self) -> Major {
        self.inner.major
    }

    /// The instance's own tables of words (`health.v2` §2.10): for each
    /// kind, the one table among the contracts the service implements, none
    /// where none declares one, and none where two differ. Under
    /// `health.v1` they play no part.
    #[must_use]
    pub fn tables(&self) -> v2::InstanceTables {
        self.inner.tables.lock().expect("not poisoned").clone()
    }

    /// The number a word of `kind` is written as (`health.v2` §2.10): 0
    /// for no text, the profile's number or the instance's own, and
    /// `out_of_range` for any other text, which then goes to the owner's
    /// log when `log`.
    fn word(&self, kind: v2::Kind, text: &str, log: bool) -> v2::Word {
        let tables = self.inner.tables.lock().expect("not poisoned");
        let word = v2::write_word(kind, (!text.is_empty()).then_some(text), tables.own(kind));
        if log && word.is_out_of_range() {
            tracing::info!(
                service = %self.inner.addr,
                table = kind.table(),
                words = text,
                "health.v2: no table names these words, so the record carries out_of_range \
                 (§2.10)"
            );
        }
        word
    }

    /// The status §2.2 calls for now, over what is declared and the
    /// current checks.
    fn aggregate(&self, st: &State) -> Aggregate {
        aggregate(self.inner.major, st.declared.as_ref(), &levels(&st.checks))
    }

    /// A check's payload, and under `health.v2` its detail's word.
    fn check_payload(
        &self,
        level: Option<Level>,
        detail: &str,
    ) -> Result<(Vec<u8>, Option<v2::Word>)> {
        let now = ns(&self.inner.minter.now());
        match self.inner.major {
            Major::V1 => Ok((
                v1::Check {
                    level: wire(level),
                    detail: detail.to_owned(),
                    checked_at_ns: now,
                }
                .encode_to_vec(),
                None,
            )),
            Major::V2 => {
                let word = self.word(v2::Kind::Detail, detail, true);
                let check = v2::Check {
                    level,
                    detail: word,
                    at: seconds(now, self.inner.epoch),
                };
                Ok((record(&check, self.inner.epoch)?, Some(word)))
            }
        }
    }

    /// Starts §2.5's watcher on the service's clock guard.
    ///
    /// The fault repeats at the period of the heartbeat covering
    /// `health.v1` (`freshness.v2` §2.4), not the status's own horizon:
    /// that heartbeat's time, the ahead time while the guard holds and an
    /// instant once it releases, is what a reader's "no" rests on (§5), so
    /// a reader that joined during the hold hears a fault within one
    /// period, as it hears that time. In revision
    /// 1.1 the status is the interface's one collection, and both are
    /// 28.5 s.
    pub(crate) fn watch_clock(&self, closed: tokio::sync::watch::Receiver<bool>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("health.v1: no tokio runtime here, so a clock ahead is not reported");
            return;
        };
        let faults = Arc::clone(&self.inner.faults);
        let minter = Arc::clone(&self.inner.minter);
        let task = match self.inner.major {
            Major::V1 => {
                let layout = zenkey_model::heartbeat::Layout::of(implementation().contract());
                let period = crate::heartbeat::period(&layout).unwrap_or(model::STATUS_REFRESH);
                rt.spawn(clock_ahead(faults, minter, period, closed))
            }
            // health.v2 §2.5: once per hold. The heartbeat says it each
            // period.
            Major::V2 => rt.spawn(clock_ahead_once(faults, minter, self.inner.epoch, closed)),
        };
        if let Some(old) = self.inner.clock.lock().expect("not poisoned").replace(task) {
            old.abort();
        }
    }

    /// The descriptor lowered the checks' bound (core §3.3).
    pub(crate) fn lower_checks(&self, n: u64) {
        self.inner.max_checks.store(n, Ordering::Relaxed);
    }

    /// The interface it implements: `health.v1`, or `health.v2`.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// The most checks this owner holds: the contract's 64, or the bound
    /// its descriptor lowers it to (§2.3).
    #[must_use]
    pub fn max_checks(&self) -> u64 {
        self.inner.max_checks.load(Ordering::Relaxed)
    }

    /// The status as published, with what was declared and why they differ;
    /// `None` until it is first put, at start at the latest.
    #[must_use]
    pub fn status(&self) -> Option<Effective> {
        self.inner.seen.lock().expect("not poisoned").clone()
    }

    /// Declares the service's level and why (§2.1). The status put is that
    /// level, raised to the worst current check's where that is worse
    /// (§2.2); it is put only when its level or reason changes, and
    /// `since_ns` moves only with its level (§2.3). A level is never
    /// `UNSPECIFIED` here: that is the start's alone (§2.6).
    ///
    /// Under `health.v2`, `reason` is a word (§2.10): a name of the
    /// profile's reasons (`serving`, `fallback`, …) or of the instance's
    /// own `health.reasons`, written as its number; no text is no word, and
    /// any other text is written `out_of_range` and logged. The status is
    /// put when its level or its word changes.
    pub async fn set_status(&self, level: Level, reason: impl Into<String>) -> Result<Effective> {
        let mut st = self.inner.state.lock().await;
        let declared = Some((level, reason.into()));
        let next = aggregate(self.inner.major, declared.as_ref(), &levels(&st.checks));
        self.put_status(&mut st, next).await?;
        st.declared = declared;
        Ok(self.remember(&st))
    }

    /// Puts the check `name` at `level` with `detail`, when either changed
    /// (§2.3), and moves the status as §2.2 asks: a check worse than the
    /// status puts the status first, any other change puts the check
    /// first, then the status if it improves. Refused past the bound on
    /// checks ([`Health::max_checks`]).
    ///
    /// Under `health.v2`, `detail` is a word (§2.10), as
    /// [`Health::set_status`]'s reason is: of the profile's details (`ok`,
    /// `timeout`, …) or of the instance's own `health.details`.
    pub async fn set_check(
        &self,
        name: &str,
        level: Level,
        detail: impl Into<String>,
    ) -> Result<Effective> {
        self.check(name, Some(level), detail.into()).await
    }

    /// Puts the check `name` as `UNSPECIFIED`, with `detail` saying why: a
    /// check the owner can no longer run, which it does not leave at its
    /// last level (§2.3, §2.6). It bounds the status no more (§2.2). Under
    /// `health.v2` the profile's word for it is [`v2::CANNOT_RUN`].
    pub async fn set_check_unknown(
        &self,
        name: &str,
        detail: impl Into<String>,
    ) -> Result<Effective> {
        self.check(name, None, detail.into()).await
    }

    /// Retires the check `name`: deletes it (core S3), then puts the status
    /// if it improves (§2.2, §2.3). A name that is not a current check
    /// changes nothing.
    pub async fn retire_check(&self, name: &str) -> Result<Effective> {
        let mut st = self.inner.state.lock().await;
        if let Some(c) = st.checks.get(name) {
            c.writer.delete().await?;
            st.checks.remove(name);
        }
        let next = self.aggregate(&st);
        let put = self.put_status(&mut st, next).await;
        let now = self.remember(&st);
        put.map(|()| now)
    }

    /// Publishes a fault (§2.3): an occurrence of severity `level`, with
    /// `detail` for a person, at the owner's clock now. It moves no status:
    /// an owner that the occurrence leaves worse off sets its status too.
    /// `code` is the application's (§2.10): of the form
    /// `[a-z][a-z0-9_]*`, prefixed with a name of its own by
    /// recommendation. A code of the profile's table is the runtime's to
    /// publish (`clock_ahead`, §2.5) and is refused, as is a malformed one.
    /// The guard does not hold a fault, which is no state.
    ///
    /// Under `health.v2` a fault carries no text: `code` is a word of the
    /// instance's own `health.codes`, `out_of_range` and logged where no
    /// table names it, the record's `argument` is `not_available`, and
    /// `detail` stays in the owner's log. [`Health::fault_argument`] gives
    /// the code its number.
    pub async fn fault(&self, code: &str, level: Level, detail: impl Into<String>) -> Result<()> {
        if self.inner.major == Major::V2 {
            let detail = detail.into();
            if !detail.is_empty() {
                tracing::info!(
                    service = %self.inner.addr,
                    code,
                    detail,
                    "health.v2: a fault carries no text, so its detail stays here (§2.3)"
                );
            }
            return self.fault_v2(code, level, v2::Argument::NotAvailable).await;
        }
        match model::code(code) {
            Code::Application(_) => {}
            Code::Profile(c) => {
                return Err(Error::Contract(format!(
                    "{c} is a code of health.v1's table, which the runtime publishes itself \
                     (health.v1 §2.5, §2.10)"
                )));
            }
            Code::Malformed(c) => {
                return Err(Error::Contract(format!(
                    "fault code {c:?} is not of the form [a-z][a-z0-9_]* (health.v1 §2.10)"
                )));
            }
        }
        publish_fault(
            &self.inner.faults,
            &self.inner.minter,
            code,
            level,
            detail.into(),
        )
        .await
    }

    /// Publishes a `health.v2` fault with its `argument` (§2.3): the
    /// code's own number, from 0 to 65,534, where the code's table says
    /// what it counts, or `None` where it takes none (`not_available`).
    /// `code` is written as [`Health::fault`] writes it. Refused under
    /// `health.v1`, whose fault carries a `detail` in words, and for
    /// 65,535, which is `not_available`'s code.
    pub async fn fault_argument(
        &self,
        code: &str,
        level: Level,
        argument: Option<u16>,
    ) -> Result<()> {
        if self.inner.major != Major::V2 {
            return Err(Error::Contract(
                "a health.v1 fault carries a detail in words, not an argument: publish it \
                 through Health::fault (health.v1 §2.3)"
                    .to_owned(),
            ));
        }
        let argument = match argument {
            None => v2::Argument::NotAvailable,
            Some(n) => v2::Argument::new(n).ok_or_else(|| {
                Error::Contract(format!(
                    "a fault's argument is 0 to {}: {n} is not_available's code (health.v2 §3)",
                    v2::Argument::MAX
                ))
            })?,
        };
        self.fault_v2(code, level, argument).await
    }

    /// One `health.v2` fault of the application's: a code of the profile's
    /// table is the runtime's to publish (`clock_ahead`, §2.5) and is
    /// refused; any other is written by the instance's own table.
    async fn fault_v2(&self, code: &str, level: Level, argument: v2::Argument) -> Result<()> {
        if v2::Kind::Code.profile().contains(&code) {
            return Err(Error::Contract(format!(
                "{code} is a code of health.v2's table, which the runtime publishes itself \
                 (health.v2 §2.5, §2.10)"
            )));
        }
        let i = &self.inner;
        let fault = v2::Fault {
            code: self.word(v2::Kind::Code, code, true),
            level: Some(level),
            argument,
            at: seconds(ns(&i.minter.now()), i.epoch),
        };
        publish_fault_v2(&i.faults, &fault, i.epoch).await
    }

    /// Turns the confirmation of the status off, or back on
    /// (`freshness.v2` §2.4, "only a value it holds current"): an owner
    /// that can no longer vouch for its level clears the status's bit in
    /// its heartbeat, and a reader judges it stale 60 s after its last
    /// confirmation (§2.4), rather than reading a level the owner cannot
    /// back as current. The status is never deleted (§2.3). Turned back
    /// on, the next heartbeat vouches for it again.
    pub fn set_confirming(&self, on: bool) {
        self.inner.status.set_vouched(on);
    }

    /// Whether the status is being confirmed: the contract's horizon
    /// confirms it, by the heartbeat under `freshness.v2`, and
    /// [`Health::set_confirming`] has not turned that off. A clock guard
    /// that holds leaves it so (§2.5).
    #[must_use]
    pub fn is_confirming(&self) -> bool {
        self.inner.status.confirm_period().is_some() && self.inner.status.is_vouched()
    }

    /// Whether the clock guard holds this owner's timed state writes (core
    /// §4.3, §2.5): `clock_ahead` is published, and its heartbeats' time is
    /// unknown. Its status is not held.
    #[must_use]
    pub fn is_clock_ahead(&self) -> bool {
        self.inner.minter.is_ahead()
    }

    /// The status's writer: zenoh stays reachable.
    #[must_use]
    pub fn status_writer(&self) -> &StateWriter {
        &self.inner.status
    }

    /// The faults' writer: zenoh stays reachable.
    #[must_use]
    pub fn faults_writer(&self) -> &Writer {
        &self.inner.faults
    }

    async fn check(&self, name: &str, level: Option<Level>, detail: String) -> Result<Effective> {
        let mut st = self.inner.state.lock().await;
        // health.v2 §2.3: a check is put when its level or its word
        // changes; its words are no part of its record.
        let word =
            (self.inner.major == Major::V2).then(|| self.word(v2::Kind::Detail, &detail, false));
        let unchanged = st.checks.get(name).is_some_and(|c| {
            c.level == level
                && match word {
                    Some(w) => c.word == Some(w),
                    None => c.detail == detail,
                }
        });
        if unchanged {
            // §2.3: a check is put on change only. The status may still owe
            // a put a failed call left behind.
            if let Some(c) = st.checks.get_mut(name) {
                c.detail = detail;
            }
            let next = self.aggregate(&st);
            let put = self.put_status(&mut st, next).await;
            let now = self.remember(&st);
            return put.map(|()| now);
        }
        let fresh = if st.checks.contains_key(name) {
            None
        } else {
            let max = self.max_checks();
            if st.checks.len() as u64 >= max {
                return Err(Error::Contract(format!(
                    "{} holds at most {max} checks (§2.3), so check {name:?} is refused",
                    self.inner.iface
                )));
            }
            Some(self.check_writer(name).await?)
        };
        let mut view = levels(&st.checks);
        view.insert(name, level);
        let next = aggregate(self.inner.major, st.declared.as_ref(), &view);
        let (payload, word) = self.check_payload(level, &detail)?;
        // §2.2: a check worse than the published status goes after the
        // status that bounds it; any other change goes first.
        let status_first = matches!(
            (level, st.published.as_ref().and_then(|p| p.level)),
            (Some(l), Some(s)) if l > s
        );
        if status_first {
            self.put_status(&mut st, next.clone()).await?;
        }
        let put = match &fresh {
            Some(w) => w.put(payload).await,
            None => st.checks[name].writer.put(payload).await,
        };
        if let Err(e) = put {
            self.remember(&st);
            return Err(e);
        }
        match fresh {
            Some(writer) => {
                st.checks.insert(
                    name.to_owned(),
                    CheckState {
                        level,
                        detail,
                        word,
                        writer,
                    },
                );
            }
            None => {
                let c = st.checks.get_mut(name).expect("a current check");
                c.level = level;
                c.detail = detail;
                c.word = word;
            }
        }
        let put = if status_first {
            Ok(())
        } else {
            self.put_status(&mut st, next).await
        };
        let now = self.remember(&st);
        put.map(|()| now)
    }

    /// Puts the status `next` when its level or reason differ from what is
    /// published; `since_ns` moves with the level only (§2.3).
    ///
    /// Under `health.v2` the record is what is compared: the status is put
    /// when its level or its reason's word changes, and a reason in other
    /// words under the same number is no change (`health.v2` §2.3).
    async fn put_status(&self, st: &mut State, next: Aggregate) -> Result<()> {
        let i = &self.inner;
        let word = (i.major == Major::V2).then(|| {
            // The words are logged when they are put, below.
            self.word(v2::Kind::Reason, &next.reason, false)
        });
        let since_ns = match &mut st.published {
            Some(p)
                if p.level == next.level
                    && match word {
                        Some(w) => p.word == Some(w),
                        None => p.reason == next.reason,
                    } =>
            {
                p.reason = next.reason;
                p.raised_by = next.raised_by;
                return Ok(());
            }
            Some(p) if p.level == next.level => p.since_ns,
            _ => ns(&i.minter.now()),
        };
        let payload = match i.major {
            Major::V1 => v1::Status {
                level: wire(next.level),
                reason: next.reason.clone(),
                since_ns,
            }
            .encode_to_vec(),
            Major::V2 => {
                let status = v2::Status {
                    level: next.level,
                    reason: self.word(v2::Kind::Reason, &next.reason, true),
                    since: seconds(since_ns, i.epoch),
                };
                record(&status, i.epoch)?
            }
        };
        i.status.put(payload).await?;
        st.published = Some(Published {
            level: next.level,
            reason: next.reason,
            word,
            since_ns,
            raised_by: next.raised_by,
        });
        Ok(())
    }

    /// A writer on the member `checks/<name>`, as the service's own state
    /// writers are made: stamped, answered by its state queryables (S1, S2).
    async fn check_writer(&self, name: &str) -> Result<StateWriter> {
        let i = &self.inner;
        let values: Bindings = [("check".to_owned(), vec![name.to_owned()])].into();
        let chunks = i
            .checks
            .template
            .build(&values)
            .map_err(|e| Error::Contract(format!("{} {CHECKS:?}: {e}", i.iface)))?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let key = data_key(&i.addr, &i.iface, i.checks.token, &refs)?;
        let w = Writer::declare(
            &i.session,
            &i.minter,
            key.into_keyexpr(),
            &i.checks,
            &values,
        )
        .await?
        .packed(i.packed.clone());
        Ok(StateWriter::new(
            w,
            Arc::clone(&i.store),
            Arc::clone(&i.minter),
            &zenkey_model::freshness::horizon_of(&i.checks),
            i.closed.clone(),
        ))
    }

    /// Records what is published for [`Health::status`], and returns it.
    fn remember(&self, st: &State) -> Effective {
        let now = st.published.as_ref().map(|p| Effective {
            level: p.level,
            reason: p.reason.clone(),
            word: p.word,
            since_ns: p.since_ns,
            declared: st.declared.as_ref().map(|(l, _)| *l),
            raised_by: p.raised_by.clone(),
            checks: st
                .checks
                .iter()
                .map(|(n, c)| (n.clone(), c.level))
                .collect(),
        });
        let mut seen = self.inner.seen.lock().expect("not poisoned");
        seen.clone_from(&now);
        now.unwrap_or_else(|| Effective {
            level: None,
            reason: String::new(),
            word: None,
            since_ns: 0,
            declared: st.declared.as_ref().map(|(l, _)| *l),
            raised_by: None,
            checks: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use prost::Message as _;
    use prost_types::field_descriptor_proto::Type;
    use serde_json::json;
    use zenkey_model::bundle::Bundle;
    use zenkey_model::schema::{ArtifactData, SchemaKind};

    use super::{BUNDLE, FINGERPRINT, Level, Major, aggregate, implementation, v1, v2};

    /// The hand-written types are the bundle's: the descriptor set it
    /// carries has exactly these fields, numbers and types, and these enum
    /// values; and a value the Rust types encode decodes through that
    /// descriptor set, field by field, as the same value.
    #[test]
    fn the_types_are_the_bundles() {
        let imp = implementation();
        assert_eq!(imp.fingerprint().hex().to_string(), FINGERPRINT);
        assert_eq!(&**imp.bundle_bytes(), BUNDLE, "served byte for byte");
        let art = imp
            .contract()
            .artifacts
            .values()
            .find(|a| a.kind == SchemaKind::Protobuf)
            .expect("a descriptor set");
        let ArtifactData::Protobuf(bytes) = &art.data else {
            panic!("protobuf data")
        };
        let set = prost_types::FileDescriptorSet::decode(bytes.as_slice()).unwrap();
        let file = set
            .file
            .iter()
            .find(|f| f.package() == "health.v1")
            .expect("the health.v1 file");

        // The enum, against the Rust enum and the model's levels.
        let level = file.enum_type.iter().find(|e| e.name() == "Level").unwrap();
        let values: Vec<(&str, i32)> = level.value.iter().map(|v| (v.name(), v.number())).collect();
        let rust: Vec<(&str, i32)> = [
            v1::Level::Unspecified,
            v1::Level::Ok,
            v1::Level::Degraded,
            v1::Level::Failed,
        ]
        .iter()
        .map(|l| (l.as_str_name(), *l as i32))
        .collect();
        assert_eq!(values, rust);
        for l in [Level::Ok, Level::Degraded, Level::Failed] {
            assert_eq!(v1::Level::from(l) as i32, l.wire());
            assert_eq!(
                v1::Level::from_str_name(v1::Level::from(l).as_str_name()),
                Some(v1::Level::from(l))
            );
        }

        // The messages: every field, its number, type and enum type.
        let fields = |name: &str| -> Vec<(String, i32, Type, String)> {
            let m = file
                .message_type
                .iter()
                .find(|m| m.name() == name)
                .unwrap_or_else(|| panic!("{name} is in the descriptor set"));
            m.field
                .iter()
                .map(|f| {
                    (
                        f.name().to_owned(),
                        f.number(),
                        f.r#type(),
                        f.type_name().to_owned(),
                    )
                })
                .collect()
        };
        let row = |n: &str, i: i32, t: Type, tn: &str| (n.to_owned(), i, t, tn.to_owned());
        let lvl = ".health.v1.Level";
        assert_eq!(
            fields("Status"),
            [
                row("level", 1, Type::Enum, lvl),
                row("reason", 2, Type::String, ""),
                row("since_ns", 3, Type::Uint64, ""),
            ]
        );
        assert_eq!(
            fields("Check"),
            [
                row("level", 1, Type::Enum, lvl),
                row("detail", 2, Type::String, ""),
                row("checked_at_ns", 3, Type::Uint64, ""),
            ]
        );
        assert_eq!(
            fields("Fault"),
            [
                row("code", 1, Type::String, ""),
                row("level", 2, Type::Enum, lvl),
                row("detail", 3, Type::String, ""),
                row("at_ns", 4, Type::Uint64, ""),
            ]
        );
        assert_eq!(file.message_type.len(), 3, "no other message");

        // Rust-encoded values, decoded through the bundle's descriptor set.
        let bundle = Bundle::verify(BUNDLE).unwrap();
        let through = |template: &str, token: &str, bytes: Vec<u8>| {
            let ty = zenkey_model::decode::type_of(&bundle, token, template, "type").unwrap();
            match zenkey_model::decode::decode(&bundle, ty, Some("application/protobuf"), &bytes) {
                zenkey_model::decode::Rendered::Value(v) => v,
                other => panic!("{other:?}"),
            }
        };
        let status = v1::Status {
            level: v1::Level::Failed as i32,
            reason: "r".into(),
            since_ns: 7,
        };
        assert_eq!(
            through("status", "state", status.encode_to_vec()),
            json!({"level": "LEVEL_FAILED", "reason": "r", "sinceNs": "7"})
        );
        let check = v1::Check {
            level: v1::Level::Degraded as i32,
            detail: "d".into(),
            checked_at_ns: 8,
        };
        assert_eq!(
            through("checks/{check}", "state", check.encode_to_vec()),
            json!({"level": "LEVEL_DEGRADED", "detail": "d", "checkedAtNs": "8"})
        );
        let fault = v1::Fault {
            code: "app_x".into(),
            level: v1::Level::Ok as i32,
            detail: "f".into(),
            at_ns: 9,
        };
        assert_eq!(
            through("faults", "stream", fault.encode_to_vec()),
            json!({"code": "app_x", "level": "LEVEL_OK", "detail": "f", "atNs": "9"})
        );
    }

    /// The standard contract carries the horizon the status's heartbeat
    /// confirms it within, under `freshness.v2` (text 0.8), and the
    /// checks' bound.
    #[test]
    fn the_contract_drives_the_refresh_and_the_bound() {
        let imp = implementation();
        assert!(zenkey_model::freshness::uses_v2(imp.contract()));
        let status = imp.resource(super::STATUS).unwrap();
        assert_eq!(
            zenkey_model::freshness::horizon_of(status).refresh_bound(),
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(imp.resource(super::CHECKS).unwrap().cardinality, Some(64));
        assert!(imp.resource(super::FAULTS).is_ok());
    }

    /// §2.2: the declared level, raised to the worst current check's, the
    /// first by name among equals; an unknown check bounds nothing; nothing
    /// declared is UNSPECIFIED, bounded by nothing.
    #[test]
    fn aggregation() {
        let declared = (Level::Ok, "serving".to_owned());
        let mut checks = BTreeMap::new();
        let a = aggregate(Major::V1, Some(&declared), &checks);
        assert_eq!(
            (a.level, a.reason.as_str(), a.raised_by),
            (Some(Level::Ok), "serving", None)
        );
        checks.insert("net", Some(Level::Failed));
        checks.insert("disk", Some(Level::Failed));
        checks.insert("cpu", None);
        let a = aggregate(Major::V1, Some(&declared), &checks);
        assert_eq!(a.level, Some(Level::Failed));
        assert_eq!(a.raised_by.as_deref(), Some("disk"));
        assert_eq!(a.reason, "check disk is FAILED (declared OK: serving)");
        let worse = (Level::Failed, "down".to_owned());
        let a = aggregate(Major::V1, Some(&worse), &checks);
        assert_eq!(
            (a.level, a.reason.as_str(), a.raised_by),
            (Some(Level::Failed), "down", None),
            "a status may be worse than its checks"
        );
        let a = aggregate(Major::V1, None, &checks);
        assert_eq!((a.level, a.reason.as_str()), (None, "starting"));
        checks.clear();
        checks.insert("cpu", None);
        assert_eq!(
            aggregate(Major::V1, Some(&declared), &checks).level,
            Some(Level::Ok)
        );
    }

    /// `health.v2`'s standard contract is the published bundle, served
    /// byte for byte: revision 2.0, its status confirmed by the heartbeat
    /// within its horizon, its checks bounded, and each resource's packed
    /// type the model's record under the tag v2.md §3 gives it.
    #[test]
    fn the_v2_contract_is_the_published_bundle() {
        use zenkey_model::health::v2::{Packed as _, RecordType};
        let imp = v2::implementation();
        assert_eq!(imp.fingerprint().hex().to_string(), v2::FINGERPRINT);
        assert_eq!(&**imp.bundle_bytes(), v2::BUNDLE, "served byte for byte");
        assert_eq!(imp.iface(), &v2::iface());
        assert!(zenkey_model::freshness::uses_v2(imp.contract()));
        let status = imp.resource(super::STATUS).unwrap();
        assert_eq!(
            zenkey_model::freshness::horizon_of(status).refresh_bound(),
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(imp.resource(super::CHECKS).unwrap().cardinality, Some(64));
        for (resource, ty) in [
            (super::STATUS, RecordType::Status),
            (super::CHECKS, RecordType::Check),
            (super::FAULTS, RecordType::Fault),
        ] {
            let r = imp.resource(resource).unwrap();
            let packed = super::PackedType::of(imp.contract(), r, 0)
                .unwrap()
                .unwrap_or_else(|| panic!("{resource}: a packed type"));
            assert_eq!(packed.tag().get(), ty.tag(), "{resource}");
        }
        // What the owner writes, the contract's own layout admits.
        let checks = imp.resource(super::CHECKS).unwrap();
        let packed = super::PackedType::of(imp.contract(), checks, 0)
            .unwrap()
            .unwrap();
        let check = v2::Check {
            level: Some(Level::Failed),
            detail: v2::write_word(v2::Kind::Detail, Some("timeout"), None),
            at: Some(1_794_062_759),
        };
        packed.admit(&super::record(&check, 0).unwrap()).unwrap();
        assert_eq!(v2::Check::tag(), packed.tag().get());
    }

    /// `health.v2` §2.2, §2.10: a status raised to its worst check carries
    /// the profile's reason `check`, and one not yet established
    /// `starting`; the levels are `health.v1`'s.
    #[test]
    fn a_v2_status_raised_to_a_check_says_check() {
        let declared = (Level::Ok, "serving".to_owned());
        let mut checks = BTreeMap::new();
        checks.insert("disk", Some(Level::Degraded));
        let a = aggregate(Major::V2, Some(&declared), &checks);
        assert_eq!(
            (a.level, a.reason.as_str(), a.raised_by.as_deref()),
            (Some(Level::Degraded), v2::CHECK, Some("disk"))
        );
        let a = aggregate(Major::V2, None, &checks);
        assert_eq!((a.level, a.reason.as_str()), (None, v2::STARTING));
        checks.clear();
        let a = aggregate(Major::V2, Some(&declared), &checks);
        assert_eq!((a.level, a.reason.as_str()), (Some(Level::Ok), "serving"));
        assert_eq!(
            super::seconds(1_794_062_759_900_000_000, 0),
            Some(1_794_062_759)
        );
        assert_eq!(
            super::seconds(5_000_000_000, 1_767_225_600),
            None,
            "a clock the epoch does not code: the field is left absent (§2.3)"
        );
    }

    /// An application shares the handle across tasks, and spawns its calls.
    #[test]
    fn the_handle_is_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        fn spawnable<F: std::future::Future + Send>(_: F) {}
        shared::<super::Health>();
        let _ = |h: super::Health| {
            spawnable(async move {
                let _ = h.set_status(Level::Ok, "serving").await;
                let _ = h.set_check("disk", Level::Ok, "").await;
                let _ = h.retire_check("disk").await;
                let _ = h.fault("app_x", Level::Ok, "").await;
                let _ = h.fault_argument("app_x", Level::Ok, Some(1)).await;
            });
        };
    }
}
