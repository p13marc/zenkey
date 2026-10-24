//! A zk2 owner to interoperate with (#609's live half, #610): it brings up
//! one service implementing the given contracts and runs until its standard
//! input closes.
//!
//! ```text
//! cargo run -p zenkey --example owner -- [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral]
//!     [--health] [--health-status <LEVEL>[:<reason>]] [--health-check <name>=<LEVEL>[:<detail>]]...
//!     [--tokenless <iface>]... [--clock-reference <key>] [--clock-offset-ms <ms>]
//!     [--cardinality <iface>:<resource>=<n>]...
//!     [--config] [--config-floor <s>] [--config-apply-after-ms <ms>] [--config-no-persist]
//!     [--alarms] [--alarms-record <file>] [--alarms-archive <system>/<service>]
//!     [--alarms-rules <rule>[,<rule>...]] [--telemetry]
//!     [--desired-author] [--desired-record <file>] [--desired-window <s>]
//!     [--desired-target <system>/<service>] [--desired-unbound] [--desired-archive <system>/<service>]
//!     [--desired-watch] [--desired-reread-ms <ms>] [--desired-hold]
//!     <address> <contract.toml>...
//! ```
//!
//! `owner --help` (or `-h`) prints the interface a live runner drives —
//! its options, what it prints, each command word and its answers, the
//! `alarms.v1` surface whole — and exits 0 (#800, F-133). This doc is the
//! long form of the same.
//!
//! The address is `<system>/<service>`, or `@hostid.v1/<service>` for a
//! system minted from the machine id (`spec/profiles/hostid/v1.md` §2.3,
//! #719). The system is minted before the session opens (§2.7), and a host
//! with no id stops the owner with nothing declared (§2.6): the error names
//! every path, and the exit is 1. `--hostid-root <dir>` reads the inputs
//! under `<dir>` instead of `/`, its links resolved in it, as a scenario's
//! root (`spec/profiles/hostid/scenarios.md`); `--hostid-ephemeral` is
//! `hostid = { ephemeral = true }`.
//!
//! By default the session is a router listening on an ephemeral loopback
//! port, so a test connects to it as a client. Two lines on standard output
//! say what to connect to and what came up:
//!
//! ```text
//! listening tcp/127.0.0.1:<port>
//! ready zk2/<system>/<service>/@zk/instance/<instance>
//! ```
//!
//! With `--connect <endpoint>`, the session is instead a client of the
//! router at `<endpoint>`, with its HLC on (§4.3), and the first line is
//! `connected <endpoint>`. That is the owner the black-box scenarios watch
//! through a separate router (#660): `state.md §1` tells the owner's stamp
//! from the router's only when the two sessions differ (§4.2, "Observing
//! S1"), and `presence.md §2` sees a refusal only through a router that
//! outlives the owner.
//!
//! Every resource of every contract is exposed (§8.1–§8.4), and for a
//! counterpart to read:
//! - a state resource with no template parameters and a `raw` type holds
//!   the value `ok`, stamped (S1), answered on GET (S2), and put before the
//!   tokens, so a GET made the moment they appear finds it (§8.2, "State
//!   values", F-68);
//! - every operation is served, so everything exposed is callable (§8.2,
//!   "Exposed"; #670): a `raw` request and response echo the request; any
//!   other types refuse with `app` and no detail (O3, §5.2), since this
//!   owner decodes no schema, and an operation with no `error` type has no
//!   detail to send (F-65). An operation with template parameters is served
//!   over the whole template (§5.1): a call whose key binds every parameter
//!   answers for that member, and an echo to a fan-out that leaves one
//!   unbound names no member, so it is refused `internal`, as §5.1 says of
//!   a server that cannot name one.
//!
//! - a metric stream or state with no template parameters (`telemetry.v1`
//!   §2.1) has its writer declared before start, the one the `telemetry`
//!   commands put through, so its publisher knows its subscribers before
//!   the first put; nothing is put on it until a command says so.
//!
//! Capabilities named by `capability:` gates are all held, so gated
//! resources are exposed too.
//!
//! **`health.v1`** (`spec/profiles/health/v1.md`, #721) is implemented by
//! the runtime ([`zenkey::health`]) with `--health`, or when a contract
//! file given is the standard one (its fingerprint is `e9dbcdcb…`, as
//! `spec/profiles/health/health.v1.toml` is): the status is put before the
//! tokens, `UNSPECIFIED` with reason `starting` unless
//! `--health-status <LEVEL>[:<reason>]` sets one (§2.3); each
//! `--health-check <name>=<LEVEL>[:<detail>]` is a check put before the
//! tokens too, the status raised to the worst of them (§2.2). A level is
//! `OK`, `DEGRADED` or `FAILED`, in any case; a check may be `UNSPECIFIED`,
//! one it cannot run (§2.3). The status is re-put every 28.5 s.
//!
//! **Commands**, one per line on standard input, each answered by one line
//! on standard output, while the owner runs:
//!
//! ```text
//! status <LEVEL> [<reason>...]           set_status
//! check <name> <LEVEL> [<detail>...]     set_check (UNSPECIFIED: set_check_unknown)
//! retire <name>                          retire_check
//! fault <code> <LEVEL> [<detail>...]     fault: an application's code (§2.10)
//! confirm on|off                         set_confirming: off lets the status go stale (§2.4)
//! clock-offset <ms>                      the clock's simulated offset (0: set right)
//! put <iface> <resource> [<value>...]    one write on a member (core §2.7)
//! delete <iface> <resource> [<value>...] a state member's delete
//! fill <iface> <resource> <n>            put members m0 … m<n−1>, one write each
//! ```
//!
//! **The budget** (core §2.7, #741), for a runner to reach each bound:
//! `put` writes once on the member of a stream, state or event resource
//! whose template parameters the values bind, in order (a rest parameter
//! takes its value's `/`-separated chunks): a stamped state put, a stream
//! sample, or one event occurrence, each of the bytes `ok`, which is a
//! value only of a raw type. It is answered `put <key>`, an occurrence's
//! key with its ULID. `delete` deletes a state member, answered `deleted
//! <key>`. `fill` puts members `m0` to `m<n−1>` (every parameter bound to
//! `m<i>`), stopping at the first refusal, answered `filled <n>`. A write
//! the runtime refuses — a new member while the live members are at the
//! bound, an occurrence beyond the member's rate — is answered `refused
//! <command>: <why>`, `why` naming the resource, the bound and whose it is
//! (or the rate), and for `fill` how many it put first. `--cardinality
//! <iface>:<resource>=<n>` lowers a template's bound in the descriptor
//! (core §3.3), e.g. `--cardinality budget.v1:state/items/{item}=2`.
//!
//! A `health.v1` command is answered `health status=<LEVEL> declared=<LEVEL>
//! raised_by=<name>` (`UNSPECIFIED` for no level, `-` for none): the status
//! as published, the level declared, and the check that raised it (§2.2).
//! `clock-offset` is answered `clock-offset <ms>`. Anything refused, by the
//! runtime or here, is answered `refused <command>: <why>`, and changes
//! nothing the runtime did not report.
//!
//! **A clock ahead** (§2.5): `--clock-reference <key>` is the router-stamped
//! heartbeat the clock guard watches (core §4.3), and `--clock-offset-ms`
//! shifts this owner's clock (the runtime's `Minter::simulate_offset`).
//! With `--connect`, a nonzero offset leaves the session's HLC off: a
//! session whose HLC runs re-stamps a stamp beyond its own delta before it
//! leaves (measured in `tests/profile_health.rs`), so the simulated stamps
//! would never reach the router. Off, they reach it as a session whose HLC
//! is itself ahead would send them, and the router re-stamps the
//! `clock_ahead` fault (core §4.1).
//!
//! `--tokenless <iface>` puts an interface in the deployment's tokenless
//! set (core §8.1, U22): `health.v1`, for one.
//!
//! **`config.v1`** (`spec/profiles/config/v1.md`, #768) is implemented by
//! the runtime ([`zenkey::config_v1`]) with `--config`, or when a contract
//! file given is the standard one (its fingerprint is `727ff009…`, as
//! `spec/profiles/config/config.v1.toml` is). The schema is
//! `conformance/lifecycle.json`'s `rf`, as `scenarios.md` runs it, and a
//! contract group after it:
//!
//! | Group | Class | Parameter | Startup |
//! |---|---|---|---|
//! | `duty` | hot | `fraction_ppm`, 0 to 1000000 ppm | 10000, `file` |
//! | `radio` | reach | `frequency_hz`, 150000000 to 960000000 Hz | 868100000, `file` |
//! | `link` | reach | `net_id`, 0 to 255 | 1, `default` |
//! | `sim` | hot | `pin`, text, sensitive | `"0000"`, `file` |
//! | `queue` | hot, outside the allowlist | `tx_queue_len`, at least 0 packets | 100, `default` |
//! | `modulation` | contract | `spreading_factor`, 7 to 12 | 9, `file` |
//!
//! `--config-floor <s>` is the floor (10 by default, §2.8);
//! `--config-apply-after-ms <ms>` is how long after a reach `set` the
//! device applies it, the `apply_at` it states (1000 by default, below the
//! floor, §2.9); `--config-no-persist` leaves the owner without a persisted
//! layer, so `persist` is listed unavailable, cause `config` (§2.13). The
//! device is simulated: it holds what it is told, in memory, and its
//! persisted layer survives `config restart`. The runtime logs each request
//! on standard error, a sensitive value written `"<redacted>"` (§2.15):
//! `zenkey::config_v1` logs at `info` by default here.
//!
//! Its commands, each answered by one line:
//!
//! ```text
//! config fail-apply [<words>...]     the device refuses its next apply, in these words
//! config fail-undo [unknown]         its next undo fails, reading back what it holds (unknown: nothing)
//! config fail-persist [<words>...]   its next persisted-layer write fails: persist answers internal
//! config block-persist [<words>...]  its persisted layer cannot be written until cleared: unavailable, cause config
//! config clear                       every failure above cleared
//! config device                      what the device holds and did
//! config desired <json>              a DesiredConfig to converge (§2.18); the report is the view's and the status's
//! config offset <ms>                 the owner's clock shifted from this host's (a test's: §2.8, §2.14)
//! config restart                     a restart (§2.12): the service closes and comes up as a new instance
//! ```
//!
//! A failure command is answered `config device` followed by the device's
//! state, as `config device` is: `held=<json> applies=<n> undos=<n>
//! persisted=<json|none> next=<the failures set, or ->`, `held` being what
//! the device runs with, a sensitive value written `"<redacted>"` (§2.15),
//! `applies` the applies it made (a refused one is not), and `undos` every
//! undo it was asked for, a failed one included. `desired` is
//! answered `config desired <revision>`, `offset` `config offset <ms>`, and
//! `restart` `config restarted zk2/<system>/<service>/@zk/instance/<id>`, the
//! new instance's key, once the new instance's tokens are declared. A
//! restart keeps the persisted layer and the change events of the last
//! hour (§2.17), and the device runs its startup values again; anything else
//! of the old instance is gone, as a process restart leaves it, but the
//! desired document `config desired` bound: that binding is the
//! deployment's, so the new instance converges it again, its view carrying
//! no `desired` until it has acted (§2.18; text 0.3, F-114).
//!
//! **`alarms.v1`** (`spec/profiles/alarms/v1.md`, #800) is implemented by
//! the runtime ([`zenkey::alarms`]) with `--alarms`, implied by any
//! `--alarms-*` option, or when a contract file given is the standard one
//! (its fingerprint is `95579151…`, as `spec/profiles/alarms/alarms.v1.toml`
//! is). With `--alarms`, an `alarms.v1` contract file of another revision is
//! implemented by the runtime on that revision instead, its
//! `freshness.ttl_s` setting the confirmation period: a runner's copy of
//! the standard contract with a short horizon is its fast tier. Each firing
//! alarm is re-put every 427.5 s at the standard horizon (§2.7).
//!
//! - `--alarms-record <file>` is the owner's persistent record
//!   ([`zenkey::alarms::FileRecord`]): read at start, and rewritten whole
//!   after every put and every delete. It is JSON, `{"profile":
//!   "alarms.v1", "alarms": {<alert key>: <document>, …}}`, each document
//!   as it was put, the resolved one included until its delete.
//! - `--alarms-archive <system>/<service>` is an archive on the owner's own
//!   side (core §4.4), read explicitly at start when the record has
//!   nothing (§2.9), within 2 s.
//! - `--alarms-rules <rule>,…` are the rules the owner evaluates: a firing
//!   document of another rule found at start is resolved (§2.9, step 3).
//!   By default, every rule.
//! - `--cardinality alarms.v1:state/alarms/{alert_key}=<n>` lowers the bound
//!   (§2.8), before the adoption.
//!
//! **A restart is the process's**, as §5 of the scenarios runs it: kill the
//! owner (a process that ends without closing), or close its standard input
//! (a clean close, which resolves and deletes nothing, §2.9), then start it
//! again with the same `--alarms-record`. It comes up on a new session,
//! with a new zid, and before its tokens it deletes what its record holds
//! resolved, undecodable or under a key that does not re-derive, resolves
//! what a rule no longer evaluated raised, and puts the rest again with
//! their timestamps. A runner writes documents into the record only while
//! no owner runs on it.
//!
//! Its commands, each answered by one line:
//!
//! ```text
//! alarm raise <json>                      the condition holds: {"rule", "severity", "summary", "labels"?, "category"?}
//! alarm resolve <alert_key>               the resolved put, then the delete (§2.5)
//! alarm confirm <alert_key>|all on|off    off: the owner stops evaluating it, and lets it go stale (§2.7)
//! alarm list                              the alarms held
//! alarm adoption                          what the start took over (§2.9)
//! ```
//!
//! `raise` is answered `alarm <act> <alert_key> <document>`, `act` being
//! what the runtime made of it from what it holds (§2.5): `raised`,
//! `escalated` (another severity: a new `timestamp`), `refreshed` (its
//! summary, category or a host-scoped label changed: the `timestamp` kept,
//! `observed_at_ms` set), or `unchanged` (nothing put); `document` is the
//! alarm's JSON on one line. A raise past the bound is answered `refused
//! …`, naming the bound and whose it is (core §2.7). `resolve` is answered
//! `alarm resolved <alert_key> <document>`, the resolved document, or
//! `alarm none <alert_key>` when none is held there. `confirm` is answered
//! as it was asked, `alarm confirm <alert_key>|all on|off`; any act on an
//! alarm turns its confirmation back on. `list` is answered `alarm held
//! <json>`, an object of the documents held by alert key. `adoption` is
//! answered `alarm adoption source=<record|archive|nothing> adopted=<keys>
//! retired=<key>:<resolved|undecodable|phantom>,… resolved=<keys>
//! over_bound=<keys>`, each list comma-separated, `-` when empty.
//!
//! **`telemetry.v1`** (`spec/profiles/telemetry/v1.md`, #819) is the
//! runtime's writers' ([`zenkey::telemetry`]): every writer of a metric
//! resource checks each put against the resource's declaration, refuses a
//! counter or histogram below the last value it put in its epoch, and
//! writes the epoch (the instance id) where the application left it out.
//! **Its contract.** `--telemetry` implements `meter.v1`, the scenarios'
//! contract (`spec/profiles/telemetry/scenarios.md`, "Conventions"),
//! embedded in this example byte for byte as the runtime keeps it
//! (`zenkey/tests/contracts/meter.v1.toml` and `meter.json`, with the
//! profile's `telemetry.v1.json`), so the owner needs no file: its
//! fingerprint is that file's. A contract file given that implements
//! `meter.v1` is implemented instead, a runner's own copy. The `telemetry`
//! commands reach the metric resources of every implemented contract,
//! `--telemetry` or not: a resource is named by its template (`frames`),
//! or as the descriptor names it (`stream/frames`), and as
//! `<iface>:<resource>` when two interfaces have it. A resource with
//! template parameters is refused at this version.
//!
//! Its commands, each answered by one line:
//!
//! ```text
//! telemetry put <resource> <json> [<attachment json>]   through the checked writer
//! telemetry raw <resource> <json> [<attachment json>]   from the session, unchecked
//! telemetry new-epoch                                   Service::new_epoch (core §8.1)
//! telemetry instance                                    the current instance id
//! ```
//!
//! `<json>` is the payload as JSON text, sent as written for a JSON
//! resource and as the same data item in CBOR for a CBOR one; the
//! attachment, a second JSON text on the line, likewise (`""` is the empty
//! string, an epoch no reader accepts). `put` goes through the member's
//! writer, the one `put` and `fill` use, which keeps the last value; it is
//! answered `telemetry put <key> epoch=<id>`, the epoch the writer holds the
//! member to (written in the Point's `epoch`, or the Epoch attachment, where
//! the application left it out, §2.9), or refused `refused <line>:
//! telemetry.v1 <why>: <words>`, `why` one of `does_not <problem>,…` (§2.6's
//! tokens, in its order), `undecodable`, `epoch given=<json> current=<id>`
//! or `decreased epoch=<id>`; nothing is published on a refusal. `raw` puts
//! the bytes on the member's key from the owner's session with the
//! resource's encoding, as an owner the text does not allow would, answered
//! `telemetry raw <key>`. `new-epoch` mints a new instance id
//! make-before-break, answered `telemetry new-epoch <id>` once the old
//! tokens are withdrawn: a Point put after it carries the new id. `instance`
//! is answered `telemetry instance <id>`.
//!
//! **`desired.v1`** (`spec/profiles/desired/v1.md`, #840) is the runtime's
//! ([`zenkey::desired`]), on the scenarios' contracts
//! (`spec/profiles/desired/scenarios.md`, "The contracts"), embedded byte
//! for byte as `zenkey/tests/contracts/` keeps them (`tasking.v1.toml`,
//! `worker.v1.toml`, `tasking.json`, with the profile's `desired.v1.json`):
//! a contract file given that implements `tasking.v1` or `worker.v1` is
//! implemented instead, a runner's own copy. One owner may be the author,
//! the target, or both (§2.1).
//!
//! - `--desired-author` implements `tasking.v1` and authors its
//!   `tasks/{system}/{service}` (§2.11): a document is put only when its
//!   content changes; at a start, the record's documents are put again and
//!   its deletes within the window made again, before the tokens.
//! - `--desired-record <file>` is the author's record
//!   ([`zenkey::desired::FileRecord`]), read at start and rewritten whole
//!   after every put and delete: `{"profile": "desired.v1", "documents":
//!   {"<system>/<service>": {"stamp": "<time>/<id>", "document": <JSON>}},
//!   "deletes": {"<system>/<service>": {"stamp": …, "at_ms": <n>}}}`.
//! - `--desired-window <s>` is the service's tombstone window (core S3), 60
//!   s by default.
//! - `--desired-target <system>/<service>` implements `worker.v1`, its role
//!   `task` bound to that author with `{system} = self.system` and
//!   `{service} = self.service` (§2.3): its slice is
//!   `zk2/<author>/tasking.v1/state/tasks/<own system>/<own service>`, its
//!   report `…/worker.v1/state/desired/task`, what it runs
//!   `…/worker.v1/state/running`, its baseline `{"revision": 0, "mode":
//!   "idle", "rate_hz": 1}`. `--desired-unbound` implements `worker.v1`
//!   with the role unbound, the kill switch (§2.12).
//! - `--desired-archive <system>/<service>` is the archive the target reads
//!   its slice's last-known from (§2.9), within 1 s, when a seed draws
//!   nothing and a complete presence read shows the author absent (core
//!   §8.1).
//! - `--desired-watch` gives the target a watch session of its own, a
//!   second client of the same router, where it follows the author's
//!   instance tokens (core §8.1, 0.37); without it, it reads them every
//!   500 ms. `--desired-reread-ms <ms>` is the deployment's re-read period
//!   (§2.5). `--desired-hold` holds the drive's first act from its start
//!   until `desired release` (`scenarios.md` §5).
//!
//! The drive is simulated, as `scenarios.md`'s "The target" says: it refuses
//! a document holding a member `endpoint` `reach` at `/endpoint`, and one
//! whose `rate_hz` is above 100 `invalid` at `/rate_hz`; it applies any
//! other by putting its value on `Run`, a delete by putting the baseline.
//! **A restart is the process's**, as for `alarms.v1`: kill the owner, or
//! close its standard input, and start it again (the same
//! `--desired-record` for an author). Run `--connect` to a router of the
//! runner's: a target reads presence through its session, and a router
//! alone reaches no other node.
//!
//! Its commands, each answered by one line:
//!
//! ```text
//! desired put <target> <json>               author: <target>'s document, <system>/<service>
//! desired delete <target>                   author: its delete
//! desired documents                         author: the documents held
//! desired adoption                          author: what the start took over (§2.11)
//! desired new-epoch                         a re-mint in this session (core §8.1): the author puts again what it keeps
//! desired report                            target: its report (§2.10)
//! desired running                           target: what the drive runs
//! desired fail-next                         target: the drive's next act fails after setting the rate
//! desired refuse-next <code> [<pointer>]    target: the drive refuses its next act
//! desired hold | release                    target: the drive's next act waits at its start | goes on
//! desired reread <ms>|off                   target: the re-read period (§2.5)
//! desired acts                              target: the acts and seeds so far
//! ```
//!
//! `put` is answered `desired put <key> <stamp>`, the stamp as a report
//! writes it (`<time>/<id>`, §2.4), or `desired unchanged <key>` when the
//! document is the one held (nothing put); a put past the bound is
//! refused, naming it (§2.13). `delete` is answered `desired deleted <key>
//! <stamp>`, or `desired none <key>` when no document is held. `documents`
//! is answered `desired documents <json>`, the documents by target;
//! `adoption` `desired adoption found=<record|archive|nothing>
//! reput=<targets> redeleted=<targets> expired=<targets> failed=<targets>`,
//! each list comma-separated, `-` when empty; `new-epoch` `desired new-epoch
//! zk2/<system>/<service>/@zk/instance/<id>`; `report` `desired report
//! <json>`, the `desired.v1.Report` as held now; `running` `desired running
//! <json>`; `acts` `desired acts <n> seeds=<n> held=<true|false>`; every
//! other command as it was asked.

use std::collections::HashMap;
use std::future::Future;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use zenkey::alarms::{self, Alarms, Condition, FileRecord};
use zenkey::config_v1::{self, Configuration, DesiredConfig, Startup};
use zenkey::desired::{self, AuthorStartup, ReportAt, TargetStartup, Written};
use zenkey::health::{self, Effective, Health, Level};
use zenkey::hostid::{HostIdMinter, HostIdSource};
use zenkey::model::authoring::{Encoding as WireEncoding, Kind};
use zenkey::model::contract::{Body, Contract, Resource, load_path};
use zenkey::model::grammar::IfaceId;
use zenkey::model::schema::TypeId;
use zenkey::model::telemetry as telemetry_model;
use zenkey::model::template::{Bindings, Segment};
use zenkey::operation::OperationServer;
use zenkey::state::StateWriter;
use zenkey::writer::{EventWriter, Writer};
use zenkey::{Address, Implementation, OpError, Service, ServiceBuilder, ServiceConfig};

#[path = "owner/config.rs"]
pub mod config;

#[path = "owner/desired.rs"]
pub mod drive;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The command line.
pub struct Options {
    /// `--connect <endpoint>`: a client of that router, not a router.
    pub connect: Option<String>,
    /// `--hostid-root <dir>`: the directory standing in for `/` when the
    /// address is minted (hostid.v1); `/` otherwise.
    pub hostid_root: Option<PathBuf>,
    /// `--hostid-ephemeral`: `hostid.ephemeral` (hostid.v1 §2.6).
    pub hostid_ephemeral: bool,
    /// `--health`, or implied by `--health-status`, `--health-check` or
    /// the standard contract among the files: `health.v1` by the runtime.
    pub health: bool,
    /// `--health-status <LEVEL>[:<reason>]`: the status put before start.
    pub health_status: Option<(Level, String)>,
    /// `--health-check <name>=<LEVEL>[:<detail>]`: checks put before start
    /// (`None`: `UNSPECIFIED`).
    pub health_checks: Vec<(String, Option<Level>, String)>,
    /// `--tokenless <iface>`: the deployment's tokenless set (core §8.1).
    pub tokenless: Vec<String>,
    /// `--clock-reference <key>`: the clock guard's reference (core §4.3).
    pub clock_reference: Option<String>,
    /// `--clock-offset-ms <ms>`: this owner's clock, shifted.
    pub clock_offset_ms: i64,
    /// `--cardinality <iface>:<resource>=<n>`: a template's bound lowered in
    /// the descriptor (core §3.3, §2.7).
    pub cardinality: Vec<(String, String, u64)>,
    /// `--config`, or implied by a `--config-*` option or the standard
    /// contract among the files: `config.v1` by the runtime.
    pub config: bool,
    /// `--config-floor <s>`: the floor (config.v1 §2.8).
    pub config_floor_s: u64,
    /// `--config-apply-after-ms <ms>`: the reach apply's delay (§2.9).
    pub config_apply_after_ms: u64,
    /// Without `--config-no-persist`: a persisted layer (§2.13).
    pub config_persist: bool,
    /// `--alarms`, or implied by an `--alarms-*` option or the standard
    /// contract among the files: `alarms.v1` by the runtime.
    pub alarms: bool,
    /// `--alarms-record <file>`: the owner's persistent record (§2.9).
    pub alarms_record: Option<PathBuf>,
    /// `--alarms-archive <system>/<service>`: an archive on its own side.
    pub alarms_archive: Option<String>,
    /// `--alarms-rules <rule>,…`: the rules it evaluates; `None`: every one.
    pub alarms_rules: Option<Vec<String>>,
    /// `--telemetry`: `meter.v1`, the scenarios' contract, implemented
    /// (telemetry.v1), unless a contract file implements it.
    pub telemetry: bool,
    /// `--desired-author`, or implied by `--desired-record`: the
    /// scenarios' `tasking.v1` implemented, its tasks authored (desired.v1
    /// §2.11).
    pub desired_author: bool,
    /// `--desired-record <file>`: the author's record (§2.11).
    pub desired_record: Option<PathBuf>,
    /// `--desired-window <s>`: the service's tombstone window (core S3).
    pub desired_window_s: Option<u64>,
    /// `--desired-target <system>/<service>`: `worker.v1` implemented, its
    /// role `task` bound to that author (§2.3).
    pub desired_target: Option<String>,
    /// `--desired-unbound`: `worker.v1` implemented, its role unbound
    /// (§2.12).
    pub desired_unbound: bool,
    /// `--desired-archive <system>/<service>`: the target's last-known
    /// (§2.9).
    pub desired_archive: Option<String>,
    /// `--desired-watch`: the target follows its author on a watch session
    /// of its own (core §8.1); else it reads presence every 500 ms.
    pub desired_watch: bool,
    /// `--desired-reread-ms <ms>`: the target's re-read period (§2.5).
    pub desired_reread_ms: Option<u64>,
    /// `--desired-hold`: the drive holds its first act until `desired
    /// release`.
    pub desired_hold: bool,
    pub address: String,
    pub contracts: Vec<PathBuf>,
}

/// A level as the command line spells it: `Some(None)` for `UNSPECIFIED`.
fn parse_level(s: &str) -> Option<Option<Level>> {
    match s.to_ascii_uppercase().as_str() {
        "OK" => Some(Some(Level::Ok)),
        "DEGRADED" => Some(Some(Level::Degraded)),
        "FAILED" => Some(Some(Level::Failed)),
        "UNSPECIFIED" => Some(None),
        _ => None,
    }
}

/// What `owner --help` prints (#800, AC; zk2py's F-133): the usage, what
/// standard output carries, every command word, and the `alarms.v1`
/// surface whole, as the agreed interface a live runner drives. The answer
/// formats are the module doc's, unchanged.
pub const HELP: &str = "\
owner: a zk2 owner to interoperate with (#609, #610). It brings up one service
implementing the given contracts, and runs until its standard input closes.

usage: owner [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral]
    [--health] [--health-status <LEVEL>[:<reason>]] [--health-check <name>=<LEVEL>[:<detail>]]...
    [--tokenless <iface>]... [--clock-reference <key>] [--clock-offset-ms <ms>]
    [--cardinality <iface>:<resource>=<n>]...
    [--config] [--config-floor <s>] [--config-apply-after-ms <ms>] [--config-no-persist]
    [--alarms] [--alarms-record <file>] [--alarms-archive <system>/<service>]
    [--alarms-rules <rule>[,<rule>...]] [--telemetry]
    [--desired-author] [--desired-record <file>] [--desired-window <s>]
    [--desired-target <system>/<service>] [--desired-unbound]
    [--desired-archive <system>/<service>] [--desired-watch] [--desired-reread-ms <ms>]
    [--desired-hold]
    <system>/<service>|@hostid.v1/<service> <contract.toml>...

Standard output, first:
  listening tcp/127.0.0.1:<port>       a router on an ephemeral port (with --connect:
                                       connected <endpoint>, a client of that router)
  ready zk2/<system>/<service>/@zk/instance/<instance>
then one line per line of standard input. A command refused, by the runtime or
here, is answered `refused <line>: <why>`; an unknown command word is answered
`refused <line>: unknown command`, and a malformed command of a known word with
that word's usage.

Command words:
  status, check, retire, fault, confirm   health.v1 (--health)
  clock-offset <ms>                       the clock's simulated offset
  put, delete, fill                       one write on a member (core §2.7)
  config ...                              config.v1 (--config)
  alarm ...                               alarms.v1 (--alarms), below
  telemetry ...                           telemetry.v1's writers, below
  desired ...                             desired.v1 (--desired-*), below

alarms.v1 (spec/profiles/alarms/v1.md)
  Implemented by the runtime with --alarms, implied by any --alarms-* option,
  or when a contract file given is the standard alarms.v1 (fingerprint
  95579151...). With --alarms, an alarms.v1 contract file of another revision
  is implemented on that revision instead, its freshness.ttl_s setting the
  confirmation period: a runner's copy of the standard contract with a short
  horizon is accepted (each firing alarm re-put every 427.5 s at the
  standard 900 s, every 1.9 s at 4 s).

  --alarms                        implement alarms.v1
  --alarms-record <file>          the owner's persistent record, read at start and
                                  rewritten whole after every put and every delete:
                                  {\"profile\": \"alarms.v1\", \"alarms\": {\"<alert key>\": <document>, ...}},
                                  each document as it was put, a resolved one
                                  included until its delete. Write documents into it
                                  only while no owner runs on it.
  --alarms-archive <sys>/<svc>    an archive on the owner's side, read explicitly at
                                  start (within 2 s) when the record holds nothing
  --alarms-rules <rule>,...       the rules evaluated: a firing document of another
                                  rule found at start is resolved (default: every rule)
  --cardinality alarms.v1:state/alarms/{alert_key}=<n>
                                  the bound, lowered before the adoption

  alarm raise <json>              the condition holds; <json>, the rest of the line as
                                  written: {\"rule\": <string>,
                                  \"severity\": \"info\"|\"warning\"|\"critical\",
                                  \"summary\": <string>, \"labels\"?: {<name>: <value>, ...},
                                  \"category\"?: <string>}
  alarm resolve <alert_key>       the resolved put, then the delete
  alarm confirm <alert_key>|all on|off
                                  off: the owner stops evaluating it and lets it go
                                  stale; any act on an alarm turns it back on
  alarm list                      the alarms held
  alarm adoption                  what the start took over

  Answers, one line each (<document> is the alarm's JSON on one line, as put):
    alarm raised|escalated|refreshed|unchanged <alert_key> <document>
    alarm resolved <alert_key> <document>, or alarm none <alert_key>
    alarm confirm <alert_key>|all on|off
    alarm held <json>             the documents held, by alert key
    alarm adoption source=<record|archive|nothing> adopted=<keys>
      retired=<key>:<resolved|undecodable|phantom>,... resolved=<keys> over_bound=<keys>
                                  each list comma-separated, - when empty
    refused <line>: <why>         a raise past the bound names the bound and whose it is

  A restart is the process's: kill it (no close), or close standard input (a
  clean close, which resolves and deletes nothing), then start it again with
  the same --alarms-record.

telemetry.v1 (spec/profiles/telemetry/v1.md)
  Every writer of a metric resource checks each put against its declaration,
  refuses a counter or histogram below the last value it put in its epoch,
  and writes the epoch (the instance id) where the put leaves it out: a
  counter's or histogram's Point gains \"epoch\", a bare one whose attachment
  is telemetry.v1.Epoch gets it as its attachment.

  --telemetry                     implement meter.v1, the scenarios' contract
                                  (scenarios.md, Conventions), embedded as
                                  zenkey/tests/contracts/meter.v1.toml has it; a
                                  meter.v1 contract file given is used instead
  The commands reach the metric resources of every contract implemented, named
  by template (frames), as the descriptor names them (stream/frames), or as
  <iface>:<resource>; a resource with template parameters is refused.

  telemetry put <resource> <json> [<attachment json>]
                                  through the member's checked writer
  telemetry raw <resource> <json> [<attachment json>]
                                  from the owner's session, unchecked
  telemetry new-epoch             a new instance id, make-before-break
  telemetry instance              the current instance id
  <json> is sent as written to a JSON resource, as the same item in CBOR to a
  CBOR one; the attachment likewise (\"\" is the empty string).

  Answers, one line each:
    telemetry put <key> epoch=<id>
    telemetry raw <key>
    telemetry new-epoch <id>      once the old tokens are withdrawn
    telemetry instance <id>
    refused <line>: telemetry.v1 <why>: <words>
                                  <why>: does_not <problem>[,<problem>...] (v1.md
                                  §2.6's tokens), undecodable,
                                  epoch given=<json> current=<id>, decreased epoch=<id>;
                                  nothing is published

desired.v1 (spec/profiles/desired/v1.md)
  On the scenarios' contracts (scenarios.md, The contracts), embedded as
  zenkey/tests/contracts/ keeps them: tasking.v1 (the author's) and worker.v1
  (the target's); a tasking.v1 or worker.v1 contract file given is used
  instead. One owner may be the author, the target, or both. Use --connect.

  --desired-author                implement tasking.v1 and author tasks/{system}/{service}:
                                  a document put only when its content changes
  --desired-record <file>         the author's record, read at start (each document put
                                  again, each delete within the window made again,
                                  before the tokens) and rewritten whole after every
                                  put and delete: {\"profile\": \"desired.v1\",
                                  \"documents\": {\"<system>/<service>\": {\"stamp\": \"<time>/<id>\",
                                  \"document\": <JSON>}}, \"deletes\": {\"<system>/<service>\":
                                  {\"stamp\": ..., \"at_ms\": <n>}}}
  --desired-window <s>            the tombstone window (core S3), 60 s by default
  --desired-target <sys>/<svc>    implement worker.v1, its role task bound to that author,
                                  {system} = self.system, {service} = self.service
  --desired-unbound               implement worker.v1, the role unbound: the kill switch
  --desired-archive <sys>/<svc>   the target's last-known (within 1 s), read when a seed
                                  draws nothing and a complete presence read shows the
                                  author absent
  --desired-watch                 the target follows its author on a watch session of its
                                  own; else it reads its presence every 500 ms
  --desired-reread-ms <ms>        the target's re-read period
  --desired-hold                  the drive holds its first act until desired release

  The drive: a document with a member endpoint is refused reach at /endpoint, one
  with rate_hz above 100 invalid at /rate_hz; any other is applied, its value put on
  worker.v1's state/running; a delete puts the baseline
  {\"revision\": 0, \"mode\": \"idle\", \"rate_hz\": 1}.

  desired put <target> <json>     author: <target> (<system>/<service>)'s document, the
                                  rest of the line as written
  desired delete <target>         author: its delete
  desired documents               author: the documents held
  desired adoption                author: what the start took over
  desired new-epoch               a re-mint in this session: the author puts again what
                                  it keeps, before the new tokens
  desired report                  target: its desired.v1.Report
  desired running                 target: what the drive runs
  desired fail-next               target: the drive's next act fails after setting the rate
  desired refuse-next <code> [<pointer>]
                                  target: the drive refuses its next act so
  desired hold                    target: the drive's next act waits at its start
  desired release                 target: a held act goes on
  desired reread <ms>|off         target: the re-read period
  desired acts                    target: the acts and seeds so far

  Answers, one line each:
    desired put <key> <stamp>     the stamp as a report writes it, <time>/<id>
    desired unchanged <key>       the document held: nothing put
    desired deleted <key> <stamp>, or desired none <key>
    desired documents <json>      the documents held, by target
    desired adoption found=<record|archive|nothing> reput=<targets> redeleted=<targets>
      expired=<targets> failed=<targets>
                                  each list comma-separated, - when empty
    desired new-epoch zk2/<system>/<service>/@zk/instance/<id>
    desired report <json>
    desired running <json>
    desired acts <n> seeds=<n> held=<true|false>
    desired fail-next|refuse-next <code>|hold|release|reread <ms>|off
    refused <line>: <why>

  A restart is the process's: kill it, or close standard input, and start it
  again (an author with the same --desired-record).
";

fn level_name(l: Option<Level>) -> String {
    l.map_or_else(|| "UNSPECIFIED".to_owned(), |l| l.to_string())
}

impl Options {
    /// Reads the options of the module doc's usage, in any order before
    /// the address.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let usage = || {
            "usage: owner [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral] \
             [--health] [--health-status <LEVEL>[:<reason>]] \
             [--health-check <name>=<LEVEL>[:<detail>]]... [--tokenless <iface>]... \
             [--clock-reference <key>] [--clock-offset-ms <ms>] \
             [--cardinality <iface>:<resource>=<n>]... \
             [--config] [--config-floor <s>] [--config-apply-after-ms <ms>] [--config-no-persist] \
             [--alarms] [--alarms-record <file>] [--alarms-archive <system>/<service>] \
             [--alarms-rules <rule>[,<rule>...]] [--telemetry] \
             [--desired-author] [--desired-record <file>] [--desired-window <s>] \
             [--desired-target <system>/<service>] [--desired-unbound] \
             [--desired-archive <system>/<service>] [--desired-watch] \
             [--desired-reread-ms <ms>] [--desired-hold] \
             <system>/<service>|@hostid.v1/<service> <contract.toml>..."
                .to_owned()
        };
        let mut o = Self {
            connect: None,
            hostid_root: None,
            hostid_ephemeral: false,
            health: false,
            health_status: None,
            health_checks: Vec::new(),
            tokenless: Vec::new(),
            clock_reference: None,
            clock_offset_ms: 0,
            cardinality: Vec::new(),
            config: false,
            config_floor_s: 10,
            config_apply_after_ms: 1000,
            config_persist: true,
            alarms: false,
            alarms_record: None,
            alarms_archive: None,
            alarms_rules: None,
            telemetry: false,
            desired_author: false,
            desired_record: None,
            desired_window_s: None,
            desired_target: None,
            desired_unbound: false,
            desired_archive: None,
            desired_watch: false,
            desired_reread_ms: None,
            desired_hold: false,
            address: String::new(),
            contracts: Vec::new(),
        };
        let mut rest = args;
        loop {
            match rest {
                [flag, value, tail @ ..] if flag == "--connect" => {
                    o.connect = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--hostid-root" => {
                    o.hostid_root = Some(PathBuf::from(value));
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--hostid-ephemeral" => {
                    o.hostid_ephemeral = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--health" => {
                    o.health = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--health-status" => {
                    let (level, reason) = value.split_once(':').unwrap_or((value, ""));
                    let Some(Some(level)) = parse_level(level) else {
                        return Err(format!(
                            "--health-status {value:?}: OK, DEGRADED or FAILED; a status is \
                             UNSPECIFIED only by default, at start (health.v1 §2.6)"
                        ));
                    };
                    o.health = true;
                    o.health_status = Some((level, reason.to_owned()));
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--health-check" => {
                    let parsed = value.split_once('=').and_then(|(name, l)| {
                        let (level, detail) = l.split_once(':').unwrap_or((l, ""));
                        Some((name.to_owned(), parse_level(level)?, detail.to_owned()))
                    });
                    let Some(check) = parsed else {
                        return Err(format!(
                            "--health-check {value:?}: <name>=<LEVEL>[:<detail>], LEVEL one of \
                             OK, DEGRADED, FAILED, UNSPECIFIED"
                        ));
                    };
                    o.health = true;
                    o.health_checks.push(check);
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--tokenless" => {
                    o.tokenless.push(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--clock-reference" => {
                    o.clock_reference = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--clock-offset-ms" => {
                    o.clock_offset_ms = value
                        .parse()
                        .map_err(|e| format!("--clock-offset-ms {value:?}: {e}"))?;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--cardinality" => {
                    let parsed = value.split_once(':').and_then(|(iface, r)| {
                        let (resource, n) = r.rsplit_once('=')?;
                        Some((iface.to_owned(), resource.to_owned(), n.parse().ok()?))
                    });
                    let Some(c) = parsed else {
                        return Err(format!(
                            "--cardinality {value:?}: <iface>:<resource>=<n>, as \
                             budget.v1:state/items/{{item}}=2"
                        ));
                    };
                    o.cardinality.push(c);
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--config" => {
                    o.config = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--config-floor" => {
                    o.config_floor_s = value
                        .parse()
                        .map_err(|e| format!("--config-floor {value:?}: {e}"))?;
                    o.config = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--config-apply-after-ms" => {
                    o.config_apply_after_ms = value
                        .parse()
                        .map_err(|e| format!("--config-apply-after-ms {value:?}: {e}"))?;
                    o.config = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--config-no-persist" => {
                    o.config_persist = false;
                    o.config = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--alarms" => {
                    o.alarms = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--alarms-record" => {
                    o.alarms_record = Some(PathBuf::from(value));
                    o.alarms = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--alarms-archive" => {
                    value
                        .parse::<zenkey::model::grammar::Addr>()
                        .map_err(|e| format!("--alarms-archive {value:?}: {e}"))?;
                    o.alarms_archive = Some(value.clone());
                    o.alarms = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--alarms-rules" => {
                    o.alarms_rules = Some(
                        value
                            .split(',')
                            .filter(|r| !r.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    );
                    o.alarms = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--telemetry" => {
                    o.telemetry = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--desired-author" => {
                    o.desired_author = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--desired-record" => {
                    o.desired_record = Some(PathBuf::from(value));
                    o.desired_author = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--desired-window" => {
                    o.desired_window_s = Some(
                        value
                            .parse()
                            .map_err(|e| format!("--desired-window {value:?}: {e}"))?,
                    );
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--desired-target" => {
                    value
                        .parse::<zenkey::model::grammar::Addr>()
                        .map_err(|e| format!("--desired-target {value:?}: {e}"))?;
                    o.desired_target = Some(value.clone());
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--desired-unbound" => {
                    o.desired_unbound = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--desired-archive" => {
                    value
                        .parse::<zenkey::model::grammar::Addr>()
                        .map_err(|e| format!("--desired-archive {value:?}: {e}"))?;
                    o.desired_archive = Some(value.clone());
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--desired-watch" => {
                    o.desired_watch = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--desired-reread-ms" => {
                    o.desired_reread_ms = Some(
                        value
                            .parse()
                            .map_err(|e| format!("--desired-reread-ms {value:?}: {e}"))?,
                    );
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--desired-hold" => {
                    o.desired_hold = true;
                    rest = tail;
                }
                [flag, ..] if flag.starts_with("--") => return Err(usage()),
                _ => break,
            }
        }
        let [address, files @ ..] = rest else {
            return Err(usage());
        };
        if o.desired_target.is_some() && o.desired_unbound {
            return Err(
                "--desired-target and --desired-unbound: a role is bound or not".to_owned(),
            );
        }
        let target = o.desired_target.is_some() || o.desired_unbound;
        if !target
            && (o.desired_archive.is_some()
                || o.desired_watch
                || o.desired_hold
                || o.desired_reread_ms.is_some())
        {
            return Err(
                "--desired-archive, --desired-watch, --desired-reread-ms and --desired-hold are \
                 the target's: give --desired-target or --desired-unbound"
                    .to_owned(),
            );
        }
        o.address = address.clone();
        o.contracts = files.iter().map(PathBuf::from).collect();
        Ok(o)
    }
}

/// Brings the owner up, says each output line through `say`, and runs until
/// `until` resolves.
pub async fn run(
    opts: Options,
    say: impl Fn(String),
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let (_, none) = flume::unbounded();
    run_with(opts, say, none, until).await
}

/// [`run`], answering each line of `commands` (the module doc's) through
/// `say` while it runs. When `until` resolves, the lines already sent are
/// answered first.
pub async fn run_with(
    mut opts: Options,
    say: impl Fn(String),
    commands: flume::Receiver<String>,
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let mut address: Address = opts.address.parse()?;
    if let Address::HostId { ephemeral, .. } = &mut address {
        *ephemeral = opts.hostid_ephemeral;
    }
    let mut config = ServiceConfig::at(address);
    for t in &opts.tokenless {
        config = config.tokenless(t.parse()?);
    }
    config.clock_reference = opts.clock_reference.clone();
    let mut imps = Vec::new();
    let mut alarms_revision = None;
    // desired.v1 (#840): the scenarios' tasking.v1 and worker.v1, embedded,
    // unless a file given implements one.
    let target = opts.desired_target.is_some() || opts.desired_unbound;
    let mut tasking = None;
    let mut worker = None;
    for f in &opts.contracts {
        let l = load_path(f);
        let Some(c) = l.contract else {
            return Err(format!("{}:\n{}", f.display(), l.report).into());
        };
        for r in &c.resources {
            for cap in r.gate.iter().filter_map(|g| g.strip_prefix("capability:")) {
                config = config.capability(cap);
            }
        }
        let imp = Implementation::new(c);
        // The standard health.v1 is the runtime's to implement (#721).
        if imp.iface() == &health::iface()
            && imp.fingerprint().hex().to_string() == health::FINGERPRINT
        {
            opts.health = true;
            continue;
        }
        // So is the standard config.v1 (#768).
        if imp.iface() == &config_v1::iface()
            && imp.fingerprint().hex().to_string() == config_v1::FINGERPRINT
        {
            opts.config = true;
            continue;
        }
        // And alarms.v1 (#800): the standard one, or with --alarms another
        // revision of it.
        if imp.iface() == &alarms::iface() {
            if imp.fingerprint().hex().to_string() == alarms::FINGERPRINT {
                opts.alarms = true;
                continue;
            }
            if opts.alarms {
                alarms_revision = Some(imp);
                continue;
            }
        }
        if (opts.desired_author || target) && imp.iface().to_string() == "tasking.v1" {
            tasking = Some(imp);
            continue;
        }
        if target && imp.iface().to_string() == "worker.v1" {
            worker = Some(imp);
            continue;
        }
        imps.push(imp);
    }
    let desired_ctx = if opts.desired_author || target {
        let (t, w) = drive::contracts()?;
        Some(DesiredCtx {
            tasking: tasking.unwrap_or_else(|| Implementation::new(t)),
            worker: worker.unwrap_or_else(|| Implementation::new(w)),
            watch: None,
        })
    } else {
        None
    };
    config.tombstone_window_s = opts.desired_window_s;
    if let Some(author) = &opts.desired_target {
        // §2.3: one author, each target parameter its own identity.
        config = config.bind(drive::ROLE, &[author.as_str()]);
        if let Some(b) = config.bindings.get_mut(drive::ROLE) {
            b.params = [
                ("system".to_owned(), "self.system".to_owned()),
                ("service".to_owned(), "self.service".to_owned()),
            ]
            .into();
        }
    }
    // telemetry.v1 (#819): the scenarios' meter.v1, unless a file has it.
    if opts.telemetry && !imps.iter().any(|i| i.iface().to_string() == "meter.v1") {
        imps.push(Implementation::new(meter_contract()?));
    }

    // hostid.v1 §2.7: the system is minted before the session opens, and a
    // host without an id stops here, with nothing declared (§2.6).
    let under_root;
    let minter = match &opts.hostid_root {
        Some(root) => {
            under_root = HostIdMinter::new(HostIdSource::at(root));
            &under_root
        }
        None => HostIdMinter::global(),
    };
    config.resolve_with(minter)?;

    let mut z = zenoh::Config::default();
    z.insert_json5("scouting/multicast/enabled", "false")?;
    let (session, endpoint) = match &opts.connect {
        None => {
            z.insert_json5("mode", "\"router\"")?;
            z.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
            let session = zenoh::open(z).await?;
            let endpoint = session
                .info()
                .locators()
                .await
                .into_iter()
                .map(|l| l.to_string())
                .find(|l| l.starts_with("tcp/127.0.0.1:"))
                .ok_or("no loopback listener")?;
            say(format!("listening {endpoint}"));
            (session, endpoint)
        }
        Some(endpoint) => {
            z.insert_json5("mode", "\"client\"")?;
            z.insert_json5("connect/endpoints", &serde_json::to_string(&[endpoint])?)?;
            // §4.3: a session that serves state enables its HLC. A simulated
            // offset leaves it off (the module doc): the HLC would re-stamp
            // the offset clock's stamps before they leave.
            if opts.clock_offset_ms == 0 {
                z.insert_json5("timestamping/enabled", "true")?;
            }
            let session = zenoh::open(z).await?;
            say(format!("connected {endpoint}"));
            (session, endpoint.clone())
        }
    };
    // desired.v1 §2.5, core §8.1 (0.37): a watch session of the target's
    // own, a client of the same router, where it follows its author.
    let mut desired_ctx = desired_ctx;
    if let Some(d) = desired_ctx.as_mut()
        && opts.desired_watch
    {
        let mut w = zenoh::Config::default();
        w.insert_json5("scouting/multicast/enabled", "false")?;
        w.insert_json5("mode", "\"client\"")?;
        w.insert_json5("connect/endpoints", &serde_json::to_string(&[&endpoint])?)?;
        d.watch = Some(zenoh::open(w).await?);
    }

    // config.v1: the rf owner's startup, and its simulated device.
    let configured = opts.config.then(|| {
        let startup = config::rf_startup(
            true,
            opts.config_floor_s,
            Duration::from_millis(opts.config_apply_after_ms),
            opts.config_persist,
        );
        let device = Arc::new(config::SimDevice::new(&startup));
        (startup, device)
    });
    let ctx = Ctx {
        opts: &opts,
        session: &session,
        config,
        minter,
        imps,
        configured,
        alarms_revision,
        desired: desired_ctx,
    };
    let mut up = ctx.up(Vec::new()).await?;
    say(format!("ready {}", up.svc.instance_key()?));

    let mut until = std::pin::pin!(until);
    let mut open = true;
    loop {
        tokio::select! {
            () = &mut until => break,
            line = commands.recv_async(), if open => match line {
                Ok(line) => {
                    let answer;
                    (answer, up) = ctx.command(&line, up).await?;
                    say(answer);
                }
                Err(_) => open = false,
            },
        }
    }
    while let Ok(line) = commands.try_recv() {
        let answer;
        (answer, up) = ctx.command(&line, up).await?;
        say(answer);
    }
    up.close().await?;
    if let Some(w) = ctx.desired.as_ref().and_then(|d| d.watch.as_ref()) {
        w.close().await?;
    }
    session.close().await?;
    Ok(())
}

/// What every bring-up of the service is made from: the first, and each
/// `config restart`'s.
struct Ctx<'a> {
    opts: &'a Options,
    session: &'a zenoh::Session,
    config: ServiceConfig,
    minter: &'a HostIdMinter,
    imps: Vec<Implementation>,
    /// config.v1's startup and device, with `--config`.
    configured: Option<(Startup, Arc<config::SimDevice>)>,
    /// The revision of alarms.v1 among the files, with `--alarms`; `None`:
    /// the standard one.
    alarms_revision: Option<Implementation>,
    /// desired.v1's contracts and the target's watch session, with a
    /// `--desired-*` option.
    desired: Option<DesiredCtx>,
}

/// desired.v1's part of every bring-up (#840).
struct DesiredCtx {
    /// `tasking.v1`: the author's interface, and what the target is
    /// compiled against (R4).
    tasking: Implementation,
    /// `worker.v1`: the target's.
    worker: Implementation,
    /// The target's watch session, with `--desired-watch`.
    watch: Option<zenoh::Session>,
}

/// One running instance of the service, and what it holds.
struct Up {
    svc: Service,
    servers: Vec<OperationServer>,
    _states: Vec<StateWriter>,
    health: Option<Health>,
    members: Members,
    config: Option<Configuration>,
    alarms: Option<Alarms>,
    /// desired.v1's author, its target and the target's drive.
    author: Option<desired::Author>,
    target: Option<desired::Target>,
    drive: Option<Arc<drive::Drive>>,
}

impl Up {
    async fn close(self) -> Result<(), BoxError> {
        drop(self.members);
        self.svc.close().await?;
        drop(self.servers);
        Ok(())
    }
}

impl Ctx<'_> {
    /// Answers one command line (the module doc's): `config restart` here,
    /// since it replaces the instance; every other one by [`command`].
    async fn command(&self, line: &str, mut up: Up) -> Result<(String, Up), BoxError> {
        if line.split_whitespace().next() == Some("telemetry") {
            let words: Vec<&str> = line.split_whitespace().collect();
            let answer = telemetry_command(line, &mut up, self.session)
                .await
                .unwrap_or_else(|why| format!("refused {}: {why}", words.join(" ")));
            return Ok((answer, up));
        }
        if line.split_whitespace().collect::<Vec<_>>() != ["config", "restart"] {
            let answer = command(line, &mut up, self.configured.as_ref()).await;
            return Ok((answer, up));
        }
        let (Some(cfg), Some((startup, device))) = (up.config.clone(), &self.configured) else {
            let refused = format!("refused {line}: config.v1 is not implemented here (--config)");
            return Ok((refused, up));
        };
        // §2.12: the old instance goes first, the device runs its startup
        // values again, and the new instance starts from them, with the
        // persisted layer and the change events of the last hour (§2.17).
        let events = cfg.published().await;
        // The desired document's binding is the deployment's, here this
        // process's stdin, and outlives the instance (text 0.3, F-114).
        let bound = cfg.desired_document().await;
        up.close().await?;
        device.restart(startup);
        let up = self.up(events).await?;
        if let (Some(doc), Some(next)) = (bound, &up.config) {
            next.set_desired(doc).await;
        }
        let answer = format!("config restarted {}", up.svc.instance_key()?);
        Ok((answer, up))
    }

    /// Brings the service up: every contract's resources, `health.v1` and
    /// `config.v1` as asked, then `start()`. `events` are config.v1's
    /// change events of the last hour (§2.17).
    async fn up(&self, events: Vec<SystemTime>) -> Result<Up, BoxError> {
        let opts = self.opts;
        let mut b = ServiceBuilder::with_hostid(self.session, self.config.clone(), self.minter);
        b.minter().simulate_offset(opts.clock_offset_ms);
        let mut states = Vec::new();
        let mut servers = Vec::new();
        let mut members = Members::default();
        let none = Bindings::new();
        for imp in self.imps.iter().cloned() {
            let iface = imp.iface().clone();
            let resources = imp.contract().resources.clone();
            members
                .contracts
                .insert(iface.clone(), imp.contract().clone());
            b.implement(imp)?;
            for r in resources {
                let name = zenkey::implementation::resource_name(&r);
                let raw = |t: &TypeId| matches!(t, TypeId::Raw { .. });
                match &r.body {
                    Body::Data(d)
                        if r.kind == Kind::State && !r.template.has_params() && raw(&d.type_) =>
                    {
                        states.push(b.declare_state_writer(&iface, &name, &none).await?);
                    }
                    Body::Operation(o) => {
                        let echo = raw(&o.request) && raw(&o.response);
                        // Over the whole template when it has parameters (§5.1),
                        // else on the operation's one key.
                        let values = (!r.template.has_params()).then_some(&none);
                        servers.push(
                            b.serve(&iface, &name, values, move |call| async move {
                                if !echo {
                                    // §5.2: `app` from any operation, and no detail
                                    // where there is no `error` type to carry one.
                                    return Err(OpError::app_without_detail(
                                        "this interop owner decodes no schema",
                                    ));
                                }
                                let body = call
                                    .payload()
                                    .map(|p| p.to_bytes().into_owned())
                                    .unwrap_or_default();
                                call.reply(body)
                                    .await
                                    .map_err(|e| OpError::internal(e.to_string()))
                            })
                            .await?,
                        );
                    }
                    Body::Data(_)
                        if !r.template.has_params()
                            && matches!(r.kind, Kind::Stream | Kind::State)
                            && telemetry_model::declaration_of(&members.contracts[&iface], &r)
                                .declared()
                                .is_some() =>
                    {
                        // telemetry.v1 (#819): a metric member's writer is
                        // declared before start, so that its publisher knows
                        // its subscribers before the first `telemetry put`.
                        let key = b.key(&iface, &name, &none)?.to_string();
                        let held = if r.kind == Kind::State {
                            Held::State(b.declare_state_writer(&iface, &name, &none).await?)
                        } else {
                            Held::Stream(Box::new(b.declare_writer(&iface, &name, &none).await?))
                        };
                        members.writers.insert(key, held);
                    }
                    _ => {
                        b.expose(&iface, &name)?;
                    }
                }
            }
        }
        // Core §3.3: a template's bound lowered in the descriptor, which the
        // runtime then keeps (§2.7). alarms.v1's goes in its startup, set
        // before the adoption.
        let alarms_iface = alarms::iface().to_string();
        for (iface, resource, n) in &opts.cardinality {
            if opts.alarms && *iface == alarms_iface {
                continue;
            }
            b.cardinality(&iface.parse()?, resource, *n)?;
        }
        // health.v1 §2.3: its status and checks before the tokens; the status
        // is UNSPECIFIED, "starting", when none is given.
        let health = if opts.health {
            let h = b.health().await?;
            if let Some((level, reason)) = &opts.health_status {
                h.set_status(*level, reason.as_str()).await?;
            }
            for (name, level, detail) in &opts.health_checks {
                match level {
                    Some(l) => h.set_check(name, *l, detail.as_str()).await?,
                    None => h.set_check_unknown(name, detail.as_str()).await?,
                };
            }
            Some(h)
        } else {
            None
        };
        // config.v1 §2.3: the view and the status are put here, before the
        // tokens; the owner starts from its startup values and the persisted
        // layer the device holds (§2.12).
        let config = match &self.configured {
            Some((startup, device)) => {
                let next = device.next_startup(startup, events);
                Some(
                    b.config_v1(next, Arc::clone(device) as Arc<dyn config_v1::Device>)
                        .await?,
                )
            }
            None => None,
        };
        // alarms.v1 §2.9: the previous instance's alarms taken over here,
        // before the tokens.
        let alarms = if opts.alarms {
            let mut startup = alarms::Startup::default();
            if let Some(path) = &opts.alarms_record {
                startup = startup.with_record(Arc::new(FileRecord::new(path)));
            }
            if let Some(a) = &opts.alarms_archive {
                startup = startup.with_archive(a.parse()?, Duration::from_secs(2));
            }
            if let Some(rules) = &opts.alarms_rules {
                startup = startup.evaluating(rules.iter().cloned());
            }
            for (iface, resource, n) in &opts.cardinality {
                if *iface == alarms_iface {
                    if resource != alarms::ALARMS {
                        return Err(format!(
                            "--cardinality {iface}:{resource}: alarms.v1's template is {}",
                            alarms::ALARMS
                        )
                        .into());
                    }
                    startup = startup.with_cardinality(*n);
                }
            }
            let imp = self
                .alarms_revision
                .clone()
                .unwrap_or_else(alarms::implementation);
            Some(b.alarms_with(imp, startup).await?)
        } else {
            None
        };
        // desired.v1 (#840): the author takes over its record before the
        // tokens (§2.11); the target puts its baseline report before them
        // (§2.10), and reads from start() on.
        let (mut author, mut target, mut drive_) = (None, None, None);
        if let Some(d) = &self.desired {
            let none = Bindings::new();
            if opts.desired_author {
                let iface = d.tasking.iface().clone();
                b.implement(d.tasking.clone())?;
                let mut st = AuthorStartup::default();
                if let Some(path) = &opts.desired_record {
                    st = st.with_record(Arc::new(desired::FileRecord::new(path)));
                }
                author = Some(b.desired_author(&iface, drive::TASKS, st).await?);
            }
            if opts.desired_target.is_some() || opts.desired_unbound {
                let wi = d.worker.iface().clone();
                b.implement(d.worker.clone())?;
                let run = b.declare_state_writer(&wi, drive::RUNNING, &none).await?;
                run.put_value(&drive::baseline()).await?;
                let dr = Arc::new(drive::Drive::new(run));
                if opts.desired_hold {
                    dr.hold();
                }
                let mut st = TargetStartup::default();
                if let Some(a) = &opts.desired_archive {
                    st = st.with_archive(a.parse()?, Duration::from_secs(1));
                }
                if let Some(ms) = opts.desired_reread_ms {
                    st = st.with_reread(Duration::from_millis(ms));
                }
                st.watch = d.watch.clone();
                target = Some(
                    b.desired_target(
                        drive::ROLE,
                        &d.tasking,
                        drive::TASKS,
                        ReportAt::new(wi, drive::REPORT),
                        Arc::clone(&dr) as Arc<dyn desired::Handler>,
                        st,
                    )
                    .await?,
                );
                drive_ = Some(dr);
            }
        }
        // §8.2 "State values" (F-68): the value is put before the tokens, so a
        // GET made the moment they appear finds it.
        for w in &states {
            w.put("ok").await?;
        }
        let svc = b.start().await?;
        Ok(Up {
            svc,
            servers,
            _states: states,
            health,
            members,
            config,
            alarms,
            author,
            target,
            drive: drive_,
        })
    }
}

/// The status as a command's answer.
fn describe(e: &Effective) -> String {
    format!(
        "health status={} declared={} raised_by={}",
        level_name(e.level),
        e.declared.map_or_else(|| "-".to_owned(), |l| l.to_string()),
        e.raised_by.as_deref().unwrap_or("-")
    )
}

/// A member's writer, kept for its next write.
enum Held {
    State(StateWriter),
    Stream(Box<Writer>),
    Event(EventWriter),
}

/// The members `put`, `delete` and `fill` write (core §2.7): each
/// implemented contract, and a writer per member written.
#[derive(Default)]
struct Members {
    contracts: HashMap<IfaceId, Contract>,
    writers: HashMap<String, Held>,
}

/// The template's parameters bound to `values`, in order: a rest parameter
/// takes its value's `/`-separated chunks.
fn bind(r: &Resource, values: &[String]) -> Result<Bindings, String> {
    let params: Vec<(String, bool)> = r
        .template
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Param(n) => Some((n.clone(), false)),
            Segment::Rest(n) => Some((n.clone(), true)),
            Segment::Literal(_) => None,
        })
        .collect();
    if params.len() != values.len() {
        return Err(format!(
            "{} takes {} value(s), one per parameter, and {} were given",
            r.template,
            params.len(),
            values.len()
        ));
    }
    Ok(params
        .into_iter()
        .zip(values)
        .map(|((name, rest), v)| {
            let chunks = if rest {
                v.split('/').map(str::to_owned).collect()
            } else {
                vec![v.clone()]
            };
            (name, chunks)
        })
        .collect())
}

impl Members {
    /// The writer of the member `key` of `r`, made on its first write: one
    /// per member, which `put`, `fill` and `telemetry put` share.
    async fn held(
        &mut self,
        svc: &mut Service,
        iface: &IfaceId,
        r: &Resource,
        bound: &Bindings,
        key: &str,
    ) -> Result<&Held, String> {
        if !self.writers.contains_key(key) {
            let resource = zenkey::implementation::resource_name(r);
            let err = |e: zenkey::Error| e.to_string();
            let held = match r.kind {
                Kind::State => Held::State(
                    svc.state_writer(iface, &resource, bound)
                        .await
                        .map_err(err)?,
                ),
                Kind::Stream => Held::Stream(Box::new(
                    svc.writer(iface, &resource, bound).await.map_err(err)?,
                )),
                Kind::Event => Held::Event(svc.event_writer(iface, &resource, bound).map_err(err)?),
                Kind::Operation => return Err(format!("{resource:?} is an operation")),
            };
            self.writers.insert(key.to_owned(), held);
        }
        Ok(self.writers.get(key).expect("held"))
    }

    /// The metric resource `name` names (the module doc's `telemetry`
    /// commands): its interface and resource, refused unless it is one
    /// (telemetry.v1 §2.1) without template parameters.
    fn metric(&self, name: &str) -> Result<(IfaceId, Resource), String> {
        let (iface, res) = match name.split_once(':') {
            Some((i, r)) => (Some(i.parse::<IfaceId>().map_err(|e| format!("{e}"))?), r),
            None => (None, name),
        };
        let mut found: Vec<(&IfaceId, &Contract, &Resource)> = self
            .contracts
            .iter()
            .filter(|(i, _)| iface.as_ref().is_none_or(|want| want == *i))
            .flat_map(|(i, c)| c.resources.iter().map(move |r| (i, c, r)))
            .filter(|(_, _, r)| {
                zenkey::implementation::resource_name(r) == res || r.template.as_str() == res
            })
            .collect();
        found.sort_by_key(|(i, _, _)| i.to_string());
        let (i, c, r) = match found.as_slice() {
            [] => return Err(format!("no resource {name:?} is implemented here")),
            [one] => *one,
            many => {
                let names: Vec<String> = many
                    .iter()
                    .map(|(i, _, r)| format!("{i}:{}", zenkey::implementation::resource_name(r)))
                    .collect();
                return Err(format!(
                    "{name:?} is ambiguous: {}; name one as <iface>:<resource>",
                    names.join(", ")
                ));
            }
        };
        match telemetry_model::declaration_of(c, r) {
            telemetry_model::Declaration::Declared(_) => {}
            other => {
                return Err(format!(
                    "{i} {:?} is not a metric resource (telemetry.v1 §2.1): its declaration is {}",
                    zenkey::implementation::resource_name(r),
                    other.as_str()
                ));
            }
        }
        if r.template.has_params() {
            return Err(format!(
                "{i} {:?} has template parameters, which the telemetry commands do not bind",
                zenkey::implementation::resource_name(r)
            ));
        }
        Ok((i.clone(), r.clone()))
    }

    /// One write on the member of `resource` the values bind (§2.7), or why
    /// not; for `delete`, the state member's delete. Answers its key.
    async fn write(
        &mut self,
        svc: &mut Service,
        iface: &str,
        resource: &str,
        values: &[String],
        delete: bool,
    ) -> Result<String, String> {
        let iface: IfaceId = iface.parse().map_err(|e| format!("{e}"))?;
        let r = self
            .contracts
            .get(&iface)
            .ok_or_else(|| format!("{iface} is not implemented here"))?
            .resources
            .iter()
            .find(|r| zenkey::implementation::resource_name(r) == resource)
            .ok_or_else(|| format!("{iface} has no resource {resource:?}"))?
            .clone();
        let bound = bind(&r, values)?;
        let key = svc
            .key(&iface, resource, &bound)
            .map_err(|e| e.to_string())?
            .to_string();
        let err = |e: zenkey::Error| e.to_string();
        match (self.held(svc, &iface, &r, &bound, &key).await?, delete) {
            (Held::State(w), false) => w.put("ok").await.map(|_| key).map_err(err),
            (Held::State(w), true) => w.delete().await.map(|_| key).map_err(err),
            (Held::Stream(w), false) => w.put("ok").await.map(|()| key).map_err(err),
            (Held::Event(w), false) => w.put("ok").await.map_err(err),
            (_, true) => Err(format!(
                "{resource:?} is not state: only a state is deleted"
            )),
        }
    }
}

/// The simulated device's state, as a `config` command answers it: a
/// sensitive value written `"<redacted>"`, as everything this owner says is
/// (config.v1 §2.15).
fn device_line(d: &config::SimDevice, schema: &config_v1::Schema) -> String {
    d.with(|s| {
        let mut held = serde_json::to_value(&s.held).unwrap_or_default();
        for g in &schema.groups {
            for p in g.parameters.iter().filter(|p| p.sensitive) {
                if let Some(v) = held.get_mut(&g.name).and_then(|g| g.get_mut(&p.name)) {
                    *v = serde_json::Value::String(zenkey::model::config::REDACTED.to_owned());
                }
            }
        }
        let mut next = Vec::new();
        if s.refuse_apply.is_some() {
            next.push("fail-apply");
        }
        match s.fail_undo {
            Some(config::UndoFailure::Held) => next.push("fail-undo"),
            Some(config::UndoFailure::Unknown) => next.push("fail-undo-unknown"),
            None => {}
        }
        if s.fail_persist.is_some() {
            next.push("fail-persist");
        }
        if s.block_persist.is_some() {
            next.push("block-persist");
        }
        let json = |v: &config_v1::Values| serde_json::to_string(v).unwrap_or_default();
        format!(
            "config device held={held} applies={} undos={} persisted={} next={}",
            s.applies.len(),
            s.undos.len(),
            s.persisted.as_ref().map_or_else(|| "none".to_owned(), json),
            if next.is_empty() {
                "-".to_owned()
            } else {
                next.join(",")
            }
        )
    })
}

/// Answers a `config` command but `restart` (the module doc's).
async fn config_command(
    words: &[&str],
    cfg: &Configuration,
    device: &config::SimDevice,
) -> Result<String, String> {
    let words_from = |n: usize| words.get(n..).map(|w| w.join(" ")).unwrap_or_default();
    let said = |n: usize, default: &str| {
        let w = words_from(n);
        if w.is_empty() { default.to_owned() } else { w }
    };
    match words {
        ["config", "fail-apply", ..] => {
            let w = said(2, "the simulated device refused the apply");
            device.with(|s| s.refuse_apply = Some(w));
        }
        ["config", "fail-undo"] => device.with(|s| s.fail_undo = Some(config::UndoFailure::Held)),
        ["config", "fail-undo", "unknown"] => {
            device.with(|s| s.fail_undo = Some(config::UndoFailure::Unknown));
        }
        ["config", "fail-persist", ..] => {
            let w = said(2, "the simulated persisted layer failed its write");
            device.with(|s| s.fail_persist = Some(w));
        }
        ["config", "block-persist", ..] => {
            let w = said(2, "the simulated persisted layer is read-only");
            device.with(|s| s.block_persist = Some(w));
        }
        ["config", "clear"] => device.with(|s| {
            s.refuse_apply = None;
            s.fail_undo = None;
            s.fail_persist = None;
            s.block_persist = None;
        }),
        ["config", "device"] => {}
        ["config", "desired", ..] => {
            let doc: DesiredConfig = serde_json::from_str(&words_from(2))
                .map_err(|e| format!("not a DesiredConfig: {e}"))?;
            let revision = doc.revision;
            cfg.set_desired(doc).await;
            return Ok(format!("config desired {revision}"));
        }
        ["config", "offset", ms] => {
            let ms: i64 = ms.parse().map_err(|e| format!("{e}"))?;
            cfg.simulate_offset(ms);
            return Ok(format!("config offset {ms}"));
        }
        _ => {
            return Err(
                "usage: config fail-apply [<words>] | fail-undo [unknown] | fail-persist [<words>] | \
                 block-persist [<words>] | clear | device | desired <json> | offset <ms> | restart"
                    .to_owned(),
            );
        }
    }
    Ok(device_line(device, cfg.schema()))
}

/// The raw remainder of `line` after its first `n` words: a JSON argument
/// keeps its own spacing.
fn after_words(line: &str, n: usize) -> &str {
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    rest
}

/// A document on one line.
fn one_line(doc: &alarms::Alarm) -> String {
    serde_json::to_string(doc).unwrap_or_default()
}

/// A list as `adoption` answers it: comma-separated, `-` when empty.
fn listed(items: impl IntoIterator<Item = String>) -> String {
    let v: Vec<String> = items.into_iter().collect();
    if v.is_empty() {
        "-".to_owned()
    } else {
        v.join(",")
    }
}

/// Answers an `alarm` command (the module doc's).
async fn alarm_command(words: &[&str], rest: &str, a: &Alarms) -> Result<String, String> {
    match words {
        ["alarm", "raise", _, ..] => {
            let c: Condition = serde_json::from_str(rest).map_err(|e| {
                format!("not a condition {{rule, severity, summary, labels?, category?}}: {e}")
            })?;
            let done = a.raise(c).await.map_err(|e| e.to_string())?;
            Ok(format!(
                "alarm {} {} {}",
                done.act.as_str(),
                done.alert_key,
                one_line(&done.document)
            ))
        }
        ["alarm", "resolve", key] => match a.resolve(key).await.map_err(|e| e.to_string())? {
            Some(done) => Ok(format!(
                "alarm resolved {} {}",
                done.alert_key,
                one_line(&done.document)
            )),
            None => Ok(format!("alarm none {key}")),
        },
        ["alarm", "confirm", key, on @ ("on" | "off")] => {
            let on_ = *on == "on";
            if *key == "all" {
                a.set_confirming_all(on_).await;
            } else if !a.set_confirming(key, on_).await {
                return Err(match a.get(key).await {
                    Some(_) => format!(
                        "the alarm on {key} is held resolved, its delete due, and a resolved \
                         alarm is never confirmed (alarms.v1 §2.7)"
                    ),
                    None => format!("no alarm is held on {key}"),
                });
            }
            Ok(format!("alarm confirm {key} {on}"))
        }
        ["alarm", "list"] => {
            let held: serde_json::Map<String, serde_json::Value> = a
                .held()
                .await
                .into_iter()
                .map(|(k, d)| (k, serde_json::to_value(d).unwrap_or_default()))
                .collect();
            Ok(format!("alarm held {}", serde_json::Value::Object(held)))
        }
        ["alarm", "adoption"] => {
            let r = a.adoption();
            Ok(format!(
                "alarm adoption source={} adopted={} retired={} resolved={} over_bound={}",
                r.source.as_str(),
                listed(r.adopted),
                listed(
                    r.retired
                        .into_iter()
                        .map(|(k, why)| format!("{k}:{}", why.as_str()))
                ),
                listed(r.resolved),
                listed(r.over_bound)
            ))
        }
        _ => Err(
            "usage: alarm raise <json> | resolve <alert_key> | confirm <alert_key>|all on|off | \
             list | adoption"
                .to_owned(),
        ),
    }
}

/// `meter.v1`, the telemetry.v1 scenarios' contract, as the runtime keeps
/// it (`zenkey/tests/contracts/`), with the profile's schema: embedded, so
/// `--telemetry` needs no file.
const METER_TOML: &str = include_str!("../tests/contracts/meter.v1.toml");
const METER_JSON: &str = include_str!("../tests/contracts/meter.json");
const TELEMETRY_JSON: &str = include_str!("../../spec/profiles/telemetry/telemetry.v1.json");

/// `meter.v1` loaded from the embedded files, laid out as the source tree
/// lays them out, so the contract lists its schemas by the same paths and
/// keeps its fingerprint.
fn meter_contract() -> Result<Contract, BoxError> {
    let root = tempfile::tempdir()?;
    let contracts = root.path().join("zenkey/tests/contracts");
    let profile = root.path().join("spec/profiles/telemetry");
    std::fs::create_dir_all(&contracts)?;
    std::fs::create_dir_all(&profile)?;
    std::fs::write(contracts.join("meter.v1.toml"), METER_TOML)?;
    std::fs::write(contracts.join("meter.json"), METER_JSON)?;
    std::fs::write(profile.join("telemetry.v1.json"), TELEMETRY_JSON)?;
    let l = load_path(&contracts.join("meter.v1.toml"));
    l.contract
        .ok_or_else(|| format!("meter.v1, embedded:\n{}", l.report).into())
}

/// The payload and the optional attachment of a `telemetry put|raw`: two
/// JSON texts, each as written.
fn json_args(rest: &str) -> Result<(&str, Option<&str>), String> {
    let mut values = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    match values.next() {
        Some(Ok(_)) => {}
        Some(Err(e)) => return Err(format!("the payload is not JSON: {e}")),
        None => return Err("a payload, as JSON".to_owned()),
    }
    let end = values.byte_offset();
    let (payload, tail) = (rest[..end].trim(), rest[end..].trim());
    if tail.is_empty() {
        return Ok((payload, None));
    }
    serde_json::from_str::<serde_json::Value>(tail)
        .map_err(|e| format!("the attachment is not one JSON text: {e}"))?;
    Ok((payload, Some(tail)))
}

/// A JSON text as a resource carries it: as written for JSON, the same
/// data item for CBOR.
fn as_wire(text: &str, wire: Option<WireEncoding>) -> Result<Vec<u8>, String> {
    match wire {
        Some(WireEncoding::Cbor) => {
            let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            ciborium::into_writer(&v, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        _ => Ok(text.as_bytes().to_vec()),
    }
}

/// A writer's refusal as `telemetry put` answers it (the module doc's).
fn telemetry_refusal(e: &zenkey::Error) -> String {
    use zenkey::telemetry::Reason;
    let zenkey::Error::Telemetry { reason, .. } = e else {
        return e.to_string();
    };
    let why = match &**reason {
        Reason::DoesNot(problems) => format!(
            "does_not {}",
            problems
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
        Reason::Undecodable(_) => "undecodable".to_owned(),
        Reason::Epoch { given, current } => format!(
            "epoch given={} current={current}",
            serde_json::to_string(given).unwrap_or_default()
        ),
        Reason::Decreased { epoch, .. } => format!("decreased epoch={epoch}"),
    };
    format!("telemetry.v1 {why}: {e}")
}

/// Answers a `telemetry` command (the module doc's).
/// A list as `desired adoption` answers it: comma-separated, `-` when
/// empty.
fn desired_listed(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(",")
    }
}

/// The `desired` commands (the module doc's), each answered by one line.
async fn desired_command(words: &[&str], rest: &str, up: &mut Up) -> Result<String, String> {
    let author = || {
        up.author.clone().ok_or_else(|| {
            "this owner authors no desired.v1 document (--desired-author)".to_owned()
        })
    };
    let target = || {
        up.target
            .clone()
            .ok_or_else(|| "this owner is no desired.v1 target (--desired-target)".to_owned())
    };
    let drive = || {
        up.drive
            .clone()
            .ok_or_else(|| "this owner is no desired.v1 target (--desired-target)".to_owned())
    };
    let err = |e: zenkey::Error| e.to_string();
    let stamp = |ts: &zenoh::time::Timestamp| desired::stamp(ts).to_string();
    match words {
        ["desired", "put", t, _, ..] => {
            let a = author()?;
            let doc: serde_json::Value =
                serde_json::from_str(rest).map_err(|e| format!("the document is not JSON: {e}"))?;
            let key = a.key(t).map_err(err)?;
            let bytes = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
            match a.put(t, bytes).await.map_err(err)? {
                Written::Put(ts) => Ok(format!("desired put {key} {}", stamp(&ts))),
                _ => Ok(format!("desired unchanged {key}")),
            }
        }
        ["desired", "delete", t] => {
            let a = author()?;
            let key = a.key(t).map_err(err)?;
            match a.delete(t).await.map_err(err)? {
                Written::Deleted(ts) => Ok(format!("desired deleted {key} {}", stamp(&ts))),
                _ => Ok(format!("desired none {key}")),
            }
        }
        ["desired", "documents"] => {
            let docs = author()?.documents().await;
            let map: serde_json::Map<String, serde_json::Value> = docs
                .into_iter()
                .map(|(t, p)| {
                    let v = serde_json::from_slice(&p).unwrap_or(serde_json::Value::Null);
                    (t, v)
                })
                .collect();
            Ok(format!(
                "desired documents {}",
                serde_json::Value::Object(map)
            ))
        }
        ["desired", "adoption"] => {
            let a = author()?.adoption();
            let failed: Vec<String> = a.failed.iter().map(|(t, _)| t.clone()).collect();
            Ok(format!(
                "desired adoption found={} reput={} redeleted={} expired={} failed={}",
                a.found.as_str(),
                desired_listed(&a.reput),
                desired_listed(&a.redeleted),
                desired_listed(&a.expired),
                desired_listed(&failed)
            ))
        }
        ["desired", "new-epoch"] => {
            up.svc.new_epoch().await.map_err(err)?;
            Ok(format!(
                "desired new-epoch {}",
                up.svc.instance_key().map_err(err)?
            ))
        }
        ["desired", "report"] => {
            let r = target()?.report().await;
            Ok(format!("desired report {}", r.to_json()))
        }
        ["desired", "running"] => {
            let d = drive()?;
            Ok(format!("desired running {}", d.with(|s| s.running.clone())))
        }
        ["desired", "fail-next"] => {
            drive()?.fail_next();
            Ok("desired fail-next".to_owned())
        }
        ["desired", "refuse-next", code, pointer @ ..] if pointer.len() <= 1 => {
            if code.is_empty() || code.chars().count() > zenkey::model::desired::MAX_CODE {
                return Err("a refusal's code is 1 to 64 characters (desired.v1 §3)".to_owned());
            }
            drive()?.refuse_next(desired::Refusal {
                code: (*code).to_owned(),
                pointer: pointer.first().map(|p| (*p).to_owned()),
                detail: Some("refused on the runner's word".to_owned()),
            });
            Ok(format!("desired refuse-next {code}"))
        }
        ["desired", "hold"] => {
            drive()?.hold();
            Ok("desired hold".to_owned())
        }
        ["desired", "release"] => {
            drive()?.release();
            Ok("desired release".to_owned())
        }
        ["desired", "reread", period] => {
            let t = target()?;
            if *period == "off" {
                t.set_reread(None);
            } else {
                let ms: u64 = period
                    .parse()
                    .map_err(|e| format!("reread <ms>|off: {e}"))?;
                t.set_reread(Some(Duration::from_millis(ms)));
            }
            Ok(format!("desired reread {period}"))
        }
        ["desired", "acts"] => {
            let t = target()?;
            let held = drive()?.with(|s| s.held);
            Ok(format!(
                "desired acts {} seeds={} held={held}",
                t.acts(),
                t.seeds()
            ))
        }
        _ => Err(
            "usage: desired put <target> <json> | delete <target> | documents | adoption | \
             new-epoch | report | running | fail-next | refuse-next <code> [<pointer>] | hold | \
             release | reread <ms>|off | acts"
                .to_owned(),
        ),
    }
}

async fn telemetry_command(
    line: &str,
    up: &mut Up,
    session: &zenoh::Session,
) -> Result<String, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Up { svc, members, .. } = up;
    match words.as_slice() {
        ["telemetry", "instance"] => Ok(format!("telemetry instance {}", svc.instance())),
        ["telemetry", "new-epoch"] => {
            let id = svc.new_epoch().await.map_err(|e| e.to_string())?;
            Ok(format!("telemetry new-epoch {id}"))
        }
        ["telemetry", verb @ ("put" | "raw"), name, _, ..] => {
            let (payload, attachment) = json_args(after_words(line, 3))?;
            let (iface, r) = members.metric(name)?;
            let Body::Data(d) = &r.body else {
                return Err(format!("{name:?} is an operation"));
            };
            let none = Bindings::new();
            let resource = zenkey::implementation::resource_name(&r);
            let key = svc
                .key(&iface, &resource, &none)
                .map_err(|e| e.to_string())?
                .to_string();
            let bytes = as_wire(payload, d.encoding)?;
            let attached = attachment
                .map(|a| as_wire(a, d.attachment_encoding))
                .transpose()?;
            if *verb == "raw" {
                session
                    .put(&key, bytes)
                    .encoding(zenkey::writer::wire_encoding(&d.type_, d.encoding, &none))
                    .attachment(attached)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(format!("telemetry raw {key}"));
            }
            let (put, epoch) = match members.held(svc, &iface, &r, &none, &key).await? {
                Held::State(w) => (
                    w.put_with(bytes, attached).await.map(|_| ()),
                    w.writer().epoch(),
                ),
                Held::Stream(w) => (w.put_with(bytes, attached).await, w.epoch()),
                Held::Event(_) => return Err(format!("{name:?} is an event")),
            };
            put.map_err(|e| telemetry_refusal(&e))?;
            Ok(format!(
                "telemetry put {key} epoch={}",
                epoch.map_or_else(|| "-".to_owned(), |e| e.to_string())
            ))
        }
        _ => Err(
            "usage: telemetry put <resource> <json> [<attachment json>] | raw <resource> <json> \
             [<attachment json>] | new-epoch | instance"
                .to_owned(),
        ),
    }
}

/// Answers one command line (the module doc's) but `config restart`.
async fn command(
    line: &str,
    up: &mut Up,
    configured: Option<&(Startup, Arc<config::SimDevice>)>,
) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refuse = |why: &str| format!("refused {}: {why}", words.join(" "));
    let rest = |n: usize| words.get(n..).map(|w| w.join(" ")).unwrap_or_default();
    if words.first() == Some(&"alarm") {
        let Some(a) = &up.alarms else {
            return refuse("alarms.v1 is not implemented here (--alarms)");
        };
        return match alarm_command(&words, after_words(line, 2), a).await {
            Ok(answer) => answer,
            Err(why) => refuse(&why),
        };
    }
    if words.first() == Some(&"desired") {
        if up.author.is_none() && up.target.is_none() {
            return refuse(
                "desired.v1 is not implemented here (--desired-author, --desired-target)",
            );
        }
        return match desired_command(&words, after_words(line, 3), up).await {
            Ok(answer) => answer,
            Err(why) => refuse(&why),
        };
    }
    if words.first() == Some(&"config") {
        let (Some(cfg), Some((_, device))) = (&up.config, configured) else {
            return refuse("config.v1 is not implemented here (--config)");
        };
        return match config_command(&words, cfg, device).await {
            Ok(answer) => answer,
            Err(why) => refuse(&why),
        };
    }
    let health = up.health.as_ref();
    let svc = &mut up.svc;
    let members = &mut up.members;
    if let ["clock-offset", ms] = words.as_slice() {
        return match ms.parse::<i64>() {
            Ok(ms) => {
                svc.minter().simulate_offset(ms);
                format!("clock-offset {ms}")
            }
            Err(e) => refuse(&e.to_string()),
        };
    }
    // Core §2.7: one write on a member, its delete, or a run of members.
    let owned = |from: usize| -> Vec<String> {
        words
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|w| (*w).to_owned())
            .collect()
    };
    match words.as_slice() {
        ["put", iface, resource, ..] => {
            return match members.write(svc, iface, resource, &owned(3), false).await {
                Ok(key) => format!("put {key}"),
                Err(why) => refuse(&why),
            };
        }
        ["delete", iface, resource, ..] => {
            return match members.write(svc, iface, resource, &owned(3), true).await {
                Ok(key) => format!("deleted {key}"),
                Err(why) => refuse(&why),
            };
        }
        ["fill", iface, resource, n] => {
            let Ok(n) = n.parse::<u64>() else {
                return refuse("fill takes a count");
            };
            let Ok(id) = iface.parse::<IfaceId>() else {
                return refuse("not an interface id");
            };
            let Some(params) = members
                .contracts
                .get(&id)
                .and_then(|c| {
                    c.resources
                        .iter()
                        .find(|r| zenkey::implementation::resource_name(r) == *resource)
                })
                .map(|r| r.template.params().count())
            else {
                return refuse(&format!("{id} has no resource {resource:?} here"));
            };
            for i in 0..n {
                let values = vec![format!("m{i}"); params];
                if let Err(why) = members.write(svc, iface, resource, &values, false).await {
                    return refuse(&format!("{i} put, then: {why}"));
                }
            }
            return format!("filled {n}");
        }
        ["put" | "delete" | "fill", ..] => {
            return refuse(
                "usage: put <iface> <resource> [<value>...] | delete <iface> <resource> \
                 [<value>...] | fill <iface> <resource> <n>",
            );
        }
        _ => {}
    }
    let Some(verb) = words.first() else {
        return refuse("an empty line");
    };
    if !["status", "check", "retire", "fault", "confirm"].contains(verb) {
        return refuse("unknown command");
    }
    let Some(h) = health else {
        return refuse("health.v1 is not implemented here (--health)");
    };
    let done = match words.as_slice() {
        ["status", level, ..] => match parse_level(level) {
            Some(Some(l)) => h.set_status(l, rest(2)).await,
            Some(None) => {
                return refuse("a status is UNSPECIFIED only at start (health.v1 §2.6)");
            }
            None => return refuse("a level is OK, DEGRADED or FAILED"),
        },
        ["check", name, level, ..] => match parse_level(level) {
            Some(Some(l)) => h.set_check(name, l, rest(3)).await,
            Some(None) => h.set_check_unknown(name, rest(3)).await,
            None => return refuse("a level is OK, DEGRADED, FAILED or UNSPECIFIED"),
        },
        ["retire", name] => h.retire_check(name).await,
        ["fault", code, level, ..] => match parse_level(level) {
            Some(Some(l)) => h
                .fault(code, l, rest(3))
                .await
                .map(|()| h.status().expect("put at start")),
            _ => return refuse("a fault's level is OK, DEGRADED or FAILED (health.v1 §2.3)"),
        },
        ["confirm", on @ ("on" | "off")] => {
            h.set_confirming(*on == "on");
            Ok(h.status().expect("put at start"))
        }
        _ => {
            return refuse(
                "usage: status <LEVEL> [<reason>] | check <name> <LEVEL> [<detail>] | \
                 retire <name> | fault <code> <LEVEL> [<detail>] | confirm on|off",
            );
        }
    };
    match done {
        Ok(e) => describe(&e),
        Err(e) => refuse(&e.to_string()),
    }
}

#[tokio::main]
async fn main() {
    // The runtime's logs go to stderr (warnings unless `RUST_LOG` says
    // otherwise), so a runner driving this owner as a black box sees what
    // the runtime says, such as hostid.v1's ephemeral warning (§2.6, 0.3),
    // and config.v1's requests, redacted (§2.15).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(
                    "warn,zenkey::config_v1=info,zenkey::alarms=info",
                )
            }),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return;
    }
    let opts = match Options::parse(&args) {
        Ok(o) => o,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    // Answer each line of standard input, and run until it closes.
    let (lines, commands) = flume::unbounded::<String>();
    let stdin_closed = async {
        let _ = tokio::task::spawn_blocking(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        })
        .await;
    };
    if let Err(e) = run_with(opts, |line| println!("{line}"), commands, stdin_closed).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
