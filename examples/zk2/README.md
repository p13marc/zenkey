# zk2 example contracts

These are contracts written in the **zk2 authoring format**: the walkthrough
interfaces of `docs/zk2/architecture.md` (r3.2) and the three adopters'
mappings. They are design artifacts. `zenkey-model` (#608) will validate,
canonicalize, fingerprint and bundle them; until then they are checked by
review only.

| Directory | What it is | Issue |
|---|---|---|
| [`walkthrough/`](walkthrough/) | The r3 walkthrough interfaces: `nav.v2`, `health.v1`, `twist_cmd.v1`, `thruster.v1`, `camera.v1`, `detections.v1`, `mission_plan.v1`, `geo.v1` | #608 |
| [`tcgui/`](tcgui/) | The pilot: tcgui's v1 `tc.toml`, mapped | #589 |
| [`zenoh-modem/`](zenoh-modem/) | zenoh-modem's management contract, mapped | #623 |
| [`zensight/`](zensight/) | ZenSight's 22 registries, mapped, with a representative cut | #622 |

## The authoring format (draft 0, r3.2)

This draft fixes the shape every mapping is written in. `zenkey-model` turns
it into `spec/contract.schema.json` plus lints (#608, #607), and anything it
corrects is fixed here in the same change.

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

### `[schemas]`

```toml
[schemas]
protobuf   = ["proto/nav/v2/nav.proto"]   # proto2/proto3 only (no editions yet)
jsonschema = ["schemas/nav.json"]         # definitions under $defs
```

**Type references**, used by `type`, `attachment`, `request`, `response` and
`error`:

| Spelling | Means |
|---|---|
| `"nav.v2.Position"` | a protobuf message, fully qualified, defined in a listed `.proto` file. The well-known types (`google.protobuf.Empty`, …) are always available. |
| `"json:Status"` | the `$defs/Status` definition in a listed JSON Schema file (the zk2 subset of 2020-12) |
| `{ raw = "image/jpeg" }` | opaque bytes of a media type. Generic tools show the type and size and never decode it. |

### `[resources."<template>"]`

The table key is the **resource template**: the resource path below the kind
chunk.
- **Literal chunks** follow v1's charset: `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`.
- **`{name}`** is one chunk. Its value is slugged injectively (v1's `x-` rule).
- **`{name...}`** is a *rest* parameter: one or more chunks, each slugged.
  It may only be the **last** segment.
- **`{occurrence}`** is a lowercase ULID. It may only be the last segment,
  and only on a `stream` (§3.2 of r3.2).
- **Uniqueness:** a template is unique across the whole interface, whatever
  its kind.

**Fields for every kind:**

| Field | Values | Notes |
|---|---|---|
| `kind` | `"stream"` \| `"state"` \| `"operation"` | The key's kind chunk is `stream`/`@stream`, `state`, or `@op`. |
| `doc` | string | Documentation; not in the fingerprint. |
| `params` | `{ <name> = "string" \| "uint" \| "path" }` | Required when the template has parameters. `path` is for `{x...}` only. `{occurrence}` is implicit. |
| `cardinality` | integer | **Required** when the template has a parameter: the expected population bound. |
| `optional` | bool (default false) | An implementation may declare the resource unavailable. |
| `gate` | `"capability:<n>"` \| `"config:<n>"` \| `"feature:<n>"`, or a list of them (AND) | Only with `optional = true`. It names *why* the resource may be absent. |
| `annotations` | table | Keys are `<profile>.<key>`, and the profile must be listed in `uses`. |

**Stream and state:**

| Field | Values | Default |
|---|---|---|
| `type` | type reference | required |
| `attachment` | type reference | none |
| `explicit` | bool (stream only) | false. `true` gives the `@stream` token: never on ambient selectors. |
| `reliability` | `"best_effort"` \| `"reliable"` | stream / `@stream`: best_effort; state: reliable |
| `congestion` | `"drop"` \| `"block"` | stream / `@stream`: drop; state: block |
| `priority` | `real_time` \| `interactive_high` \| `interactive_low` \| `data_high` \| `data` \| `data_low` \| `background` | stream: data; `@stream`: data_low; state: data |
| `express` | bool | false |
| `history` | bool | false. Opt in to zenoh-ext advanced pub/sub (history and recovery). Plain kinds only, never `explicit`. |
| `retention` | duration (`"7d"`) | Occurrence streams only: how far back a replay GET may reach. |

**Operations** (the key is `…/@op/<template>`):

| Field | Values | Default |
|---|---|---|
| `request` / `response` | type reference | required |
| `error` | type reference | none. The `app` error detail, carried in the core envelope. |
| `idempotent` | bool | false |
| `fanout` | `"forbidden"` \| `"allowed"` | forbidden |
| `serving` | `"exclusive"` \| `"replicated"` | exclusive. `replicated` requires `idempotent = true`. |
| `replies` | `"one"` \| `"many"` | one |
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
annotations = { "arbitration.policy" = "priority" }
```

### What stays out of the contract

- **Deployment facts:** the system and service names, bindings, link faces.
- **Instance facts:** which optional resources are available here, and the
  capabilities this instance holds. These are the descriptor's.
