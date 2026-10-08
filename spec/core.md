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
  application source (explorers, CI, generators);
- **implementation**: any software claiming conformance. The static rules
  (§1, §9) bind it, whichever of the roles above it plays.

An archive (§4.4) is an owner of its own keys.

**Evidence.** Each MUST cites what checks it:
- `[F: path]` is a fixture under `conformance/`, which every implementation
  runs;
- `[Sc: file §n]` is a scenario under `scenarios/`: a network behaviour with
  a setup, steps and expected observations;
- `[F: pending]` would mark a fixture still owed. None is, in this version
  (#607).
- `[F: compat/]` is the one family whose expected values no implementation
  checks yet. They are written by hand, and the classifier (#618) evaluates
  them. Until then, the reference runner only checks that every input
  loads.

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
| 6+ | resource chunks | Built from the resource's template (§2.2). In a member token, `<member>` is the slugged value of the template's `epoch` parameter. |

A key that fits none of the five forms is not a zk2 key. An implementation
MUST refuse to build one, and MUST report one it is asked to parse as not a
zk2 key. Every key it accepts MUST build back to the same string.
`[F: keys.json]`

### 1.2 Lexical rules

- **A plain chunk** is `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`. Literal template
  chunks, slugged parameter values, system names, service names and member
  values are plain chunks. `[F: keys.json]`
- **A verbatim chunk** starts with `@`. In zk2, only the reserved tokens
  use one: `@stream`, `@state`, `@op` and `@zk`. A user-supplied value is
  never verbatim: it is slugged (§1.4). Verbatim kinds registered by a
  profile are reserved for a later version, which will define their form.
  This version refuses them. `[F: keys.json]`
- **An interface id** is `<name>.v<major>`.
  - The name is one or more dot-separated segments `[a-z][a-z0-9_]*`, and
    its last segment is not `v` followed by digits.
  - `<major>` is a decimal integer from 0 to 2^32−1, without leading zeros.

  `[F: keys.json; contracts/e001-interface]`
- **An instance id** is 64 random bits written as 16 lowercase hex digits.
  `[F: keys.json]`
- **A fingerprint** (§9.5) is `sha256:` followed by 64 lowercase hex digits.
  In a contract key, only the 64 hex digits are written. In an interface
  token, `<fp16>` is the first 16 of them. `[F: keys.json]`
- **A member value** is a plain chunk: the slugged value of the epoch
  parameter. `<epoch>` is an instance id. `[F: keys.json]`
- **A ULID chunk** (events, §2.6) is 26 characters of Crockford's base32 in
  lowercase: digits, and lowercase letters except `i`, `l`, `o`, `u`.
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
  `[Sc: grammar.md §1]`

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
  capabilities and access-control prefix. It holds to about 5,000 devices
  per presence domain (§8.3). Above that, member tokens (§8.1) are the
  shape, at a third of the tokens.
- **The instance id is a continuity epoch.** An owner MUST mint a new
  instance id whenever its counters reset or its state's ordering restarts
  (§4.3): a process restart, a device re-enumerated, a polled device's
  uptime rewound, a container restarted.
  - It SHOULD do so in a new session, so that its timestamps' id changes
    too (§4.3). That change, and presence, are how a consumer sees the
    discontinuity.
  - A counter that crosses a face where neither is visible carries its
    epoch in its value: a profile's counter type does.

  `[Sc: presence.md §3]`
- **Moving a service to another host changes no key.**
- **No implicit identity.** Every key form names its system and service
  (§1.1), so a key cannot be built without them. A fleet-wide selection is
  spelled by name or wildcard, never defaulted from the local process.
  `[F: keys.json]`

### 1.6 Namespaces and base-relative keys

Keys are **base-relative**: they start at `zk2`. A deployment prefix, when
there is one, is the Zenoh session namespace. The session adds it on egress
and strips it on ingress. Application code SHOULD NOT spell a deployment
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
- **A literal chunk** is a plain chunk that does not start with `x-`, the
  slug prefix. Otherwise a literal could shadow a slugged value.
- **`{name}`** is one chunk holding a slugged value. A parameter name is
  `[a-z][a-z0-9_]*`, at most once per template.
- **`{name...}`** is a **rest** parameter: one or more chunks, each slugged.
  It MAY appear only as the last segment, and its value is a list of
  strings.
- **Parameter types** are `string`, `uint` and `path`.
  - `path` is the type of a rest parameter, and only of one.
  - A `uint` value is written in decimal without leading zeros, then slugged
    like any value.

`[F: contracts/e010-template, e010-literal-x, e011-params, e012-path]`

**Shapes and precedence.**
- **A template's shape** is its text with each parameter replaced by `{}`
  and each rest parameter by `{...}`. Literals are kept: `a/{x}` and `b/{y}`
  have different shapes.
- **Within one interface,** two templates under the same kind token MUST NOT
  have the same shape `[F: contracts/e021-shape]`. Templates under different
  kind tokens MAY, because their keys differ.
- **Resolution is match, then rank.** A template matches a key's resource
  chunks when every literal is equal, every parameter chunk decodes (§1.4),
  a rest parameter takes one or more chunks, and no chunk is left over.
  Among the templates that match, the winner is the one that ranks higher
  at the first segment where they differ (literal > `{p}` > `{p...}`). With
  an equal prefix, the longer template wins. `[F: templates.json]`
- **Overlapping templates** with different types are a warning.
  `[F: contracts/w101-overlap]`

**Cardinality.** A template with parameters MUST declare `cardinality`, a
positive integer that bounds its population in one instance. A template
without parameters MUST NOT declare it. An instance MAY declare a lower
bound in its descriptor (§3.3). On an event, `cardinality` bounds the
template's own parameters, and the key population is cardinality × rate ×
retention. `[F: contracts/e013-cardinality, e014-field]`

### 2.3 Resource attributes

| Attribute | Rule | Fixture |
|---|---|---|
| `optional` | An owner MAY declare an optional resource unavailable (§3.3). An owner MUST expose every required one, or not start (§8.2). | `[Sc: presence.md §2]` |
| `gate` | Only with `optional = true`. One or more of `build:<n>`, `config:<n>`, `capability:<n>`, where `<n>` is `[a-z0-9][a-z0-9_.-]*`. A list is an AND. | `[F: contracts/e016-gate-optional, e017-gate-syntax]` |
| `epoch` | Names one single-chunk parameter. The owner holds a member token per value of it, cycled on discontinuity (§8.1). | `[F: contracts/e022-epoch]` |
| `deprecated` | `{since, replaced_by, reason}`. `replaced_by` names another template of the contract, and `since` is not later than `minor`. A deprecated resource stays in the contract until the next major, because removing a resource is breaking (§9.8). | `[F: contracts/e031-deprecated; compat/contract/resource-removed]` |
| `attachment` | A type for the sample attachment, fingerprinted and encoded like a payload (§7). | `[F: contracts/ok-kinds]` |
| `annotations` | Keys are `<profile>.<key>`, and the profile is listed in `uses` (§10). | `[F: contracts/e020-annotation]` |

### 2.4 QoS

Each stream, state and event resource carries Zenoh QoS in its contract
(`[F: contracts/ok-kinds, ok-defaults-defaulted]`). An owner MUST publish
with it. The fields map one-to-one onto Zenoh's stable QoS.
`[Sc: types.md §3]`

| Field | Values | Default: `stream` | Default: `@stream` | Default: `state`, `@state`, `events` |
|---|---|---|---|---|
| `reliability` | `best_effort` \| `reliable` | best_effort | best_effort | reliable |
| `congestion` | `drop` \| `block` | drop | drop | block |
| `priority` | `real_time`, `interactive_high`, `interactive_low`, `data_high`, `data`, `data_low`, `background` | data | data_low | data |
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

zenoh-ext's advanced publication is behind its `unstable` feature in
1.10.1, so `history` is the one place where a participant opts into an
unstable API (§0). An advanced publisher declares its own queryable and
token under `<key>/@adv/…`, so its grants (§11.1) must include that
subtree.

### 2.6 Events

- **One key per occurrence.** An owner publishes each occurrence once, with
  a one-shot put, on `…/<iface>/events/<template>/<ulid>`. The last chunk is
  a lowercase ULID (§1.2) minted by the owner. A reader strips it before
  resolving the template. `[F: keys.json]`
- **`rate`** is `rare` (at most 1/h), `low` (at most 1/min) or `burst(<n>/h)`.
  **`retention`** is `<n>` followed by `s`, `m`, `h`, `d` or `w`. Both are
  required on an event. `[F: contracts/e015-event, e026-rate]`
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
| R1 | **Deployment:** a role MUST be bound by configuration, never in code. A binding is a list of service addresses, exact (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, meaning every service on `vehicle-01` that implements the interface). The binding configuration's format is a recommendation, not a rule. | `[Sc: bindings.md §1]` |
| R2 | **Deployment:** a template parameter MAY be bound too. `{vehicle} = self.system` (or `self.service`) binds the consumer to its own slice of a provider's collection. | `[Sc: bindings.md §2]` |
| R3 | **Owner:** the descriptor (§3.3) MUST list every requirement with its bindings and parameter bindings as configured. **Tool:** the data-flow graph is read from descriptors, never inferred. | `[Sc: bindings.md §3]` |
| R4 | A consumer compiled against `X.vN` binds to providers of any revision of `X.vN` (§9.8). | `[F: compat/]` |
| R5 | A consumer MAY wait on presence for its bound providers. A binding resolves at once without it. | `[Sc: bindings.md §1]` |
| R6 | **Consumer:** MUST discard a sample, or a GET reply, whose key expression is not concrete. Zenoh delivers a put on a wildcard key with the publisher's key. | `[Sc: bindings.md §4]` |
| R7 | A binding MUST NOT require presence. Across a constrained face, a consumer binds statically and judges liveness from the freshness of what crosses. Where nothing crosses, liveness is *unobservable*, and a tool MUST say so rather than report the provider down. | `[Sc: constrained.md §3]` |

When several providers are bound, choosing between them is the consumer's
(`arbitration.v1`).

### 3.3 The descriptor record

Every instance serves a **descriptor**: a JSON document answered on GET at
its instance key, and put on every change. Its schema is
[`descriptor.schema.json`](descriptor.schema.json), and a checker MUST report
exactly the `D…` codes that `conformance/descriptors/expect.json` lists for
each document, checked against the fixture contract. `[F: descriptors/]`

```json
{
  "format": "zk2-descriptor/0.1",
  "service": "vehicle-01/navigation",
  "instance": "3fa9c2d41b7e0012",
  "interfaces": [
    {"iface": "nav.v2", "contract": "sha256:…", "minor": 1,
     "unavailable": [{"resource": "state/covariance", "cause": "capability", "reason": "no IMU"}],
     "cardinality": {"state/tracks/{track}": 50}}
  ],
  "capabilities": ["imu", "gnss"],
  "requires": [
    {"role": "cmd", "interface": "twist_cmd.v1", "declared_by": "thruster.v1",
     "bindings": ["vehicle-01/safety", "vehicle-01/teleop"]},
    {"role": "plan", "interface": "mission_plan.v1", "declared_by": null,
     "bindings": ["ground/fleet-mgr"], "params": {"vehicle": "self.system"}}
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

  A listed resource that is not an optional resource of the contract is an
  error (D005). One that a missing capability already implies is a warning
  (D006). `[F: descriptors/d005-*, d006-implied]`
- **`cardinality`** MAY lower a template's bound for this instance, keyed
  `<kind token>/<template>`. It MUST NOT raise it. `[F: descriptors/d007-*]`
- **`requires`** lists each role with its bindings and parameter bindings
  (R2, R3). A role declared in a contract names that contract's interface in
  `declared_by`. A role declared by the component's manifest names `null`.
  `[F: descriptors/d009-*]`
- **Size.** A descriptor SHOULD stay within 1 KB. At the constrained level
  (§12), it MUST fit one fragment. `[Sc: constrained.md §5]`
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
| S2 | **Owner:** a GET reply MUST carry the timestamp of the mutation it represents. The owner's state queryables cover its interfaces' `state/**` and `@state/**`. It MUST answer a selector over them with every matching live key, plus a `reply_del` for each matching key it deleted within the window. | `[Sc: state.md §1, §2]` |
| S3 | **Owner:** for a key it deleted within the tombstone window, the owner MUST answer a GET with `reply_del` and the deletion's timestamp. The window is 60 s, unless the deployment configures the owner with another. A deployment MUST configure it, for state that an archive records, to at least the longest outage it expects between the archive and the owner. Across a constrained face, that is the face's maximum outage (§8.5). | `[Sc: state.md §2, §6]` |
| S4 | **Consumer:** a state GET MUST be addressed to the owner's keys, with target `All` and consolidation `Latest` set explicitly. **Deployment:** MUST NOT run a storage that answers on an owner's `state/**` or `@state/**` keys. | `[Sc: state.md §3]` |
| S5 | **Consumer:** last-known state MUST be read from an archive (§4.4), explicitly. **Owner of an archive:** §4.4. | `[Sc: state.md §4–§6, §9]` |
| S6 | **Consumer:** current state is the owner's answer. A consumer turns to an archive only when the owner gave no reply within the GET's timeout, or presence shows it absent. That silence is not a verdict about the key (O5). The archive's answer is last-known, never current, and a consumer MUST NOT present it as current. | `[Sc: state.md §4]` |
| S7 | **Owner:** clocks are bounded both ways (§4.3). | `[Sc: state.md §7]` |

A **tool** checks S4 against the routers' storage admin space. A consumer
cannot tell under `Latest` which replier answered.

### 4.3 Clocks (S7)

- **HLC.** A session that serves state MUST enable Zenoh's HLC. At the
  constrained level (§12), a wall clock with a per-session bump takes its
  place. `[Sc: state.md §1, §7]`
- **Minting.** zenoh 1.10.1 keeps `Session::hlc()` internal. An owner
  therefore mints each state timestamp as the greater of
  `Session::new_timestamp()` and the last timestamp it issued plus one
  tick, with its session's zid as the id. `[Sc: state.md §1]`
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
- **Ahead.** An owner SHOULD NOT stamp beyond its router's HLC delta: it
  cannot know its own offset without a reference.
  - It SHOULD detect drift against a reference it holds for the purpose,
    such as a subscription to a router-stamped heartbeat key, because its
    own puts are never echoed back.
  - When a detection shows it beyond the delta, it MUST stop writing
    state, and SHOULD report it (`health.v1` is the standard way).
  - Spike S12 showed the harm: a clock 2 s ahead produced a revision with
    two timestamps, and the next correctly clocked write looked stale.

### 4.4 Archives

An **archive** is a service implementing `archive.v1` (a profile, #613). This
section is what the core requires of it.

- **It records** owners' mutations under its own address, with each
  mutation's timestamp and type identity (§7.1), and keeps tombstones for at
  least the window (S3).
- **Its key** is the archived key without its leading `zk2` chunk, slugged
  as a rest parameter, under the archive's own `@state` token:
  `zk2/<system>/<archive>/archive.v1/@state/{origin...}`. For example,
  `zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01`.
  A verbatim chunk of the origin, such as `@state`, is slugged to
  `x-_x40state`. The archive's population is the sum of what it records,
  which a contract cannot fix in advance (`archive.v1` declares no
  ceiling).
- **It answers GETs only.** It never puts on its keys, because it is not a
  second publisher of the data.
- **Each reply's attachment** is a JSON object:
  `{"iface", "contract", "type", "confirmed"}`.
  - The first three are the archived value's type identity (§7.1).
  - The members are encoded as in a descriptor and the canonical form:
    `iface` is the interface id, `contract` the fingerprint, and `type`
    the type object of §9.5.
  - `confirmed` is `false` while alignment has not confirmed the key (below).
- **Its backend MUST NOT accept a put older than a delete it holds.** The
  storage manager of zenoh 1.10.1 accepts one, and resurrects the key, so it
  cannot be an archive's store as released.
- **Alignment.** When an owner becomes reachable again, the archive MUST
  re-read the owner's recorded collection before serving it as confirmed.
  While the owner stays absent, it re-reads that collection from an archive
  on the owner's side.
  - **It drops a key only on positive evidence:** a `reply_del` for it from
    the source.
  - **A key the source neither reports nor tombstones** is kept, and served
    with `confirmed: false`. An empty or partial reply set is not a verdict
    (O5): an access-control refusal, or a route that has not crossed yet,
    returns empty too.
  - **A raised window (S3) carries this rule.** With the window at least the
    longest expected outage, every delete the archive missed is still
    answered with `reply_del` after the heal.
  - **An unconfirmed key expires.** An outage longer than that can leave a
    key that alignment never confirms. The archive MAY drop such a key after
    a deployment-set horizon, and MUST keep serving it as unconfirmed until
    then.
- **Placement.** An archive on the consumer's side covers losing the link,
  and one on the owner's side covers losing the owner. Store-and-forward
  (`desired.v1`) uses both.

`[Sc: state.md §4–§6, §9]`

---

## 5. Operations

### 5.1 Rules

| # | Rule | Evidence |
|---|---|---|
| O1 | **Owner:** an operation's queryable MUST be declared on its concrete key (or its template), and MUST be `complete`. A concrete call with `BestMatching` then executes on at most one instance **while one instance serves the operation**. `BestMatching` reaches one `complete` queryable on each router, so a split-brain across routers runs a call on each side. Exclusivity beyond that is `redundancy.v1`'s. | `[Sc: operations.md §1]` |
| O2 | **Owner:** a call whose key expression is not concrete MUST be refused with `fanout_forbidden`, unless the operation declares `fanout = "allowed"`, whatever the access control allows. **Caller:** a call to a fan-out operation MUST use target `All` and consolidation `None`. | `[Sc: operations.md §2]` |
| O3 | **Owner:** a reply MUST go on the operation's own concrete key. Success is a value reply; failure is a `reply_err` carrying the error envelope (§5.2). The active instance of a service MUST answer a call to an optional operation it does not expose with `unavailable` and its cause. A standby declares no operation queryable (§6), so it cannot intercept calls. | `[Sc: operations.md §3]`; `[F: errors/cases.json]` |
| O4 | **Caller:** MUST NOT retry an operation that is not declared `idempotent`. | `[Sc: operations.md §4]` |
| O5 | **Caller and tool:** MUST NOT treat an empty reply set as a verdict. Access-control refusals return empty since zenoh 1.3. A tool attributes silence through presence. | `[Sc: operations.md §5]` |
| O6 | **Owner:** an operation declared `replies = "many"` gives zero or more value replies, then completion. With a declared `summary`, each replier MUST end with exactly one summary reply, whose attachment is the ASCII bytes `summary`; value replies carry none. **Caller:** MUST use consolidation `None`. `Latest` and `Auto` kept 1 reply of 10 in spike S6. | `[Sc: operations.md §6]`; `[F: contracts/e033-summary]` |
| O7 | **Caller:** a request MAY carry an attachment, the JSON object `{"actor", "request_id"}` (strings), which an owner MAY record for audit. It is claimed, never authentication. | `[Sc: operations.md §7]` |

Work that outlives a query timeout belongs to `jobs.v1`. A request type
SHOULD NOT repeat a template parameter. `[F: contracts/w102-repeat]`

### 5.2 The error envelope

A failed call replies with `reply_err` and an envelope with these fields:

| Field | Type | Meaning |
|---|---|---|
| `code` | string | One of `invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app` |
| `message` | string | For a human, never parsed |
| `cause` | string or null | With `unavailable` only, and then required: `build`, `config` or `capability` (§2.3) |
| `detail` | a value, bytes, or null | With `app` only: the operation's declared `error` type, as a value inline (JSON, CBOR) or as its encoded message (protobuf) |

**Encoding.** The envelope's kind follows the operation's `error` type when
it declares one, else its `response` type:
- **A JSON Schema type:** the envelope is encoded in the operation's
  `encoding`, JSON or CBOR.
- **A protobuf type:** the envelope is the message `zk2.core.v1.Error`.
- **A raw type:** the envelope is JSON.
- **The reply's `Encoding`** MUST say which:
  - `application/json` or `application/cbor`;
  - or `application/protobuf` with the schema suffix `zk2.core.v1.Error`.

The two definitions are [`core/error.proto`](core/error.proto) and
[`core/error.schema.json`](core/error.schema.json).

**Decoding.** A tool decodes an envelope by its reply's `Encoding`, without
the contract. It MUST refuse:
- an unknown encoding (`encoding`);
- malformed bytes, a duplicate or unknown member, or a missing `code` or
  `message` (`decode`);
- an unknown code, the empty string included, since a protobuf envelope
  without one decodes as `""` (`code`);
- `unavailable` without a valid cause, or a cause on any other code
  (`cause`);
- a detail on any code but `app` (`detail`).

A protobuf decoder ignores unknown fields, as protobuf does.
`[F: errors/cases.json]`

**Transport errors.** Zenoh reports some failures locally, such as a query
timeout, as a reply error with encoding `zenoh/string`. A tool tells the two
apart by encoding: only the three encodings above carry an envelope, and
any other reply error is the transport's.

---

## 6. Ownership and serving

- **Ownership.** Every key under `zk2/<system>/<service>/` is written only
  by that service: its puts, deletes, queryables and tokens.
  - An owner MUST NOT write under another service's prefix.
  - A deployment's access control MUST NOT grant it (§11.1).

  This is the invariant §11's grants rest on. `[Sc: security.md §1]`
- **Serving.** Every resource is `exclusive`: at most one instance at a time
  exposes it. An operation MAY declare `serving = "replicated"`, which lets
  any number of instances expose it, costs one execution per router per
  call, and requires `idempotent = true`. `[F: contracts/e018-replicated]`
- **No core roles.**
  - A standby instance exposes no exclusive resource, and declares no
    interface token for an interface it exposes nothing of.
  - Two instances of one service holding an interface token for the same
    interface, for longer than a grace period, is a **finding**. A tool
    diagnoses it, and the runtime does not fence.
    - The grace period is a tool setting. It SHOULD exceed the longest
      re-mint overlap the deployment allows (§8.1).
    - An owner SHOULD keep a re-mint's overlap below one second.

    `[Sc: operations.md §8]`
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
| `raw` | media type only | Images, audio, opaque buffers |

`flatbuffer`, `ros2msg` and `cdr` are reserved kinds. Authoring format
draft 1 has no spelling for them, so a contract declares such payloads as
`raw` with a media type, and tools MAY decode them through a plugin.
`[Sc: types.md §2]`

**Type references in a contract:**

| Spelling | Means |
|---|---|
| `"nav.v2.Position"` | a protobuf message, fully qualified, defined in a listed file. The well-known types of §9.4 are always available. |
| `"json:Status"` | `$defs/Status`. The name MUST be unique across the listed files. |
| `"json:telemetry#Point"` | `$defs/Point` in the listed file whose stem is `telemetry` |
| `{ raw = "image/jpeg" }` | opaque bytes of a media type |
| `{ raw = "video/*", media_param = "codec" }` | a raw family. The sample's `Encoding` carries the concrete subtype, and `media_param` names the single-chunk template parameter tied to it. |

A reference that does not resolve is an error. So is a `$ref` outside the
listed files.
`[F: contracts/e023-type, e024-ambiguous, e025-media-param, e029-schema, e032-ref-dangling, e032-ref-outside, ok-json-qualified, ok-protobuf-well-known]`

### 7.2 Encodings and rendering

- **The sample's `Encoding`.** An owner MUST set Zenoh's predefined
  `Encoding` for every sample whose type has one: protobuf, JSON, CBOR, or a
  predefined media type. It MUST NOT add a schema suffix to it, except on
  the error envelope (§5.2). A raw media type that Zenoh does not predefine
  travels as Zenoh's custom encoding, the media type in the suffix, as Zenoh
  does by itself. A suffix is sent with every put, and is capped at 255
  bytes. `[Sc: types.md §1]`
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
  - `type`, `properties`, `required`, `additionalProperties`, `items`,
    `prefixItems`;
  - `enum`, `const`;
  - `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`;
  - `minLength`, `maxLength`, `minItems`, `maxItems`;
  - `$ref`, local or to another listed file;
  - `oneOf` and `anyOf`.
- **Annotations, ignored:** `$schema`, `$id`, `$defs`, `$comment`, `title`,
  `description`, `default`, `examples`, `format`, `deprecated`, `readOnly`,
  `writeOnly`.
- **Refused:** every other keyword in a schema position (`pattern`,
  `patternProperties`, `allOf`, `not`, `if`/`then`/`else`, `contains`,
  `propertyNames`, `unevaluatedProperties`, …). Their validation differs
  between implementations (regular-expression dialects), or they defeat
  the compatibility rules. A contract whose schemas use one does not load
  (E037). A property *named* like a keyword is data, not a keyword.
  `[F: contracts/e037-subset]`

**`oneOf`, `anyOf` and `prefixItems`** are in the subset because Rust-first
contracts (schemars output) use them for enums, `Option<T>` and tuples.
Their containment is not decided in general. The classifier (§9.8) treats
any change inside one conservatively, as *review*.

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
  owner MUST hold one per interface **of which it currently exposes at least
  one resource**, and none otherwise (open item U22, §13). "Who implements `nav.v2`" is
  the liveliness selector `zk2/*/*/@zk/alive/nav.v2/**`.
  `[Sc: presence.md §1]`
- **Member token,** at `…/@zk/member/<iface>/<member>/<epoch>`, for the
  template that declares `epoch`. An interface has at most one such
  template, because the key carries no template
  (`[F: contracts/e022-two-epochs]`). The owner MUST hold one member token
  per member, and cycle it whenever that member's continuity breaks.
  `[Sc: presence.md §3]`
- **Re-minting is make-before-break.** An owner that starts a new instance
  id while running MUST declare the new tokens and descriptor first, then
  undeclare the old ones. There is an overlap, never a gap.
  `[Sc: presence.md §3]`
- **Reading presence.** A caller or tool's liveliness GET on a session that holds a
  liveliness subscriber MUST use a callback or an unbounded handler. With
  zenoh's default 256-slot handler, such a GET hung at every measured size
  from 996 tokens (zenoh#2678). `[Sc: presence.md §4]`

### 8.2 Start-up order

An owner MUST bring itself up in this order, so that alive ⇒ callable:
1. declare its resources (publishers and queryables);
2. validate that every required resource is exposed;
3. declare the descriptor's queryable, and a contract queryable (§8.4) for
   each interface it implements, holding that interface's bundle;
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

Contract bundles (§9.6) live at the location-free key
`zk2/@zk/contract/<iface>/<sha256>`. Every holder MUST declare a
`complete` queryable on it. An owner holds the bundles of the interfaces it
implements (§8.2), and any other participant MAY hold bundles too, because
the hash is the check. A caller or tool MUST retrieve a bundle as follows:

1. GET with target `BestMatching`. That reaches the nearest holder on each
   router the query visits.
2. Verify each reply **as it arrives** (§9.6), and accept the first valid
   one, without waiting for the GET to complete.
3. If none was valid, retry once with target `All`.
4. If still none, report the contract **unavailable**. Never accept an
   unverified bundle.

`[Sc: retrieval.md §1–§3]`

### 8.5 Constrained faces

A **face** is a router's link to a part of the network. A deployment marks
a face constrained, and gives its maximum outage, in its face configuration.
`link.v1` standardizes that configuration, but the core rules below need
only the two facts.
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
- **A batch SHOULD take at most a third of the lease to cross the face.**
  - At 2,400 bit/s with the 10 s lease, 1 KB batches qualify, and zenoh's
    64 KB default does not: the link reconnected in a loop.
  - A link's batch is the minimum of the configured size, the link's MTU and
    the other end's. Over TCP, the face therefore needs a router configured
    for it.

  `[Sc: constrained.md §4]`
- **`@stream` keys SHOULD be denied across the face** unless `link.v1`
  downsamples them. A named key's frames are queued, not dropped, and every
  request waits behind them.

---

## 9. Contracts and bundles

This section is precise enough to reproduce `conformance/contracts/`,
`sets/`, `bundles/`, `history/` and `compat/` without the reference
implementation. Where it and a fixture disagree, the fixture is right and
this text has a bug.

### 9.1 The authoring format

A contract is a **TOML 1.0** file, one per interface major, named
`<name>.v<major>.toml` (W107 otherwise; the fixtures do not check file
names). A reader MAY accept later TOML, but a contract MUST NOT need it.
The reference reader does not yet refuse TOML 1.1 syntax, so a 1.1-only
contract can load in Rust and fail in a 1.0 reader. A lint for this is
owed (#607 follow-up).
Its shape is [`contract.schema.json`](contract.schema.json) (JSON Schema
2020-12). An unknown table or field, a value of the wrong type, or text that
is not TOML is **E000**, reported once, and it stops the load.
`[F: contracts/e000-not-toml, e000-unknown-field]`

| Table | Fields |
|---|---|
| `[interface]` | `name`, `major` (an integer, 0 to 2^32−1), `minor` (W104 when absent), `summary`, `uses` (profile ids) |
| `[defaults]`, `[defaults.<kind>]` | the defaultable fields of Appendix D, plus `annotations`. `<kind>` is an authoring kind: `stream`, `state`, `event`, `operation`. |
| `[schemas]` | `protobuf` (files), `proto_include` (import roots), `jsonschema` (files) |
| `[resources."<template>"]` | `kind` (`stream`, `state`, `event`, `operation`), plus the fields of Appendix D |
| `[requires.<role>]` | `interface`, `resources`, `cardinality` (`one`, `many`), `optional`, `doc`, `annotations` |

**Field legality.**
- A field that its kind does not take, per Appendix D, is **E014**, once
  per field and resource.
- `history` is the exception. Outside a plain stream or state, it is
  **E019**.
- The same rules apply to `[defaults.<kind>]`, where `<kind>` decides.

`[F: contracts/e014-field, e019-history]`

**Required fields.** One **E015** per missing field:
- `type` on stream, state and event;
- `rate` and `retention` on event;
- `request` and `response` on operation.

`[F: contracts/e015-required]`

**Values.** Annotation values are any TOML value except a datetime (E020).
Floats are allowed. Every other value has the type
`contract.schema.json` gives it.

### 9.2 The lints

Diagnostics carry stable codes. `E…` makes a contract invalid; `W…` does
not. An implementation MUST report, for each fixture, exactly the codes
`contracts/expect.json` lists: sorted, with repeats. `[F: contracts/*]`

| Code | Condition | Reported |
|---|---|---|
| E000 | not TOML, or outside the format's shape | once; stops the load |
| E001 | the interface name is not one or more `.`-joined segments `[a-z][a-z0-9_]*`, or it ends in, or is, a segment `v<digits>` | once |
| E002 | a `uses` entry is not `<name>.v<major>` | per entry |
| E010 | a template is empty, has an empty segment, a literal that is not a plain chunk or starts with `x-`, a parameter name not `[a-z][a-z0-9_]*`, a repeated parameter, or a rest parameter that is not last | per resource; stops that resource's other checks |
| E011 | a template parameter missing from `params`, or a `params` entry the template lacks | per parameter |
| E012 | a `path` parameter that is not a rest parameter, or a rest parameter that is not `path` | per parameter |
| E013 | a template with parameters has no `cardinality`, or 0 | per resource |
| E014 | an illegal field (§9.1); `cardinality` without parameters | per field |
| E015 | a required field is missing (§9.1) | per field |
| E016 | `gate` without `optional = true` | per resource |
| E017 | a gate is not `build:`, `config:` or `capability:` followed by `[a-z0-9][a-z0-9_.-]*` | per gate |
| E018 | `serving = "replicated"` without `idempotent = true` | per resource |
| E019 | `history` on an explicit resource, an event or an operation | per occurrence, defaults included |
| E020 | an annotation key not `<profile>.<key>` (split at the **last** dot; the profile is a `uses` name without its major, the key `[a-z][a-z0-9_]*`), a profile not in `uses`, or a value holding a datetime | per key and table; once for `[defaults]`, whatever it reaches |
| E021 | two templates under one kind token with the same shape (§2.2) | once per template after the first of its shape |
| E022 | `epoch` does not name a single-chunk parameter of its template; or a second template of the interface declares `epoch` | per resource for the first condition; once per `epoch` template after the first, for the second |
| E023 | a type reference that does not resolve (§9.4), or a raw type that is not a media type | per reference |
| E024 | a `json:` name defined by several listed files, or two listed JSON Schema files with one stem | per reference or file |
| E025 | `media_param` does not name a single-chunk parameter of the template | per reference |
| E026 | `rate` not `rare`, `low` or `burst(<n>/h)` (n ≥ 1, decimal, no leading zero); `retention` not `<n>` + `s`/`m`/`h`/`d`/`w` (n ≥ 1) | per field |
| E027 | a string in the canonical form (§9.5) outside printable ASCII (0x20–0x7E), keys included | per value |
| E028 | an integer outside ±(2^53−1) in the canonical form, or in a JSON Schema artifact | per value; once per artifact |
| E029 | a schema file missing, unreadable, not JSON, or not compiling (protobuf editions included) | per file |
| E030 | a role not `[a-z][a-z0-9_]*`, an `interface` not `<name>.v<major>`, a `resources` entry that is not a template, or `resources = []` | per finding |
| E031 | `deprecated.replaced_by` names no other template of the interface, or `since` is greater than `minor` (checked only when `minor` is present) | per condition (both can fire on one resource) |
| E032 | a `$ref` with a scheme (`:`), outside the listed files, or to a missing definition | per `$ref` |
| E033 | `summary` without `replies = "many"` | per resource |
| E034 | `history.depth` is 0: on a resource where `history` is legal (where it is not, E019 alone); in `[defaults]`; in `[defaults.<kind>]`, where an illegal `history` gives both E019 and E034 | per occurrence |
| E035 | a requirement names a resource its interface does not declare, when that interface is in the set | per resource name (set check) |
| E036 | two contracts of a set declare one interface id | per duplicate (set check) |
| E037 | a JSON Schema keyword outside the subset (§7.3) | once per keyword and file |
| W101 | two resolved resources under one token whose templates overlap, with different types (the payload type, or an operation's request and response) | per pair |
| W102 | an operation's request type has a top-level field named like a template parameter | per resource |
| W103 | `encoding` or `attachment_encoding` written on a resource where no JSON Schema type takes it | per field (a defaulted one is silent) |
| W104 | `minor` is absent | once |
| W105 | an annotation key outside its profile's interim vocabulary (Appendix D) | per key |
| W107 | the file is not named `<name>.v<major>.toml` | once |

**Cascades.** A fixture's codes do not depend on the order lints run in,
because of these rules:
1. **E010** stops its resource's other checks. Two checks run over the raw
   resources table anyway: `deprecated.replaced_by` (E031), and the second
   `epoch` template (E022).
2. **E029** for one schema kind suppresses E023 for that kind.
3. **E021** and the `replaced_by` half of **E031** are checked over every
   template, whatever else is wrong with its resource.
4. **W101** is checked over resources without errors.
5. **E027** and **E028** on the canonical form run only when no other
   error was found, because there is no canonical form otherwise. E028 on a
   JSON Schema artifact runs at load.

E035 and E036 are set checks: each file is loaded on its own first.
`[F: sets/]`

### 9.3 Defaults and resolution

A field's value is taken from the first of these that sets it, field by
field:
1. the resource;
2. `[defaults.<kind>]`;
3. `[defaults]`;
4. the built-in default.

A default applies only where its field is legal (Appendix D), so
`[defaults] history = true` skips `@stream`, events and operations.
**Annotations** merge key by key, in the same precedence. **Requirements**
take no defaults.

| Field | Built-in default |
|---|---|
| `reliability`, `congestion`, `priority` | §2.4's table, by kind token |
| `optional`, `express` | `false` |
| `history` | none. `true` means `{depth = 1}`, and `false` means none. |
| `encoding` | `json` when the payload type is a JSON Schema type, or, for an operation, when any of its four types is. Otherwise there is no encoding. |
| `attachment_encoding` | `json` when the attachment is a JSON Schema type, otherwise none |
| `idempotent` | `false` |
| `fanout` | `forbidden` |
| `serving` | `exclusive` |
| `replies` | `one` |
| `timeout_ms` | none |
| `priority` of an operation | `interactive_high` |
| a requirement's `cardinality`, `optional`, `annotations` | `one`, `false`, `{}`; `resources` absent means "all" |

A defaulted contract and the same contract with every value spelled out
MUST have the same fingerprint.
`[F: contracts/ok-defaults-defaulted, ok-defaults-spelled]`

### 9.4 Schema artifacts and type references

**Protobuf.**
- **Import roots** are `proto_include`, relative to the contract.
  Otherwise the root is `proto/` when that directory exists, else the
  contract's own directory.
- **Each listed file** is compiled alone, with its imports and without
  source info, into a `FileDescriptorSet` holding the file and its imports
  in dependency order: `protoc --include_imports`, without
  `--include_source_info`.
- **The artifact** is those bytes. Its `name` is the file's path relative to
  its import root. A file using editions does not compile (E029).
- **A message reference** resolves to the one listed file that defines the
  message:
  - E023 when only an imported file defines it;
  - E023 when two listed files do.
- **The well-known types** are always available, each compiled on demand as
  its own artifact named `google/protobuf/<file>.proto`:
  - `any.proto`: `Any`;
  - `duration.proto`: `Duration`;
  - `empty.proto`: `Empty`;
  - `field_mask.proto`: `FieldMask`;
  - `struct.proto`: `Struct`, `Value`, `ListValue`;
  - `timestamp.proto`: `Timestamp`;
  - `wrappers.proto`: the nine wrappers.

  Other `google.protobuf` messages are not.

`[F: contracts/ok-protobuf-well-known, e023-type, e029-schema]`

**JSON Schema.**
- **The artifact** is the document. Its id hashes the document's JCS bytes,
  and its `name` is the file's stem, which MUST be unique among the listed
  files (E024).
- **`json:Name`** is `$defs/Name` in the one listed file that defines it
  (E023 when none does, E024 when several do).
- **`json:stem#Name`** looks only in the file with that stem.
- **A `$ref`'s file part** resolves as a path relative to the referencing
  file, lexically normalized, and MUST name a listed file. A `$ref` with a
  scheme (`:`) is refused. Its pointer MUST resolve. Each failure is E032.

`[F: contracts/e032-ref-dangling, e032-ref-outside, ok-json-qualified, e024-ambiguous]`

**Raw.** A media type is `<type>/<subtype>` or `<type>/*`. Each token is a
lowercase letter or digit followed by lowercase letters, digits or
`!#$&-^_.+` (E023 otherwise).

### 9.5 The canonical form and the fingerprint

The canonical form of a valid contract is this JSON object. Every member is
always present.

| Member | Value |
|---|---|
| `format` | `"zk2-contract/draft-1"` |
| `interface` | the interface id, `"<name>.v<major>"` |
| `uses` | the profile ids, deduplicated, sorted by (name as a string, major as a number) |
| `schemas` | one `{"id", "kind", "name"}` per artifact (§9.4), sorted by `id`. `kind` is `protobuf` or `jsonschema`. |
| `resources` | one object per resource, sorted by (kind token's spelling, template), bytewise: `@op` < `@state` < `@stream` < `events` < `state` < `stream` |
| `requires` | an object keyed by role |

**Every resource** has these members:

| Member | Value |
|---|---|
| `token` | the kind token (§1.3) |
| `template` | the template as written |
| `kind` | `stream`, `state`, `event` or `operation` |
| `params` | parameter name → `string`, `uint` or `path`; `{}` when none |
| `cardinality` | an integer, or `null` |
| `epoch` | a parameter name, or `null` |
| `optional` | a boolean |
| `gate` | the gates, sorted and deduplicated; `[]` when none |
| `deprecated` | `{"since", "replaced_by", "reason"}` (absent members `null`), or `null` |
| `annotations` | the merged annotations (§9.3); `{}` when none |

**A stream, state or event** adds:

| Member | Value |
|---|---|
| `type`, `attachment` | a type (below); `attachment` may be `null` |
| `encoding`, `attachment_encoding` | `json`, `cbor`, or `null` (§9.3) |
| `reliability`, `congestion`, `priority`, `express` | the resolved QoS (§9.3) |
| `history` | `{"depth", "miss_detection_ms"}` (`miss_detection_ms` may be `null`), or `null` |
| `rate` | `"rare"`, `"low"`, `"burst(<n>/h)"`, or `null` |
| `retention_s` | the retention in seconds (s×1, m×60, h×3600, d×86400, w×604800), or `null` |

**An operation** adds:

| Member | Value |
|---|---|
| `request`, `response` | a type |
| `error`, `summary` | a type, or `null` |
| `encoding` | `json`, `cbor`, or `null` |
| `idempotent`, `fanout`, `serving`, `replies`, `timeout_ms`, `priority` | resolved (§9.3); `timeout_ms` may be `null` |

**A type** is one of these:
- `{"kind": "protobuf", "name": <fully qualified message>, "schema": <artifact id>}`;
- `{"kind": "jsonschema", "name": <the $defs key, without any stem>, "schema": <artifact id>}`;
- `{"kind": "raw", "media_type": …, "media_param": <name or null>}`.

**A requirement** is `{"interface", "resources", "cardinality",
"optional", "annotations"}`. `resources` is the sorted, deduplicated list,
or `null` for "all".

**Excluded:** `minor`, the interface's `summary`, and every `doc`. An
operation's `summary` *type* is included, and so is `deprecated.reason`.

**Restrictions.** Every string, keys included, is printable ASCII
(0x20–0x7E), and every integer is within ±(2^53−1). A contract that breaks
either is invalid (E027, E028). On that domain every JCS implementation
agrees. `[F: contracts/e027-ascii, e028-integer]`

**Bytes and fingerprint.** The canonical bytes are the RFC 8785 (JCS)
serialization. The fingerprint is `sha256:` followed by the lowercase hex
sha256 of those bytes. An implementation MUST produce, for every valid
fixture, the bytes in `<stem>.canonical.json` and the fingerprint in
`expect.json`. `[F: contracts/*.canonical.json, expect.json]`

**A worked example.** `contracts/ok-minimal.toml` declares one resource,
`[resources.position]` with `kind = "stream"` and
`type = { raw = "text/plain" }`. Its canonical bytes are:

```json
{"format":"zk2-contract/draft-1","interface":"seed.v1","requires":{},"resources":[{"annotations":{},"attachment":null,"attachment_encoding":null,"cardinality":null,"congestion":"drop","deprecated":null,"encoding":null,"epoch":null,"express":false,"gate":[],"history":null,"kind":"stream","optional":false,"params":{},"priority":"data","rate":null,"reliability":"best_effort","retention_s":null,"template":"position","token":"stream","type":{"kind":"raw","media_param":null,"media_type":"text/plain"}}],"schemas":[],"uses":[]}
```

Its fingerprint is
`sha256:8c7493396b63865ef028879e04c641c76fd18178f4d6a6cac8c166f7b94c8577`.

**Portability.**
- Canonical bytes are portable for JSON Schema and raw types.
- For protobuf, they are portable only between implementations whose
  `FileDescriptorSet`s match byte for byte: `protox` 0.9 matches `protoc`
  3.21.12 with the flags above (spike S7). The protobuf fixtures assume
  that toolchain.
- **Verifying** a bundle (§9.6) is portable for every kind, because a
  bundle carries the bytes. Bundles are therefore built once, by the
  contract's CI, and only verified elsewhere.

### 9.6 Bundles

A bundle is the canonical contract with every schema artifact it lists, and
the extra documents it references:

```json
{"contract": <canonical contract>,
 "schemas": {"sha256:…": {"kind": "protobuf", "data": "<base64 FileDescriptorSet>"},
             "sha256:…": {"kind": "jsonschema", "data": <JSON Schema document>}},
 "extras": {"sha256:…": {"media_type": "application/json", "data": <JSON document>}}}
```

**Extras** are exactly the documents that the contract's `views.document`
annotations reference. That is the only extra-carrying annotation in this
version. The reference builder does not carry extras yet, so a contract
that uses `views.document` cannot be bundled by it until `views.v1` lands
(#613).

A bundle file is the JCS serialization of that object, so one contract
revision has one bundle. To verify bundle bytes, an implementation MUST
check the following in order, and refuse with the given tag at the first
failure. `[F: bundles/*, expect.json]`

| Step | Check | Tag |
|---|---|---|
| 1 | The bytes are UTF-8 | `shape` |
| 2 | They are JSON with no duplicate member | `json` |
| 3 | The top level is an object with no member besides `contract`, `schemas`, `extras` | `shape` |
| 4 | `contract` is present | `shape` |
| 5 | `contract.format` is `zk2-contract/draft-1` (a `contract` that is not an object fails here) | `format` |
| 6 | The contract keeps §9.5's restrictions | `restrictions` |
| 7 | `schemas` and `extras` are objects when present (absent means empty), and `contract.schemas` is a list of `{id, kind}` | `shape` |
| 8 | Every key of `schemas` is listed by the contract | `unlisted_schema` |
| 9 | For each listed schema, in id order: it is present (`missing_schema`); its `kind` is the listed one (`schema_kind`); it has a `data` member, base64 text for protobuf, and its kind is `protobuf` or `jsonschema` (`shape`); it hashes to its id, from the decoded bytes for protobuf and the JCS bytes for jsonschema (`schema_hash`) | as given |
| 10 | For each extra, in id order: it has `data` (`shape`), and the JCS bytes of `data` hash to its id (`extra_hash`) | as given |
| 11 | The extras' ids are exactly the `views.document` values of the contract's resources | `extras` |
| 12 | Where the caller expects a fingerprint (from a contract key or a descriptor), the contract's fingerprint equals it | `fingerprint` |

Integrity is Merkle-style. The fingerprint covers the canonical contract,
which lists every schema id and every extra's id (through its annotations).
Each artifact is checked against its id, so the whole bundle is verified.

### 9.7 History and retention

- **History.** A contract's CI keeps every published bundle at
  `contracts/.history/<iface>/<hex>.bundle.json`, where `<hex>` is the
  fingerprint without `sha256:`. The directory is append-only.
- **The history check** MUST verify:
  - every bundle (§9.6);
  - its fingerprint against its file name;
  - its interface against its directory;
  - that it is in JCS form.

  It reports `directory` (an entry that is not an interface directory),
  `file_name`, `io`, `interface`, `jcs`, or a bundle tag. `[F: history/]`
- **Retention.** A rebuild that the classifier judges identical to the
  newest published revision keeps that revision's bundle, and mints no
  fingerprint. For protobuf, identity compares the `FileDescriptorSet`s with
  source info dropped and every default `json_name` dropped. Fingerprints
  may over-detect a change, never under-detect one.
  `[F: compat/expect.json, same_revision]`

### 9.8 Compatibility

Revisions inside a major are checked **FULL_TRANSITIVE**: a candidate
against every revision in the history, in both directions. Each rule is
judged per direction:
- for streams, state, events and responses, the owner writes and the
  consumer reads;
- for requests, the caller writes and the owner reads.

A change is **compatible**, **review** or **breaking**. The candidate's
class is the worst over both directions and every earlier revision. A
revision that does not load at all is **invalid**, like the
`add-pattern` case.
- **Breaking:** a contract's CI MUST NOT publish the revision.
- **Review:** a human MUST accept it before publication.

The classifier MUST classify every case of `compat/` as `expect.json` says,
including the transitive cases, where a revision compatible with its
predecessor breaks against an earlier one. `[F: compat/]`

**Contract metadata:**

| Change | Class |
|---|---|
| A resource removed | breaking (deprecated resources are kept until the next major) |
| `idempotent` true → false | breaking |
| `fanout` allowed → forbidden | breaking |
| `reliability` reliable → best_effort | review |
| optional → required | breaking |
| `priority` changed, `express` toggled | review |
| `congestion` drop → block | review (it can stall the producer) |
| `explicit` false → true | breaking for ambient consumers |
| `explicit` true → false | review (link budgets) |
| `replies` one → many | breaking (callers' consolidation) |
| A required role added | breaking |
| An optional role added | compatible |
| A role's interface or cardinality changed | breaking |
| Documentation and `minor` only | compatible |

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
- **Warning** `field_deleted_unreserved` (reported alongside the class): a
  field deleted without
  reserving its number. Reuse is caught against the whole history.

**JSON Schema payloads:** the subset of §7.3. Readers tolerate unknown
properties, and writers send only what their schema declares.
- **Compatible:** an optional property added, even to a closed schema, or
  removed.
- **Breaking:**
  - a required property added;
  - optional → required, or required → optional;
  - integer ↔ number;
  - an enum value added or removed;
  - a bound changed: a tightened `maximum` breaks old writers, and a
    loosened `maxLength` breaks old readers;
  - a `oneOf` branch added.
- **Review:** any other change inside `oneOf`, `anyOf` or `prefixItems`.
  Their containment is not decided.

---

## 10. Extension points for profiles

A **profile** is an independently versioned specification, such as
`timing.v1` or `archive.v1`. The core never depends on one. Where a rule
here names a profile (`health.v1` for reporting, `link.v1` for face
configuration, `archive.v1` for archives), it names the standard way to
meet the rule, which the core states in its own terms. A profile
contributes through exactly four points:
1. **A standard contract**: an interface it defines.
2. **An annotation vocabulary.** Keys are `<profile>.<key>`. A contract
   that uses one MUST list the profile in `uses`, which is fingerprinted.
   `[F: contracts/e020-annotation]` Until a profile publishes its
   vocabulary, the interim tables of Appendix D apply, and a key outside
   them is a warning. `[F: contracts/w105-vocabulary]`
3. **A registered verbatim kind**, such as `@blob`, at position 5. This
   point is reserved: this version refuses such keys (§1.2), and a later
   version defines their form.
4. **The descriptor's `profiles` list.**

---

## 11. Security

### 11.1 Grant shapes

Ownership (§6) reduces access control to three grant shapes:

| Grant | Rule |
|---|---|
| **Own** | A service principal puts, deletes, declares queryables and declares tokens under `zk2/<system>/<service>/**`. It also holds each verbatim subtree, spelled out because `**` never crosses one: `…/*/@stream/**`, `…/*/@state/**`, `…/*/@op/**`, `…/@zk/**`. A service using advanced publication (§2.5) also holds `…/*/stream/**/@adv/**` and `…/*/state/**/@adv/**`. |
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name, plus their `@adv` subtrees where the consumer uses history. |
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
| Descriptor (§3.3) | MUST fit one fragment (4 KB with zenoh-pico's default `Z_FRAG_MAX_SIZE`) `[Sc: constrained.md §5]` |
| Bundles (§8.4) | Served by a gateway holder where the device cannot receive or hold them. A zenoh-pico 1.10.1 owner sent 100 KB replies, so the limit is on receiving. |
| Faces (§8.5) | Attached as a client, with batches within the lease, `@zk` denied, and `@stream` denied unless downsampled. Presence never crosses. |

`[Sc: constrained.md]`

---

## 13. Open items

These are the maintainer's to decide, at the acceptance of this version or
later. Each changes a rule above; until it is decided, the rule stands as
written.

| # | Question | Lean | Rule affected |
|---|---|---|---|
| U22 | A deployment above the presence budget (§8.3): how does it cut the per-service token multiplier (1 instance token + one per interface)? | An interface every service implements (`health.v1`, the framework set) declares no interface token, and its providers are found through instance tokens and descriptors. The cost is that "who implements X" for those interfaces needs descriptors, and the split-brain check (§6) narrows. | §8.1 interface tokens |
| U23 | The far side of a constrained face when it is more than one session: a site with its own router, or a service commanding many vehicles | Measure zenoh 1.10.1's `gateway.south` regions, which place a far router south by zid, interface or region name, so that declarations reach it only on interest | §8.5 attachment |

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
| Deprecate, never reuse, and keep the ledger append-only. | §2.3, §9.7 |
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
| `conformance/contracts/` | §2, §7, §9.1–§9.5, §10 |
| `conformance/sets/` | §9.2 (E035, E036) |
| `conformance/bundles/` | §9.6 |
| `conformance/history/` | §9.7 |
| `conformance/compat/` | §9.7 retention, §9.8, R4 |
| `conformance/descriptors/` | §3.3 |
| `conformance/errors/` | §5.2 |
| `scenarios/grammar.md` | §1.3, §1.6 |
| `scenarios/state.md` | §4 |
| `scenarios/operations.md` | §5, §6 |
| `scenarios/presence.md` | §1.5, §3.3, §8.1–§8.2 |
| `scenarios/bindings.md` | §3.2 |
| `scenarios/retrieval.md` | §8.4 |
| `scenarios/types.md` | §2.4, §7.1–§7.2 |
| `scenarios/security.md` | §11 |
| `scenarios/constrained.md` | §1.6, R7, §8.5, §12 |

The compatibility cases (`compat/`) are evaluated by the classifier (#618).
Until it lands, the reference runner checks that every input loads.

## Appendix D. Authoring fields by kind

Normative for §9.1. A field outside its kind's column is E014 (`history`:
E019).

| Field | stream | state | event | operation | Defaultable |
|---|---|---|---|---|---|
| `kind`, `doc`, `params`, `cardinality`, `epoch`, `optional`, `gate`, `deprecated`, `annotations` | ✓ | ✓ | ✓ | ✓ | `annotations` only |
| `type` | required | required | required | — | no |
| `attachment` | ✓ | ✓ | ✓ | — | no |
| `encoding` | ✓ | ✓ | ✓ | ✓ | yes |
| `attachment_encoding` | ✓ | ✓ | ✓ | — | yes |
| `explicit` | ✓ | ✓ | — | — | no |
| `reliability`, `congestion`, `priority`, `express` | ✓ | ✓ | ✓ | `priority` only | yes |
| `history` | plain only | plain only | — (E019) | — (E019) | yes |
| `rate`, `retention` | — | — | required | — | no |
| `request`, `response` | — | — | — | required | no |
| `error`, `summary`, `idempotent`, `fanout`, `serving`, `replies`, `timeout_ms` | — | — | — | ✓ | yes, except `error` and `summary` |

**Interim annotation vocabularies** (W105 outside them, until each profile
publishes its own):

| Profile | Keys |
|---|---|
| `freshness` | `ttl_s` |
| `timing` | `period_ms`, `deadline_ms`, `lifespan_ms` |
| `telemetry` | `unit`, `kind`, `buckets`, `semantic` |
| `link` | `exposure`, `downsample_ms` |
| `arbitration` | `policy` |
| `desired` | `target_param`, `target` |
| `alarms` | `severity_default` |
| `media` | `tiers`, `tier_param`, `frame_clock`, `control`, `receiver_report` |
| `views` | `document` |
| `redundancy` | `election` |

## Appendix E. Fixture formats

Each fixture file states its own format in its `description` member. In
brief:
- **`keys.json`:** each case is a key and its parse, or `null` when the key
  is refused.
  - **A data key** parses to `{form: "data", system, service, iface, token,
    resource: [chunks]}`.
  - **A control key** parses to `{form: "instance" | "alive" | "member" |
    "contract", …}`, with the fields its form names: `instance`, `iface`,
    `fp16`, `member`, `epoch`, `sha256`.
  - Every accepted key builds back to the same string.
- **`slugs.json`:** `slug` cases (value → chunk) and `unslug` cases (chunk →
  value, or `null`).
- **`templates.json`:** templates and resource chunks → the winning
  template's text and its bindings, each a list of strings (one element for
  a single parameter), or `null`.
- **`contracts/`, `descriptors/`, `sets/`:** expected codes, sorted. For
  valid contracts, also the fingerprint and `<stem>.canonical.json`.
- **`bundles/`, `history/`, `errors/`:** expected tags, or the decoded
  value.
- **`compat/`:** the class, warnings and `same_revision`.
