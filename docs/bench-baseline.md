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

## Fleet engine baseline (issue #44)

Criterion benches: `zenkey-fleet/benches/fleet.rs`. Run with
`cargo bench -p zenkey-fleet`. `docs/zero-copy.md` names which id pins which
rule.

- Commit: `183d496` (branch `chunk-n-perf-program`)
- Date: 2026-08-10
- Machine: Linux 7.1.3-200.fc44.x86_64 (dev workstation, default governor)

| Bench | Time (point) | |
|---|---|---|
| stats/record_hit | 41.6 ns | the per-sample floor |
| stats/record_new_key | 267 ns | the insert branch |
| stats/record_past_the_bound | 1.68 µs | steady state at the bound, evict amortised |
| stats/totals_10k | 58.5 µs ~ | the O(keys) fold the pump no longer takes a lock for |
| stats/retire_unwatched_1k | 899 µs ~ | |
| decode/structural_json | 499 ns | per sample on every render path |
| decode/structural_value_json | 749 ns ~ | |
| decode/structural_cbor | 1.40 µs | |
| decode/structural_text | 176 ns | |
| decode/structural_opaque | 234 ns | |
| tree/build_1k | 535 µs | |
| tree/build_10k | 7.35 ms | per tick |
| tree/build_50k | 58.8 ms | per tick at `DEFAULT_MAX_KEYS` |
| skeleton/build_fixture | 23.2 ms ~ | |
| skeleton/merge_10k | 74.8 ms ~ | per tick, render path |
| facts/project_v1 | 1.50 µs ~ | |
| facts/project_unparsed | 1.37 µs ~ | |
| facts/project_not_under_base | 38.6 ns ~ | |
| facts/resolve_registered | 59.8 µs ~ | |
| facts/resolve_unregistered | 45.1 µs ~ | |
| facts/describe_key | 28.1 µs | |
| registry/refine_hit | 27.0 µs | per sample on zenctl's decode path |
| registry/refine_miss | 38.8 µs ~ | |
| monitor/ingest | 383 ns ~ | the whole per-sample cost |
| monitor/tick_10k | 13.9 ms ~ | |
| fanin/origin_attribution_256 | 64.1 µs | 256 reply keys |

**Rows marked `~` are not trustworthy as a comparison point.** They varied by
more than 2× across repeated runs of *identical code* on this workstation
(`skeleton/merge_10k` read 19.5 ms and 74.8 ms; `stats/totals_10k` 16 µs and
58 µs). Re-take them on a quiet machine before using them as a regression
gate. The unmarked rows agreed within noise across every run.

At `monitor/ingest`, 100k msg/s costs a few percent of one core — which is the
claim `zenkey-fleet/src/tree.rs` makes and `tests/ledger.rs` now soaks.

### What the #44 fixes moved

Fresh back-to-back, the methodology this file already insists on: the
pre-fix commit benched from a worktree immediately before the post-fix run,
same filter, same flags (`--sample-size 10`, so the intervals are wide).
Controls first — these are code the fixes did not touch, and they agree:

| Bench | before | after | |
|---|---|---|---|
| stats/record_hit | 41.2 ns | 41.0 ns | control |
| decode/structural_text | 158 ns | 145 ns | control |
| stats/totals_10k | 16.4 µs | 17.7 µs | control |
| tree/build_10k | 14.1 ms | 7.49 ms | **−47%** (no `String` per chunk per hit) |
| tree/build_50k | 137 ms | 57.4 ms | **−58%** (same) |
| skeleton/merge_10k | 66.8 ms | 19.5 ms | **−71%** (borrowed keyexpr, path buffer) |
| registry/refine_hit | 115 µs | 38.9 µs | **−66%** (patterns parsed once, not per call) |
| facts/resolve_registered | 137 µs | 38.9 µs | **−72%** (dominated by refine) |

These are **lower bounds**: the after-run went second, and the second run of a
back-to-back pair is penalised on this machine (see below). `tree/build_50k`
at 58 ms against a 250 ms tick is the one that matters — it was 137 ms, more
than half the budget, and the tick holds the ingest mutex for all of it.

`stats/retire_unwatched_1k` is deliberately absent: the pair above was taken
with the flawed version of that bench (it built its fixture inside `b.iter`,
so it mostly measured `StatsTable::record`), and the corrected bench could not
separate the change from noise on this machine.

**Methodology, extended.** The note above records that a same-day re-run
drifted 50% on unchanged paths. Sharper finding from this round: in a
back-to-back pair, **the second run is penalised**, and by enough to invert a
verdict. Running the pair one way said the change was 2.2× slower; running it
the other way said 1.5× faster — same two binaries, same filter, minutes
apart. So: record which order was used, put the *new* code first when you want
a conservative number, and check the controls before believing any delta. A
delta whose controls disagree is a measurement, not a result.


## zengui palette key list (#110), 2026-08

Before: `view()` cloned every cached key into a fresh `Vec<String>` *every
frame regardless of overlay state* (the argument was evaluated before
`overlay()` could decline it), and twice more per palette keypress — at the
#107 bound of 50k keys, megabytes of string churn per redraw. After: a closed
overlay allocates nothing; an open jump-to overlay collects one `Vec<&str>`
(fat pointers only) and clones exactly the ≤ 20 drawn rows; activation clones
exactly one key. A note rather than a bench, deliberately: zengui has no
criterion harness, and the change is structural (O(cache) → O(drawn)) — a
number here would measure the allocator, not the design.
