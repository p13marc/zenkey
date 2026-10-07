# S7 — bundle stability and classifier feasibility (#603)

Written by `spike s7`; the latest run (`unix_s` 1791415931). protox 0.9.1, protoc (system), buf 1.73.0.

## Determinism

| Contract | File | protox twice | protoc bytes | buf bytes | Normalized |
|---|---|---|---|---|---|
| camera.v1 | camera/v1/camera.proto | true | equal | differ (784 vs 265 B) | protoc identical, buf identical |
| detections.v1 | detections/v1/detections.proto | true | equal | differ (977 vs 330 B) | protoc identical, buf identical |
| geo.v1 | geo/v1/geo.proto | true | equal | differ (763 vs 275 B) | protoc identical, buf identical |
| health.v1 | health/v1/health.proto | true | equal | differ (1295 vs 454 B) | protoc identical, buf identical |
| mission_plan.v1 | mission_plan/v1/mission_plan.proto | true | equal | differ (1202 vs 471 B) | protoc identical, buf identical |
| nav.v2 | nav/v2/nav.proto | true | equal | differ (2062 vs 757 B) | protoc identical, buf identical |
| thruster.v1 | thruster/v1/thruster.proto | true | equal | differ (733 vs 262 B) | protoc identical, buf identical |
| twist_cmd.v1 | twist_cmd/v1/twist_cmd.proto | true | equal | differ (482 vs 207 B) | protoc identical, buf identical |

## Protobuf classifiers (WIRE_JSON)

`buf` columns: `buf breaking new --against old` (forward) and the swapped order (reverse). `ours`: buf's rules re-implemented on prost-reflect, same two orders. `zk2`: the directional reader/writer semantics proposed for #618.

| Case | Expected | buf fwd | buf rev | ours fwd | ours rev | agree | zk2 |
|---|---|---|---|---|---|---|---|
| add-enum-value | compatible | ok | breaking | ok | ENUM_VALUE_NO_DELETE_UNLESS_NAME_RESERVED + ENUM_VALUE_NO_DELETE_UNLESS_NUMBER_RESERVED | true | WIRE_JSON: breaking (new reads old: ok; old reads new: enum value unknown to the reader (JSON names it)). WIRE only: compatible |
| add-field | compatible both ways | ok | breaking | ok | FIELD_NO_DELETE_UNLESS_NAME_RESERVED + FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED | true | WIRE_JSON: compatible both ways (new reads old: ok; old reads new: ok). WIRE only: compatible |
| delete-enum-value | breaking | breaking | ok | ENUM_VALUE_NO_DELETE_UNLESS_NAME_RESERVED + ENUM_VALUE_NO_DELETE_UNLESS_NUMBER_RESERVED | ok | true | WIRE_JSON: breaking (new reads old: enum value unknown to the reader (JSON names it); old reads new: ok). WIRE only: compatible |
| delete-field | breaking | breaking | ok | FIELD_NO_DELETE_UNLESS_NAME_RESERVED + FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED | ok | true | WIRE_JSON: compatible both ways (new reads old: ok; old reads new: ok); W: field deleted without reserving its number. WIRE only: compatible |
| delete-field-reserved | compatible (WIRE_JSON) | ok | breaking | ok | RESERVED_NAME_NO_DELETE + RESERVED_RANGE_NO_DELETE | true | WIRE_JSON: compatible both ways (new reads old: ok; old reads new: ok). WIRE only: compatible |
| doc-only | compatible (no change) | ok | ok | ok | ok | true | WIRE_JSON: compatible both ways (new reads old: ok; old reads new: ok). WIRE only: compatible |
| int32-to-int64 | breaking under WIRE_JSON (int64 is a JSON string) | breaking | breaking | FIELD_WIRE_JSON_COMPATIBLE_TYPE | FIELD_WIRE_JSON_COMPATIBLE_TYPE | true | WIRE_JSON: breaking (new reads old: type differs; old reads new: type differs). WIRE only: breaking |
| json-name-option | breaking under WIRE_JSON | breaking | breaking | FIELD_SAME_JSON_NAME | FIELD_SAME_JSON_NAME | true | WIRE_JSON: breaking (new reads old: json name differs; old reads new: json name differs). WIRE only: compatible |
| move-into-oneof | breaking | breaking | breaking | FIELD_SAME_ONEOF | FIELD_SAME_ONEOF | true | WIRE_JSON: breaking (new reads old: oneof membership differs; old reads new: oneof membership differs). WIRE only: breaking |
| rename-enum-value | breaking under WIRE_JSON (JSON uses names) | breaking | breaking | ENUM_VALUE_SAME_NAME | ENUM_VALUE_SAME_NAME | true | WIRE_JSON: breaking (new reads old: enum value renamed (JSON names it); old reads new: enum value renamed (JSON names it)). WIRE only: compatible |
| rename-field | breaking under WIRE_JSON (json name), compatible on wire | breaking | breaking | FIELD_SAME_JSON_NAME + FIELD_SAME_NAME | FIELD_SAME_JSON_NAME + FIELD_SAME_NAME | true | WIRE_JSON: breaking (new reads old: json name differs; old reads new: json name differs). WIRE only: compatible |
| renumber-field | breaking | breaking | breaking | FIELD_NO_DELETE_UNLESS_NAME_RESERVED + FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED | FIELD_NO_DELETE_UNLESS_NAME_RESERVED + FIELD_NO_DELETE_UNLESS_NUMBER_RESERVED | true | WIRE_JSON: breaking (new reads old: field renumbered (data silently dropped); old reads new: field renumbered (data silently dropped)); W: field deleted without reserving its number. WIRE only: breaking |
| scalar-to-repeated | breaking | breaking | breaking | FIELD_WIRE_JSON_COMPATIBLE_CARDINALITY | FIELD_WIRE_JSON_COMPATIBLE_CARDINALITY | true | WIRE_JSON: breaking (new reads old: cardinality differs; old reads new: cardinality differs). WIRE only: breaking |
| string-to-bytes | breaking under WIRE_JSON (base64 in JSON) | breaking | breaking | FIELD_WIRE_JSON_COMPATIBLE_TYPE | FIELD_WIRE_JSON_COMPATIBLE_TYPE | true | WIRE_JSON: breaking (new reads old: type differs; old reads new: type differs). WIRE only: breaking |

## JSON Schema: the zk2 subset and its classifier

Subset keywords: `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `const`, numeric/length/item bounds, local `$ref`, `oneOf`, and annotations (`description`, `title`, `default`, `examples`, `format`, `deprecated`, `readOnly`, `writeOnly`, `$comment`). Rule: a reader accepts every instance its writer produces, with readers tolerating unknown properties and writers sending only what their schema declares (as protobuf); full compatibility needs both directions. `jsoncompat` judges literal containment: exit 1 means incompatible in that role.

| Case | Expected | new reads old | old reads new | zk2 verdict | jsoncompat (old → new) |
|---|---|---|---|---|---|
| add-enum-value | breaking (old readers reject it) | ok | Status.state: the writer may produce a value the reader's enum/const lacks | breaking | serializer: exit Some(1), deserializer: exit Some(0) |
| add-optional-property | compatible both ways | ok | ok | compatible both ways | serializer: exit Some(0), deserializer: exit Some(0) |
| add-optional-property-closed | compatible under the zk2 rule (readers tolerate unknown properties); literal containment says breaking | ok | ok | compatible both ways | serializer: exit Some(1), deserializer: exit Some(0) |
| add-pattern | outside the zk2 subset | ok | ok | refused: outside the zk2 subset (pattern) | serializer: exit Some(0), deserializer: exit Some(1) |
| add-required-property | breaking (old data lacks it) | Status.extra: the reader requires it and the writer may omit it | ok | breaking | serializer: exit Some(0), deserializer: exit Some(1) |
| description-only | compatible (no change) | ok | ok | compatible both ways | serializer: exit Some(0), deserializer: exit Some(0) |
| integer-to-number | breaking (old readers reject fractions) | ok | Status.since_ms: the writer may produce `number`, which the reader rejects | breaking | serializer: exit Some(1), deserializer: exit Some(0) |
| loosen-maxlength | breaking (old readers reject longer labels) | ok | Status.label: the reader's `maxLength` is tighter than the writer's | breaking | serializer: exit Some(1), deserializer: exit Some(0) |
| number-to-integer | breaking (new readers reject old fractions) | Status.load: the writer may produce `number`, which the reader rejects | ok | breaking | serializer: exit Some(0), deserializer: exit Some(1) |
| oneof-add-branch | breaking (old readers reject the new branch) | ok | Status.detail: the writer's oneOf branch 1 fits no reader branch | breaking | serializer: exit Some(1), deserializer: exit Some(0) |
| optional-to-required | breaking | Status.note: the reader requires it and the writer may omit it | ok | breaking | serializer: exit Some(0), deserializer: exit Some(1) |
| remove-enum-value | breaking | Status.state: the writer may produce a value the reader's enum/const lacks | ok | breaking | serializer: exit Some(0), deserializer: exit Some(1) |
| remove-optional-property | compatible under the zk2 rule (writers send only declared properties); literal containment says breaking | ok | ok | compatible both ways | serializer: exit Some(1), deserializer: exit Some(0) |
| required-to-optional | breaking (old readers require it) | ok | Status.state: the reader requires it and the writer may omit it | breaking | serializer: exit Some(1), deserializer: exit Some(0) |
| tighten-maximum | breaking | Status.load: the reader's `maximum` is tighter than the writer's | ok | breaking | serializer: exit Some(0), deserializer: exit Some(1) |

## Bundles for the cross-language check

24 example bundles written to `bundles/`; `s7/verify_bundles.py` checks them with Python `rfc8785` (see `python.txt`).
