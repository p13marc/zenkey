# CLAUDE.md — the `zenkey` runtime crate

## Crate layers (each module maps to an RFC section)

- `grammar` — chunk charset, reserved tokens, structural assembly/parse
  (RFC 03), plus the wire-observer helpers (`parse_full`, `fleet_rpc_key`,
  `service_rpc_key`, `all_liveliness_wildcard`, `service_alive_key`).
- `origin` — `h-<12hex>` minting from machine-id + app salt (RFC 06 §1);
  `profile` supplies the salt and fallback persistence.
- `slug` — canonical, injective slugging of foreign values, `_xNN_` escape (RFC 03 §2).
- `qos` — the five named QoS profiles as a closed enum (RFC 04 §3); the zenoh
  mappings sit behind the default `zenoh` feature (zenkey-build depends on
  zenkey with `default-features = false`).
- `context` — `V1Context` bundles origin + producer; producers build all keys
  through it.
- `slice` — `RegistrySlice`, the `introspect` reply type + diff (RFC 08 §6),
  including per-producer `[[blob]]` tier declarations (v1.8) and `[[media]]`
  stream declarations (v1.16). Optional metadata fields
  (qos/ttl/unit/rate/cardinality) must stay **optional** — forward-compat is
  pinned by zenctl's foreign-slice tests (blob and media both).
- `tests/guard.rs` — RFC 03 §4 design properties D1–D6 pinned as executable
  tests. If a grammar change breaks these, the change is wrong (or needs an RFC
  amendment).
