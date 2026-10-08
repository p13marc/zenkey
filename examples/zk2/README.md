# zk2 example contracts

These are contracts written in the **zk2 authoring format**: the walkthrough
interfaces of `docs/zk2/architecture.md` (r3.3) and the three adopters'
mappings. They are design artifacts, all in draft 1. `zenkey-model` (#608)
validates, canonicalizes, fingerprints and bundles every one of them, and
requires each to load without a single finding
(`zenkey-model/tests/examples.rs`). To check one by hand:

```bash
cargo run -p zenkey-model --bin zk2 -- contract lint examples/zk2/walkthrough/nav.v2.toml
```

**Their history.** [`.history/`](.history/) is the history root (`spec/core.md`
§9.7) of every example in this tree, whatever its directory: a contract's
history is `.history/<iface>/`, found by its interface id. The same test
requires every example's current revision to be published there, and to be
`compatible` with every earlier revision of its interface. After an
intended change, publish the new revision:

```bash
cargo run -p zenkey-model --bin zk2 -- contract bundle <file> --history examples/zk2/.history
```

| Directory | What it is | Issue |
|---|---|---|
| [`walkthrough/`](walkthrough/) | The r3 walkthrough interfaces: `nav.v2`, `health.v1`, `twist_cmd.v1`, `thruster.v1`, `camera.v1`, `detections.v1`, `mission_plan.v1`, `geo.v1` | #608 |
| [`tcgui/`](tcgui/) | The pilot: tcgui's v1 `tc.toml`, mapped | #589 |
| [`zenoh-modem/`](zenoh-modem/) | zenoh-modem's management contract, mapped | #623 |
| [`zensight/`](zensight/) | ZenSight's 22 registries, mapped, with a representative cut | #622 |
| [`shapes.md`](shapes.md) | Deployment shapes: natural system/service names (S8 on paper) | #604 |

## The authoring format (draft 1, r3.3)

**Draft 1** applies r3.3's decisions (D1–D26, `docs/zk2/architecture.md`
§0.3) to draft 0, which the mappings were first written in. `zenkey-model`
implements it (#608): [`spec/contract.schema.json`](../../spec/contract.schema.json)
is generated from its authoring types, and its lints carry stable codes
(`zenkey_model::diag::CODES`, one fixture per code in
[`spec/conformance/contracts/`](../../spec/conformance/contracts/)). **Changes
from draft 0 are marked ✱.** What implementing it settled is listed
[at the end](#what-implementing-draft-1-settled-608).

### One file per interface major

Each interface lives in `<name>.v<major>.toml`, next to the schema sources
it references:

```text
<dir>/
  <name>.v<major>.toml       the contract
  proto/<name>/v<major>/     protobuf sources (package <name>.v<major>, recommended)
  schemas/                   JSON Schema sources (2020-12, the zk2 subset)
```

### `[interface]`

```toml
[interface]
name    = "nav"               # [a-z][a-z0-9_]* segments, dot-separated ("acme.nav"); never ends in .v<int>
major   = 2                   # the key chunk is "<name>.v<major>": nav.v2
minor   = 1                   # an informative label; CI keeps it monotonic; not in the fingerprint
summary = "Navigation solution of one vehicle"   # documentation: not in the fingerprint
uses    = ["freshness.v1"]    # the profiles whose annotations this contract uses (in the fingerprint)
```

### ✱ `[defaults]` and `[defaults.<kind>]` (D2)

```toml
[defaults]                    # every resource
annotations = { "freshness.ttl_s" = 60 }

[defaults.stream]             # every kind = "stream" resource
priority = "data_low"
encoding = "cbor"
```

A resource's own field overrides a default. Defaults are expanded *before*
canonicalization, so the fingerprint is the same whether a value is
defaulted or written out.

### `[schemas]`

```toml
[schemas]
protobuf   = ["proto/nav/v2/nav.proto"]   # proto2/proto3 only (no editions yet)
jsonschema = ["schemas/nav.json", "../profiles/telemetry/v1/telemetry.json"]
```

**Type references**, used by `type`, `attachment`, `request`, `response`,
`error` and `summary`:

| Spelling | Means |
|---|---|
| `"nav.v2.Position"` | a protobuf message, fully qualified, defined in a listed `.proto` file. The well-known types (`google.protobuf.Empty`, …) are always available. |
| `"json:Status"` | the `$defs/Status` definition. ✱ The name must be unique across the listed files (lint). |
| ✱ `"json:telemetry#Point"` | the qualified form: `$defs/Point` in the listed file whose stem is `telemetry`. This is how a contract refers to a profile type or another interface's type (D7). |
| `{ raw = "image/jpeg" }` | opaque bytes of a media type. Generic tools show the type and size and never decode it. |
| ✱ `{ raw = "video/*", media_param = "codec" }` | a raw family. The sample's `Encoding` carries the concrete subtype, and `media_param` ties it to a template parameter (D6). |

✱ **A cross-file `$ref`** may point only at listed files, which are bundled.
**A type's identity** is (kind, name, artifact sha256).

### `[resources."<template>"]`

The table key is the **resource template**: the resource path below the kind
chunk.
- **Literal chunks** follow v1's charset: `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`.
- **`{name}`** is one chunk. Its value is slugged injectively (v1's `x-` rule).
- **`{name...}`** is a *rest* parameter: one or more chunks, each slugged.
  It may only be the **last** segment.
- ✱ **Uniqueness and precedence (D1).** Two templates under the same kind
  token may not have the same *shape* (literal/parameter positions).
  Overlapping templates are allowed. A key resolves most-literal-first,
  chunk by chunk: literal > `{p}` > `{p...}`. A lint warns when overlapping
  templates carry different types. The same shape under different kind
  tokens is allowed, because their keys differ.
- ✱ **Occurrences are their own kind (D3).** Draft 0's `{occurrence}`
  parameter is gone; use `kind = "event"`.

**Fields for every kind:**

| Field | Values | Notes |
|---|---|---|
| `kind` | `"stream"` \| `"state"` \| ✱ `"event"` \| `"operation"` | The key's kind chunk is `stream`/`@stream`, `state`/✱`@state`, ✱`events`, or `@op`. |
| `doc` | string | Documentation; not in the fingerprint. |
| `params` | `{ <name> = "string" \| "uint" \| "path" }` | Required when the template has parameters. `path` is for `{x...}` only. |
| `cardinality` | integer | **Required** when the template has a parameter: the contract's ceiling on the population. ✱ An instance's descriptor may declare a lower one (D8). |
| ✱ `epoch` | parameter name | Per-member continuity epochs: the implementation holds a member token per value of that parameter, cycled on discontinuity (D9). |
| `optional` | bool (default false) | An implementation may declare the resource unavailable. |
| ✱ `gate` | `"build:<n>"` \| `"config:<n>"` \| `"capability:<n>"`, or a list of them (AND) | Only with `optional = true`. Names are `[a-z0-9][a-z0-9_.-]*`. Draft 0's `feature:` is now `build:` (D4). |
| ✱ `deprecated` | `{ since = <minor>, replaced_by = "<template>", reason = "…" }` | Fingerprinted; still served until the next major (D17). |
| `annotations` | table | Keys are `<profile>.<key>`; the profile must be listed in `uses`. |

**Stream, state and event:**

| Field | Values | Default |
|---|---|---|
| `type` | type reference | required |
| `attachment` | type reference | none |
| ✱ `encoding` | `"json"` \| `"cbor"` (jsonschema types only; protobuf is always binary) | `"json"` (D5) |
| ✱ `attachment_encoding` | as `encoding` | the same rule as the payload, from the attachment type's own kind |
| `explicit` | bool, stream ✱and state | false. `true` gives the `@stream` / ✱`@state` token: never on ambient selectors. |
| `reliability` | `"best_effort"` \| `"reliable"` | stream / `@stream`: best_effort; state, event: reliable |
| `congestion` | `"drop"` \| `"block"` | stream / `@stream`: drop; state, event: block |
| `priority` | `real_time` \| `interactive_high` \| `interactive_low` \| `data_high` \| `data` \| `data_low` \| `background` | stream: data; `@stream`: data_low; state, event: data |
| `express` | bool | false |
| ✱ `history` | `false` \| `true` \| `{ depth = <n>, miss_detection_ms = <ms> }` | false. zenoh-ext advanced pub/sub, on plain stream and state only; `true` means depth 1 (D20). |
| ✱ `rate` | `"rare"` (≤ 1/h) \| `"low"` (≤ 1/min) \| `"burst(<n>/h)"` | event only, required |
| `retention` | duration (`"7d"`) | event only, required: how far back a replay GET may reach. The cardinality is rate × retention. |

**Operations** (the key is `…/@op/<template>`):

| Field | Values | Default |
|---|---|---|
| `request` / `response` | type reference | required. ✱ A request type SHOULD NOT repeat a template parameter (lint, D21). |
| `error` | type reference | none. The `app` error detail, carried in the core envelope. |
| `idempotent` | bool | false |
| `fanout` | `"forbidden"` \| `"allowed"` | forbidden |
| `serving` | `"exclusive"` \| `"replicated"` | exclusive. `replicated` requires `idempotent = true`. |
| `replies` | `"one"` \| `"many"` | one |
| ✱ `summary` | type reference | none. With `replies = "many"`, each replier ends with one summary reply: partial, scanned, cursor (D10). |
| `timeout_ms` | integer | none (the caller's default) |
| `priority` | as above | interactive_high (a recommendation; replies inherit the caller's QoS) |

### `[requires.<role>]`

A requirement: data that reaches this interface's implementers from bound
providers (P3, §3.4).

```toml
[requires.cmd]
interface   = "twist_cmd.v1"
resources   = ["cmd"]          # what is consumed (default: everything)
cardinality = "many"           # "one" | "many"
optional    = false
doc         = "Velocity commands, arbitrated by priority."   # ✱ documentation (G23); not in the fingerprint
annotations = { "arbitration.policy" = "priority" }
```

### ✱ Interim profile vocabularies (D19)

Until each profile's spec ships its own table (#613), these are the
annotation keys contracts may use. Each key is `<profile>.<key>`, and its
profile must be listed in `uses`.

| Profile | Keys |
|---|---|
| `freshness.v1` | `ttl_s` (seconds; `0` = never stale) |
| `timing.v1` | `period_ms`, `deadline_ms`, `lifespan_ms` |
| `telemetry.v1` | `unit` (UCUM), `kind` (`counter` \| `gauge` \| `histogram` \| `text`), `buckets` (list), `semantic` |
| `link.v1` | `exposure` (`host` \| `link`), `downsample_ms` |
| `arbitration.v1` | `policy` (`priority` \| `freshest` \| `lease`) |
| `desired.v1` | `target_param` (template parameter naming the target), `target` (`self.system` \| `self.service`) |
| `alarms.v1` | `severity_default`; the key recipe is fixed by the profile, not annotated |
| `media.v1` | `tiers` (list), `tier_param` (template parameter naming the tier), `frame_clock` (`capture` \| `encode`, or the FrameMeta clock field), `control` and `receiver_report` (bool: mark the stream-control and receiver-feedback operations) |
| `views.v1` | `document` (`sha256:…` of a presentation artifact carried in the bundle's `extras`) |
| `redundancy.v1` | `election` (`claim` \| `external`) |

### What stays out of the contract

- **Deployment facts:** the system and service names, bindings, link face
  policy (which may widen `link.v1` exposure per principal).
- **Instance facts:** the capabilities held, the unavailable exceptions, a
  lower cardinality. These are the descriptor's.

### What implementing draft 1 settled (#608)

Writing `zenkey-model` turned these open points into rules. The examples
follow them, and the fixtures pin them.

- **Protobuf sources.** `[schemas] proto_include` lists the import roots,
  relative to the contract (default: `proto/` when it exists, else the
  contract's directory). A message resolves to the *listed* file that
  defines it; a type defined only by an imported file must be listed too.
  `google.protobuf.*` is always available.
- **Encodings exist only for JSON Schema types.** On a protobuf or raw
  type, `encoding` is ignored with a warning (W103), and so is
  `attachment_encoding`. **An operation takes `encoding` too**, and it
  applies to every JSON Schema type the operation names (request,
  response, error, summary).
- **Cross-file `$ref` in a bundle** resolves by the referenced file's stem,
  so stems are unique per contract (E024).
- **`cardinality` is for templates with parameters** (E014 otherwise). On
  an event it bounds the template's own parameters, and the key population
  is cardinality × rate × retention.
- **A rest parameter's type is `path`, and `path` is only a rest
  parameter's** (E012).
- **Defaults apply where their field is legal:** `[defaults] history = true`
  skips `@stream`, events and operations. A field in `[defaults.<kind>]`
  that the kind does not take is an error (E014, E019).
- **Small rules:**
  - `retention` is `<n>s|m|h|d|w`;
  - `history.depth` is at least 1 (E034);
  - `summary` needs `replies = "many"` (E033);
  - `deprecated.since` cannot exceed `minor` (E031);
  - a requirement's `cardinality` defaults to `"one"`, and `resources = []` is refused (E030).
- **Annotation keys are checked** against the interim vocabularies above
  (W105).
