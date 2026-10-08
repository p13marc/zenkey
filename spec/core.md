# zk2 core specification

**Version 0.1, draft of 2026-10-08.** Status: for acceptance (#606). Until
it is accepted, it may change without an amendment record. After that, every
change goes through [`CHANGELOG.md`](CHANGELOG.md), amendment-style.

This is the normative core of zk2, the keyspace and contract layer for
applications built on Zenoh. It is written so that an implementation in any
language can be built from it, plus the fixtures in
[`conformance/`](conformance/) and the scenarios in [`scenarios/`](scenarios/),
without reading the Rust reference implementation (`zenkey-model`, the
runtime that follows it).

The design of record, with the reasoning, the alternatives and the
measurements, is [`docs/zk2/architecture.md`](../docs/zk2/architecture.md) (r4)
and [`docs/zk2/spike-report.md`](../docs/zk2/spike-report.md). This document
states the rules. It does not repeat their argument, except in the rationale
appendix.

## 0. Conventions

**Keywords.** MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in
RFC 2119 and RFC 8174, when written in capitals.

**Roles.** Every rule names the role it binds:
- **owner**: the service that declares a resource and is the only writer of
  its keys;
- **consumer**: a component that receives an owner's data through a binding;
- **caller**: a client that calls an operation;
- **deployment**: whoever configures sessions, routers, bindings and access
  control;
- **tool**: a generic program that reads the bus or contracts without
  application source (explorers, CI, generators).

**Evidence.** Each MUST cites what checks it:
- `[F: path]` is a fixture under `conformance/`, which every implementation
  runs;
- `[Sc: file §n]` is a scenario under `scenarios/`: a network behaviour with
  a setup, steps and expected observations;
- `[F: pending]` marks a fixture this draft still owes (#607). Version 0.1 is
  accepted only once none is left.

**Zenoh.** zk2 is specified against Zenoh **1.10.1**. Facts about Zenoh that
the rules depend on are listed in Appendix B. A participant uses only stable
Zenoh API in the core; unstable features are opt-in and never change a key's
shape.

**Spikes.** "Spike Sn" refers to a measurement in the spike report. State
rules are named S1–S7 in §4, and operation rules O1–O7 in §5.

**Examples** are informative. Keys in examples are base-relative (§1.6).

---

## 1. Identity and grammar

### 1.1 Key forms

Every zk2 key starts with the chunk `zk2`, the grammar major. There are five
forms:

```text
zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource…>              data
zk2/<system>/<service>/@zk/instance/<instance>                          instance token, descriptor
zk2/<system>/<service>/@zk/alive/<iface>.v<major>/<instance>/<fp16>     interface token
zk2/<system>/<service>/@zk/member/<iface>.v<major>/<member>/<epoch>     member token
zk2/@zk/contract/<iface>.v<major>/<sha256>                              contract bundle
```

| Position | Chunk | Rule |
|---|---|---|
| 1 | `zk2` | The grammar major. A plain chunk, never verbatim. |
| 2 | `<system>`, or `@zk` | A system name (§1.2). In a contract key, the control token. |
| 3 | `<service>` | A service name (§1.2). |
| 4 | `<iface>.v<major>`, or `@zk` | An interface id (§1.2), or the control token. |
| 5 | kind token | One of the six of §1.3. |
| 6+ | resource chunks | Built from the resource's template (§2.2). |

A key that fits none of the five forms is not a zk2 key. An implementation
MUST refuse to build one, and MUST report one it is asked to parse as not a
zk2 key. Every key it accepts MUST build back to the same string.
`[F: keys.json]`

### 1.2 Lexical rules

- **A plain chunk** is `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`. Literal template
  chunks, slugged parameter values, system names, service names and member
  values are plain chunks. `[F: keys.json]`
- **A verbatim chunk** starts with `@`. In zk2, only the reserved tokens use
  one: `@stream`, `@state`, `@op` and `@zk`, plus a verbatim kind registered
  by a profile (§10). A user-supplied value is never verbatim: it is slugged
  (§1.4). `[F: keys.json]`
- **An interface id** is `<name>.v<major>`. The name is one or more
  dot-separated segments `[a-z][a-z0-9_]*`, and `<major>` is a decimal
  integer without leading zeros. A name never ends in `.v<integer>`.
  `[F: keys.json; contracts/e001-interface]`
- **An instance id** is 64 random bits written as 16 lowercase hex digits.
  `[F: keys.json]`
- **A fingerprint** (§9.3) is `sha256:` followed by 64 lowercase hex digits.
  In a contract key, only the 64 hex digits are written. In an interface
  token, `<fp16>` is the first 16 of them. `[F: keys.json]`
- **A member value** is a plain chunk. `<epoch>` is an instance id.
  `[F: keys.json]`

### 1.3 Kind tokens

| Token | Chunk | A resource of kind … | Selected by |
|---|---|---|---|
| `stream` | plain | `stream` | Ambient selectors (`zk2/<system>/**`): recorders, link budgets |
| `@stream` | verbatim | `stream`, `explicit = true` | Only consumers that name it |
| `state` | plain | `state` | Ambient selectors; archives (§4.4) |
| `@state` | verbatim | `state`, `explicit = true` | Only consumers and archives that name it |
| `events` | plain | `event` | Union storages; bounded replay GETs (§2.6) |
| `@op` | verbatim | `operation` | Calls only (§5) |

Zenoh's `*` and `**` never match a verbatim chunk. Two consequences follow,
and an implementation MUST NOT break either:
- **The guard.** An ambient selector (`zk2/**`, `zk2/<system>/**`, or any
  selector without a verbatim chunk at position 5) never delivers an
  `@stream` or `@state` sample, and a GET on it never reaches an `@op`
  queryable or a control key. `[Sc: grammar.md §1]`
- **Exact kind selectors.** Positions 1–5 have fixed arity, so
  `zk2/*/*/*/state/**` selects exactly the plain state of every service.
  `[F: keys.json]`

### 1.4 Slugging

A template parameter's value is written into its chunk by an injective slug,
and read back by its decoder. Both MUST be implemented exactly as follows.
`[F: slugs.json]`

1. **A value that is already a plain chunk** (§1.2), and does not start
   with the reserved prefix `x-`, is its own chunk.
2. **Any other value** becomes `x-` followed by its UTF-8 bytes, where each
   byte outside `[a-z0-9]` is written `_xHH` (two lowercase hex digits, no
   closing underscore). The exception is `.` and `-`, which stay literal
   unless they are the value's last byte.
3. **The empty value** is `x-_x`.

A value with several textual forms SHOULD be canonicalized by the owner before
it is slugged, or the same entity gets two chunks:
- an IPv6 address in RFC 5952 form, and an IPv4 address in minimal
  dotted-quad form;
- a ULID in lowercase.

The decoder MUST accept exactly the canonical chunks the encoder produces,
and MUST refuse any other chunk as a value. A rest parameter (§2.2) slugs
each of its chunks separately. `[F: slugs.json; templates.json]`

### 1.5 Identity

| Concept | Chosen by | Where it lives |
|---|---|---|
| Deployment | the operator | the session namespace (§1.6), if any |
| **System**: the deployment's ownership unit and outer policy boundary | the deployment | position 2 |
| **Service**: logical and stable; owns its keys | the deployment | position 3 |
| **Interface**: name + major | the contract's author | position 4 |
| **Instance**: one run of a service | the runtime | tokens, the descriptor |
| Host, process, build, zid | the runtime | the descriptor only |

- **A single robot uses one constant system.** A fleet uses many, and
  `hostid.v1` mints a system from a machine id where system = host.
- **Device-as-service.** A process serving several devices MAY host one
  service per device. Each such service then has its own presence, exposure,
  capabilities and access-control prefix.
- **The instance id is a continuity epoch.** An owner MUST mint a new
  instance id whenever its counters reset or its state's ordering restarts
  (§4.3 S7): a process restart, a device re-enumerated, a polled device's
  uptime rewound, a container restarted. Consumers MUST treat an instance
  change as the only legitimate counter discontinuity. `[Sc: presence.md §3]`
- **Moving a service to another host changes no key.**
- **No implicit identity.** A builder MUST NOT supply a system or service
  that its caller did not pass. Fleet-wide selection is spelled by name or
  wildcard, never defaulted. `[F: pending]`

### 1.6 Namespaces and base-relative keys

Keys are **base-relative**: they start at `zk2`. A deployment prefix, when
there is one, is the Zenoh session namespace. The session adds it on egress
and strips it on ingress. Application code MUST NOT spell a deployment
prefix. `[Sc: grammar.md §2]`

At the constrained level (§12), a participant without session namespaces
(zenoh-pico 1.10.1) writes the prefix literally in its keys. A namespaced
peer reads those keys as base-relative. `[Sc: constrained.md §2]`

### 1.7 Coexistence

Grammar majors (`zk2`, `zk3`) and interface majors (`nav.v2`, `nav.v3`) are
mutually invisible to each other's selectors. v1 keys (`v1/…`), rmw_zenoh
keys and other applications can share the bus. A tool that serves several
interface majors subscribes once per major. zk2 has no cross-major wildcard.

---

## 2. Resources and QoS

### 2.1 Patterns

An interface **provides** resources of four kinds:

| Kind | Meaning | Zenoh mapping | Key token |
|---|---|---|---|
| `stream` | A sequence of values pushed by the owner; each stands alone | A publisher | `stream`, or `@stream` when `explicit` |
| `state` | The latest value is the truth; it may be deleted | A publisher, plus a non-`complete` queryable over the interface's state keys (§4) | `state`, or `@state` when `explicit` |
| `event` | One key per occurrence, kept by union storages | A one-shot put per occurrence | `events` |
| `operation` | Request → one reply, or many | A `complete` queryable on the concrete key (§5) | `@op` |

There is no input kind: data reaches a component through its bindings (§3).
There is no presence kind: presence belongs to instances (§8).

### 2.2 Templates

A resource's **template** is its path below the kind chunk.
- **A literal chunk** is a plain chunk.
- **`{name}`** is one chunk holding a slugged value.
- **`{name...}`** is a **rest** parameter: one or more chunks, each slugged.
  It MAY appear only as the last segment.
- **Parameter types** are `string`, `uint` and `path`. `path` is the type of
  a rest parameter, and only of one.

`[F: contracts/e010-template, e011-params, e012-path]`

**Shapes and precedence.** A template's *shape* is its sequence of literal,
single-parameter and rest positions. Within one interface:
- two templates under the same kind token MUST NOT have the same shape
  `[F: contracts/e021-shape]`;
- templates under different kind tokens MAY share a shape, because their
  keys differ;
- overlapping templates are allowed. A key resolves to the template that is
  most literal, chunk by chunk from the left: a literal beats `{p}`, which
  beats `{p...}` `[F: templates.json]`;
- overlapping templates with different types are a warning
  `[F: contracts/w101-overlap]`.

**Cardinality.** A template with parameters MUST declare `cardinality`, a
positive integer that bounds its population in one instance. A template
without parameters MUST NOT declare it. An instance MAY declare a lower
bound in its descriptor (§3.3). On an event, `cardinality` bounds the
template's own parameters, and the key population is cardinality × rate ×
retention. `[F: contracts/e013-cardinality, e014-field]`

### 2.3 Resource attributes

| Attribute | Rule | Fixture |
|---|---|---|
| `optional` | An implementation MAY declare an optional resource unavailable (§3.3). A required one MUST be exposed. | `[Sc: presence.md §2]` |
| `gate` | Only with `optional = true`. One or more of `build:<n>`, `config:<n>`, `capability:<n>`, where `<n>` is `[a-z0-9][a-z0-9_.-]*`. A list is an AND. | `[F: contracts/e016-gate-optional, e017-gate-syntax]` |
| `epoch` | Names one single-chunk parameter. The owner holds a member token per value of it, cycled on discontinuity (§8.1). | `[F: contracts/e022-epoch]` |
| `deprecated` | `{since, replaced_by, reason}`. `replaced_by` names another template of the contract, and `since` is not later than `minor`. A deprecated resource MUST still be served until the next major. | `[F: contracts/e031-deprecated]` |
| `attachment` | A type for the sample attachment, fingerprinted and encoded like a payload (§7). | `[F: contracts/ok-kinds]` |
| `annotations` | Keys are `<profile>.<key>`, and the profile is listed in `uses` (§10). | `[F: contracts/e020-annotation]` |

### 2.4 QoS

Each stream, state and event resource carries Zenoh QoS in its contract. A
generated publisher MUST apply it. The fields map one-to-one onto Zenoh's
stable QoS. `[F: contracts/ok-kinds, ok-defaults-defaulted]`

| Field | Values | Default: `stream` | Default: `@stream` | Default: `state`, `event` |
|---|---|---|---|---|
| `reliability` | `best_effort` \| `reliable` | best_effort | best_effort | reliable |
| `congestion` | `drop` \| `block` | drop | drop | block |
| `priority` | `real_time` … `background` (Zenoh's seven) | data | data_low | data |
| `express` | bool | false | false | false |

- **State and events** SHOULD keep `reliable` and `block`. A state declared
  `best_effort` relies on re-puts to heal a lost update: zenoh-modem re-puts
  each document every `ttl_s/2` (`freshness.v1`). An archive (§4.4)
  recording such a state can then miss values between re-puts. Spike S5
  lost 10,917 of 100,000 values put with `drop` on their way to a storage.
- **`express`** is opt-in. A contract's author SHOULD set it only on
  measurement. Spike S9 measured `real_time` + express losing 0.6–4 % of
  samples at 1–5 kHz on loopback, with no latency gain.
- **Operations** carry a recommended `priority`. Zenoh replies inherit the
  query's QoS, so the caller applies it.

### 2.5 Advanced publication

zenoh-ext's advanced publication (history, miss detection) MAY be used on
plain `stream` and `state` resources only. A contract MUST NOT declare
`history` on an explicit resource, an event or an operation.
`[F: contracts/e019-history, e034-history-depth]`

Under a verbatim chunk, zenoh-ext's `@adv` key parser fails, so a late
publisher is never found and heartbeat recovery never runs. History from
publishers already present still works there.

### 2.6 Events

- **One key per occurrence.** An owner publishes each occurrence once, with
  a one-shot put, on `…/<iface>/events/<template>/<ulid>`. The last chunk is
  a lowercase ULID minted by the owner. `[F: keys.json]`
- **`rate`** is `rare` (at most 1/h), `low` (at most 1/min) or `burst(<n>/h)`.
  **`retention`** is `<n>` followed by `s`, `m`, `h`, `d` or `w`. Both are
  required on an event. `[F: contracts/e026-rate]`
- **Storage.** A deployment MAY run a union storage on `zk2/*/*/*/events/**`.
  An event key is never an owner's state (§4.4).
- **Replay.** A consumer replays with a wildcard GET bounded by the
  retention. Enforcing the bound needs a time-series backend, or a filter on
  the consumer's side: the storage manager's memory backend ignores `_time`
  (spike S5). `[Sc: state.md §8]`

---

## 3. Requirements, bindings and the descriptor

### 3.1 Requirements

A **requirement** is a named role, typed by an interface, through which data
reaches a component from bound providers.

```toml
[requires.cmd]
interface   = "twist_cmd.v1"
resources   = ["cmd"]          # what is consumed (default: everything)
cardinality = "many"           # "one" | "many" (default "one")
optional    = false
```

A requirement MAY be declared in a contract, where it is fingerprinted, or in
a component's own manifest, where it is not. `resources` names resources of
the required interface, and MUST NOT be empty.
`[F: contracts/e030-requirement]`

### 3.2 Binding rules

| # | Rule | Evidence |
|---|---|---|
| R1 | A role is bound at deployment, never in code. A binding is a list of service addresses, exact (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, meaning every service on `vehicle-01` that implements the interface). The binding configuration's format is a recommendation, not a rule. | `[Sc: bindings.md §1]` |
| R2 | A template parameter MAY be bound too: `{vehicle} = self` binds the consumer to its own slice of a provider's collection. `self` resolves to the consumer's own system or service, whichever the binding names. | `[Sc: bindings.md §2]` |
| R3 | The descriptor (§3.3) lists every requirement with its bindings as declared. The data-flow graph is read from descriptors, never inferred. | `[Sc: bindings.md §3]` |
| R4 | A consumer compiled against `X.vN` binds to providers of any revision of `X.vN` (§9.6). | `[F: pending]` (the compatibility matrix) |
| R5 | A consumer MAY wait on presence for its bound providers. A binding resolves at once without it. | `[Sc: bindings.md §1]` |
| R6 | A consumer MUST discard a sample whose key expression is not concrete. Zenoh delivers a put on a wildcard key with the publisher's key. | `[Sc: bindings.md §4]` |
| R7 | A binding MUST NOT require presence. Across a constrained face, a consumer binds statically and judges liveness from the freshness of what crosses. Where nothing crosses, liveness is *unobservable*, and a tool MUST say so rather than report the provider down. | `[Sc: constrained.md §3]` |

When several providers are bound, choosing between them is the consumer's
(`arbitration.v1`).

### 3.3 The descriptor record

Every instance serves a **descriptor**: a JSON document answered on GET at
its instance key, and put on every change. Its schema is
[`descriptor.schema.json`](descriptor.schema.json). `[F: pending]` (the
schema and its valid and invalid documents)

```json
{
  "format": "zk2-descriptor/0.1",
  "service": "vehicle-01/navigation",
  "instance": "3fa9c2d41b7e0012",
  "interfaces": [
    {"iface": "nav.v2", "contract": "sha256:…", "minor": 1,
     "unavailable": [{"resource": "state/covariance", "cause": "capability", "reason": "no IMU"}],
     "cardinality": {"tracks/{track}": 50}}
  ],
  "capabilities": ["imu", "gnss"],
  "requires": [
    {"role": "cmd", "interface": "twist_cmd.v1", "declared_by": "thruster.v1",
     "bindings": ["vehicle-01/safety", "vehicle-01/teleop"]}
  ],
  "profiles": ["freshness.v1"],
  "meta": {"host": "…", "build": "…", "zid": "…"}
}
```

- **Exposure is compact.**
  - The exposed resources of an interface are its contract's resources,
    minus the optional ones gated on a capability the instance does not
    hold, minus those listed in `unavailable`.
  - `unavailable` lists only optional resources whose absence the missing
    capabilities do not already imply.
  - Each entry gives a cause, `build`, `config` or `capability`, saying why
    the resource is absent here.
  - `capabilities` lists the capabilities held.

  `[F: pending]`
- **`cardinality`** MAY lower a template's bound for this instance. It MUST
  NOT raise it. `[F: pending]`
- **`requires`** lists each role with its bindings (R3). A role declared in a
  contract names that contract's interface in `declared_by`. A role declared
  by the component's manifest names `null`.
- **Size.** A descriptor SHOULD stay within 1 KB, and MUST fit one fragment
  at the constrained level (§12).
- **Updates.** The owner MUST put the descriptor on its instance key whenever
  it changes, and MUST answer a GET there with the current one.
  `[Sc: presence.md §2]`

---

## 4. State

### 4.1 Zenoh facts this section relies on

- Routers stamp puts, where timestamping is enabled. Peers and clients do
  not stamp by default. Deletes and query replies are never stamped by the
  network.
- `Latest` consolidation keeps, per key, the reply with the greatest
  timestamp, ranks an unstamped reply lowest, and delivers at query
  completion.
- `reply_del` is stable. `Session::new_timestamp()` uses the HLC when it is
  enabled, else a wall clock with the zid.
- Routers re-stamp a future-dated put (beyond the HLC's maximum delta, 500 ms
  by default), or drop it with `timestamping.drop_future_timestamp`. They
  never re-stamp a GET reply.
- Every Zenoh timestamp carries the id of the HLC that issued it, which is
  the issuing session's zid.

### 4.2 Rules for owners and consumers

| # | Rule | Evidence |
|---|---|---|
| S1 | **Owner:** every state mutation, put **and** delete, MUST carry a timestamp the owner set. | `[Sc: state.md §1]` |
| S2 | **Owner:** a GET reply MUST carry the timestamp of the mutation it represents. | `[Sc: state.md §1]` |
| S3 | **Owner:** for a key it deleted within the tombstone window, the owner MUST answer a GET with `reply_del` and the deletion's timestamp. The window is 60 s unless the contract's annotation sets another. A deployment MUST raise it, for state recorded by an archive across a constrained face, to at least that face's maximum outage (`link.v1`'s face policy, which is not fingerprinted). | `[Sc: state.md §2, §6]` |
| S4 | **Consumer:** a state GET is addressed to the owner's keys, with target `All` and consolidation `Latest` set explicitly. **Deployment:** no storage may answer on an owner's `state/**` or `@state/**` keys. | `[Sc: state.md §3]` |
| S5 | **Archive:** last-known state is read from an archive (§4.4), explicitly. | `[Sc: state.md §4–§6]` |
| S6 | **Consumer:** current state is the owner's answer. A consumer turns to an archive only when the owner gave no reply within the GET's timeout, or presence shows it absent. That silence is not a verdict about the key (O5). The archive's answer is last-known, never current, and a consumer MUST NOT present it as current. | `[Sc: state.md §4]` |
| S7 | **Owner:** clocks are bounded both ways (§4.3). | `[Sc: state.md §7]` |

A **tool** checks S4 against the routers' storage admin space. A consumer
cannot tell under `Latest` which replier answered.

### 4.3 Clocks (S7)

- **HLC.** A session that serves state MUST enable Zenoh's HLC. At the
  constrained level (§12), a wall clock with a per-session bump takes its
  place.
- **Catch-up.** An owner MUST NOT stamp at or below the last timestamp it
  issued for its keys. Before its first write, it takes that timestamp from
  its own persistent record, else from an explicit read of an archive on its
  own side. A consumer-side archive can be stale by a whole partition.
- **New epoch.** With neither source, the owner MUST start a new epoch: a new
  instance (§1.5) whose session has a fresh, unpinned zid.
  - **The epoch rides each value.** Every timestamp carries its HLC's id
    (§4.1), so no per-sample attribution is needed.
  - **Consumer rule.** A consumer MUST order an owner's values by time within
    one timestamp id. It MUST accept the first value under a new id, and
    restart its ordering there.
  - **Residual risk.** A value from the old epoch that is still in transit
    can arrive after the new epoch starts. That is accepted at this version.
- **Ahead.** An owner MUST NOT stamp beyond its router's HLC delta.
  - It SHOULD detect drift against a reference it holds for the purpose,
    such as a subscription to a router-stamped heartbeat key, because its
    own puts are never echoed back.
  - When a detection shows it beyond the delta, it stops writing state and
    reports through `health.v1`.
  - Spike S12 showed the harm: a clock 2 s ahead produced a revision with
    two timestamps, and the next correctly clocked write looked stale.

### 4.4 Archives

An **archive** is a service implementing `archive.v1` (a profile, #613). This
section is what the core requires of it.

- **It records** owners' mutations under its own address, with each
  mutation's timestamp and type identity (§7.1), and keeps tombstones for at
  least the window (S3).
- **Its key** is the archived key's address, slugged as a rest parameter,
  under the archive's own `@state` token:
  `zk2/<system>/<archive>/archive.v1/@state/{origin...}`. For example,
  `zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01`.
  A verbatim chunk of the origin, such as `@state`, is slugged to
  `x-_x40state`.
- **It answers GETs only.** It never puts on its keys, because it is not a
  second publisher of the data. Each reply carries the archived value's
  type identity (interface, contract fingerprint, type) in its attachment.
- **Its backend MUST NOT accept a put older than a delete it holds.** The
  storage manager of zenoh 1.10.1 accepts one, and resurrects the key, so it
  cannot be an archive's store as released.
- **Alignment.** When an owner becomes reachable again, the archive MUST
  re-read the owner's recorded collection before serving it again. While the
  owner stays absent, it re-reads it from an archive on the owner's side. It
  drops the keys the source neither reports nor tombstones, and only after
  the source's GET completed without a timeout. A partial reply set is not a
  verdict (O5).
- **Placement.** An archive on the consumer's side covers losing the link,
  and one on the owner's side covers losing the owner. Store-and-forward
  (`desired.v1`) uses both.

`[Sc: state.md §4–§6]`

---

## 5. Operations

### 5.1 Rules

| # | Rule | Evidence |
|---|---|---|
| O1 | **Owner:** an operation's queryable is declared on its concrete key (or its template), and is `complete`. A concrete call with `BestMatching` then executes on at most one instance **while one instance serves the operation**. `BestMatching` reaches one `complete` queryable on each router, so a split-brain across routers runs a call on each side. Exclusivity beyond that is `redundancy.v1`'s. | `[Sc: operations.md §1]` |
| O2 | **Owner:** a call whose key expression is not concrete MUST be refused with `fanout_forbidden`, unless the operation declares `fanout = "allowed"`, whatever the access control allows. **Caller:** a call to a fan-out operation uses target `All` and consolidation `None`. | `[Sc: operations.md §2]` |
| O3 | **Owner:** a reply goes on the operation's own concrete key. Success is a value reply; failure is a `reply_err` carrying the error envelope (§5.2). | `[Sc: operations.md §3]`; `[F: pending]` (the envelope) |
| O4 | **Caller:** only an operation declared `idempotent` MAY be retried. | `[Sc: operations.md §4]` |
| O5 | **Caller and tool:** an empty reply set is not a verdict. Access-control refusals return empty since zenoh 1.3; a tool attributes silence through presence. | `[Sc: operations.md §5]` |
| O6 | **Caller:** an operation declared `replies = "many"` gives zero or more value replies, then completion. The caller MUST use consolidation `None`: `Latest` and `Auto` kept 1 reply of 10 in spike S6. With a declared `summary`, each replier ends with exactly one summary reply. | `[Sc: operations.md §6]`; `[F: contracts/e033-summary]` |
| O7 | **Caller:** a request MAY carry an attachment `{actor, request_id}`, which an owner MAY record for audit. It is claimed, never authentication. | `[Sc: operations.md §7]` |

Work that outlives a query timeout belongs to `jobs.v1`. A request type
SHOULD NOT repeat a template parameter. `[F: contracts/w102-repeat]`

### 5.2 The error envelope

A failed call replies with `reply_err` and an envelope with these fields:

| Field | Type | Meaning |
|---|---|---|
| `code` | string | One of `invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app` |
| `message` | string | For a human. Tools MUST NOT parse it. |
| `cause` | string or null | With `unavailable` only: `build`, `config` or `capability` (§2.3) |
| `detail` | bytes or null | With `app` only: the operation's declared `error` type |

**Encoding.**
- **A JSON Schema operation** (its types are `json:` types) encodes the
  envelope in its `encoding`, JSON or CBOR. `detail` is then the `error`
  type's value inline.
- **A protobuf operation** encodes it as the message `zk2.core.v1.Error`,
  with `detail` as the encoded `error` message.
- **The reply's `Encoding`** MUST say which:
  - `application/json` or `application/cbor`;
  - or `application/protobuf` with the schema suffix `zk2.core.v1.Error`.

The two definitions are [`core/error.proto`](core/error.proto) and
[`core/error.schema.json`](core/error.schema.json). `[F: pending]`

---

## 6. Ownership and serving

- **Ownership.** Every key under `zk2/<system>/<service>/` is written only by
  that service: its puts, deletes, queryables and tokens. No other principal
  writes there. This is the invariant §11's grants rest on.
- **Serving.** An operation is `exclusive` by default: at most one instance
  at a time exposes it. `serving = "replicated"` lets any number expose it,
  costs one execution per router per call, and requires
  `idempotent = true`. `[F: contracts/e018-replicated]`
- **No core roles.**
  - A standby instance exposes no exclusive resource, and declares no
    interface token for an interface it exposes nothing of.
  - Two instances of one service holding an interface token for the same
    interface, for longer than a grace period, is a **finding**. A tool
    diagnoses it; the runtime does not fence. The grace period MUST exceed
    a re-mint's overlap (§8.1). `[Sc: operations.md §8]`
  - Election and fencing belong to `redundancy.v1`. An actuator keeps an
    exclusivity lock outside the bus.

---

## 7. Types and schemas

### 7.1 Type identity

Every resource payload, attachment, request, response, error and summary
has a **type identity**: (kind, name, sha256 of its schema artifact).

| Kind | Generic tools | Use |
|---|---|---|
| `protobuf` | MUST decode | proto2 and proto3. Editions are not supported. |
| `jsonschema` | MUST decode | The zk2 subset of JSON Schema 2020-12 (§7.3) |
| `flatbuffer` | MAY decode | Large payloads read in place |
| `ros2msg`, `cdr` | MAY decode | The ROS 2 bridge; RIHS01 is the schema hash |
| `raw` | media type only | Images, audio, opaque buffers |

**Type references in a contract:**

| Spelling | Means |
|---|---|
| `"nav.v2.Position"` | a protobuf message, fully qualified, defined in a listed file. `google.protobuf.*` is always available. |
| `"json:Status"` | `$defs/Status`. The name MUST be unique across the listed files. |
| `"json:telemetry#Point"` | `$defs/Point` in the listed file whose stem is `telemetry` |
| `{ raw = "image/jpeg" }` | opaque bytes of a media type |
| `{ raw = "video/*", media_param = "codec" }` | a raw family. The sample's `Encoding` carries the concrete subtype, and `media_param` names the single-chunk template parameter tied to it. |

A reference that does not resolve is an error. So is a `$ref` outside the
listed files.
`[F: contracts/e023-type, e024-ambiguous, e025-media-param, e029-schema, e032-ref-dangling, e032-ref-outside, ok-json-qualified, ok-protobuf-well-known]`

### 7.2 Encodings and rendering

- **The sample's `Encoding`.** An owner MUST set the predefined Zenoh
  `Encoding` id on every sample. It MUST NOT add a schema suffix, except on
  the error envelope (§5.2). A suffix is sent with every put, and is capped
  at 255 bytes. `[Sc: types.md §1]`
- **JSON Schema types** are encoded as JSON (the default) or CBOR, declared
  by `encoding`. Bytes are CBOR byte strings, or base64 in JSON. An
  attachment declares its own `attachment_encoding` by the same rule.
  Protobuf is always binary, and `encoding` on a protobuf or raw type is
  ignored, with a warning. `[F: contracts/w103-encoding]`
- **Decode order** for a tool is: the sample's `Encoding`, then the
  contract's type, then sniffing.
- **Honest rendering.** A tool MUST render a kind it cannot decode as its
  declared type and size, or through a plugin. It MUST NOT show garbage, and
  MUST NOT drop the sample silently. `[Sc: types.md §2]`

### 7.3 The JSON Schema subset

- **Keywords that carry meaning:**
  - `type`, `properties`, `required`, `additionalProperties`, `items`;
  - `enum`, `const`;
  - `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`;
  - `minLength`, `maxLength`, `minItems`, `maxItems`;
  - local `$ref`, and `$ref` to another listed file by its stem;
  - `oneOf` with a discriminator.
- **Annotations, ignored:** `$schema`, `$defs`, `$comment`, `title`,
  `description`, `default`, `examples`, `format`, `deprecated`, `readOnly`,
  `writeOnly`.
- **Refused:** every other keyword (`pattern`, `allOf`, `anyOf`, `not`,
  `if`/`then`/`else`, `patternProperties`, …). Containment is not decidable
  for them. A contract whose schemas use one does not load.
  `[F: pending]` (one fixture per refused keyword class)

### 7.4 Shared memory

Zenoh SHM is transparent to keys. A generated `@stream` publisher SHOULD
accept SHM buffers.
- zenoh-shm 1.10.1 locks every segment it creates or maps: the pool, plus
  1,280 KiB of metadata.
- A deployment SHOULD set `RLIMIT_MEMLOCK` to at least the pool plus 2 MiB.
  Below the pool's need, SHM silently falls back to TCP, and a metadata
  segment that cannot be locked panics.
- A tool SHOULD check the limit at start.

---

## 8. Presence, descriptors, contract retrieval

### 8.1 Tokens

Liveliness tokens carry no payload; everything is in the key.
- **Instance token,** at `…/@zk/instance/<instance>`, the same key as the
  descriptor's queryable. Every instance MUST hold one, including pure
  consumers, so the data-flow graph has no invisible node.
  `[Sc: presence.md §1]`
- **Interface token,** at `…/@zk/alive/<iface>/<instance>/<fp16>`. An
  instance holds one per interface **of which it currently exposes at least
  one resource**, and holds none otherwise. "Who implements `nav.v2`" is
  the liveliness selector `zk2/*/*/@zk/alive/nav.v2/**`.
  `[Sc: presence.md §1]`
- **Member token,** at `…/@zk/member/<iface>/<member>/<epoch>`, for a
  template that declares `epoch`. The owner holds one per member, and cycles
  it whenever that member's continuity breaks. `[Sc: presence.md §3]`
- **Re-minting is make-before-break.** An owner that starts a new instance
  id while running declares the new tokens and descriptor first, then
  undeclares the old ones. There is an overlap, never a gap.
  `[Sc: presence.md §3]`
- **Reading presence.** A liveliness GET on a session that holds a
  liveliness subscriber MUST use a callback or an unbounded handler. With
  zenoh's default 256-slot handler, such a GET hung at every measured size
  from 996 tokens (zenoh#2678). `[Sc: presence.md §4]`

### 8.2 Start-up order

An owner MUST bring itself up in this order, so that alive ⇒ callable:
1. declare its resources (publishers and queryables);
2. validate that every required resource is exposed;
3. declare the descriptor's queryable and the contract queryables;
4. declare the instance token, then the interface tokens.

`[Sc: presence.md §1]`

### 8.3 The presence budget

Presence costs per token, whatever the layout. A router holds 2.1–2.4 KiB per
token, and a router-to-router link carries 63–85 B per declaration (spike
S2). A **presence domain** is the set of routers whose router-to-router
links carry each other's declarations.

A deployment SHOULD keep a domain within about 10–15k tokens, which kept
discovery within 2–4 s in spike S2. At 50k tokens, discovery took 46–49 s or
never finished. The budget is shared with every other declaration, which
spike S2 did not measure.

### 8.4 Contract retrieval

Contract bundles (§9.4) live at the location-free key
`zk2/@zk/contract/<iface>/<sha256>`. Every holder declares a `complete`
queryable on it. A holder MAY be any participant, because the hash is the
check. A client MUST retrieve a bundle as follows:

1. GET with target `BestMatching`. That reaches the nearest holder on each
   router the query visits.
2. Verify each reply **as it arrives** (§9.4), and accept the first valid
   one, without waiting for the GET to complete.
3. If none was valid, retry once with target `All`.
4. If still none, report the contract **unavailable**. Never accept an
   unverified bundle.

`[Sc: retrieval.md §1–§3]`

### 8.5 Constrained faces

A deployment marks a face constrained in `link.v1`'s face policy.
- **No `@zk` traffic is required across it.** Bindings resolve statically
  (R7), and bundles come from holders on the same side or are
  pre-provisioned.
- **Attachment decides what crosses.**
  - A router-to-router link carries every declaration of both sides. A
    `@zk` deny on it hides presence from the far side, but the denied
    declarations still cross, key strings included (zenoh 1.10.1).
  - A session that is a **client** of the near router receives only the
    declarations its interests ask for.
- **A deployment SHOULD attach one far-side session, or a gateway session,
  as a client, with a `@zk` deny on the face.** A gateway:
  - is the only session on its side that talks across the face;
  - is a principal of its own;
  - never answers or republishes on the near side's keys, and re-keys what
    it relays under its own address.

  A far side with several sessions, or with its own router, is open (U23).
  `[Sc: constrained.md §1]`
- **A batch MUST cross the face well within the lease.** A link's batch is
  the minimum of the configured size, the link's MTU and the other end's.
  Over TCP, the face therefore needs a router configured for it, with about
  1 KB batches at 2,400 bit/s. `[Sc: constrained.md §4]`
- **`@stream` keys SHOULD be denied across the face** unless `link.v1`
  downsamples them. A named key's frames are queued, not dropped, and every
  request waits behind them.

---

## 9. Contracts and bundles

### 9.1 The authoring format

A contract is a TOML file, `<name>.v<major>.toml`, one per interface major.
Its shape is [`contract.schema.json`](contract.schema.json) (JSON Schema
2020-12). A reader MUST refuse an unknown field. The reference for each
field is `examples/zk2/README.md` ("draft 1"). `[F: contracts/e000-unknown-field, e000-not-toml]`

**Lints.** A contract's checks report diagnostics with stable codes. An
implementation MUST report, for each fixture, exactly the codes listed in
`contracts/expect.json`, repeats included, in sorted order. Codes `E…` make
a contract invalid; codes `W…` do not. `[F: contracts/*, expect.json]`

| Codes | What they check |
|---|---|
| E000 | the file is TOML, and fits the format's shape |
| E001, E002 | the interface id; each `uses` entry is `<name>.v<major>` |
| E010–E013, E021 | templates, parameters, parameter types, `cardinality`, shape uniqueness |
| E014, E015 | fields allowed and required per kind |
| E016, E017 | gates |
| E018 | `replicated` requires `idempotent` |
| E019, E034 | `history`: where it is allowed; depth ≥ 1 |
| E020 | annotation keys and `uses` |
| E022, E025 | `epoch` and `media_param` name a single-chunk parameter |
| E023, E024, E029, E032 | type references, names, schema files, `$ref` |
| E026 | `rate`, `retention` |
| E027, E028 | the canonical restrictions (§9.3) |
| E030, E035 | requirements; a required resource the interface does not declare |
| E031 | `deprecated` |
| E033 | `summary` needs `replies = "many"` |
| E036 | two contracts declare one interface id |
| W101–W105, W107 | overlapping types; a repeated parameter; an ignored encoding; no `minor`; an annotation outside the profile's vocabulary; the file name |

A fixture's codes are independent of the order lints run in, because of four
cascade rules:
1. a template that does not parse (E010) stops the other checks of its
   resource;
2. a schema file that fails to load (E029) suppresses E023 for its schema
   kind;
3. shapes (E021) and `deprecated` (E031) are checked over every template
   that parses;
4. overlaps (W101) are checked over resources without errors.

E035 and E036 are set checks over several files. `[F: pending]` (set fixtures)

### 9.2 Defaults

`[defaults]` sets fields for every resource, and `[defaults.<kind>]` for
every resource of a kind. A resource's own field overrides a default.
Defaults apply only where their field is legal: `[defaults] history = true`
skips `@stream`, events and operations. A field in `[defaults.<kind>]` that
the kind does not take is an error. Defaults are expanded **before**
canonicalization, so a defaulted contract and one with every value spelled
out MUST have the same fingerprint.
`[F: contracts/ok-defaults-defaulted, ok-defaults-spelled]`

### 9.3 The canonical form and the fingerprint

The canonical form of a valid contract is a JSON value:
- **Fully explicit.** Every field of every resource is present, `null` when
  absent, with defaults expanded.
- **Excluded:** documentation (`doc`, `summary`) and `minor`.
- **The format tag:** `"format": "zk2-contract/draft-1"`.
- **Ordering:**
  - resources are a list sorted by (kind token, template);
  - gates, and a requirement's resources, are sorted sets without
    duplicates;
  - object members are ordered by JCS.
- **Schemas.** `schemas` lists every artifact the contract carries, as
  `{id, kind, name}`, including files that are only `$ref`'d. A change to any
  of them changes the fingerprint.
  - A JSON Schema artifact's id is `sha256:` + the hex sha256 of its JCS
    bytes.
  - A protobuf artifact is the `FileDescriptorSet` of the listed file and
    its imports, and its id is the sha256 of those bytes.
- **Restrictions.** Every string is printable ASCII, and every integer is
  within ±(2^53−1). A contract that breaks either is invalid
  `[F: contracts/e027-ascii, e028-integer]`. On that domain every JCS
  implementation agrees.

The canonical bytes are the RFC 8785 (JCS) serialization of that value. The
**fingerprint** is `sha256:` followed by the lowercase hex sha256 of the
canonical bytes. An implementation MUST produce, for every valid fixture,
exactly the bytes in `<stem>.canonical.json` and the fingerprint in
`expect.json`. `[F: contracts/*.canonical.json, expect.json]`

**Portability.** Canonical bytes are portable for JSON Schema and raw types.
For protobuf, they are portable only between implementations that encode
`FileDescriptorSet`s alike: `protox` 0.9 matches `protoc` 3.21 byte for byte
(spike S7). **Verifying** a bundle is portable for every kind, because a
bundle carries the bytes. Bundles are therefore built once, by the
contract's CI, and only verified elsewhere.

### 9.4 Bundles

A bundle is the canonical contract with every schema artifact it lists:

```json
{"contract": <canonical contract>,
 "schemas": {"sha256:…": {"kind": "protobuf", "data": "<base64 FileDescriptorSet>"},
             "sha256:…": {"kind": "jsonschema", "data": <JSON Schema document>}},
 "extras": {"sha256:…": {"data": <JSON document>}}}
```

A bundle file is the JCS serialization of that object, so one contract
revision always has one bundle. To verify bundle bytes, an implementation
MUST check, in order, refusing with the given tag at the first failure:

| Step | Check | Tag |
|---|---|---|
| 1 | The bytes are UTF-8 JSON with no duplicate member | `json` |
| 2 | The top level is an object with no member besides `contract`, `schemas`, `extras` | `shape` |
| 3 | `contract.format` is `zk2-contract/draft-1` | `format` |
| 4 | The contract keeps the restrictions of §9.3 | `restrictions` |
| 5 | Every schema in the bundle is listed by the contract | `unlisted_schema` |
| 6 | Every listed schema is in the bundle | `missing_schema` |
| 7 | Each schema's `kind` is the one listed | `schema_kind` |
| 8 | Each schema hashes to its id: the base64-decoded bytes for protobuf, the JCS bytes for jsonschema | `schema_hash` |
| 9 | Each extra's `data` hashes, by JCS, to its id | `extra_hash` |
| 10 | Where the caller expects a fingerprint (from a contract key or a descriptor), the contract's fingerprint equals it | `fingerprint` |

A malformed member inside the steps (no `data`, data that is not base64, an
unknown kind) is refused as `shape`. `[F: bundles/*, expect.json]`

Integrity is Merkle-style. The fingerprint covers the canonical contract,
the contract lists every schema's id, and each schema is checked against its
id, so the whole bundle is verified. **Extras** (`views.v1` documents) are
carried and verified, never interpreted.

### 9.5 History and retention

- **History.** A contract's CI keeps every published bundle at
  `contracts/.history/<iface>/<hex>.bundle.json`, where `<hex>` is the
  fingerprint without `sha256:`. The directory is append-only. A history
  check MUST verify every bundle (§9.4), its fingerprint against its file
  name, and its interface against its directory. `[F: pending]`
- **Retention.** A rebuild that the classifier judges identical to the
  newest published revision keeps that revision's bundle, and mints no new
  fingerprint. For protobuf, the identity check compares `FileDescriptorSet`s
  normalized: source info dropped, and every `json_name` equal to its
  default dropped. Fingerprints may over-detect a change, never
  under-detect one. `[F: pending]`

### 9.6 Compatibility

Every revision inside a major MUST be **FULL_TRANSITIVE** compatible with
every earlier revision in the history, in both directions. Each rule is
judged per direction: for streams, state, events and responses the owner
writes and the consumer reads; for requests the caller writes and the owner
reads. A change is **compatible**, **review** (a human decides) or
**breaking**. `[F: pending]` (the matrix, with transitive cases)

**Contract metadata:**

| Change | Class |
|---|---|
| `idempotent` true → false | breaking |
| `fanout` allowed → forbidden | breaking |
| `reliability` reliable → best_effort | review |
| optional → required | breaking for implementers |
| A value added to a contract-level enumeration | review |
| `priority` changed, `express` toggled | review |
| `congestion` drop → block | review (it can stall the producer) |
| `explicit` false → true | breaking for ambient consumers |
| `explicit` true → false | review (link budgets) |
| `replies` one → many | breaking (callers' consolidation) |
| A required role added to an interface | breaking |
| An optional role added | compatible |
| A role's interface or cardinality changed | breaking |

**Protobuf payloads:** WIRE semantics with renumber detection. A reader
ignores unknown fields and defaults missing ones.
- **Breaking:**
  - a field's declared scalar type changes, including int32 → int64 and
    string → bytes;
  - its cardinality changes;
  - it moves into or out of a oneof;
  - it is renumbered: a deletion plus an addition of the same field, which
    silently drops the data both ways.
- **Review:** a field renamed, an enum value renamed or deleted, a
  `json_name` option.
- **Compatible:** a field added; a value added to a proto3 (open) enum. A
  value added to a proto2 (closed) enum is review.
- **Warning:** a field deleted without reserving its number. Reuse is caught
  against the whole history.

**JSON Schema payloads:** the subset of §7.3. Readers tolerate unknown
properties, and writers send only what their schema declares.
- **Compatible:** an optional property added, even to a closed schema, or
  removed.
- **Breaking:**
  - a required property added, or optional → required, or required →
    optional;
  - integer ↔ number;
  - an enum value added or removed;
  - a bound changed (a tightened `maximum` breaks old writers, a loosened
    `maxLength` breaks old readers);
  - a `oneOf` branch added.

---

## 10. Extension points for profiles

A **profile** is an independently versioned specification, such as
`timing.v1` or `archive.v1`. The core never depends on one. A profile
contributes through exactly four points:
1. **A standard contract**: an interface it defines.
2. **An annotation vocabulary.** Keys are `<profile>.<key>`. A contract
   that uses one MUST list the profile in `uses`, which is fingerprinted.
   `[F: contracts/e020-annotation]` Until a profile publishes its
   vocabulary, the interim tables of `examples/zk2/README.md` apply, and a
   key outside them is a warning. `[F: contracts/w105-vocabulary]`
3. **A registered verbatim kind**, such as `@blob`, which a profile adds at
   position 5.
4. **The descriptor's `profiles` list.**

---

## 11. Security

### 11.1 Grant shapes

Ownership (§6) reduces access control to three grant shapes:

| Grant | Rule |
|---|---|
| **Own** | A service principal puts, deletes, declares queryables and declares tokens under `zk2/<system>/<service>/**`. It also holds each verbatim subtree, spelled out because `**` never crosses one: `…/*/@stream/**`, `…/*/@state/**`, `…/*/@op/**`, `…/@zk/**`. |
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name. |
| **Call** | Query on specific `…/@op/<op>` keys. |

- **Contract bundles are open:** any principal MAY hold or fetch
  `zk2/@zk/contract/**`, because the hash is the check.
- **There are no cross-principal write grants.**
- **An archive principal** has Own on its own prefix, plus Consume on what it
  records.

`[Sc: security.md §1]`

### 11.2 Compiling grants

- **Under `default_permission: deny`**, a grant compiles to allow rules.
  This is RECOMMENDED where the router can afford the enumeration.
- **Under `allow`**, Zenoh does not evaluate allow rules. A grant compiles
  into denies of its complement, regenerated on every contract revision.
- **Egress is checked by inclusion** against the query's or subscription's
  own key expression. A generator MUST therefore add every consumer
  selector that intersects a provider's keys to that provider's egress
  grant. It MUST also grant the same selectors for the provider's ingress
  `reply`, refusals included.

`[Sc: security.md §2]`

### 11.3 Facts a deployment designs around

- Under `default_permission: allow`, a put on a wildcard key bypasses a
  deny on a concrete key it covers. P3's guarantees rest on O2 and R6, with
  access control as defense in depth.
- Deny works by inclusion: deny with the widest pattern.
- Multicast transports bypass access control.
- `zids` subjects are unauthenticated. Principals are bound by certificate
  CN or username.
- Access control is enforced per hop, and the running configuration is not
  observable on the bus.
- On a router-to-router link, a deny hides a denied declaration from the far
  side, but its key string still crosses. Access control is not a
  confidentiality boundary for key names there (§8.5).

---

## 12. Conformance levels

**Full.** An implementation at the full level meets every rule above.

**Constrained.** An implementation at the constrained level meets every rule
above, with these relaxations and additions:

| Topic | Relaxation or addition |
|---|---|
| Clock (S7) | A wall clock with a per-session bump instead of the HLC. The catch-up and the ahead bound still apply. |
| Namespace (§1.6) | The deployment prefix written literally in keys |
| Descriptor (§3.3) | MUST fit one fragment (4 KB with zenoh-pico's default `Z_FRAG_MAX_SIZE`) |
| Bundles (§8.4) | Served by a gateway holder where the device cannot receive or hold them. A zenoh-pico 1.10.1 owner sent 100 KB replies, so the limit is on receiving. |
| Faces (§8.5) | Attached as a client, with batches within the lease, `@zk` denied, and `@stream` denied unless downsampled. Presence never crosses. |

`[Sc: constrained.md]`

---

## Appendix A. Rationale: lessons v1 paid for

These are reasons, not rules. Each was a real bug or outage in v1.

| Lesson | Where zk2 carries it |
|---|---|
| Verbatim chunks make separation hermetic, and they break zenoh-ext's `@adv` parser. | §1.3, §2.5 |
| Fan-in: target `All`, reply on your own concrete key, consolidation `None`, attribute silence through presence. | O2, O5, O6 |
| Silence is never a verdict. "Not asked" and "unobservable" are different facts. | O5, R7, S6 |
| Alive ⇒ callable: declare queryables before presence. | §8.2 |
| Presence is not a lock. | §6 |
| Deprecate, never reuse, and keep the ledger append-only. | §2.3, §9.5 |
| No implicit identity in a builder. | §1.5 |
| Servers refuse broadcast writes; access control cannot. | O2 |
| No cross-key atomicity: a snapshot is one document on one key. | §2.1 |
| Set `Encoding` on every sample. | §7.2 |
| Retire state with a Zenoh delete, never a payload marker. | S3 |
| Injective slugging ships with its decoder and a round-trip test. | §1.4 |
| No type hash in keys, no central schema registry, no per-sample schema id. | §7, §8.4 |
| Core participants use only stable Zenoh API. | §0 |

## Appendix B. Zenoh 1.10.1 facts relied on

The sourced list, with file and line, is `docs/zk2/architecture.md`
Appendix B. These are the ones the rules above cite:
- Liveliness tokens carry no payload, and routers hold every token.
- Liveliness and data are separate key spaces.
- `*` and `**` never match a verbatim chunk.
- A put on a wildcard key is legal, and subscribers receive the publisher's
  key.
- `BestMatching` reaches the nearest `complete` queryable on each router.
- Routers stamp puts, not deletes or replies, and re-stamp future-dated puts
  beyond the HLC delta (500 ms).
- A timestamp carries its HLC's id, the zid.
- A client connects to one endpoint at a time.
- A link's batch is the minimum of the configured size, the MTU and the
  other end's.
- A query timeout arrives as a reply error.
- Under `allow`, allow rules are not evaluated, and access control works by
  inclusion.

## Appendix C. Evidence index

| Evidence | Covers |
|---|---|
| `conformance/keys.json`, `slugs.json`, `templates.json` | §1, §2.2 |
| `conformance/contracts/` | §2, §7, §9.1–§9.3, §10 |
| `conformance/bundles/` | §9.4 |
| `scenarios/grammar.md` | §1.3, §1.6 |
| `scenarios/state.md` | §4 |
| `scenarios/operations.md` | §5, §6 |
| `scenarios/presence.md` | §1.5, §3.3, §8.1–§8.2 |
| `scenarios/bindings.md` | §3.2 |
| `scenarios/retrieval.md` | §8.4 |
| `scenarios/types.md` | §7.2 |
| `scenarios/security.md` | §11 |
| `scenarios/constrained.md` | §1.6, R7, §8.5, §12 |

**Pending fixtures (#607):**
- the descriptor schema and documents;
- the error envelope;
- the JSON Schema subset's refusals;
- the set checks E035 and E036;
- history and retention;
- the compatibility matrix with transitive cases;
- identity in builders (§1.5).
