# zenctl

Claude Code loads this file when working in `zenctl/`; the root `CLAUDE.md` is the index.

- `zenctl/` — the **bus explorer CLI** (Apache-2.0, **not published**:
  Forgejo release binaries via `release.yml` / `cargo install --git`; 0.1.x
  stays on crates.io un-yanked): app-neutral, and zk2's since FJ9 (#612),
  when what was left of v1 went — `why`, `check cutover|retired|conform`,
  `config`, `blob`, `export`, `--registry` and the slice cache — and the
  binary stopped linking v1's `zenkey` (`why` and `check conform` came back
  at FK1, `config` at #768). Verbs read contracts from the
  bundles the bus serves (spec §8.4) or `--contracts <path>`; resolved verbs
  open their session in the deployment namespace — `--namespace` (alias
  `--base`) resolves flag > env `ZENCTL_BASE` > the active named context
  (`zenctl context create …`) > **empty**; raw verbs (`get <selector>`,
  `echo`, `pub`, `admin`, `storage list`, `namespace list`) stay
  un-namespaced. Raw `get` resolves each reply through the lens, as `echo`
  does; `pub` and `replay` write a foreign key as bytes, on recorded or
  `--qos` axes (`priority/congestion/reliability[+express]`), a tombstone
  always behind `--i-know`. FJ5's acts and reads go
  through the runtime's own client and consumer: `call` (one address
  `BestMatching`, a fan-out only to `fanout = "allowed"`, refused before
  anything is sent otherwise), `get state` (the owner's S4 GET, or
  `--last-known <archive>`, never shown as current) beside the raw `get
  <selector>`, `watch` (R6 discards counted apart), `replay --namespace`;
  `pub` refuses a key a zk2 service owns (P3), and `retire` is gone.
  FJ6's `doctor` judges a zk2 deployment against the core: thirty-three
  checks since #909's BC (`zenkey_fleet::judge::doctor`, ids in `report::CheckId`,
  an appendable vocabulary whose JSON Schema is one open string branch
  since `zenwatch.v1` 1.1, so a new id leaves that contract unchanged), each a
  `Judgement` whose finding is the yes, read through a session in the
  namespace and one in none for the admin space and the presence domain
  (the admin space through `@/*/router` and the storage manager's status
  subtree, `bus::admin::STORAGES`, never a selector
  `zenkey_fleet::reaches_tables_lock` finds, which zenoh 1.10.1 answers
  under its routing tables' read lock: core §4.2, 0.48, #890);
  exit 1 on a finding at or above `--fail-on` (default warning), 2 when a
  check is unobservable or the scope is empty. The watchdog's `doctor` rule
  and `record --on` run it too.
  FJ7's `acl gen` compiles spec §11's grants offline from `--contracts` and
  an enrollment (`examples/zk2/acl/`: principals by user or CN, never a
  zid, running services, archives and tools with their bindings and calls,
  and a tool's `inspects`, presence alone, since core 0.27; a principal
  holds the union of what it runs) in `zenkey_fleet::model::acl`; a tool
  enrolled with `admin` holds the admin read, `query` and `reply` on
  `@/*/router`, the storage manager's status subtree and `@/*/*`
  (`model::acl::ADMIN_READ`, held equal to `bus::admin`'s `ROUTERS`,
  `STORAGES` and `TOPOLOGY`), never a key `reaches_tables_lock` finds, so
  under `deny` a router refuses a GET of its declaration records from any
  principal at its ingress, complete and empty (core §11.1, §11.2, 0.50,
  #894; `tests/acl_live.rs`' `s4_the_admin_read_reaches_no_declaration_
  record`, beside no writer); a
  wildcard presence selector joins each provider's egress, since the
  descriptor read travels to it (0.27); `--default-permission allow` compiles each
  grant into denies of its complement. `--face-policy <FILE>` (#769 LB)
  guards the router's constrained faces from a `link.v1` face policy
  (`examples/zk2/link/`, `examples/zk2/acl/walkthrough.face.toml`; the old
  `--face/--attach/--far/--region` are refused by name, exit 2): each
  face's plan (`zenkey_model::link`, carried whole in `AclPlan::faces`) as
  rules `face:<face>:<id>` in the far principal's policy alone, which loses
  its presence, history, admin-space and contract grants; every other
  principal's subject narrowed to the link protocols outside the faces
  whose far principal it is not (`AclSubject::link_protocols`; one left
  with none refused; a face's far principal is placed by its user name or
  CN, never by its id: link.v1 0.5, F-121, #769 LE; and, face or no face,
  an enrollment binding a principal by a `user` and a `cn` both is refused
  whole, exit 2: core 0.33 §11.1, #788),
  since zenoh ties no credential to a link, and under
  `allow` a subject `face:<face>` per face, selected by its link protocols
  alone and listed in the far principal's policy (link.v1 0.3, F-118, #769
  LC); zenoh's `downsampling` block beside `access_control`, one rule per
  crossing stream key; for a south region `gateway.south` and
  `face-declarations` over every queryable the far side may query as its
  owner declares it (`state/**`, an operation's pattern; core §8.5, 0.30).
  Several revisions of a listed interface go to the model, which lets a
  resource cross only where every one exposes it and warns
  `exposure_disagrees` (F-119). Two faces naming one far principal each
  carry both plans' denies, since no zenoh rule is limited to a link
  protocol: written so, and warned (`far_of_two_faces`, a linter warning
  with no face, never a refusal; text 0.7, F-125, #769 LH). Router to
  router is the policy's `router_attach`, refused. `check face
  <ROUTER-CONFIG> --face-policy …
  --enrollment … [--window ADDRESS=SECS]` is its judgement
  (`zenkey_fleet::check_face`, report `FaceCheck`): `link.v1` §2.15 per face,
  `block_disabled` for access control not enabled (`enabled` false or
  absent, read as zenoh's loader reads it; beside every other finding, and
  beside the comparison's `disabled`: text 0.7, F-124, #769 LH), the
  denies read subject by subject; over every subject and the subject
  list of every policy (text 0.4, F-120, #769 LD), `near_subject_on_face`
  (bound to another principal, whatever else it names, or naming a user
  name and a CN both: text 0.5, F-121, #769 LE),
  `face_subject_beyond_face` and `face_subject_misplaced` (a subject bound
  to nobody on the face must select its link protocols alone and ride
  exactly the far principal's policies, those of its subjects naming it
  alone in one list and nothing in the other), and under `allow`
  `face_subject_missing` and `narrowed_too_far` (no face: a name whose
  session over a link protocol it is not kept off matches no subject, user
  names and CNs read apart since 0.5); the
  windows of §2.11 when given, and `acl gen --check`'s comparison, which
  now covers the downsampling items and a subject's link protocols. Its
  plans are pinned (`tests/fixtures/acl/`, `ACL_BLESS=1`; every face plan
  passes its own `check face` in both postures) and judged by a live router
  (`tests/acl_live.rs`, `tests/acl_face.rs`, and `tests/link_face.rs`,
  `link.v1`'s scenarios §1–§13 one test per section over a `unixsock-stream`
  face, §7 in both postures, §9 and §10 the hand-edited blocks 0.4
  refuses, §11 those 0.5 refuses, one session over mTLS presenting a CN
  beside its user name, with certificates minted per run by `rcgen`; each
  with what it leaks; §12 the block's text without what zenoh defaults,
  `enabled`, `default_permission` and a rule's `flows`, and §13 two faces
  of one far principal, 0.7's).
  FJ8a's writes and captures: `gen <address>` and `serve <address> <iface>
  <operation>` are mock owners — real zk2 services through
  `zenkey::ServiceBuilder` (`zenkey_fleet::tape::{generate, mock}`), refused
  at an address whose instance token is present unless `--i-know`, marked
  synthetic in the descriptor's `meta`; `gen` synthesizes every member
  deterministically per (seed, tick) (`tape::synth`: the §7.3 JSON Schema
  subset, validated; protobuf with every field filled; raw bytes of the media
  type's size class; a metric resource's value by its declared kind, a
  counter or histogram never decreasing, so the runtime's checked writer
  accepts it, #819 TB). `bench call` times each reply where it arrives and
  attributes it by key, envelopes and silence counted apart. `.zrec` is
  version 3 (header `excluded`, `@op`, `@zk` and `@adv` alone under
  `--explicit`; rows `qos_axes`; 1 and 2 still read), and
  `record --on` refuses the rules that judge v1.
  FJ8b's observers and checks: the raw observers (`echo`, `rate`, `field`,
  `timeline`, `snapshot`, `watchdog`, `record --on`; `watchdog`, `record
  --on`, `alarms --for`, `telemetry` and `check conform` hold their
  liveliness subscribers on `Deployment::watch_session()`, #830, a
  `zenkey_fleet::WatchSession` in no namespace on the wire keys, #875;
  `gen` and `serve`'s candidates keep a namespaced watch, the runtime's)
  keep a wire selector in
  no namespace (the default `<ns>/zk2/**`; `--explicit` on `record`,
  `record --on`, `echo` and `timeline` adds `<ns>/zk2/*/*/*/@stream/**`
  and `<ns>/zk2/*/*/*/@state/**`, on `snapshot` the `@state` one alone,
  every verb's scope from `cmd::zk2::wire_scope`, refused beside a typed
  selector, exit 2; the three are the data plane but `@op` and `@zk`,
  `observer.md` §8.2, #924; `rate` and `field` keep one selector; with
  `--across-face SYSTEM/SERVICE`, repeatable, on all eight, the scope is
  `cmd::zk2::FaceScope`'s, core 0.76 §8.5 and observer 0.16 §7.5, #980
  C1b: one selector per resource of the contracts in hand under each
  address, an exact key or a template's key with each parameter a
  wildcard, never `<ns>/zk2/**`, a typed or rule selector narrowed to it
  (`FaceScope::narrow`), no presence read, refused without `--contracts`,
  and under `watchdog` and `record --on` a rule reading presence or the
  doctor refused, through the fleet's `watchdog_across`,
  `TriggerSpec::across_face` and `run_field_over`; `live_zk2.rs`'s
  `constrained_s7_a_tools_scope_across_a_face`)
  and resolve every key through one session-free **lens**
  (`zenkey_fleet::model::lens`, fed by `bus::lens`): presence in
  `--namespace`, the contract each descriptor names, the rung named where
  resolution stops (O2). A `.zrec` reads through the same lens, so live and
  file windows are one projection, its zk2c envelopes read first (core §7.5, #976 A2): a record rendered as its type, a refused envelope undecodable with its refusal named, `--raw` sniffing the record. Their shapes — `KeyIdentity`/`KeyGroup`,
  `Conformance`, `QosMismatch`, `LensScope`, `PayloadCheck` — are in
  `report/observe.rs`. A stamp is the owner's when its id is the descriptor's
  `meta.zid`, by value (O7). `.zsnap` is version 2 and `snapshot diff`
  aligns by zk2 key (no origin alignment), compares records, never
  envelopes, and names a re-put "re-stamped in band (run r, seq s → …)"
  (#976 A4, `tests/cmd/fixtures/inband-{a,b}.zsnap`); `timeline` keeps an
  enveloped sample in its resource's lane, its run and seq beside its HLC
  (`in band run r, seq s`), the `unstamped` lane only for neither stamp
  (A4). `check expect`, `check probe` and
  `check schema` take an address, an interface revision and a resource;
  `origin-down` is `instance-gone` — judged per address on FL1's presence
  projection with zenwatch's meaning, and folded into one state per rule
  (firing while any address is gone; #747) — `alert-firing <SEL>
  [info|warning|critical]` reads alarms.v1 under a base-relative selector
  in the namespace (firing on yes, ok on no, unobservable otherwise, never
  ok; #800, AC), and field-stuck waits to be ported onto
  `freshness.v1`, which exists since #720 (`docs/zk2/tooling-guide.md`
  §6). FK1 (#585) brought back zk2's `why`, the ladder from `namespace`
  to `last-known` (#702), and `check conform`, one service against the
  revision its descriptor claims (#703). #800's AC added `alarms
  [SELECTOR] [--at-least SEV] [--for SECS] [--clocks-synced]`, a root
  judgement beside `health` (exit 1 an alarm firing at or above the floor,
  0 none with every owner asked known whole and the presence read that
  found them complete, 2 otherwise), and `check
  conform`'s `alarm-key` and `alarm-document` (21 cases). #819's TC added
  `telemetry [SELECTOR] [--for SECS] [--across-face] [--settle SECS]`, a root judgement
  over `telemetry.v1` (exit 1 a sample that does not conform or a decrease
  within an established epoch, 0 everything asked answered clean, 2
  otherwise, a counter or histogram that formed no pair, or only assumed
  ones, text 0.4, and a sample that could not be read, text 0.5,
  included; no
  `--clocks-synced`, since every time base subtracts two readings of one
  clock), the doctor's `telemetry-declaration` (a warning, from contracts)
  and `telemetry-sample` (`--deep`), and `check conform`'s
  `telemetry-declaration`, `telemetry-sample` and `telemetry-continuity`
  (24 cases); `gen` and `serve` open an owner's session (#825), since core
  0.73 with timestamping off, a governed interface's candidate's on (§8.2),
  and `--run-file PATH` gives each instance its next 16-bit run from a
  `FileRunStore`, refused beside a governed interface before any session,
  exit 2 (#976 A3); `get state`'s rows and `watch`'s samples carry
  `inband`, the run and seq, beside `timestamp`, which the table's detail
  line and the watch's head line say (`stamped in band: run 0003, seq 12`). #840's DD added `desired [SYSTEM/SERVICE] [--role R] [--for SECS]
  [--clocks-synced] [--contracts PATH]`, a root judgement over
  `desired.v1` (exit 1 a refused or failed act, or a target "not yet" on
  one version of its author's for the whole window, by the readings or a
  trusted clock; 0 every role asked converged on its author's current
  document; 2 otherwise; an absent author's slice read from the archives,
  never current), the doctor's `desired-refused`, `desired-behind` (under
  `--clocks-synced`, past `--desired-behind`, 60 s; one reading never
  finds a target behind) and `desired-last-known` (info), and `check
  conform`'s `desired-report` (a role covered only by its interface's own
  reports, DE), `desired-binding` and `desired-author` (27
  cases). Scenarios §9 run live in `tests/live_zk2.rs` on the owner
  example's drive. #841's RC added `redundancy [SYSTEM/SERVICE] [--iface]
  [--grace] [--across-face] [--contracts]`, a root judgement beside
  `health`: exit 1 on a split, a twin or a lasting vacancy; 0 when each
  governed interface has one incumbent and every instance is accounted
  for; 2 otherwise. The doctor gains `redundancy-vacancy` and `check
  conform` `redundancy-declaration` and `redundancy-standby` (29 cases).
  `gen` and `serve` of a governed contract are candidates on a reader and
  a watch session of their own (`cmd/serve.rs`'s `elect`), with the
  address guard unchanged. §12 runs as `live_zk2.rs`'s `redundancy_s12_…`.
  #908's MC added `timing [SELECTOR] [--for SECS] [--clock-bound MS]
  [--across-face] [--owner-zid ZID]… [--settle] [--lease]
  [--wait-before-close] [--contracts]`, a root judgement beside
  `telemetry` over `run_timing`: §5's three questions per member in the
  fixtures' tokens, rolled up per resource and per run; exit 1 a deadline
  missed with the owner present and nothing lost, a period not kept
  likewise, or a sample expired; 0 every question asked on its other
  pole; 2 otherwise (no `--clock-bound` leaves every lifespan `no_bound`).
  The doctor gains `timing-declaration` (32 checks), and `check conform`
  `timing-period` and `timing-deadline` (31 cases). Given a selector,
  `timing` and `telemetry` report what it cannot reach (#950, F-287:
  `report::ScopeReach`, a `scope` field, the verbatim chunks it names
  none of through `zrec_excluded` and the timed streams or metric
  resources of the owners in scope it misses; a note, and stderr through
  `cmd::zk2::scope_note`, observer.md §8.2); `alarms` needs none, every
  alarms.v1 key being plain state, and `arbitration` offers no wildcard.
  grammar.md §1 step 5 runs as `live_zk2.rs`'s
  `grammar_s1_a_tools_wildcard_scope_names_what_it_excludes`, and
  operations.md §5 step 2 (O3 judged from outside, `--calls-granted` or
  not) as its `operations_s5_a_frozen_servers_silence_…`. timing scenarios §6
  runs as `live_zk2.rs`'s `timing_s6_a_tools_window`, on the owner
  example's timing drive compiled in (`timing every <member> <ms>` its
  step 3, `timing declare <axis>` a member token alone; text 0.5's step
  5, an owner publishing nothing closed 0.15 s in, `window_too_short`,
  F-278, and step 6, `timing remove x` then `timing add x`, read over its
  first life, F-280).
  #909's BC added `arbitration [SYSTEM/SERVICE] [--role R] [--for SECS]
  [--lease] [--wait-before-close] [--contracts]`, a root judgement beside
  `timing` and `redundancy` over `run_arbitration`: §5's three questions
  per consumer's arbitrated role in the fixtures' tokens, the selection
  labelled the tool's own view in every medium, the binding naming its
  wildcard and shadowed entries; exit 1 an invalid declaration, the view
  in the dead-man or a wildcard or shadowed entry (a warning); 0 every
  question asked on its other pole; 2 otherwise; `--for 0` listens not at
  all (`not_listened`). The doctor gains `arbitration-declaration` (33
  checks). arbitration scenarios §10 runs as `live_zk2.rs`'s
  `arbitration_s10_a_tools_questions`, against the actuator and the
  commanders brought up through the runtime as the owner example's
  `--arbitrate` and `--commander` do.
  `--skip` sends nothing: no call for `operation`, no fan-out probe for
  `fanout-refused`, no standby probe for `redundancy-standby` (#841, RD).
  PF's `health [SYSTEM/SERVICE]` (#721) reads health.v1 as its §2.11 reader
  does (the clock on `--clocks-synced` or a `--for` window), the doctor adds
  `health-*` and `hostid-duplicate`, and `check conform` a `health` and a
  `health-aggregation` case. #735 lights the population budget (core 0.24,
  §2.7) through `zenkey_model::budget`: `check conform`'s `budget` (per
  templated state, by the owner's GET), `budget-window` (per templated
  stream or event, complete over a whole liveness span, lossless, the owner
  present throughout: its instance tokens watched from no namespace on the
  wire keys, their barrier standing to the window's end, #872, #875) and
  `rate`
  (per event) cases, `--skip <CASE>`, and
  the doctor's `population-over-bound` under `--deep`. Core 0.25 (#740)
  establishes "lossless" rather than assuming it: the reader lost nothing
  and the resource is `reliable` with `block` (`budget::lossless`), the
  owner held its token until a lease after the window (`--lease`, zenoh's
  10 s by default), watched past it for the longer of `--lease` and
  `--wait-before-close`, plus the reply timeout
  (`zenkey_model::budget::presence_tail`, `ConformSpec::presence_tail`;
  observer text 0.6's F-320, which made `wait_before_close` lengthen it),
  and the window counts from its declaration plus the reply timeout; the run's subscription detects no misses. Core 0.27
  (#753) adds the reader's own stall: `bus::stall` watches the run's
  session's transport events and a ticker on a thread of its own over the
  window and the watch after it, and a closed transport or a gap past
  `budget::stall_bound` (`--wait-before-close`, 5 s, or `--lease` less a
  keep-alive) reads `reader_stalled`. Every `--wait-before-close` says
  its 5 s default is zenoh 1.10.1's, and a deployment on another binding
  states its own (`zm-1.10.2-r1`'s is 10 s; #1011). The runtime
  keeps §2.7 (#741): `zenkey::budget` refuses a new member at the bound,
  an occurrence beyond the rate and a lowering below the live members
  (`Error::Budget`), so a test that needs an owner over its bound uses
  two instances of one service, or raw puts.
  CC's `config get|set|confirm|cancel|extend|persist <SYSTEM/SERVICE>`
  (#768) are `config.v1`'s, every rule the engine's: `bus::config`
  (`run_config` reads presence, the descriptor — the premise: listed, not
  listed, unread, absent, incomplete, across a face — then S4's GET of the
  view and status and a `--for` window on them and `events/change`;
  `call_config` makes one act through `zenkey::Client`), `model::config`
  (`freshness.v1` at 60 s, a GET's stamp aged only on a trusted clock; the
  projection; values typed by the view's schema, else the status, else
  their spelling; the owner's own `validate_set` run before anything is
  sent; consent for a reach change, `--i-know` or a terminal's yes; a
  sensitive value only from `@FILE`/`@-`, shown and scrubbed as
  `<redacted>`) and `judge::config` (§5's questions, each yes, no,
  `not_yet`, `unobservable` or `not_asked`: a stale document answers
  nothing, never a no). `config get` is a read (0 decoded, 1 undecodable,
  2 nothing read); an act exits 0 on a reply of its type, 1 on a refusal or
  a malformed reply, 2 on silence. The doctor adds `config-projection`,
  `config-overdue`, `config-unpersisted` and `config-desired` (the last two
  under `--clocks-synced`), `check conform` five `config-*` cases asked of
  any service whose descriptor lists `config.v1` (its refusal probes are
  dry runs against a revision no owner holds), and `acl gen` grants one
  member of a templated operation in place (`"set/duty"`,
  `examples/zk2/acl/config.enrollment.toml`). Scenarios §13–§14 run live
  in `tests/live_zk2.rs` against the owner example's simulated device.
  **Tree (#307)**, and the depth carries meaning: a **noun** is something
  declared, alive or persisted and gets verbs under it (`service
  list|show`, `iface list|show`, `schema show`, `namespace list`,
  `storage list|gen`, `acl gen`, `admin routers|graph`, `key`, `bench
  call`, `config get|set|confirm|cancel|extend|persist`); a **wire verb**
  is an act or observation on live traffic and
  hangs off the root (`get` and `get state`, `call`, `watch`, `echo`, `pub`,
  `rate`, `field`, `record`, `replay`, `timeline`, `snapshot`, `graph`,
  `serve`, `gen`, `scout`); `compat` compares two contract revisions
  offline, exit-coded like a judgement; a **judgement** is exit-coded
  (`check expect|probe|schema|face`, `doctor`, `health`, `alarms`,
  `telemetry`, `timing`, `desired`, `redundancy`, `arbitration`, `watchdog`).
  **Flag vocabulary**: `--for` is every passive window (f64 seconds),
  `--timeout` is reply-wait only, `--duration` bounds generated output (`gen`
  alone), `--watch` is a bare bool with `--every` as the one period,
  `--count` is a stop bound (`--at-least` asserts, `--calls` sizes a bench,
  `--times` repeats a publish), `--from` names an input source, `--i-know` is
  one guard per verb, `--explicit` is a bare bool opting a raw observer's
  default scope into the explicit kinds, never a default (#924). **Exit
  contract**, written once in `zenctl/src/exit.rs`
  and cited by every verb: 0 asked-and-clean, 1 asked-and-a-finding, 2 no
  verdict — which covers clap's
  usage errors, everything else this tool refuses of your input
  (`exit::Unaskable`), silence under fan-out, and any pre-run failure of a
  verdict verb (`exit::asked`). Verdicts exit through
  `zenkey_fleet::judgement_exit_code` over observer §1's `Judgement`, never
  a hand-rolled match. `zenctl/CHANGELOG.md` carries the old→new table.
