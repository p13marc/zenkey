# zenwatch

Claude Code loads this file when working in `zenwatch/`; the root `CLAUDE.md` is the index.

- `zenwatch/` — the **notifier** (Apache-2.0, **not published**; Forgejo
  release binaries, like zenctl and zengui; #388, epic #387). The third
  binary on the same engine: zenctl explores a bus, zengui renders one,
  zenwatch tells you when something is wrong while you are asleep. One
  process, one JSON5 config, N rules, M sinks. **zk2's read side since FL2
  (#614)**: the rules, presence, contracts and doctor run on the in-tree
  fleet, on **two sessions** (`bus.rs`) — one in the deployment namespace
  (`bus.namespace`, alias `base`; `--namespace`/`--base`, resolved flag >
  config > env > context > empty) for presence, descriptors, contracts and
  the doctor, one in none for wire selectors, foreign tokens and the admin
  space; plus a watch session of each kind a rule needs (`bus::Watches`,
  #830), each a `zenkey_fleet::WatchSession` in no namespace (#875): on
  the deployment's wire keys for the presence projection's monitor and the
  `alerts`/`alert-firing` readers' instance tokens, on the bus root's for
  the foreign-token ledger's monitor — no session both holds a liveliness
  subscriber and reads presence; contracts by `contracts` and `--contracts` (zenctl's loading),
  never a named context. **The write side since FL3**: the daemon is a zk2
  service on the namespaced session (`publish.rs`, `SelfService` over
  `ServiceBuilder`), `health.v1` through `zenkey::health` and its own
  **`zenwatch.v1`** (`contracts/`, its own `.history`; Rust-first: its
  types are `publish.rs`'s structs bound by `json_type`, the committed
  schema held to them by `check_schema`; `build.rs` is zk2's
  `zenkey-build`; minor 2, #978 F4, uses `freshness.v2`: its states, and
  health.v1 1.1's status, are confirmed by the runtime's heartbeats, never
  re-put, and `tests/publish.rs` checks the descriptor lists `freshness.v2`). zenwatch links no v1 crate. A rule is one spelling of the **closed**
  vocabulary — `zenctl watchdog`'s zk2 eight on the fleet's watchdog
  (`rate-above`, `rate-below`, `silent-for`, `invalid-payload`,
  `qos-mismatch`, `doctor`, `alert-firing <SEL> [info|warning|critical]`,
  `dropped`) plus three of its own: `instance-gone <SYSTEM/SERVICE>`
  (either position `*`), `liveliness-gone <SEL>` and `alerts <SEL>` (#800
  AD, `alarms.rs`: the fleet's `AlarmFeed` per distinct base-relative
  selector on the namespaced session, its GET's confirmations dated
  under the top-level `catalog`, `AlarmRules::open`'s `catalogs`, #1087; one state per alarm key, firing on
  §5's first answer yes, ok on no, unobservable otherwise, never ok; an
  escalation notifies, a refresh or confirmation does not; an answer that
  is only the reader's sight leaves the key at its last word, a loss is
  the rule's `observer`'s once, and an owner's absence is a refresh's
  presence read, never a token's withdrawal alone; an alarm firing at the
  first look is told; alarms.v1 §2.16, text 0.4). They are **not**
  `Condition`s: a `Condition` is one state per rule, these are one state
  per address, token or alarm. `presence.rs`
  reads FL1's live projection (a `PresenceFeed` on its own monitor; patterns
  tracked): an address up while an instance token is held, gone only under
  a complete read (a withdrawal heard live waits for the read it owes;
  interface, member and foreign-token identities too, core 0.37);
  `liveliness-gone` on the deployment's zk2 tokens rides it
  per address / address+iface / address+iface+member (a handover is no
  gap); on anything else `liveliness.rs` keeps the daemon's own ledger,
  seeded by liveliness reads that say when they ran to their timeout. Both
  are cancel-safe in the `select!`, and a `Dropped(n)` or an incomplete
  read is the rule's `<rule_id>:observer` going `unobservable`, never `ok`.
  The **first look is a baseline**: an address down at start is *assumed*
  (`NoticeMeta::assume`) and said once in a `baseline` notification (an
  alarm firing at start is told, not assumed);
  under a wildcard nothing can be gone that was never seen. That address
  reading is the fleet's `model::presence_watch::AddressWatch`, which
  `zenctl watchdog` and `record --on` share (#747). **Refused by
  name, exit 2, never ignored**: `alerts`/`alert-firing` over v1's alert
  plane (saying what to write), an alarm selector carrying the namespace,
  `origin-down`, inhibition switched on (removed with v1's catalog),
  `group_by: ["origin"]` (zk2 groups by `system`/`address`),
  `bus.registry`, the doctor's `sample`, a version-1 state file, and at
  `run`'s start a `liveliness-gone` naming an interface that an instance
  present marks `"token": false` (#982 H3: a first-hop owner holds no
  interface token; `PresenceRules::refuse_tokenless`, saying to write
  `instance-gone`; a presence read for it that failed refuses nothing and
  is said at warn, naming the rules left unchecked, `engine::tokenless_refusal`,
  #1074).
  `check-config` and `run` pass one gate (`load_checked`: the file, the
  context, the namespace, the contracts, the state file). Sinks: `ntfy`,
  `smtp`, `webhook`, `exec` — the one that runs code, deliberately last.
  **Secrets are env-var names or file paths, never inline**: a bare string
  in a secret position fails to parse. **The three states are preserved**
  into every sink payload; a presence notice carries the engine's pinned
  `PresenceTransition` (`report::presence`), an alert its `AlarmTransition`
  (`kind: alert`, the document rendered `schema`); an alarm under a zk2c envelope is read and merged by run and seq in the fleet's `AlarmFeed` (core §7.5, #976 A2), and zenwatch reads wire bytes nowhere. Its own zenwatch.v1 and health.v1 puts carry the runtime's in-band stamps (#976 A3, core 0.73), and `tests/publish.rs`'s self-conformance passes `state-stamp` and `state-get` on them. Routing — which rule, which
  sink, whether it is a duplicate — is the daemon's (`engine::route`, over
  a `Notice` seam). `--dry-run` swaps every sink for a printing one. **The
  discipline** (#389, `discipline/`) sits between `route` and delivery and
  takes the clock as an argument: `for`, dedup, grouping by a label set,
  repeat, the four resolved kinds. The ledger is bounded and survives a
  restart in `state_file`, **version 2** (zk2 identities, the namespace
  they were judged in; missing is fresh, malformed, version 1 or another
  namespace's is exit 2). **A real daemon, explicitly launched, is a zk2
  service** at `@hostid.v1/zenwatch` (minted in `load_checked` before any
  session opens; a host with no id is exit 2 unless `publish.address`,
  `publish.ephemeral: true` or `publish: false`), up after its observers
  are seeded: zenwatch.v1's `firing/{rule_id}` (cardinality 256, the
  runtime's `Error::Budget` past it counted in `firing_refused`), `doctor`
  (optional, gated `config:doctor`, after every scheduled run) and
  `counters` (what the bounds cost); health.v1's status declared `OK` and
  raised to its worst check (`sinks`, `presence`, `tokens`, `doctor`,
  `state-file`, `firing`, `alarms`). It stops last — the final tick, the observers,
  the sinks drained, then its tokens — so it never pages about its own
  clean stop; and it caches no discovery: it is not the noun
  `docs/redesign-2026-07.md` §6.1 rejected. **The scheduled
  doctor** (#390, `doctor.rs`): a `doctor` block runs zk2's `run_doctor`
  (`deep`, thirty-five checks since #982's H3, `first-hop-token` not asked
  since zenwatch takes no face policy, `arbitration-declaration`
  and `catalog-missing` among them; top-level `clocks_synced` for the
  `health-*` checks, which a `doctor` rule reads too; top-level `catalog`,
  the deployment catalogs core §8.6 provisions, loaded by `load_checked`
  into `Loaded::catalogs` and given the scheduled doctor alone,
  `Schedule::with_catalogs`, so a `doctor` rule asks no `catalog-missing`) every few **hours** beside the drain, as a rule
  of its own (`RuleKind::ScheduledDoctor`, id `doctor`). The first run is a
  **baseline** (one `info` notification; every finding assumed); later runs
  notify each new finding and `resolved` each fixed one, keyed `(check,
  subject)` on zk2's `CheckId` — fixed only where the run established the
  check, carried otherwise; a run over an empty scope is `unobservable`
  with the previous report retained, and every message ends with the
  coverage (scope read, checks established / unobservable / not asked).
  `zenwatch/CHANGELOG.md` carries the old→new tables.
