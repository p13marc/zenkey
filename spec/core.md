# zk2 core specification

**Version 0.92** (0.1 accepted on 2026-10-08, #606). Every change goes
through [`CHANGELOG.md`](CHANGELOG.md), amendment-style: each entry records
what changed, what deliberately did not, and why.

This is the normative core of zk2, the keyspace and contract layer for
applications built on Zenoh. It is written so that an implementation in any
language can be built from it, plus [`observer.md`](observer.md) for a
participant that reads (below), the fixtures in
[`conformance/`](conformance/) and the scenarios in [`scenarios/`](scenarios/),
without reading the Rust reference implementation (`zenkey-model`, the
runtime that follows it).

What a reader does with what it reads is a normative text of its own,
[`observer.md`](observer.md), with its own version and changelog: how a
tool, or any participant reading presence, state or the admin space,
reads, when a read is complete, and what it may conclude.
This document states what participants put on the wire, and how owners,
consumers, callers and deployments behave. A block moved to `observer.md`
leaves a one-line pointer in its old place, so its section number still
leads to it.

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
  bind it, whichever of the roles above it plays. They are the rules of §1,
  §7.5, §7.6, §7.7, §8.6 and §9 that name no role, and the others §12 lists: those on what a
  contract states, those naming their lint code, and §3.3's rule for a
  descriptor checker. Each binds it where it does what the rule governs
  (§12).

An archive (§4.4) is an owner of its own keys. Conformance is claimed by
role (§12). [`RULES.md`](RULES.md) lists each class's rules stated with
MUST, MUST NOT, SHOULD or SHOULD NOT. A class also holds the rules its
texts state without one, such as [`observer.md`](observer.md) §1, §2, §5
and §8.

**Evidence.** Each MUST cites what checks it:
- `[F: path]` is a fixture under `conformance/`, which every implementation
  runs;
- `[Sc: file §n]` is a scenario under `scenarios/`: a network behaviour with
  a setup, steps and expected observations;
- `[F: pending]` would mark a fixture still owed. None is, in this version.

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
  selector without a verbatim chunk) never delivers an `@stream` or
  `@state` sample, and a GET on it never reaches an `@op` queryable or a
  control key. `[Sc: grammar.md §1]`
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
  `hostid.v1` mints a system from a machine id where system = host
  ([`profiles/hostid/v1.md`](profiles/hostid/v1.md)).
- **Device-as-service.** A process serving several devices MAY host one
  service per device. Each such service then has its own presence, exposure,
  capabilities and access-control prefix. It holds to about 5,000 devices
  per presence domain (§8.3). Above that, member tokens (§8.1) are the
  shape, at a third of the tokens.
- **The instance id is a continuity epoch.** An owner MUST mint a new
  instance id whenever its counters reset: a process restart, a device
  re-enumerated, a polled device's uptime rewound, a container restarted.
  `[Sc: presence.md §3]`
  - **A new instance writes under a new run** (§4.3). A run that changes
    within an instance is no new instance.
  - The owner SHOULD mint it in a new session, so that the Zenoh
    timestamps §4.3's table keeps change id too. `[Sc: state.md §7]` The
    new run, that id and presence are how a consumer sees the
    discontinuity.
  - An owner's counter that crosses a face where neither is visible
    SHOULD carry its epoch in-band, in its value or its attachment, as
    [`telemetry.v1`](profiles/telemetry/v1.md)'s counter does (its §2.9).
    `[Sc: profiles/telemetry/scenarios.md §8]` Without one, a reader across
    the face reads a decrease as a reset
    ([`link.v1`](profiles/link/v1.md) §2.13; `telemetry.v1` §2.10, §2.14).
- **Moving a service to another host changes no key.**
- **No implicit identity.** Every key form names its system and service
  (§1.1), so a key cannot be built without them. A fleet-wide selection is
  spelled by name or wildcard, never defaulted from the local process.
  `[F: keys.json]` A binding to `self.system` (R1, R2) is spelled in the
  configuration, so it is not an implicit identity: it names the service's
  own system, and is resolved once, at start.

### 1.6 Namespaces and base-relative keys

Keys are **base-relative**: they start at `zk2`. A deployment prefix, when
there is one, is the Zenoh session namespace. The session adds it on egress
and strips it on ingress. Application code SHOULD NOT spell a deployment
prefix. `[Sc: grammar.md §2]`

At the constrained level (§12), a participant without session namespaces
(Appendix B) writes the prefix literally in its keys. A namespaced
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
  A rest parameter MUST NOT appear except as the last segment (E010), and
  its value is a list of strings.
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
positive integer that bounds its population in one instance, or
`"unbounded"` (below). A template without parameters MUST NOT declare it.
An instance MAY declare a lower bound in its descriptor, from 1 to the
contract's (§3.3). On an event, `cardinality` bounds the template's own
parameters, and the key population is cardinality × rate × retention.
`[F: contracts/e013-cardinality, e014-field]`
What a member is, when one is live, and what a tool concludes from a
reading are §2.7's.
- **No ceiling.** A template whose population no contract can fix, such as
  an archive's `{origin...}` (§4.4), still declares a cardinality, written
  **`"unbounded"`** (the canonical form's 4294967295, 2^32−1), which reads
  "no ceiling". Its value is a positive integer like any other, so
  E013 holds. A contract MAY write the number instead: the two spellings
  have one canonical form (§9.5), so one fingerprint, and an
  implementation MUST read them alike. Any other string is E000 (§9.1). A
  tool reads the value as no bound, never as a population to budget or
  estimate with. An instance that knows its population MAY still lower it
  in its descriptor (§3.3).
  `[F: contracts/ok-cardinality-unbounded, ok-cardinality-no-ceiling, e000-cardinality-string]`

### 2.3 Resource attributes

| Attribute | Rule | Fixture |
|---|---|---|
| `optional` | An owner MAY declare an optional resource unavailable (§3.3). An owner MUST expose every required one, or not start (§8.2). | `[Sc: presence.md §2]` |
| `gate` | Only with `optional = true`. One or more of `build:<n>`, `config:<n>`, `capability:<n>`, where `<n>` is `[a-z0-9][a-z0-9_.-]*`. A list is an AND. An empty list is no gate, so it needs no `optional`. | `[F: contracts/e016-gate-optional, e017-gate-syntax, ok-gate-empty]` |
| `epoch` | Names one single-chunk parameter. The owner holds a member token per value of it, cycled on discontinuity (§8.1). | `[F: contracts/e022-epoch]` |
| `deprecated` | `{since, replaced_by, reason}`. `replaced_by` names another template of the contract, and `since` is not later than `minor`. A deprecated resource stays in the contract until the next major, because removing a resource is breaking (§9.8). | `[F: contracts/e031-deprecated; compat/contract/resource-removed]` |
| `attachment` | A type for the sample attachment, fingerprinted and encoded like a payload (§7). | `[F: contracts/ok-kinds]` |
| `annotations` | Keys are `<profile>.<key>`, and the profile is listed in `uses` (§10). | `[F: contracts/e020-annotation]` |
| `frame` | On a plain stream with no parameter and a packed type: its samples travel as frames (§7.7). | `[F: contracts/ok-frame, e046-frame-place]` |

### 2.4 QoS

Each stream, state and event resource carries Zenoh QoS in its contract
(`[F: contracts/ok-kinds, ok-defaults-defaulted]`). An owner MUST publish
with it. `[Sc: types.md §3]`
- **Three of its fields**, `congestion`, `priority` and `express`, map
  one-to-one onto Zenoh's stable QoS.
- **`reliability`** is unstable API in zenoh 1.10.1 (Appendix B). An owner
  that opts into it (§0) sets it. One that does not, or whose binding
  offers no such API, MAY publish reliable, zenoh's default. A tool's
  readings rest on the declared QoS ([`observer.md`](observer.md) §5.1),
  so an owner that publishes reliable moves none of them.

| Field | Values | Default: `stream` | Default: `@stream` | Default: `state`, `@state`, `events` |
|---|---|---|---|---|
| `reliability` | `best_effort` \| `reliable` | best_effort | best_effort | reliable |
| `congestion` | `drop` \| `block` | drop | drop | block |
| `priority` | `real_time`, `interactive_high`, `interactive_low`, `data_high`, `data`, `data_low`, `background` | data | data_low | data |
| `express` | bool | false | false | false |

- **A contract's state and events** SHOULD keep `reliable` and `block`. A
  state declared `best_effort` relies on re-puts to heal a lost update:
  zenoh-modem re-puts each document every `ttl_s/2`, as
  [`freshness.v1`](profiles/freshness/v1.md) §2.4 requires of an owner
  whose state declares a horizon. An archive (§4.4) recording such a
  state can then miss values between re-puts, and `drop` loses values on
  their way to a storage (spike S5, Appendix B).
- **What a tool can conclude rests on it**: [`observer.md`](observer.md) §5.1.
- **`express`** is opt-in. A contract's author SHOULD set it only on
  measurement, since with `real_time` it lost samples and gained no
  latency (spike S9, Appendix B).
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
1.10.1, so `history` is one of the places where a participant opts into
an unstable API (§0). The others are an owner's `reliability` (§2.4), and
a reader's links, replier id and transport events
([`observer.md`](observer.md) §3.5, §4.2, §5.2). An advanced publisher
declares its own queryable and token under `<key>/@adv/…`, so its grants
(§11.1) must include that subtree.

**Numbering.** An owner's advanced publisher numbers its samples. Its
session runs without timestamping (§8.2), and zenoh-ext refuses to
declare a cache whose samples carry neither a number nor a timestamp
(Appendix B).

**`history.miss_detection_ms`** declares miss detection: the owner's
advanced publisher also sends a heartbeat with its last number every that
many milliseconds (zenoh-ext's sample-miss detection). Without it, a
subscriber detects a gap between two samples it received, but not a lost
last sample. What a detected miss shows, and what no detected miss does
not, is §2.7's.

### 2.6 Events

- **One key per occurrence.** An owner publishes each occurrence once, with
  a one-shot put, on `…/<iface>/events/<template>/<ulid>`. The last chunk is
  a lowercase ULID (§1.2) minted by the owner. A reader strips it before
  resolving the template. `[F: keys.json]`
- **`rate`** is `rare` (at most 1/h), `low` (at most 1/min) or `burst(<n>/h)`,
  for each member of the template, within any period that long (§2.7).
  **`retention`** is `<n>` followed by `s`, `m`, `h`, `d` or `w`.
  Both are required on an event. `[F: contracts/e015-event, e026-rate]`
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
  A union storage prunes nothing by retention. zenoh 1.10.1's
  storage manager garbage-collects tombstones only (Appendix B), so an
  occurrence outlives its retention unless a backend, or the operator,
  prunes it. The retention is the bound a consumer applies on replay
  (below).
  An event key is never an owner's state (§4.4).
- **Replay.** A consumer replays with a wildcard GET bounded by the
  retention. Enforcing the bound needs a time-series backend, or a filter on
  the consumer's side: the storage manager's memory backend ignores `_time`
  (spike S5). `[Sc: state.md §8]`

### 2.7 The population budget

**At scale.** A deployment needs this section only once a service's
templated resources hold members that come and go in numbers near their
`cardinality`, or a tool judges their budget. Below that, an owner writes
its `cardinality` (§2.2) and keeps each event's `rate` (below), and the
rest is for a tool that judges budgets.

A templated resource's `cardinality` (§2.2) is a budget the owner keeps,
and a tool can see broken. It counts members, not tokens: the
presence budget (§8.3) is another. `[F: budget/]`

- **The bound.** In an instance, a templated resource's bound is the
  `cardinality` its descriptor states for it (§3.3), else the contract's.
  - A stated value outside 1 to the contract's is D007's, and does not
    apply: the contract's does.
  - The no-ceiling value, 4294967295, is no bound, from either source
    (§2.2).
  - **One service, one population.** A key names no instance (§1.1), so
    the members of a resource are the service's. At most one instance
    exposes it (§6), and a re-mint's overlap shows two instances holding
    the same members. A tool judges them against the greatest bound among
    the instances whose descriptors expose the resource.

  `[F: budget/bounds.json]`
- **A member** is one value of the template's parameters: a key, for a
  stream or a state; for an event, its key without the ULID chunk (§2.6).
- **The owner's rule.** An owner MUST NOT hold more live members of a
  templated stream, state or event than its bound. A member is **live**:
  - **state:** while its key has a value. A deleted key is not live;
  - **stream:** for an hour after the owner last published on it;
  - **event:** for the retention after its last occurrence.

  `[F: budget/population.json]` `[Sc: budget.md §4–§6]`
  - **Keeping it.** An owner keeps its bound by refusing the write
    that would make a new member live while its live members are at the
    bound: a put of a new state key, a publication on a stream member not
    live, an occurrence of an event member not live. A put to a key that
    is live adds no member, and a delete frees one. A refused write
    publishes nothing, and its writer is told: a refusal is never silent.
    An occurrence beyond the rate is refused the same way.
  - **Why an hour.** A stream has no delete, so when a member stops
    existing is the owner's knowledge and never the reader's. A reader
    that forgets a stream member after an hour of silence holds at most
    the bound. "Since the instance started" was rejected: an owner whose
    entities churn, such as interfaces created and destroyed, would
    exceed any bound in a long run, and could keep it only by minting
    instances it does not need (§1.5). An hour is the longest period the
    core already names (`rare`, `burst`).
- **An operation's** `cardinality` (E013 requires one on a template) is
  the population of values its owner serves calls for. No reading of the
  owner counts it, since the values are the callers', so it is not judged
  at this version.
- **`rate` bounds each member** (§2.6): at most one occurrence of a member
  within any hour (`rare`), within any minute (`low`), or `<n>` within any
  hour (`burst(<n>/h)`). An owner MUST NOT publish more: `<n>` + 1
  occurrences of one member less than the period apart break it.
  `[F: budget/rate.json]` `[Sc: budget.md §7]`
  - **Per member, because** §2.2 already states an event's key population
    as cardinality × rate × retention, which holds only when each member
    has the rate. A rate per template would make it rate × retention.
  - An owner keeps it by counting its own occurrences per member, and a
    tool measures it from keys alone: the ULID stripped and the template
    resolved (§2.2). A rate per instance could not be measured, since a
    key names no instance, and a re-mint does not reset it.
  - **Across a restart.** A restart is a re-mint (§1.5), so the
    rate holds across it: a restarted owner counts the occurrences its
    earlier instances published within the period, from a record of their
    instants that it keeps across the restart. The bus need not give them
    back, since a union storage is optional (§2.6). An owner that keeps no
    such record cannot keep the rate across a restart: its occurrences in
    the period after a start can break it, which a tool measuring from keys
    reports like any other break.
  - A template without parameters has one member, so its rate bounds the
    resource.

**What a tool reads**: [`observer.md`](observer.md) §5.2.

**What a tool concludes**: [`observer.md`](observer.md) §5.3.

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
| R1 | **Deployment:** a role MUST be bound by configuration, never in code. A binding is an ordered list of service addresses, exact (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, meaning every service on `vehicle-01` that implements the interface). A provider on the service's own system is spelled `self.system/<service>`, or `self.system/*` for every service on it that implements the interface (below). The order is the configuration's, and it is significant only where a profile says so (below). The binding configuration's format is a recommendation, not a rule. | `[Sc: bindings.md §1, §5, §6]` |
| R2 | **Deployment:** a template parameter MAY be bound too. `{vehicle} = self.system` (or `self.service`) binds the consumer to its own slice of a provider's collection. | `[Sc: bindings.md §2]` |
| R3 | **Owner:** the descriptor (§3.3) MUST list every requirement with its bindings, in their configured order, and its parameter bindings, as configured, a `self.system` provider resolved in its place (below). **Tool:** the data-flow graph: [`observer.md`](observer.md) §7.1. | `[Sc: bindings.md §3, §5, §6]` `[F: descriptors/ok-binding-order]` |
| R4 | A consumer compiled against `X.vN` binds to providers of any revision of `X.vN` (§9.8). | `[F: compat/]` |
| R5 | A consumer MAY wait on presence for its bound providers. A binding resolves at once without it. | `[Sc: bindings.md §1]` |
| R6 | **Consumer:** MUST discard a sample, or a GET reply, whose key expression is not concrete. Zenoh delivers a put on a wildcard key with the publisher's key. | `[Sc: bindings.md §4]` |
| R7 | **Consumer:** a binding MUST NOT require presence. Across a constrained face, a consumer binds statically and judges liveness from the freshness of what crosses, as [`freshness.v1`](profiles/freshness/v1.md) §2.9 measures it. Where nothing crosses, liveness is *unobservable*; what a tool says of it: [`observer.md`](observer.md) §7.1. A `health.v1` status that crosses is such evidence, and its health is unobservable where the face closes it ([`health.v1`](profiles/health/v1.md) §2.8). | `[Sc: constrained.md §3]` |

When several providers are bound, choosing between them is the consumer's.
[`arbitration.v1`](profiles/arbitration/v1.md) says how, for a role whose
contract declares a policy; a `many` role without one receives
from every bound provider.

- **A binding is ordered.** A runtime keeps a role's bindings in the
  order its configuration gives them, an address listed twice included,
  and its descriptor lists them so (R3). The core reads no order: a
  binding means the same set of providers in any order, and a checker
  reads neither the order nor a repeat (§3.3, cascade 8). A profile MAY
  give the order a meaning for a role whose contract uses it:
  `arbitration.v1` reads it as a priority, the first entry the highest
  (its §2.4). `[Sc: bindings.md §6]` `[F: descriptors/ok-binding-order]`

- **A provider on the service's own system.** A consumer's runtime MUST
  resolve a provider whose system position is `self.system` into the
  service's own system, once, when the service starts, as it resolves the
  rest of its configuration: `self.system/<service>` becomes
  `<system>/<service>`, and `self.system/*` becomes `<system>/*`. The
  system is the service's, literal or derived (a minted one,
  [`profiles/hostid/v1.md`](profiles/hostid/v1.md) §2.7), and it does not
  change while the service runs, across a re-mint included.
  `[Sc: bindings.md §5]`
  - **The descriptor lists the provider resolved** (R3): a tool draws an
    edge to an address it can find. A parameter bound to `self.system`
    (R2) is listed as configured.
  - **`self.system` at a provider's system position always means the
    service's own system.** It is also a plain chunk (§1.2), so a system
    literally named `self.system` is bound only by `*`. A deployment
    SHOULD NOT name a system `self.system` or `self.service`.
  - **Only the system position.** `self.service` is not a provider
    spelling, and neither is `self.system` at the service position: each
    is the literal plain chunk it spells.
  - **A tool has no system of its own**: [`observer.md`](observer.md) §7.1.

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
state). Each put of it (below) carries a Zenoh timestamp the owner sets.
A caller GETs with consolidation `None`, as §8.4 does, takes the first
reply, and waits for it as long as it chooses (§8.1).
`[Sc: presence.md §2]`

```json
{
  "format": "zk2-descriptor/0.1",
  "service": "vehicle-01/navigation",
  "instance": "3fa9c2d41b7e0012",
  "run": "0003",
  "catalog": "0319128a",
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
  - An entry marked `"standby": true` exposes none of them, whatever else
    it says (below).
  - `unavailable` lists only optional resources whose absence the missing
    capabilities do not already imply.
  - Each entry gives a cause, `build`, `config` or `capability`, saying why
    the resource is absent here.
  - `capabilities` lists the capabilities held.

  A listed resource that is not an optional resource of the contract is an
  error (D005). One that a missing capability already implies is a warning
  (D006). `[F: descriptors/d005-*, d006-implied]`
- **`token`** is `false` for an interface in the owner's tokenless set
  (§8.1), and `true` otherwise, which is the default. It states the
  configuration, not what is held now: a standby's entry states `true`,
  and its instance holds no token of the interface (§6).
- **`standby`** is `true` for an interface the instance **stands
  by** on: it implements the interface, lists it, and serves none of it
  now (§6). Absent is serving, and `false` written out is the same as
  absent, as for a role's `optional`. `[F: descriptors/ok-standby]`
  - **At scale.** A deployment needs this only once it runs a service in
    two instances, one standing by for the other under `redundancy.v1`
    (§6); with one instance per service, no entry is marked, and absent is
    serving.
  - **It exposes nothing.** A standby entry exposes none of the
    interface's resources, required ones included (§8.2). Its
    `unavailable` and `cardinality` state what the instance will expose
    once it serves, and are checked as any entry's (D005–D007). A tool
    reads them so, never as exposure.
  - **Only as a profile says.** An instance stands by on an interface only
    as a profile it follows says when it stands by and when it serves:
    [`redundancy.v1`](profiles/redundancy/v1.md) for an interface whose
    contract uses it (§6, §10).
  - **Never tokenless.** An entry marked both `"standby": true` and
    `"token": false` is D012: the instance serving an interface that has a
    standby is known by its interface token (§6), and a tokenless
    interface has none. `[F: descriptors/d012-standby-tokenless]`
- **`cardinality`** MAY lower a template's bound for this instance, keyed
  `<kind token>/<template>`, to a value from 1 to the contract's. The
  owner's descriptor MUST NOT raise it. `[F: descriptors/d007-*]` The bound
  it states is the one the instance's owner keeps, and a tool judges
  against (§2.7).
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
  (§3.2). A `self.system` provider is listed resolved, as
  `<system>/<service>` (R1). The bindings keep their configured
  order, an address listed twice included (R3).
  `[F: descriptors/d009-*, ok-binding-order]` `[Sc: bindings.md §5, §6]`
  - **`optional`** is `true` for a role the instance works without, and
    absent otherwise: absent is required, and `false` written out is the
    same as absent. For a manifest role it is the only place a tool learns
    the role's need. For a role a contract declares, it repeats that
    contract's `[requires]`, and a tool takes the need from the contract:
    a value that disagrees is not checked (below). `[F: descriptors/ok-optional-role,
    ok-optional-unchecked]`
- **`profiles`** lists the profiles the instance follows, deduplicated,
  and sorted as §9.5 sorts a contract's `uses`: by name as a string, then
  by major as a number. So `views.v2` comes before `views.v10`, and
  `a.v1` before `a.b.v1`, where a sort of the whole strings gives the
  reverse of each. The checker does not read the order (cascade 8). It is
  the union of two sets:
  - the `uses` of the contracts the instance implements;
  - the derivation-only profiles the instance follows (§10). Today that is
    `hostid.v1`, listed by an instance whose system is minted
    ([`profiles/hostid/v1.md`](profiles/hostid/v1.md) §2.8).

  `[F: descriptors/ok-derivation-profile]`
- **`run`** is the run the instance writes under now (§4.3), as its bytes
  in lowercase hexadecimal: 4 digits for a 16-bit run, and 8 for a 32-bit
  one. So `"00a1"` and `"000000a1"` are two runs.
  - An owner MUST state its run there, and MUST put its descriptor again
    when the run changes, before it writes under the new run.
    `[F: descriptors/ok-run, d002-run-*]` `[Sc: state.md §1, §7]`
  - A checker reads `run` as optional, so a descriptor that states none is
    not D000. Any other spelling than 4 or 8 lowercase hex digits is D002.
    `[F: descriptors/ok-run-wide, d000-run-*]`
- **`catalog`** is the id of the catalog the instance writes under (§8.6),
  as 8 lowercase hex digits.
  - An owner given a catalog MUST state its id there.
    `[F: descriptors/ok-catalog, d002-catalog-*]` `[Sc: presence.md §2]`
  - An owner with none leaves the member out, and counts its `time`
    fields from 0 (§8.6).
  - A checker reads `catalog` as optional. Any other spelling than 8
    lowercase hex digits is D002, and `null` is D000.
    `[F: descriptors/ok-minimal, d002-catalog-*, d000-catalog-null]`
- **`meta`** is informative: host, process, build, and nothing in it is
  checked. One member is used: an owner SHOULD state its session's zid
  as `meta.zid`, as the reference does. How a tool attributes a state
  stamp by it: [`observer.md`](observer.md) §4.4.
  - **How a zid compares.** zenoh 1.10.1 writes a zid as lowercase
    hexadecimal without leading zeros, at most 32 digits (Appendix B), so a
    zid's text can be shorter than 32 digits, and another writer can spell
    the same id differently. An owner SHOULD write `meta.zid` as zenoh
    writes it. How a tool compares two zids: [`observer.md`](observer.md) §4.4.
- **Size.** An owner's descriptor SHOULD stay within 1 KB. At the
  constrained level (§12), it MUST fit one fragment.
  `[Sc: constrained.md §5]`
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
| D002 | `service` is not `<system>/<service>`, two plain chunks; `instance` is not 16 lowercase hex digits; `run` is not 4 or 8 lowercase hex digits; `catalog` is not 8 lowercase hex digits | error | per member |
| D003 | an interface entry's `iface` is not an interface id; its `contract` is not a fingerprint (§1.2); or the interface is listed twice | error | per entry and condition |
| D004 | the entry's interface is among the given contracts, but not at its fingerprint | error | per entry |
| D005 | `unavailable` names a resource the contract does not declare, or a required one | error | per entry |
| D006 | `unavailable` names a resource that a capability not held already implies | warning | per entry |
| D007 | a `cardinality` key names no templated resource of the contract, or its bound is not from 1 to the contract's | error | per key |
| D008 | a capability is not `[a-z0-9][a-z0-9_.-]*`; or one is listed twice | error | once per malformed occurrence; once more for the list when any value repeats, however many do (below) |
| D009 | in a requirement entry: `role` is not `[a-z][a-z0-9_]*`; `interface` is not an interface id; `declared_by` is not one of the interfaces this descriptor lists; a `params` key is not `[a-z][a-z0-9_]*`, or its value is empty; a binding is not `<system>/<service>`, each chunk plain or `*` | error | per finding |
| D010 | a profile is not `<name>.v<major>`; or one is listed twice | error | as D008: once per malformed occurrence, once for the repeats |
| D011 | an interface entry for `archive.v1` is marked `"token": false`: an archive is never tokenless (§4.4) | error | per entry |
| D012 | an interface entry is marked both `"standby": true` and `"token": false`: an interface with a standby is never tokenless (§6, §8.1) | error | per entry |

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
6. D011 reads an entry's `iface` and `token`, and nothing else. It
   is reported for every entry whose `iface` is `archive.v1`, whether or
   not that contract is given, and whatever its `contract` says: cascades
   3 and 5 do not suppress it. An entry checked no further by cascade 2 is
   no `archive.v1` entry. `[F: descriptors/d011-tokenless-archive,
   d011-bad-fingerprint]`
7. D012 reads an entry's `standby` and `token`, and nothing else, as
   D011 reads its `iface` and `token`: cascades 3 and 5 do not
   suppress it, and an entry checked no further by cascade 2 is not
   checked for it. An `archive.v1` entry so marked gives D011 and D012.
   `[F: descriptors/d012-standby-tokenless, d012-bad-fingerprint]`
8. Not checked, deliberately `[F: descriptors/ok-unchecked]`:
   - that a `cause` agrees with the resource's gates: an owner may lack a
     resource for a reason its gates do not name;
   - R3's completeness, that every requirement a contract declares is
     listed: a scenario checks it (`bindings.md §3`);
   - that a role `declared_by` an interface is in that contract's
     `[requires]`, and that `params` values fit the required interface;
   - that `profiles` is the union of the contracts' `uses` and the
     derivation-only profiles the instance follows: a checker cannot see
     which profiles an instance follows, so a profile no given
     contract uses is not a finding `[F: descriptors/ok-derivation-profile]`;
   - the order of `profiles`: an owner writes it, and a tool reads
     the list as a set;
   - the order of a role's `bindings`, and an address listed twice: an
     owner writes them as configured (R3), and a profile that
     gives the order a meaning says what a repeat means
     (`arbitration.v1` §2.4) `[F: descriptors/ok-binding-order]`;
   - that a `self.system` provider was resolved: `self.system` is a
     plain chunk, so an unresolved one reads as a system of that name.
     `bindings.md §5` checks the runtime instead;
   - `minor`, an integer from 0 to 2^64−1 that no check reads (a tool MAY
     read it to order two revisions, §9.8), and `token` except on
     `archive.v1` (D011, cascade 6) and beside `standby` (D012, cascade
     7);
   - that a standby's interface is one a profile governs, and that a
     `"token": false` entry's contract does not use `redundancy.v1`: a
     checker reads the mark for its form, and the profile says
     what a tool concludes of either (`redundancy.v1` §2.14, its
     `descriptors.json`). The owner's start refuses the second (§8.2);
   - that a role's `optional` agrees with the contract that declares it:
     a tool reads the contract's `[requires]` instead.
     `[F: descriptors/ok-optional-unchecked]`
   - that `run` is stated: an owner states it (above), and a descriptor
     that states none is checked as any other is
     `[F: descriptors/ok-minimal]`;
   - that `catalog` names a catalog the checker holds: it reads the id's
     form, and a reader resolves it (§8.6) `[F: descriptors/ok-catalog]`.

---

## 4. State

### 4.1 Zenoh facts this section relies on

- **A router stamps only puts,** where its timestamping is enabled. It
  never stamps a delete or a query reply.
- **Timestamping's defaults.** It is enabled by default for a router, and
  disabled for a peer and a client. A node whose timestamping is enabled
  has an HLC.
- **A session whose HLC is enabled** stamps every put and every delete it
  is not given a timestamp for, a stream sample included, from its own
  clock.
- **A given timestamp is sent.** A put, a delete or a reply given a
  timestamp carries it, on a session without timestamping too.
- **An HLC follows the stamps it receives.** A node whose HLC is
  enabled, a session's included, moves it to the stamp of each put it
  routes, a put it receives for its own subscriber included, when that
  stamp is later than its own clock and within its delta. A stamp further
  ahead it does not take: it re-stamps the put from its own clock, or
  drops it, as a router does (below). Its later stamps can then be ahead
  of its host's clock by up to that delta, so a bound on how far an
  owner's stamps and a reader's clock disagree must hold of the stamps,
  not of the hosts' clocks alone ([`timing.v1`](profiles/timing/v1.md)
  §2.6).
- **An HLC stamps an unstamped put it routes.** A node whose
  HLC is enabled, a session's included, stamps each put it routes that
  carries no stamp, a put it receives for its own subscriber included,
  from its own clock, as a router does. So a reader whose own session has
  an HLC never holds an unstamped sample: it holds one stamped with its
  own zid at its receipt.
- `Latest` consolidation keeps, per key, the reply with the greatest
  timestamp, ranks an unstamped reply lowest, and delivers at query
  completion. Among unstamped replies it keeps the last to arrive.
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
| S1 | **Owner:** every state mutation, put **and** delete, MUST carry its in-band stamp (§4.3): a put in its payload's envelope, a delete in its attachment (§7.5). On a resource §4.3's table lists, the owner MUST also set a Zenoh timestamp on each mutation. | `[Sc: state.md §1]` |
| S2 | **Owner:** a GET reply MUST carry the in-band stamp of the mutation it represents: a reply as the bytes the mutation put, a `reply_del` as the delete's attachment. The owner's reply MUST also carry the mutation's Zenoh timestamp, where the mutation carried one. The owner's reply of a member of a state that §4.3's table lists under `freshness.v2` MUST carry instead that member's last confirmation, as the table says. That holds whether or not a heartbeat vouched for the member since its last mutation. A key of such a state deleted within the window is a member too. Its `reply_del` carries the timestamp minted at the delete. The owner's state queryables cover its interfaces' `state/**` and `@state/**`. It MUST answer a selector over them with every matching live key, plus a `reply_del` for each matching key it deleted within the window. | `[Sc: state.md §1, §2]` `[Sc: profiles/freshness/scenarios-v2.md §3]` `[Sc: profiles/alarms/scenarios.md §3]` |
| S3 | **Owner:** for a key it deleted within the tombstone window, the owner MUST answer a GET with `reply_del` and the deletion's stamps (S2). The window is 60 s, unless the deployment configures the owner with another. A deployment MUST configure it, for state that an archive records, to at least the longest outage it expects between the archive and the owner. Across a constrained face, that is the face's maximum outage (§8.5), and [`link.v1`](profiles/link/v1.md) §2.11 floors the window of every state behind the face at it. | `[Sc: state.md §2, §6]` |
| S4 | **Consumer:** a state GET MUST be addressed to the owner's keys, with target `All` and consolidation `Latest` set explicitly. **Deployment:** MUST NOT run a storage that answers on an owner's `state/**` or `@state/**` keys. | `[Sc: state.md §3]` |
| S5 | **Consumer:** last-known state MUST be read from an archive (§4.4), explicitly. **Owner of an archive:** §4.4. | `[Sc: state.md §4–§6, §9]` |
| S6 | **Consumer:** current state is the owner's answer. A consumer turns to an archive only when the owner gave no reply within the GET's timeout, or presence shows it absent. That silence is not a verdict about the key (O5). The archive's answer is last-known, never current, and a consumer MUST NOT present it as current. | `[Sc: state.md §4]` |
| S7 | **Owner:** a run is never reused, and its seq for state and events never wraps within it (§4.3). | `[F: seed/inband.json]` `[Sc: state.md §7]` |

**A re-put is a mutation.** A put that repeats a member's current
value, as an owner refreshing it under
[`freshness.v1`](profiles/freshness/v1.md) §2.4 makes, is a put like any
other. It carries a new seq (S1). On a resource §4.3's table lists, it
also carries a Zenoh timestamp minted above every one the owner issued
(§4.3). The owner's GET answers carry those stamps from then on (S2). A
reply represents the member's latest put, not its latest change.
`[Sc: profiles/freshness/scenarios.md §1]`

**A heartbeat is no mutation.** Under
[`freshness.v2`](profiles/freshness/v2.md) an owner puts no member again
to confirm it: its heartbeat (§7.5) vouches for it, and moves no seq of
its state. A reply then represents the member's latest put, stamped with
its last confirmation (§4.3's table).
`[Sc: profiles/freshness/scenarios-v2.md §1, §3]`

**A late joiner's seed.** A consumer or a tool that wants an
owner's current state and every change after it, a **late joiner**, seeds
from S4's GET and follows a subscription.
- **The order.** A late joiner MUST declare its subscription first, then
  make S4's GET. In the other order, a mutation the owner makes after it
  answers the GET and before the subscription reaches it is lost, and a
  lost delete leaves a deleted key standing. `[Sc: state.md §10]`
- **The merge.** A late joiner MUST merge the GET's replies and the
  subscription's samples per key, whichever path brought each one. It
  MUST keep the newer by its in-band stamp: within one run, by seq
  (§4.3). Across two runs, a consumer MUST instead keep what §4.3's
  consumer rule accepts. `[F: seed/inband.json]` `[Sc: state.md §7, §10]`
  - A live put or delete newer than a reply replaces it, and a reply newer
    than a live sample replaces that.
  - A `reply_del` is a deletion with its stamps (S3), merged as a live
    delete is.
  - Two samples with one in-band stamp are the same mutation, kept once.
  - **Across two runs**, nothing orders the values (§4.3). A consumer
    orders by §4.3's consumer rule from its first value on, its seed's
    replies included. So it keeps a value under a run other than its held
    value's. A tool keeps the later arrival
    ([`observer.md`](observer.md) §4.1). Either way, a value of the old
    run that arrives after one of the new is §4.3's residual risk.
    `[Sc: state.md §7]`
  - **A sample with no envelope** is an `@stream` sample (§4.3), a
    foreign key's (§1.7), or one that broke S1. It is ordered among
    samples with no envelope by its Zenoh timestamp, as below. Beside an
    in-band stamp it is unordered, as across runs. `[F: seed/inband.json]`
    - **Zenoh timestamps compare as `Latest` compares replies** (§4.1),
      which is zenoh's timestamp order (Appendix B). First by time, an
      unsigned 64-bit NTP64 value. On equal times, by id: each id's
      128-bit value written as 16 bytes little-endian, least significant
      byte first, and the two compared byte by byte from the first. So the
      least significant byte that differs decides, which is not the ids'
      order as integers: at one time, id `100` (hex) is below id `2`.
      `[F: seed/merge.json]` `[Sc: state.md §11]`
      - Within one timestamp id, that is by time. Across two ids, a
        consumer accepts a value under an id other than its held value's,
        as under another run (§4.3). A tool keeps this order across them
        ([`observer.md`](observer.md) §4.1).
    - **A sample with neither stamp** ranks below every stamped one, an
      in-band stamp included, as `Latest` ranks an unstamped reply lowest
      (§4.1). It is no owner's state (S1). It is an `@stream` sample, or
      a foreign key's (§1.7), put by a session with no HLC through no node
      that stamps, the reader's own session included (§4.1).
      - It is kept on a key that holds nothing, or only such samples, and
        never over a stamped one.
      - Two such samples of a key have no order but their arrival, so the
        later replaces the earlier. That includes a reply that arrives
        after a live sample.

      `[F: seed/merge.json]` `[Sc: state.md §11]`
- **R6 and a tool's seed**: [`observer.md`](observer.md) §4.1.
- **A selector over several owners.** A GET on a selector that matches
  several owners' keys is addressed to the owner's keys, as S4 requires,
  when each key it matches has one owner (P3, §6): `Latest` then
  consolidates each key over that owner's answers. The keys it matches
  that no zk2 owner has, foreign keys (§1.7), are outside S4, and a storage
  may answer them.
- **What a seed cannot know**: [`observer.md`](observer.md) §4.1.

A **tool** checks S4 against the routers' storage admin space: [`observer.md`](observer.md) §4.2.
- **The admin space is off by default** in zenoh 1.10.1 (Appendix B). A
  deployment that wants S4 checked enables it, read-only, for the tools'
  principals.
- **What the check reads**: observer §4.2.
- **What a tool does not ask**: observer §4.3.
- **Who answered**: observer §4.2.

**Observing S1.** An in-band stamp is the owner's, since only the owner
writes its payload. What a tester checks is the Zenoh timestamps, through
two routers, the owner a client of each.
- **Outside §4.3's table,** the owner's puts arrive with no Zenoh
  timestamp through a router whose timestamping is disabled. Through one
  with it enabled they arrive with the router's zid. Its deletes and
  replies arrive with none either way (§4.1). `[Sc: state.md §1]`
- **On a resource the table lists,** each mutation and reply carries the
  owner's timestamp, told from a router's by its id: the owner's
  session's zid, against the router's (§4.1). Where the owner's session
  is itself the router, the two zids are one, and that check proves
  nothing. `[Sc: state.md §1]`
- **A tool's S1 check**: [`observer.md`](observer.md) §4.4.

### 4.3 Runs and sequence numbers (S7)

An owner's values are ordered by a stamp it writes in band, in §7.5's
envelope: a **run**, an ordering epoch, and a **seq**, which counts
within it. A few resources carry a Zenoh timestamp beside it, for the
time a profile reads (the table below).

- **The in-band stamp.** An owner MUST write each put on a `stream`,
  `state`, `@state` or `events` key, and each delete of a state member,
  with §7.5's envelope. `[Sc: state.md §1]`
  - The sample's Encoding is `zenoh/bytes`, and its record is encoded as
    the contract's `encoding` says (§7.2).
  - **Explicit streams.** An `@stream` sample carries no envelope at this
    version. Its frames would be copied into one, or out of one, which
    loses the zero-copy path §7.4 serves.
  - **What keeps its wire.** Operations, the descriptor and bundles carry
    no envelope.
- **The run** is an instance's ordering epoch, of 16 or 32 bits, as
  §7.5's width bit says. Two runs of different widths are two runs,
  whatever their values.
  - **No reuse.** An owner MUST NOT write under a run that it, or another
    instance of its service, wrote under before. A random run (below)
    meets it. The chance of a repeat is accepted. `[Sc: state.md §7]`
  - **A 16-bit run** is a counter in a record the owner keeps. The record
    is durable before the first sample written under each run it issues. It
    moves with the service, since a move changes no key (§1.5). An owner
    that cannot keep such a record draws 32-bit runs.
  - **A 32-bit run** is drawn at random. An owner draws one when it keeps
    no record, when its counter is spent at 0xFFFF, and when it is a
    candidate of a governed interface
    ([`redundancy.v1`](profiles/redundancy/v1.md)). It draws again while
    the run equals one it used.
  - **A new run** is taken by each new instance (§1.5). Within an
    instance, an owner takes a new run before its seq would wrap (below).
    It MAY take one at any other time, such as when it can no longer keep
    each stream key's seq. A run that changes within an instance mints no
    new instance.
  - **The descriptor** states the run the instance writes under (§3.3).
- **The seq** is an unsigned 24-bit count within one run.
  - **State and events.** An owner keeps one seq per run for all its
    state mutations and event occurrences. It starts at 0, and adds 1 at
    each. So a key's mutations carry increasing seqs, with gaps.
  - **No wrap.** An owner MUST NOT let its seq for state and events wrap.
    Before it would pass 16,777,215, the owner takes a new run and puts
    its descriptor stating it. Then it writes under the new run from
    seq 0. `[Sc: state.md §7]`
  - **Streams.** Each stream key's seq is its own. It starts at 0 at the
    key's first sample in a run, adds 1 at each sample, and wraps modulo
    2^24. A frame of n records takes n seqs, one for each record (§7.7). An owner MUST NOT restart a stream key's seq within a run, even
    where it declares the key's publisher again: a takeover, or a member
    removed and added. `[Sc: state.md §13]`
  - **Serial order.** Of two seqs s and s′ of one stream key in one run,
    s′ is newer when (s′ − s) mod 2^24 is from 1 to 2^23 − 1, and older
    when it is above 2^23. At exactly 2^23 they are unordered, as two runs
    are. A tool's merge of a stream key orders so, its `@adv` history
    included. `[F: seed/inband.json]` `[Sc: state.md §13]`
  - **Plain order.** A state or event key's seqs compare as integers.
    Within one run, a state key's seq can trail the owner's by more than
    2^23, which serial order would read as older. `[F: seed/inband.json]`
- **Consumer rule.** A consumer MUST order each state key's values by seq
  within one run, from the first value it receives, a late joiner's seed
  replies included (§4.2). A consumer MUST accept a value under a run
  other than its held value's, and order by seq within that run from then
  on. `[F: seed/inband.json]` `[Sc: state.md §7]`
  - **A value with no envelope**, one that broke S1, is ordered by its
    Zenoh timestamp: by time within one timestamp id, and under another id
    as under another run. Beside an in-band stamp, it is as under another
    run. `[F: seed/inband.json]`
  - **A profile's order.** A profile MAY order a consumer's values more
    strictly. A `desired.v1` target refuses an older version of an id it
    remembers, whichever id it holds ([`desired.v1`](profiles/desired/v1.md)
    §2.6).
- **Nothing orders two runs.** A reader compares two runs for equality,
  never as numbers: a run names a span of an owner's writing, not when it
  began. Ordering two runs is left to a later version.
- **Residual risk.** A value of an old run that is still in transit can
  arrive after the new run's first value. A consumer accepts it, as
  another run's, and a tool keeps it, as the later arrival. The key steps
  back to the old run until the new one's next value. That is accepted at
  this version. `[Sc: state.md §7]`
  - **Inside a seed.** An old run's value can arrive after the new run's
    for as long as the GET is open, not only in transit. `Latest` delivers
    every reply at the GET's end, after the samples heard during it
    (§4.1). So a consumer and a tool both hold the reply's value over a
    value of the new run heard live, until the key's next value.
    `[Sc: state.md §7]`
  - **Where two instances answer**, `Latest` keeps one reply per key. It
    keeps the one with the greater Zenoh timestamp where a reply carries
    one, else the last to arrive (§4.1). So a seed can hold either
    instance's value. `[Sc: state.md §7]`
- **The Zenoh timestamps kept.** Only these resources carry a Zenoh
  timestamp the owner sets, beside the in-band stamp. Each is kept for the
  time a profile reads from it. A state among them carries one on each
  mutation and reply (S1, S2), and a stream as its profile says. A state
  of a contract that uses `freshness.v2` carries one on its replies alone.

  | Resource | Read for | Carried as |
  |---|---|---|
  | A state whose contract declares `freshness.ttl_s` above 0, and does not use `freshness.v2` | a reply's age ([`freshness.v1`](profiles/freshness/v1.md) §2.4, §2.6) | S1, S2 |
  | A state whose contract declares `freshness.ttl_s` above 0, and uses `freshness.v2` | a reply's age, from its last confirmation ([`freshness.v2`](profiles/freshness/v2.md) §2.4, §2.6) | on its replies alone, never on a put: the later of the timestamps the owner minted at its last mutation and at the last heartbeat that vouched for it (S2, "Minting" below) |
  | A `desired.v1` desired resource | the order of an author's versions ([`desired.v1`](profiles/desired/v1.md) §2.4, §2.6, §2.11) | S1, S2 |
  | A state that is a metric resource of `telemetry.v1`, or of `telemetry.v2` and no frame resource | a rate's time base ([`telemetry.v1`](profiles/telemetry/v1.md) §2.11, which [`telemetry.v2`](profiles/telemetry/v2.md) §0 restates) | S1, S2 |
  | A stream that declares a `timing.v1` lifespan | a sample's age ([`timing.v1`](profiles/timing/v1.md) §2.6) | that section says |
  | A `health.v1` fault | nothing a reader concludes ([`health.v1`](profiles/health/v1.md) §2.5) | where the owner sets one, as that section allows |
  | Every put of a `redundancy.v1` candidate | which instance is the incumbent ([`redundancy.v1`](profiles/redundancy/v1.md) §2.10) | by the candidate session's HLC, as that section says |

  - **Minting.** zenoh 1.10.1 keeps `Session::hlc()` internal. So an
    owner mints each such timestamp as the greater of
    `Session::new_timestamp()` and the last it issued plus one tick. Its
    id is the session's zid. On a session without timestamping, as an
    owner's runs (§8.2), `Session::new_timestamp()` is the wall clock with
    the zid (§4.1). `[Sc: state.md §1]`
    - **A tick** is the smallest step the owner's timestamp type can take
      above the last. Timestamps are compared by their time, an NTP64
      value (Appendix B), whose unit is 2^−32 s: the reference adds one
      unit. A larger step keeps the rule, which needs only that each stamp
      exceed the last (Appendix B gives zenoh-python's smallest step).
    - **A last confirmation** (the table's second row) is minted so too.
      An owner mints one at each mutation of such a member, a delete
      among them, though the mutation carries none. It mints one as it
      writes each heartbeat (§7.5).
      That heartbeat's time is the timestamp's instant, to the
      millisecond, counted from the deployment epoch (§7.6). While the
      clock guard holds ("Ahead", below), the time is unknown.
      `[Sc: profiles/freshness/scenarios-v2.md §3]`
  - **Order and id.** Two such timestamps with one id order by time. A
    timestamp under another id is another session's.
  - **Catch-up**, which profiles cite, is the last such timestamp an owner
    issued for its keys, read from its own persistent record, else from an
    explicit read of an archive on its own side. A consumer-side archive
    can be stale by a whole partition.
  - **A new epoch**, which profiles cite too, is a new instance (§1.5)
    whose session has a fresh, unpinned zid.
  - **Across a restart.** A profile that orders these timestamps across a
    restart says how its owner keeps them above the last.
    [`desired.v1`](profiles/desired/v1.md) §2.11 does.
- **Ahead.** A router re-stamps or drops a timestamp beyond its HLC's
  delta (§4.1), and freshness ages by the table's timestamps. An owner
  SHOULD NOT stamp beyond its router's HLC delta: it cannot know its own
  offset without a reference. `[Sc: state.md §7]`
  - The owner SHOULD detect drift against a reference it holds for the
    purpose, such as a subscription to a router-stamped heartbeat key,
    because its own puts are never echoed back. `[Sc: state.md §7]`
  - When a detection shows it beyond the delta, the owner MUST stop the
    state writes that carry a Zenoh timestamp, re-puts included, and
    SHOULD report it. `[Sc: state.md §7]`
    `[Sc: profiles/freshness/scenarios.md §2]`
  - **What it holds.** A state write carries a Zenoh timestamp where the
    table lists its resource. On a session with timestamping enabled
    (§8.2), every state write carries one. Those members go stale, by
    design ([`freshness.v1`](profiles/freshness/v1.md) §2.10).
  - **What goes on.** The owner's other state writes go on, and its
    events and streams. A put or a delete that carries no Zenoh timestamp
    gives a router no time of the owner's to re-stamp or drop. On a
    session without timestamping, a state of a contract that uses
    `freshness.v2` is one: its heartbeat goes on vouching for it, with its
    time unknown ([`freshness.v2`](profiles/freshness/v2.md) §2.10). Its
    replies carry a last confirmation from the clock ahead, which a router
    never re-stamps (§4.1). `[Sc: state.md §7]`
    `[Sc: profiles/freshness/scenarios-v2.md §2]`
  - **A candidate's.** A `redundancy.v1` candidate's session stamps every
    write (§8.2), so the guard holds every write of its `freshness.v2`
    members too. Its heartbeat goes on, but never vouches for a member
    whose write the guard held, until a write of it goes on
    ([`freshness.v2`](profiles/freshness/v2.md) §2.10).
    `[Sc: profiles/freshness/scenarios-v2.md §2]`
  - **The report.** The standard way is a stream sample: a `health.v1`
    owner publishes a `faults` sample with code `clock_ahead`
    ([`profiles/health/v1.md`](profiles/health/v1.md) §2.5). Its status is
    a `freshness.v2` state, which the guard does not hold, so the status
    says nothing of the clock. `[Sc: profiles/health/scenarios.md §4]`
    - Where the owner stamps the sample from its clock, which is ahead
      (the table above), a router re-stamps it, or drops it under
      `timestamping.drop_future_timestamp`, which loses the report.
  - Spike S12 showed the harm: a clock 2 s ahead produced a revision with
    two timestamps, and the next correctly clocked write looked stale.

### 4.4 Archives

An **archive** is a service implementing `archive.v1`, a profile not yet
written. This section is what the core requires of it.

- **It records** owners' mutations under its own address, with each
  mutation's stamps (S1) and type identity (§7.1), and keeps tombstones
  for at least the window (S3). A put's bytes are kept as the owner wrote
  them, its envelope included, and a reply gives them back verbatim.
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
- **An archive's backend MUST NOT accept a put older than a delete it
  holds.** The storage manager of zenoh 1.10.1 accepts one, and resurrects
  the key, so it cannot be an archive's store as released.
  - **Older** is by seq within one run (§4.3). A put of another run is
    recorded, since nothing orders two runs. A late put of an old run so
    revives a deleted key until the next alignment: §4.3's residual risk,
    as an archive meets it. `[Sc: state.md §9]`
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
    the peer holds the owner's delete, with its stamps.
    `[Sc: state.md §5]`
  - **Older never wins.** A reply older than what the archive holds changes
    nothing, as a put older than a held delete does not (above). One with
    the same stamp can confirm the key, never unconfirm it.
  - **A raised window (S3) carries this rule.** With the window at least the
    longest expected outage, every delete the archive missed is still
    answered with `reply_del` after the heal.
  - **An unconfirmed key expires.** An outage longer than that can leave a
    key that alignment never confirms. The archive MAY drop such a key after
    a deployment-set horizon, and MUST keep serving it as unconfirmed until
    then.
- **Found by its token.** An archive MUST hold its `archive.v1`
  interface token: it is never in the tokenless set (§8.1). Consumers and
  tools find archives by that token for last-known reads (S6), and a
  tokenless one would be invisible to them. An owner whose tokenless set
  names `archive.v1` is refused at §8.2's step 2, and a descriptor that
  marks it `"token": false` is D011.
  `[F: descriptors/d011-tokenless-archive]`
- **Placement.** An archive on the consumer's side covers losing the link,
  and one on the owner's side covers losing the owner. Store-and-forward
  ([`desired.v1`](profiles/desired/v1.md) §2.9) uses both.
- **What a tool can see**: [`observer.md`](observer.md) §4.5.

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
| O5 | **Caller and tool:** MUST NOT treat an empty reply set as a verdict. Access-control refusals return empty since zenoh 1.3. A tool attributes silence through presence, which a refusal can empty too (observer.md §3.4). | `[Sc: operations.md §5]` |
| O6 | **Owner:** an operation declared `replies = "many"` gives zero or more value replies, then completion. With a declared `summary`, each replier MUST end with exactly one summary reply, whose attachment is the ASCII bytes `summary`; value replies carry none. **Caller:** MUST use consolidation `None`, since `Latest` and `Auto` drop replies (spike S6, Appendix B). A replier without exactly one summary is possibly partial (below). | `[Sc: operations.md §6]`; `[F: contracts/e033-summary]` |
| O7 | **Caller:** a request MAY carry an attachment, the JSON object `{"actor", "request_id"}` (strings), which an owner MAY record for audit. It is claimed, never authentication, and never a reason to refuse a call (below). | `[Sc: operations.md §7]` |

Work that outlives a query timeout belongs to `jobs.v1`. A request type
SHOULD NOT repeat a template parameter (W102). `[F: contracts/w102-repeat]`

**Calling.**
- **A concrete call (O1).** `BestMatching` reaches one `complete`
  queryable on each router, and consolidation `None` delivers each reply as
  it arrives, as on every operation call (O2, O6). On a concrete key,
  zenoh's default is `Latest` (§4.1): it holds the reply until the query
  completes, and keeps one reply per key. So the second execution of a
  split-brain would arrive unseen. A one-reply caller takes the first value
  or envelope on the call's key, without waiting for the query to
  complete. `[Sc: operations.md §1]`
- **The timeout** is the caller's (§8.1). A caller that sets none waits
  the operation's `timeout_ms`, the contract's recommendation, and with
  neither, 10 s, zenoh's default query timeout (Appendix B). Across a
  constrained face, it waits at least the face's reply timeout (§8.1). A
  call that ends there with no value and no envelope is silent (O5).
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
  queryable gets a value or an envelope. How a tool judges O3 from
  outside, **Where a tool learns its grants** and what it holds when
  **Told so**: [`observer.md`](observer.md) §7.2.

  A handler that ends without
  replying is answered `internal`, so a live server's own bug is never
  silence (O5). With `replies = "many"`, a declared `summary` is owed too:
  a handler that ends without it is answered `internal`, after any values
  it sent (O6). Without one, zero values then completion is the
  operation's own answer, which a caller cannot tell from silence.
  - **Sending nothing needs no member.** Only a reply needs a
    key (below, "Over a template"). So a server over a template whose
    handler names no member and sends nothing ends the call as zero values
    then completion, when no `summary` is declared, and is answered
    `internal` when one is.

  `[Sc: operations.md §2, §3, §6]`
- **The order of refusals.** Before any handler runs, an owner
  refuses a call for the first of these that applies, in this order:
  1. a key expression that is not concrete, on an operation that forbids
     fan-out: `fanout_forbidden` (O2);
  2. an operation the instance does not expose now: `unavailable`, with
     its cause (O3);
  3. a key that names no member (below): `invalid_request`.

  So a wildcard call to an operation that forbids fan-out is
  `fanout_forbidden`, whatever its other chunks hold. A request that does
  not decode is refused after these, by the server's decode (below).
  `[Sc: operations.md §2]`
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
- **`unavailable` from a handler.** An operation the instance
  exposes may still be unavailable for one member of its template: a
  member its configuration or its capabilities leave out now, such as
  `config.v1`'s groups outside the startup allowlist. Its handler MAY then
  refuse a call to that member with `unavailable` and the cause, `config`
  or `capability`, after the refusals above and the request's decode. The
  descriptor does not list it: §3.3's `unavailable` names resources, not
  members. A profile that refuses so names the member and its cause in
  its own state, as `config.v1`'s `writable` does, so a tool sees it before
  it calls. `[Sc: profiles/config/scenarios.md §9]`

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

**The forms.** §7.5 defines a call's three forms: its request, a value
and an error. No rule at this version has a caller or an owner write
them. A call's request, its values and its errors keep the wire this
section and §5.2 describe.
- **The as-of mark.** A value's and an error's run and seq are the
  replying instance's **as-of mark**. The run is the one it writes under
  when it replies (§4.3). The seq is the last it minted for state and
  events under that run, or 0 when it has minted none. A reply mints no
  seq.
- **What it tells a caller.** It learns which run answered, without
  presence. A run that changes is a new instance, or one that took a new
  run (§4.3). Within one run, it can order the reply against the state it
  holds, by seq.
- **A request** carries no run: its caller need not be an owner.
- **An error's run** attributes nothing. A `reply_err` still carries no
  key, so a fan-out's errors stay unattributed (above). A tool that holds
  descriptors may map the run to the instance that states it (§3.3).

### 5.2 The error envelope

A failed call replies with `reply_err` and an envelope with these fields:

| Field | Type | Meaning |
|---|---|---|
| `code` | string | One of `invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app` |
| `message` | string | For a human, never parsed |
| `cause` | string or null | With `unavailable` only, and then required: `build`, `config` or `capability` (§2.3) |
| `detail` | a value, bytes, or null | With `app` only, and optional: a value of the operation's declared `error` type, inline (JSON, CBOR), as its encoded message (protobuf), or as base64 text of its bytes (raw, below) |

**The codes.** Each says what failed, for a caller that reads no
message:
- `invalid_request`: the request, or the member the key names, is
  malformed, or breaks the operation's rules (§5.1, "The request" and "A
  key that names no member").
- `not_found`: a well-formed member, or a thing the request names, that
  the owner does not have.
- `unavailable`: the operation, or one member of it, is not served here
  now, for its cause (O3; §5.1, "`unavailable` from a handler").
- `forbidden`: a rule of the owner's own refuses the act, and goes on
  refusing it while what the rule rests on holds. It is a profile's or an
  application's rule, such as `config.v1`'s refusal to persist a change
  that set a sensitive value. It is never access control's, which is
  silent (O5, §11.3), and never a configuration or a capability that
  leaves the act out, which is `unavailable`.
- `fanout_forbidden`: a fan-out to an operation that forbids one (O2).
- `busy`: the owner's state refuses the call now, and the same call may
  succeed later. Calling again is a new call, never a retry (O4).
- `internal`: the owner failed, as a handler that ends without replying
  does (O3).
- `app`: the operation's own failure (below).

Only `app` carries a detail. A refusal of another code that has
something for a program to read leaves it in the owner's state, which a
program can GET, and names it for a person in the message, as
`config.v1`'s `busy` names the pending change's token.

**Encoding.** The envelope's kind follows the operation's `error` type when
it declares one, else its `response` type:
- **A JSON Schema type:** the envelope is encoded in the operation's
  `encoding`, JSON or CBOR.
- **A protobuf type:** the envelope is the message `zk2.core.v1.Error`.
- **A raw type:** the envelope is JSON. A raw `error` type's detail is its
  bytes as base64 text (RFC 4648 §4, padded), the JSON form of bytes
  (§7.2). `[F: errors/cases.json]`
- **The `Encoding` of the owner's reply** MUST say which:
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
  protobuf one, MUST NOT be sent by an owner. The reference sends
  `internal` instead.
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

**The error form.** §7.5's error carries the same fields in bytes, under
the `Encoding` `zenoh/bytes`. No rule at this version has an owner answer
in it: a failed call's `reply_err` carries the envelope above (§5.1, "The
forms").
- **The code and the cause** are numbers, in the order of the code table
  above and of `build`, `config` and `capability`.
- **The message** is optional, and at most 255 bytes of UTF-8.
- **The detail** is the bytes of a value of the operation's `error` type,
  encoded as that type says. A JSON Schema type's are JSON or CBOR, on
  the operation's `encoding`. A protobuf type's are its encoded message,
  and a raw type's are themselves, never base64.
- **Its refusals.** §7.5's parse refuses a cause or a detail where the
  envelope's decoding does, with the same tokens, `cause` and `detail`.
  No code is unknown: three bits hold eight.

`[F: errors/forms.json]`

**Transport errors.** Zenoh reports some failures locally, such as a query
timeout, as a reply error with encoding `zenoh/string`. A tool tells the two
apart by encoding: only the three encodings above carry an envelope, and
any other reply error is the transport's.
- **A router's own.** A router answers a query its timeout ends with a
  reply error of its own: the 7 bytes `Timeout`, under `zenoh/bytes`
  (Appendix B). It is the transport's by the rule above.
- **It is no error form either.** Its first byte, `T`, is `0x54`: a
  value's CH, which a `reply_err` does not carry (`sample_kind`, §7.5).
  `[F: errors/forms.json]`

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
- **No core roles, one mark.**
  - **A standby.** An instance **stands by** on an interface it implements
    when its descriptor marks the interface's entry `"standby": true`
    (§3.3). It exposes none of the interface's resources, required ones
    included (§8.2), declares none of its operation queryables,
    `unavailable` ones included (§5.1), and holds none of its interface or
    member tokens (§8.1). It holds its instance token and serves its
    descriptor, which lists the interface marked, so alive ⇒ callable
    holds, and a call finds no standby (O1, O3). It still holds the
    interface's bundle (§8.2 step 3, §8.4), which is not one of the
    interface's resources. `[Sc: profiles/redundancy/scenarios.md §1, §8]`
  - **When an instance stands by,** and when it serves, is a profile's:
    [`redundancy.v1`](profiles/redundancy/v1.md) for an interface whose
    contract uses it (§10). The core defines no election.
  - Two instances of one service holding an interface token for the same
    interface, for longer than a grace period, is a **finding**, except
    where replicated serving explains it. A tool diagnoses it,
    and the runtime does not fence, but for an interface whose contract
    uses `redundancy.v1`: its holders step down as that profile's §2.9
    says, which is no fence either, since writes in flight still
    land.
    - How a tool diagnoses it (**A holder is an incumbent, whatever its
      mark**, **Replicated serving**, **Longer than the grace period**, and
      the grace period a tool setting): [`observer.md`](observer.md) §6.1.
    - An owner SHOULD keep a re-mint's overlap below one second.

    `[Sc: operations.md §8]`
  - **Reading the mark**: [`observer.md`](observer.md) §6.2.
  - Election and fencing belong to `redundancy.v1`. An actuator keeps an
    exclusivity lock outside the bus, which the profile names
    `election = "external"` and does not replace (its §2.11).

---

## 7. Types and schemas

### 7.1 Type identity

Every resource payload, attachment, request, response, error and summary
has a **type identity**: (kind, name, sha256 of its schema artifact).

| Kind | Use |
|---|---|
| `protobuf` | proto2 and proto3. Editions are not supported. |
| `jsonschema` | The zk2 subset of JSON Schema 2020-12 (§7.3) |
| `raw` | Images, audio, opaque buffers |
| `packed` | Records of bit fields, sized from declared ranges and resolutions (§7.6) |

`flatbuffer`, `ros2msg` and `cdr` are reserved kinds. Authoring format
draft 4 has no spelling for them, so a contract declares such payloads as
`raw` with a media type. `[Sc: types.md §2]` What a tool that shows or
judges a payload decodes, these kinds through a plugin included:
[`observer.md`](observer.md) §7.3.

**Type references in a contract:**

| Spelling | Means |
|---|---|
| `"nav.v2.Position"` | a protobuf message, fully qualified, defined in a listed file. The well-known types of §9.4 are always available. |
| `"packed:Reading"` | `types/Reading` in a listed packed file. The name MUST be unique across the listed packed files (E024). `[F: contracts/e024-packed-ambiguous]` |
| `"packed:sensor#Reading"` | `types/Reading` in the listed packed file whose stem is `sensor` |
| `"json:Status"` | `$defs/Status`. The name MUST be unique across the listed JSON Schema files (E024). |
| `"json:telemetry#Point"` | `$defs/Point` in the listed file whose stem is `telemetry` |
| `{ raw = "image/jpeg" }` | opaque bytes of a media type |
| `{ raw = "video/*", media_param = "codec" }` | a raw family. The sample's `Encoding` carries the concrete subtype, and `media_param` names the single-chunk template parameter tied to it. |

A reference that does not resolve is an error. So is a `$ref` outside the
listed files.
`[F: contracts/e023-type, e024-ambiguous, e025-media-param, e029-schema, e032-ref-dangling, e032-ref-outside, ok-json-qualified, ok-protobuf-well-known]`

**Where a packed type goes.** A packed type MUST be the `type` of a
stream that is not explicit, a state or an event, or an operation's
`request`, `response`, `error` or `summary` (E044).
- **An operation's types** travel in §7.5's forms, whose CH carries the
  tag.
- **An explicit stream's type and an attachment** travel with no CH, so
  no reader could check the record's tag. A packed type is never one.

`[F: contracts/e044-packed-place, ok-packed, ok-packed-operation]`

### 7.2 Encodings and rendering

- **The sample's `Encoding`.** An owner MUST set Zenoh's predefined
  `Encoding` for every sample whose type has one (protobuf, JSON, CBOR, or
  a predefined media type), except a sample that carries §7.5's envelope
  (§4.3). It MUST NOT add a schema suffix to it, except on the error
  envelope (§5.2). `[Sc: types.md §1]`
  - **An envelope's sample** has the `Encoding` `zenoh/bytes`, and its
    record is encoded as its type says, on the contract's `encoding`
    (below).
  - **A raw media type** that Zenoh does not predefine travels, outside an
    envelope, as Zenoh's custom encoding, the media type in the suffix, as
    Zenoh does by itself. A suffix is sent with every put, and is capped at
    255 bytes.
- **JSON Schema types** are encoded as JSON (the default) or CBOR, declared
  by `encoding`. Bytes are CBOR byte strings, or base64 in JSON. An
  attachment declares its own `attachment_encoding` by the same rule.
  Protobuf is always binary, and `encoding` on a protobuf, packed or raw
  type is ignored, with a warning. `[F: contracts/w103-encoding]`
- **A packed record** always travels in an envelope, under the
  `Encoding` `zenoh/bytes`, which costs no byte (§7.5, §7.6).
- **Decode order** for a tool, and **Honest rendering**: [`observer.md`](observer.md) §7.3.

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
  - **"Exactly `null`"** reads the `type` as a set of names,
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
- A tool's check of the limit at start: [`observer.md`](observer.md) §7.4.

### 7.5 The zk2c envelope

The **zk2c envelope** carries an owner's run and sequence number with a
sample, before its record. This section defines its bytes. §4.3 adopts
it: which samples carry one, what a run names and how a seq counts. §7.2
says the `Encoding` of a sample that carries one.

An operation's call has **forms** of the same shape: its request, its
value and its error. This section defines their bytes too. §5.1 says what
a value's and an error's run and seq name, and §5.2 what an error holds.
No rule at this version has a caller or an owner write them (§5).

**The layouts.** A put carries an envelope as its payload, and a delete as
its attachment. A call's query, reply and reply error carry a form as
their payload:

```text
mutation, a put's payload            CH · run · seq · record
delete, a delete's attachment        CH · run · seq
heartbeat, a put's payload           CH · run · seq · body
frame, a put's payload               CH · run · seq · body
request, a query's payload           CH · record
value, a reply's payload             CH · run · seq · record
error, a reply error's payload       CH · run · seq · error
```

**The header byte, CH,** holds three fields. Its bits are numbered from 7,
the most significant, down to 0:

| Bits | Field | Value |
|---|---|---|
| 7–5 | form | the form's code (below) |
| 4 | run width | 0 for a 16-bit run, 1 for a 32-bit run |
| 3–0 | tag | 0, unless the record's type kind defines a tag |

So CH is the form's code × 32, plus the width bit × 16, plus the tag. With
tag 0, a mutation's CH is `0x00` with a 16-bit run and `0x10` with a 32-bit
run. A delete's is `0x20` or `0x30`. And `0x1b` is `000 1 1011`: form 0, a
32-bit run and tag 11. Only the `packed` kind defines a tag (§7.6). A
delete carries no record, so its tag is 0. A heartbeat's tag names a
contract revision (below). A frame's tag is its declaration's (§7.7).
- **A request's** tag is its operation's `request` type's. Its width bit
  is 0, since it carries no run.
- **A value's** is the operation's `response` type's, or its `summary`
  type's on a summary reply (O6).
- **An error's** is the operation's `error` type's, whether a detail
  follows or not. It is 0 for an operation that declares none.

**The forms.** Each form has one code. This version defines six of
them, and reserves the other two for later versions:

| Code | Form | What it carries |
|---|---|---|
| 0 | mutation | a put's run and seq, then its record |
| 1 | delete | a delete's run and seq, as its attachment, and no record |
| 2 | value | a call's request, as its query's payload; and a reply's run and seq, then its record |
| 3 | error | a reply error's run and seq, then §5.2's error in bytes (below) |
| 4 | event | reserved: an event's occurrence |
| 5 | frame | a stream's records under one header, a put's payload (§7.7) |
| 6 | heartbeat | an owner's confirmation of the state it holds for one interface (below) |
| 7 | snapshot | reserved: a collection's state in one reply |

A value carried by a query is a **request**. It has no run and no seq.

**The run and the seq.** The run is an unsigned integer of 16 or 32 bits,
as CH's width bit says. The seq is an unsigned 24-bit integer, from 0 to
16,777,215. Both are written big-endian, the most significant byte first.
Together they are the envelope's stamp, the sample's in-band stamp.
This section fixes their widths and their byte order. What a run names,
and how a seq counts, are §4.3's. A value's and an error's are the
replying instance's as-of mark, which §5.1 defines.

**The record** is every byte after a mutation's seq, possibly none. It
is read as the resource's type says (§7.1). A heartbeat and a frame carry
a **body** there instead, never a record of the resource's type. A
heartbeat's is read with its covered revision's collections (below). A
frame's is read with its declaration (§7.7). A sample that carries
an envelope has the Encoding `zenoh/bytes`, whatever its record's type:
the contract names the type.
- **A request's record** is every byte after its CH, possibly none. It is
  read as the operation's `request` type says.
- **A value's** is every byte after its seq, possibly none. It is read as
  the `response` type says, or the `summary` type on a summary reply.
- **A form's sample** has the Encoding `zenoh/bytes` too: a query, a
  reply and a reply error alike.

**After an error's seq** come one byte, then a message and a detail,
each where the byte says so:

```text
error      byte · message, when its bit is 1 · detail, when its bit is 1
message    its length in bytes, from 0 to 255, in one byte · that many bytes of UTF-8
detail     every byte after the message, or after the byte without one
```

The byte holds five fields, its bits numbered as CH's are:

| Bits | Field | Value |
|---|---|---|
| 7–5 | code | §5.2's code, numbered in its table's order: 0 `invalid_request`, 1 `not_found`, 2 `unavailable`, 3 `forbidden`, 4 `fanout_forbidden`, 5 `busy`, 6 `internal`, 7 `app` |
| 4–3 | cause | 0 for none, 1 `build`, 2 `config`, 3 `capability` |
| 2 | detail | 1 when a detail follows |
| 1 | message | 1 when a message follows |
| 0 | extension | 0. A later version may define 1. |

So `0xa0` is `busy`, with no cause, detail or message. `0x50` is
`unavailable` for `config`, and `0xe4` is `app` with a detail.
- **The message** is for a human, never parsed (§5.2). An empty one has
  its bit set and length 0, and reads back empty, not absent.
- **The detail** is a value of the operation's `error` type, encoded as
  that type says. It can be empty, as a protobuf message with no field
  written is.
- **Writing a message.** An implementation that writes an error MUST cut
  a message longer than 255 bytes to its longest prefix of whole
  characters within 255 bytes. `[F: errors/forms.json]`
- **Writing what the parse refuses.** An implementation MUST NOT write
  an error with a cause on a code other than `unavailable`, `unavailable`
  without one, or a detail on a code other than `app`.
  `[F: errors/forms.json]`

**Sizes.** A mutation is 6 bytes over its record, or 8 with a 32-bit run.
So is a heartbeat over its body, and a frame over its body.
A delete's attachment is 6 or 8 bytes, and an attachment costs 2 more on
the wire. Beside them, a Zenoh timestamp is 26 bytes. A predefined
Encoding is 1 byte, and `zenoh/bytes` costs none (Appendix B).
- **A request** is 1 byte over its record, and a value 6 or 8.
- **An error** is 7 bytes, or 9 with a 32-bit run. A message adds its
  length and a byte, and a detail its own bytes.

**Why the payload.** Zenoh gives a sample one attachment.
[`telemetry.v1`](profiles/telemetry/v1.md)'s Epoch, a contract's declared
`attachment` (§2.3) and an archive's replies (§4.4) already use it. So a
put's envelope leads its payload. A delete has no payload, so its envelope
is its attachment. It keeps CH, which states the run's width and lets a
reader refuse an attachment that is not an envelope.

**Parsing.** An envelope or a form is parsed without the contract, as a
key is (§1.1). The parser knows what carries the bytes, its **sample**:
- a put's payload, or the payload of a GET's reply that represents a put
  (S2);
- a delete's attachment, or a `reply_del`'s (S2);
- a call's query, its reply or its `reply_err`, each one's payload.

For a frame it also knows its key's kind token, which the grammar reads
from the key alone. It knows nothing more. So it reports the tag of every
form but a delete, and checks none: §7.6's reader, a heartbeat's reader
and §7.7's do. Of an error, it reads the byte and the message, and
takes the detail's bytes unread. An
implementation MUST refuse an envelope or a form for the first of these
that applies, in this order:
- no byte at all, so no CH (`short`);
- a reserved form, 4 or 7 (`form`);
- a form its sample does not carry (`sample_kind`). A put carries a
  mutation, a heartbeat or a frame, and a delete's attachment a delete.
  A query and a reply carry a value, and a `reply_err` an error;
- a frame on a key whose kind token is not `stream`, such as a state's,
  an event's or an `@stream` key (`key_kind`);
- a delete whose tag is not 0 (`tag`);
- a request whose width bit is 1 (`request_run`);
- fewer bytes than CH, the run and the seq need, and an error's byte
  after them (`short`);
- a byte after a delete's seq (`trailing`);
- then, reading an error's byte and what follows it:
  - a cause on a code other than `unavailable`, or none on `unavailable`
    (`cause`);
  - a detail on a code other than `app` (`detail`);
  - the extension bit set (`extension`);
  - a message cut short: no length byte, or fewer bytes than it states
    (`short`);
  - a message that is not UTF-8, as RFC 3629 defines it (`utf8`);
  - a byte after the message, or after the error's byte without one,
    where no detail follows (`trailing`).

`[F: envelope/vectors.json, carries.json]`

Every envelope and form an implementation accepts MUST write back to the
same bytes. `[F: envelope/vectors.json]`

**The heartbeat.** A **heartbeat** is a put an owner sends for one
interface it serves, its **covered interface**. It vouches for the members of that interface's
state that are still current, without putting their values again. This
section defines its bytes and how they are read.

- **CH's tag** is the low 4 bits of the last byte of the covered
  revision's fingerprint (§9.5). An implementation that writes a
  heartbeat MUST write its covered revision's tag in CH.
  `[F: heartbeat/layouts.json]`
- **The tag is a check.** A reader compares it with the revision it
  holds, as §7.6's reader compares a type's tag. Another revision still
  matches it 1 time in 16.
- **Its run and seq** are its key's. They count as any stream key's do
  (§4.3), so a gap in the seqs is a heartbeat lost.
- **Who sends one** is [`freshness.v2`](profiles/freshness/v2.md)'s to
  say: its key, its period, and what a reader concludes from it.

**The collections.** A heartbeat's body is read by its covered
revision's collections.
- **A collection** is a `state` or `@state` resource whose resolved
  `freshness.ttl_s` is above 0 ([`freshness.v1`](profiles/freshness/v1.md)
  §2.1).
- **Their order** is the canonical form's resource order: by kind token,
  then template (§9.5).
- **A collection is templated** when its template has a parameter. Its
  cardinality c then bounds its live members (§2.2). A collection whose
  template has no parameter has one member.
- **A live member** is one whose last mutation is a put. Its **stamp** is
  that put's run and seq.
- **Key order** is the byte order of the live members' base-relative
  keys (§1.1). A member's **rank** is its place in key order, from 0.

`[F: heartbeat/layouts.json]`

**The body** is a run of bit fields, most significant bit first, with
no padding between them, as §7.6's records are:

```text
body       time · map · a section for each map bit set, in map order · pad
time       42 bits: the owner's clock when it wrote the heartbeat
map        1 bit for each collection, in order: 1 when its section follows
section    digest · members
digest     32 bits: the collection's digest (below)
members    a collection of one member:  vouched, 1 bit
           a templated collection:      live · sel · (bitmap | count · ranks)
live       the live members, in ⌈log₂(c + 1)⌉ bits
sel        1 bit: 0 for a bitmap, 1 for a list
bitmap     1 bit for each live member, in key order: 1 when vouched for
count      the members vouched for, in ⌈log₂(live + 1)⌉ bits
ranks      their ranks, ascending, each in ⌈log₂ live⌉ bits
pad        zero bits up to the next whole byte, fewer than 8
```

- **The time** is §7.6's `time` field with `ms`. It counts 32 bits of
  seconds from the deployment epoch, then 10 bits of milliseconds.
- **Unknown.** All 42 bits set say the owner's clock is unknown. Any
  other millisecond code from 1000 up is unused.
- **A map bit of 0** says nothing of its collection. Its section is left
  out.
- **Vouched.** A bit set vouches for its member: the owner holds the
  member's value as current. A bit clear vouches for nothing. A member
  that is not live is never vouched for.
- **The list or the bitmap.** An implementation that writes a templated
  section MUST write the list only where it is shorter than the bitmap.
  `[F: heartbeat/bodies.json]`

**The digest** of a collection is the XOR of one 32-bit value for each
of its live members. With none live it is 0. A member's value is the
first 4 bytes, read big-endian, of the sha256 of these bytes:
- its base-relative key, in UTF-8;
- a zero byte;
- a byte holding its run's width bit, 0 for a 16-bit run and 1 for a
  32-bit run;
- its run, then its seq, big-endian, as an envelope writes them.

An implementation MUST compute a digest so. `[F: heartbeat/digests.json]`
- **Why the XOR.** It does not depend on order. So an owner updates it at
  each mutation, and a reader computes it over the members it holds.
- **What it catches.** A reader that missed a put or a delete, or holds
  another run's value, holds another digest. Two views that differ still
  share a digest 1 time in 2^32.

**Reading a body.** A body is read with the covered revision's
collections. An implementation MUST refuse a body for the first of these
that applies, in this order, naming the refusal:
- reading the fields in order:
  - the body ends before a bit its fields need (`short`);
  - a code is unused (`code`): a millisecond code from 1000 up but the
    unknown time's, a live count above c, a count above the live count, or
    a rank not below the live count or not above the rank before it;
- the body holds a byte after its pad (`trailing`);
- a bit of the pad is 1 (`pad`).

`[F: heartbeat/bodies.json]`

Every body an implementation accepts MUST write back to the same bytes.
`[F: heartbeat/bodies.json]`

### 7.6 Packed records

A **packed** type's record is a run of bit fields, each as wide as its
declared codes need. So a record carries little more than its
information. A packed record always travels in §7.5's envelope, and its
type defines CH's tag.

**The schema file.** A packed type is declared in a file that
`[schemas] packed` lists (§9.1, §9.4). The file is strict JSON.
- **Its top level** is an object holding `types`, and optionally
  `description`. `types` maps each type's name to its record.
- **A record** is an object holding `fields`, a list, and optionally
  `description`. Each item of `fields` is a field, or the extension
  marker, the string `"..."`.
- **A field** is an object holding `name` and `type`, and optionally
  `optional` (a boolean, `false` when absent) and `description`. Its
  `type` names one of the field types below, and the other members it
  holds are that type's.
- **Names.** A type's name is `[A-Z][A-Za-z0-9]*`, a field's
  `[a-z][a-z0-9_]*`, and an enum's value `[A-Za-z][A-Za-z0-9_]*`.

Anything else in the file is E038.
`[F: contracts/e038-packed-shape, e043-packed-type]`

**Field types.** n is an enum's number of values, and k the number of
reserved codes a field declares. ⌈log₂ c⌉ is the fewest bits that hold c
codes.

| `type` | Members | Width in bits |
|---|---|---|
| `int` | `minimum`, `maximum`: integers; `reserved` | ⌈log₂(`maximum` − `minimum` + 1 + k)⌉ |
| `fixed` | `minimum`, `maximum`, `resolution`: decimal strings; `reserved` | ⌈log₂((`maximum` − `minimum`) / `resolution` + 1 + k)⌉ |
| `enum` | `values`: names, in order; `escape`: a boolean; `width` | `width`, else ⌈log₂ n⌉, or ⌈log₂(n + 1)⌉ with `escape` |
| `bool` | none | 1 |
| `counter` | `width` | `width` |
| `time` | `ms`: a boolean | 32, or 42 with `ms` |

- **Bounds.** A range is never empty, and neither is an enum or a
  record (E039). A width is from 1 to 64 bits, and an enum's `width`
  holds its codes (E041). A name is never repeated: a field's in its
  record, a value in its enum, a reserved code in its list (E042).
  `[F: contracts/e039-packed-empty, e041-packed-width, e042-packed-duplicate]`
- **A decimal string** is `-?(0|[1-9][0-9]*)(\.[0-9]+)?`, read as the
  exact decimal it writes. A `resolution` is above 0, and divides
  `maximum` − `minimum` (E040). `[F: contracts/e040-packed-fixed]`
- **An integer** in the file, an `int`'s bound or a `width`, is a JSON
  number written with no fraction and no exponent. So `5.0` there is
  E038, as a float is in §9.1's TOML. `[F: contracts/e038-packed-shape]`
- **Its shortest form** has no zero at the end of a fraction, no point
  with no digit after it, and is `0` for zero.

**Codes.** Each field is written as an unsigned integer, its code, in its
width.
- **`int`:** a value v is written as v − `minimum`.
- **`fixed`:** a value x is written as round((x − `minimum`) /
  `resolution`), rounding a tie to the even integer. The code reads back
  as `minimum` + code × `resolution`. So a value read is within half a
  `resolution` of the value written: that bound is part of the
  declaration.
- **A fixed-point value's range** is its codes'. A value has a code when
  its rounded code is from 0 to (`maximum` − `minimum`) / `resolution`.
  So a value within half a `resolution` past a bound is written. Exactly
  half below `minimum` rounds to code 0. Exactly half above `maximum`
  rounds to the even one of the top code and the next, so it has a code
  only when the top code is even. `[F: packed/vectors.json]`
- **Exact arithmetic.** An implementation MUST compute a fixed-point
  code, and the value it reads back, in exact decimal arithmetic, never
  in binary floating point. `[F: packed/vectors.json]`
- **Reserved codes.** `reserved` lists some of `not_available`, `error`
  and `out_of_range`. They take the top codes of the width, from all ones
  down, in that order. With all three, `not_available` is all ones,
  `error` the code below it and `out_of_range` the one below that.
- **Who writes `out_of_range`.** A value outside the range has no code,
  and an implementation refuses it (below). The application that holds
  such a value gives the name `out_of_range` instead, where the field
  declares it. An implementation never puts the code in a value's place.
- **`enum`:** the value at index i of `values` is written as i. With
  `escape`, the all-ones code is the escape, a value this revision does
  not name. A reader reads every code from n up as the escape, so a later
  revision can name more values within the width.
- **`bool`:** 1 is true and 0 is false.
- **`counter`:** a count c is an integer from 0 to 2^64 − 1, written as
  c modulo 2^`width`. A count outside it has no code. A width below 64 is
  a declared loss. A reader of a counter that wraps cannot
  recover its total, and more than one wrap between two readings goes
  unseen.
- **`time`:** an instant t, in milliseconds since the Unix epoch, is
  written as ⌊(t − 1000 × E) / 1000⌋, the seconds since the deployment
  epoch E, in 32 bits. With `ms`, (t − 1000 × E) modulo 1000 follows in 10
  bits. Without it, the milliseconds are dropped.
- **The deployment epoch** E is a whole number of seconds since the Unix
  epoch. Writer and reader each hold it as an input: the epoch of the
  catalog in force (§8.6). Where none is, E is 0, the Unix epoch, and a
  `time` field's 32 bits are Unix seconds, to 2106.
- **Unused codes.** A code that is no value and no declared reserved code
  is unused. So is a millisecond code from 1000 to 1023. An enum with an
  escape has none.

**The layout.** Fields are written in `fields` order, most significant
bit first, with no padding between them.
- **A field** is its code, in its width.
- **An optional field** starts with a presence bit. It is 1, followed by
  the code, when the field is present, and 0 alone when it is absent.
- **The extension marker** is one bit. It is 1 when a field after it is
  present, and the record goes on. It is 0 when none is, and it ends the
  record.
- **After a marker,** every field is optional (E045).
  `[F: contracts/e045-packed-extension]`
- **The pad** follows the record's last bit. It is the zero bits up to
  the next whole byte, fewer than 8.

So the record's length follows from the layout and its presence and
extension bits. A type with no optional field and no marker has one
length.

**Growth.** A later revision appends optional fields after its
predecessor's last marker, and a marker of its own when it may grow
again. A reader of the earlier revision reads its own fields, meets that
marker as 1, and ignores what follows. A reader of the later revision
reads an earlier writer's marker as 0, and its appended fields as absent.
§9.8 classifies the change.

**The tag.** A type's **layout form** describes its root's wire: the
fields before its first marker, and that marker. It is a JSON array of
each root field's form, then `"..."` where the root ends at a marker. A
field's form is an object with `type`, `width` and `optional`, and:
- for `int`, `minimum`, `maximum` and `reserved`;
- for `fixed`, `minimum`, `maximum` and `resolution`, each a decimal
  string in its shortest form, and `reserved`;
- for `enum`, `escape`, and `values`, the number of values, when there is
  no escape;
- for `time`, `ms`.

`reserved` is a list of the declared names, in the order above. Names and
descriptions are left out, so a field renamed keeps its tag. The tag is
the low four bits of the last byte of the sha256 of the form's JCS bytes
(RFC 8785). An implementation that writes a record MUST write the type's
tag in CH's tag bits (§7.5). `[F: packed/vectors.json]`

**Writing.** An implementation that writes a record MUST write the bits
above for its value, then the pad. `[F: packed/vectors.json]` An
implementation MUST NOT write a record for a value that has no code: a value outside its range, a name that is not declared,
or an instant outside the epoch's 2^32 seconds. `[F: packed/vectors.json]`
An implementation MUST NOT write a record for an object that holds a
member its type does not have, or lacks a field that is not optional.
`[F: packed/vectors.json]`

**Reading.** An implementation that reads a record of a packed type MUST
refuse it, naming the refusal, for the first of these that applies, in
this order:
- CH's tag is not the type's (`tag`).
- Then, reading the fields in order:
  - the record ends before a bit the layout needs (`length`);
  - a field's code is unused (`code`).
- A marker read as 1 is followed by no present field (`extension`).
- The record holds a byte after its pad (`length`).
- A bit of the pad is 1 (`pad`).

`[F: packed/vectors.json]`
- **A later revision's fields.** Where an implementation's layout ends
  with a marker read as 1, the rest of the record is a later revision's,
  which it ignores. That rest MUST hold at least one bit (`length`). The
  `extension`, `length` and `pad` checks after the fields are then not
  made.
  `[F: packed/vectors.json]`

**What the checks catch.** A reader that holds another revision of a
type, or another type, still matches its tag 1 time in 16.
- **The length** refuses most of those. A type with no optional field and
  no marker has one length, so a record of another length is refused.
- **Optional fields and markers** weaken it. The record's length then
  depends on its presence bits, which the reader reads by its own
  layout, so a record of the wrong revision can have a length it
  accepts.
- **A marker read as 1** at the end of the reader's layout leaves only a
  lower bound on the length.
- **Unused codes and the pad** refuse some more.
- **Where more is needed,** a reader takes the fingerprint from the
  owner's descriptor (§3.3), never from the record.

**The JSON reading.** A tool shows a record as a JSON object, with a
member for each field present:
- `int`: an integer;
- `fixed`: a string holding `minimum` + code × `resolution`, in its
  shortest form;
- a reserved code of either: its name, a string;
- `enum`: the value's name, or `null` for the escape;
- `bool`: `true` or `false`;
- `counter`: an integer;
- `time`: an integer, milliseconds since the Unix epoch.

An absent optional field has no member, and neither has a field after a
marker read as 0. `[F: packed/vectors.json]`

**What a writer takes** is the same object, and `null` for an escape.
- **A fixed-point value** is a string or a JSON number. A string is a
  JSON number's text (RFC 8259 §6). So `5e-1` reads as `0.5`, and `+0.5`,
  `.5`, `1.` and `05` are no value.
- **A JSON number** with no fraction and no exponent, from −2^63 to
  2^64 − 1, is read as that integer, exactly. Any other is read as the
  binary64 a JSON reader holds for it, by that double's shortest decimal
  text. So `0.15` is the decimal `0.15`. A value with more digits than a
  double keeps is given as a string.
- **An `int`, `counter` or `time` value** is a JSON number written with
  no fraction and no exponent. So `5.0` is a value of the wrong type, and
  a 64-bit counter takes 18446744073709551615.
- **A binary64 of magnitude 2^63 or more** given for one of them is out
  of range, not of the wrong type. A JSON reader with no big integers
  holds integer text past 64 bits as such a double, so the two cannot be
  told apart.

`[F: packed/vectors.json]`

### 7.7 Frames

A **frame** puts several records of one packed type (§7.6) under one
envelope (§7.5), with one time for all of them. A **frame resource** is a
plain `stream` whose contract declares a `frame` table (§9.1). Its type is
a packed type with no extension marker, and that type's fields are the
frame's **fields**. A record holds every field, or only the fields that
changed. A frame travels only as a put's payload, on its resource's key.

**The declaration.** The `frame` table holds these keys. All but
`deadband` are integers.

| Key | Value | When absent |
|---|---|---|
| `records` | the most records one frame holds, from 1 to 256 | 1 |
| `step_ms` | from 1 to 2^32 − 1: record i is scheduled `step_ms` × i milliseconds after record 0. Given exactly when `records` is above 1. | no step |
| `jitter_ms` | from 0 to `step_ms` / 2: how far a scheduled record may lie from its scheduled instant. Given only with `step_ms`. | 0, with a step |
| `keyframe_every` | from 1 to 65,536: a full record comes at least once every this many records | 1 |
| `deadband` | an object: a field's name → `{delta, max_records}` (below) | none |

- **A one-record frame** has no step. Its one time is measured, and how
  often its owner puts one is the owner's.
- **The span** of a frame with a step is `records` × `step_ms`. Every
  record of the frame lies within it.
- **The interval** of a frame resource is its span where it has a step.
  Otherwise it is the `timing.period_ms` of a declaration that
  [`timing.v1`](profiles/timing/v1.md) §2.2 reads valid (§2.3). With none,
  it is unstated.
- **A deadband** is a field's `delta` and `max_records`. `delta` is a
  decimal string (§7.6) above 0, in the field's own units: a value, not a
  code. `max_records` is from 1 to 65,536. Only an `int`, `fixed` or
  `counter` field takes one.
- **No parameter.** A frame resource's template has none, and its type's
  fields are concrete.

§9.2's E046 to E050 lint the declaration.
`[F: contracts/e046-frame-place, e047-frame-type, e048-frame-value, e049-frame-deadband, e050-frame-timing, ok-frame]`

**The body.** A frame's body follows its seq. It is a run of bit fields,
most significant bit first, with no padding between them, as §7.6's
records are:

```text
frame, a put's payload   CH · run · seq · body
body       time · count · record · … · record · pad
time       42 bits: record 0's measured time; all ones when unknown
count      the records in the frame, less 1, in ⌈log₂ records⌉ bits
record     full · when · offset · fields
full       1 bit: 1 for a full record, 0 for a delta record
when       1 bit, in record 1 and later only: 1 when the record is scheduled
offset     when `when` is 0: the record's time, in milliseconds after
           record 0's, in ⌈log₂ span⌉ bits
fields     a full record: every field, as §7.6 writes it
           a delta record: one bit per field, in field order, 1 when the
           field follows; then each field whose bit is 1, as §7.6 writes it
pad        zero bits up to the next whole byte, fewer than 8
```

- **Count.** A frame holds count + 1 records. With `records` = 1 the
  count takes no bit, and the frame holds one record.
- **A field as §7.6 writes it** is its code, after a presence bit when
  the field is optional. So a delta record can carry a field that became
  absent: its bit 1, then its presence bit 0.
- **The time** is §7.6's `time` field with `ms`: 32 bits of seconds from
  the deployment epoch, then 10 bits of milliseconds. All 42 bits set say
  the owner's clock is unknown. Any other millisecond code from 1000 up is
  unused.
- **A record's time.** Record 0's is `time`. A later record's is an
  offset from it, so it is known when `time` is not:
  - **scheduled** (`when` 1): `step_ms` × i after record 0, within
    `jitter_ms` either way;
  - **measured** (`when` 0): its offset after record 0.
- **What the time is not.** A scheduled record's time is an instant and
  a bound, never a measurement.
- **Its seq.** The envelope's seq is record 0's. Record i's is that seq
  plus i, modulo 2^24, so a frame of n records takes n seqs of its key
  (§4.3).

**The frame's tag.** A frame's **frame form** is a JSON object of two
members:
- `frame`: an object of `records`, `step_ms` and `jitter_ms`, the last
  two `null` without a step, and `deadband`, a list with one item per
  field in field order. An item is `null`, or `{"delta", "max_records"}`
  with `delta` in its shortest form (§7.6).
- `record`: the record type's layout form (§7.6).

Names are left out, so a field renamed keeps the tag. `keyframe_every`
is left out too: it changes no bit a reader reads. The tag is the low
four bits of the last byte of the sha256 of the frame form's JCS bytes
(RFC 8785), as a layout form's is. So a frame and a mutation of the same
type carry different tags but 1 time in 16. An implementation that
writes a frame MUST write its tag in CH's tag bits (§7.5).
`[F: frame/layouts.json]`

**Writing.** An implementation that writes a frame MUST write the bits
above for its records, then the pad. `[F: frame/bodies.json]`
- **What it never writes.** An implementation MUST NOT write a frame of no
  record, or of more than `records`. An implementation MUST NOT write a
  full record that lacks a field that is not optional, or a field §7.6
  refuses.
  `[F: frame/bodies.json]`
- **Times.** An implementation MUST NOT write an offset at or past the
  span, or a record that lies before the record before it (below).
  `[F: frame/bodies.json]`

**Reading.** A body is read with its frame resource's declaration. An
implementation that reads a frame MUST refuse it for the first of these
that applies, in this order, naming the refusal:
- CH's tag is not the frame's (`tag`);
- then, reading the fields in order:
  - the body ends before a bit its fields need (`length`);
  - a code is unused (`code`): a millisecond code from 1000 up but the
    unknown time's, a count of `records` or more, an offset at or past
    the span, or a field's unused code (§7.6);
  - a record lies before the record before it (`order`): its latest
    time is below that record's earliest. A scheduled record lies from
    `jitter_ms` before its instant to `jitter_ms` after it, and any other
    record at its time.
- the body holds a byte after its pad (`length`);
- a bit of the pad is 1 (`pad`).

`[F: frame/bodies.json]`

Every body an implementation accepts MUST write back to the same bytes.
`[F: frame/bodies.json]`

**No reading.** A frame on a resource that declares no `frame` table,
and a mutation on a frame resource, have no reading. A reader counts
either as undecodable.

**What the checks catch.** A reader that holds another declaration of the
frame, or another record type, still matches its tag 1 time in 16. A
body's length then depends on its full and changed bits, so the length
check catches less than it does for a record of one length. Where more is
needed, a reader takes the fingerprint from the owner's descriptor (§3.3).

**The owner's frames.** An owner writes a frame resource's samples as
frames, each of 1 to `records` records, in the order it measured them.
- **Never a mutation.** An owner MUST NOT put a mutation (§7.5) on a frame
  resource's key: a reader has no reading of one (below).
  `[Sc: frames.md §1]`
- **Seqs.** An owner MUST give a frame the seq of its record 0, and the
  frame after it the seq that follows its last record's.
  `[Sc: frames.md §1]`
- **Full records.** An owner MUST start a key's first frame in a run with
  a full record. An owner MUST write a full record at least once every
  `keyframe_every` records. A **keyframe** is a frame whose record 0 is
  full, and it decodes alone. `[Sc: frames.md §1, §5]`
- **Delta records.** A delta record is read against the record before it:
  the frame's previous record, or the previous frame's last. In a delta
  record, an owner MUST include each field with no deadband whose code or
  presence differs from that record's. Including an unchanged field is
  never wrong. `[Sc: frames.md §1]`
- **The deadband.** An owner MAY omit a field with a deadband while three
  things hold: the field is present and holds a value, not a reserved
  code; so did the last record that included it; and the two values
  differ by at most `delta`. An owner MUST include the field in any other
  case, and in at least one of every `max_records` records. `[Sc: frames.md §4]`
  So a deadband field that is absent, or holds a reserved code, is in
  every record.
- **Times.** An owner writes record 0's time from its wall clock, or all
  ones while its clock is unknown. It measures a later record's offset on
  its monotonic clock, from record 0. An owner MUST mark a record
  scheduled only when it measured it within `jitter_ms` of its scheduled
  instant.
  `[Sc: frames.md §3]`
- **The span.** A record that would lie at or past its frame's span ends
  the frame, and starts the next one as its record 0. An owner MAY end a
  frame before it holds `records` records, and puts it then.
  `[Sc: frames.md §3]`

**The reader's fields.** A **reader** of a frame key is a consumer or a
tool. It keeps one state for each field of the key, in the run it holds:

| State | When | What the field holds |
|---|---|---|
| `known` | included in this record; or omitted, with no deadband, nothing lost since it was included | its value, at the record's time |
| `within` | omitted under a deadband, fewer than `max_records` records after a record that included a value | its last value, give or take `delta` |
| `unknown` | after a record lost, at a run it does not hold, after a rewind, `max_records` records after it was included, or omitted under a deadband after an inclusion that was no value | nothing |
| `absent` | included absent: its presence bit 0 | nothing: the owner wrote it absent |

- **The frames it expects.** Within the run it holds, a reader expects
  the seq after the last record it read. A frame at that seq continues
  the key. One whose seq is newer (§4.3's serial order) follows a
  **gap**: records lost. Any other is a **rewind**: the key restarted
  under a run it had used, which §4.3 forbids.
- **What resets.** A consumer or a tool MUST turn every field `unknown`
  at a gap, at a rewind, and at a frame of a run other than the one it
  holds, its first frame included. It then reads the frame, whose
  included fields are `known` again, or `absent`.
  `[F: frame/apply.json]` `[Sc: frames.md §2, §5, §6]`
- **Past the deadband.** A consumer or a tool MUST turn a field with a
  deadband `unknown` at the `max_records`-th record after the one that
  included it, when that record omits it too.
  `[F: frame/apply.json]` `[Sc: frames.md §4]`
- **Never a value.** A consumer or a tool MUST NOT show or use an
  `unknown` or `absent` field as a value, its last value or 0 among
  them. `[F: frame/apply.json]` `[Sc: frames.md §2, §6]`

So a lost record costs the fields its successors omit, until each is
included again, and a keyframe heals them all. zenoh keeps one key's puts
in order on one priority, so a rewind is never a reorder on one path.

**History.** A frame resource MAY declare `history` (§2.5). A late joiner
then reads the frames its owner's cache holds, and the last keyframe among
them heals its fields at once.
- **In order.** A late joiner hands its reader the cache's frames before
  the frames it hears live, in their seqs' order (§4.3), and never a frame
  older than one the reader read. The reader would take that frame for a
  rewind (above), and turn every field `unknown`.
- **Across a face** the `@adv` deny keeps the cache home
  ([`link.v1`](profiles/link/v1.md) §2.6). A far late joiner heals at the
  next full record alone, and every put still carries the advanced
  publisher's source info across the face, which `link.v1` warns of
  (`history_on_frame`, §2.8).

**Sizes.** A frame is 6 bytes over its body, or 8 with a 32-bit run. A
keyframe of one record of 63 bits is 42 + 1 + 63 = 106 bits, so its body
is 14 bytes, and the put's payload 20.

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
  A standby (§6) exposes none of the interface's resources, so it
  holds no interface token for it, and no member token.
  `[Sc: profiles/redundancy/scenarios.md §1]`
- **The tokenless set** (U22, §13).
  - **At scale.** A deployment needs this only once a presence domain
    nears the presence budget, about 10–15k tokens (§8.3); below that, it
    leaves the set empty, and every interface keeps its token.
  - A deployment MAY configure an owner with interfaces for which it holds
    no interface token. These SHOULD be the interfaces every service of the
    deployment implements: `health.v1`
    ([`profiles/health/v1.md`](profiles/health/v1.md) §2.7), `alarms.v1`
    ([`profiles/alarms/v1.md`](profiles/alarms/v1.md) §2.13),
    `freshness.v2` ([`profiles/freshness/v2.md`](profiles/freshness/v2.md)
    §3) and the rest of a framework set.
  - The owner's descriptor marks each such interface `"token": false`
    (§3.3).
  - It never names `archive.v1` (§4.4), nor an interface
    whose contract uses `redundancy.v1`: the instance serving it is known
    by its interface token (§6, [`redundancy.v1`](profiles/redundancy/v1.md)
    §2.14). §8.2 step 2 refuses both.
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
  - **At scale.** A deployment needs this only once a contract declares
    `epoch`, as one service serving its devices, containers or streams as
    members does. That is the shape §1.5 gives above about 5,000 devices
    per presence domain. Below that, a service per device keeps each
    one's continuity in its own instance id (§1.5), and an owner whose
    contract declares no `epoch` holds no member token.
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
- **Reading presence**: [`observer.md`](observer.md) §3. Its parts keep their names there:
  - **The subscribers bind too:** observer §3.1;
  - **A fault read from presence shapes holds in two reads**: observer §3.2;
  - a liveliness GET that ended at its timeout, and **A read that runs to
    its session's interests timeout is not complete**: observer §3.3;
  - **A refused read is complete, and empty**: observer §3.4;
  - **A read through a session cut off is not complete**, with
    **What the check reads**, **A read cut off while it runs** and
    **A profile's premise**: observer §3.5;
  - **Absence is established only by a complete read**, with
    **Acting on a withdrawal**: observer §3.6;
  - **The read that establishes absence goes on a session with no
    liveliness subscriber**, the watch session and its round trip:
    observer §3.7;
  - **A read on a token's own session can answer that token itself**:
    observer §3.8;
  - **What a complete read covers**: observer §3.9.
- **Timeouts are the caller's.** How long a liveliness GET, a descriptor
  GET (§3.3) or a retrieval attempt (§8.4) waits, and how long a tool waits
  for presence after an owner starts, are the caller's choices. The
  scenarios, and so a conformance run, use 1 s unless they say otherwise.
  S6's "the GET's timeout" is that choice.
  - **Across a constrained face.** A caller, a consumer or a tool across a
    constrained face whose face policy states a reply timeout
    ([`link.v2`](profiles/link/v2.md) §2.1's `reply_timeout_ms`) SHOULD
    wait at least that long for each reply it asks for: a call's, a GET's
    and a seed's. `[Sc: constrained.md §8]`
    - **Why the policy says it.** Over a radio, GET replies came back
      3.3–7.4 s after their GET, past every default timeout. A router
      answers a query `Timeout` at the query's own timeout and drops a
      later reply (Appendix B). So no router setting makes up for a caller
      that gave up first.
    - **The reference** raises its defaults to it: a call that sets no
      timeout waits the longer of §5.1's and the face's. A timeout the
      caller sets is taken at its word.
  - **When the wait for presence starts.** An owner starts when its
    session opens, before §8.2's first step, and a tool cannot see that
    instant. The scenarios count the wait from the later of two instants it
    can see: its own session connected to the network the owner joins, and
    the owner's launch. Where the owner's session is the router the tool
    connects to, a client connects only once that session is open, so the
    connection is the later (Appendix B).

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

- **The address comes first.** An owner's address, and every key
  built from it, is fixed before step 1, and a pure consumer's before its
  instance token. Where the deployment asks a profile to derive the system
  (§10; `hostid.v1` mints one, [`profiles/hostid/v1.md`](profiles/hostid/v1.md)
  §2.7), the runtime derives it then, before the service's session opens
  where it can. It derives nothing for a literal address.
  - A derivation that fails refuses the start as step 2 does: no instance
    token appears, and nothing of the service is declared or put. The
    profile states its failures, and its scenarios watch the refusal
    (`hostid.v1` §2.6, its `scenarios.md` §3).
  - The `self.system` providers of R1 are resolved at the same time, from
    the same system (§3.2).
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
  claimed. It also refuses an owner whose tokenless set names `archive.v1`,
  whether or not the owner implements it (§4.4): the set is the
  deployment's configuration, and naming the archive in it is an error.
  `[Sc: presence.md §2]` And it refuses one whose tokenless set names an
  interface it implements whose contract uses `redundancy.v1`: that
  interface's incumbent is the instance holding its token
  ([`redundancy.v1`](profiles/redundancy/v1.md) §2.14).
  `[Sc: profiles/redundancy/scenarios.md §11]`
- **A standby** (§6) exposes none of an interface's resources,
  required ones included, and its descriptor says so (§3.3): step 2 does
  not refuse it. It validates at its start what it will expose once it
  serves, as its entry's `unavailable` states, so that its taking over
  cannot fail step 2. `[Sc: profiles/redundancy/scenarios.md §1]`
- **Serving later, and stopping.** An instance that starts to
  serve an interface while it runs, a standby taking over, does steps 1
  to 4 for that interface, in their order: its resources, then its
  descriptor put again without the mark, then the interface's token, so
  alive ⇒ callable holds as at a start. One that stops serving an
  interface while it runs undoes them in the reverse order: the
  interface's tokens first, then its resources, then its descriptor put
  marked. `[Sc: profiles/redundancy/scenarios.md §1, §7]`
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
  - A profile MAY make it a MUST for its own resources. `health.v1` does
    for its status, which a reader then finds when the instance token
    appears, tokenless or not
    ([`profiles/health/v1.md`](profiles/health/v1.md) §2.3).
    `[Sc: profiles/health/scenarios.md §1]`

**Session posture.** An owner MUST NOT set a Zenoh timestamp on a put or
a delete of a data key, except on a resource §4.3's table lists. A
timestamp its session adds falls under the posture below. So does one
the owner sets where its session would add one anyway.
`[Sc: state.md §1]`
- **The owner's session** SHOULD run with timestamping disabled, zenoh's
  default for a client and a peer (§4.1). A session with it enabled stamps
  every put and delete it is not given a timestamp for. `[Sc: state.md §1]`
  - **The exception** is a candidate of a governed interface:
    [`redundancy.v1`](profiles/redundancy/v1.md) §2.10 asks it to enable
    timestamping.
- **A router on a constrained face.** A deployment SHOULD run a router
  that forwards puts onto a constrained face (§8.5) with timestamping
  disabled. A router with it enabled stamps every unstamped put it routes,
  and the stamp crosses the face. `[Sc: state.md §1]`
  - **What it gives up.** A router with timestamping disabled cannot run
    zenoh's storage manager, which refuses to start without it
    (Appendix B). Nor can it serve as a clock guard's reference (§4.3,
    "Ahead"), since it stamps nothing.
  - Nothing checks this posture at this version.

### 8.3 The presence budget

**At scale.** A deployment needs this section only once a presence domain
nears 10,000 liveliness tokens (§8.1); below that, discovery is quick
(Appendix B), and the budget asks nothing of it.

Presence costs per token, whatever the layout: every router holds each
token, and a router-to-router link carries each declaration (spike S2's
costs are in Appendix B). A **presence domain** is the set of routers whose
router-to-router links carry each other's declarations.

A deployment SHOULD keep a domain within about 10–15k tokens. Above that,
discovery slows, until it can fail to finish (spike S2, Appendix B). The
budget is shared with every other declaration, which spike S2 did not
measure.

**What a tool can count**: [`observer.md`](observer.md) §3.10.

### 8.4 Contract retrieval

Contract bundles (§9.6) live at the location-free key
`zk2/@zk/contract/<iface>/<sha256>`. Every holder MUST declare a
`complete` queryable on it. An owner holds the bundles of the interfaces it
implements (§8.2), and any other participant MAY hold bundles too, because
the hash is the check. A caller or tool MUST retrieve a bundle as follows:

1. GET with target `BestMatching` and consolidation `None`. That reaches
   the nearest holder on each router the query visits. A holder on the
   retrieving session answers too, so a caller or tool can get its own
   reply and the nearest remote one: it assumes nothing about the count.
2. Verify each reply **as it arrives** (§9.6), and accept the first valid
   one, without waiting for the GET to complete.
3. If none was valid, retry once with target `All`, consolidation `None`.
4. If still none, report the contract **unavailable**. Never accept an
   unverified bundle.

`[Sc: retrieval.md §1–§3]`

- **Consolidation `None` is a MUST.** zenoh's default consolidation on a
  concrete key holds every reply until the query finalizes (`Latest`,
  §4.1), so step 2 cannot happen, and a slow corrupt reply can displace the
  valid one, so that a caller or tool following the steps would report
  the contract unavailable (Appendix B). A caller or tool MUST set
  consolidation `None` on both attempts.
  `[Sc: retrieval.md §2]`
- **The reply.** A holder answers with one reply, the bundle's bytes, with
  `Encoding` `application/json`. A caller or tool MUST NOT depend on the
  encoding: the hash is the check. `[Sc: retrieval.md §1]`
- **An attempt ends** when its GET completes, or at the timeout the
  caller or tool gave it (§8.1).
- **From a token to a fingerprint.** An interface token carries
  `fp16`, the first 16 hex digits of the fingerprint (§1.2), which is not
  enough to retrieve by. A tool reads the full fingerprint from the
  instance's descriptor (§3.3, the interface's `contract`), and retrieves
  by that. There is no retrieval by prefix: a holder declares its
  queryable on full contract keys, and a GET on a wildcard over them is
  not a step of the procedure above.

### 8.5 Constrained faces

A **face** is a router's link to a part of the network. A deployment marks
a face constrained, and gives its maximum outage, in its face configuration.
[`link.v1`](profiles/link/v1.md) standardizes that configuration, its
face policy (§2.1), but the core rules below need only the two
facts.
- **No `@zk` traffic is required across it.** Bindings resolve statically
  (R7), and bundles come from holders on the same side or are
  pre-provisioned.
- **Liveness and health across it** come from what crosses (R7). A
  `health.v1` status re-put every 30 s is one put per service across the
  face, and the far side's only word on that service's health
  ([`profiles/health/v1.md`](profiles/health/v1.md) §2.8). A face
  that closes it leaves health unobservable there.
  - **A heartbeat that crosses** is liveness evidence too: one put per
    covered interface each period, never downsampled
    ([`freshness.v2`](profiles/freshness/v2.md) §2.9,
    [`link.v1`](profiles/link/v1.md) §2.8, §2.12).
- **Attachment decides what crosses.**
  - A router-to-router link carries every declaration of both sides. A
    `@zk` deny on it hides presence from the far side, but the denied
    declarations still cross, key strings included (zenoh 1.10.1).
  - A session that is a **client** of the near router receives only the
    declarations its interests ask for.
- **A deployment SHOULD attach the far side in one of two shapes, with a
  `@zk` deny on the face** (U23, §13):
  - **A far router in a south region** of the near router: the far router
    names its region (`region_name`), and the near router lists that name in
    `gateway.south`, while keeping its own clients and peers south.
    Declarations then cross on interest, and the deny keeps the denied
    families off the link. Router to router, they would cross it (spike
    S3, Appendix B).
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
- **A deployment's batch SHOULD take at most a third of the lease to cross
  the face.**
  - At 2,400 bit/s with the 10 s lease, a batch of up to 1,000 B
    qualifies: 1,000 B take 3.33 s, exactly a third. zenoh's 64 KB
    default does not: the link reconnected in a loop (Appendix B).
  - A link's batch is the minimum of the configured size, the link's MTU and
    the other end's. Over TCP, the face therefore needs a router configured
    for it.

  `[Sc: constrained.md §4]`
- **A deployment SHOULD deny `@stream` keys across the face** unless `link.v1`
  downsamples them: an `@stream` resource whose contract exposes it, one
  rule per concrete key ([`link.v1`](profiles/link/v1.md) §2.7, §2.8).
  A named key's frames are queued, not dropped, and every request
  waits behind them. The reference generator compiles a face policy into
  these denies and the downsampling rules (`zenctl acl gen --face-policy`).
- **Keys across the face.** A put toward a face rides the deepest key
  expression declared on that face for a prefix of its key. The rest of
  its key travels as text. No router declares a publisher's key toward
  the far side, and a prefix a router declares itself serves only routes
  computed after it. So the prefixes a put toward the far side can count
  on are those its own subscriptions declared (Appendix B).
  - A consumer across a constrained face SHOULD subscribe to each concrete
    resource it reads by its exact key, and to a template by its literal
    prefix up to the first parameter.
  - A consumer across a constrained face SHOULD NOT use a selector wider
    than what it reads.
  - **A template's selector** is its key with each parameter a wildcard,
    and an event's ULID chunk too. Its literal prefix is what is declared.
  - **What it buys.** Each put toward such a consumer carries a scope id
    of 1 or 2 bytes, plus the member's chunks under a template. Under
    `zk2/**` it carries every chunk after `zk2` as text.
  - **What no subscription shortens.** A reply carries its whole key as
    text through every router: an S4 reply, a re-read and a call's reply
    alike.
  - **A tool's half** is [`observer.md`](observer.md) §7.5.

  `[Sc: constrained.md §7]`
- **A far router in a south region needs declarations sent toward it.**
  It learns of queryables only from the declarations the near router
  sends it. So a deployment MUST grant the far side's principal
  `declare_queryable` on egress over every queryable its side may query,
  as the owner declares it. A declaration is checked by inclusion against
  the queryable's own key expression (§11.3), which the far side's
  selectors need not include. The queryables the grant covers are:
  - for a state GET, the owner's state queryables over `state/**` and
    `@state/**` (S2);
  - for a call, the operation's queryable on its concrete key or over its
    template (O1): `@op/set/*` for a call of `set/duty`;
  - for an archive, its `@state` forms (§4.4).

  Without that grant, its queries are never routed.
  `[Sc: link.v1 scenarios §2]`
- **A declaration the medium lost is never repaired.** Zenoh 1.10.1 sends
  each declaration once. A subscriber, a queryable or a key expression
  whose declaration a medium lost below zenoh stays unknown to the router
  for as long as the transport stays up (Appendix B). A face policy says
  whether its medium repairs a lost frame
  ([`link.v2`](profiles/link/v2.md) §2.1's `arq`).
  - A consumer or a tool across a constrained face whose medium repairs
    no lost frame SHOULD declare a subscription again when it has heard
    nothing on it for two of its horizons while its session's transport
    stayed up. `[Sc: constrained.md §9]`
  - **A subscription's horizon** is that of what it hears: its resource's
    `freshness.ttl_s` above 0, or a heartbeat's, the shortest horizon of
    the collections it covers
    ([`freshness.v2`](profiles/freshness/v2.md) §2.4). A state of a
    contract that uses `freshness.v2` has none of its own: its owner
    re-puts nothing, and its heartbeat speaks for it. A subscription to
    what has no horizon is never declared again.
  - **The silence** counts from the latest of three instants: the
    subscription's declaration, its last sample, and its session's last
    transport opening or closing. A transport that opens again declares
    every subscription anew. Through a session cut off from every router
    nothing is declared.
  - **Declaring again** undeclares the subscription, then declares it
    anew. Beside the old one, zenoh 1.10.1 would join the new subscriber
    to it and send nothing (Appendix B). A sample put between the two is
    missed, after two horizons of silence. It costs one declaration a
    silent subscription every two horizons.
  - **What it repairs** is a subscriber's declaration. A lost
    `DeclareKeyExpr` it does not: a session's runtime declares a prefix
    toward its router once a transport (Appendix B). Only a transport
    that opens again repairs that.
  - A caller across the face MAY open its session again after two GETs
    in a row drew no reply, its transport up. A session that opens again
    declares everything anew. Toward a far router in a south region, the
    routers' own declarations stay as they were.
  - **A tool's half** is [`observer.md`](observer.md) §7.5.
- **Access control is per hop** (§11.3). The near router sees the far
  router as one principal, carrying the union of the grants the far side's
  sessions need.
- **A router's timestamping** on the face: §8.2, "Session posture".

### 8.6 The deployment catalog

A **catalog** is a file that a deployment provisions to its owners, and to
the far side of a constrained face (§8.5). It states the **deployment
epoch**, which §7.6's `time` fields count from. It also gives each
contract revision it lists an **index**. A later form can carry that
small number in place of an interface id and a fingerprint. A catalog is
never fetched, and no participant serves one on the bus, so no key form
names one (§1.1).

```json
{
  "format": "zk2-catalog/1",
  "epoch": 1767225600,
  "lineage": [{"id": "eddb48d5", "entries": 2}],
  "entries": [
    {"index": 0, "iface": "modem.v3", "contract": "sha256:…"},
    {"index": 1, "iface": "config.v1", "contract": "sha256:…"},
    {"index": 2, "iface": "health.v1", "contract": "sha256:…"}
  ]
}
```

**The document** is strict JSON: RFC 8259, with no member twice in one
object. Its schema is [`catalog.schema.json`](catalog.schema.json). Its
top level holds these members, and no other:

| Member | Value |
|---|---|
| `format` | Exactly `zk2-catalog/1`. |
| `epoch` | The deployment epoch: whole seconds since the Unix epoch, an integer from 0 to 2^53 − 1. |
| `entries` | The revisions: an array of at most 1,024 entries, each an object holding `index`, `iface` and `contract`, and no other member. |
| `lineage` | Optional, and never empty: every catalog this one succeeds, oldest first. Each item is an object holding that catalog's `id` and its count of `entries`, and no other member. A first catalog has none. |

- **An integer** here is a JSON number written as digits alone, with no
  sign, fraction or exponent. So `-0`, `1.0` and `1e3` are no integer.
- **An entry's `index`** is its position in `entries`, from 0. Stating it
  guards a hand edit. With at most 1,024 entries, an index fits in 10
  bits.
- **An entry's `iface`** is an interface id, and its `contract` a
  fingerprint (§1.2). No two entries hold the same pair. An interface
  appears once per revision it lists.
- **A lineage item's `id`** is a catalog id (below), never an earlier
  item's or the catalog's own. Its `entries` is at least the item
  before it's, and at most this catalog's count of entries.

**Reading.** An implementation that reads a catalog MUST refuse it,
naming the refusal, for the first of these that applies, in this order:
- it is not strict JSON (`json`);
- its top level is not an object whose `format` is the string
  `zk2-catalog/1` (`format`);
- the top level lacks `epoch` or `entries`, or holds another member than
  the four above (`member`);
- `epoch` is not an integer from 0 to 2^53 − 1 (`epoch`);
- `entries` is not an array of objects, or holds more than 1,024 (`entries`);
- then, for each entry in order:
  - it lacks `index`, `iface` or `contract`, or holds another member
    (`member`);
  - its `index` is not the integer of its position (`index`);
  - its `iface` is not a string holding an interface id (`iface`);
  - its `contract` is not a string holding a fingerprint (`contract`);
  - its pair is an earlier entry's (`duplicate`);
- then, where `lineage` is present, it is not a non-empty array of
  objects (`lineage`);
- then, for each lineage item in order:
  - it lacks `id` or `entries`, or holds another member (`member`);
  - its `id` or its `entries` breaks the document's rules above
    (`lineage`).

`[F: catalog/documents.json]`

**The id.** A catalog's id is the first 4 bytes of the sha256 of its JCS
bytes (RFC 8785), written as 8 lowercase hex digits. An implementation
MUST compute it so. JCS reads the document, so neither its spacing nor
its members' order moves its id. `[F: catalog/ids.json]`
- **Four bytes** keep the id short where a later form carries it. Two
  catalogs share an id about once in 2^32 pairs, and nothing in a reading
  tells such two apart.

**Succession.** A catalog B **succeeds** a catalog A when all three hold:
- B's `lineage` is A's, followed by A's id and A's count of entries;
- B's `epoch` is A's;
- A's entries are B's first ones, each at its own index.

A deployment MUST NOT change a catalog in force other than by provisioning
one that succeeds it. So it never reassigns or removes an index, and
never moves the epoch. `[F: catalog/succession.json]`

A checker of succession MUST refuse a B that does not succeed A, naming
the first of these that applies, in this order:
- B's `epoch` is not A's (`epoch_moved`);
- an index below A's count holds another pair in B (`reassigned`);
- B holds fewer entries than A (`removed`);
- B's `lineage` is not A's, followed by A's id and count (`lineage`).

`[F: catalog/succession.json]`

**The epoch in force.** An owner given a catalog counts its `time` fields
from the catalog's epoch, and states its id in its descriptor (§3.3). An
owner with none counts them from 0, the Unix epoch (§7.6). Across a
constrained face no descriptor crosses (§8.5). There, which catalog an
owner writes under is the deployment's word, as its bindings are (R7).

**Reading an index.** A reader reads an index, or a time counted from the
epoch, through the catalog the owner names.
- **Through a successor.** A reader may instead read through a catalog
  whose `lineage` holds that id. It then reads only the indices below
  that item's count of entries, and the successor's epoch, which is the
  same.
- **No catalog named.** An owner that names none writes no index, and a
  reader counts its times from 0.
- **Never a guess.** A reader MUST refuse each of these:
  - an id, when it holds neither that catalog nor one whose `lineage`
    holds it (`unknown_catalog`);
  - an index from an owner that names no catalog (`unknown_catalog`);
  - an index at or above the count it reads through (`unknown_index`).

  `[F: catalog/succession.json]`

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
| `[schemas]` | `protobuf` (files), `proto_include` (import roots), `jsonschema` (files), `packed` (files, §7.6) |
| `[resources."<template>"]` | `kind` (`stream`, `state`, `event`, `operation`), plus the fields of Appendix D |
| `[requires.<role>]` | `interface`, `resources`, `cardinality` (`one`, `many`), `optional`, `doc`, `annotations` |

**Field legality.**
- A field that its kind does not take, per Appendix D, is **E014**, once
  per field and resource.
- `history` is the exception. Outside a plain stream or state, it is
  **E019**.
- `frame` is another. Outside a plain stream whose template has no
  parameter, it is **E046**. It is a resource's own, so `[defaults]` and
  `[defaults.<kind>]` take none (E000).
- The same rules apply to `[defaults.<kind>]`, where `<kind>` decides.

`[F: contracts/e014-field, e019-history, e046-frame-place]`

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
- `cardinality` is an integer or the string `"unbounded"`, which is
  4294967295 from resolution on (§2.2, §9.5). Any other string is a
  value of the wrong type, so E000.
  `[F: contracts/ok-cardinality-unbounded, e000-cardinality-string]`
- `frame` is a table of §7.7's keys. Its integers are bounded by E048,
  not by the schema, and a deadband's `delta` is a string.
  `[F: contracts/e048-frame-value]`
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
| E002 | a `uses` entry is not `<name>.v<major>`; or two entries name one profile at two majors: an annotation key names its profile without a major (§10 point 2) | per entry; once per profile listed at more than one major |
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
| E024 | a `json:` or `packed:` name defined by several listed files of its kind; two listed files of one kind, JSON Schema or packed, with one stem; or two listed JSON Schema or packed files with one id | per reference; per listed file whose stem or id an earlier one took, once (that file is not loaded, §9.4) |
| E025 | `media_param` does not name a single-chunk parameter of the template | per reference |
| E026 | `rate` not `rare`, `low` or `burst(<n>/h)` (n from 1 to 2^32−1, decimal, no leading zero); `retention` not `<n>` + `s`/`m`/`h`/`d`/`w` (n ≥ 1, decimal digits, leading zeros allowed, the seconds at most 2^64−1, unsigned) (§2.6) | per field |
| E027 | a string in the canonical form (§9.5) outside printable ASCII (0x20–0x7E), keys included | per value |
| E028 | a number outside the canonical domain (§9.5): an integer outside ±(2^53−1), a float that is not finite, or a float JCS writes as such an integer; in the canonical form, or in a JSON Schema or packed artifact | per value; once per artifact |
| E029 | a schema file missing, unreadable, not JSON (a duplicate member included), or not compiling (protobuf editions, and a file under no import root, included) (§9.4) | per file |
| E030 | a role not `[a-z][a-z0-9_]*`, an `interface` not `<name>.v<major>`, a `resources` entry that is not a template, or `resources = []` | per finding |
| E031 | `deprecated.replaced_by` names no other template of the interface, or `since` is greater than `minor` (checked only when `minor` is present) | per condition (both can fire on one resource) |
| E032 | a `$ref` at a schema position (§7.3) whose file part has a scheme (`:`) or names no listed file, or whose fragment is not a JSON Pointer that resolves (§9.4) | per `$ref` |
| E033 | `summary` written, and the resolved `replies` (§9.3) is not `many` | per resource |
| E034 | `history.depth` is 0: on a resource where `history` is legal (where it is not, E019 alone); in `[defaults]`; in `[defaults.<kind>]`, where an illegal `history` gives both E019 and E034 | per occurrence |
| E035 | a requirement names a resource its interface does not declare, when that interface is in the set; checked against the interface's first declaration (below) | per resource name (set check) |
| E036 | two contracts of a set declare one interface id | per declaration after the first (set check) |
| E037 | a JSON Schema keyword outside the subset (§7.3) | once per keyword and file |
| E038 | a packed file outside §7.6's shape: its top level, a type or a field missing a member it needs, holding one it does not take, or holding a value of the wrong type or spelling, a name's included | once per object: the file, a type or a field |
| E039 | an empty packed declaration: a range whose `minimum` is above its `maximum`, an enum with no value, or a record with no field (§7.6) | per field or record |
| E040 | a fixed-point field whose `minimum`, `maximum` or `resolution` is not a decimal string, or whose `resolution` is not above 0 or does not divide `maximum` − `minimum` (§7.6) | per field |
| E041 | a packed field whose width is not from 1 to 64 bits, or an enum whose `width` holds fewer codes than it needs (§7.6) | per field |
| E042 | a name repeated in a packed file: a field's in its record, a value in its enum, or a reserved code in its list | per repeat, after the first |
| E043 | a packed field whose `type` names no field type of §7.6 | per field |
| E044 | a packed type with no CH to carry its tag: an explicit stream's type, or an attachment (§7.1) | per reference |
| E045 | an extension marker misplaced: a field after a marker that is not optional, or a marker right after another (§7.6) | per field or marker |
| E046 | `frame` on a resource that is not a plain stream, or whose template has a parameter (§7.7) | per occurrence |
| E047 | a frame resource's type that is not packed, or a packed type with an extension marker (§7.7) | per resource |
| E048 | a `frame` value outside §7.7's table: `records` not from 1 to 256; `step_ms` not from 1 to 2^32 − 1, given with `records` 1, or absent with `records` above 1; `jitter_ms` without `step_ms`, or above `step_ms` / 2; `keyframe_every` not from 1 to 65,536; a deadband's `delta` not a decimal string above 0, or its `max_records` not from 1 to 65,536 | per key; per deadband member |
| E049 | a deadband naming no field of the record type, or a field that is not an `int`, `fixed` or `counter` (§7.7) | per deadband entry |
| E050 | timing a frame resource cannot keep (`timing.v1` §2.3): a `timing.deadline_ms` below its span; a deadband beside a `timing.deadline_ms`, unless its `max_records` × the frame's interval is at most the deadline, an unstated interval never; a `timing.lifespan_ms` | per condition: the deadline once, each deadband once, the lifespan once |
| W101 | two resolved resources under one token whose templates overlap, with different types (the payload type, or an operation's request and response) | per pair |
| W102 | an operation's request type has a top-level field named like a template parameter | per resource |
| W103 | `encoding` or `attachment_encoding` written on a resource where no JSON Schema type takes it, judged on resolved types only (cascade 6) | per field (a defaulted one is silent) |
| W104 | `minor` is absent | once |
| W105 | an annotation key outside its profile's vocabulary: the table the profile publishes, else its interim one (Appendix D); a profile with neither has none to be outside of (§10) | per key |
| W107 | the file is not named `<name>.v<major>.toml` | once |
| W108 | a frame resource whose interval (§7.7) is above half its `freshness.ttl_s` ([`freshness.v1`](profiles/freshness/v1.md) §2.4): its owner puts a frame at least every ttl/2, so its stated cadence is one it cannot keep | per resource |

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
7. **In a packed file,** a type whose name is not a type name is E038, and
   defines no type. Nothing in its record is linted. A file whose `types` is missing, or is not an object,
   defines none.
   - A field gets at most one of E038 and E043, and then nothing more.
     E043 is checked first, on a `type` that is a string.
   - A field gets at most one of E039, E040 and E041, the first met in
     this order: a decimal string that does not read (E040), an empty
     range (E039), a `resolution` not above 0 or not dividing (E040), and
     the width (E041).
   - E042 and E045 read the fields of a record that have neither E038 nor
     E043. A repeat inside one field's `values` or `reserved` is E042 too,
     on a field without E038.
   `[F: contracts/e038-packed-shape, e042-packed-duplicate, e045-packed-extension]`
8. **On a frame resource,** E046 alone where `frame` cannot be.
   - E047 and E049 read the record type, so they wait for one that
     resolves (E023, E024). E047's marker half and E049 also wait for a
     packed declaration with no finding (E038 to E045).
   - E050 waits for the values it reads: none of them E048's. It reads
     the `timing.*` values as `timing.v1` §2.2 reads a declaration. Where
     one of them does not read, the declaration is invalid. E050 then
     reads none of its values, as `timing.v1` judges none.
   - W108 waits for E048's none too. It reads the horizon as
     `freshness.v1` §2.1 reads one, and an unstated interval gives none.
   `[F: contracts/e023-frame-unresolved, e047-frame-type, e049-frame-deadband, e050-frame-timing, w108-frame-horizon]`

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
  - **In a bundle,** which keeps no paths, the file part names the JSON
    Schema artifact whose `name` is the stem of its last path segment.
    Stems and ids are unique per contract, so every loaded file is an
    artifact under its own stem, and this is the file the path named: in
    the bundle of a contract with no E024 and no E032, every `$ref`
    resolves.

`[F: contracts/e032-ref-dangling, e032-ref-outside, e032-fragment, ok-json-qualified, e024-ambiguous, e024-identical]`

**Packed.** A packed file is read as a JSON Schema file is, and declares
§7.6's types.
- **The artifact** is the document, read as strict JSON (E029). Its id
  hashes the document's JCS bytes, and its `name` is the file's stem.
- **Stems and ids.** A stem MUST be unique among the listed packed files,
  and an id among the listed JSON Schema and packed files (E024). A later
  file whose stem or id is taken is not loaded. The JSON Schema files are
  taken before the packed files, each list in its order.
- **`packed:Name`** is `types/Name` in the one listed packed file that
  defines it (E023 when none does, E024 when several do).
- **`packed:stem#Name`** looks only in the packed file with that stem.
- **Its declarations** are linted E038 to E045, and its placement E044
  (§7.1).

`[F: contracts/ok-packed, e024-packed-ambiguous, e024-packed-twice]`

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
| `schemas` | one `{"id", "kind", "name"}` per artifact (§9.4), sorted by `id`. `kind` is `protobuf`, `jsonschema` or `packed`. |
| `resources` | one object per resource, sorted by (kind token's spelling, template), bytewise: `@op` < `@state` < `@stream` < `events` < `state` < `stream` |
| `requires` | an object keyed by role |

**Every resource** has these members:

| Member | Value |
|---|---|
| `token` | the kind token (§1.3) |
| `template` | the template as written |
| `kind` | `stream`, `state`, `event` or `operation` |
| `params` | parameter name → `string`, `uint` or `path`; `{}` when none |
| `cardinality` | an integer, or `null`; `"unbounded"` is written 4294967295 (§2.2) |
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

**A frame resource** adds `frame`, and no other resource has the member:

| Member | Value |
|---|---|
| `frame` | `{"records", "step_ms", "jitter_ms", "keyframe_every", "deadband"}`, each resolved (§7.7). `step_ms` and `jitter_ms` are `null` without a step. `deadband` is an object by field name of `{"delta", "max_records"}`, `delta` in its shortest form (§7.6), and `{}` when none. |

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
- `{"kind": "packed", "name": <the types key, without any stem>, "schema": <artifact id>}`;
- `{"kind": "raw", "media_type": …, "media_param": <name or null>}`.

**A requirement** is `{"interface", "resources", "cardinality",
"optional", "annotations"}`. `resources` is the sorted, deduplicated list,
or `null` for "all".

**Excluded:** `minor`, the interface's `summary`, and every `doc`. An
operation's `summary` *type* is included, and so is `deprecated.reason`.

**The format tag** is unchanged by authoring drafts 3 and 4. A contract
that lists no packed file has the canonical bytes, and the fingerprint, it
had under draft 2. One that lists one holds the kind `packed`, which no
earlier form holds, so the two cannot be confused. Draft 4 adds `frame`.
A contract with no frame resource keeps its draft 3 bytes, and one with a
frame resource holds a member no earlier form holds.
`[F: contracts/ok-packed, ok-frame]`

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
- Canonical bytes are portable for JSON Schema, packed and raw types.
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
             "sha256:…": {"kind": "jsonschema", "data": <JSON Schema document>},
             "sha256:…": {"kind": "packed", "data": <packed document>}},
 "extras": {"sha256:…": {"media_type": "application/json", "data": <JSON document>}}}
```

**Extras** are exactly the documents that the `views.document` annotations
of the contract's resources reference. That is the only extra-carrying
annotation in this version.
- A `views.document` value references one document by its id, a
  `sha256:…` string. Another value references nothing, and requirement
  annotations carry no extras.
- Where a builder finds the document for an id is `views.v1`'s to define,
  a profile not yet written, and not the core's. Until it does, a core builder carries no extras,
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
| 9 | For each listed schema, in id order: it is present (`missing_schema`); its `kind` is the listed one, an entry that is not an object or has no `kind` included (`schema_kind`); it has a `data` member and no member besides `kind` and `data`, `data` is base64 text for protobuf, and its kind is `protobuf`, `jsonschema` or `packed` (`shape`); it hashes to its id, from the decoded bytes for protobuf and the JCS bytes for jsonschema and packed (`schema_hash`) | as given |
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
  such as one built from two identical files. A conforming builder
  produces neither: the `$ref` is E032, or, for two files with one id,
  E024 (§9.4). The classifier reads both as
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
    that tree, or `spec/profiles/.history` for the profiles' standard
    contracts (§10).
  - **One revision, two roots.** A contract that moves to a new
    root keeps its published revisions in the old one, which is
    append-only, and is published in the new one from then on, its
    revisions copied there byte for byte. A bundle's file name is its
    fingerprint, so the copies are one revision. `health.v1`'s revision
    1.0 is in `examples/zk2/.history` and `spec/profiles/.history`.
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
    and packed documents by their JCS bytes, and protobuf `FileDescriptorSet`s as
    messages, once source info and every default `json_name` are dropped.
    The default `json_name` is protoc's: the field name with each `_`
    removed and the letter after it upper-cased. Raw types have no
    artifact.

  Fingerprints may over-detect a change, never under-detect one.
  `[F: compat/expect.json, same_revision]`
- **A published revision was accepted.** A revision in the history root
  passed §9.8's review when it was published. The root records no other
  acceptance. So a check of it against the root does not refuse it for a
  review. It reports the review as accepted, or says nothing of it. It
  still refuses a breaking change, which §9.8 forbids publishing.

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

- **On the bus, revisions carry no order.** A bundle keeps no
  `minor` (§9.5), and two providers can serve two revisions side by side.
  A tool that classifies them against each other MAY take the order from
  the `minor` their descriptors state, when the two differ. Otherwise it
  classifies both ways: the pair is clean only when both directions are
  compatible, a finding when neither is, and undecided when they disagree.

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

**Frames** (§7.7), a frame resource paired with its counterpart:

| Change | Class | Rule |
|---|---|---|
| `frame` added or removed | breaking: a reader of the other revision reads none of its samples | `frame_toggled` |
| `records`, `step_ms` or `jitter_ms` changed | breaking: the bits and the times move | `frame_changed` |
| A deadband added, removed or changed | breaking: what an omitted field holds moves, either way | `deadband_changed` |
| `keyframe_every` changed | review: how long a reader waits to heal | `keyframe_changed` |

**A deadband is compared by its field's place** in the record type, as
the frame form lists it (§7.7), never by its name. So a field renamed
keeps its deadband. The rename is the packed rules' `field_renamed`, a
review.

The record type follows the packed rules below. It has no marker, so
every change of its layout is breaking. Each breaking change above, and
each of the layout, moves the frame's tag (§7.7). A reader of the other
revision then refuses its frames by tag 15 times in 16.
`[F: compat/frame/]`

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
| The schema kind changed (raw, protobuf, jsonschema, packed) | breaking | `type_kind_changed` |
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
  - **"As written"** means by its text: the `$ref` value and
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

**Packed payloads:** the wire is the layout (§7.6), so the rules read it
field by field. Names and descriptions are not the wire.
- **What is compared.** Each revision's `fields`, item by item, from the
  first. A marker meets a marker. Two fields are compared by their
  declarations, the rules below.
- **A different length** is growth behind a marker when the shorter list
  ends with a marker, and the longer goes on after it. An optional field
  appended or removed there is compatible. Any other difference of length
  or order is breaking (`fields_changed`).
- **Breaking:**
  - a field's `type` changed (`type_changed`), or a marker where the other
    revision has a field (`fields_changed`);
  - `optional` toggled (`presence_changed`): the presence bit appears or
    goes;
  - a bound moved, `minimum` or `maximum`, compared as values
    (`bound_changed`), so `"2.00"` and `"2"` are one bound;
  - a `resolution` changed (`resolution_changed`);
  - a width changed (`width_changed`): a counter's `width`, an enum's
    width, or `ms` toggled;
  - `reserved` changed (`reserved_changed`): the codes move;
  - an enum's escape toggled, a value moved to another index, or a value
    added or removed where no escape reads it (`enum_changed`).
- **Review:**
  - a field renamed (`field_renamed`);
  - an enum value renamed in place, to a name the other revision does not
    hold (`enum_value_renamed`);
  - a value removed from the end of an enum with an escape
    (`enum_value_removed`): an old writer still writes it, and a new
    reader reads the escape.
- **Compatible:** a value added at the end of an enum with an escape,
  within its width: an old reader reads it as the escape. And every
  change to a name or a `description` not listed above.

`[F: compat/payload/packed/append-behind-marker, remove-behind-marker,
append-closed, marker-added, type-changed, optional-toggled, bound-moved,
bound-respelled, resolution-changed, width-widened, reserved-added,
enum-reordered, enum-closed-added, enum-escape-widened, field-renamed,
enum-value-renamed, enum-escape-removed, enum-escape-added,
description-only, compat/transitive/packed-group-reused]`

---

## 10. Extension points for profiles

A **profile** is an independently versioned specification, such as
`timing.v1` or `archive.v1`. The core never depends on one. Where a rule
here names a profile, it names the standard way to meet the rule, which
the core states in its own terms. The profiles named so are
[`health.v1`](profiles/health/v1.md) for reporting,
[`link.v1`](profiles/link/v1.md) for face configuration, `archive.v1` for archives,
[`freshness.v1`](profiles/freshness/v1.md) for a value's age,
[`telemetry.v1`](profiles/telemetry/v1.md) for a counter's epoch,
[`desired.v1`](profiles/desired/v1.md) for store-and-forward,
[`redundancy.v1`](profiles/redundancy/v1.md) for exclusivity, and
[`arbitration.v1`](profiles/arbitration/v1.md) for choosing between bound
providers. A profile contributes through these four points, and through no
other:
1. **A standard contract or a standard schema.**
   - **A standard contract** is an interface the profile defines. It
     lives in the profile's directory, published in
     [`profiles/.history/`](profiles/.history/) (§9.7). The first is
     `health.v1`'s
     ([`profiles/health/health.v1.toml`](profiles/health/health.v1.toml));
     `config.v1`'s and `alarms.v1`'s followed
     ([`profiles/alarms/alarms.v1.toml`](profiles/alarms/alarms.v1.toml)).
     An instance implementing it lists the contract's `uses` in its
     descriptor's `profiles`, as for any contract (§3.3), and the
     interface in its `interfaces`.
   - **A standard schema** is a schema artifact that contracts
     list, as they list any file (§9.4). It lives in the profile's
     directory. Its types are the profile's when the artifact's id (§7.1,
     §9.4: the hash of its JCS bytes) is one the profile publishes, so a
     contract listing a copy, byte for byte or JCS-equal, carries the
     profile's types, and a type of the same name in another artifact
     does not. The first is `telemetry.v1`'s
     ([`profiles/telemetry/telemetry.v1.json`](profiles/telemetry/telemetry.v1.json),
     its §3), and `desired.v1`'s followed
     ([`profiles/desired/desired.v1.json`](profiles/desired/desired.v1.json),
     its §3). A standard schema adds no interface, and a
     contract that lists it is classified and published as any other
     (§9.7, §9.8).
2. **An annotation vocabulary.** Keys are `<profile>.<key>`. A contract
   that uses one MUST list the profile in `uses`, which is fingerprinted.
   `[F: contracts/e020-annotation]` Until a profile publishes its
   vocabulary, the interim tables of Appendix D apply, and a key outside
   them is a warning. `[F: contracts/w105-vocabulary]` A profile that
   Appendix D does not list has no interim table, so none of its keys is
   outside one: no W105. `[F: contracts/ok-profile-without-vocabulary]`
   A published vocabulary replaces its interim table, and W105 reads it
   the same way. The first is `freshness.v1`'s
   ([`profiles/freshness/v1.md`](profiles/freshness/v1.md) §4);
   `link.v1`'s ([`profiles/link/v1.md`](profiles/link/v1.md) §4),
   `telemetry.v1`'s
   ([`profiles/telemetry/v1.md`](profiles/telemetry/v1.md) §4),
   `desired.v1`'s ([`profiles/desired/v1.md`](profiles/desired/v1.md) §4),
   `redundancy.v1`'s
   ([`profiles/redundancy/v1.md`](profiles/redundancy/v1.md) §4),
   `timing.v1`'s ([`profiles/timing/v1.md`](profiles/timing/v1.md) §4)
   and `arbitration.v1`'s, the first read on a requirement
   ([`profiles/arbitration/v1.md`](profiles/arbitration/v1.md) §4),
   followed. `[F: contracts/w105-redundancy-epoch,
   w105-timing-max-rate, w105-arbitration-deadline]` A requirement's
   annotations are checked as a resource's are (§9.2).
   `[F: contracts/e020-requirement]`
   A profile may publish an empty one, as `alarms.v1` does
   ([`profiles/alarms/v1.md`](profiles/alarms/v1.md) §4): it
   defines no key, so W105 warns of every key under its name.
   `[F: contracts/w105-alarms-vocabulary]`
   What a published vocabulary defines (`views.v1`'s documents among them,
   §9.6) is the profile's, not the core's.
   **One major per contract.** A key names its profile without a
   major, so a contract uses one major of a profile: two in one `uses` is
   E002. `[F: contracts/e002-two-majors]` Two contracts of one instance may
   use two majors of it, so a descriptor's `profiles` can list both
   (§3.3). W105 reads the table of the major the contract uses:
   `telemetry.v2`'s ([`profiles/telemetry/v2.md`](profiles/telemetry/v2.md)
   §4) adds `series` to `telemetry.v1`'s four keys.
   `[F: contracts/w105-telemetry-series, ok-telemetry-v2-series]`
3. **A registered verbatim kind**, such as `@blob`, at position 5. This
   point is reserved: this version refuses such keys (§1.2), and a later
   version defines their form.
4. **The descriptor's `profiles` list** (§3.3).

**The standby mark** is the core's own member (§3.3, §6), not a
fifth point: the core says what a standby is, holds and exposes, and a
profile says only when an instance stands by and when it serves, as
`redundancy.v1` does for an interface whose contract uses it.

**Derivation-only profiles.** A profile uses any of the four points,
and states which. One that uses point 4 alone is **derivation-only**:
- it defines how a participant derives something it puts on the bus, and
  no contract, annotation or kind. `hostid.v1` derives a system name from
  the machine id;
- it adds nothing of its own to the bus: what it derives goes where the
  core already puts it (a system name at position 2), and an instance that
  follows it lists it in the descriptor's `profiles` (§3.3);
- a contract's `uses` names vocabularies (point 2), so it has no reason to
  be there. No lint refuses one, since the core does not know which
  profiles derive: the profile says what such a listing means for it.

The core still depends on none. §3.3 names `hostid.v1` as today's
derivation-only profile, and a checker reads `profiles` for its form
alone: each entry `<name>.v<major>`, none repeated (D010).
`[F: descriptors/ok-derivation-profile]`

**Where profiles live.** Each profile has a directory under
[`profiles/`](profiles/): its text, one file per wire major
(`hostid/v1.md`), its own `CHANGELOG.md`, its fixtures and its scenarios.
[`profiles/README.md`](profiles/README.md) holds the index, the template
every profile follows, and the process.

---

## 11. Security

### 11.1 Grant shapes

Ownership (§6) reduces access control to a few grant shapes.

**The input.** A generator compiles a deployment:
- its principals, each bound to a user name or a certificate CN, never both
  and never a zid (§11.3). A generator MUST refuse an input that binds one
  principal by both: zenoh matches one value of each property
  a subject names, so a subject naming both matches only a session
  presenting both, and under `default_permission: allow` a session
  presenting one of them would be allowed everything.
  `[Sc: link.v1 scenarios §11]`;
- the services each principal runs, with the contracts they implement;
- each holder's bindings: role, interface, providers, resources,
  parameters (R2), and whether it reads with history;
- each holder's calls: interface, providers, operations;
- the services a tool inspects without binding or calling them, and
  whether it reads the admin space;
- the archives: what each records, and its peers;
- the namespace (§1.6).

The format is the generator's own. The reference reads an enrollment file
(`examples/zk2/acl/*.enrollment.toml`, informative).

| Grant | Rule |
|---|---|
| **Own** | A service principal puts, deletes, declares queryables, replies and declares tokens under `zk2/<system>/<service>/**`, on ingress. It also holds each verbatim subtree, spelled out because `**` never crosses one: `…/*/@stream/**`, `…/*/@state/**`, `…/*/@op/**`, `…/@zk/**`. A service using advanced publication (§2.5) also holds `…/*/stream/**/@adv/**` and `…/*/state/**/@adv/**`. On egress, it receives queries and subscriber declarations on the same keys (spike S14). |
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name, plus their `@adv` subtrees where the consumer uses history, plus **presence**: liveliness reads (GETs and subscriptions, §8.1) and the descriptor's GET and subscription (§3.3) on the `…/@zk/**` subtree of each provider they name. Plus each **heartbeat** a binding needs: where a bound interface's contract uses `freshness.v2` and the binding consumes one of its collections (§7.5), the subscription to each named provider's `…/freshness.v2/stream/heartbeat/<iface>`. A consumer denied it hears no confirmation, and every member it follows goes stale (`freshness.v2` §2.5). |
| **Call** | Query on specific `…/@op/<op>` keys, plus presence on the `…/@zk/**` subtree of each service whose operations it calls. |
| **Tool** | A role a principal plays, with no address of its own: Consume and Call on what it reads and calls, and presence on the services it inspects. **Inspection** is presence alone, on the `…/@zk/**` subtree of each service named, exact or wildcard, and none of its data. A tool drawing the graph (R3), a doctor, or a service listing reads descriptors through presence. A tool that checks S4, runs a doctor or draws the mesh (§4.2) also holds the **admin read**: `query` on `@/*/router`, `@/*/router/status/plugins/storage_manager/storages/**` and `@/*/*`, and their `reply`, never namespaced. Under `deny`, no other grant reaches the admin space. |

- **Contract bundles are open:** any principal MAY hold or fetch
  `zk2/@zk/contract/**`, because the hash is the check.
- **No principal declares queryables under `@/**`**: the routers
  serve the admin space themselves. A generator allows it to none under
  `deny`, and denies it to every principal under `allow` (observer.md
  §4.2, "Who answered"). `[Sc: security.md §3]`
- **The admin read admits no GET of a node's declaration records.** A
  node answers its declaration records, its
  `linkstate/*` and its `route/successor/**` under its routing tables'
  read lock, and one GET of them beside another face's declaration
  freezes it for good (observer.md §4.3, Appendix B). A query is admitted by
  inclusion in an allowed key (§11.2), so it reaches only what that key
  reaches.
  - Under `deny`, a generator MUST NOT compile an allow rule for an
    ingress `query` on a key expression that intersects one of those keys
    of any node,
    `@/<zid>/<whatami>/{subscriber,publisher,queryable,querier,token}/**`,
    `…/linkstate/*` or `…/route/successor/**`. The GET that freezes a
    node is the asker's, decided at the asker's face on ingress. A router
    then refuses a GET of them from any principal at
    its ingress, before the admin space runs, and answers it complete and
    empty. `[Sc: security.md §4]`
  - The admin read is the three selectors the reference's tools read:
    S4's two (§4.2), which the doctor and the conformance suite read, and
    every node's root document, which a mesh view reads. None intersects
    those keys.
  - A wider admin read, `@/*/router` and `@/*/router/**`, would admit a
    GET of every one of a router's records. It never reaches a peer's
    records, nor a client's. It would refuse a GET of `@/*/*`, which
    neither of its keys includes.
  - Under `allow`, the generator compiles no admin read, and a GET wider
    than every deny is in none (§11.3), so a router there is not
    protected: a tool's GET of `@/*/router/queryable/**` under an `allow`
    block is admitted and answered (Appendix B). This protects a router
    under `deny` alone.
- **There are no cross-principal write grants.**
- **The shapes compose per principal.** A principal holds
  the union of the grants of everything it runs: a principal that runs
  services may run a tool too, and holds the tool's grants beside its
  Own, the admin read included. This is safe because no shape writes
  beyond the writer's own prefix:
  - Own stays scoped to the services the principal runs;
  - a tool's grants read, and call specific operations: Consume, Call,
    presence and the admin read write nothing;
  - the admin read is a `query` on the nodes' admin space, each node with
    its admin space on answering for itself (the read includes `@/*/*`,
    which a client's and a peer's root documents answer too), and every
    principal is still denied a queryable under `@/**`.

  A principal's grants therefore never let it write what a service it
  does not run owns, which is what §6 rests on. `[Sc: security.md §1]`
- **Inspection travels to the provider.** A liveliness read is
  answered by the routers and never reaches the provider, but the
  descriptor's GET and subscription, which ride the same presence grant,
  do. A wildcard presence selector, an inspection of `*/*` or a binding
  over `vehicle-01/*`, therefore joins each provider's egress grant
  (§11.2). It does not join the provider's ingress `reply`: a descriptor
  is answered with a value, checked against its own key. Measured: without
  it, a wildcard descriptor read is refused at the provider, while the
  liveliness read still answers. `[Sc: security.md §1, §2]`
- **An archive principal** has Own on its own prefix, plus Consume on what it
  records, on each peer archive's form of it (§4.4), and presence on the
  owners and peers it reads.
- **The descriptor rides presence.** It shares the `@zk` subtree
  with the tokens, so the grant that reads one reads the other. A consumer
  resolves a token's `fp16` through it (§8.4), and a refused descriptor
  read is as silent as a refused token read.
- **History** (§2.5): a consumer's advanced subscriber MUST NOT declare a
  subscriber-detection token, because the token would sit under the
  provider's prefix, which no grant gives it. Its history grant covers
  publisher detection (liveliness reads of the `@adv` subtree) and the
  cache's queries. The reference consumer declares no advanced subscriber.
- **A union storage** (§2.6) serves `…/events/**` under other services'
  keys. That is the one cross-principal serving grant: Consume on what it
  stores, plus `declare_queryable` and `reply` on those keys. A deployment
  names it explicitly. The reference generator does not emit it at this
  version.
- **The namespace** (§1.6): grant key expressions carry the deployment's
  namespace, because the router sees full keys. The admin space (`@/**`)
  carries none.

`[Sc: security.md §1]`

### 11.2 Compiling grants

- **Under `default_permission: deny`**, a grant compiles to allow rules.
  A deployment SHOULD run `deny` where the router can afford the
  enumeration.
- **Under `allow`**, Zenoh does not evaluate allow rules. A grant compiles
  into denies of its complement, regenerated on every contract revision.
- **A query is admitted by inclusion** (Appendix B).
  A rule allows a query when its key expression includes the query's,
  never when the two only intersect. Under an allowed `@/*/router`, a
  query on `@/*/*` is refused; under an allowed
  `@/*/router/status/plugins/storage_manager/storages/**`, one on
  `@/*/router/**` is. A router refuses a query on ingress at the face it
  arrived on, before it routes it, with the final reply alone.
  `[Sc: security.md §4]`
- **Egress is checked by inclusion** against the query's or subscription's
  own key expression. A generator MUST therefore add to a provider's egress
  grant every consumer selector that intersects what the provider serves.
  A provider serves its descriptor, so this includes every presence
  selector that intersects its `…/@zk/**` and that its Own grant does not
  include: a reader's descriptor GET and subscription (above). Not
  for the provider's ingress `reply`: no refusal answers a descriptor
  read.
- **A value reply is checked against its own key, an error reply against
  the query's**. This was measured on zenoh 1.10.1. S14's "a reply
  is checked against the query's key" holds only for refusals, which carry
  no key (§5.2).
  - So a provider's value replies pass through its Own grant.
  - A generator MUST grant the consumer selectors above for the provider's
    ingress `reply` too, so that its refusals reach the caller.
- **An R2-narrowed grant has no complement by inclusion.** Under
  `allow`, the members of a template that no other principal names stay
  readable. `deny` is the posture that holds such a grant exactly. A
  generator warns.
- **Messages and flows.** §11.1 names actions; zenoh's access
  control names messages on a flow. Under `deny`, each grant compiles to
  these allow rules. Both implementations measured them on zenoh 1.10.1:

  | Grant | Ingress (from the holder) | Egress (toward the holder) |
  |---|---|---|
  | Own | `put`, `delete`, `declare_queryable`, `reply`, `liveliness_token` | `query`, `declare_subscriber` |
  | Fan-in (a provider, §11.2 above) | `reply` | `query`, `declare_subscriber` |
  | Consume | `declare_subscriber`, `query` | `put`, `delete`, `reply` |
  | History (§2.5) | `declare_subscriber`, `query`, `declare_liveliness_subscriber`, `liveliness_query` | `put`, `delete`, `reply`, `liveliness_token` |
  | Presence | `declare_liveliness_subscriber`, `liveliness_query`, `query`, `declare_subscriber` | `liveliness_token`, `reply`, `put` |
  | Call | `query` | `reply` |
  | Contracts | `declare_queryable`, `reply`, `query` | `query`, `reply` |
  | Admin read (Tool) | `query` | `reply` |

  A liveliness read needs both of its rows. Without the egress
  `liveliness_token`, a reader's liveliness GET returns nothing.
- **The complement's key set.** Key expressions have no
  negation. Under `allow`, a grant's complement is taken over the
  deployment's own keys: each resource the enrolled services' contracts
  declare, each service's `…/@zk/**`, and the contract keys. Keys no
  contract declares stay open under `allow`.
  - **A principal's own keys.** A generator MUST deny, in each principal's policy, on each
    message and flow, the keys of the services it does not run that its
    grants do not name for that message and flow. A Consume grant on
    another service's key names it for reading and receiving only, so the
    principal's writes to it are denied (the table above).
  - It MAY leave the keys of the services the principal runs out of that
    principal's complement: P3 makes the principal their only writer.
    Under `allow`, the principal then also reads and receives its own
    keys on the messages and flows that its Own grant's rows do not name.
    Under `deny`, it gets those rows alone.
  - A generator that denies them too, per message and flow, is stricter,
    and conforms. The reference leaves them out.
  - The two differ only for the principal's own sessions, and for a
    subject that two principals' policies list. No grant shape of §11.1
    writes such a subject, and `link.v1` reports one on a face. No fixture
    pins either reading, since none carries a principal's complement;
    `link.v1`'s scenario §11, step 7, shows both. `[Sc: link.v1 scenarios §11]`
- **"Every principal" compiles into each principal's own policy.** In
  zenoh 1.10.1, a subject that matches every session (one with
  no attribute, or `link_protocols: ["tcp"]`) makes the per-user subjects
  lose their denies. This was measured. The `@/**` queryable deny (§11.1)
  is therefore a rule in each principal's policy, never one catch-all
  subject's.

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
- A refused liveliness read is answered complete and empty (observer.md
  §3.4). A deny
  on presence hides a service from a reader exactly as its absence would,
  so a reader the grants refuse attributes silence to absence.
- Any session can answer under `@/**`, a router's own keys included,
  unless the grants deny it (observer.md §4.2, "Who answered").
- **Under `allow`, a wildcard call is in no deny.** A
  `fanout = "allowed"` operation executes on every provider for a
  principal never granted it; only its answers are denied. For a
  deployment under `allow`, an operation that must not run for everyone
  MUST be `fanout = "forbidden"`, which its server refuses (O2), or the
  deployment runs `deny`.
- **Under `allow`, an ungranted wildcard GET gets nothing back**: each
  value reply is denied by its own key.
- **A session that matches no subject gets no policy**, so it gets
  `default_permission`, which under `allow` is everything. A deployment
  under `allow` MUST refuse unauthenticated sessions at the link.

---

## 12. Conformance classes

Conformance is claimed by role. Five of §0's roles have a **class** each:
that role's rules, and the static rules that bind every implementation
(§0). The static rules are those of §1, §7.5, §7.6, §7.7, §8.6 and §9, and those on what a
contract states in §2.2, §2.4, §2.5, §2.6, §3.1, §7.1 and §10. A rule of
any of these sections, §1, §7.5, §7.6, §7.7, §8.6 and §9 included, that names a role is that
role's. A rule elsewhere that names the
lint code checking it (§9.2) is static too, and so is §3.3's rule for a
descriptor checker. A class holds its role's rules in this text and in
[`observer.md`](observer.md).

**How a static rule binds.** A static rule binds an implementation where
it does what the rule governs. Such acts are parsing or building a key,
and parsing or writing an envelope or a call's form (§7.5), a packed
record (§7.6) or a frame (§7.7). So are reading a catalog or checking its
succession (§8.6), slugging a value, linting a contract, building or
verifying a bundle, checking a history and classifying a revision. A
rule on what a contract states binds it through the contracts it writes
and the lints it runs.
§9.8's publication rules bind whoever publishes a revision: its CI, and the
person who accepts it.

**The index.** [`RULES.md`](RULES.md) lists each class's rules stated with
MUST, MUST NOT, SHOULD or SHOULD NOT, generated from both texts. A class
also holds the rules its texts state without one, such as `observer.md`
§1, §2, §5 and §8.

**The classes.**
- **Owner**, **consumer**, **caller** and **deployment**: one class for
  each of those roles.
- **Observer**: the class of §0's **tool** role. It is named for
  `observer.md`, which holds most of its rules. The role keeps its name.
- **A reader's rules.** `observer.md` also binds any participant that
  reads presence, state or the admin space and concludes from it
  (observer.md §0). Those rules belong to the class of the role it plays
  there. A consumer's wait on presence (R5) is the consumer class's.

**Claiming classes.** An implementation claims one class for each role it
plays, and meets every rule of each class it claims. A rule that binds two
roles, such as O5's caller and tool, is in both classes. A service that
only publishes a stream plays the owner role alone. Its class is the
owner's rules and the static rules: in [`RULES.md`](RULES.md), the
entries under owner and implementation. Of the static rules, it meets those
that govern what it does (above).

**Full** is the level of a class claimed without the constrained modifier.
The union of the five classes is every rule of both texts. A rule binds
only the role it names, so an implementation at the full level meets the
rules of the classes it claims.

**Constrained** is a modifier on a class. A class claimed at the
constrained level holds every rule of the class, with these relaxations and
additions:

| Topic | Relaxation or addition |
|---|---|
| Namespace (§1.6) | The deployment prefix written literally in keys |
| Descriptor (§3.3) | An owner's descriptor MUST fit one fragment (Appendix B gives its size under zenoh-pico's default) `[Sc: constrained.md §5]` |
| Bundles (§8.4) | Served by a gateway holder where the device cannot receive or hold them. A zenoh-pico 1.10.1 owner sent 100 KB replies, so its limit is on receiving (Appendix B). |
| Faces (§8.5) | The far side attached as a client or as a south region, with batches within the lease, `@zk` denied, and `@stream` denied unless downsampled ([`link.v1`](profiles/link/v1.md)). Presence never crosses. |

`[Sc: constrained.md]`

**Profiles** stay opt-in, one class each (§10). An implementation claims a
profile's class by following the profile. A service lists it in its
descriptor's `profiles` (§3.3), and a tool answers its questions (its §5).
A profile's rules name the roles they bind, in core §0's words, as each
profile's §0 says. So a profile's class holds the profile's rules for each
role the implementation plays. No class of this text, at either level,
holds a profile's rules.

---

## 13. Open items

The items the design left open, and how each was settled. A decision
that changes a rule lands as an amendment.

| # | Question | Lean | Rule affected |
|---|---|---|---|
| U22 | Cutting the per-service token multiplier above the presence budget | **Decided:** a deployment-configured tokenless set, recommended for the framework interfaces every service implements; the descriptor records it (§8.1, §3.3) | §8.1 |
| U23 | The far side of a constrained face when it is more than one session: a site with its own router, or a service commanding many vehicles | **Settled:** the far router is placed in a `gateway.south` region of the near router, with the `@zk` deny (spike S3, U23 addendum; Appendix B) | §8.5 attachment |

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
- A router stamps only puts, where its timestamping is enabled, and
  never a delete or a reply: `treat_timestamp` reads a `PushBody::Put`
  alone (`dispatcher/pubsub.rs`). It re-stamps a future-dated put beyond
  the HLC delta (500 ms).
- Timestamping is enabled by default for a router, and disabled for a
  peer and a client (`zenoh-config`'s `defaults.rs`, `timestamping`). A
  runtime has an HLC only where it is enabled.
- A session with its HLC enabled stamps every put and every delete it is
  not given a timestamp for, a publisher's included, from its own HLC
  (`api/session.rs`, `resolve_put`).
- A put or a delete given a timestamp sends it, on a session without an
  HLC too (`resolve_put`), and so does a reply or a `reply_del`
  (`api/queryable.rs`, `_reply_sample`). A reply given none carries none.
  `Session::new_timestamp()` reads the HLC where there is one, and else
  the wall clock with the session's zid (`api/session.rs`).
- A link carries each priority in a transmission queue of its own,
  batched apart (`zenoh-transport`'s `common/pipeline.rs`, `stage_in` by
  priority). So two puts of different priorities can arrive in the other
  order, which `freshness.v2` §2.5's second heartbeat absorbs.
- A put's `reliability` is unstable API. `PublisherBuilder::reliability`
  and `PublicationBuilder::reliability` exist only under the `unstable`
  feature. Without it a put is sent `Reliability::DEFAULT`, which is
  `Reliable`. A publisher declared without a reliability is reliable too
  (`api/session.rs`, `declare_publisher` and `resolve_put`;
  zenoh-protocol's `core/mod.rs`; §2.4). This is read from zenoh 1.10.1's
  source, not measured. The reference builds zenoh with `unstable` (the workspace's
  `Cargo.toml`), and sets it on every publisher and every event's put.
- The admin space is disabled by default (`adminspace.enabled: false`).
- A zid is written in lowercase hexadecimal without leading zeros.
- A timestamp carries its HLC's id, the zid. Its time is an NTP64 value,
  whose low 32 bits are a fraction of a second, so its unit is 2^−32 s.
  It counts from the Unix epoch, 1970-01-01T00:00:00Z: an HLC's physical
  clock is its host's wall clock (uhlc's `system_time_clock`).
- A timestamp travels as its NTP64 time, a varint of at most 9 bytes,
  then its id's length and its id's significant bytes. A time in 2026
  takes 9 bytes, so a stamp with a 16-byte zid is 26 bytes
  (zenoh-codec's `core/timestamp.rs`). A put leaves out the Encoding
  `zenoh/bytes`, which is the empty one. A predefined Encoding without a
  schema is 1 byte (`zenoh/put.rs`, `core/encoding.rs`). An attachment
  under 128 bytes costs 2 bytes over its own: its extension's header and
  its length (`common/extension.rs`). The stamp's 26 bytes and the
  Encoding's 1 were measured in a put on zenoh-modem's simulated radio
  link (`docs/zk2/measurements/wire-baseline/`, "W1 on RF", finding 3).
  §7.5 compares the envelope with them.
- A node with its HLC enabled updates it with the stamp of every put it
  routes, a put a session receives for its own subscriber included. Its
  clock moves to a later stamp, and a stamp more than its delta ahead of
  its clock is refused, the put re-stamped or dropped
  (`dispatcher/pubsub.rs`, `treat_timestamp`; uhlc 0.8's
  `update_with_timestamp`; §4.1). It stamps from its own HLC each put it
  routes that carries no stamp (`treat_timestamp`: "Timestamp not
  present; add one"; §4.1). Measured by the second implementation, a
  client whose HLC is on received 20 of 20 samples of an unstamped owner,
  through a router that stamps nothing, stamped with its own zid.
- Timestamps order by time, then by id. An id is stored as the 16 bytes
  of its 128-bit value little-endian, and compared as that byte array
  (uhlc 0.8's derived `Ord` on `Timestamp` and `ID`), so its least
  significant byte decides first. `Latest` keeps the reply whose
  timestamp is greatest in that order, an unstamped one lowest, and a
  later one on a tie, so the last to arrive among unstamped replies. It
  delivers them at the final reply (`api/session.rs`). Checked against
  zenoh's own `Timestamp` by `zenkey-fleet`'s `bus::seed` tests.
- An advanced publisher with a cache and no sample numbering orders its
  history by its session's timestamps, and is refused at its declaration
  on a session with no HLC (zenoh-ext's `advanced_publisher.rs`, `cache`
  and `Sequencing::Timestamp`). With sample-miss detection it numbers its
  samples instead, and needs no HLC (§2.5).
- A numbered sample carries a SourceInfo extension: the publisher's zid,
  its entity id and the number. With a 16-byte zid that is 21 bytes for a
  number below 128, and up to 25 (zenoh-codec's `zenoh/mod.rs`,
  `SourceInfoType`).
- zenoh's storage manager refuses to start on a session without
  timestamping, and so runs no storage there
  (zenoh-plugin-storage-manager's `lib.rs`; §8.2).
- A reply error carries a payload and an encoding, and no key expression.
  `Reply::replier_id` is behind the `unstable` feature. It names the session
  that sent the reply, whatever key the reply is on. §4.2's admin-space
  verification rests on it: a zenoh release that removed or changed it
  would leave every admin answer unverified, and the checks that read the
  admin space unobservable, never clean.
- The storage manager garbage-collects tombstones, not values: an event
  occurrence a union storage holds is never pruned by its retention.
- A router's admin document (`@/<zid>/router`) lists its sessions, each
  with the peer's zid and `whatami`: `router`, `peer` or `client`.
- A node's admin space answers its declaration records
  (`@/<zid>/<whatami>/{subscriber,publisher,queryable,querier,token}/**`),
  its `linkstate/*` and a `route/successor/src/<zid>/dst/<zid>` key while
  it holds the routing tables' read lock, and each reply takes that read
  lock again. Rust's std leaves the `RwLock`'s priority policy to the
  platform; on Linux its futex lock makes a new reader wait behind a
  waiting writer.
  There, a declaration, an interest or a new session from
  another face between two replies deadlocks the node: every put, query
  and declaration through it stops for good (`net/runtime/adminspace.rs`,
  `resources_data`; `dispatcher/queries.rs`, `route_send_response`; the
  same on zenoh's main; §4.2;
  `docs/zk2/upstream/adminspace-reply-under-tables-read-lock.md`). Its
  root document (`@/<zid>/<whatami>`), its metrics and the plugins' status
  are answered without the lock held across a reply.
- Access control decides a message by inclusion: a rule applies when its
  key expression includes the message's (`interceptor/authorization.rs`,
  `policy_decision_point`, `nodes_including`). A request refused on
  ingress is answered at once with its final response alone, and never
  routed (`primitives/demux.rs`, `handle_message`), so a refused admin GET
  never reaches the admin space (§11.1).
- A query that sets no timeout waits `queries_default_timeout`, 10 s by
  default.
- A client connects to one endpoint at a time.
- A link's batch is the minimum of the configured size, the MTU and the
  other end's.
- A query timeout arrives as a reply error. A liveliness GET's too: one
  that reaches its timeout ends with the error reply `Timeout` (encoding
  `zenoh/string`), and one the routers finished ends with none.
- A router that routed a query answers it when the query's timeout ends
  before the queryable's final reply. Its answer is a reply error of its
  own, the 7 bytes `Timeout` under the empty Encoding, `zenoh/bytes`
  (`dispatcher/queries.rs`, `QueryCleanup`). The querier's own session
  ends a query at its timeout with `Timeout` under `zenoh/string`
  (`api/session.rs`). The `zenoh` crate and its storage manager send no
  other reply error of their own. This is read from zenoh 1.10.1's
  source, not measured. §5.2 tells both from an owner's.
- A client's or a peer's runtime ends a liveliness GET its routers have
  not finished at `routing/interests/timeout` (10 s by default) with the
  final reply alone and no error reply, before the GET's own timeout when
  that is longer (`dispatcher/interests.rs`, `CurrentInterestCleanup`;
  §8.1).
- A liveliness GET whose caller drops it, times it out or cancels it is
  answered in full all the same: the session sends its router nothing
  (`api/session.rs`, `liveliness_query`, `cancel_liveliness_query`). The
  runtime ends it at `routing/interests/timeout`, and logs each reply that
  arrives after that `Unknown interest` (`hat/client/interests.rs`;
  §8.1; `docs/zk2/upstream/liveliness-get-dropped-is-answered-in-full.md`).
  A GET its own timeout ends is handed an error reply, `Timeout`, and its
  final reply, and nothing after them (`api/session.rs`,
  `liveliness_query`); its router answers it all the same, and a GET the
  session asks meanwhile waits behind it (§8.1).
- A client whose transport opens again declares its session's interests
  again in `CurrentFuture` mode, whatever mode they were declared in
  (`hat/client/interests.rs`, `repropagate_interests`): a liveliness
  subscriber without history is told every token it matches, those
  declared while the transport was down included, and none withdrawn
  then. A liveliness GET on that session asked after the reopen ends
  after that telling (§8.1).
- A router handles one session's interests in turn: a liveliness GET's
  final reply follows the registration, and the history, of every
  subscriber's interest the session sent before it, whatever the GET
  matches. A router resolves a client's interest itself and ignores one
  from another router (`dispatcher/interests.rs`,
  `hat/router/interests.rs`); one access control refuses is answered with
  its final reply at ingress, in turn (`net/primitives/demux.rs`;
  §8.1).
- A liveliness subscriber without history is told a token's withdrawal
  by the token's key though it was never told the token
  (`hat/broker/token.rs`, `maybe_unpropagate_token`), and a session in a
  namespace does not strip its namespace from that key
  (`routing/namespace.rs`, `ENamespace`), so it hears none (§8.1).
  A session in no namespace subscribed to the wire key, the namespace in
  front, hears it (§8.1).
- A liveliness GET that access control refuses is answered with the final
  reply alone: no token and no error reply. The session's own tokens can
  be in it all the same, answered by the session itself (§8.1). The
  session sends the GET to its router before it hands itself that answer
  (`dispatcher/interests.rs`, `Face::interest`; `dispatcher/face.rs`,
  `send_interest`). A final reply that arrives first ends the GET without
  them (`api/session.rs`, `WeakSession::send_declare`).
- A router propagates no liveliness token from one of its peers to
  another: under the `auto` gateway its peers are one region, expected to
  be linked to each other (`hat/peer/token.rs`).
  `routing.router.peers_failover_brokering` is deprecated, and has no
  effect (`zenoh-config`; §8.1).
- A session with no transport to another zenoh node has every liveliness
  GET answered at once with the final reply alone, and every GET
  ([`scenarios/presence.md`](scenarios/presence.md) §7). A transport that closes withdraws, at the reader, every token
  learned through it, while their owners still hold them (§8).
- Under `allow`, allow rules are not evaluated, and access control works by
  inclusion.

**Keys across a face** (§8.5, observer §7.5). Read in zenoh 1.10.1's
`src/`, each with its file and line:
1. **A put toward a face** rides the deepest key expression either end
   declared on that face for a prefix of its key, and sends the rest as
   text. Where neither end declared one, it carries the whole key.
   - The route's key is `Resource::get_best_key`'s
     (`net/routing/dispatcher/resource.rs:740`). A data route calls it
     toward a client (`hat/broker/pubsub.rs:268`) and toward a peer
     (`hat/peer/pubsub.rs:258`).
   - A declaration the far side made wins over the router's own at the
     same resource: `get_wire_expr` reads `remote_expr_id` before
     `local_expr_id` (`resource.rs:742–758`).
   - A scope id is a varint, 1 byte below 128 and 2 below 16,384
     (zenoh-codec's `core/wire_expr.rs:42`).
2. **A session declares a key expression on its own face** for three
   things:
   - its publisher's whole key (`api/builders/publisher.rs:484`);
   - its subscriber's non-wild prefix (`api/builders/subscriber.rs:215`);
   - its queryable's non-wild prefix (`api/builders/queryable.rs:231`).

   Each goes through `Session::declare_prefix` (`api/session.rs:1491`).
   A non-wild prefix ends before the chunk holding the first `*`
   (zenoh-keyexpr's `get_nonwild_prefix`). So a publisher's declaration
   shortens only the first hop, from the owner to its router.
3. **A router declares a prefix toward a face** only while it propagates
   a subscriber, a queryable or a token there. It does so only toward a
   north-bound face, or one whose remote asked for key expressions in an
   interest (`Resource::decl_key`, `resource.rs:661–729`). No router
   propagates a publisher, so none declares a stream's key toward the far
   side. Under link.v1's two attachments the far side is south or a
   client, and its own declarations are the prefixes that exist there.
4. **Cached routes.** A prefix the far side declares disables the cached
   data and query routes of its resource and its matches (`register_expr`,
   `resource.rs:1005`). So does a subscriber's registration
   (`dispatcher/pubsub.rs:72`). A prefix the router declares itself
   (`decl_key`) disables none. A put whose route was computed before that
   declaration keeps its text for the route's life.
   - Seen on zenoh-modem's simulated radio: config.v1's re-puts carried
     their key's text all run, put before the router declared their scope
     ([`wire-baseline`](../docs/zk2/measurements/wire-baseline/README.md),
     W2 finding 4).
   - The draft upstream report, not filed:
     [`router-prefix-keeps-cached-routes.md`](../docs/zk2/upstream/router-prefix-keeps-cached-routes.md).
5. **A reply always carries its key as text.** A router rewrites each
   reply's key expression to the whole key, with no scope
   (`dispatcher/queries.rs:582–590`). Its comment reads "TODO: consider to
   optimize keyexpr for 2.0". No declaration shortens an S4 reply, a
   re-read or a call's reply across a router.
   - The draft upstream report, not filed:
     [`replies-carry-their-key-as-text.md`](../docs/zk2/upstream/replies-carry-their-key-as-text.md).
6. **A lost `DeclareKeyExpr` is never sent again.** The far router then
   drops every put on that scope, and logs "Route data with unknown
   scope" for each (`dispatcher/pubsub.rs:249`).
   - Measured on zenoh-modem's simulated radio: 25 of 25 health puts were
     dropped over 12 minutes (the wire baseline's "A declaration lost in
     the setup is never repaired").
   - A far-side prefix at the same resource heals the puts, by fact 1. It
     does not heal a queryable whose declaration was lost.

**A lost declaration and a late reply** (§8.1, §8.5, observer §7.5).
Read in zenoh 1.10.1's `src/`, and in `zenoh-transport`'s:
1. **A declaration is sent once.** A session sends a subscriber's
   declaration on its face once, with no acknowledgement
   (`api/session.rs:1788`). A transport's receiver takes a frame whose
   sequence number is past the next one expected as the next, and reads
   no gap as a loss (`zenoh-transport`'s `unicast/universal/rx.rs:222–241`,
   `roll`). So a frame a medium lost below zenoh takes its declarations
   with it, and the transport stays up.
2. **A subscriber beside a twin sends nothing.** A subscriber on the key
   of one the session holds joins it, and sends no declaration
   (`register_subscriber`, `api/session.rs:398–450`). So only a
   subscriber declared after the silent one was undeclared sends a
   `DeclareSubscriber`.
3. **A prefix is declared toward a router once a transport.** The
   session's runtime declares a subscription's non-wild prefix toward its
   router the first time it propagates one there. It keeps the mapping
   in the face's `local_mappings` (`Resource::decl_key`,
   `net/routing/dispatcher/resource.rs:661–729`). That reference keeps
   the resource from ever being cleaned (`Resource::clean`, `:535–561`).
   So a lost `DeclareKeyExpr` is never sent again on that transport.
   - Measured by the reference: a subscriber undeclared and declared
     again rode the lost scope, its `DeclareSubscriber` and no key text,
     and heard nothing.
4. **A late reply is dropped.** A router answers a query it routed
   `Timeout` at the query's own timeout, and forgets the query
   (`net/routing/dispatcher/queries.rs:476–515`). A reply that comes after
   is dropped and logged "Route reply: Query not found!" (`queries.rs:623`).
   - Measured on zenoh-modem's simulated radio: GET replies came back
     3.3–7.4 s after their GET, where the reference's defaults were 1 s
     and 5 s (the wire baseline's W2 findings 6 and 7).
   - The draft upstream report, not filed:
     [`declarations-are-never-repaired.md`](../docs/zk2/upstream/declarations-are-never-repaired.md).

Measured by [`scenarios/constrained.md`](scenarios/constrained.md) §8 and
§9 (`zenkey/tests/constrained.rs`, `zenkey-fleet/tests/redeclare.rs`):
- a call whose reply took 1.5 s drew none at its contract's 1 s. Across a
  face whose reply timeout is 3 s, it drew the value;
- a subscription whose `DeclareSubscriber` a proxy dropped heard nothing
  for two horizons. Declared again once, it heard the stream from then
  on. With ARQ, it stayed silent;
- one whose `DeclareKeyExpr` the proxy dropped stayed silent, declared
  again.

Measured by [`scenarios/constrained.md`](scenarios/constrained.md) §7
(`zenkey/tests/constrained.rs`), through a TCP proxy toward a far client:
- a stream put crossed in 45–46 B under its exact key, with none of its
  key's text;
- the same put crossed in 82 B under `zk2/**`. That is its key's 35 B
  after `zk2`, and a byte of length, more;
- a templated member's put crossed in 52 B under the template's literal
  prefix, carrying `/roll`;
- a GET's reply carried its whole key, though the far client had
  subscribed to it exactly.

**Measured for the rules above**, each with the rule it serves:
- **State's and events' QoS** (§2.4): spike S5 lost 10,917 of 100,000
  values put with `drop` on their way to a storage.
- **`express`** (§2.4): spike S9 measured `real_time` + express losing
  0.6–4 % of samples at 1–5 kHz on loopback, with no latency gain.
- **A call with many replies** (O6): `Latest` and `Auto` kept 1 reply of
  10 in spike S6.
- **The wait for presence** (§8.1): counted from a client's connection to
  the router that is the owner's session, zenoh-python saw the reference
  owner's tokens within about 1 ms.
- **The presence budget** (§8.3, spike S2): a router holds 2.1–2.4 KiB per
  token, and a router-to-router link carries 63–85 B per declaration.
  Below 10,000 tokens discovery took under 2 s (under 50 ms at 1,000). At
  10–15k tokens it took 2–4 s. At 50k it took 46–49 s, or never finished.
- **Retrieval's consolidation** (§8.4): with zenoh's default consolidation
  on a concrete key, the corrupt reply alone arrived, at 2 s, and the
  valid one never (`scenarios/retrieval.md` §2).
- **A far router in a south region** (§8.5, U23; spike S3, U23 addendum):
  200 denied tokens cost 506 B, against 11.3 KB router to router.
- **A batch against the lease** (§8.5): at 2,400 bit/s with the 10 s
  lease, the spike's 1,024 B batches took 3.41 s to cross, a little over a
  third of the lease, and the link held. With zenoh's 64 KB default
  batch, the link reconnected in a loop.
- **The admin read under `allow`** (§11.1): zk2py's GET of
  `@/*/router/queryable/**` under either generator's `allow` block was
  admitted and answered.

**Measured for observer.md's reads**, each with the section it serves:
- **Handlers** (observer §3.1): on a session holding a liveliness
  subscriber, a liveliness GET with zenoh's default 256-slot handler hung
  at every size measured from 996 tokens (zenoh#2678). A bounded
  subscriber nobody drained starved a callback GET on its session: with
  zenoh-python 1.10.1, the GET ended at its timeout with 257 of 2,002
  tokens, and said nothing.
- **The interests timeout** (observer §3.3): measured by the reference, a
  read with a 110 s timeout beside 10,000 tokens came back at 10 s, with
  no error reply, holding 6,121 of them. The same read was whole, in
  22 s, under a 120 s interests timeout.
- **A refused read** (observer §3.4): measured with a `liveliness_query`
  deny on the query's ingress at the router. On `egress` alone, the same
  rule refused nothing.
- **The links check** (observer §3.5): measured by the reference,
  `transports()` missed the cut in 4 of 10 runs of `presence.md` §7 step
  5, and `links()` in none of 20.
- **A watch session's history** (observer §3.7): with history, its round
  trip took 23 s beside 10,000 tokens in a debug build (the reference),
  and without it 6 ms there. In a namespace with history, a release
  build took 3.1–5.9 s (zk2py, under load).
- **The wire keys** (observer §3.7): a watch session in no namespace, its
  subscribers without history on the wire keys, made its round trip in
  1.2 ms beside 2,000 tokens in `ns`, and in 44 ms beside 10,000, debug
  builds.
- **The round trip after a reopen** (observer §3.7): beside 10,000 tokens
  in `ns` it took 3.9–4.1 s in a release build (zk2py: 3.2–4.7 s) and
  24 s in a debug one, where the round trip at the session's start took
  49–66 ms; beside 2,000, 0.15 s and 0.75–1.5 s. The next was one round
  trip again (zk2py, release: beside 10,000 tokens 3.7–4.2 s after the
  reopen, then 0.8–1.7 ms; beside 2,000, 145–146 ms, then 0.3–0.8 ms).
- **A session's first read** (observer §3.7): beside 10,000 tokens a
  session's first read took 3.3–4.2 s and a later one 0.52–0.58 s
  (zk2py, release). zk2py's follower seeded in 3.1–3.9 s, and its next
  read took 0.47–0.49 s. The round trip after a reopen ran to 4.54 s
  beside those seeds' 3.91 s.
- **One read at a time** (observer §3.7): measured by the reference
  before it kept to it, a reader whose caller dropped its call every
  250 ms, beside 10,000 tokens whose read took 0.5–3 s, asked 450 reads
  in 120 s and folded none, and never read its scope complete again
  after a 3 s cut.
- **A GET behind one its timeout ended** (observer §3.7): beside 10,000
  tokens, a GET waited 258–280 ms after a read its own 0.2 s timeout
  ended on a session that had read them before, and 2.6–3.4 s after a
  session's first read, against 6–10 ms on a quiet session (zk2py).
- **A read on a token's own session** (observer §3.8): through a router
  whose access control refuses `liveliness_query` on ingress, the
  holder's own read was always complete, holding its token or nothing.
  The same read on another session of the same router was complete and
  empty every time (the reference, `zenkey/tests/presence.rs`,
  `a_refused_read_on_a_tokens_own_session_can_return_it`). Over 40 runs
  of 20 reads, the holder's read held its token in 799 of 800 on an idle
  host. It held it in 733 of 800 with two busy loops on the test's two
  cores.
- **Peers behind one router** (observer §3.9): two peers behind one
  router, neither listening, one holding a token. The other's read was
  complete and empty in every one of six reads over 3 s, with gossip on
  and with it off, while a client of the same router read the token.
  With both listening, and gossiping through a router that gossips, they
  linked and the token was read (the reference,
  `a_peers_read_through_a_router_misses_an_unlinked_peers_token`).

**One binding's facts**, which a rule's wording allows for:
- zenoh-pico 1.10.1 has no session namespaces (§1.6).
- One fragment is 4 KB with zenoh-pico's default `Z_FRAG_MAX_SIZE` (§12,
  the descriptor).
- A zenoh-pico 1.10.1 owner sent 100 KB replies, so its limit on bundles
  is on receiving (§12).
- zenoh-python builds an NTP64 from seconds and nanoseconds, so its
  smallest step above a timestamp is 1 ns (§4.3, a tick).
- zenoh-python 1.10.1 has no unbounded handler, so a callback is its way
  to meet observer §3.1.
- zenoh-python 1.10.1 marks `Reliability` unstable too, and `Session.put`
  takes none. Measured by zk2py, a publisher declared without one reads
  back reliable, with congestion `drop` and priority `data` (§2.4).
- In Rust, `Session::config()` reads an open session's interests timeout;
  zenoh-python 1.10.1 gives an open session no accessor for it (observer
  §3.3).

**What a path drops, and what a reader sees** (§2.7). Measured on
2026-10-10 by [`scenarios/budget.md`](scenarios/budget.md) §1–§3
(`zenkey/tests/budget.rs`), with the source each rests on:
- A sample that is `reliable` with congestion `block` is not droppable. A
  full queue makes its push wait, and one it cannot queue within
  `wait_before_close` closes the transport (`unicast/universal/tx.rs`).
  A best-effort sample, or one with `drop`, is droppable once
  `wait_before_drop` passes. zenoh 1.10.1's defaults are 5 s and 1 ms.
  Through two routers in processes of their own, to a reader that took
  2 ms over each 16 KiB sample:
  - reliable, `block`: 1,000 of 1,000 arrived in order, and the owner's
    put waited;
  - best effort, `drop`: 695–832 of 1,000 were lost, and nothing the
    reader received said so;
  - reliable, `block`, the reader's callback stuck for 7 s on one sample:
    433–786 were lost in 27 runs that recorded what follows. The
    reader's own transport closed in
    every one. It saw the owner's instance token withdrawn only when the
    token had reached it before it stalled: in each of the 23 runs where
    it had, never in the 4 where it had not. On 2026-10-11 six runs that
    did not record it lost 661–792, and saw no withdrawal in 2. A ticker on a thread of the reader's own
    process saw no gap above 107 ms meanwhile: only zenoh's receive
    thread was held;
  - reliable, `block`, the reader's whole process stopped (SIGSTOP) for
    7 s, the owner's token received first: 421–765 were lost in 13
    runs. Each time its ticker saw a gap of 7.0–7.1 s on its first tick
    after, its session read its transport's close (0.35–0.51 s later in
    the 4 runs that timed it), and it saw the token withdrawn.
- **Another binding's default.** zm-1.10.2-r1, the zenoh fork that
  zenoh-modem builds on, defaults `wait_before_close` to 10 s, its lease.
  A 7 s stall falls inside both there. Measured on 2026-11-01, a reader
  stuck 7 s lost none of 1,000, and no transport closed, in 3 of 3 runs.
  With the routers stating 5 s, the stuck reader lost 653 and the stopped
  one 876, each with its transport closed. So `budget.md` §1's routers
  state their `wait_before_close`.
- A transport that closes withdraws the tokens learned through it, between
  routers and on a client's own link alike. The withdrawal reaches the
  reader within 1–2 ms when the link closes, and one lease after a link
  goes silent: 986–1,037 ms at a 1 s lease, 9,961–10,047 ms at zenoh
  1.10.1's default 10 s (spike S10). The samples put meanwhile are lost.
- **Only a token the reader had received is withdrawn.** A
  router that gives up on a stalled reader closes the reader's transport
  and discards its queue toward it. In the runs above with no
  withdrawal, the reader read everything its router had sent before the
  close, and the owner's token was not among it. Its session saw the
  token only after it reconnected, as a new token. A run with more than
  one withdrawal had another transport close beside the reader's: the
  owner's own push had waited `wait_before_close` too, and its session
  closed its transport to its router.
- **Not measured: a withdrawal later than one lease, under a
  `wait_before_close` longer than the lease.** Every run above had a
  `wait_before_close` shorter than its lease, or a link that closed or
  went silent. Observer §5.2's watch after a window allows for such a
  withdrawal all the same. It reasons from the push above, which waits
  `wait_before_close` before its transport closes. On zm-1.10.2-r1, a
  lease below 10 s beside the default `wait_before_close` is this case.
- A session sees its own transports open and close through zenoh's
  transport events (`Session::info().transport_events_listener()`,
  unstable). A close is reported once the session's receive task has
  read it, after a stalled callback returns (`unicast/universal/
  transport.rs`, `delete`), and a link's removal as soon as its task
  fails (`del_link`, `link.rs`).
- **A close's withdrawals come before its transport event.** A closing transport's links go first, then its
  routing face, which tells the session's liveliness subscribers each
  token learned through it withdrawn, then the transport itself, whose
  close the transport events listener hears (the order observer.md
  §3.5's cut-off check reads). Measured by the reference
  (`zenkey/tests/presence.rs`,
  `f268_a_close_tells_its_withdrawals_before_its_transport_event`): a
  watch session through a link the test cut and healed, its subscriber
  without history on three interface tokens, heard each close's three
  withdrawals 0.025–0.51 ms before its listener heard the close. It did
  so at 40 of 40 closes in eight runs, and held no link when each was
  heard. Its first close, before it had been told the tokens, told
  nothing. On zenoh-python 1.10.1, zk2py measured 0.15–0.5 ms.
- **An open is told before the subscribers' interests are declared again.**
  A transport that opens is handed to the session's handlers
  first, the transport events listener's among them, and only then to the
  router, whose `new_transport_unicast` makes the face and re-propagates
  the subscribers' interests (`net/runtime/mod.rs`, `new_unicast`;
  `hat/client/mod.rs`). Measured by the reference (`zenkey/tests/presence.rs`,
  `a_round_trip_asked_at_the_open_event_can_precede_the_reopen`): a round
  trip asked at the open event stood before the reopen's telling at 6 of
  60 reopens in six runs (0 to 2 of each run's 10). It stood in 0.24 ms,
  where one that reached the router took 41–45 ms. One asked 100 ms after
  the event followed the telling at 60 of 60.
- A reliable sample takes any link when no reliable one is up: over a UDP
  link alone it travels best-effort (link selection in
  `unicast/universal/tx.rs`).
- zenoh-ext's sample-miss detection takes a publisher's first sample heard
  as its start (`advanced_subscriber.rs`, `handle_sample`). A subscriber
  declared after a publisher's first five samples heard the sixth or the
  seventh, and reported no miss. A gap after a sample heard was reported.
  The periodic heartbeat is published with congestion `drop`.
- The sessions of one process share zenoh's receive threads, two by
  default (`ZENOH_RUNTIME`). With two routers and a reader whose callback
  blocks in one process, 3 of 6 runs lost 674–970 of 1,000 reliable,
  `block` samples. The routers' transport closes stalled, and no token was
  withdrawn while the runs lasted.

**A liveliness GET beside a liveliness subscriber.** Measured
on 2026-10-24 by the zenoh-only reproduction in
[`docs/zk2/upstream/liveliness-get-repro/`](../docs/zk2/upstream/liveliness-get-repro/),
61,920 trials per version on zenoh 1.10.1 and on zenoh's main branch at
`766b455`, each a fresh process with a router, a token holder and a
reader, clients, some under CPU load:
- A complete liveliness GET from a session holding liveliness subscribers
  that covered the live tokens missed one or more of them:
  - history subscribers on the reading session: 1,656 of 23,040 (1.10.1),
    1,647 of 23,040 (main);
  - subscribers without history there: 159 of 16,560 and 167 of 16,560,
    only for tokens declared after they subscribed;
  - the subscribers on another session of the same process: 0 of 12,240;
    no subscriber: 0 of 10,080.
- A GET issued right after declaring a history subscriber on the same
  session missed nearly every time: 720 of 720 (1.10.1) and 719 of 720
  (main) in the reproduction; zenoh-python 1.10.1 held the token in 7 of
  120 (zk2py). It is the race below, which the session's own
  answer can win.
- **Why**, shown by traces and by a one-condition patch that took both
  races to 0 of 6,480 trials: the GET is a `Current` token interest,
  answered only by declarations carrying its id. The session's own routing
  answers it from the tokens it already knows — those its liveliness
  subscribers asked for — and the router drops its answer for every token
  already declared to the session's face for a subscriber
  (`hat/broker/interests.rs`, `propagate_current_token`, `local_tokens`).
  A token still on its way to the subscriber is in neither answer; and the
  router's final reply can overtake the session's own answer, more often
  under load.
- A second GET at once still left up to 10 of 720 trials wrong under load;
  waiting longer before the GET did not remove the overtaking race. A
  fresh session found every token every time.
- After the subscriber is undeclared the router keeps those declarations
  (`unregister_interest` leaves `local_tokens` as it is; read from the
  code, not measured), so a session that ever held one stays exposed.
- Ordinary GETs are not on this path.
- The draft upstream report, not filed:
  [`docs/zk2/upstream/liveliness-get-misses-beside-liveliness-subscriber.md`](../docs/zk2/upstream/liveliness-get-misses-beside-liveliness-subscriber.md).

## Appendix C. Evidence index

The evidence of the rules in [`observer.md`](observer.md) is indexed
there too (its Appendix B). The rows below keep the core
section each was written against, whose pointer leads on.

| Evidence | Covers |
|---|---|
| `conformance/keys.json`, `slugs.json`, `templates.json` | §1, §2.2 |
| `conformance/contracts/` | §2, §7, §9.1–§9.5, §10 |
| `conformance/sets/` | §9.2 (E035, E036) |
| `conformance/bundles/` | §9.6 |
| `conformance/history/` | §9.7 |
| `conformance/compat/` | §9.7 retention, §9.8, R4 |
| `conformance/descriptors/` | §3.3 |
| `conformance/budget/` | §2.7, `lossless.json` and the reader's stall included |
| `conformance/seed/` | §4.2 a late joiner's merge: `merge.json` the Zenoh timestamps' order, unstamped samples, and a tool's across two ids; `inband.json` §4.3's in-band stamps, plain and serial order, and a tool's and a consumer's across two runs |
| `conformance/errors/` | §5.2: `cases.json` the envelope, `forms.json` §7.5's error form and a router's `Timeout` |
| `conformance/envelope/` | §7.5, a call's forms included |
| `conformance/frame/` | §7.7 |
| `conformance/catalog/` | §8.6 |
| `scenarios/grammar.md` | §1.3, §1.6 |
| `scenarios/state.md` | §4; §1 in-band stamps through a router that stamps nothing and one that stamps, and §8.2's session posture; §7 runs, a record's and a random draw's, the run before the seq wraps, and a consumer's and a tool's late joiner across two runs, after their seed and inside it; §9 an archive within a run and across two; §10 a late joiner's seed; §11 its merge's open cases; §12 S4's check reads beside a router that is declaring, where its recorder can hear a read that freezes a node; §13 a stream key's seq wrapping, and kept across a takeover |
| `scenarios/operations.md` | §5, §6 |
| `scenarios/presence.md` | §1.5, §3.2, §3.3, §8.1–§8.2; §4 the handler rule on the watch session's round trip; §7–§9 what a complete presence read is: a session cut off, before a read or while it runs, a withdrawal heard live, a read beside a liveliness subscriber, each observing the reader's own verdict; §10 the round trip at a population, its GET matching no token, subscribers without history and a namespace's, a round trip lost with its transport, and a read that runs to the interests timeout; a watch session in no namespace on the wire keys, a round trip through a watch session cut off, and a move unconfirming what a reader holds; a reopen telling every token, the round trip after it waiting for them, and one round trip and one read at a time; step 10's counts, the reader's own sessions'; what a black box is judged on there, at what period its output is sampled, and one that prints nothing while its link is cut; Appendix B |
| `scenarios/bindings.md` | §3.2; §6 a binding's order kept by the descriptor |
| `scenarios/retrieval.md` | §8.4 |
| `scenarios/types.md` | §2.4, §7.1–§7.2 |
| `scenarios/security.md` | §11; §4 the admin read admitting no GET of a node's declaration records, and a query admitted by inclusion |
| `scenarios/constrained.md` | §1.6, R7, §8.5, §12; §7 keys across a face, and Appendix B; §8 a reply across a face (§8.1); §9 a declaration lost across a face, declared again (§8.5) |
| `scenarios/budget.md` | §2.7: what a path drops and a reader sees (§1–§3), an owner keeping its bound and its rate (§4–§7); a stalled reader, and a tool's window over one (§1); Appendix B |
| `profiles/<name>/conformance/`, `profiles/<name>/scenarios.md` | each profile's own rules (§10; [`profiles/README.md`](profiles/README.md)) |
| `profiles/freshness/scenarios.md` §1, §2 | §4.2 a re-put, §4.3 an owner ahead |
| `profiles/health/scenarios.md` §1, §4 | §8.2 a status before the tokens, §4.3 an owner ahead reported by a stream |
| `profiles/link/scenarios.md` §1–§6 | §8.5 a face compiled and enforced, both attachments, `@stream` downsampled, an event that must not cross; S3 a window raised to the outage; R7 liveness across a face |
| `profiles/desired/scenarios.md` §1, §3, §6, §7, §10 | §4.2 a late joiner seeding again when its owner's instance token appears; S3 a delete answered after an outage within the window, and lost past it; §4.3 a new instance's re-puts under a fresh zid; S5, S6 last-known from an archive, the owner absent, and an owner silent within the timeout across a cut, its own answer after the heal (§10) |
| `profiles/arbitration/scenarios.md` §3, §4, §7, §9 | §3.2, §3.3 a binding's order kept by the descriptor, and read as a priority (§9); §8.1 a withdrawal heard live acted on without concluding absence, and undone by a token or a read (§3, §4), and what a move of the watch session makes a reader hear held back until its round trip stands (§7); a close's withdrawals told before its transport event, and a move begun at the first (§7's control); the round trip after an open asked a margin after its event (§7) |
| `profiles/redundancy/scenarios.md` §1, §5, §7, §8, §11, §12 | §3.3 the standby mark; §6 a standby, and a tool reading the mark; §8.1 a standby holding no interface token; §8.2 a standby exposing nothing, taking over and stepping down in the steps' order, and a governed interface refused in the tokenless set; O1, O3 a call finding no standby; §8.1 a refused read on the claim's own session, and peers that link (§5) |
| `profiles/timing/scenarios.md` §4, §5 | §4.1 a stream sample stamped by its owner's HLC, by a router when its owner's session has none, and unstamped through a router that stamps nothing; §3.3 `meta.zid` attributing a stamp |

The compatibility cases (`compat/`) are evaluated by the reference
classifier. [`compat/README.md`](conformance/compat/README.md)
records where its protobuf verdicts depart from `buf breaking` with WIRE,
and why.

## Appendix D. Authoring fields by kind

Normative for §9.1. A field outside its kind's column is E014 (`history`:
E019; `frame`: E046).

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
| `frame` | plain, no parameter | — (E046) | — (E046) | — (E046) | no |
| `rate`, `retention` | — | — | required | — | no |
| `request`, `response` | — | — | — | required | no |
| `error`, `summary`, `idempotent`, `fanout`, `serving`, `replies`, `timeout_ms` | — | — | — | ✓ | yes, except `error` and `summary` |

**Published annotation vocabularies** (W105 outside them): the
profile's own table replaces its interim one.

| Profile | Keys | Published by |
|---|---|---|
| `freshness` | `ttl_s` | [`profiles/freshness/v1.md`](profiles/freshness/v1.md) §4 |
| `link` | `exposure` | [`profiles/link/v1.md`](profiles/link/v1.md) §4 |
| `alarms` | none | [`profiles/alarms/v1.md`](profiles/alarms/v1.md) §4 |
| `telemetry` | `kind`, `unit`, `buckets`, `semantic`; `telemetry.v2` §4 adds `series` | [`profiles/telemetry/v1.md`](profiles/telemetry/v1.md) §4, [`profiles/telemetry/v2.md`](profiles/telemetry/v2.md) §4 |
| `desired` | `target_param` | [`profiles/desired/v1.md`](profiles/desired/v1.md) §4 |
| `redundancy` | `election` | [`profiles/redundancy/v1.md`](profiles/redundancy/v1.md) §4 |
| `timing` | `period_ms`, `deadline_ms`, `lifespan_ms` | [`profiles/timing/v1.md`](profiles/timing/v1.md) §4 |
| `arbitration` | `policy`, on a requirement | [`profiles/arbitration/v1.md`](profiles/arbitration/v1.md) §4 |

`link.downsample_ms` left with the interim `link` row: a face's
rate is its face policy's `interval_s` (`link.v1` §2.8), a deployment
fact, so it is outside `link.v1`'s published table, and a contract that
writes it draws W105. `[F: contracts/w105-link-downsample]`

`alarms.severity_default` left with the interim `alarms` row: an
alarm's document always carries its `severity` (`alarms.v1` §2.4), so
there is nothing for a default to fill, and `alarms.v1` publishes no key.
A contract that writes `alarms.<key>` draws W105.
`[F: contracts/w105-alarms-vocabulary]`

`desired.target` left with the interim `desired` row: the binding
already says which identity fills each target parameter (R2), and no
contract wrote it, so it is outside `desired.v1`'s published table, and a
contract that writes it draws W105. `[F: contracts/w105-desired-target]`

`redundancy.v1` publishes the interim `redundancy` row's one key
unchanged: `election`, `claim` or `external`, which a contract
writes once on `[defaults]` (`redundancy.v1` §2.2). A contract that writes
any other `redundancy.<key>`, a fencing epoch among them, draws W105: the
profile carries no epoch in a payload at 1.0 (its §2.16).
`[F: contracts/w105-redundancy-epoch]`

`timing.v1` publishes the interim `timing` row's three keys unchanged:
`period_ms`, `deadline_ms` and `lifespan_ms`, each a whole number of
milliseconds, meaningful on a stream (`timing.v1` §2.2, §2.3). A
contract that writes any other `timing.<key>` draws W105: a rate ceiling
among them, since a period is a floor on an owner's cadence, never a
ceiling (`timing.v1` §2.4), and how often an event may occur is `rate`'s
(§2.6). `[F: contracts/w105-timing-max-rate]`

`arbitration.v1` publishes the interim `arbitration` row's one key
unchanged: `policy`, which a contract writes on a requirement
(`[requires.<role>]`), never on a resource (`arbitration.v1` §2.2). Its
value `priority` is defined; `freshest` and `lease` are reserved, and a
declaration naming one is invalid until a text defines it. A contract
that writes any other `arbitration.<key>`, a deadline tightened on the
requirement among them, draws W105: no deadline is tightened at 1.0 (its
§2.15). `[F: contracts/w105-arbitration-deadline]`

**Interim annotation vocabularies** (W105 outside them, until each profile
publishes its own):

| Profile | Keys |
|---|---|
| `media` | `tiers`, `tier_param`, `frame_clock`, `control`, `receiver_report` |
| `views` | `document` (an extra's id, `sha256:…`; §9.6) |

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
  - **`errors/forms.json`** holds §7.5's error form. An error's byte → its
    code, its cause and its two bits, or the refusal (`bytes`). An error
    and its stamp → the payload's bytes, or the writer's refusal
    (`writes`). A reply error's `Encoding` and payload → the error it
    carries, or the refusal (`reads`). Its tokens are §7.5's, and
    `encoding` for a payload under another `Encoding`.
- **`budget/`**: a bound from the contract's `cardinality` and
  each instance's stated one (`bounds.json`); a population's verdict and
  reason class from a kind, a bound and one reading (`population.json`);
  a rate's from the occurrences a window heard (`rate.json`). A window
  case gives how long it listened, whether it lost no delivery, and
  whether the owner was present throughout. Whether a window lost no
  delivery comes from the resource's declared QoS, the reader's own loss,
  what it saw of its own receive path (its ticker's greatest gap, the
  bound, whether a transport of its session closed) and miss
  detection (declared, used, misses detected) (`lossless.json`).
  The bound comes from `wait_before_close`, the lease and the keep-alive
  count (`lossless.json`'s `stall_bounds`). Each file's
  description states the order a reason is chosen in.
- **`seed/`**: two Zenoh timestamps → their order (`merge.json`'s
  `order`), and one key's arrivals, each a stamp or none and a kind →
  which are kept (`cases`). A stamp is `{time, id}`, the id written as
  zenoh writes it.
  - **`inband.json`** holds in-band stamps, each `{width, run, seq}`. A
    key's kind and two stamps → their order: `less`, `equal`, `greater`
    or `unordered` (`order`).
  - Its `cases` are a key's kind and its arrivals, each a put or a delete.
    An arrival carries an in-band stamp, a Zenoh timestamp, both or
    neither. They give which arrivals a tool's merge keeps, and for a
    state key, which a consumer accepts.
- **`envelope/`**: one byte → CH's form, run width and tag
  (`vectors.json`'s `headers`). An envelope's or a form's bytes in hex,
  and the sample that carries them → the parse, or the refusal's token,
  the first that applies in §7.5's order (`cases`).
  - **The sample** is `put` or `delete` for an envelope, and `query`,
    `reply` or `reply_err` for a call's form.
  - **The parse** is the form, the run's width in bits, the run, the seq,
    the tag, and the record in hex. The record is `null` for a delete and
    an error, and a request's width, run and seq are `null`.
  - **An error's parse** adds `error`: its code, its cause, its message
    and its detail in hex, each absent one `null`.
  - Every header and every accepted case writes back to its bytes.
  - **`carries.json`:** a key, as a session hears it, and the sample's
    `Encoding` → whether the sample carries an envelope (`cases`, read by
    [`observer.md`](observer.md) §7.3). A key and a put's payload under
    `zenoh/bytes` → how it reads: plain, its form, or the refusal's token
    (`reads`).
- **`frame/`**: one packed document, shared by every case (`schema`).
  A record type and a `frame` table → the frame form, its sha256, its
  tag, the count's and the offset's widths, and the span (`layouts.json`).
  A layout and a frame's value → the body's bits before the pad, its pad
  and its bytes in hex, or the writer's refusal (`bodies.json`'s
  `writes`). A layout, a tag and a body's bytes → the value, or the
  reader's refusal, the first in §7.7's order (`reads`). A value lists
  each record's time and its fields: those it includes, as §7.6's JSON
  reading spells them, and those it includes absent. Every accepted write
  reads back to its value.
- **`packed/`**: one packed document, shared by every case (`schema`).
  Each of its types → its layout form, that form's sha256, its tag, each
  item's width and start, and the shortest and longest record this
  revision writes (`layouts`). A type, a deployment epoch where its
  fields need one, and a value → the record's bits before the pad, its pad
  and its bytes in hex, or the writer's refusal (`writes`). A type, a tag
  and a record's bytes → the JSON reading, or the reader's refusal, the
  first in §7.6's order (`reads`). Every accepted write reads back under
  the type's tag, to its value or to the `reads_as` its case gives.
- **`catalog/`**: a catalog's text → its id, or the reader's refusal,
  the first in §8.6's order (`documents.json`). A catalog's text → its
  JCS bytes, their sha256 and its id (`ids.json`). Named catalogs, each a
  JSON value, then pairs of them → whether the second succeeds the first,
  or the checker's refusal (`succession.json`'s `successions`). The
  catalogs a reader holds, the id an owner names or none, and an index or
  none → the epoch and the entry, or the reader's refusal (`reads`).
- **`compat/`:** the class, the warning rule names (sorted and
  deduplicated) and `same_revision`. The case layout and the one-resource
  wrapper are in [`compat/README.md`](conformance/compat/README.md).
