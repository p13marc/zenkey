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
//!     [--standby] [--election claim|external] [--instance-id <id>] [--name <word>]
//!     [--retry-ms <ms>] [--grace-ms <ms>] [--poll-ms <ms>] [--lease-ms <ms>]
//!     [--lock-file <path>] [--refuse-without-lock] [--hold-heal] [--peer] [--wait-go]
//!     [--timing] [--no-hlc]
//!     [--arbitrate <entry>[,<entry>...]] [--arbitrate-no-watch] [--watch-connect <endpoint>]
//!     [--clock-bound-ms <ms>] [--commander] [--commander-lead-ms <ms>]
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
//!   with the role unbound, the kill switch (§2.12). A runner's copies
//!   whose requirement names two or more desired resources of the author's
//!   declaration stop the owner with the start's refusal, `not started:
//!   role "task": its binding is invalid (two_documents) (desired.v1
//!   §2.3)`, exit 1, before any token (§2.1, text 0.7, F-224).
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
//! desired gets <target>                     author: the GETs that reached <target>'s document key
//! desired new-epoch                         a re-mint in this session (core §8.1): the author puts again what it keeps
//! desired report                            target: its report (§2.10)
//! desired running                           target: what the drive runs
//! desired fail-next                         target: the drive's next act fails after setting the rate
//! desired refuse-next <code> [<pointer>]    target: the drive refuses its next act
//! desired hold | release                    target: the drive's next act waits at its start | goes on
//! desired reread <ms>|off                   target: the re-read period (§2.5)
//! desired acts                              target: the acts and seeds so far
//! desired seeds                             target: what its last seed found (§2.5, §2.9)
//! ```
//!
//! `put` puts the rest of the line byte for byte, once it reads as strict
//! JSON (§2.10's reading), so every number keeps its exact value (text
//! 0.4, F-194); it is answered `desired put <key> <stamp>`, the stamp as a
//! report writes it (`<time>/<id>`, §2.4), or `desired unchanged <key>` when the
//! document is the one held (nothing put); a put past the bound is
//! refused, naming it (§2.13). `delete` is answered `desired deleted <key>
//! <stamp>`, or `desired none <key>` when no document is held. `documents`
//! is answered `desired documents <json>`, the documents by target;
//! `adoption` `desired adoption found=<record|archive|nothing>
//! reput=<targets> redeleted=<targets> expired=<targets> failed=<targets>`,
//! each list comma-separated, `-` when empty; `new-epoch` `desired new-epoch
//! zk2/<system>/<service>/@zk/instance/<id>`; `report` `desired report
//! <json>`, the `desired.v1.Report` as held now; `running` `desired running
//! <json>`; `acts` `desired acts <n> seeds=<n> held=<true|false>`; `gets`
//! `desired gets <key> <n>`, the GETs the author's state queryables
//! answered whose selector reaches the key, a document held or not
//! (`scenarios.md` §8); `seeds` `desired seeds <n> answered=<n>
//! author=<present|absent|incomplete|not-read|not-made> archive=<-|answered|nothing>`,
//! with ` because <why>` after a read not made, or `desired seeds 0 none`
//! before the first (text 0.4, F-197: the replies the GET drew, the
//! author's presence, read only when the GET drew nothing and it decides
//! something, and the archive, read only on a complete read showing the
//! author absent); every other command as it was asked.
//!
//! **`redundancy.v1`** (`spec/profiles/redundancy/v1.md`, #841) is the
//! runtime's ([`zenkey::redundancy`]), for every contract given that lists
//! `redundancy.v1` in `uses` (a governed interface: the scenarios'
//! `ledger.v1` and `valve.v1`, `zenkey/tests/contracts/`). A governed
//! contract needs `--standby`, implied by every option below but
//! `--instance-id`, `--name`, `--lease-ms` and `--peer`, and `--standby`
//! needs a governed contract: either alone refuses to start, exit 1. The
//! owner claims at its start (its descriptor marks each governed interface
//! `"standby": true`, its instance token), follows the address on a watch
//! session of its own, reads on a reader of its own, and serves only once
//! elected; the `ready` line says it claimed, not that it serves.
//!
//! - `--election claim|external` checks that every governed contract
//!   declares that election (§2.2): the contract's word, never set here.
//! - `--instance-id <16 hex>` is the first instance's id, a test's choice:
//!   the scenarios order their instances by id.
//! - `--name <word>`: a raw operation answers `<word>` instead of its echo,
//!   and a raw state with no template parameters holds `<word>`, put at the
//!   start, or at the takeover for a governed interface (§2.6 step 3).
//!   Every execution of an operation is counted (`redundancy calls`).
//! - `--retry-ms <ms>` (2000) is the retry interval (§2.5), `--grace-ms
//!   <ms>` (2000) the grace of a heal's two reads (§2.9), `--poll-ms <ms>` a
//!   poll of the address this often instead of a watch session (§2.7).
//! - `--lease-ms <ms>` is every session's lease, which its router reads: a
//!   silent link is seen that much later (§2.8).
//! - `--lock-file <path>` is `external`'s lock, a `flock(LOCK_EX |
//!   LOCK_NB)` taken when free, held until `redundancy lock lose` or the
//!   process ends (§2.11); released, it is taken again only after
//!   `redundancy lock hold`. Without it, the lock is the commands':
//!   `redundancy lock hold|lose`, not held at the start.
//!   `--refuse-without-lock` refuses to start without the lock, exit 1,
//!   with nothing declared, rather than stand by: with the commands' lock,
//!   which is not held at the start, always, so it goes with
//!   `--lock-file`.
//! - `--hold-heal` holds the heal's reads from the start (`scenarios.md`
//!   §9's hook), until `redundancy hold-heal off`.
//! - `--peer`, with `--connect`: every session a peer whose only endpoint
//!   is the router's, listening on loopback and gossiping, since a peer
//!   reads only what its routers' clients and its linked peers hold (core
//!   §8.1, 0.40; F-206).
//! - `--wait-go`: every session opens, the owner says `waiting go`, and the
//!   service starts at a `go` line on standard input (`scenarios.md` §3,
//!   text 0.3, F-216), so a runner starts two instances together; any
//!   other line before it is refused.
//! - A contract listing `redundancy.v1` is written only once the start has
//!   checked its declaration (F-214): an invalid one stops the owner with
//!   `not started: … invalid`, exit 1.
//!
//! Its commands, each answered by one JSON line, or `refused <line>:
//! <why>`:
//!
//! ```text
//! redundancy role                            {"redundancy": "role", "instance": <id>, "interfaces": {<iface>: <standing>}}
//! redundancy reads                           {"redundancy": "reads", "interfaces": {<iface>: {"made", "complete", "not_made", "polls", "last", "poll_log"}}, "sessions": {"reader": {"reads", "settles"}, "watch"?: …}}
//! redundancy zid                             {"redundancy": "zid", "zid": <zid>, "reader": <zid>, "watch"?: <zid>}
//! redundancy lock hold|lose                  {"redundancy": "lock", "held": <bool>}
//! redundancy calls                           {"redundancy": "calls", "calls": <n>}
//! redundancy hold-heal on|off                {"redundancy": "hold-heal", "held": <bool>}
//! redundancy wait <iface> serving|standby <ms>   {"redundancy": "wait", "iface", "reached": <bool>, "standing"}
//! redundancy new-epoch                       {"redundancy": "new-epoch", "instance": <id>}
//! ```
//!
//! A `<standing>` is `{"role": "unclaimed"|"standby"|"serving", "why"?,
//! "instance"?, "fault"?}`, `fault` saying why it elects no more after an
//! act zenoh refused: `why` the last stand-by's (`incumbent`, `incomplete`,
//! `own_claim_missing`, `descriptor_unread`, `lower_candidate`, `no_lock`,
//! as `elections.json` writes them), with the instance it names. `reads`
//! counts the election's reads (made, of them complete, not made: its
//! reader cut off) and the poll's, `last` the last one's reading, and each
//! session's liveliness GETs. `lock` asks the lock again at once. `wait`
//! answers once the role is reached, or at the deadline. `new-epoch` is a
//! re-mint in this session (core §8.1): a serving interface's token is
//! re-minted make-before-break, and the election knows its new id (F-209).
//! **A crash is the process's**: kill it, as the scenarios' §1 and §10 do.
//!
//! **`timing.v1`** (`spec/profiles/timing/v1.md`, #908) is the runtime's
//! ([`zenkey::timing`]): with `--timing` the owner implements `pace.v1`,
//! the scenarios' contract (`spec/profiles/timing/scenarios.md`,
//! `zenkey/tests/contracts/pace.v1.toml`, embedded), unless a contract file
//! given is `pace.v1`, and drives it from standard input: each member
//! published at its period while it is on, `cmd` paused at the start and
//! each axis on once added (`timing add <axis>`, its member token first;
//! `timing declare <axis>` declares the token alone, scenarios §3 step 5),
//! every sample its sequence number as text, stamped by the runtime's
//! writer from the service's clock. `timing offset <ms>` shifts that clock
//! (§4's skewed owner), and `timing stamp none` puts raw through the
//! session with no stamp set, its payload a wall clock 10 s old (§5's
//! unstamped owner). `--no-hlc` leaves the session's HLC off with
//! `--connect`: an HLC follows the stamps its own session puts (core §4.1),
//! so an offset would grow under one. Each command is answered by one JSON
//! line; `--help` lists them. The reader is `consume --timing`.
//!
//! **`arbitration.v1`** (`spec/profiles/arbitration/v1.md`, #909) is the
//! runtime's ([`zenkey::arbitration`]), on the scenarios' `rudder.v1` and
//! `steer.v1` (`zenkey/tests/contracts/`, embedded). `--arbitrate
//! <entry>,…` makes the owner an actuator: `rudder.v1` implemented, its
//! role `cmd` bound to the entries in order and arbitrated over the
//! `steer.v1` it holds (a `steer.v1` file given, else the embedded one),
//! refused, exit 1, when the declaration is invalid, before anything of it
//! is declared or put (`ServiceBuilder::arbitrate` reads it at once, text
//! 0.3, F-274); it prints
//! each switch, one JSON line, as `scenarios.md`'s conventions write it,
//! and puts every command it delivers on `angle`. `--commander` makes it a
//! commander: `steer.v1`'s `cmd` put at its period, `<service> <n>`, from
//! `--commander-lead-ms` after its tokens (F-269), or raw round its writer,
//! `standby <n>` (scenarios §11, F-276). `arbitrate …` and
//! `commander …` lines are answered by one JSON line each; `--help` lists
//! them.

use std::collections::HashMap;
use std::future::Future;
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
use zenkey::redundancy::{FileLock, Lock, ManualLock, RedundancyStartup, Role, Takeover};
use zenkey::state::StateWriter;
use zenkey::writer::{EventWriter, Writer};
use zenkey::{Address, Implementation, OpError, Service, ServiceBuilder, ServiceConfig};

#[path = "owner/config.rs"]
pub mod config;

#[path = "owner/desired.rs"]
pub mod drive;

#[path = "owner/timing.rs"]
pub mod timing_drive;

#[path = "owner/arbitrate.rs"]
pub mod arbitrate;

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
    /// `--standby`, or implied by a redundancy option: `redundancy.v1` for
    /// every governed contract, on a reader and a watch session of the
    /// owner's own (#841).
    pub standby: bool,
    /// `--election claim|external`: the election every governed contract
    /// must declare (§2.2): the contract's word, checked, never set here.
    pub election: Option<zenkey::redundancy::Election>,
    /// `--instance-id <16 hex>`: the first instance's id, a test's choice
    /// (`scenarios.md`: the test orders its instances by id).
    pub instance_id: Option<zenkey::model::grammar::InstanceId>,
    /// `--name <word>`: what a raw operation answers instead of its echo,
    /// and the value of a raw state put at the start or the takeover.
    pub name: Option<String>,
    /// `--retry-ms <ms>`: the retry interval (§2.5), 2000 by default.
    pub retry_ms: u64,
    /// `--grace-ms <ms>`: the grace of a heal's two reads (§2.9), 2000.
    pub grace_ms: u64,
    /// `--poll-ms <ms>`: no watch session; a poll of the address this often.
    pub poll_ms: Option<u64>,
    /// `--lease-ms <ms>`: every session's lease, which its router reads.
    pub lease_ms: Option<u64>,
    /// `--lock-file <path>`: `external`'s lock, a `flock`; without it, the
    /// lock is the commands' (`redundancy lock hold|lose`).
    pub lock_file: Option<PathBuf>,
    /// `--refuse-without-lock`: under `external`, refuse to start without
    /// the lock (§2.11).
    pub refuse_without_lock: bool,
    /// `--hold-heal`: the heal's reads held from the start.
    pub hold_heal: bool,
    /// `--peer`: with `--connect`, every session a peer that listens on
    /// loopback and gossips, rather than a client (core §8.1, 0.40).
    pub peer: bool,
    /// `--wait-go`: the service starts at a `go` line on standard input,
    /// its sessions open before (`redundancy.v1` scenarios §3; F-216).
    pub wait_go: bool,
    /// `--timing`: `pace.v1`, `timing.v1`'s scenarios' contract,
    /// implemented and driven (#908), unless a contract file implements it.
    pub timing: bool,
    /// `--no-hlc`: with `--connect`, the session's HLC off.
    pub no_hlc: bool,
    /// `--arbitrate <entry>,…`: `rudder.v1` implemented, its role `cmd`
    /// bound to the entries in their order and arbitrated
    /// (`arbitration.v1`, #909), unless a contract file implements it.
    pub arbitrate: Option<Vec<String>>,
    /// `--arbitrate-no-watch`: the actuator holds no watch session, and
    /// judges every provider by its deadline alone (§2.7, §2.12).
    pub arbitrate_watch: bool,
    /// `--watch-connect <endpoint>`: the actuator's watch session a client
    /// of that endpoint, rather than `--connect`'s (`scenarios.md` §7's
    /// link the runner cuts alone).
    pub watch_connect: Option<String>,
    /// `--clock-bound-ms <ms>`: ε, the arbitrated binding's
    /// `clock_bound_ms` (`timing.v1` §2.6).
    pub clock_bound_ms: Option<u64>,
    /// `--commander`: `steer.v1` implemented and its `cmd` put at its
    /// period, unless a contract file implements it.
    pub commander: bool,
    /// `--commander-lead-ms <ms>`: how long after its tokens the commander
    /// starts putting (500 by default; text 0.2, F-269).
    pub commander_lead_ms: u64,
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
    [--standby] [--election claim|external] [--instance-id <id>] [--name <word>]
    [--retry-ms <ms>] [--grace-ms <ms>] [--poll-ms <ms>] [--lease-ms <ms>]
    [--lock-file <path>] [--refuse-without-lock] [--hold-heal] [--peer] [--wait-go]
    [--timing] [--no-hlc]
    [--arbitrate <entry>[,<entry>...]] [--arbitrate-no-watch] [--watch-connect <endpoint>]
    [--clock-bound-ms <ms>] [--commander] [--commander-lead-ms <ms>]
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
  redundancy ...                          redundancy.v1 (--standby), below
  timing ...                              timing.v1 (--timing), below
  arbitrate ...                           arbitration.v1's actuator (--arbitrate), below
  commander ...                           arbitration.v1's commander (--commander), below

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

  desired put <target> <json>     author: <target> (<system>/<service>)'s document: the
                                  rest of the line, put byte for byte once it reads as
                                  strict JSON, so every number keeps its exact value
  desired delete <target>         author: its delete
  desired documents               author: the documents held
  desired adoption                author: what the start took over
  desired gets <target>           author: the GETs its state queryables answered whose
                                  selector reaches <target>'s document key
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
  desired seeds                   target: what its last seed found

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
    desired gets <key> <n>
    desired seeds <n> answered=<n> author=<present|absent|incomplete|not-read|not-made>
      archive=<-|answered|nothing> [because <why>]
                                  the replies the last seed's GET drew; the author's
                                  presence, read when the GET drew nothing and it decides
                                  the archive or a retry (not-read otherwise; not-made,
                                  and why, for a read not made); the archive, read only
                                  when a complete read shows the author absent;
                                  desired seeds 0 none before the first
    desired fail-next|refuse-next <code>|hold|release|reread <ms>|off
    refused <line>: <why>

  A restart is the process's: kill it, or close standard input, and start it
  again (an author with the same --desired-record).

redundancy.v1 (spec/profiles/redundancy/v1.md)
  For every contract given that lists redundancy.v1 in uses (a governed
  interface; the scenarios' ledger.v1 and valve.v1). A governed contract needs
  --standby (implied by every option here but --instance-id, --name,
  --lease-ms and --peer), and --standby a governed contract: either alone
  refuses to start, exit 1. The owner claims at its start (its descriptor marks
  each governed interface \"standby\": true, its instance token), follows the
  address on a watch session, reads on a reader of its own, and serves only
  once elected: ready says it claimed, not that it serves.

  --standby                       take part in redundancy.v1's election
  --election claim|external       every governed contract must declare it (checked,
                                  never set: the election is the contract's)
  --instance-id <id>              the first instance's id, 16 lowercase hex digits
  --name <word>                   a raw operation answers <word> instead of its echo;
                                  a raw state with no parameters holds <word>, put at
                                  the start, or at the takeover when governed
  --retry-ms <ms>                 the retry interval, 2000 by default
  --grace-ms <ms>                 the grace of a heal's two reads, 2000 by default
  --poll-ms <ms>                  no watch session: a poll of the address this often
  --lease-ms <ms>                 every session's lease, as its router reads it
  --lock-file <path>              external's lock, a flock taken when free and held
                                  until redundancy lock lose or the process ends;
                                  without it the lock is redundancy lock hold|lose's,
                                  not held at the start
  --refuse-without-lock           under external, refuse to start without the lock,
                                  exit 1, nothing declared (the commands' lock is
                                  not held at the start: give --lock-file)
  --hold-heal                     the heal's reads held until redundancy hold-heal off
  --peer                          with --connect, every session a peer of that router,
                                  listening on loopback and gossiping
  --wait-go                       open every session, say waiting go, and start the service
                                  at a go line on standard input (scenarios.md §3)

  redundancy role                 each governed interface's standing
  redundancy reads                the election's reads, the poll's last reads, and each
                                  session's GETs
  redundancy zid                  the zids of the claim's session, the reader and the watch
  redundancy lock hold|lose       the lock (external), asked again at once
  redundancy calls                the operations' executions so far
  redundancy hold-heal on|off     the heal's reads held, or released
  redundancy wait <iface> serving|standby <ms>
                                  answered once the role is reached, or at <ms>
  redundancy new-epoch            a re-mint in this session, the election told first

  Answers, one JSON line each:
    {\"redundancy\": \"role\", \"instance\": <id>, \"interfaces\": {<iface>: <standing>}}
    {\"redundancy\": \"reads\", \"interfaces\": {<iface>: {\"made\": <n>, \"complete\": <n>,
      \"not_made\": <n>, \"polls\": <n>, \"last\": <reading>}}, \"sessions\":
      {\"reader\": {\"reads\": <n>, \"settles\": <n>}, \"watch\"?: {...}}}, and per
      interface \"poll_log\": [{\"made\", \"complete\", \"holders\", \"appeared\",
      \"withdrawn\"}], the poll's last 32 reads, the latest last
    {\"redundancy\": \"zid\", \"zid\": <zid>, \"reader\": <zid>, \"watch\"?: <zid>}
    {\"redundancy\": \"lock\", \"held\": <bool>}
    {\"redundancy\": \"calls\", \"calls\": <n>}
    {\"redundancy\": \"hold-heal\", \"held\": <bool>}
    {\"redundancy\": \"wait\", \"iface\": <iface>, \"reached\": <bool>, \"standing\": <standing>}
    {\"redundancy\": \"new-epoch\", \"instance\": <id>}
    <standing>: {\"role\": \"unclaimed\"|\"standby\"|\"serving\", \"why\"?: <why>,
      \"instance\"?: <id>, \"fault\"?: <words>}, <why> as elections.json writes it:
      incumbent, incomplete, own_claim_missing, descriptor_unread, lower_candidate,
      no_lock; fault: an act zenoh refused, undone, and no more elections
    refused <line>: <why>

  A crash is the process's: kill it (scenarios.md §1, §10).

timing.v1 (spec/profiles/timing/v1.md)
  --timing implements pace.v1, the scenarios' contract
  (spec/profiles/timing/scenarios.md, its fingerprint sha256:6528b66a...),
  unless a contract file given is pace.v1: cmd (period 100 ms, deadline
  500 ms, lifespan 300 ms) and axis/{axis} (period 100 ms, deadline 500 ms,
  a member token per axis). Every member publishes at its period while it
  is on, each sample its sequence number as text, through the runtime's
  writer, which stamps each from the service's clock (its id the session's
  zid, the descriptor's meta.zid, timing.v1 §2.6). cmd starts paused; an
  axis starts on once added.

  --timing                        implement and drive pace.v1
  --no-hlc                        with --connect, the session's HLC off: the
                                  stamps are the session's wall clock with its
                                  zid, so `timing offset` shifts them alone (an
                                  HLC would follow them, core §4.1); and `timing
                                  stamp none` puts unstamped, as §5 has it

  timing pause|resume <member>    stop or start publishing <member>: cmd, or
                                  axis/<axis>
  timing every <member> <ms>      publish <member> every <ms> rather than at its
                                  period (scenarios.md §6 step 3: slower than
                                  its period, within its deadline)
  timing declare <axis>           declare axis/<axis>'s member token alone (core
                                  §8.1), publishing nothing on it (scenarios.md §3
                                  step 5)
  timing add <axis>               declare axis/<axis>'s member token (core §8.1),
                                  unless `timing declare` holds it, then publish
                                  it at the period
  timing remove <axis>            stop axis/<axis>, then withdraw its member token
  timing offset <ms>              the stamping clock's offset (negative: behind)
  timing stamp own|none           none: every member put raw through the session,
                                  no stamp set, its payload the owner's wall clock
                                  less 10 s in nanoseconds since the Unix epoch, as
                                  text (scenarios.md §5); own: the writers again
  timing published                the samples each member published
  timing zid                      the session's zid
  timing writers                  whether each member's writer stamps its puts

  Answers, one JSON line each:
    {\"timing\": \"pause\"|\"resume\", \"member\": <member>, \"key\": <key>, \"on\": <bool>}
    {\"timing\": \"every\", \"member\": <member>, \"key\": <key>, \"ms\": <ms>}
    {\"timing\": \"declare\", \"member\": <member>, \"epoch\": <id>}
    {\"timing\": \"add\", \"member\": <member>, \"key\": <key>, \"epoch\": <id>|null}
      (epoch null: the token `timing declare` declared, kept)
    {\"timing\": \"remove\", \"member\": <member>}
    {\"timing\": \"offset\", \"ms\": <ms>}
    {\"timing\": \"stamp\", \"mode\": \"own\"|\"none\"}
    {\"timing\": \"published\", \"members\": {<member>: <n>, ...}}
    {\"timing\": \"zid\", \"zid\": <zid>}
    {\"timing\": \"writers\", \"stamped\": {<member>: <bool>, ...}}
    refused <line>: <why>

  The reader is `consume --timing` (its --help).

arbitration.v1 (spec/profiles/arbitration/v1.md)
  On the scenarios' contracts (spec/profiles/arbitration/scenarios.md, its
  conventions), embedded as zenkey/tests/contracts/ keeps them: rudder.v1, the
  actuator's (sha256:dfcdac25...), and steer.v1, the commanders'
  (sha256:5162db05...). A rudder.v1 contract file given is implemented instead
  of the embedded one; a steer.v1 file given is the revision the actuator
  holds, and the one a commander implements.

  The actuator (--arbitrate) implements rudder.v1, its role cmd bound to the
  entries in their order, the first the highest (core R1, R3), and arbitrated
  over the steer.v1 it holds, by the runtime's Consumer::arbitrate. It is
  refused at start when the declaration is invalid (arbitration.v1 §2.2):
  stderr says `not started: <system>/<service>: role \"cmd\" (steer.v1):
  arbitration.v1 invalid: <reason>[, <reason>...] (...)`, exit 1, nothing
  declared or put. Its sessions: the service's, which holds the subscription and
  makes every read again; and a watch session of its own, a client of
  --watch-connect's endpoint or --connect's, where it hears withdrawals.
  It puts every command it delivers on angle, its payload unchanged (at its
  start angle holds ok, the owner's raw state value, put once the declaration
  is read valid and before its tokens: a refused actuator puts nothing, text
  0.3, F-274).

  --arbitrate <entry>,...         implement rudder.v1, cmd bound to these
                                  <system>/<service> entries (either chunk *), in order
  --arbitrate-no-watch            no watch session: every provider judged by its
                                  deadline alone (§2.7, §2.12)
  --watch-connect <endpoint>      the watch session a client of this endpoint
  --clock-bound-ms <ms>           ε, the binding's clock_bound_ms: a sample older than
                                  its lifespan by it is expired (timing.v1 §2.6)

  Standard output, after ready, one JSON line per switch, as it tells its
  application (§2.10), the first its start's:
    {\"switch\": {\"from\": <address>|null, \"to\": <address>|null,
      \"cause\": \"start\"|\"sample\"|\"deadline\"|\"withdrawn\"}, \"at_ms\": <ms>,
      \"last_ms\": <ms>|null}
    null is no provider, the dead-man; at_ms the instant of the event that made
    the switch, last_ms from's last arrival (null when from is null), each in
    milliseconds since the actuator's start on its monotonic clock, a float
    exact to the nanosecond.

  arbitrate counts                {\"counts\": {\"providers\": {<address>: {\"delivered\": <n>,
                                  \"not_active\": <n>, \"deleted\": <n>, \"expired\": <n>,
                                  \"not_incumbent\": <n>}, ...}, \"wild\": <n>,
                                  \"unresolved\": <n>, \"unbound\": <n>}} (§2.5), every
                                  provider known, in rank order
  arbitrate state                 {\"arbitrate\": \"state\", \"active\": <address>|null,
                                  \"providers\": [...], \"withdrawn\": [...], \"moving\": <bool>,
                                  \"watching\": <bool>, \"deadline_ms\": <ms>}
  arbitrate reads                 {\"arbitrate\": \"reads\", \"reads\": <n>, \"not_made\": <n>,
                                  \"round_trips\": <n>}: the reads again made (§2.7), those
                                  not made (cut off), and the watch session's round trips
                                  (the one at its start, then one after each move, §2.8)

  The commander (--commander) implements steer.v1 and puts cmd at its period,
  each sample `<service> <n>`, n counting from 0, through the runtime's writer,
  from --commander-lead-ms after its tokens (500 by default: text 0.2, F-269).
  With --standby on a governed steer.v1 (one listing redundancy.v1), its
  writer refuses while it stands by, and each refusal is counted.

  --commander                     implement and drive steer.v1
  --commander-lead-ms <ms>        the lead before its first put

  commander hold|resume           stop or start putting, the tokens kept
  commander raw on|off            on: every put raw through the session, round the
                                  writer, stamped by the session's HLC, each sample
                                  `standby <n>`: a standby's samples (scenarios.md §11)
  commander sent                  {\"commander\": \"sent\", \"sent\": <n>, \"refused\": <n>}
  commander new-epoch             a re-mint, make-before-break (core §8.1)
  commander zid                   the session's zid

  Answers, one JSON line each:
    {\"commander\": \"hold\"|\"resume\", \"on\": <bool>}
    {\"commander\": \"raw\", \"raw\": <bool>}
    {\"commander\": \"new-epoch\", \"instance\": <id>}
    {\"commander\": \"zid\", \"zid\": <zid>}
    refused <line>: <why>

  A crash is the process's: kill it (scenarios.md §3).
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
             [--standby] [--election claim|external] [--instance-id <id>] [--name <word>] \
             [--retry-ms <ms>] [--grace-ms <ms>] [--poll-ms <ms>] [--lease-ms <ms>] \
             [--lock-file <path>] [--refuse-without-lock] [--hold-heal] [--peer] [--wait-go] \
             [--timing] [--no-hlc] \
             [--arbitrate <entry>[,<entry>...]] [--arbitrate-no-watch] \
             [--watch-connect <endpoint>] [--clock-bound-ms <ms>] \
             [--commander] [--commander-lead-ms <ms>] \
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
            standby: false,
            election: None,
            instance_id: None,
            name: None,
            retry_ms: 2000,
            grace_ms: 2000,
            poll_ms: None,
            lease_ms: None,
            lock_file: None,
            refuse_without_lock: false,
            hold_heal: false,
            peer: false,
            wait_go: false,
            timing: false,
            no_hlc: false,
            arbitrate: None,
            arbitrate_watch: true,
            watch_connect: None,
            clock_bound_ms: None,
            commander: false,
            commander_lead_ms: 500,
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
                [flag, tail @ ..] if flag == "--standby" => {
                    o.standby = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--election" => {
                    o.election = Some(match value.as_str() {
                        "claim" => zenkey::redundancy::Election::Claim,
                        "external" => zenkey::redundancy::Election::External,
                        _ => return Err(format!("--election {value:?}: claim or external")),
                    });
                    o.standby = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--instance-id" => {
                    o.instance_id = Some(value.parse().map_err(|e| {
                        format!("--instance-id {value:?}: 16 lowercase hex digits: {e}")
                    })?);
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--name" => {
                    o.name = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--retry-ms" => {
                    o.retry_ms = value
                        .parse()
                        .map_err(|e| format!("--retry-ms {value:?}: {e}"))?;
                    o.standby = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--grace-ms" => {
                    o.grace_ms = value
                        .parse()
                        .map_err(|e| format!("--grace-ms {value:?}: {e}"))?;
                    o.standby = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--poll-ms" => {
                    o.poll_ms = Some(
                        value
                            .parse()
                            .map_err(|e| format!("--poll-ms {value:?}: {e}"))?,
                    );
                    o.standby = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--lease-ms" => {
                    o.lease_ms = Some(
                        value
                            .parse()
                            .map_err(|e| format!("--lease-ms {value:?}: {e}"))?,
                    );
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--lock-file" => {
                    o.lock_file = Some(PathBuf::from(value));
                    o.standby = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--refuse-without-lock" => {
                    o.refuse_without_lock = true;
                    o.standby = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--hold-heal" => {
                    o.hold_heal = true;
                    o.standby = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--peer" => {
                    o.peer = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--wait-go" => {
                    o.wait_go = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--timing" => {
                    o.timing = true;
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--no-hlc" => {
                    o.no_hlc = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--arbitrate" => {
                    let entries: Vec<String> = value
                        .split(',')
                        .filter(|e| !e.is_empty())
                        .map(str::to_owned)
                        .collect();
                    zenkey::arbitration::Binding::parse(&entries)
                        .map_err(|e| format!("--arbitrate {value:?}: {e}"))?;
                    o.arbitrate = Some(entries);
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--arbitrate-no-watch" => {
                    o.arbitrate_watch = false;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--watch-connect" => {
                    o.watch_connect = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--clock-bound-ms" => {
                    o.clock_bound_ms = Some(
                        value
                            .parse()
                            .map_err(|e| format!("--clock-bound-ms {value:?}: {e}"))?,
                    );
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--commander" => {
                    o.commander = true;
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--commander-lead-ms" => {
                    o.commander_lead_ms = value
                        .parse()
                        .map_err(|e| format!("--commander-lead-ms {value:?}: {e}"))?;
                    o.commander = true;
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
        if o.peer && o.connect.is_none() {
            return Err("--peer: give --connect, the peers' only endpoint".to_owned());
        }
        if o.arbitrate.is_none()
            && (!o.arbitrate_watch || o.watch_connect.is_some() || o.clock_bound_ms.is_some())
        {
            return Err(
                "--arbitrate-no-watch, --watch-connect and --clock-bound-ms are the actuator's: \
                 give --arbitrate"
                    .to_owned(),
            );
        }
        if o.arbitrate.is_some() && o.connect.is_none() {
            return Err("--arbitrate: give --connect, the actuator's router".to_owned());
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

/// Each line `open`'s reader gives, as commands, and a future that
/// resolves once it ends: what `main` hands [`run_with`] for standard
/// input. The reader runs on a thread of its own **from the call on**, so
/// a line sent before the service is up reaches the run: `--wait-go`'s
/// `go` among them. It used to start inside the future, which the run
/// polls only once the service is up, so `--wait-go` heard nothing (text
/// 0.4, F-220).
pub fn command_lines<R, F>(open: F) -> (flume::Receiver<String>, impl Future<Output = ()> + Send)
where
    R: std::io::BufRead,
    F: FnOnce() -> R + Send + 'static,
{
    let (lines, commands) = flume::unbounded::<String>();
    let (ended, end) = flume::bounded::<()>(1);
    std::thread::spawn(move || {
        for line in open().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
        let _ = ended.send(());
    });
    (commands, async move {
        let _ = end.recv_async().await;
    })
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
    let mut arb_steer = None;
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
        // arbitration.v1 (#909): a steer.v1 file is the revision the
        // actuator holds, and implemented only by a commander.
        if opts.arbitrate.is_some() && imp.iface() == &arbitrate::steer_iface() {
            arb_steer = Some(imp.shared_contract());
            if !opts.commander {
                continue;
            }
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
    // timing.v1 (#908): the scenarios' pace.v1, unless a file has it.
    if opts.timing && !imps.iter().any(|i| i.iface() == &timing_drive::iface()) {
        imps.push(Implementation::new(timing_drive::contract()?));
    }
    // arbitration.v1 (#909): the scenarios' rudder.v1 and steer.v1, unless
    // a file has them.
    if opts.commander && !imps.iter().any(|i| i.iface() == &arbitrate::steer_iface()) {
        imps.push(Implementation::new(arbitrate::steer()?));
    }
    let arb_steer = match &opts.arbitrate {
        Some(entries) => {
            if !imps.iter().any(|i| i.iface() == &arbitrate::rudder_iface()) {
                imps.push(Implementation::new(arbitrate::rudder()?));
            }
            let entries: Vec<&str> = entries.iter().map(String::as_str).collect();
            config = config.bind(arbitrate::ROLE, &entries);
            if let Some(b) = config.bindings.get_mut(arbitrate::ROLE) {
                b.clock_bound_ms = opts.clock_bound_ms;
            }
            Some(match arb_steer {
                Some(c) => c,
                None => Arc::new(arbitrate::steer()?),
            })
        }
        None => None,
    };

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
    if let Some(ms) = opts.lease_ms {
        z.insert_json5("transport/link/tx/lease", &ms.to_string())?;
    }
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
            side(&mut z, &opts, endpoint)?;
            // §4.3: a session that serves state enables its HLC. A simulated
            // offset leaves it off (the module doc): the HLC would re-stamp
            // the offset clock's stamps before they leave.
            if opts.clock_offset_ms == 0 && !opts.no_hlc {
                z.insert_json5("timestamping/enabled", "true")?;
            }
            let session = zenoh::open(z).await?;
            say(format!("connected {endpoint}"));
            (session, endpoint.clone())
        }
    };
    // redundancy.v1 §2.5 step 1, §2.7 (#841; text 0.2, F-204): a reader
    // and, unless it polls, a watch session of the owner's own, beside the
    // claim's.
    let red = if opts.standby {
        let open = || async {
            let mut c = zenoh::Config::default();
            c.insert_json5("scouting/multicast/enabled", "false")?;
            if let Some(ms) = opts.lease_ms {
                c.insert_json5("transport/link/tx/lease", &ms.to_string())?;
            }
            side(&mut c, &opts, &endpoint)?;
            zenoh::open(c).await
        };
        let reader = open().await?;
        let watch = match opts.poll_ms {
            Some(_) => None,
            None => Some(open().await?),
        };
        let file = opts.lock_file.as_ref().map(|p| Arc::new(FileLock::new(p)));
        Some(RedCtx {
            reader,
            watch,
            manual: Arc::new(ManualLock::new(false)),
            file,
        })
    } else {
        None
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

    // arbitration.v1 §2.7 (#909): the actuator's watch session, a client
    // of its own endpoint.
    let arb_watch = match (&opts.arbitrate, opts.arbitrate_watch) {
        (Some(_), true) => {
            let at = opts.watch_connect.as_ref().unwrap_or(&endpoint);
            let mut w = zenoh::Config::default();
            w.insert_json5("scouting/multicast/enabled", "false")?;
            w.insert_json5("mode", "\"client\"")?;
            w.insert_json5("connect/endpoints", &serde_json::to_string(&[at])?)?;
            w.insert_json5(
                "connect/retry",
                "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
            )?;
            Some(zenoh::open(w).await?)
        }
        _ => None,
    };
    let (lines_out, lines_in) = flume::unbounded::<String>();

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
        red,
        calls: Arc::default(),
        first_up: std::sync::atomic::AtomicBool::new(true),
        arb_steer,
        arb_watch,
        lines: lines_out,
    };
    // --wait-go (F-216): every session is open; the service starts at `go`.
    if opts.wait_go {
        say("waiting go".to_owned());
        loop {
            match commands.recv_async().await {
                Ok(l) if l.trim() == "go" => break,
                Ok(l) => say(format!("refused {l}: this owner starts at go (--wait-go)")),
                Err(_) => return Ok(()),
            }
        }
    }
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
            // arbitration.v1's switches, as the actuator tells them.
            Ok(line) = lines_in.recv_async() => say(line),
        }
    }
    while let Ok(line) = lines_in.try_recv() {
        say(line);
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
    if let Some(w) = &ctx.arb_watch {
        w.close().await?;
    }
    if let Some(r) = &ctx.red {
        r.reader.close().await?;
        if let Some(w) = &r.watch {
            w.close().await?;
        }
    }
    session.close().await?;
    Ok(())
}

/// A session beside the claim's, or the claim's own with `--connect`: a
/// client of `endpoint`, or with `--peer` a peer whose only endpoint it is,
/// listening on loopback and gossiping (core §8.1, 0.40: a peer reads what
/// its linked peers hold), retrying fast.
fn side(c: &mut zenoh::Config, opts: &Options, endpoint: &str) -> Result<(), BoxError> {
    c.insert_json5("connect/endpoints", &serde_json::to_string(&[endpoint])?)?;
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )?;
    if opts.peer {
        c.insert_json5("mode", "\"peer\"")?;
        c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
        c.insert_json5("scouting/gossip/enabled", "true")?;
    } else {
        c.insert_json5("mode", "\"client\"")?;
    }
    Ok(())
}

/// `redundancy.v1`'s sessions and locks (#841), the process's.
struct RedCtx {
    reader: zenoh::Session,
    watch: Option<zenoh::Session>,
    /// The commands' lock (`redundancy lock hold|lose`), when no
    /// `--lock-file` is given.
    manual: Arc<ManualLock>,
    file: Option<Arc<FileLock>>,
}

impl RedCtx {
    fn lock(&self) -> Arc<dyn Lock> {
        match &self.file {
            Some(f) => Arc::clone(f) as Arc<dyn Lock>,
            None => Arc::clone(&self.manual) as Arc<dyn Lock>,
        }
    }
}

/// What a takeover puts (§2.6 step 3): each raw state of the interface
/// with no template parameters, its value `--name`'s, else `ok`.
struct PutStates {
    states: Vec<(IfaceId, Arc<StateWriter>)>,
    value: String,
}

impl Takeover for PutStates {
    fn taking_over<'a>(&'a self, iface: &'a IfaceId) -> zenkey::call::BoxFuture<'a, ()> {
        Box::pin(async move {
            for (i, w) in &self.states {
                if i == iface
                    && let Err(e) = w.put(self.value.clone()).await
                {
                    eprintln!("redundancy.v1: a takeover's put failed: {e}");
                }
            }
        })
    }
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
    /// redundancy.v1's sessions and locks, with `--standby`.
    red: Option<RedCtx>,
    /// The operations' executions, every instance's.
    calls: Arc<std::sync::atomic::AtomicU64>,
    /// Whether the next bring-up is the first: `--instance-id` is its.
    first_up: std::sync::atomic::AtomicBool,
    /// arbitration.v1's revision of steer.v1 held, with `--arbitrate`.
    arb_steer: Option<Arc<Contract>>,
    /// The actuator's watch session, with `--arbitrate`.
    arb_watch: Option<zenoh::Session>,
    /// Lines said between commands: the actuator's switches.
    lines: flume::Sender<String>,
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
    _states: Vec<(IfaceId, Arc<StateWriter>)>,
    /// redundancy.v1's elections, with `--standby`, and the lock.
    red: Option<(zenkey::redundancy::Redundancy, Option<Arc<dyn Lock>>)>,
    calls: Arc<std::sync::atomic::AtomicU64>,
    health: Option<Health>,
    members: Members,
    config: Option<Configuration>,
    alarms: Option<Alarms>,
    /// desired.v1's author, its target and the target's drive.
    author: Option<desired::Author>,
    target: Option<desired::Target>,
    drive: Option<Arc<drive::Drive>>,
    /// timing.v1's drive, with `--timing`.
    timing: Option<timing_drive::Drive>,
    /// arbitration.v1's actuator, with `--arbitrate`.
    actuator: Option<arbitrate::Actuator>,
    /// arbitration.v1's commander, with `--commander`.
    commander: Option<arbitrate::Commander>,
}

impl Up {
    async fn close(self) -> Result<(), BoxError> {
        drop(self.actuator);
        drop(self.commander);
        drop(self.timing);
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
        if line.split_whitespace().next() == Some("timing") {
            let answer = match &up.timing {
                Some(_) => {
                    let Up { svc, timing, .. } = &mut up;
                    timing_drive::command(line, svc, timing.as_ref().expect("checked")).await
                }
                None => format!("refused {line}: timing.v1 is not driven here (--timing)"),
            };
            return Ok((answer, up));
        }
        if line.split_whitespace().next() == Some("redundancy") {
            let answer = redundancy_command(line, &mut up, self.red.as_ref(), self.session).await;
            return Ok((answer, up));
        }
        if line.split_whitespace().next() == Some("arbitrate") {
            let answer = match &up.actuator {
                Some(a) => arbitrate::command(line, a),
                None => format!("refused {line}: no actuator here (--arbitrate)"),
            };
            return Ok((answer, up));
        }
        if line.split_whitespace().next() == Some("commander") {
            let Up { svc, commander, .. } = &mut up;
            let answer = match commander.as_ref() {
                Some(c) => arbitrate::commander_command(line, svc, c).await,
                None => format!("refused {line}: no commander here (--commander)"),
            };
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
        if self
            .first_up
            .swap(false, std::sync::atomic::Ordering::SeqCst)
            && let Some(id) = &opts.instance_id
        {
            b.set_instance_id(id.clone());
        }
        let value = opts.name.clone().unwrap_or_else(|| "ok".to_owned());
        let mut governed = std::collections::BTreeSet::new();
        let mut gated = std::collections::BTreeSet::new();
        let mut states = Vec::new();
        let mut servers = Vec::new();
        let mut members = Members::default();
        let none = Bindings::new();
        let mut arbitrated = false;
        for imp in self.imps.iter().cloned() {
            let iface = imp.iface().clone();
            let resources = imp.contract().resources.clone();
            // redundancy.v1 §2.2 (#841): a governed interface is elected,
            // and --election says which election it must declare.
            if let zenkey::model::redundancy::Declaration::Governed(e) =
                zenkey::model::redundancy::declaration_of(imp.contract(), false)
            {
                if let Some(want) = opts.election
                    && want != e
                {
                    return Err(format!(
                        "--election {want}: {iface}'s contract declares {e} (redundancy.v1 §2.2)"
                    )
                    .into());
                }
                governed.insert((iface.clone(), e));
            }
            // F-214: an interface whose contract lists redundancy.v1 is
            // written only once start() has checked its declaration.
            if zenkey::model::redundancy::is_governed(imp.contract()) {
                gated.insert(iface.clone());
            }
            members
                .contracts
                .insert(iface.clone(), imp.contract().clone());
            let arbitrates = imp
                .contract()
                .requires
                .get(arbitrate::ROLE)
                .is_some_and(|r| r.interface == arbitrate::steer_iface());
            b.implement(imp)?;
            // arbitration.v1 §2.2 (#909): the role's declaration read over
            // the revision held as soon as its contract is implemented,
            // before any of the instance's writers is declared or put (text
            // 0.3, F-274): an invalid one stops the owner here, `not
            // started: … invalid`, nothing declared or put; start() reads it
            // again.
            if arbitrates && let Some(steer) = &self.arb_steer {
                b.arbitrate(arbitrate::ROLE, Arc::clone(steer))?;
                arbitrated = true;
            }
            for r in resources {
                let name = zenkey::implementation::resource_name(&r);
                let raw = |t: &TypeId| matches!(t, TypeId::Raw { .. });
                match &r.body {
                    Body::Data(d)
                        if r.kind == Kind::State && !r.template.has_params() && raw(&d.type_) =>
                    {
                        let w = b.declare_state_writer(&iface, &name, &none).await?;
                        states.push((iface.clone(), Arc::new(w)));
                    }
                    Body::Operation(o) => {
                        let echo = raw(&o.request) && raw(&o.response);
                        // Over the whole template when it has parameters (§5.1),
                        // else on the operation's one key.
                        let values = (!r.template.has_params()).then_some(&none);
                        let calls = Arc::clone(&self.calls);
                        let named = opts.name.clone();
                        servers.push(
                            b.serve(&iface, &name, values, move |call| {
                                let calls = Arc::clone(&calls);
                                let named = named.clone();
                                async move {
                                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                    if !echo {
                                        // §5.2: `app` from any operation, and no detail
                                        // where there is no `error` type to carry one.
                                        return Err(OpError::app_without_detail(
                                            "this interop owner decodes no schema",
                                        ));
                                    }
                                    // --name: the instance answers with its name.
                                    let body = match named {
                                        Some(n) => n.into_bytes(),
                                        None => call
                                            .payload()
                                            .map(|p| p.to_bytes().into_owned())
                                            .unwrap_or_default(),
                                    };
                                    call.reply(body)
                                        .await
                                        .map_err(|e| OpError::internal(e.to_string()))
                                }
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
        // --arbitrate with no contract requiring steer.v1 as `cmd`: the
        // runtime's refusal, as before anything is put.
        if !arbitrated && let Some(steer) = &self.arb_steer {
            b.arbitrate(arbitrate::ROLE, Arc::clone(steer))?;
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
        // GET made the moment they appear finds it. A governed interface's at
        // its takeover, before its token (redundancy.v1 §2.6).
        let elected: std::collections::BTreeSet<&IfaceId> =
            governed.iter().map(|(i, _)| i).collect();
        for (iface, w) in &states {
            // A contract listing redundancy.v1 is never written before
            // start() checks its declaration (text 0.3, F-214): an invalid
            // one is refused there, `not started: … invalid`, and a valid
            // one is written at its takeover.
            if !gated.contains(iface) {
                w.put(value.clone()).await?;
            }
        }
        let red = match &self.red {
            Some(r) => {
                let mut st = RedundancyStartup::new(r.reader.clone())
                    .with_retry(Duration::from_millis(opts.retry_ms))
                    .with_grace(Duration::from_millis(opts.grace_ms))
                    .with_takeover(Arc::new(PutStates {
                        states: states
                            .iter()
                            .filter(|(i, _)| elected.contains(i))
                            .map(|(i, w)| (i.clone(), Arc::clone(w)))
                            .collect(),
                        value: value.clone(),
                    }));
                match (&r.watch, opts.poll_ms) {
                    (Some(w), _) => st = st.with_watch(w.clone()),
                    (None, Some(ms)) => st.poll = Duration::from_millis(ms),
                    (None, None) => {}
                }
                let external = governed
                    .iter()
                    .any(|(_, e)| *e == zenkey::redundancy::Election::External);
                let lock = external.then(|| r.lock());
                if let Some(l) = &lock {
                    st = st.with_lock(Arc::clone(l));
                }
                st.refuse_without_lock = opts.refuse_without_lock;
                Some((b.redundancy(st)?, lock))
            }
            None => None,
        };
        let svc = b.start().await?;
        if let Some((r, _)) = &red
            && opts.hold_heal
        {
            r.hold_heal(true);
        }
        let timing = if opts.timing {
            Some(timing_drive::Drive::start(&svc, self.session).await?)
        } else {
            None
        };
        let actuator = match &self.arb_steer {
            Some(steer) => {
                let angle = states
                    .iter()
                    .find(|(i, _)| *i == arbitrate::rudder_iface())
                    .map(|(_, w)| Arc::clone(w));
                Some(
                    arbitrate::Actuator::start(
                        &svc,
                        Arc::clone(steer),
                        self.arb_watch.clone(),
                        angle,
                        self.lines.clone(),
                    )
                    .await?,
                )
            }
            None => None,
        };
        let commander = if opts.commander {
            Some(
                arbitrate::Commander::start(
                    &svc,
                    self.session,
                    Duration::from_millis(opts.commander_lead_ms),
                )
                .await?,
            )
        } else {
            None
        };
        Ok(Up {
            svc,
            servers,
            _states: states,
            red,
            calls: Arc::clone(&self.calls),
            health,
            members,
            config,
            alarms,
            author,
            target,
            drive: drive_,
            timing,
            actuator,
            commander,
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

/// `desired seeds`' answer: the seeds made, and what the last one found
/// (§2.5, §2.9; text 0.4, F-197).
fn desired_seeds(seeds: u64, last: Option<&desired::Seeded>) -> String {
    let Some(s) = last else {
        return format!("desired seeds {seeds} none");
    };
    let (author, why) = match &s.author {
        desired::AuthorSeen::NotRead => ("not-read", None),
        desired::AuthorSeen::Present => ("present", None),
        desired::AuthorSeen::Absent => ("absent", None),
        desired::AuthorSeen::Incomplete => ("incomplete", None),
        desired::AuthorSeen::NotMade(why) => ("not-made", Some(why)),
    };
    let archive = match s.archive {
        None => "-",
        Some(true) => "answered",
        Some(false) => "nothing",
    };
    let mut out = format!(
        "desired seeds {seeds} answered={} author={author} archive={archive}",
        s.answered
    );
    if let Some(why) = why {
        out.push_str(&format!(" because {why}"));
    }
    out
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
            // The line as written, byte for byte, once it reads as strict
            // JSON (§2.10's reading): every number keeps its exact value
            // (text 0.4, F-194).
            if zenkey::model::desired::strict_json(rest.as_bytes()).is_none() {
                return Err(
                    "the document is not strict JSON (desired.v1 §2.10's reading): UTF-8, \
                     RFC 8259, no duplicate member, every number finite, at most 256 deep"
                        .to_owned(),
                );
            }
            let key = a.key(t).map_err(err)?;
            match a.put(t, rest.as_bytes().to_vec()).await.map_err(err)? {
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
        ["desired", "gets", t] => {
            let a = author()?;
            let key = a.key(t).map_err(err)?;
            Ok(format!("desired gets {key} {}", a.gets(t).map_err(err)?))
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
        ["desired", "seeds"] => {
            let t = target()?;
            Ok(desired_seeds(t.seeds(), t.last_seed().as_ref()))
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
             gets <target> | new-epoch | report | running | fail-next | refuse-next <code> \
             [<pointer>] | hold | release | reread <ms>|off | acts | seeds"
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

/// An interface's standing as `redundancy role` answers it.
fn standing_json(st: &zenkey::redundancy::Standing) -> serde_json::Value {
    let mut v = serde_json::json!({"role": st.role.as_str()});
    if let Some(why) = &st.why {
        v["why"] = why.as_str().into();
        if let Some(i) = why.instance() {
            v["instance"] = i.as_str().into();
        }
    }
    if let Some(f) = &st.fault {
        v["fault"] = f.as_str().into();
    }
    v
}

/// Answers a `redundancy` command (the module doc's), one JSON line, or
/// `refused <line>: <why>`.
async fn redundancy_command(
    line: &str,
    up: &mut Up,
    red: Option<&RedCtx>,
    claim: &zenoh::Session,
) -> String {
    use serde_json::json;
    let words: Vec<&str> = line.split_whitespace().collect();
    let refuse = |why: &str| format!("refused {}: {why}", words.join(" "));
    let (Some((r, lock)), Some(ctx)) = (&up.red, red) else {
        return refuse("redundancy.v1 takes part in no election here (--standby)");
    };
    let roles = || -> serde_json::Map<String, serde_json::Value> {
        r.governed()
            .iter()
            .map(|i| (i.to_string(), standing_json(&r.standing(i))))
            .collect()
    };
    let gets = |s: &zenoh::Session| {
        let g = zenkey::presence::liveliness_gets(s);
        json!({"reads": g.reads, "settles": g.settles})
    };
    let answer = match words.as_slice() {
        ["redundancy", "role"] => json!({
            "redundancy": "role",
            "instance": up.svc.instance().as_str(),
            "interfaces": roles(),
        }),
        ["redundancy", "reads"] => {
            let reads: serde_json::Map<String, serde_json::Value> = r
                .governed()
                .iter()
                .map(|i| {
                    let x = r.reads(i);
                    (
                        i.to_string(),
                        json!({"made": x.made, "complete": x.complete, "not_made": x.not_made,
                               "polls": x.polls, "last": x.last,
                               "poll_log": x.poll_log.iter().map(|p| json!({
                                   "made": p.made,
                                   "complete": p.complete,
                                   "holders": p.holders.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                                   "appeared": p.appeared.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                                   "withdrawn": p.withdrawn.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                               })).collect::<Vec<_>>()}),
                    )
                })
                .collect();
            let mut sessions = json!({"reader": gets(&ctx.reader)});
            if let Some(w) = &ctx.watch {
                sessions["watch"] = gets(w);
            }
            json!({"redundancy": "reads", "interfaces": reads, "sessions": sessions})
        }
        ["redundancy", "lock", how @ ("hold" | "lose")] => {
            let Some(_) = lock else {
                return refuse("no interface here is elected by external (§2.11)");
            };
            let hold = *how == "hold";
            let held = match &ctx.file {
                Some(f) => {
                    if hold {
                        f.want();
                        f.hold().await
                    } else {
                        f.release();
                        false
                    }
                }
                None => {
                    ctx.manual.set(hold);
                    hold
                }
            };
            r.lock_changed();
            json!({"redundancy": "lock", "held": held})
        }
        ["redundancy", "zid"] => {
            let mut v = json!({
                "redundancy": "zid",
                "zid": claim.zid().to_string(),
                "reader": ctx.reader.zid().to_string(),
            });
            if let Some(w) = &ctx.watch {
                v["watch"] = w.zid().to_string().into();
            }
            v
        }
        ["redundancy", "calls"] => json!({
            "redundancy": "calls",
            "calls": up.calls.load(std::sync::atomic::Ordering::SeqCst),
        }),
        ["redundancy", "hold-heal", on @ ("on" | "off")] => {
            r.hold_heal(*on == "on");
            json!({"redundancy": "hold-heal", "held": *on == "on"})
        }
        [
            "redundancy",
            "wait",
            iface,
            role @ ("serving" | "standby"),
            ms,
        ] => {
            let Ok(iface) = iface.parse::<IfaceId>() else {
                return refuse("not an interface id");
            };
            let Ok(ms) = ms.parse::<u64>() else {
                return refuse("a wait in milliseconds");
            };
            let want = if *role == "serving" {
                Role::Serving
            } else {
                Role::Standby
            };
            let reached = tokio::time::timeout(Duration::from_millis(ms), r.until(&iface, want))
                .await
                .is_ok();
            json!({
                "redundancy": "wait",
                "iface": iface.to_string(),
                "reached": reached,
                "standing": standing_json(&r.standing(&iface)),
            })
        }
        ["redundancy", "new-epoch"] => match up.svc.new_epoch().await {
            Ok(id) => json!({"redundancy": "new-epoch", "instance": id.as_str()}),
            Err(e) => return refuse(&e.to_string()),
        },
        _ => {
            return refuse(
                "usage: redundancy role | reads | zid | lock hold|lose | calls | hold-heal on|off \
                 | wait <iface> serving|standby <ms> | new-epoch",
            );
        }
    };
    answer.to_string()
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
    let (commands, stdin_closed) = command_lines(|| std::io::stdin().lock());
    if let Err(e) = run_with(opts, |line| println!("{line}"), commands, stdin_closed).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
