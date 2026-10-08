# Type scenarios (core §7.2)

## §1 `Encoding` on every sample

**Steps.** An owner publishes a protobuf stream, a JSON Schema state
(`encoding = "cbor"`) and a raw `image/jpeg` stream.

**Expected.**
- Every sample carries the predefined Zenoh `Encoding`: protobuf, CBOR and
  `image/jpeg`.
- No sample carries a schema suffix.

## §2 Decoding and honest rendering

**Steps.** A tool, given only the bus and the bundles it fetches, subscribes
to:
- a protobuf stream;
- a JSON Schema state;
- a raw stream of `application/x-flatbuffers` (a flatbuffer payload,
  declared raw), for which it has no plugin.

**Expected.**
- The tool decodes and shows the protobuf and JSON Schema values field by
  field.
- For the raw stream, it shows the declared type and each sample's size,
  neither garbage nor nothing.
- **Bytes that do not decode** (0.8). A protobuf sample of three bytes
  `ff ff ff`, and a JSON Schema sample `{` on a CBOR-declared state: each
  is shown as its declared type, named as a decoded one is
  (`detections.v1.Objects`, `json:Status`), its size and the reason. The
  reason for the second says it was read as CBOR.

## §3 QoS applied

**Steps.** An owner publishes a stream declared `priority = "data_high"`,
`express = true`, and a state with the defaults.

**Expected.** Each received sample's QoS (priority, congestion control,
express) is the declared one.
