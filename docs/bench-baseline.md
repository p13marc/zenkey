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

## After the 0.3 core rewrite (2026-07-19)

Methodology note: the first same-day re-run showed +50% on *unchanged* code
paths (slug, intersects) — machine-state skew from hours of compilation. So
the comparison below is **fresh back-to-back**: the pre-rewrite commit
(774ff39) benched from a worktree immediately before the post-rewrite run;
controls (producer_chunk, slug, intersects) agree within noise.

| Bench | before (fresh) | after | delta |
|---|---|---|---|
| build/data_key_3_chunks | 393 ns | 581 ns | +48% ¹ |
| build/rpc_key | 368 ns | 510 ns | +39% ¹ |
| build/producer_chunk | 143 ns | 143 ns | control |
| parse/structural_5_keys | 1.978 µs | 1.687 µs | **−15%** (borrowed subject) |
| slug/clean_chunk | 39 ns | 40 ns | control |
| slug/dirty_chunk | 1.66 µs | 1.80 µs | ~noise |
| match/keyexpr_intersects | 196 ns | 207 ns | control |
| generated/key_build | 638 ns | 189 ns | **−70%** (single-pass, no re-validation) |
| generated/subject_parse_hit | 47 ns | 44 ns | −6% |
| generated/subject_parse_miss | 12.9 ns | 7.8 ns | **−39%** |
| generated/parse_metric | 137 ns | 75 ns | **−45%** (stack buffer) |
| generated/any_subject_dispatch | 65 ns | 61 ns | −6% |

¹ The raw `grammar::*` builders now pay `OwnedKeyExpr` validation inside
`Key::from_canonical` — the price of the validated type at the unvalidated-
input boundary. Deliberate trade: the production paths are the *generated*
builders (3.4× faster — they skip grammar re-validation entirely because
their inputs are typed `Chunk`s) and `V1Context`. An `unsafe` unchecked
constructor could reclaim it and was declined (no-unsafe policy); revisit
only if a profile shows a real workload bound on raw grammar builds.
