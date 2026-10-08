# `compat/`: the compatibility cases

Each case is an old/new pair, or a three-revision history, with its
FULL_TRANSITIVE class in [`expect.json`](expect.json) (`core.md` §9.8).

| Family | Inputs | How a runner loads them |
|---|---|---|
| `payload/protobuf/<case>/` | `old/m.proto`, `new/m.proto` | Each wrapped in a one-resource contract (below), with `type` from `expect.json` |
| `payload/jsonschema/<case>/` | `old.json`, `new.json` | The same, as a `jsonschema` artifact |
| `contract/<case>/` | `old.toml`, `new.toml` | As contracts |
| `transitive/<case>/` | `v1`, `v2`, `v3`, each a protobuf directory or a `.json` | Wrapped like the payload cases. `v3` is checked against the history `[v1, v2]`, and `against` gives its class against each |

The wrapping contract, with `<kind>` `protobuf` or `jsonschema` and
`<artifact>` the case's file. It sits in the directory that holds its
artifact, which fixes the import root and so the protobuf file's name
(`core.md` §9.4):
- a protobuf revision is wrapped in its own directory (`old/`, `new/`,
  `v1/`…), listing `m.proto`, so the artifact is named `m.proto` on both
  sides and `same_revision` can hold;
- a JSON Schema revision is wrapped in the case directory, listing
  `old.json`, `new.json` or `v1.json`…

A runner may write the wrapper to a file there, or load its text as if it
were there.

```toml
[interface]
name = "m"
major = 1
minor = 0
[schemas]
<kind> = ["<artifact>"]
[resources.s]
kind = "state"
type = "<type from expect.json>"
```

Each entry in `expect.json` holds:
- `class`: `compatible`, `review` or `breaking`, or `invalid` when the new
  revision must not load. An `invalid` case's new revision fails with E037
  alone; its old revision loads, as every revision of every other case
  does;
- `warnings`: the warning rule names, sorted and deduplicated;
- `same_revision`, where present: §9.7's retention identity.

## Where the protobuf verdicts depart from `buf`

zk2's protobuf rules are WIRE with renumber detection (§9.8). `buf breaking`
with the `WIRE` category is the closest tool, and it departs from zk2 in two
ways. Measured with buf 1.73.0 (`breaking.use: [WIRE]`) on every
`payload/protobuf/` case, run in both orders, on 2026-10-08:

| Case | buf: new against old / old against new | zk2 | Why |
|---|---|---|---|
| `add-field` | ok / breaking | compatible | buf in the swapped order reads an addition as a deletion. FULL is not buf run twice (spike S7). |
| `add-enum-value` | ok / breaking | compatible | The same |
| `delete-field-reserved` | ok / breaking | compatible | The same: the swap reads the reservation as removed |
| `delete-field` | breaking / ok | compatible, warning `field_deleted_unreserved` | Reuse is caught against the whole history, so the deletion itself breaks nothing |
| `delete-enum-value` | breaking / ok | review | An old reader of a new writer never sees the value. A new reader of an old writer reads it as unknown: a human decides |
| `int32-to-int64` | ok / ok | breaking | The declared type is the contract. WIRE accepts it because the varint decodes, and a narrower reader truncates |
| `string-to-bytes` | ok / breaking | breaking | buf flags only bytes → string. zk2 judges both directions alike |
| `rename-field`, `rename-enum-value`, `json-name-option` | ok / ok | review | Tools decode with the writer's bundle, so a name relabels a display. A human accepts it |
| `presence-toggled` | ok / ok | review | A reader stops telling a default from an absent value |
| `proto2-enum-value-added` | ok / breaking | review | Closed enums: an old reader keeps the value as an unknown field and reads the default |

Every other case agrees: `doc-only` (ok), and `move-into-oneof`,
`scalar-to-repeated`, `renumber-field`, `reserved-reused`,
`proto2-required-added`, `proto2-required-removed`,
`proto2-optional-to-required` (breaking).

The protobuf cases added by amendment 0.5 (`move-between-oneofs`,
`proto2-default-changed`, `packed-option`, `message-renamed`,
`enum-value-renumbered`, `proto2-to-proto3`, `nested-type-added`) were not
measured with buf.

## Rule names and buf's

zk2 names its own rules (§9.8), because where a buf rule exists its meaning
often differs. The buf ids below are the ones buf 1.73.0 reported on these
cases (`--error-format json`).

| zk2 rule | buf (WIRE) | Same meaning? |
|---|---|---|
| `cardinality_changed` | `FIELD_WIRE_COMPATIBLE_CARDINALITY` | Yes |
| `oneof_changed` | `FIELD_SAME_ONEOF` | Yes |
| `required_field_added`, `required_field_removed`, `required_label_changed` | `MESSAGE_SAME_REQUIRED_FIELDS`, `FIELD_WIRE_COMPATIBLE_CARDINALITY` | Yes |
| `type_changed` | `FIELD_WIRE_COMPATIBLE_TYPE` | No: zk2 breaks on any change of declared scalar type |
| `renumbered` | `FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED`, as a side effect | No: buf sees a deletion, zk2 names the move |
| `field_deleted_unreserved` (warning) | `FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED` | No: a warning, and reuse is caught against the history |
| `enum_value_removed` | `ENUM_VALUE_NO_DELETE_UNLESS_NUMBER_RESERVED` | No: review |
| `reserved_reused` | `RESERVED_MESSAGE_NO_DELETE` | Partly: buf flags any reservation dropped, zk2 a reserved number reused |
| `field_renamed`, `json_name_changed`, `enum_value_renamed`, `presence_changed`, `closed_enum_value_added` | None in WIRE (`FIELD_SAME_JSON_NAME` is WIRE_JSON) | zk2 adds them, as review |
