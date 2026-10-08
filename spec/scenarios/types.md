# Type scenarios (core §7.2)

## §1 `Encoding` on every sample

**Steps.** An owner publishes a protobuf stream, a JSON Schema state
(`encoding = "cbor"`) and a raw `image/jpeg` stream.

**Expected.**
- Every sample carries the predefined Zenoh `Encoding`: protobuf, CBOR and
  `image/jpeg`.
- No sample carries a schema suffix.

## §2 Honest rendering

**Steps.** A tool subscribes to a `flatbuffer` stream it has no plugin for,
and to a raw stream.

**Expected.** The tool shows each sample's declared type and size. It shows
neither garbage nor nothing.
