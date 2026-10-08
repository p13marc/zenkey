# zk2 core specification

**Version 0.8** (0.1 accepted on 2026-10-08, #606; amended the same day:
U23 in 0.2, the classifier's rule set in 0.3, TOML 1.0 enforced in 0.4, the
second implementation's findings in 0.5, its findings against 0.5 and the
archive's gaps in 0.6, in 0.7 the findings of its live half, the
operations runtime's decisions and the codegen's gaps, and in 0.8 what
implementing 0.7 found, a refused presence read first).
Every change goes through [`CHANGELOG.md`](CHANGELOG.md), amendment-style.

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

A key is parsed without a contract, so parsing applies the lexical rules
alone. A resource chunk is any plain chunk (§1.2), `x-eth0` included,
although no template could produce that one: which template a key belongs
to, if any, is resolution's question (§2.2). `[F: keys.json]`

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
  lowercase: digits, and lowercase letters except `i`, `l`, `o`, `u`. The
  check is lexical and the chunk is never decoded, so the first character
  has no bound: `8zzzzzzzzzzzzzzzzzzzzzzzzz`, beyond 128 bits, is a ULID
  chunk. `[F: keys.json]`

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
  - Parameter types play no part in matching: a `uint` parameter matches
    `03` or `abc` as it matches any canonical slug. A consumer that needs
    the typed value parses the bound value after resolution.
  - Two templates that rank alike at every segment and have one length
    have one shape, which a contract refuses (E021). Given such a list, a
    resolver takes the first listed. `[F: templates.json]`
- **Overlapping templates** with different types are a warning.
  `[F: contracts/w101-overlap]`

**Cardinality.** A template with parameters MUST declare `cardinality`, a
positive integer that bounds its population in one instance. A template
without parameters MUST NOT declare it. An instance MAY declare a lower
bound in its descriptor, from 1 to the contract's (§3.3). On an event,
`cardinality` bounds the template's own parameters, and the key population
is cardinality × rate × retention. `[F: contracts/e013-cardinality, e014-field]`
- **No ceiling.** A template whose population no contract can fix, such as
  an archive's `{origin...}` (§4.4), still declares a cardinality. By
  convention it declares **4294967295** (2^32−1), which reads "no ceiling".
  The value is a positive integer like any other, so E013 holds and the
  authoring format needs no second spelling for it. A tool reads it as no
  bound, never as a population to budget or estimate with. An instance
  that knows its population MAY still lower it in its descriptor (§3.3).
  `[F: contracts/ok-cardinality-no-ceiling]`

### 2.3 Resource attributes

| Attribute | Rule | Fixture |
|---|---|---|
| `optional` | An owner MAY declare an optional resource unavailable (§3.3). An owner MUST expose every required one, or not start (§8.2). | `[Sc: presence.md §2]` |
| `gate` | Only with `optional = true`. One or more of `build:<n>`, `config:<n>`, `capability:<n>`, where `<n>` is `[a-z0-9][a-z0-9_.-]*`. A list is an AND. An empty list is no gate, so it needs no `optional`. | `[F: contracts/e016-gate-optional, e017-gate-syntax, ok-gate-empty]` |
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
  - In a rate, `<n>` is decimal without a leading zero, from 1 to 2^32−1:
    the canonical form keeps the rate's text, so `burst(012/h)` would be a
    second spelling.
  - In a retention, `<n>` is decimal digits, leading zeros allowed, and at
    least 1: the canonical form keeps seconds, so `07d` is `7d`. The
    seconds, `<n>` times the unit's, MUST fit an **unsigned** 64-bit
    integer, at most 2^64−1 (E026). Like every canonical integer, they
    MUST also be within ±(2^53−1) (E028), so 2^63 s is E028 alone.
    `[F: contracts/ok-retention-leading-zero, e026-retention-range, e028-retention]`
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

- **An unbound required role.** An owner whose configuration binds a
  required role to nothing MUST NOT start, as one missing a required
  resource does not (§8.2 step 2): no instance token appears. This refuses a
  missing configuration, not a missing provider: a bound role still
  resolves at once, without presence (R5, R7). `[Sc: presence.md §2]`
  - **Observing it.** A refusal is the absence of a token, and silence is
    not a verdict (O5). A tester therefore watches through a router that
    stays up whatever the owner does, never one the owner's own process
    runs, and checks it against a control: the same owner, configured to
    start, shows its token there within the wait (§8.1).
- **An unbound optional role** is listed in the descriptor all the same,
  with `"bindings": []` and `"params": {}` (R3): the graph keeps every edge
  a contract declares, bound or not. `[Sc: bindings.md §3]`

### 3.3 The descriptor record

Every instance serves a **descriptor**: a JSON document answered on GET at
its instance key, and put on every change. Its schema is
[`descriptor.schema.json`](descriptor.schema.json), and a checker MUST report
exactly the `D…` codes that `conformance/descriptors/expect.json` lists for
each document, checked against the fixture contract. `[F: descriptors/]`

**The GET.** One `complete` queryable on the instance key answers with one
reply: the current descriptor's bytes, with `Encoding` `application/json`,
no attachment and no timestamp (S1–S2 bind state, and a descriptor is not
state). Each put of it (below) carries the owner's timestamp. A caller GETs
with consolidation `None`, as §8.4 does, takes the first reply, and waits
for it as long as it chooses (§8.1). `[Sc: presence.md §2]`

```json
{
  "format": "zk2-descriptor/0.1",
  "service": "vehicle-01/navigation",
  "instance": "3fa9c2d41b7e0012",
  "interfaces": [
    {"iface": "nav.v2", "contract": "sha256:…", "minor": 1, "token": true,
     "unavailable": [{"resource": "state/covariance", "cause": "config", "reason": "calibrating"}],
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

In the example, the instance holds `imu`, so `state/covariance` (gated on
`capability:imu`) is exposed unless listed; it is listed, for a cause the
gate does not name.

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
- **`token`** is `false` for an interface in the owner's tokenless set
  (§8.1), and `true` otherwise, which is the default.
- **`cardinality`** MAY lower a template's bound for this instance, keyed
  `<kind token>/<template>`, to a value from 1 to the contract's. It MUST
  NOT raise it. `[F: descriptors/d007-*]`
- **Integers.** As in a contract (§9.1), the schema's `format` is a bound,
  not an annotation. Here `uint64` is 0 to 2^64−1, a JSON number with no
  TOML limit, for `minor` and every `cardinality` value. A value outside it
  is not the record's shape (D000), so a `cardinality` of 2^64 is D000, and
  one of 2^64−1 above the contract's bound is D007.
  `[F: descriptors/d000-cardinality-range, d007-cardinality-max]`
- **`requires`** lists each role with its bindings and parameter bindings
  (R2, R3). A role declared in a contract names that contract's interface in
  `declared_by`. A role declared by the component's manifest names `null`.
  A role the configuration leaves unbound is listed with `"bindings": []`
  (§3.2). `[F: descriptors/d009-*]`
- **`profiles`** is the union of the `uses` of the contracts the instance
  implements, sorted and deduplicated (§10 point 4).
- **Size.** A descriptor SHOULD stay within 1 KB. At the constrained level
  (§12), it MUST fit one fragment. `[Sc: constrained.md §5]`
- **Updates.** The owner MUST put the descriptor on its instance key whenever
  it changes, and MUST answer a GET there with the current one.
  `[Sc: presence.md §2]`
  - **The first descriptor is put too.** The owner puts it when it
    declares the descriptor's queryable, in §8.2 step 3, and so before any
    token. A subscriber to instance keys that is already up, such as
    `presence.md §1`'s tool, receives each descriptor an instance puts, its
    first included. A re-mint puts the new instance's first descriptor the
    same way, before the new tokens (§8.1). `[Sc: presence.md §1]`

**The checks.** A checker reads a descriptor with the contracts it is given,
and reports these codes. `[F: descriptors/]`

| Code | Condition | Severity | Reported |
|---|---|---|---|
| D000 | not strict JSON (a duplicate member included), or outside [`descriptor.schema.json`](descriptor.schema.json): an unknown or missing member, a value of the wrong type (an integer written with a fraction or an exponent included), an integer outside its `format` (`uint64`: 0 to 2^64−1), a `cause` not `build`, `config` or `capability` | error | once; stops the check |
| D001 | `format` is not `zk2-descriptor/0.1` | error | once |
| D002 | `service` is not `<system>/<service>`, two plain chunks; `instance` is not 16 lowercase hex digits | error | per member |
| D003 | an interface entry's `iface` is not an interface id; its `contract` is not a fingerprint (§1.2); or the interface is listed twice | error | per entry and condition |
| D004 | the entry's interface is among the given contracts, but not at its fingerprint | error | per entry |
| D005 | `unavailable` names a resource the contract does not declare, or a required one | error | per entry |
| D006 | `unavailable` names a resource that a capability not held already implies | warning | per entry |
| D007 | a `cardinality` key names no templated resource of the contract, or its bound is not from 1 to the contract's | error | per key |
| D008 | a capability is not `[a-z0-9][a-z0-9_.-]*`; or one is listed twice | error | once per malformed occurrence; once more for the list when any value repeats, however many do (below) |
| D009 | in a requirement entry: `role` is not `[a-z][a-z0-9_]*`; `interface` is not an interface id; `declared_by` is not one of the interfaces this descriptor lists; a `params` key is not `[a-z][a-z0-9_]*`, or its value is empty; a binding is not `<system>/<service>`, each chunk plain or `*` | error | per finding |
| D010 | a profile is not `<name>.v<major>`; or one is listed twice | error | as D008: once per malformed occurrence, once for the repeats |

**Cascades and scope.**
1. D000 stops the check: no other code is reported.
2. An interface entry whose `iface` is not an interface id (D003) is
   checked no further, and is not one of the descriptor's interfaces: a role
   `declared_by` it is D009. `[F: descriptors/d003-iface]`
3. An entry whose `contract` is not a fingerprint (D003) is checked no
   further: no D004. Its interface still counts for `declared_by`.
   `[F: descriptors/d003-fingerprint]`
4. An interface listed twice (D003) is otherwise checked like the first.
   A repeated capability or profile is counted once for the whole list,
   whichever values repeat and however often: `["a", "a", "b", "b"]` gives
   one D008. A malformed value is counted at each occurrence as well, so
   `["A", "A"]` gives three. `[F: descriptors/d008-two-repeated-values,
   d008-malformed-twice, d010-two-repeated-values, d010-malformed-twice]`
5. An entry is checked against the given contract with its interface and
   fingerprint (D005–D007). An interface none of the given contracts
   declares is checked for syntax only; one given at other fingerprints
   only is D004. `[F: descriptors/ok-unknown-interface, d004-revision]`
6. Not checked, deliberately `[F: descriptors/ok-unchecked]`:
   - that a `cause` agrees with the resource's gates: an owner may lack a
     resource for a reason its gates do not name;
   - R3's completeness, that every requirement a contract declares is
     listed: a scenario checks it (`bindings.md §3`);
   - that a role `declared_by` an interface is in that contract's
     `[requires]`, and that `params` values fit the required interface;
   - that `profiles` is the union of the contracts' `uses`;
   - `minor`, an integer from 0 to 2^64−1 that nothing reads, and `token`.

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

**Observing S1.** An owner's stamp is told from a router's by its id: the
owner's session's zid, against the router's (§4.1). Where the owner's
session is itself the router, the two zids are one, and the check proves
nothing. A tester therefore runs the owner as a client of a router with
timestamping enabled, which keeps the stamp a put carries unless it is
future-dated (§4.1), and checks the check against a control: a put without
a timestamp, through the same router, arrives with the router's zid.
`[Sc: state.md §1]`

### 4.3 Clocks (S7)

- **HLC.** A session that serves state MUST enable Zenoh's HLC. At the
  constrained level (§12), a wall clock with a per-session bump takes its
  place. `[Sc: state.md §1, §7]`
- **Minting.** zenoh 1.10.1 keeps `Session::hlc()` internal. An owner
  therefore mints each state timestamp as the greater of
  `Session::new_timestamp()` and the last timestamp it issued plus one
  tick, with its session's zid as the id. `[Sc: state.md §1]`
  - **A tick** is the smallest step the owner's timestamp type can take
    above the last. Timestamps are compared by their time, an NTP64 value
    (Appendix B), whose unit is 2^−32 s: the reference adds one unit. A
    larger step keeps the rule, which needs only that each stamp exceed the
    last: zenoh-python builds an NTP64 from seconds and nanoseconds, so its
    smallest step is 1 ns.
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
  which a contract cannot fix in advance, so its `{origin...}` template
  declares the no-ceiling cardinality, 2^32−1 (§2.2).
- **A pattern over its keys** is formed from an origin selector the same
  way, chunk by chunk: the leading `zk2` is dropped, `*` and `**` stay as
  written, and every other chunk is slugged (§1.4). The archive form of
  `zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*` at `ground/archive`
  is `zk2/ground/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/*`.
  - Only whole-chunk wildcards carry over: a selector with a chunk that
    holds `$*` has no archive form.
  - The form can select more than the selector does. A slugged verbatim
    chunk is plain, so a wildcard in the archive form matches it, although
    the same wildcard in the selector never matches the verbatim chunk
    (§1.3): `zk2/g/s/i.v1/*/plans/*` does not select `…/@state/plans/a`,
    and its archive form selects `…/x-_x40state/plans/a`. A reply read
    through the form counts only when the selector selects its decoded
    origin.
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
  on the owner's side, through the collection's archive form (above).
  - **Reachable** is judged by presence: the owner's instance token
    (§8.1). With the owner gone, a peer archive's token appearing is the
    sign that the link healed, so an archive aligns then too. It reads the
    peer archives it is configured with in turn, and aligns from the first
    whose reply set holds a reply that counts.
  - **It drops a key only on positive evidence:** a `reply_del` for it from
    the source.
  - **A key the source neither reports nor tombstones** is kept, and served
    with `confirmed: false`. An empty or partial reply set is not a verdict
    (O5): an access-control refusal, or a route that has not crossed yet,
    returns empty too.
  - **Retrying.** An archive SHOULD repeat an alignment that left keys
    unconfirmed, a bounded number of times. An owner's instance token can
    reach the archive before the route to the owner's state queryable has,
    and a read made then returns empty, so it confirms nothing. A source
    still silent after the last attempt leaves its keys unconfirmed, which
    is what they are. The reference makes 5 attempts, the n-th retry
    200 ms × n after the previous attempt ends. `[Sc: state.md §5]`
  - **What a reply confirms.** A value read from the owner is confirmed.
    A value read from a peer archive keeps the peer's confirmation: it is
    confirmed when the peer's attachment says `"confirmed": true`, and
    unconfirmed otherwise, an absent or unreadable attachment included. A
    `reply_del` from a peer is positive evidence, as one from the owner is:
    the peer holds the owner's delete, with its timestamp.
    `[Sc: state.md §5]`
  - **Older never wins.** A reply older than what the archive holds changes
    nothing, as a put older than a held delete does not (above). One at the
    same timestamp can confirm the key, never unconfirm it.
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
| O1 | **Owner:** an operation's queryable MUST be declared on its concrete key (or its template), and MUST be `complete`. A concrete call with `BestMatching` then executes on at most one instance **while one instance serves the operation**. `BestMatching` reaches one `complete` queryable on each router, so a split-brain across routers runs a call on each side. Exclusivity beyond that is `redundancy.v1`'s. **Caller:** a concrete call MUST use target `BestMatching` and consolidation `None`, both set explicitly. | `[Sc: operations.md §1]` |
| O2 | **Owner:** a call whose key expression is not concrete MUST be refused with `fanout_forbidden`, unless the operation declares `fanout = "allowed"`, whatever the access control allows. **Caller:** a call to a fan-out operation MUST use target `All` and consolidation `None`. | `[Sc: operations.md §2]` |
| O3 | **Owner:** a reply MUST go on the operation's own concrete key, a member's for a call over a template (below). Success is a value reply; failure is a `reply_err` carrying the error envelope (§5.2). Every call that reaches an owner's queryable MUST get one or the other, never silence (below). The active instance of a service MUST answer a call to an optional operation it does not expose with `unavailable` and its cause. A standby declares no operation queryable (§6), so it cannot intercept calls. | `[Sc: operations.md §3]`; `[F: errors/cases.json]` |
| O4 | **Caller:** MUST NOT retry an operation that is not declared `idempotent`, and retries one that is only after silence (below). | `[Sc: operations.md §4]` |
| O5 | **Caller and tool:** MUST NOT treat an empty reply set as a verdict. Access-control refusals return empty since zenoh 1.3. A tool attributes silence through presence, which a refusal can empty too (§8.1, 0.8). | `[Sc: operations.md §5]` |
| O6 | **Owner:** an operation declared `replies = "many"` gives zero or more value replies, then completion. With a declared `summary`, each replier MUST end with exactly one summary reply, whose attachment is the ASCII bytes `summary`; value replies carry none. **Caller:** MUST use consolidation `None`. `Latest` and `Auto` kept 1 reply of 10 in spike S6. A replier without exactly one summary is possibly partial (below). | `[Sc: operations.md §6]`; `[F: contracts/e033-summary]` |
| O7 | **Caller:** a request MAY carry an attachment, the JSON object `{"actor", "request_id"}` (strings), which an owner MAY record for audit. It is claimed, never authentication, and never a reason to refuse a call (below). | `[Sc: operations.md §7]` |

Work that outlives a query timeout belongs to `jobs.v1`. A request type
SHOULD NOT repeat a template parameter. `[F: contracts/w102-repeat]`

**Calling.**
- **A concrete call (O1).** `BestMatching` reaches one `complete`
  queryable on each router, and consolidation `None` delivers each reply as
  it arrives, as on every operation call (O2, O6). zenoh's default on a
  concrete key is `Latest` (§4.1): it holds the reply until the query
  completes, and keeps one reply per key, so the second execution of a
  split-brain would arrive unseen. A one-reply caller takes the first value
  or envelope on the call's key, without waiting for the query to
  complete. `[Sc: operations.md §1]`
- **The timeout** is the caller's (§8.1). A caller that sets none waits
  the operation's `timeout_ms`, the contract's recommendation, and with
  neither, 10 s, zenoh's default query timeout (Appendix B). A call that
  ends there with no value and no envelope is silent (O5).
- **Retries (O4)** are opt-in: a caller makes one attempt unless it is
  configured for more, and calls an operation that is not `idempotent` once,
  whatever the configuration.
  - A retry follows silence only: no value and no envelope before the
    timeout, the transport's own error reply included (§5.2).
  - An envelope is an answer, `busy` included, and so is a malformed one.
    Calling again after `busy` is a new call, which the application
    decides, never a retry.
  - The reference retries no fan-out: every service that answered would
    run it again.

  `[Sc: operations.md §4]`

**Answering.**
- **Every call is answered (O3).** A call that reaches an owner's
  queryable gets a value or an envelope. A handler that ends without
  replying is answered `internal`, so a live server's own bug is never
  silence (O5). With `replies = "many"`, a declared `summary` is owed too:
  a handler that ends without it is answered `internal`, after any values
  it sent (O6). Without one, zero values then completion is the
  operation's own answer, which a caller cannot tell from silence.
  `[Sc: operations.md §3, §6]`
- **A key that names no member.** A concrete call whose resource chunks
  resolve to no member of the operation's template, because a parameter
  chunk is not a canonical slug (§1.4, §2.2), is refused with
  `invalid_request`: the key is malformed. `not_found` is for a well-formed
  member the owner does not have. `[Sc: operations.md §3]`
- **The request.** A request that does not decode as the operation's
  `request` type, by its `Encoding` and then the contract's (§7.2), is
  refused with `invalid_request`.
  - That decode is the check the core requires. An owner MAY decode into a
    type generated from the schema, or checked against it, and need not
    evaluate the JSON Schema itself; the reference does not.
  - It MAY check further, and refuses what it rejects with
    `invalid_request` too.
  - A caller MUST NOT depend on a refusal of what the schema refuses: a
    writer sends only what its schema declares (§9.8).

  `[Sc: operations.md §3]`
- **The active instance (O3).** An instance is active on an interface when
  it exposes at least one of the interface's resources (§8.2). It then
  holds the interface's token, unless the interface is in its tokenless
  set (§8.1). The active instance declares a `complete` queryable over each
  optional operation of the interface that it does not expose, and answers
  there with `unavailable` and the cause its descriptor gives or implies
  (§3.3). A standby is active on nothing, so it declares none.
  - **Beside replicas.** Several instances are legitimately active on one
    interface only through replicated operations (§6). An instance whose
    exposed resources of an interface are all replicated operations MUST
    NOT declare an `unavailable` queryable on an exclusive operation of it:
    the instance serving that operation may be another, and a concrete call
    reaches whichever `complete` queryable is nearest (O1).
  - Replicas SHOULD expose the same replicated operations, since one
    answers `unavailable` where another would have served.

  `[Sc: operations.md §3, §8]`

**Fan-out.**
- **Over a template (O2, O3).** A fan-out's key expression can hold a
  wildcard where the template has a parameter, so it names no member,
  while every reply goes on a concrete key.
  - A server declared on one member's concrete key answers for that
    member, on that key.
  - A server declared over the whole template learns from such a call only
    what its key expression binds: each parameter at a concrete chunk,
    unslugged (§1.4), and none at a wildcard. It names the member each
    reply answers for, and replies on that member's key, which the call's
    key expression MUST select. One that names no member has no key to
    reply on, and refuses the call: the reference refuses it `internal`.
  - A concrete parameter chunk that is not a canonical slug (§1.4) names
    no member. A server over the template refuses such a call
    `invalid_request` before any handler runs, fan-out or not, as
    operations.md §3 refuses a concrete call to such a key.
    `[Sc: operations.md §2]`
  - A caller can rely on each value reply it keeps being on a concrete
    member key that its call selected, which names the service and the
    member's values (R6 discards the rest). It cannot rely on one reply per
    member: how many members a server answers for is the server's own. A
    template-wide server answers for one member per call, **whatever
    `replies` is**: with `"many"`, every value it sends goes on that one
    member's key, and naming a second member is refused to the handler.
    One with a queryable per member answers for each it holds.

  `[Sc: operations.md §2]`
- **Attribution (O3, O5).** A value reply is attributed by its key. A
  refusal cannot be: in zenoh 1.10.1, a `reply_err` carries no key
  expression, and the replier's id is unstable API (Appendix B), which the
  core does not use (§0). A caller reports a fan-out's envelopes
  unattributed. Which selected services sent no value it learns from
  presence (§8.1): each of them refused or was silent, and the caller
  cannot tell which. `[Sc: operations.md §2]`
- **Possibly partial (O6).** A **replier** is the replies on one concrete
  key. With a declared `summary`, a caller reads a replier as complete only
  when it ended with exactly one summary.
  - One with none was cut off by the timeout, or broke off.
  - One with two or more is several instances on one key, a split-brain or
    a replicated operation run once per router (§6), whose replies cannot
    be told apart.

  A caller reports both as possibly partial. Without a declared `summary`,
  a replier's completion cannot be told. `[Sc: operations.md §6]`

**Call metadata (O7).** An owner reads a request's attachment as call
metadata only when it is a JSON object whose `actor` and `request_id`,
each optional, are strings where present. Other members are ignored.
Anything else is no metadata: bytes that are not JSON, a value that is not
an object, or a member of another type, `null` included. A call is never
refused for its attachment. `[Sc: operations.md §7]`

### 5.2 The error envelope

A failed call replies with `reply_err` and an envelope with these fields:

| Field | Type | Meaning |
|---|---|---|
| `code` | string | One of `invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app` |
| `message` | string | For a human, never parsed |
| `cause` | string or null | With `unavailable` only, and then required: `build`, `config` or `capability` (§2.3) |
| `detail` | a value, bytes, or null | With `app` only, and optional: a value of the operation's declared `error` type, inline (JSON, CBOR), as its encoded message (protobuf), or as base64 text of its bytes (raw, below) |

**Encoding.** The envelope's kind follows the operation's `error` type when
it declares one, else its `response` type:
- **A JSON Schema type:** the envelope is encoded in the operation's
  `encoding`, JSON or CBOR.
- **A protobuf type:** the envelope is the message `zk2.core.v1.Error`.
- **A raw type:** the envelope is JSON. A raw `error` type's detail is its
  bytes as base64 text (RFC 4648 §4, padded), the JSON form of bytes
  (§7.2). `[F: errors/cases.json]`
- **The reply's `Encoding`** MUST say which:
  - `application/json` or `application/cbor`;
  - or `application/protobuf` with the schema suffix `zk2.core.v1.Error`.

**`app`** is the operation's own failure, and any operation MAY refuse
with it.
- **The detail** is optional. An operation that declares no `error` type
  has none to send: its envelope follows the `response` type, and an owner
  MUST NOT put a detail in it. With an `error` type, an owner MAY send a
  detail, and then it is a value of that type, in the form above.
- **A detail that does not fit** its envelope, such as bytes in a JSON or
  CBOR envelope other than a raw type's base64 text, or a value in a
  protobuf one, MUST NOT be sent. The reference sends `internal` instead.
- **Reading one.** A tool decodes the envelope without the contract, so a
  raw detail reads as a string, and a caller holding the contract decodes
  it as base64. A protobuf detail is absent only when its field is not
  written: an empty one is present. `[F: errors/cases.json]`

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

The decoding edges, each pinned by a case `[F: errors/cases.json]`:
- **The encoding** is compared as an exact string. A parameter
  (`application/json;charset=utf-8`), a bare `application/protobuf`, or
  another schema suffix is an unknown encoding (`encoding`).
- **JSON and CBOR:** a member of the wrong type (`"code": 3`) is `decode`,
  like a missing one. A `cause` or `detail` that is `null` is absent.
- **CBOR:** the bytes hold one data item and nothing after it (`decode`
  otherwise). Indefinite lengths are accepted. A tag decodes as its content,
  and `undefined` as `null`. A map key that is not text, or a float that is
  not finite, is `decode`. An integer decodes from −2^63 to 2^64−1, a signed
  or an unsigned 64-bit value, so an encoder's `uint64` reads back; CBOR's
  other integers, −2^64 to −2^63−1, are `decode`. A byte string inside a
  `detail` reads as base64 text (RFC 4648 §4, padded), the JSON form of bytes
  (§7.2), so a CBOR detail decodes like the same detail sent as JSON.
- **Protobuf:** a known field with the wrong wire type is `decode`. A missing
  `message` decodes as `""` and is accepted, since protobuf cannot tell the
  two apart. A `cause` present but empty is not a valid cause (`cause`).

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
  Each instance that exposes a replicated operation is active on its
  interface (§5.1), and so holds the interface's token, tokenless sets
  aside (§8.1).
- **No core roles.**
  - A standby instance exposes no exclusive resource, and declares no
    interface token for an interface it exposes nothing of.
  - Two instances of one service holding an interface token for the same
    interface, for longer than a grace period, is a **finding**, except
    where replicated serving explains it (below). A tool diagnoses it, and
    the runtime does not fence.
    - **Replicated serving.** Holders are not a finding when at most one
      of them exposes an exclusive resource of the interface: an interface
      whose holders expose only replicated operations is held by any number
      of instances. A tool decides this only for an interface whose
      contract declares a replicated operation, from the contract and the
      holders' descriptors (§3.3's exposure). One that cannot read a
      holder's descriptor or the contract reports the holders as
      undecided, neither a finding nor clear (O5).
    - **Longer than the grace period** is judged from two presence reads,
      `grace` apart: the service and interface held by two or more
      instances in both reads, not necessarily the same ones. A re-mint's
      overlap shows in one read at most, and a standby in neither. A read
      that ended at its timeout can miss a holder, never invent one (§8.1).
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
  - **A type is named one way** (0.8), decoded or not: as a type reference
    is written (§9.1). That is the message name for protobuf,
    `json:<name>` for a JSON Schema type, and the media type for a raw
    one. Which wire a JSON Schema type was read from, JSON or CBOR, is
    part of why it failed to decode, not of its name.

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
  `propertyNames`, `unevaluatedProperties`, draft-07's `definitions`, …).
  Their validation differs between implementations (regular-expression
  dialects), or they defeat the compatibility rules. A contract whose
  schemas use one does not load (E037, once per keyword and file). A
  property *named* like a keyword is data, not a keyword.
  `[F: contracts/e037-subset]`
- **Schema positions** are the document root; each value of `properties`
  and `$defs`; each element of `prefixItems`, `oneOf` and `anyOf`; and the
  value of `items` and `additionalProperties`. A keyword is a member of an
  object at a schema position. Nothing else is a schema position:
  - not the value of a refused keyword, so E037 reports the refused keyword
    alone, never what it holds `[F: contracts/e037-nested]`;
  - not data (`enum`, `const`, `default`, `examples`), so a `$ref` written
    there is not a `$ref` `[F: contracts/e032-fragment]`.

  E037, E032 (§9.4) and the classifier's comparisons (§9.8) read schema
  positions only.

**`oneOf`, `anyOf` and `prefixItems`** are in the subset because Rust-first
contracts (schemars output) use them for enums, `Option<T>` and tuples.
Their containment is not decided in general. The classifier (§9.8) treats
any change inside one conservatively, as *review*, a nullable form aside
(below).

**A nullable** has two spellings, which are one type.
- `{"type": ["integer", "null"]}` is what schemars 1 writes for an
  `Option` of a scalar.
- `{"anyOf": [S, {"type": "null"}]}` is the **nullable form**, which
  schemars 1 writes for an `Option` of a referenced type, and other
  generators for every optional value. It is an `anyOf` with nothing beside
  it that carries meaning, holding two branches in either order: the null
  schema, whose `type` is exactly `null` and which holds nothing else that
  carries meaning, and any schema S.
  - **"Exactly `null`"** (0.8, F-71) reads the `type` as a set of names,
    as everywhere in this subset: `"null"` and `["null"]` are both the
    null schema. `[F: compat/payload/jsonschema/nullable-null-as-list]`
- **Its reading** is S, its `$ref`s followed (§9.8), with `null` added to
  its `type` and, where it has one, to its `enum`. A form has a reading
  when that S is an object with a `type`, and without `const`, `oneOf` or
  `anyOf`: those constrain a null too, so adding `null` to the `type`
  would not admit one.
- A form means what its reading means. Wherever an implementation compares
  two schemas for meaning, the classifier (§9.8) or a generator's check
  that a type agrees with its committed schema, it reads a form with a
  reading as that reading. So `{"anyOf": [{"type": "integer"}, {"type":
  "null"}]}` is `{"type": ["integer", "null"]}`, and `{"anyOf": [{"type":
  "string", "enum": ["a", "b"]}, {"type": "null"}]}` is `{"type":
  ["string", "null"], "enum": ["a", "b", null]}`.
  `[F: compat/payload/jsonschema/nullable-type-to-anyof,
  nullable-anyof-to-type, nullable-enum-inlined]`

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
  one resource**, and none otherwise, except for its **tokenless set**.
  "Who implements `nav.v2`" is the liveliness selector
  `zk2/*/*/@zk/alive/nav.v2/**`. `[Sc: presence.md §1]`
- **The tokenless set** (U22, decided at acceptance).
  - A deployment MAY configure an owner with interfaces for which it holds
    no interface token. These SHOULD be the interfaces every service of the
    deployment implements: `health.v1` and the rest of a framework set.
  - The owner's descriptor marks each such interface `"token": false`
    (§3.3).
  - Their providers are found through instance tokens and descriptors, not
    through a token selector. The split-brain check (§6) does not cover
    them.
  - This is how a deployment above the presence budget (§8.3) cuts its
    multiplier. ZenSight's shape falls from about 42k tokens to about 12k.

  `[F: descriptors/ok-tokenless]` `[Sc: presence.md §5]`
- **Member token,** at `…/@zk/member/<iface>/<member>/<epoch>`, for the
  template that declares `epoch`. An interface has at most one such
  template, because the key carries no template
  (`[F: contracts/e022-two-epochs]`). The owner MUST hold one member token
  per member, and cycle it whenever that member's continuity breaks.
  `[Sc: presence.md §3]`
  - A member exists from the owner's first declaration of it, when the
    entity its value names appears (a device enumerated, a first value to
    publish), never because the contract declares the template.
  - An owner with no member yet holds no member token: one that publishes
    nothing under the template holds none.
- **Re-minting is make-before-break.** An owner that starts a new instance
  id while running MUST declare the new descriptor and tokens first, in
  §8.2's order (the descriptor's queryable and first put, then the
  tokens), then undeclare the old ones. There is an overlap, never a gap.
  `[Sc: presence.md §3]`
- **Reading presence.** A caller or tool's liveliness GET on a session that holds a
  liveliness subscriber MUST use a callback or an unbounded handler. With
  zenoh's default 256-slot handler, such a GET hung at every measured size
  from 996 tokens (zenoh#2678). `[Sc: presence.md §4]`
  - **The subscribers bind too.** Every liveliness subscriber on that
    session MUST be callback-driven, or drained as its samples arrive. A
    bounded subscriber nobody drains starves even a callback GET: with
    zenoh-python 1.10.1, one ended at its timeout with 257 of 2,002 tokens,
    and silently. zenoh-python has no unbounded handler, so a callback is
    the way there. `[Sc: presence.md §4]`
  - A tool SHOULD treat a liveliness GET that ended at its timeout, rather
    than at the routers' final reply, as possibly incomplete: silence is not
    a verdict (O5). In zenoh 1.10.1 the difference shows: a GET that
    reached its timeout ends with an error reply, `Timeout`, and one the
    routers finished ends with none (Appendix B). A read that received any
    error reply is possibly incomplete. `[Sc: presence.md §6]`
  - **A refused read is complete, and empty** (0.8). A router whose access
    control refuses a liveliness GET answers it the way it answers a
    selector no token matches: a final reply, no token, no error reply.
    Measured on zenoh 1.10.1 with a `liveliness_query` deny on the query's
    ingress at the router (on `egress` alone, the same rule refused
    nothing). A reader cannot tell a refused read from absence, so O5's
    attribution through presence is only as good as the reader's grants:
    - The Consume and Call grants (§11.1) include liveliness reads on the
      `@zk` subtree of every service they name, so a principal that may
      consume from a service or call it may also see it alive.
    - A tool reports absence as what its reader could see. Where it cannot
      rule out a refusal, it SHOULD say so, as it says a read is possibly
      incomplete.

    `[Sc: presence.md §6]`
- **Timeouts are the caller's.** How long a liveliness GET, a descriptor
  GET (§3.3) or a retrieval attempt (§8.4) waits, and how long a tool waits
  for presence after an owner starts, are the caller's choices. The
  scenarios, and so a conformance run, use 1 s unless they say otherwise.
  S6's "the GET's timeout" is that choice.
  - **When the wait for presence starts.** An owner starts when its
    session opens, before §8.2's first step, and a tool cannot see that
    instant. The scenarios count the wait from the later of two instants it
    can see: its own session connected to the network the owner joins, and
    the owner's launch. Where the owner's session is the router the tool
    connects to, a client connects only once that session is open, so the
    connection is the later. Measured from there, zenoh-python saw the
    reference owner's tokens within about 1 ms (#609).

### 8.2 Start-up order

An owner MUST bring itself up in this order, so that alive ⇒ callable:
1. declare its resources (publishers and queryables);
2. validate that every required resource is exposed, and every optional
   one exposed or absent as its descriptor will say (below);
3. declare the descriptor's queryable and put the first descriptor (§3.3),
   then a contract queryable (§8.4) for each interface it implements,
   holding that interface's bundle;
4. declare the instance token, then the interface tokens.

`[Sc: presence.md §1]`

- **Exposed** is what an instance serves, as its descriptor says (§3.3).
  At start-up, a resource is exposed once what serves it is declared, or,
  for a template whose members appear later, once the owner serves the
  template:
  - an operation, by its `complete` queryable, on the concrete key or over
    the template (O1);
  - a state, by its interface's state queryables (S2) and its publisher,
    or for a template, the publisher of each member as it appears. Its
    value is not part of being exposed (below);
  - a stream, by its publisher, or for a template, each member's as it
    appears; an event, by the owner publishing its occurrences, which are
    one-shot puts that need no declaration.

  So a templated resource with no member yet is exposed by its template
  (§8.1), and the descriptor claims it. The reference counts a resource
  exposed when it is declared, or marked exposed, on the service before
  start.
- **Step 2** refuses to start an owner with a required resource not
  exposed (§2.3), an optional one neither exposed nor absent as the
  descriptor says (gated on a capability not held, or listed
  `unavailable`), and one both exposed and listed. A descriptor's exposure
  is compact (§3.3), so an optional resource left out of `unavailable` is
  claimed. `[Sc: presence.md §2]`
- **The order of steps 1 and 2** is free: what it protects is that
  nothing of steps 3 and 4 happens unless step 2 passes. The reference
  validates first, then declares its state queryables and `unavailable`
  queryables (§5.1).
- **State values.** Alive ⇒ callable holds for operations through the
  steps. A state's value is not a declaration, so an owner that holds a
  state member's value at start SHOULD put it before step 4: a GET made
  when the interface token appears then finds it. A key the owner has not
  written yet is absent from its answer, which a consumer cannot tell from
  a reply that has not crossed: silence is not a verdict (S6).
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

1. GET with target `BestMatching` and consolidation `None`. That reaches
   the nearest holder on each router the query visits. A holder on the
   caller's own session answers too, so a caller can get its own reply and
   the nearest remote one: it assumes nothing about the count.
2. Verify each reply **as it arrives** (§9.6), and accept the first valid
   one, without waiting for the GET to complete.
3. If none was valid, retry once with target `All`, consolidation `None`.
4. If still none, report the contract **unavailable**. Never accept an
   unverified bundle.

`[Sc: retrieval.md §1–§3]`

- **Consolidation `None` is a MUST.** zenoh's default consolidation on a
  concrete key holds every reply until the query finalizes (`Latest`,
  §4.1), so step 2 cannot happen, and a slow corrupt reply can displace the
  valid one: measured, the corrupt reply alone arrived at 2 s, the valid
  one never, and a caller following the steps would report the contract
  unavailable. A caller MUST set consolidation `None` on both attempts.
  `[Sc: retrieval.md §2]`
- **The reply.** A holder answers with one reply, the bundle's bytes, with
  `Encoding` `application/json`. A caller MUST NOT depend on the encoding:
  the hash is the check. `[Sc: retrieval.md §1]`
- **An attempt ends** when its GET completes, or at the caller's timeout
  (§8.1).
- **From a token to a fingerprint** (0.8). An interface token carries
  `fp16`, the first 16 hex digits of the fingerprint (§1.2), which is not
  enough to retrieve by. A tool reads the full fingerprint from the
  instance's descriptor (§3.3, the interface's `contract`), and retrieves
  by that. There is no retrieval by prefix: a holder declares its
  queryable on full contract keys, and a GET on a wildcard over them is
  not a step of the procedure above.

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
- **A deployment SHOULD attach the far side in one of two shapes, with a
  `@zk` deny on the face** *(0.2, U23)*:
  - **A far router in a south region** of the near router: the far router
    names its region (`region_name`), and the near router lists that name in
    `gateway.south`, while keeping its own clients and peers south. Declarations
    then cross on interest, and the deny keeps the denied families off the
    link: 200 denied tokens cost 506 B, against 11.3 KB router to router.
    The far site keeps its own router and clients.
    `[Sc: constrained.md §6]`
  - **One far-side session, or a gateway session, as a client** of the near
    router.

  A gateway:
  - is the only session on its side that talks across the face;
  - is a principal of its own;
  - never answers or republishes on the near side's keys, and re-keys what
    it relays under its own address.

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
names). A contract MUST NOT need later TOML. A reader MAY be a parser of
later TOML, provided it reports the constructs below as **E000**. They are
the syntax TOML 1.1.0 adds to 1.0, and the list is closed: a construct of a
later TOML version joins it by amendment.
- a newline, a comment or a trailing comma inside an inline table;
- the `\e` and `\xHH` escapes;
- a time without seconds, alone or in a date-time.

`[F: contracts/e000-toml11-inline-newline, e000-toml11-trailing-comma,
e000-toml11-escape-e, e000-toml11-escape-x, e000-toml11-time]`

**Integers.** A TOML 1.0 integer is 64-bit signed. An integer outside
−2^63 to 2^63−1 is not TOML 1.0, so it is **E000**, even for a reader that
can hold it. `[F: contracts/e000-integer-range]`

Its shape is [`contract.schema.json`](contract.schema.json) (JSON Schema
2020-12). An unknown table or field, a value of the wrong type, or text that
is not TOML is **E000**, reported once, and it stops the load.
`[F: contracts/e000-not-toml, e000-unknown-field]`
- A float where an integer is due (`major = 1.0`) is a value of the wrong
  type.
- The schema's `format` is a bound here, not an annotation: `uint32` is 0
  to 2^32−1, and `uint64` is 0 to 2^63−1, TOML's own bound. The `uint32`
  fields also carry their bound as `maximum`. In a descriptor, which is
  JSON, `uint64` is 0 to 2^64−1 (§3.3).

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

**Values.**
- Annotation values are any TOML value except one holding a datetime: any
  of TOML's four kinds (offset date-time, local date-time, local date,
  local time), at any depth of the value (E020).
  `[F: contracts/e020-datetime, e020-twice]`
- Floats are allowed, within the canonical domain (§9.5). `nan` and `inf`
  have no JSON form, and an integral float of magnitude beyond 2^53−1 and
  below 10^21 serializes as an integer outside it: both are E028.
  `[F: contracts/e028-float, e028-non-finite, ok-float-integral]`
- Every other value has the type `contract.schema.json` gives it.
- A field is judged as written: `history = false` on an event is E019, as
  any `history` there is. `[F: contracts/e019-history-false]`

### 9.2 The lints

Diagnostics carry stable codes. `E…` makes a contract invalid; `W…` does
not. An implementation MUST report, for each fixture, exactly the codes
`contracts/expect.json` lists: sorted, with repeats. `[F: contracts/*]`

| Code | Condition | Reported |
|---|---|---|
| E000 | not TOML 1.0 (1.1-only syntax and integers beyond 64 bits included), or outside the format's shape (§9.1) | once; stops the load |
| E001 | the interface name is not one or more `.`-joined segments `[a-z][a-z0-9_]*`, or it ends in, or is, a segment `v<digits>` | once |
| E002 | a `uses` entry is not `<name>.v<major>` | per entry |
| E010 | a template is empty, has an empty segment, a literal that is not a plain chunk or starts with `x-`, a parameter name not `[a-z][a-z0-9_]*`, a repeated parameter, or a rest parameter that is not last | per resource; stops that resource's other checks |
| E011 | a template parameter missing from `params`, or a `params` entry the template lacks | per parameter |
| E012 | a `path` parameter that is not a rest parameter, or a rest parameter that is not `path` | per parameter |
| E013 | a template with parameters has no `cardinality`, or 0 | per resource |
| E014 | an illegal field (§9.1); `cardinality` without parameters | per field |
| E015 | a required field is missing (§9.1) | per field |
| E016 | a non-empty `gate` without `optional = true` | per resource |
| E017 | a gate is not `build:`, `config:` or `capability:` followed by `[a-z0-9][a-z0-9_.-]*` | per gate |
| E018 | the resolved `serving` (§9.3) is `replicated` and the resolved `idempotent` is not `true` | per resource |
| E019 | `history` on an explicit resource, an event or an operation | per occurrence, defaults included |
| E020 | an annotation key not `<profile>.<key>` (split at the **last** dot; the profile is a `uses` name without its major, the key `[a-z][a-z0-9_]*`), a profile not in `uses`, or a value holding a datetime (§9.1); in a resource, a requirement, `[defaults]` or `[defaults.<kind>]` | per key and table: once for the value, and once for the key or its profile, so one key can give two; `[defaults]` and each `[defaults.<kind>]` once, whatever they reach |
| E021 | two templates under one kind token with the same shape (§2.2) | once per template after the first of its shape, in template order (below) |
| E022 | `epoch` does not name a single-chunk parameter of its template; or a second template of the interface declares `epoch` | per resource for the first condition; once per `epoch` template after the first, in template order, for the second |
| E023 | a type reference that does not resolve (§9.4), or a raw type that is not a media type | per reference |
| E024 | a `json:` name defined by several listed files, or two listed JSON Schema files with one stem or one id | per reference; per listed file whose stem or id an earlier one took, once (that file is not loaded, §9.4) |
| E025 | `media_param` does not name a single-chunk parameter of the template | per reference |
| E026 | `rate` not `rare`, `low` or `burst(<n>/h)` (n from 1 to 2^32−1, decimal, no leading zero); `retention` not `<n>` + `s`/`m`/`h`/`d`/`w` (n ≥ 1, decimal digits, leading zeros allowed, the seconds at most 2^64−1, unsigned) (§2.6) | per field |
| E027 | a string in the canonical form (§9.5) outside printable ASCII (0x20–0x7E), keys included | per value |
| E028 | a number outside the canonical domain (§9.5): an integer outside ±(2^53−1), a float that is not finite, or a float JCS writes as such an integer; in the canonical form, or in a JSON Schema artifact | per value; once per artifact |
| E029 | a schema file missing, unreadable, not JSON (a duplicate member included), or not compiling (protobuf editions, and a file under no import root, included) (§9.4) | per file |
| E030 | a role not `[a-z][a-z0-9_]*`, an `interface` not `<name>.v<major>`, a `resources` entry that is not a template, or `resources = []` | per finding |
| E031 | `deprecated.replaced_by` names no other template of the interface, or `since` is greater than `minor` (checked only when `minor` is present) | per condition (both can fire on one resource) |
| E032 | a `$ref` at a schema position (§7.3) whose file part has a scheme (`:`) or names no listed file, or whose fragment is not a JSON Pointer that resolves (§9.4) | per `$ref` |
| E033 | `summary` written, and the resolved `replies` (§9.3) is not `many` | per resource |
| E034 | `history.depth` is 0: on a resource where `history` is legal (where it is not, E019 alone); in `[defaults]`; in `[defaults.<kind>]`, where an illegal `history` gives both E019 and E034 | per occurrence |
| E035 | a requirement names a resource its interface does not declare, when that interface is in the set; checked against the interface's first declaration (below) | per resource name (set check) |
| E036 | two contracts of a set declare one interface id | per declaration after the first (set check) |
| E037 | a JSON Schema keyword outside the subset (§7.3) | once per keyword and file |
| W101 | two resolved resources under one token whose templates overlap, with different types (the payload type, or an operation's request and response) | per pair |
| W102 | an operation's request type has a top-level field named like a template parameter | per resource |
| W103 | `encoding` or `attachment_encoding` written on a resource where no JSON Schema type takes it, judged on resolved types only (cascade 6) | per field (a defaulted one is silent) |
| W104 | `minor` is absent | once |
| W105 | an annotation key outside its profile's interim vocabulary (Appendix D); a profile without one has none to be outside of (§10) | per key |
| W107 | the file is not named `<name>.v<major>.toml` | once |

**Cascades.** A fixture's codes do not depend on the order lints run in,
because of these rules:
1. **E010** stops its resource's other checks. Two checks run over the raw
   resources table anyway: `deprecated.replaced_by` (E031), and the second
   `epoch` template (E022).
2. **E029** for one schema kind suppresses E023 for that kind.
3. **E021** and the `replaced_by` half of **E031** are checked over every
   template, whatever else is wrong with its resource.
4. **W101** is checked over the resources whose own checks found no error.
   The table-wide checks of 1 and 3 (E021, the second-`epoch` E022, the
   `replaced_by` half of E031) do not take a resource out, so one pair can
   give E021 and W101. `[F: contracts/e021-shape-types]`
5. **E027** and **E028** on the canonical form run only when no other
   error was found, because there is no canonical form otherwise. E028 on a
   JSON Schema artifact runs at load.
6. **W103** is judged on resolved types only. A type that is missing (E015)
   or does not resolve (E023, E024) says nothing about whether a JSON Schema
   type takes the field: `encoding` waits for the payload type, or, on an
   operation, for all four types; `attachment_encoding` for the attachment.
   `[F: contracts/e023-encoding]`

**Order.** Resources are taken in the bytewise order of their template
text: a sorted table's order, not the file's, since TOML gives a table's
keys no order. "The first" of E021 and E022 is the template that sorts
first. The codes do not depend on it, only where they are reported.

**Set checks.** E035 and E036 check a set of contract files, taken in
file-name order. `[F: sets/]`
- Each file is loaded on its own first. When one does not load, its own
  codes stand, and the set checks do not run: they need every member.
- E036 falls on each declaration of an interface after the first.
- E035 checks a requirement against the interface's first declaration.
  `[F: sets/e036-first-declaration]`

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
take no defaults. A lint that reads a defaultable field judges its resolved
value: `[defaults.operation] idempotent = true` satisfies E018 for every
operation it reaches. `[F: contracts/ok-resolved-operation]`

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
- **A listed file's name.** A listed file is a path relative to the
  contract. It is named by its path relative to the first import root that
  contains it; a listed file under no import root has no name, and does
  not compile (E029).
  `[F: contracts/e029-outside-include, ok-protobuf-nested]`
- **Each listed file** is compiled alone, with its imports and without
  source info, into a `FileDescriptorSet` holding the file and its imports
  in dependency order: `protoc --include_imports`, without
  `--include_source_info`.
- **The artifact** is those bytes, as protoc 3.21.12 writes them: every
  field carries its `json_name`. Its `name` is the file's name above. A file
  using editions does not compile (E029).
- **A message reference** names a message, a nested one included
  (`pkg.Outer.Inner`), and never an enum (E023). It resolves to the one
  listed file that defines the message:
  - E023 when only an imported file defines it;
  - E023 when two listed files do.

  Listed files are searched before the well-known types, so a listed file
  that defines `google.protobuf.Empty` provides it.
  `[F: contracts/ok-protobuf-nested, e023-enum]`
- **The well-known types** are always available, each compiled on demand as
  its own artifact named `google/protobuf/<file>.proto`, from the sources
  protoc 3.21.12 ships in its `include/` directory. Those sources fix the
  artifacts' bytes, and so their ids. A listed file's
  `import "google/protobuf/…"` resolves through the import roots first,
  then to the same sources:
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
- **The artifact** is the document, read as strict JSON: a duplicate member
  makes the file not JSON (E029). Its id hashes the document's JCS bytes.
  Its `name` is the file's stem, the last path segment without a final
  `.json`, which MUST be unique among the listed files (E024). A later file
  with a taken stem is not loaded. `[F: contracts/e029-duplicate-member]`
- **One id, one name.** The id MUST be unique among the listed files too:
  a later file whose JCS bytes equal an earlier one's, whatever its own
  bytes, is E024, once even when its stem is taken as well, and is not
  loaded. Two such files would be one artifact, which the canonical form
  lists under one name, so a bundle `$ref` by the other stem would name
  nothing (below). `[F: contracts/e024-identical]`
- **`json:Name`** is `$defs/Name` in the one listed file that defines it
  (E023 when none does, E024 when several do).
- **`json:stem#Name`** looks only in the file with that stem: the first
  listed, since a later one is not loaded. `[F: contracts/e024-stem]`
- **A `$ref`** is a member of a schema position (§7.3). Its value is split
  at the first `#`. Each failure below is E032.
  - **The file part**, before the `#`, names the same file when empty. One
    with a `:` has a scheme, and is refused. Otherwise it resolves as a path
    relative to the referencing file, lexically normalized, and MUST name a
    listed file that is loaded: one E024 did not refuse.
  - **The fragment**, after the `#`, is a JSON Pointer (RFC 6901), applied
    as written: `~0` and `~1` are unescaped, and nothing is percent-decoded.
    An empty or absent fragment is the whole document. A fragment that does
    not start with `/` (`#anchor`) is not a pointer. The pointer MUST
    resolve.
  - **In a bundle,** which keeps no paths, the file part names the artifact
    whose `name` is the stem of its last path segment. Stems and ids are
    unique per contract, so every loaded file is an artifact under its own
    stem, and this is the file the path named: in the bundle of a contract
    with no E024 and no E032, every `$ref` resolves.

`[F: contracts/e032-ref-dangling, e032-ref-outside, e032-fragment, ok-json-qualified, e024-ambiguous, e024-identical]`

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
(0x20–0x7E), and every number is in the **canonical domain**:
- an integer within ±(2^53−1);
- a float that is finite and, when JCS writes it as an integer, within the
  same range. JCS writes a float as an integer when it is integral and its
  magnitude is below 10^21, so `1e16` is `10000000000000000`, outside, and
  `1e21` is `1e+21`, a float.

A contract that breaks either is invalid (E027, E028). On that domain every
JCS implementation agrees. An integral float has the canonical bytes of the
integer, so `1.0` and `1` fingerprint alike.
`[F: contracts/e027-ascii, e028-integer, e028-float, e028-non-finite, ok-float-integral]`

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

**Extras** are exactly the documents that the `views.document` annotations
of the contract's resources reference. That is the only extra-carrying
annotation in this version.
- A `views.document` value references one document by its id, a
  `sha256:…` string. Another value references nothing, and requirement
  annotations carry no extras.
- Where a builder finds the document for an id is `views.v1`'s to define
  (#613), not the core's. Until it does, a core builder carries no extras,
  and the bundle of a contract that uses `views.document` fails step 11
  below, so it cannot be published. A verifier checks extras as below
  meanwhile.

A bundle file is the JCS serialization of that object, with all three
members, so one contract revision has one bundle: a builder writes
`"schemas": {}` and `"extras": {}` when they are empty. A verifier accepts
their absence (step 7), and the history check refuses it as not JCS
(§9.7). To verify bundle bytes, an implementation MUST check the following
in order, and refuse with the given tag at the first failure.
`[F: bundles/*, expect.json]`

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
| 9 | For each listed schema, in id order: it is present (`missing_schema`); its `kind` is the listed one, an entry that is not an object or has no `kind` included (`schema_kind`); it has a `data` member and no member besides `kind` and `data`, `data` is base64 text for protobuf, and its kind is `protobuf` or `jsonschema` (`shape`); it hashes to its id, from the decoded bytes for protobuf and the JCS bytes for jsonschema (`schema_hash`) | as given |
| 10 | For each extra, in id order: it has `data`, no member besides `media_type` and `data`, and a `media_type` that is a string when present (`shape`); the JCS bytes of `data` hash to its id (`extra_hash`) | as given |
| 11 | The extras' ids are exactly the `views.document` values of the contract's resources | `extras` |
| 12 | Where the caller expects a fingerprint (from a contract key or a descriptor), the contract's fingerprint equals it | `fingerprint` |

Integrity is Merkle-style. The fingerprint covers the canonical contract,
which lists every schema id and every extra's id (through its annotations).
Each artifact is checked against its id, and an entry carries nothing
besides its artifact, so the whole bundle is verified. An extra's
`media_type` is the one member no hash covers: it is informative.

The steps' details `[F: bundles/*]`:
- **Base64** is RFC 4648 §4: the standard alphabet, padded, decoded
  strictly. Unpadded, URL-safe or whitespace-broken text is `shape`.
- **A JSON document holding a number outside the canonical domain** (§9.5)
  has no JCS bytes every implementation agrees on, so it matches no id:
  `schema_hash`, or `extra_hash`.
- **Verification checks hashes, not content.** A protobuf artifact that
  hashes to its id verifies even if it does not decode. So does a JSON
  Schema artifact holding a `$ref` that names no artifact of the bundle,
  such as the bundle 0.5's rule built from two identical files. A
  conforming builder produces neither: the `$ref` is E032, or, for two
  files with one id, E024 since 0.6 (§9.4). The classifier reads both as
  `schema_unreadable` (§9.8). `[F: bundles/ref-names-no-artifact]`
- **One id listed twice** in `contract.schemas` is one artifact. A
  conforming builder lists each id once, under one name. Two listed JSON
  Schema files never share an id (E024, §9.4), and two protobuf files share
  one only when they share a name too, since a descriptor set's bytes carry
  its file's name.

### 9.7 History and retention

- **History.** A contract's CI keeps every published bundle under a
  **history root** it configures, at `<root>/<iface>/<hex>.bundle.json`,
  where `<hex>` is the fingerprint without `sha256:`. The root is
  append-only.
  - A tool finds a contract's history by its interface id under the root,
    never by the contract file's location, so one root holds the histories
    of many interfaces.
  - By convention the root is `.history` beside the contracts:
    `contracts/.history`, or `examples/zk2/.history` for every example in
    that tree.
- **The history check** MUST verify:
  - every bundle (§9.6);
  - its fingerprint against its file name;
  - its interface against its directory;
  - that it is in JCS form.

  It reports `directory` (an entry that is not an interface directory),
  `file_name`, `io`, `interface`, `jcs`, or a bundle tag. `[F: history/]`
- **The check's order.** It takes the root's entries, then each interface
  directory's, in bytewise order of name.
  - An entry of the root that is not a directory named by an interface id
    is `directory`, and is not looked into.
  - In an interface directory, an entry not named
    `<64 lowercase hex>.bundle.json` is `file_name`, a subdirectory
    included.
  - For each file, in order: reading it (`io`); verifying it (§9.6),
    expecting the fingerprint its name gives (a bundle tag, so a name that
    is not the bundle's fingerprint is `fingerprint`); its interface against
    its directory (`interface`); its bytes against the JCS of the bundle,
    all three members written (`jcs`). The first three stop that file's
    checks; `interface` and `jcs` are both reported.

  `[F: history/wrong-directory-not-jcs, no-extras, stray-entries]`
- **Retention.** A rebuild identical to the newest published revision keeps
  that revision's bundle, and mints no fingerprint. Identity is not one of
  the classifier's classes. Two revisions are identical when:
  - their canonical forms are equal once each schema id is replaced by its
    artifact's `kind` and `name`; and
  - their artifacts, matched by `kind` and `name`, are equal: JSON Schema
    documents by their JCS bytes, and protobuf `FileDescriptorSet`s as
    messages, once source info and every default `json_name` are dropped.
    The default `json_name` is protoc's: the field name with each `_`
    removed and the letter after it upper-cased. Raw types have no
    artifact.

  Fingerprints may over-detect a change, never under-detect one.
  `[F: compat/expect.json, same_revision]`

### 9.8 Compatibility

Revisions inside a major are checked **FULL_TRANSITIVE**: a candidate
against every revision in the history, in both directions. A direction is a
role, writer or reader, never a swap of old and new:
- for streams, state, events and responses, the owner writes and the
  consumer reads;
- for requests, the caller writes and the owner reads.

The classifier compares each earlier revision to the candidate, never the
reverse. Each rule below is a transition from the earlier revision to the
candidate, and its class already accounts for an old reader of a new writer
and a new reader of an old writer: that is what "both directions" means.

A change is **compatible**, **review** or **breaking**. The candidate's
class is the worst over every rule and every earlier revision. A candidate
that does not load at all is **invalid**, like the `add-pattern` case.
Earlier revisions come from verified bundles, so they load.
- **Breaking:** a contract's CI MUST NOT publish the revision.
- **Review:** a human MUST accept it before publication.

The classifier MUST classify every case of `compat/` as `expect.json` says,
including the transitive cases, where a revision compatible with its
predecessor breaks against an earlier one. `[F: compat/]`

Each rule has a name, which a classifier reports with its finding. The
names below are the reference classifier's. `expect.json` pins the class of
every case and the name of every warning. Warnings are reported by rule
name, sorted and deduplicated over the whole history.

**Interface:**

| Change | Class | Rule |
|---|---|---|
| The interface id differs | breaking | `interface_changed` |
| `uses` changed | review | `uses_changed` |
| Documentation and `minor` only | compatible | |

**Resources,** paired by kind (`stream`, `state`, `event`, `operation`) and
template. A template is unique within a contract, since it is the table key,
so a toggled `explicit`, which changes the kind token (§1.3), still pairs:
that is what makes `explicit_cleared` review rather than a removal. A
resource whose kind changed does not pair: the old one is removed, and
another added. `[F: compat/contract/explicit-true-to-false, kind-changed]`

| Change | Class | Rule |
|---|---|---|
| A resource removed | breaking (a deprecated resource stays until the next major) | `resource_removed` |
| An optional resource added | compatible | |
| A required resource added | breaking (old providers lack it) | `required_resource_added` |
| optional → required | breaking (old providers may not expose it) | `optional_to_required` |
| required → optional | breaking (consumers rely on it) | `required_to_optional` |
| The kind changed | breaking: the resources do not pair, so the old one is removed | `resource_removed` |
| `explicit` false → true | breaking for ambient consumers | `explicit_set` |
| `explicit` true → false | review (link budgets) | `explicit_cleared` |
| A template parameter's type changed | breaking | `params_changed` |
| The payload or attachment encoding changed | breaking | `encoding_changed` |
| An attachment added or removed | review | `attachment_changed` |
| `deprecated` added | compatible | |
| `deprecated` removed or changed | review | `deprecated_changed` |
| `cardinality`, `epoch`, `gate` or `annotations` changed | review | `cardinality_changed`, `epoch_changed`, `gate_changed`, `annotations_changed` |

**Delivery** (streams, state, events):

| Change | Class | Rule |
|---|---|---|
| `reliability` reliable → best_effort | review | `reliability_lowered` |
| `reliability` best_effort → reliable | compatible | |
| `congestion` changed, either way | review (`block` can stall the producer; `drop` can lose samples) | `congestion_changed` |
| `priority`, `express`, `history`, `rate` or `retention_s` changed | review | `priority_changed`, `express_toggled`, `history_changed`, `rate_changed`, `retention_changed` |

**Operations:**

| Change | Class | Rule |
|---|---|---|
| `idempotent` true → false | breaking (callers may retry it) | `idempotent_cleared` |
| `idempotent` false → true | review (new callers may retry old servers) | `idempotent_set` |
| `fanout` allowed → forbidden | breaking | `fanout_forbidden` |
| `fanout` forbidden → allowed | compatible | |
| `replies` one → many | breaking (callers' consolidation) | `replies_many` |
| `replies` many → one | compatible | |
| An `error` or `summary` type added or removed | review | `error_type_changed`, `summary_type_changed` |
| `serving`, `timeout_ms` or `priority` changed | review | `serving_changed`, `timeout_changed`, `priority_changed` |

The request, response, error and summary types follow the type rules below.

**Roles** (`requires`):

| Change | Class | Rule |
|---|---|---|
| An optional role added | compatible | |
| A required role added | breaking (deployments must bind it) | `required_role_added` |
| A role removed | compatible (deployments stop binding it) | |
| A role's interface or cardinality changed | breaking | `role_changed` |
| A role optional → required | breaking | `role_required` |
| A role required → optional | compatible | |
| The resources or annotations a role consumes changed | review | `role_resources_changed` |

**Types:**

| Change | Class | Rule |
|---|---|---|
| The schema kind changed (raw, protobuf, jsonschema) | breaking | `type_kind_changed` |
| A raw media type changed | breaking | `media_type_changed` |
| A raw `media_param` changed | review | `media_param_changed` |
| An artifact that does not decode, or lacks the named type; a `$ref` the comparison follows that resolves to nothing in its revision (no artifact by that stem, or no member at that pointer) | review (a conforming builder never produces one; verification checks hashes, not content) | `schema_unreadable` |

Only the artifacts a type reaches are compared. A listed artifact that no
type references can change, which changes the fingerprint, and that is
compatible: nothing reads it. `[F: compat/contract/unreferenced-artifact-changed]`

**Protobuf payloads:** WIRE semantics with renumber detection. A reader
ignores unknown fields and defaults missing ones. Fields are matched by
number. A field missing by number but present by name is renumbered.
- **What is compared.** Messages are compared recursively, from the named
  type through its message-typed fields, each pair of (earlier, candidate)
  message names once. They are compared by structure, not by name: a
  message type renamed is compatible, and a nested type that no field
  reaches is not compared. `[F: compat/payload/protobuf/message-renamed, nested-type-added]`
- **Explicit presence** is protobuf's: a field has it when it is a proto3
  `optional`, a member of a oneof, or a singular field of message type or
  in a proto2 file.
- **A oneof** is identified by its name, so a field moved between two
  oneofs moves out of one and into another. A proto3 `optional`'s
  synthetic oneof is not a oneof here. `[F: compat/payload/protobuf/move-between-oneofs]`
- **An enum** is compared by value number, and is closed when the
  candidate's file is proto2.
- **Not compared:** proto2 defaults, field options other than `json_name`
  (`packed`, `deprecated`), and reserved names. A change to them alone is
  compatible. `[F: compat/payload/protobuf/proto2-default-changed, packed-option]`
- **Breaking:**
  - a field's declared scalar type changes, including int32 → int64 and
    string → bytes (`type_changed`). The declared type is the contract: a
    narrower reader truncates, and a string reader is owed UTF-8;
  - its cardinality changes: singular, repeated or map
    (`cardinality_changed`);
  - it moves into or out of a oneof (`oneof_changed`), which alone is
    reported, even where presence toggles too;
  - it is renumbered (`renumbered`): matched by name, it silently drops the
    data both ways. A renumbered field is not a deleted one, so it raises
    no warning;
  - it reuses a number that an earlier revision reserved
    (`reserved_reused`);
  - a proto2 `required` field is added (`required_field_added`) or deleted
    (`required_field_removed`), or a label toggles to or from `required`
    (`required_label_changed`). A reader refuses a message that lacks a
    required field.
- **Review:**
  - a field renamed (`field_renamed`), or its `json_name` changed
    (`json_name_changed`). Tools decode with the writer's bundle, so a name
    only relabels a display;
  - an enum value renamed or deleted (`enum_value_renamed`,
    `enum_value_removed`); a value renumbered is a deletion and an addition
    `[F: compat/payload/protobuf/enum-value-renumbered]`;
  - explicit presence toggled, such as proto3 `optional`, or a file's
    `syntax` changed between proto2 and proto3 under a singular scalar
    (`presence_changed`): a reader stops telling a default from an absent
    value `[F: compat/payload/protobuf/proto2-to-proto3]`;
  - a value added to a proto2 (closed) enum (`closed_enum_value_added`): an
    old reader keeps it as an unknown field and reads the default.
- **Compatible:** a field added; a value added to a proto3 (open) enum.
- **Warning** `field_deleted_unreserved` (reported alongside the class): a
  field deleted without reserving its number. Reuse is caught against the
  whole history.

**JSON Schema payloads:** the subset of §7.3, read at schema positions.
Readers tolerate unknown properties, and writers send only what their schema
declares. Annotations are ignored.
- **`$ref`s are followed,** across the revision's artifacts, by stem as in a
  bundle (§9.4). Keywords beside a `$ref` (`$defs` aside) are added to its
  target, an outer one taking the place of the target's own, and the result
  is compared like any schema. A `$ref` that resolves to nothing is
  `schema_unreadable` (the types table), never looked up in the
  referencing document instead.
  `[F: compat/payload/jsonschema/ref-sibling-changed, compat/contract/cross-file-ref-retyped]`
- **A boolean schema** (`true` accepts anything, `false` nothing) changed,
  to or from anything, is review (`boolean_schema_changed`). As `items` or
  `additionalProperties`, the rules below apply instead.
  `[F: compat/payload/jsonschema/boolean-property]`
- **Inside `oneOf`, `anyOf` and `prefixItems`,** the schemas are compared
  as written, in order, with annotations dropped at schema positions: a
  reordering is a change. A `$ref` there is compared by what it resolves
  to, followed and its siblings added as above, so a change to a
  definition reached only from inside one is a change inside it, and
  inlining a definition is none. A `$ref` back to a target already being
  followed is compared as written, which ends a recursive type, and one
  that resolves to nothing is `schema_unreadable`.
  - **"As written"** (0.8, F-72) means by its text: the `$ref` value and
    its siblings, annotations dropped, not the target it resolves to. So
    renaming a recursive definition reached from inside one of these
    keywords is a change there (review), although the type is the same,
    and an annotation added inside one is none.
  `[F: compat/payload/jsonschema/oneof-reordered, oneof-annotation-only,
  oneof-ref-target-changed, anyof-recursive-renamed,
  anyof-recursive-described]`
- **A nullable form with a reading** (§7.3) is compared as its reading, so
  a change between the two spellings is none, and a change inside its S is
  classified by the rules below, through S's `$ref`s. The one exception: a
  form against an `anyOf` that is not a form with a reading. Both are then
  compared as written, like any `anyOf` changed, so a `null` branch added
  to another `anyOf` stays review.
  `[F: compat/payload/jsonschema/nullable-type-to-anyof,
  nullable-anyof-to-type, nullable-inner-retyped, nullable-null-dropped,
  nullable-ref-target-changed, nullable-enum-inlined,
  nullable-enum-null-refused, anyof-add-branch]`
- **Compatible:**
  - an optional property added, even to a closed schema, or removed;
  - `additionalProperties` or `items` changed between absent, `true` and
    `false`;
  - `enum` values reordered.
- **Breaking:**
  - the `type` set changed, including integer ↔ number (`type_changed`);
  - a required property added (`required_added`) or removed
    (`required_removed`), or a property optional ↔ required
    (`required_changed`). A `required` name with no property on either side
    still binds the member's presence, so adding or removing one is
    breaking too `[F: compat/payload/jsonschema/required-without-property]`;
  - an `enum` value added or removed (`enum_changed`), or `const` changed
    (`const_changed`);
  - a bound changed, either way (`bound_changed`): `minimum`, `maximum`,
    `exclusiveMinimum`, `exclusiveMaximum`, `minLength`, `maxLength`,
    `minItems` or `maxItems`. A tightened bound breaks old writers, and a
    loosened one breaks old readers;
  - a `oneOf` branch added (`oneof_branch_added`): both revisions have a
    `oneOf` there, and the candidate's has more branches than the earlier
    one's, whatever they hold.
- **Review:**
  - any other change inside `oneOf`, `anyOf` or `prefixItems`
    (`undecided_changed`). Their containment is not decided. The keyword
    added where the earlier revision has none, or removed, is such a
    change: an absent `oneOf` is no constraint, not zero branches, so
    neither is the measured case below
    `[F: compat/payload/jsonschema/oneof-keyword-added, oneof-keyword-removed]`;
  - `additionalProperties` or `items` gaining or losing a schema
    (`members_changed`): a map closed, or array items constrained.

The asymmetry inside `oneOf` and `anyOf` is deliberate. A `oneOf` branch
added is the one undecided change measured (spike S7): a new writer sends
the new variant, and an old reader refuses it. A branch added to an
`anyOf`, or removed from either, is left to a human.
`[F: compat/payload/jsonschema/oneof-add-branch, anyof-add-branch]`

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
   them is a warning. `[F: contracts/w105-vocabulary]` A profile that
   Appendix D does not list has no interim table, so none of its keys is
   outside one: no W105. `[F: contracts/ok-profile-without-vocabulary]`
   What a published vocabulary defines (`views.v1`'s documents among them,
   §9.6) is the profile's, not the core's.
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
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name, plus their `@adv` subtrees where the consumer uses history, plus liveliness reads (GETs and subscriptions) on the `…/@zk/**` subtree of each provider they name (§8.1, 0.8). |
| **Call** | Query on specific `…/@op/<op>` keys, plus liveliness reads on the `…/@zk/**` subtree of each service whose operations it calls (§8.1, 0.8). |

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
- A refused liveliness read is answered complete and empty (§8.1). A deny
  on presence hides a service from a reader exactly as its absence would,
  so a reader the grants refuse attributes silence to absence.

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
| Faces (§8.5) | The far side attached as a client or as a south region, with batches within the lease, `@zk` denied, and `@stream` denied unless downsampled. Presence never crosses. |

`[Sc: constrained.md]`

---

## 13. Open items

The items left open by the design, with their state at acceptance. A
decision that changes a rule lands as an amendment.

| # | Question | Lean | Rule affected |
|---|---|---|---|
| U22 | Cutting the per-service token multiplier above the presence budget | **Decided at acceptance (2026-10-08):** a deployment-configured tokenless set, recommended for the framework interfaces every service implements; the descriptor records it (§8.1, §3.3) | §8.1 |
| U23 | The far side of a constrained face when it is more than one session: a site with its own router, or a service commanding many vehicles | **Settled by amendment 0.2:** the far router is placed in a `gateway.south` region of the near router, with the `@zk` deny. Measured: 506 B for 200 denied tokens, against 11.3 KB router to router (spike S3, U23 addendum) | §8.5 attachment |

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
- A timestamp carries its HLC's id, the zid. Its time is an NTP64 value,
  whose low 32 bits are a fraction of a second, so its unit is 2^−32 s.
- A reply error carries a payload and an encoding, and no key expression.
  `Reply::replier_id` is behind the `unstable` feature.
- A query that sets no timeout waits `queries_default_timeout`, 10 s by
  default.
- A client connects to one endpoint at a time.
- A link's batch is the minimum of the configured size, the MTU and the
  other end's.
- A query timeout arrives as a reply error. A liveliness GET's too: one
  that reaches its timeout ends with the error reply `Timeout` (encoding
  `zenoh/string`), and one the routers finished ends with none.
- A liveliness GET that access control refuses is answered with the final
  reply alone: no token and no error reply.
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
| `scenarios/presence.md` | §1.5, §3.2, §3.3, §8.1–§8.2 |
| `scenarios/bindings.md` | §3.2 |
| `scenarios/retrieval.md` | §8.4 |
| `scenarios/types.md` | §2.4, §7.1–§7.2 |
| `scenarios/security.md` | §11 |
| `scenarios/constrained.md` | §1.6, R7, §8.5, §12 |

The compatibility cases (`compat/`) are evaluated by the reference
classifier (#618). [`compat/README.md`](conformance/compat/README.md)
records where its protobuf verdicts depart from `buf breaking` with WIRE,
and why.

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
| `views` | `document` (an extra's id, `sha256:…`; §9.6) |
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
  valid contracts, also the fingerprint and `<stem>.canonical.json`. The
  codes' meanings are §9.2's table (contracts and sets) and §3.3's
  (descriptors).
- **`bundles/`, `history/`, `errors/`:** expected tags, or the decoded
  value. The tags' meanings are §9.6's, §9.7's and §5.2's.
- **`compat/`:** the class, the warning rule names (sorted and
  deduplicated) and `same_revision`. The case layout and the one-resource
  wrapper are in [`compat/README.md`](conformance/compat/README.md).
