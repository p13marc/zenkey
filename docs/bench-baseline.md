# Performance baseline (issue #17)

Criterion benches: `zenkey/benches/keys.rs`, `fixture-tests/benches/generated.rs`.
Run with `cargo bench --workspace`. Times are criterion's [low, point, high]
estimates on the machine below; the point estimate is what we compare.

## Baseline — BEFORE the 0.3 core rewrite

- Commit: b249467 + benches (branch `redesign-v0.3`)
- Date: 2026-07-19
- Machine: Linux 7.1.3-200.fc44.x86_64 (dev workstation, default governor)

| Bench | Time (point) |
|---|---|
| build/data_key_3_chunks | 238 ns |
| build/rpc_key | 199 ns |
| build/producer_chunk | 97 ns |
| parse/structural_5_keys (5 keys) | 1.362 µs (~272 ns/key) |
| slug/clean_chunk | 27 ns |
| slug/dirty_chunk | 1.227 µs |
| match/keyexpr_intersects | 140 ns |
| generated/key_build | 452 ns |
| generated/subject_parse_hit | 32.6 ns |
| generated/subject_parse_miss | 8.4 ns |
| generated/parse_metric | 91.5 ns |
| generated/any_subject_dispatch | 44.4 ns |

Known allocation hotspots these numbers embody (report §14): per-chunk `String`
collect in `grammar::parse`, double-`Vec` in `V1Context::state_key`/`media_key`,
`Producer::chunk()` fresh `String` per call, generated `parse_metric`
`split('/').collect()` per call, generated `key` re-validating through grammar.

## After the 0.3 core rewrite

(appended by the post-rewrite pass — same benches, same machine)
