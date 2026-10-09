# zenkey-fleet

The fleet engine for [zk2](https://github.com/p13marc/zenkey) Zenoh tooling —
the shared core of the zk2 explorers, `zenctl` first, in five layers:

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

It reads a deployment through the zk2 runtime (`zenkey` 0.20) and its
session-free model (`zenkey-model`): presence from instance and interface
tokens, descriptors, contract bundles retrieved by fingerprint, and every
payload rendered through the contract that declares it. **`main` is the zk2
line**: v1's engine — the registry slice sets, RFC 08 §6 introspection, the
v1 roster, the schema-aware decode seam and its codec features, the v1 judges
and the profile-backed features — left at FJ9 (#612) and lives on the `v1`
branch, released as `zenkey-fleet` 0.18.x, which zengui and zenwatch build
against until #614 ports them.

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
  storages they run. Plus `session`, `serve`, `scout` and `seed`.

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
