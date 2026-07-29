# 08 — The Subject Registry

**Status: v1.2 (ratified)** · normative chapter · *amended in v1.2 — see [00-index.md](00-index.md)*

The grammar fixes positions 1–5 of every key; the registry governs the rest.
It is the single, machine-readable inventory of every subject, procedure,
media stream shape, and service origin a deployment's components may use —
the convention's equivalent of Keelson's subject registry and OTel's
semantic-convention YAML ([10-prior-art.md](10-prior-art.md)).

The registry is what keeps an open-ended `<subject...>` from decaying into
folklore: a subject that is not registered does not exist.

---

## 1. What the registry buys

- **One payload type per selector.** Every registry entry binds a subject
  pattern to exactly one payload type, so any wildcard result set is
  homogeneous and decodable without sniffing
  ([02-principles.md P5](02-principles.md)).
- **Generated constants, not string literals.** The registry compiles to
  code, so producers and consumers share one source of truth and a typo is
  a compile error. The codegen contract has two directions, both
  normative:
  - *build*: per subject, a constant for the pattern and a typed builder
    taking one argument per `{var}` (a chunk-list for a `{var...}`),
    producing a canonical key;
  - *parse*: per producer, a matcher taking a key's subject tail and
    returning which registered subject it is plus its extracted variables
    — this is what replaces positional `split('/')` re-parsing scattered
    across consumers (the incumbent's ~15 view files' worth). Match
    precedence is most-literal-first: literal chunks beat `{var}` beats
    `{var...}`, position by position, so a deprecated literal
    (`flow/duration_p50_ms`) still matches its own entry, not a
    later-added `flow/{x}`.
- **Reviewable evolution.** Adding a subject is a registry diff — visible,
  reviewable, and versioned — not a key that quietly appears on the bus.

### 1.1 The origin is an argument too — build/parse × local/remote

*Added in v1.2. The contract above says "one argument per `{var}`" and
never mentions the origin, which is how three separate implementations of
it shipped the same bug.*

A key needs an origin as well as a subject, and there are exactly **two
kinds of origin a component can hold**:

| | what it is | who has it | used to |
|---|---|---|---|
| **local** | the origin this process minted for itself ([06 §1](06-identity.md)) | every producer | **serve**: publish state/telemetry, declare a queryable |
| **remote** | an origin this process *read* — from a key it received, a health doc, a catalog entity | every consumer | **call**: address someone else's `@rpc`, subscribe to one host's `@media` |

They are both `h-<12hex>`. They are never interchangeable, and **a builder
that silently supplies the local one is a trap**: pass it to a consumer's
call path and you get a key addressed at *the caller's own host*, where no
queryable has ever lived. The failure is a timeout, at runtime, in one
view — the worst possible way to find out. The reference implementation
made this mistake in three separate commits, and the third one took every
drill-down in the product down at once ([06 §6.3](06-identity.md)).

So the codegen contract is **build/parse × local/remote**, and:

> Generated builders **MUST** make the origin an explicit argument, and
> **SHOULD** make its kind a **type**, not a convention — strengthened to
> **MUST** for builders of a `kind = "write"` procedure
> ([08 §2](#2-entry-format), [05 §3](05-control-rpc.md)).

With a type (`LocalOrigin` vs `RemoteOrigin`, and an explicit
`FleetSelector` for the deliberate `*`), "I built a key for my own host by
accident" stops being a runtime timeout and becomes a compile error. That
is the whole promise of generating this code instead of formatting
strings, and it is cheap: the origin is *already* a value at every call
site — it is only stringly-typed because nobody said not to.

The typing stays a SHOULD for read-only builders (a mis-typed origin on a
read is a wasted query — a timeout, recoverable), but is a **MUST** for
write builders (v1.4): a mis-typed origin on a *write* does not time out —
it **actuates the wrong host, or one's own**, before anyone notices. A
side-effecting write has no safe failure mode for a stringly-typed origin,
so the compile-time distinction that is merely prudent for reads is
mandatory for writes.

A `*` origin **MUST** be reachable only by asking for it by name. It is a
fleet selector, it pairs with query target **All**
([05-control-rpc.md §2.1](05-control-rpc.md)), it is forbidden on the bulk
planes ([07-bulk-planes.md §3](07-bulk-planes.md)), and it must never be
what a builder does when it has nothing better.

### 1.2 The generated surface (v1.5)

*Added in v1.5 (H1). §1 and §1.1 state the contract; this section pins how
the enforcement crate realizes it, so implementations stop re-deriving it.*

- **Keys are values of a validated key type**, not strings. Every builder —
  hand-written or generated — returns a type whose existence proves the key
  is canonical, concrete, and base-relative. Consumers convert to the
  middleware's key-expression type by moving the value, never by re-parsing
  a `String` (the re-parse-and-`expect` wrapper every adopter wrote is the
  bug this retires).
- **Variables are slugged at the API boundary.** Generated constructors take
  raw application values and apply the [03 §2](03-grammar.md) slug
  themselves; a subject value cannot hold an illegal chunk, so key
  construction from a well-formed subject is **infallible**. Call-site
  slugging (the adopters' `slug_key_chunk(...)` habit) is retired — a missed
  call was a latent grammar violation.
- **The typed origins of §1.1 live in the enforcement crate itself** —
  `LocalOrigin` (mintable only from the app profile), `RemoteOrigin`
  (parseable only from wire data), the service origin, and the deliberate
  fleet marker — so every application shares one implementation and the
  sealed origin-kind trait, instead of hoisting its own copy.
- **G2 is structural, not advisory**: generated modules emit fleet
  spellings (a fleet procedure id and its selector builder) **only for
  `fanout = allowed` procedures**. A forbidden-fanout write has *no fleet
  spelling anywhere in the generated surface* — the refusal MUST of
  [05 §2.1](05-control-rpc.md) becomes unrepresentable rather than checked.
- **Each subject family also generates a fieldless family id** (the
  variant set without its variables) with a per-family *selector* builder
  (`{var}` → `*`, `{var...}` → `**`, scoped to one origin or the fleet), so
  consumer-side routing tables and subscriptions stop re-encoding the
  registry by hand.

## 2. Entry format

One TOML document per producer (or service), checked into the repository
that owns the producer — the application repo, not the convention repo.
The `zenkey-build` crate compiles a `registry/` directory into typed
builders/parsers from the owning application's build script; the `zenkey`
runtime crate ships no registry. Example (fields annotated inline,
normative field table below):

```toml
# registry/netring.toml
[registry]
version = "1.2"                             # this file's MAJOR.MINOR (§3)
app     = "zensight"                        # owning application
convention = 1                              # the @v major it targets

[producer]
name = "netring"                            # base name; instances suffix -<int> in keys
description = "wire-level flow/L7/NDR sensor"

[[subject]]
path        = "flow/red/{quantile}"        # subject pattern
class       = "telemetry"                   # telemetry | state | events
type        = "TelemetryPoint"              # payload type (shared type table, §5)
qos         = "sampled"                     # QoS profile (04-planes §3); omit = class default
unit        = "ms"                          # primitive subjects: unit suffix convention, see §4
cardinality = 5                             # expected key-population bound for the {var}
since       = "1.0"                         # registry version that introduced it
description = "flow-lifetime RED quantiles from the capture path"

[[subject]]
path        = "alert/{alert_key}"
class       = "state"
type        = "Alert"
qos         = "alert"
cardinality = 64                            # max concurrently-firing keys, order of magnitude
ttl_s       = 900                           # live-state staleness TTL (04-planes §1.2)
since       = "1.0"
description = "detector alerts; firing→resolved on one key, delete = tombstone"

[[subject]]
path        = "capture/{ulid}"
class       = "events"
type        = "CaptureRecord"
rate        = "rare"                        # events only: rare | low | burst(n/h)
replay      = "window(7d)"                  # events only: deployment must keep 7 days queryable
since       = "1.0"
description = "a capture was triggered; immutable audit record"

[[procedure]]
path        = "capture/trigger"
kind        = "write"                       # read | write | long-running
request     = "CaptureTrigger"
reply       = "Ack"
idempotent  = false
fanout      = "forbidden"                    # forbidden | allowed; default forbidden for kind="write" (§2, 05 §2.1)
since       = "1.0"
description = "fire the pre-trigger ring / rotate the spool"

[[media]]
path        = "{stream}/video/{codec}/{profile}"
encoding    = "video/*"
attachment  = "FrameMeta"
since       = "1.0"

[[deprecated]]
path        = "flow/duration_p50_ms"
class       = "telemetry"
since       = "1.0"
gone        = "1.2"                          # still reserved; never reused
replaced_by = "flow/red/p50_ms"
```

Open-depth subjects use the rest-variable, in their **own producer's**
file (§5's ownership rule — a `gnmi/…` path inside `netring.toml` would be
namespace-squatting):

```toml
# registry/gnmi.toml
[producer]
name = "gnmi"

[[subject]]
path        = "{device}/{path...}"          # {path...}: rest-variable, see rules
class       = "telemetry"
type        = "TelemetryPoint"
cardinality = 10000                         # bounded by the device subscription list
since       = "1.0"
description = "gNMI subscription paths, slugged per 03-grammar §2"
```

Normative field table (`[[subject]]`; `[[procedure]]`/`[[media]]` analogous):

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | pattern string | yes | subject pattern; see variable rules below |
| `class` | enum `telemetry\|state\|events` | yes | data class ([04-planes.md §1](04-planes.md)) |
| `type` | type-table name | yes | the one payload type of every expansion |
| `qos` | enum, profiles of [04-planes.md §3](04-planes.md) | no (class default) | named QoS profile |
| `unit` | string | primitive numerics only | unit of the leaf value |
| `cardinality` | integer | yes if `path` has any `{var}` | expected key-population bound (order of magnitude); the budget review enforces |
| `ttl_s` | integer | live `state` only | staleness TTL; publishers refresh ≤ ttl/2, consumers age out at ttl |
| `rate` | `rare` \| `low` \| `burst(n/h)` | `events` only | rate class (CI-checked, [04-planes.md §1.3](04-planes.md)) |
| `seed` | `none` \| `latest` \| `tail(n)` | no (class default: `state` → `latest`, `telemetry` → `none`) | late-joiner entitlement ([04-planes.md §3.1](04-planes.md)); *how* it is met (storage vs cache) is deployment config |
| `detect_s` | integer | no (live `state` only; default = `ttl_s`) | max latency to detect a missed transition; values ≪ `ttl_s` require the advanced tier ([04-planes.md §3.3](04-planes.md)) |
| `replay` | `none` \| `window(t)` | `events` only | how far back events must stay queryable (met by the events storage) |
| `delivery` | `full` (default) \| `invalidate` | no | oversized-state pattern ([04-planes.md §1.2](04-planes.md)) |
| `encoding` | MIME-ish string (`application/cbor`, `application/json`, `application/protobuf`) | no (RECOMMENDED, v1.5) | the payload encoding a consumer should expect; resolution order is sample `Encoding` > this field > sniff ([04-planes.md §3](04-planes.md), §7) |
| `since` / `gone` / `replaced_by` | registry versions / path | `since` yes | lifecycle (§3) |
| `description` | string | yes | one line, human |

`[[media]]` entries (the `@media` plane, RFC 07 §1) are **not** "analogous" —
they carry opaque encoded frames, not a class payload, so they have their own
normative field table:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | pattern string | yes | media sub-path after `@media/<producer>/`; same variable rules as below |
| `encoding` | middleware `Encoding` (may be a `type/*` family) | yes | the wire `Encoding` set on every sample (`video/h264`, `image/jpeg`) — the codec is declared here, never in a payload envelope |
| `attachment` | type-table name | yes | the per-frame sidecar type on the Zenoh attachment (`FrameMeta`); **CI-resolved against the shared type table** ([§5](#5-ownership-and-process)), exactly like a `[[subject]]` `type`, so a typo or a drifted schema fails the build |
| `cardinality` | integer | yes if `path` has any `{var}` | key-population bound, budget-reviewed — the same rule as `[[subject]]`, and it now binds the highest-bandwidth plane, whose `{tier}` chunk multiplies its key count |
| `since` / `gone` / `replaced_by` | registry versions / path | `since` yes | lifecycle (§3) |
| `variant` | CamelCase string | no | generated-variant name override, same rule as `[[subject]]` (v1.5) |
| `description` | string | recommended | one line, human |

A `[[media]]` entry has **no** `class`/`qos`/`ttl_s`/`seed` — those are
data-class concepts; `@media` QoS is fixed by RFC 07 §1
(best-effort · drop · interactive-high) and is not a per-entry knob. Media key
builders are generated from these entries the same way subject/procedure
builders are, so a hand-written `media_*_key()` cannot drift from the registry.
(Promised in v1.3, delivered in v1.5: the generated media surface is a media
value type with slugging constructors, a publish builder taking the local
origin, and a viewer builder taking a remote origin — and deliberately **no**
wildcard/family selector, per the [07 §1](07-bulk-planes.md) tier-wildcard
revocation: a viewer subscribes to exactly one tier.)

`[[blob]]` entries (the `@blob` plane, [RFC 07 §2](07-bulk-planes.md)) are the
third shape, added in **v1.8**. `@blob` was the one plane with no entry kind:
its keys were normative but unmodellable, so the only plane carrying whole
files was also the only one no build-lint and no bus explorer could see. A
blob entry declares *which tiers and endpoints this origin serves*:

```toml
[[blob]]
tier        = "artifact"
endpoints   = ["manifest", "slice", "have", "fanout"]
reference   = "ArtifactDelivery"
encoding    = "application/vnd.tcpdump.pcap"
since       = "1.8"
description = "packet captures and debug bundles minted by @rpc/netring/artifact"

[[blob]]
tier        = "store"
algo        = "blake3"
since       = "1.8"
description = "content-addressed chunks backing the tree tier"
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `tier` | enum `artifact \| tree \| store` | yes | the reserved tier token after `@blob` ([07 §2](07-bulk-planes.md)). It is a **tier token, not a producer chunk** — content-addressed data has no owning component — so a blob entry generates no producer position, unlike every other entry kind |
| `endpoints` | list of reserved names | `artifact` only (yes) | which of [07 §2.2](07-bulk-planes.md)'s endpoints this origin serves: `manifest`, `slice`, `have`, `push`, `fanout`. `tree` and `store` have none — the key *is* the endpoint — and naming any on them is an error |
| `algo` | hash-algorithm name | `store` only (yes) | the `<algo>` chunk ([07 §2.4](07-bulk-planes.md)). A deployment SHOULD carry one value fleet-wide; a second entry exists only while a migration runs both |
| `reference` | type-table name | no (RECOMMENDED on `artifact`) | the payload type that conveys this blob's reference to consumers — i.e. the type that MUST carry the content root under [07 §2.1](07-bulk-planes.md). **CI-resolved against the shared type table** ([§5](#5-ownership-and-process)), exactly like a `[[subject]]` `type` |
| `encoding` | MIME-ish string | no | the encoding of the blob *content* (`application/vnd.tcpdump.pcap`), so a consumer can choose a viewer without fetching. Never the chunk framing — that is self-describing on the wire ([07 §2.4](07-bulk-planes.md)) |
| `since` / `gone` / `replaced_by` | registry versions / tier | `since` yes | lifecycle (§3) |
| `variant` | CamelCase string | no | generated-variant name override, same rule as `[[subject]]` |
| `description` | string | yes | one line, human |

The negative space is larger here than for any other kind, and each absence is
load-bearing:

- **No `path`.** Alone among the entry kinds, a blob entry has no pattern. The
  three key shapes are fixed by [07 §2](07-bulk-planes.md) and their variable
  chunks are *content addresses* — a ULID, a root hash, a chunk hash — not
  registry vocabulary. What actually varies between deployments is which tiers
  and endpoints an origin serves, and that is the whole of the declaration.
- **No `cardinality`.** [03 §3](03-grammar.md) already carves blob ids and tree
  roots out of the cardinality budget as sanctioned unbounded families. Asking
  for a number here would invite a fiction and then budget-review it.
- **No `class`/`qos`/`ttl_s`/`seed`** — the same argument `[[media]]` makes:
  these are data-class concepts, and `@blob` QoS is fixed by
  [07 §2.6](07-bulk-planes.md) as a *client* obligation discharged by default
  in the reference client. It is not a per-entry knob.

Declaring `push` in `endpoints` states a **capability, not a policy**:
[07 §2.2](07-bulk-planes.md) requires the receiving origin to gate `push/**`
behind an authorization hook and to leave it off by default, and a registry
entry does not and cannot satisfy that.

Blob key builders are generated from these entries as subject/media/procedure
builders are, with two requirements that are structural rather than
stylistic:

- **Content-address arguments are typed as content hashes, never strings.**
  `tree` and `store` take a validated hash type, so the caller-chosen-name
  spelling that [07 §2.3](07-bulk-planes.md) revoked (`tree/nightly`) has no
  expression in the generated surface — the same move H1 made for
  forbidden-fanout writes. A rule the codegen can refuse to spell does not
  need to be remembered.
- **The probe form is a distinct type.** [07 §2.5](07-bulk-planes.md) permits
  a `*`-origin probe (`have`/`manifest`, tiny replies) and forbids a
  `*`-origin bulk fetch. The generated probe builder therefore returns a
  *probe prefix*, not a key, so a probe prefix cannot be passed where a
  fetch prefix is expected. Probe-then-fetch becomes expressible from the
  registry rather than being prose a caller must obey.

One asymmetry is created here and recorded rather than hidden: blob entries
appear in the **runtime introspect slice** (§6), while `[[media]]` entries —
which have had a field table since v1.3 and codegen since v1.5 — still do
not. That is a pre-existing gap in the slice, not a decision about `@blob`;
retrofitting media is separate work and is deliberately not bundled here,
because the whole stated value of modelling `@blob` is that an explorer can
see which origins serve blobs and of which tier.

`[[procedure]]` entries (the `@rpc` plane, RFC 05) are the fourth shape and,
like `[[media]]`, carry request/reply *types* rather than a class payload, so
they too have their own normative field table:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `path` | pattern string | yes | procedure sub-path after `@rpc/<producer>/`; same variable rules as `[[subject]]` |
| `kind` | enum `read \| write \| long-running` | yes | procedure idiom ([05-control-rpc.md §3](05-control-rpc.md)) |
| `request` | type-table name | no (empty-body reads) | payload type of the query body; CI-resolved against the shared type table ([§5](#5-ownership-and-process)) |
| `reply` | type-table name | yes | payload type of a success reply (errors ride `reply_err`, 05 §3) |
| `idempotent` | bool | `write`/`long-running` only | whether a retried call is safe; documented per 05 §3 |
| `fanout` | enum `forbidden \| allowed` | no (default: `write` → `forbidden`, `read`/`long-running` → `allowed`) | may a `*`-origin fan-out call target this procedure? A `write` that broadcasts actuates the whole fleet, so `forbidden` is the default and the only sound value for a side-effecting write ([05-control-rpc.md §2.1](05-control-rpc.md)) |
| `cardinality` | integer | yes if `path` has any `{var}` | key-population bound, budget-reviewed — same rule as `[[subject]]` |
| `encoding` | MIME-ish string | no (RECOMMENDED, v1.5) | request/reply payload encoding — same semantics as the `[[subject]]` field (§7) |
| `since` / `gone` / `replaced_by` | registry versions / path | `since` yes | lifecycle (§3) |
| `description` | string | yes | one line, human |

The `fanout` field (added v1.4) is what lets the builder/registry/ACL refuse a
fleet-wide *write*: a fan-in `*` origin is safe and expected for a `read`
(collect every host's answer), but a broadcast `write` is a fleet-wide side
effect. `fanout = "forbidden"` on a `kind = "write"` procedure makes the
`*`-origin call a build-time or admission error, not a runtime surprise
([05-control-rpc.md §2.1](05-control-rpc.md)). Procedure key builders are
generated from these entries exactly as subject/media builders are.

Variable rules:

- `{var}` = exactly one chunk; MUST document its domain (device name, unit
  slug, ip-slug, ULID, hash…) in the description or a `domain` sub-key.
- `{var...}` = **rest-variable**: one or more chunks, allowed only in
  trailing position, at most one per pattern. This is how open-depth
  subjects (gNMI paths, directory-like metrics) register without
  enumerating every path: the pattern still binds exactly one payload type
  across all expansions, and its `cardinality` budget covers the whole
  family. Generated accessors expose the rest as a chunk list.
- A pattern with any variable still binds one payload type across all its
  expansions ([02-principles.md P5](02-principles.md)).
- Service origins (`@catalog`) register the same way with `[service]`
  replacing `[producer]` — same fields minus instances (services have no
  instance suffix), subjects keyed directly under the class chunk:

```toml
[service]
name = "catalog"
origin = "@catalog"
description = "identity/ontology service (zensight-correlator)"
```

## 3. Versioning policy

Two independent version axes, deliberately decoupled:

| Axis | Mechanism | Bumped when |
|---|---|---|
| **Convention major** | the `@v<int>` key chunk | grammar positions or their semantics change incompatibly — hermetic break by key algebra ([03-grammar.md §1.2](03-grammar.md)). The posture is D-Bus's "hopefully never": the protocol version froze at 1 |
| **Registry version** | the `[registry] version` header + `since`/`gone` fields, MAJOR.MINOR, one stream per registry file | MINOR: additive (new subjects/procedures, deprecations). MAJOR: a reviewed break that would otherwise be a forbidden rebind — reserved for the exceptional case where deprecate-and-add cannot express the change; in the normal course MAJOR never moves |

Each registry *file* versions independently (its producer's stream);
`since`/`gone` values refer to the file's own stream. A second application
adopting the convention starts its own files at 1.0 — there is no global
registry version to coordinate.

- **Deprecate, never reuse.** A retired subject keeps its registry entry
  (`gone` + `replaced_by`) forever; its path is never rebound to a
  different meaning or type. Renames are additions plus deprecations
  (OTel's model; [02-principles.md P10](02-principles.md)).
  `[[deprecated]]` entries are **append-only**: CI fails if one disappears
  from the file — that is what makes never-reuse mechanically checkable.
- **A subject's payload type may evolve compatibly** (additive fields) under
  the payload format's own rules (self-describing encodings — CBOR/JSON —
  tolerate additive change). An incompatible payload change is a **new
  sibling name with a numeric suffix** (`sockets` → `sockets2`, D-Bus's
  `Manager1 → Manager2` move), never a version *leaf*: a `sockets/v2` leaf
  would sit inside every wildcard that matches `sockets/**`, putting two
  payload types in one result set (violating P5), whereas a suffixed
  sibling is invisible to selectors written against the original. During a
  deprecation window the producer SHOULD serve/publish both generations
  (D-Bus services own both well-known names); `replaced_by` tells consumers
  where to migrate.
- **The wire carries the registry version out-of-band** (payload envelope or
  attachment), not in the key — the key algebra only needs to isolate
  *grammar* breaks; payload-schema evolution is diagnosable from the data
  (contrast rmw_zenoh's silent type-hash isolation,
  [03-grammar.md §6.4](03-grammar.md)).

## 4. Naming rules

- Chunks: lowercase snake_case within the lexical rules of
  [03-grammar.md §2](03-grammar.md); prefer chunk hierarchy over compound
  names (`cpu/usage`, not `cpu_usage`; `if/eth0/rx_bytes`, not
  `if-eth0-rx-bytes`) — hierarchy is what wildcards can select on.
- **Primitive numeric leaves carry their unit as a suffix** where the unit
  is not obvious from the name: `total_usec`, `p95_ms`, `rx_bytes`,
  `usage_percent` (Keelson's convention). Structured payloads carry units
  in metadata instead (OTel's convention); the registry `unit` field is
  authoritative either way. The key suffix exists for the human reading a
  raw bus, not for machines.
- Counters are singular with a `_total` suffix; gauges are bare; ratios
  say their scale (`_percent` vs `_ratio`).
- Subject vocabulary SHOULD reuse established semantic names where a
  mapping exists (OTel host metrics, SNMP MIB names) — the registry entry
  is the right place to record the cross-standard mapping, as the reference
  application does for its exporter semconv table.

## 5. Ownership and process

- Each producer's registry file lives with the producer's code; the
  application's registry directory holds the **type table** —
  `registry/types.toml`, mapping each `type` name to its kind and schema
  location (v1.5: this materializes what earlier text placed as "a
  document in the convention repository", which never existed as an
  artifact; the table lives where the types live) — and the convention
  repository holds the reserved-token list
  ([03-grammar.md §3](03-grammar.md)). A `type`/`request`/`reply`/
  `attachment` name not present in the type table fails CI (the
  `zenkey-build` resolution lint); that is what makes
  `type = "TelemetryPoint"` resolvable for a second application.

  ```toml
  # registry/types.toml
  [types.TelemetryPoint]
  kind = "json-schema"                       # schema kind served by describe (§7)
  rust = "zensight_common::telemetry::TelemetryPoint"   # informational location
  ```
- Prefix ownership is the collision rule at the vocabulary level: a
  producer may only register subjects under its own producer chunk
  (OTel's namespace-squatting rule, adapted).
- When two producers observe the same real-world concept, the process
  SHOULD converge them on one shared subject vocabulary (recorded in the
  shared type table) rather than letting parallel producer-prefixed
  spellings coexist — OPC UA's harmonized companion specifications are the
  precedent ([10-prior-art.md](10-prior-art.md)).
- CI SHOULD enforce: every published key is buildable from a registry
  entry; every registry path is lexically legal (including: no producer
  base name ending in `-<int>`, [03-grammar.md §1.5](03-grammar.md); no
  reserved token as a subject leaf); no `deprecated` path is re-registered
  and no `[[deprecated]]` entry is ever deleted; every `events` entry has
  a `rate`; every `{var}`-bearing entry has a `cardinality`; every live
  `state` entry has a `ttl_s`.
- CI MUST enforce, for `[[blob]]` entries (v1.8) — these are closed
  vocabularies fixed by [07 §2](07-bulk-planes.md), so every one of them is
  decidable at build time and none is a matter of taste:
  - `tier` is one of `artifact` | `tree` | `store`;
  - `endpoints` is present exactly when `tier = "artifact"`, and every name
    in it is from the reserved set of [07 §2.2](07-bulk-planes.md)
    (`manifest`, `slice`, `have`, `push`, `fanout`);
  - `algo` is present exactly when `tier = "store"`;
  - at most one entry per `(tier, algo)` — a second entry for a tier is a
    duplicate declaration, not a second key family;
  - `reference`, where present, resolves in the shared type table, exactly
    as a `[[subject]]` `type` does.
- CI MUST enforce (v1.5, H4): in a **service** registry, a subject pattern
  containing the variable `{host}` places it as the **first** chunk — the
  G1 desired-state proxy rule ([07 §3](07-bulk-planes.md)): the target host
  is addressing, and addressing lives at the front of the subject, where
  ACL prefix rules can reach it ([09 §3](09-operations.md)). Generated
  constructors type that variable as a host id, not a free string.
- CI **MUST** enforce the **reverse direction**: *every registered subject
  and procedure is actually served by the build that advertises it*
  (§6). Note this is a distinct check, not the mirror image of the first
  one, and the first one does **not** imply it — a registry may be a
  strict superset of what the code does and every published key still
  builds. That superset is what `introspect` ships to the fleet as truth.

  Note also that the forward lint is **vacuous wherever a producer
  registers a catch-all subject** (`{metric...}` and friends): everything
  is buildable from a catch-all, so "every published key is buildable"
  asserts nothing. A registry that leans on catch-alls has bought neither
  direction.

## 6. Runtime introspection

The static TOML is the *authority*; a running fleet additionally serves
the *observation* of it. Every producer MUST serve
`@rpc/<producer>/introspect` (read, idempotent) returning the registry
slice it was **compiled against** — its subjects, procedures, blob tiers
(v1.8), and registry file version. (Through v1.7 this sentence also said
"media shapes"; the slice has never carried them. v1.8 corrects the claim
rather than quietly widening it — see the asymmetry note in §2.) The reply is generated from the same
source as the producer's key constants, so it cannot drift from behavior
(the reason D-Bus introspection XML is trustworthy: the implementation
emits it — [10-prior-art.md](10-prior-art.md)).

What it buys: `GET <base>/v1/*/@rpc/*/introspect` is a fleet
capability-and-version inventory in one round trip (which hosts still
serve a deprecated subject; which run last month's registry); generic
explorer tooling — the `busctl`/`d-feet` equivalent — needs no compiled-in
registry.

### 6.1 The registry MUST NOT lie (normative)

*Strengthened in v1.2. v1.0 called a mismatch "a finding, not an
ambiguity" — it named the gap and declined to close it. Practice showed
that is not enough.*

A disagreement between introspection and the checked-in TOML is still a
finding in the direction the TOML *leads*: the TOML says what should run,
the introspection says what does, and a fleet mid-rollout will honestly
show both.

But the other direction is not a finding, it is a **defect**:

> **Every subject and procedure in a registry MUST be served by the build
> that ships it.** A registry entry describing a surface the code does not
> serve is not aspirational — it is a **lie transmitted to every consumer
> that calls `introspect`**, and it is worse than silence, because
> `introspect` is the one source a generic explorer is entitled to trust.

This is what makes the introspection reply trustworthy at all. D-Bus
introspection XML is dependable because *the implementation emits it*
([10-prior-art.md](10-prior-art.md)); a registry compiled from a TOML that
nobody checked against the code has none of that property and all of its
authority.

The obligation is structural, not procedural. The reference
implementation's registry was reviewed, versioned, and lint-clean against
the grammar — and still advertised **seven** surfaces that no build served
(two capture procedures, three stream procedures, an entity `link`/`unlink`
pair, a phantom subject), while omitting five that *were* served. Review
does not catch this. Only a check does (§5), and the strongest form of the
check is to make the registry the **only** way to declare a surface, so an
unserved entry is dead code rather than a lie.

An entry for a surface that is merely *planned* is not a registry entry.
It is a diff, and it lands the day the code does.

## 7. Payload self-description (v1.5)

*The gap this closes: the registry binds one payload **type name** per
subject (§2, P5), and `introspect` (§6) tells a generic tool which name a
key carries — but nothing anywhere said what the **bytes look like**. A
foreign explorer could render anonymous CBOR structure and nothing more;
a non-self-describing encoding (protobuf) would be fully opaque. Schema
*contents* stay application-owned; this section standardizes only their
**transport**.*

Every producer **SHOULD** serve a read procedure `@rpc/<producer>/describe`
— and **MUST**, if any type its slice references rides a
non-self-describing encoding (a tool can degrade to structural rendering
of CBOR/JSON; it cannot degrade protobuf) — replying with a **SchemaSet**
JSON document:

```json
{
  "schema_version": 1,
  "app": "zensight",
  "types": {
    "TelemetryPoint": {
      "kind": "json-schema",
      "hash": "sha256:ab12…",
      "schema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "…": "…" }
    },
    "FlowRecord": {
      "kind": "protobuf",
      "hash": "sha256:cd34…",
      "message": "zensight.flow.FlowRecord",
      "descriptor_b64": "<base64 FileDescriptorSet>"
    }
  }
}
```

Normative points:

- **Totality.** The set MUST cover every type name the producer's
  `introspect` slice references (`type`, `request`, `reply`,
  `attachment`, and — since v1.8 — a `[[blob]]` `reference`); a superset is
  fine. §6.1 binds `describe` exactly as it
  binds `introspect`: serving a schema for bytes the build does not emit
  is a lie.
- **Kinds are an open vocabulary.** This RFC registers `json-schema`
  (JSON Schema draft 2020-12; describes the *data model*, so it covers
  both JSON and CBOR framings of the same serde model) and `protobuf`
  (a base64 `FileDescriptorSet` plus the fully-qualified `message` name —
  exactly what a dynamic-message decoder needs). A consumer MUST skip an
  unknown `kind` without discarding the rest of the set, and MUST ignore
  unknown fields (the same forward-compat posture as the slice parser,
  §6).
- **Hash.** `hash` is `sha256:` over the schema's canonical bytes (JCS
  for JSON documents; the raw descriptor bytes for protobuf). It exists
  for client caching and for drift detection (two producers serving the
  same type name with different hashes is a `doctor` finding).
- **No per-sample schema ids.** Evolution stays additive-only under one
  name; an incompatible change is a **new suffixed sibling name** (§3's
  rule, unchanged). This deliberately rejects the Kafka wire-prefix model
  — it would break every existing payload and reintroduce the envelope
  [07 §1](07-bulk-planes.md) forbids. Rejected alternatives, recorded:
  schemas embedded in the introspect TOML (bloats the fleet-inventory
  fan-in); a retained "birth certificate" key (assumes storage/retained
  semantics the convention does not guarantee); a central schema-registry
  service (a new availability singleton in a deliberately peer-served
  design).
- **Encoding is a separate axis.** Producers SHOULD set the middleware
  `Encoding` on every sample (`application/cbor`, `application/json`,
  `application/protobuf` — the predefined constants); the registry MAY
  declare a per-entry `encoding` (§2). Consumers resolve
  **sample > registry > sniff**, and the first-byte sniff remains the
  honest last resort — mixed fleets keep working.

The decode contract for a generic tool is then mechanical: wire key →
structural parse → slice refine → type name (+ encoding) → SchemaSet
lookup (fetch `describe` on first miss, cache by hash) → decode into
named fields. A tool that cannot resolve a schema falls back to what it
could always do — structural rendering tagged with the declared type
name, or an honest `<undecoded TypeName: N bytes>`.
