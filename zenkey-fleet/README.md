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

It reads a deployment through the zk2 runtime (`zenkey` 0.23) and its
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

`doctor` (zk2's: a deployment against the core, 37 checks, each a
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

`record` (`.zrec` capture and replay, normative for the format; version 4
carries each row's QoS axes, and in its header the catalog, the revisions
and their bundles), `ingest` (the row dialect it is made of),
`snapshot` (`.zsnap`), `trigger` (armed capture), `generate`, `mock` and
`synth` (contract-driven mock owners, marked synthetic), `bench`.

---

A raw observer's session is deliberately **un-namespaced** (RFC 09 §5): it
sees the wire as it really is. Do not "fix" that.

Explorer *configuration* — named connection contexts, `~/.config`, the
completion cache — is not here; it lives in `zenkey-explorer-config`, so a
library consumer of this crate does not pay for `dirs` and `toml`.

## Changes since 0.22

- **The wire breaks 0.22's, as the runtime's does:** a 0.22 tool refuses
  a 0.23 owner's descriptor that states no `minor` (core 0.107), and
  reads no compact one (core 0.120,
  [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s C5).
  Move the tools before the owners. Every reader here reads both forms:
  a compact descriptor through the catalogs it was given and each
  revision it names, held or retrieved, and never guessed; the standard
  revisions `zenkey` embeds are every reader's own (`model::standard`),
  so a first-hop owner, which holds no bundle, reads without them given
  (core §8.4). One not read is
  `report::presence::DescriptorAnswer::Unread` and
  `model::catalog::DescriptorRead::Unread`, with why (`unknown_catalog`,
  `unknown_revision`), and the read is not complete.
- **The readers' transport by their routers' binding**
  ([#1011](https://git.marcpardo.eu/marcpardo/zenkey/issues/1011)):
  `ConformSpec`, `AlarmSpec`, `BookSpec`, `TelemetrySpec`, `TimingSpec`
  and `ArbitrationSpec` hold `routers: budget::Routers` in place of
  `lease` and `wait_before_close`, and `WatchdogSpec` and `TriggerSpec`
  gain `routers`. `--binding` names it on the tools, `zenoh-1.10.2`
  among the bindings
  ([#1165](https://git.marcpardo.eu/marcpardo/zenkey/issues/1165)).
- **The owner heartbeat, read**
  ([#1110](https://git.marcpardo.eu/marcpardo/zenkey/issues/1110),
  observer 0.34): a watch reads each group (`WatchConfirmation::group`),
  `tape::generate` writes one (`GenSpec::owner_heartbeat`), and the
  plans weigh it (`report::acl::ServiceSpec::owner_heartbeat`;
  `owner_heartbeat_denied` behind a `link.v1` face, #1145).
  `FaceCheck` gains `warnings`, `face_warnings` and `refusals`.
- **The catalog in more readers:** `TelemetrySpec`, `TimingSpec`,
  `ArbitrationSpec`, `DesiredSpec` and `RedundancySpec` gain `catalogs`;
  `bus::consume::watch_across` (`watch_resource_across`) takes the
  `&Catalogs` and a `WatchReads { far, across_face, timeout }`
  ([#1146](https://git.marcpardo.eu/marcpardo/zenkey/issues/1146));
  `get_config_records` and `observe_why` take the `&Catalogs` too, and
  `bus::presence::describe_where` what it describes through.
- **Four profiles' second majors, read beside the first:** `health.v2`
  (`HealthSpec::contracts`, `HealthGet::v2`, `HealthWindow::{heard_v2,
  status_v2}`, `ServiceReading::horizon`, and each health row's `word`),
  `alarms.v2` (`AlarmSpec::{contracts, face_major}`, `BookSpec::face_major`,
  and each alarm row's `profile`, `slot`, `words` and `record`),
  `desired.v2` (`report::desired::DesiredCurrentState::{Unstamped,
  Entry, Tombstone, NoEntry}`, and each desired row's `profile`) and
  `redundancy.v2` (`RedundancyRow::majors`, `Governed::majors`;
  `judge::redundancy::row` takes each instance's run).
- **Grown or changed, and what to write instead:**
  - a report or spec struct written field by field needs its new
    members, above, and `ConformSpec::first_hop` (the face policy's word
    that the owner is on a first hop, so `contract-served` asks it no
    bundle, core §8.4);
  - `TelemetrySpec` is no longer `Copy`, and `bus::health::ServiceHeard`
    gains private fields, so no literal builds it;
  - `TimingBook::seed` and `owed_read`, and `ViewBook::sample` and `read`,
    take one argument more; `ViewBook::described` is gone.
- **Unchanged:** `.zrec` stays at version 4, and the doctor at its 37
  checks.
- **A watch across a face is repaired as the runtime's is** (core 0.121 §8.5, [#1185](https://git.marcpardo.eu/marcpardo/zenkey/issues/1185)): `Monitor::repaired()` counts the repair subscriptions.

## What 0.23 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. These are still moving under 0.23, and a 0.24 may break them:

- **The texts zk2py has not read yet.** The independent implementation's
  rounds ([#609](https://git.marcpardo.eu/marcpardo/zenkey/issues/609))
  were paused for zenoh-modem's needs. Core 0.120's compact descriptor and
  archive attachment ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980))
  link.v2 0.30's batch allowance
  ([#1136](https://git.marcpardo.eu/marcpardo/zenkey/issues/1136)) and
  core 0.121's repair subscription
  ([#1185](https://git.marcpardo.eu/marcpardo/zenkey/issues/1185)) are the
  reference's reading alone, and a round may amend them.
- **Owners' defaults that wait for these readers.** #986's Q3 and #984's
  S5 flip an owner's default once 0.23's readers are deployed: readers
  move first.
- **#980's rest:** the near side's descriptor size, and the minor.
- **What the readers decode, and how the plans are drawn,** move with the
  texts above.

A 0.23.x patch release changes none of them. The rules are the spec's
([`spec/core.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/core.md),
its version in its header), and a release implements the spec as it
stood when it was cut.

## Zenoh's unstable API

This crate takes `zenoh` 1.10 with its `unstable` feature, as `zenkey`
does (its README lists what the runtime opts into, and why). A tool
reads more of a session than an owner does, and the fleet's library
code opts into these, each where `spec/observer.md` names it:

| Unstable in zenoh 1.10.1 and 1.10.2 | Where | Why |
|---|---|---|
| `Reply::replier_id` | `bus::admin`, `tape::snapshot` | Which node sent an admin-space answer, whatever key it is on: S4's check against the admin space verifies an answer by it (observer §4.2). |
| `SessionInfo::transport_events_listener`, `TransportEventsListener` | `bus::stall` | A window's reader watching its own receive path: a router that gave up on it closes its transport (core §2.7, observer §5.2). |
| `Sample::source_info`, `SourceInfo::source_id`, `SourceInfo::source_sn` | `bus::monitor` | The publishing entity and its sample number, where a sample carries them, for the monitor's gap count and a recording's source (zenoh 1.10.1 and 1.10.2 deliver none to a subscriber, and the tools say so). |
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
