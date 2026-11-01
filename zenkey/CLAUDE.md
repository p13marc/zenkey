# zenkey

Claude Code loads this file when working in `zenkey/`; the root `CLAUDE.md` is the index.

## v1

v1's `zenkey` (0.11.x on crates.io) is the `v1` branch's, and that branch's
`CLAUDE.md` describes it. Nothing below applies to it.

## zk2

- **`zenkey/`** 0.20.0 (MIT, unpublished) is zk2's session half (#610,
  FE): `ServiceBuilder` → `start()` in spec §8.2's order (exposure checked,
  descriptor D-checked and served, bundles on `complete` queryables, the
  instance token, then the interface tokens), and the running `Service`
  (capabilities, make-before-break `new_epoch`, member tokens). `presence`
  reads liveliness on an unbounded flume handler (zenoh#2678), and
  `retrieval` is §8.4. `writer` puts with the contract's QoS, every
  `stream`, `state` and `@state` sample sealed in §7.5's envelope under
  `zenoh/bytes` (an `@stream` sample keeps its record alone under its
  predefined `Encoding`, `Writer::record_encoding` the record's; core
  §4.3, §7.2), and a Zenoh timestamp only on §4.3's table
  (`Writer::is_stamped`: a state with `freshness.ttl_s` above 0, a
  desired resource, a state metric resource, a lifespan's stream, health's
  fault, or every write of a session whose timestamping is enabled)
  (advanced publication for `history`, which always numbers its samples,
  core §2.5; events on fresh ULID keys, each under its envelope;
  `reliability` through zenoh's `unstable` feature, which the workspace's
  `Cargo.toml` turns on: core §2.4 names it the one QoS field that is
  unstable API in zenoh 1.10.1; an owner that opts in sets it, and one
  that does not, or whose binding offers none, MAY publish reliable,
  zenoh's default (0.68, F-380), and Appendix B says so, read from zenoh
  1.10.1's source), and
  `consumer` reads through a role's bindings: R2 parameter binding, R6's
  wildcard-key discard, R5's presence wait, R7's *unobservable*; its
  presence (`Consumer::present`/`presence`/`wait_for_provider`/`liveness`,
  and `Client`'s and `Fleet`'s `present`/`presence`) is
  `presence::serving`'s: an interface token, or a tokenless interface's
  instance token and a descriptor marking it `"token": false`, absent only
  from a complete read, `Liveness::Unobservable` otherwise, a descriptor
  not read in `Present::undescribed` with why in `errors`, and no read
  past `wait_for_provider`'s deadline, a round it cuts short keeping what
  it found (#1015); tools
  consume with `Consumer::for_tool` and read presence with
  `presence::liveliness_read`, which flags a GET that ran to its timeout,
  or whose session was cut off while it ran, as possibly incomplete (§8.1;
  `PresenceRead::cut_off_during`, core 0.38: the session is checked again
  when the read ends, by its links, since zenoh 1.10.1 still lists a
  closing transport in `transports()` when it answers the read at the
  cut); every liveliness subscriber goes through
  `presence::declare_subscriber`, which marks its session a watch session
  for good (the tools' through `zenkey_fleet::WatchSession`, in no
  namespace on the wire keys, and `scripts/check-liveliness-seam.sh` keeps
  every tool-side subscriber, barrier and watch session there, #875; the
  runtime's owner-side watches keep `subscriber_needs_history`), and
  `liveliness_read` refuses one (`Error::ReadBesideSubscriber`)
  and a session cut off (`Error::CutOff`), with `presence::settle` a watch
  session's one barrier GET (#830, core 0.37), asking `barrier_key(scope)`,
  the scope's chunks to its first `@zk` and then `@zk/settle`, a key no
  token holds (#851, core 0.41), or a key given, on the wire
  (`settle_at`, `Barrier::at`, #875), and `presence::Barrier` that GET kept,
  lost at the session's next transport event, its round trip one per
  transport state on a task of its own that every waiter shares
  (`Barrier::establish_within`: a waiter that stops waiting leaves it
  running and the next takes it up, its GET waiting the session's
  interests timeout; #882, core 0.44), and never asked at once at an
  open: within `presence::OPEN_MARGIN` (100 ms) of an open that is the
  session's latest transport event the round trip waits out the rest on
  its task, a move meanwhile ending it unasked (core §8.1, 0.56, #914:
  zenoh 1.10.1 tells an open before the session's router face declares
  the subscribers' interests again; `tests/presence.rs`'s
  `a_barrier_round_trip_after_an_open_follows_the_reopen`, 60 of 60); a
  GET whose time reaches the
  session's `routing/interests/timeout` is possibly incomplete
  (`PresenceRead::at_interests_timeout`: zenoh 1.10.1 ends it there with no
  error reply), and the tools' sessions run with 120 s
  (`zenkey_fleet::TOOL_INTERESTS_TIMEOUT`) unless their config states one
  (every liveliness GET goes
  through `presence` too, counted per session by
  `presence::liveliness_gets`, and `scripts/check-liveliness-seam.sh`
  keeps the seam the only one). The
  archive hears owners on `ArchiveConfig::watch` or reads presence every
  500 ms, and R5's wait reads presence. `state`
  stamps every put and delete in band (the `Minter`, core 0.73 §4.3, #976
  A3: `Stamped { stamp, time }`, which `StateWriter::put`/`delete` and the
  typed handles return; one state and event seq per run, never wrapping:
  before it would pass 2^24 − 1 the minter takes a new run and puts the
  descriptor stating it under its own lock, then hands out seq 0,
  `Minter::simulate_seq` for tests; one seq per stream key per run,
  wrapping, kept across a publisher declared again, a takeover or a
  member's writer made again, and a new run past 65,536 stream keys; the
  run from `run::RunStore`, `FileRunStore`'s 2-byte record made durable
  before use, spent at 0xFFFF, an unreadable one read as spent and never
  reset, `ServiceBuilder::run_store` refused once a run is taken and, at
  `start()`, beside `redundancy()`; else a random 32-bit run, drawn again
  while it equals one used; `new_epoch` takes a new run, puts the new
  instance's descriptor stating it, then alarms' and desired's re-puts
  under it, then the tokens; §4.3's table's Zenoh timestamps still minted
  above the last, catch-up included; the clock guard still stops every
  state write; `start()` reads the session's timestamping for its mode,
  `Minter::session_timestamping`, and warns naming core §8.2) and answers
  GETs with the bytes put, a `reply_del` whose attachment is the delete's
  envelope, each with the mutation's Zenoh timestamp where it had one,
  within the window; `Consumer::get` is S4's owner-only
  `All`+`Latest` GET, silence being `StateGet::Silent`; `ValueOrder` is
  S7's consumer rule, per key, a value under an id other than the held
  value's accepted, handed every value from a consumer's first on, a
  seed's replies included (core 0.68; since 0.73 by run and seq,
  `state.rs`'s `s7_late_joiners_across_runs`); `Consumer::get` returns
  every reply at the GET's end, so inside a seed a consumer holds the
  value of the run that answered until the key's next value (core 0.69,
  `s7_late_joiners_across_runs_inside_the_seed`). `archive` is a
  minimal `archive.v1` (§4.4): records, refuses outdated puts, aligns on
  positive evidence only, and `last_known` reads it as last-known, never
  current (`LastKnown::inband` and `mark()`, its in-band stamp, A3; the
  typed reads alike since A4: `typed::LastKnown::inband`/`mark()`, and
  `typed::get_answer` → `typed::StateAnswer` keeping each key's in-band
  stamp, which `typed::get` and codegen's `get_*` drop, a generated
  consumer reaching it through `inner()`).
  `inband` (#976 A2) reads every data sample as core §4.3 writes it: an envelope's record as the payload and its run and seq as `Delivery::inband`/`StateAnswer::inband` (`mark()`, `ValueOrder::accept_mark`, the one way in since A4 removed `ValueOrder::accept`); a refused one undecodable, never delivered (`Subscription::undecodable`); a sample with no envelope (an `@stream` sample, a foreign key's, one that broke §4.3) read as it arrived and ordered by its Zenoh timestamp (core §4.2); the archive orders by it, refusing an outdated put only within a run.
  Operations (#621, FH): `operation` serves a `complete` queryable that
  refuses fan-out (O2) and answers `unavailable` with its cause before any
  handler runs (O3), and a call left without its reply gets `internal`,
  never silence; `client`'s `Client` calls one explicit address
  (`BestMatching`, retrying only idempotent operations, O4), and its
  `Outcome` keeps a value, an envelope and silence apart (O5); `Fleet` fans
  out only to `fanout = "allowed"` (`All` + `None`), attributing each reply
  by its key (O6); `ownership::split_brain` is §6's token check. At most
  once holds only while one instance serves: exclusivity is
  `redundancy.v1`'s (#613).
  Everything session-free stays in `zenkey-model`, including `decode`, the
  bundle-only decoder tools use (§7.2).
  Codegen's runtime half (#611, FI): `codec` (a Rust type and how it is
  carried), `typed` (the generic handles generated code is written
  against), and `call`, the session-free vocabulary (`OpError`,
  `CallInfo`, `Sink`, `Outcome<T>`, `Replies<V, S>`). Packed records
  (#979 B2, core §7.6): `packed` (session-free) is what a generated packed
  type is written against (`PackedRecord`, `Coded<T>` for reserved codes,
  `Open<E>` for an escape, the model's `BitWriter`/`BitReader`/`Fixed`/
  `time_code` helpers), and `codec::Packed<T>` carries one, its `time`
  fields under `DEFAULT_EPOCH` 0 until #980's catalog carries one
  (`Codec::decode_record` by the envelope's tag, `Codec::tag`, a typed
  writer refusing a type of another tag). Each writer of a packed resource
  (`writer::PackedType`, from the contract's artifact) seals with its
  layout's tag in CH (`Stamped::seal_tagged`) and refuses a record its
  layout refuses, `Error::Payload`, nothing published; `inband::admit`
  (and the consumer's subscribe, replay and GET) refuses a record by its
  layout under the tag CH carried, a packed put with no envelope as `tag`
  (`inband::Undecodable`, `Delivery::tag`, `StateAnswer::tags`);
  `tests/packed.rs` runs `spec/conformance/packed/vectors.json` through it
  and refuses another revision live by tag and by length
  (`tests/contracts/packed/`, `packed-wide.v1`, `packed-collide.v1`).
  The default `zenoh`
  feature gates everything holding a session; without it the crate is the
  model, `config`, `Implementation`, `call` and `codec`, so a contract
  crate's traits build without zenoh.
  `hostid` (#719, a feature of its own that `zenoh` turns on: it reads the
  host and holds no session) is `hostid.v1`'s runtime: `ServiceBuilder::new`
  resolves `address = "@hostid.v1/<svc>"` through `HostIdMinter::global()`,
  once per run, and spells out `self.system/<svc>` providers (core 0.20);
  a test hands `ServiceBuilder::with_hostid` a minter over a temp root,
  never the host's `/etc/machine-id`.
  `freshness.v1` (#720): a `StateWriter` on a resource whose contract
  declares `freshness.ttl_s` above 0 re-puts its member unchanged, under a
  fresh `Minter` stamp, whenever 95 % of ttl/2 passes without a put; on by
  default (the contract asks it of every owner), off through
  `set_refresh(false)`, stopped by the writer's drop, the service's close
  or drop, a delete, and the clock guard. The judgement is
  `zenkey_model::freshness`; `Subscription::freshness`/`clock_trust` and
  `Current::freshness` make its observations.
  `health` (#721, PE) is `health.v1`'s owner on the embedded standard
  bundle: `ServiceBuilder::health()` → a `Health` (also
  `Service::health()`) whose status is the declared level raised to the
  worst check, with §2.2's put order, put before step 3 (`UNSPECIFIED`,
  `"starting"` by default) and never deleted; checks on change, faults on
  the stream, and `clock_ahead` while the guard holds (the `Minter`'s hold
  watch). The owner example drives it from stdin.
  `config_v1` (#768, CB; `config` is the deployment's) is `config.v1`'s
  owner on the embedded standard bundle: `ServiceBuilder::config_v1(startup,
  device)` wraps `zenkey_model::config`'s `Owner` over the application's
  `Device` (apply, undo, persist; async, fallible, run on a task of its
  own), serves the five operations, puts the view and the status before
  the tokens and after every step that moves them, runs the deadline timer
  at `Owner::next_due`, and converges desired state; a restart is a new
  service from `Startup` (`new_epoch` is refused while it runs), and the
  owner example's `--config` drives it from stdin.
  `alarms` (#800, AB) is `alarms.v1`'s owner on the embedded standard
  bundle: `ServiceBuilder::alarms(Startup)` → `Alarms` (also
  `Service::alarms()`). `raise(Condition)` makes §2.5's act from what is
  held — raised, escalated, refreshed, or unchanged with nothing put —
  keyed by `zenkey_model::alarms::derive`; `resolve` puts the resolved
  document, then deletes at once; each firing alarm's `StateWriter`
  confirms it every 427.5 s from the contract's ttl until
  `set_confirming(key, false)`; a raise past the bound is the budget's
  `Error::Budget`, nothing evicted. A restart adopts inside `alarms()`,
  before anything is alive: from the application's `Record` (`FileRecord`,
  a JSON file), else an explicit archive read, never an S4 GET of its own
  keys, retiring, resolving and re-putting in §2.9's order. `new_epoch`
  re-puts every alarm first, a clean close touches none, `alarms_with`
  takes another revision (a test's short-ttl copy), and the owner
  example's `--alarms` drives it from stdin (`alarm raise|resolve|confirm|
  list|adoption`).
  `telemetry` (#819, TB) holds an owner to `telemetry.v1`: every
  `Writer`/`StateWriter` of a metric resource (declaration `declared`, raw
  excepted) decodes each put strictly (`zenkey_model::telemetry::Datum::
  decode`), runs the model's `check`, and refuses an in-band epoch other
  than its own and a counter or histogram below the last value it put in
  the epoch (judged by `pair`; a delete keeps it, since a reader pairs
  across one), all `Error::Telemetry`, nothing published; it fills a
  counter's or histogram's Point `epoch`, and a bare one's Epoch
  attachment, with the instance id, or the member's epoch on the template
  that declares `epoch`; `new_epoch` moves it. No off switch: a raw
  publisher or the session is unchecked, which is how a test writes what
  the text refuses. The owner example's `--telemetry` embeds `meter.v1`
  and drives it from stdin (`telemetry put|raw|new-epoch|instance`).
  `desired` (#840, DC) is `desired.v1`'s runtime over
  `zenkey_model::desired`'s state machine.
  `ServiceBuilder::desired_author(iface, resource, AuthorStartup)` →
  `Author` puts a target's document only on a content change within the
  instance (`Written::Unchanged`), deletes on its own word, and keeps a
  `Record` (`FileRecord`: each document and each delete within the window,
  with stamps). At a restart it catches the minter up, makes each kept
  delete again (its tombstone aged to what is left of its window,
  `StateWriter::delete_aged`), then puts each document again before the
  instance token, or paced (`Pace::Spread`), never from a GET of its own
  keys; `new_epoch` re-puts them first. `ServiceBuilder::desired_target(
  role, author, resource, ReportAt, Handler, TargetStartup)` → `Target`
  reads the role's binding by `model::binding` (an optional role left
  unbound is the kill switch: `{"source":"baseline"}`, nothing read; a
  role on two desired resources (a requirement naming none, or a
  manifest's role, names every one) refused `not started: …`
  (`Error::NotExposed`), and `start()` refused too, nothing declared,
  core §8.2 step 2, #840 DF) and
  puts its baseline report before the tokens. From `start()` on it
  subscribes to the slice and seeds by S4's GET at start, at each
  appearance of the author's instance token (a watch session's subscriber,
  without history unless the session is in a namespace, its round trip kept
  and asked before a presence read, a read made without it possibly
  incomplete, a transport move a reason to seed, core 0.41, #851; else a
  500 ms poll; retried while the author is present and silent) and
  on `set_reread`. It reads the archive's last-known only after a complete
  read shows the author absent (core §8.1, 0.37), checks each act against
  the author's bundle (`undecodable`, `invalid`) and runs it on the
  application's `Handler` on a task of its own, putting the report after
  every change. `desired_config` is config.v1's owner as a target (a
  delete reverts what the documents set to the startup values, and the
  view drops `desired`; its re-mint is a restart, desired.v1 §2.16, so
  `new_epoch` stays refused while it runs and a restart acts again,
  scenarios §12, `s12_a_config_v1_owners_re_mint_is_a_restart`). The owner example's `--desired-author` and
  `--desired-target` drive it from stdin (`desired put|delete|report|…`);
  `desired put` puts the line byte for byte once it reads as strict JSON,
  `desired seeds` says what the last seed found, and `desired gets` counts
  the GETs that reached a slice (`Author::gets`, text 0.4).
  `redundancy` (#841, RB) is `redundancy.v1`'s candidate:
  `ServiceBuilder::redundancy(RedundancyStartup)` → `Redundancy` (also
  `Service::redundancy()`) elects every governed interface the service
  implements (its contract lists `redundancy.v1` in `uses`), each by
  `zenkey_model::redundancy`'s `Elector`, the runtime deciding nothing
  itself. The claim is `start()`'s steps 3 and 4 (the descriptor marking
  the interface `"standby": true`, the instance token, no interface
  token), then a liveliness watch on `@zk/**` on the watch session handed
  in (else a poll every 500 ms) with its round trip, and a round trip on
  the claim's session; every read goes on `RedundancyStartup::reader`, a
  session holding neither a liveliness subscriber nor the claim (text
  0.2, F-204: a session can answer its own tokens even through a refusing
  router, not always, #992). A governed interface's publishers, state,
  operation and
  `unavailable` queryables and member tokens are slots of a crate-private
  gate, declared only while it serves; a write while it stands by is
  `Error::Standby`, nothing published, and raw `declare_publisher`/
  `declare_queryable` are refused on it. A takeover declares the
  resources, runs the application's `Takeover` hook, puts the descriptor
  unmarked and declares the token last; a step-down stops its writes,
  withdraws the tokens, undeclares the resources and forgets their state,
  then puts the descriptor marked. `external` asks a `Lock` (`FileLock`'s
  flock, `ManualLock`) at the start, every second and on `lock_changed()`,
  and `refuse_without_lock` refuses to start without it; `new_epoch` tells
  each elector first (F-209). A governed interface without
  `redundancy()`, an invalid declaration, or a governed interface in the
  tokenless set is refused at start, nothing declared; `zenctl gen` and
  `serve` elect (RC). Its scenarios §1–§11
  are `tests/profile_redundancy.rs`, and the owner example's `--standby`
  drives it from stdin (`redundancy role|reads|lock|calls|hold-heal|wait|
  new-epoch|zid`, one JSON line each; `reads` gives each poll's `poll_log`;
  `--wait-go` opens every session and starts at a `go` line, text 0.3,
  F-216, reading stdin from its start, `command_lines`, F-220).
  `Service::holds_member` (#841, RD) says whether a member token is held,
  so an application declaring members at each takeover never cycles one
  by declaring it twice.
  `timing` (#908, MB) is `timing.v1`'s runtime over
  `zenkey_model::timing`, never re-deriving a verdict. Every `Delivery`
  carries its `arrival` (an `Instant` read in the subscriber's callback),
  its `stamp` and its `lifespan`'s judgement. A subscription to a timed
  stream runs an engine: a `Tracker` per provider address, member = key,
  a one-shot timer at `next_due` re-armed at each change, never an idle
  tick; the owners' interface and member tokens followed on a watch
  session handed in (`Consumer::with_watch`), each withdrawal leaving
  none read again 100 ms later on the consumer's own session, absent
  only by a complete read, a member token seen at the seed, heard
  declared or withdrawn; a `LifespanReader` per provider under ε, the
  binding's `clock_bound_ms` (`Consumer::with_clock_bound` for a tool),
  `meta.zid` from the descriptors' puts, a GET at the start and again at
  an id not held. An expired sample is discarded, counted
  (`Subscription::expired`) and no arrival.
  `Consumer::subscribe_timed` hands `TimingEvent` (missed, met, ended,
  beyond_bound, expired, an expired one with its arrival's `wall`, text
  0.3, F-276) to a second callback, in order, from a task of its own;
  `subscribe` drops them, so its callers are unchanged. Without
  a watch session the owner stays unseen: no never-heard wait, a member
  ended only forgotten. A writer of a stream declaring a lifespan stamps
  each put from the service's `Minter` clock, its id the session's zid
  (`Writer::is_stamped`, `Writer::timing` for the period), and a
  configuration stating another `meta.zid` is refused at start, nothing
  declared. Scenarios §1–§5 are `tests/profile_timing.rs` over
  `tests/contracts/pace.v1.toml`; the owner example's `--timing` (with
  `--no-hlc`) drives `pace.v1` from stdin, and `consume --timing` is
  scenarios.md's C as a black box, one JSON line per event. Since core
  0.73 the owner example's session runs without timestamping unless
  `--hlc` or `--standby` (§8.2), `--run-file PATH` takes its runs from a
  `FileRunStore`, and a `seq <n>` line sets the next seq (`state.md` §7
  step 3).
  `arbitration` (#909, BB) is `arbitration.v1`'s runtime over
  `zenkey_model::arbitration`'s `Arbiter`, deciding nothing itself.
  `ServiceBuilder::arbitrate(role, revision)` reads the role's declaration
  over the revision of the required interface held at once, refusing an
  invalid one with the start's own error (text 0.3, F-274: so an owner
  that puts a state's value before its tokens calls it first and puts
  nothing of an instance that never starts), and `start()` reads every
  role carrying `arbitration.policy` again (checks 1 to 5 without a
  revision), refusing an invalid declaration with nothing declared: `not
  started: <address>: role "cmd" (steer.v1): arbitration.v1 invalid:
  <reason>`.
  `Consumer::arbitrate(on_event)` subscribes the role's one stream per
  entry (a sample two entries' subscribers hear handed over once, by the
  first), hands the arbiter each sample at its instant (never earlier
  than the last event's), `timing.v1`'s expiry from `subscribe_timed`'s
  engine with its deadline left to the arbiter, a governed interface's
  sample not the incumbent's `not_incumbent` (the lowest holder's
  `meta.zid`, its holders read once at start, redundancy.v1 §2.10), and
  tells `ArbitrationEvent::{Switched{from, to, cause, at, last},
  Delivered{source, delivery}}` in decision order from a task of its own,
  the `start` switch to the dead-man first. Withdrawals and declarations
  come from a liveliness subscriber per entry on the watch session handed
  in (`Consumer::with_watch`), without one the deadline alone; a read again
  0.1 s after a withdrawal leaving none known held, on the consumer's
  session. A move begins at the watch session's transport event or at a
  withdrawal heard while it holds no link (F-268, core 0.56); its round
  trip is asked through the barrier, which asks the one after an open
  `presence::OPEN_MARGIN` (100 ms) after it (an open is told before the
  subscribers' interests are declared again, core 0.56, #914), what the
  watch heard before
  the reply taken first, and asked again until one stands. A one-shot
  timer at `Arbiter::due`. `Arbitration` reads `active`, `counts`,
  `discards`, `reads`, `round_trips`, `moving`, `watching`. Scenarios
  §1–§9 and §11 are `tests/profile_arbitration.rs` over
  `tests/contracts/{steer,rudder,steer-governed}.v1.toml`, §7 with its
  control; `tests/walkthrough.rs`'s `w4_2_a_control_loop_arbitrated` is
  architecture §4.2. The owner example's `--arbitrate <entry>,...` is an
  actuator printing its switches as JSON lines and answering `arbitrate
  counts|state|reads`, refused before anything of it is declared or put
  (F-274), and `--commander` a commander (`commander
  hold|resume|raw on|off|sent|new-epoch|zid`, its first put 500 ms after
  its tokens, `raw on` putting `standby <n>` round its writer, scenarios
  §11's `X`, F-276).
  `tests/` runs `spec/scenarios/` sections as in-process routers and
  clients, one test per section, named after it; `common::Link`, the TCP
  proxy a test cuts, stalls and heals, also counts and can copy what it
  forwards toward the side that connected (`sent_down`, `capture_down`),
  which `constrained.rs`'s `s7_keys_across_the_face` reads (core §8.5,
  #980 C1b: 45–46 B a put by exact key, 82 B under `zk2/**`). `tests/budget.rs`'s §1
  routers state `wait_before_close` 5 s (`ROUTER_WAIT_BEFORE_CLOSE`),
  never zenoh's default, which zenoh-modem's fork `zm-1.10.2-r1` makes
  10 s (#1011). v1's `zenkey` 0.11.x stays
  on crates.io, from the `v1` branch; no member of `main` links it.
