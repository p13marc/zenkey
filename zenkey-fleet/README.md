# zenkey-fleet

The fleet engine for [keyspace-v2](https://github.com/p13marc/zenkey) Zenoh
tooling — the shared core of `zenctl` and the future `zengui` explorer:

- **`query`** — the RFC 05 §2.1 fan-in discipline (`fleet_get`: target `All`,
  consolidation `None`, attribution by the reply's own key), in exactly one
  place. Answers carry zenoh's refcounted `ZBytes` — no per-reply copies.
- **`registry`** — `SliceSet`: registry slices from the live bus
  (`introspect` fan-in) or local `registry/*.toml` dirs, one type either
  way, with precedence-correct subject refinement (shared `zenkey::pattern`
  matcher) and an on-disk cache.
- **`decode`** *(feature `decode`, default)* — the RFC 08 §7 pipeline:
  `SchemaStore` lazily fetches each producer's served `describe` schema set,
  `decode_sample` turns wire bytes into named-field JSON with honest
  structural fallback; encoding resolution is sample > registry > sniff.
  `decode-protobuf` adds dynamic protobuf via the served FileDescriptorSet.
- **`sub`** — `Monitor`: subscription multiplexing + liveliness watching
  (with `history(true)` — the roster arrives on join) into a bounded
  broadcast of events. Overflow surfaces as an explicit `Dropped(n)`;
  render loops pull the immutable `KeyTreeSnapshot` on the stats tick,
  so a hot bus cannot melt a UI.
- **`stats` / `tree`** — per-key rate/byte counters (EWMA, SourceInfo gap
  counting) and the chunk-grouped snapshot they build.
- **`roster` / `admin`** — the liveliness roster and raw `@/**` admin-space
  access.

Sessions opened here are deliberately **un-namespaced** (RFC 09 §5): an
explorer sees the wire as it really is. Do not "fix" that.
