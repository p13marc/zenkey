# zenkey-fleet

The fleet engine for [zk2](https://git.marcpardo.eu/marcpardo/zenkey) Zenoh tooling —
the shared core of the zk2 explorers (`zenctl`, `zengui` and the `zenwatch`
notifier), in five layers:

```text
  bus/     holds a session      →  observations
  model/   holds values         →  meaning
  judge/   holds meaning        →  verdicts
  report/  the serialized shapes every layer above hands out
  tape/    traffic as a thing: captured, replayed, manufactured, timed
```

Placing a module is one question in order: does it need a session (`bus/`),
can it answer from values already in hand (`model/`), does it say whether
something is *wrong* (`judge/`), does it turn a stream into a recording or
back (`tape/`)? A new serde-pinned struct is not a module question at all —
it goes to `report/`, by the rule that module's docs state.

It reads a deployment through the zk2 runtime (`zenkey` 0.21) and its
session-free model (`zenkey-model`): presence from instance and interface
tokens, descriptors, contract bundles retrieved by fingerprint, and every
payload rendered through the contract that declares it. **`main` is the zk2
line**: v1's engine — the registry slice sets, RFC 08 §6 introspection, the
v1 roster, the schema-aware decode seam and its codec features, the v1 judges
and the profile-backed features — left at FJ9 (#612) and lives on the `v1`
branch, released as `zenkey-fleet` 0.18.x. Every frontend on `main` runs on
this crate since #614 (zenwatch at FL3, zengui at FL11b); none links the v1
engine.

## Starting

An application does not start here: it writes a contract, generates its
typed module with [`zenkey-build`](https://crates.io/crates/zenkey-build)
in `build.rs`, and brings a service up with `zenkey::ServiceBuilder::new`
([`zenkey`'s README](https://crates.io/crates/zenkey) has the example).
This crate is for a tool that reads a deployment it does not own: open a
session (`bus::session`), read presence and the contracts, and render or
judge what the bus carries, as `zenctl`, `zengui` and `zenwatch` do.

## `bus/` — everything holding a session

Two kinds of session (decided 2026-10-08). A **resolved** reader opens its
session **in** the deployment's namespace, as the deployment's own consumers
do; a **raw** observer runs in **no** namespace and sees the wire as it
really is, full keys included — which is what lets it spot a key outside the
deployment.

- **`presence`** — spec §8.1: liveliness reads (one place, flagging a read
  that ran to its timeout as possibly incomplete), and zk2's services from
  their tokens and descriptors.
- **`contracts`** — spec §8.4: bundles by `(iface, fingerprint)`, verified,
  cached, handed out as revisions; a revision held offline is never
  retrieved.
- **`consume`** / **`operation`** — reading through a contract with the
  runtime's `Consumer` (current state, S4; subscriptions; an archive's
  last-known state), and calling an operation with its `Client` at one
  address or its `Fleet` over a selection.
- **`lens`** — what a raw observer resolves its keys through: one presence
  read in the namespace, every instance's descriptor, every revision they
  name.
- **`query`** — the RFC 05 §2.1 fan-in discipline (`fleet_get`: target `All`,
  consolidation `None`, attribution by the reply's own key), in exactly one
  place. Answers carry zenoh's refcounted `ZBytes` — no per-reply copies.
- **`write`** — the one way an explorer writes a foreign key: a declared
  `Publication` on `WireQos` axes (`priority/congestion/reliability[+express]`),
  with `retire` its tombstone. There is deliberately no bare-put and no
  bare-delete helper. `check_retire` refuses a wildcard outright and prices a
  concrete tombstone as the operator's act (`--i-know`).
- **`monitor`** — `Monitor`: subscription multiplexing + liveliness watching
  into a bounded broadcast of events. Overflow surfaces as an explicit
  `Dropped(n)`; render loops pull the immutable `KeyTreeSnapshot` on the
  stats tick, so a hot bus cannot melt a UI.
- **`admin`** — raw `@/**` admin-space access: routers, the mesh graph, the
  storages they run. Plus `session`, `serve`, `scout`, `seed` (a late
  joiner's, merged by `zenkey_model::seed`) and `stall` (a window's reader
  watching its own transports and a ticker, core §2.7 0.27).

## `model/` — meaning, without a session

- **`lens`** / **`render`** / **`target`** — a raw key resolved through the
  namespace, presence and the contracts in hand, to a zk2 address and
  resource or the rung where resolution stopped; a sample rendered through
  its contract (spec §7.2), or structurally (`structural`) when none is in
  hand; and what a resolved verb aims at, settled before a session opens.
- **`catalog`** — who provides which interface at which revision, who
  requires it, and what each revision's contract says.
- **`acl`** — zk2's access control (spec §11, #612 FJ7): an enrollment and
  the contracts in, a router's `access_control` block out (`plan_acl`), in
  either posture and with a constrained face; `check_acl` against a router's
  config file, `explain_acl` per direction, `to_json5` for zenohd.
- **`storage`** — the RFC 09 §2 storage planner: a deployment file of
  selectors in, the `storage_manager` block out, every derived number shown.
- **`stats` / `tree`** — per-key rate/byte counters (EWMA, SourceInfo gap
  counting) and the chunk-grouped snapshot they build.
- **`bounded` / `retain` / `examples`** — the three mechanisms every
  long-running projection shares: the O6 ceiling with its eviction ledger,
  the retention budget, and "up to N examples, and the count of what they
  stand for". Plus `compat`, `diff`, `snapshot`, `snapshot_diff`,
  `timeline`, `jsonschema` and `namespace`.

## `judge/` — verdicts

`doctor` (zk2's: a deployment against the core, thirteen checks, each a
verdict in the judgement shape, #612 FJ6) and its `doctor_delta`, `expect`,
`probe`, `field`, `condition` (the watchdog), and `common` — the caps on how
many offenders a report names.

## `report/` — every serialized contract

One rule, stated in full at the top of the module: **every serde-pinned wire
shape lives here, split by domain, with its pinned-shape test beside it; a
type the wire never sees stays in the module that computes it.** The domain
files are private and re-exported flat, so every shape is spelled
`zenkey_fleet::report::Thing` and the split can be re-cut without a call site
moving.

## `tape/` — traffic as a thing

`record` (`.zrec` capture and replay, normative for the format; version 3
carries each row's QoS axes), `ingest` (the row dialect it is made of),
`snapshot` (`.zsnap`), `trigger` (armed capture), `generate`, `mock` and
`synth` (contract-driven mock owners, marked synthetic), `bench`.

---

A raw observer's session is deliberately **un-namespaced** (RFC 09 §5): it
sees the wire as it really is. Do not "fix" that.

Explorer *configuration* — named connection contexts, `~/.config`, the
completion cache — is not here; it lives in `zenkey-explorer-config`, so a
library consumer of this crate does not pay for `dirs` and `toml`.

## Changes since 0.20

- **The wire breaks 0.20's** (core 0.100,
  [#977](https://git.marcpardo.eu/marcpardo/zenkey/issues/977)). Every
  state GET, `bus::query::state_get`'s and the seed's, the doctor's,
  `check conform`'s, `.zsnap`'s and the profiles' readers' among them, is
  under consolidation `None`. Each snapshot's page reads as its entries,
  merged per key by its stamp, and the seed applies the omission rule
  where the contracts are in hand (`seed_subscribe_with`,
  `Monitor::watch_seeded_with`). A `reply_del` on a member of a
  collection answered by snapshot is undecodable, `form`. A call's reply
  under any `Encoding` but `zenoh/bytes` renders undecodable (core 0.99,
  [#985](https://git.marcpardo.eu/marcpardo/zenkey/issues/985)'s C4d).
- **Across a face** (observer 0.29,
  [#990](https://git.marcpardo.eu/marcpardo/zenkey/issues/990)'s V1 and
  V2): `FaceScope` is a tool's scope, the face's plan when a face policy
  is given, with each heartbeat; `model::choice` picks a key's revision
  among several held by the catalog and CH's tag (`KeyIdentity::chosen`,
  and `Unresolved::NoneLeft` naming the step that left none). A record
  under a CH tag other than 0 whose type is not held renders as a packed
  record, never sniffed. `LensFeed::runs_in` and `on_runs` ask each
  address's run once, outside the lens's lock
  ([#1079](https://git.marcpardo.eu/marcpardo/zenkey/issues/1079)).
- **Removed:** `OperationAnswer::Malformed`, `ConfigActAnswer::Malformed`,
  `MalformedReply`, `RepliesView::malformed`, `BenchReport::malformed` and
  `bus::conform::FanoutSeen::malformed`. A reply error that is no error
  form is the transport's, silence, and a value with no form is
  undecodable.
- **Changed:**
  - `bus::{alarms, config, health}::get` take an `&EntryEpochs`, the epoch
    each entry's confirmation is dated under (`EntryEpochs::of_catalogs`,
    `observed`);
  - `ConfigRefusal::message` is an `Option<String>`, and
    `ConfigRefusal::error` a `ConfigAppError`, `V1(ConfigError)` or
    `config.v2`'s `V2 { code }`;
  - `CheckId::ALL` holds 35 checks, `first-hop-token` among them, and
    `AddressStatus` gains `LinkClosed`
    ([#982](https://git.marcpardo.eu/marcpardo/zenkey/issues/982)'s H3).

## What 0.21 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. Two things under 0.21 are still moving, and a 0.22 may break
either:

- **The wire.** An event's key is to end in its run and seq in place of
  a ULID ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s
  C2), and the rest of #975 follows. What the readers decode, and how
  they order it, moves with it.
- **The authoring format** and the canonical form a fingerprint is taken
  over, which the lens and the contract retrieval read. Neither moved
  from 0.20 to 0.21.

A 0.21.x patch release changes neither.

## Zenoh's unstable API

This crate takes `zenoh` 1.10 with its `unstable` feature, as `zenkey`
does (its README lists what the runtime opts into, and why). A tool
reads more of a session than an owner does, and the fleet's library
code opts into these, each where `spec/observer.md` names it:

| Unstable in zenoh 1.10.1 | Where | Why |
|---|---|---|
| `Reply::replier_id` | `bus::admin`, `tape::snapshot` | Which node sent an admin-space answer, whatever key it is on: S4's check against the admin space verifies an answer by it (observer §4.2). |
| `SessionInfo::transport_events_listener`, `TransportEventsListener` | `bus::stall` | A window's reader watching its own receive path: a router that gave up on it closes its transport (core §2.7, observer §5.2). |
| `Sample::source_info`, `SourceInfo::source_id`, `SourceInfo::source_sn` | `bus::monitor` | The publishing entity and its sample number, where a sample carries them, for the monitor's gap count and a recording's source (zenoh 1.10.1 delivers none to a subscriber, and the tools say so). |
| `Sample::reliability`, `PublicationBuilder::reliability` | `bus::monitor`, `bus::write` | A sample's QoS axes, as a `.zrec` row records them, and a foreign key written on the axes it is asked for. |
| `Config::from_file` of a `.toml` file | `bus::session` | zenoh reads a TOML config file only under `unstable`, and a tool's `--zenoh-config` may be one. |

It names no `zenoh-ext` item: its tests use `AdvancedPublisher`, a
dev-dependency.

## Documentation

- [`docs/zk2/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/README.md):
  the design record, and the
  [tooling guide](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/tooling-guide.md),
  how the reference tools meet the observer rules
  ([`spec/observer.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/observer.md)).
- Issues: [git.marcpardo.eu/marcpardo/zenkey](https://git.marcpardo.eu/marcpardo/zenkey/issues).
  The GitHub repository is a mirror.

zenkey-fleet 0.18.x on crates.io is v1's engine, maintained on the `v1`
branch. Apache-2.0.
