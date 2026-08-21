# Zero-copy discipline (issue #44)

`docs/redesign-2026-07.md` §14 promised "a short zero-copy discipline doc
(ZBytes rules, borrowed `&keyexpr` currency, `intersects` over string ops)".
This is it.

It exists because the engine has two cadences and they fail differently. A
**per-sample** path runs 100 000 times a second on a hot bus, so an allocation
there is a rate; a **per-tick** path runs four times a second but scales with
the *key population*, so an allocation there is a rate multiplied by 50 000.
Both were leaking, and neither showed up as a bug — only as a machine working
harder than the design said it should.

Numbers for every claim here live in `docs/bench-baseline.md`; the bench ids
that pin each rule are named in the sections below. Run them with
`cargo bench -p zenkey-fleet`.

---

## 1. `ZBytes` is refcounted; act like it

**`.clone()` is a refcount bump, and is the correct way to retain a payload.**
`ZBytes` is `#[repr(transparent)]` over a `ZBuf`. Cloning one into a
`SampleView`, a `FetchedValue` or a history entry keeps the buffer alive
without copying a byte.

```rust
// zenkey-fleet/src/query.rs — the canonical example, whose doc comment
// records retiring the old double copy.
Answer::Value(sample.payload().clone())
```

**`.to_bytes()` returns a `Cow<'_, [u8]>`** — borrowed for a contiguous
payload, owned only for a fragmented one. It is *not* a copy, and code that
avoids it out of caution is confusing itself. Bind it to a local and pass
`&bytes` onward:

```rust
// zenkey-fleet/src/decode.rs — hold the Cow, borrow through it.
let cow = bytes.to_bytes();
if let Ok(text) = std::str::from_utf8(&cow) { … }
```

**`.to_bytes().to_vec()` is banned on any per-sample or per-render path.**
That is the double copy §14 flagged. The last one in the tree was
`zengui/src/view/detail.rs`'s hex pane, on a render path; `hex_pane` only
reads the slice to build a string, so it now takes `&[u8]`.

**`.slices()`** iterates the non-contiguous regions borrowed, and is the
escape hatch when even the fragmented-payload copy matters — `@media` and
`@blob` traffic is where that becomes real.

**`.len()` never touches the bytes.** Size budgets (the echo ring's, the
history ring's) use it and must never materialise a payload to measure one.

*Pinned by:* `decode/structural_json`, `decode/structural_cbor`,
`decode/structural_text`, `decode/structural_opaque`.

---

## 2. Borrowed `&keyexpr` is the currency

`keyexpr::new(&str)` **validates without allocating**. `KeyExpr::new(String)`
builds an `OwnedKeyExpr` — an `Arc<str>` copy. Own one only when it must
outlive the borrow it came from (declaring a subscriber or querier, where
zenoh's builder takes ownership and the "fix" would be wrong).

The rule has teeth because the offenders were all in loops:

- `StatsTable::retire_unwatched` built an owned expr **per key in the table**,
  on every unwatch — at the 50k default bound, 100 000 allocations to answer a
  question about coverage. Now borrowed throughout.
- `skeleton::merge` cloned every watch selector into an owned expr **per
  tick**, and `is_covered` built a fresh `format!("{prefix}/**")` plus an
  owned expr **per node per tick**. Now: one borrowed expr per selector, a
  reused path buffer threaded through the descent, and a scratch buffer for
  the subtree probe.

- `NodeRoster::refresh` (`zengui/src/nodes.rs`) built a subtree selector with
  `format!` and **two** owned exprs for **every (producer × watch) pair, every
  tick**. Missed when `skeleton::merge` was fixed, and found by #178. Now: one
  borrowed expr per selector per tick, and a reused buffer for the selector.

Parse a selector **once per set**, never once per candidate.

> **This paragraph used to name `view/echo.rs` as the example that already did
> it correctly.** It was the worst offender in the crate. `EchoView::admits`
> parsed the filter *inside* the per-line loop, so a 2,000-line ring re-parsed
> the same expression 2,000 times a frame — 120,000 times a second — to answer a
> question that changes on a keystroke, and parsed each line's key beside it.
> #178 measured the fix at 344 µs → 190 µs.
>
> Left standing here because a doc comment is not a claim anybody checks. It is
> corrected rather than deleted: a discipline doc that once certified its own
> counter-example should say so, since the next false certification will read
> exactly like this one did.

*Pinned by:* `stats/retire_unwatched_1k`, `skeleton/merge_10k`,
`echo/admits_2k_keyexpr`, `roster/refresh_40p_8w`.

---

## 3. Coverage is key algebra, not string comparison

Never `starts_with`, `contains`, or a hand-rolled `split('/')` walk to decide
whether a selector covers a key. Use `intersects` (do they overlap) and
`includes` (does one contain the other).

This is not style. `**` does not cross an `@` chunk (RFC 03 §4 D2, pinned in
`zenkey/tests/guard.rs`) and `*` is chunk-scoped, so string prefixing gives
**wrong answers on exactly the keys this convention cares about**: a naive
prefix test says `v1/h-a/**` covers `v1/h-a/@rpc/sysinfo/introspect`, and it
does not.

```rust
// zenkey-fleet/src/admin.rs — state_coverage, borrowed and algebraic:
// `includes` ⇒ covered, `intersects` ⇒ partial.
if ke.includes(family) { … }
if ke.intersects(family) && coverage == Coverage::Uncovered { … }
```

Compose keys with `zenkey::grammar::with_base` and the typed builders, never
`format!` — an empty base under `format!` grows a leading slash and silently
drops every family, which `admin.rs` documents at the point of use.

The counterweight: `intersects` costs 140–200 ns
(`match/keyexpr_intersects`). Correct, but not free — so hoist the parse out
of the loop and do not run it per node per tick unguarded.

---

## 4. The per-sample allocation floor, stated

Ingesting one sample allocates exactly three things:

1. `SampleView::key` — one `String`. The view outlives the zenoh `Sample`, so
   the key must be owned.
2. `SampleView::encoding` — one `String`.
3. `Arc<SampleView>` — one, for the broadcast fan-out.

`monitor/ingest` measures all of it. **Anything above that floor on the
per-sample path is a bug.** If a change moves that number, it moved the floor.

`SampleView::attachment` (#117) does not move it: an attachment, when
present, is a fourth *refcount bump* (`Option<ZBytes>`, cloned like the
payload), not a fourth allocation — and absent it is a `None`. The
three-allocation floor stands.

The #120 QoS axes (`priority`/`congestion_control`/`reliability`/`express`)
and `source: Option<SampleSource>` do not move it either: every one is
`Copy`. The floor is still three.

The per-tick floor is `KeyTreeSnapshot::build`: one `TreeNode` per *new* node,
and — since the `contains_key`/`get_mut` fix — no `String` at all for a chunk
that already exists. `tree/build_50k` is the number that says so.

---

## 5. Deliberately left alone

Recorded so they are not "fixed" by the next reader:

| Site | Why it stays |
|---|---|
| `zenctl/src/cmd/echo.rs` — `sample.payload.to_bytes()` | **Not a copy.** Returns `Cow::Borrowed` for a contiguous payload. Issue #44 named this as a surviving violation at `zenctl/src/main.rs:917`; that line is clap dispatch, the loop moved in #48, and the construct was never the problem. The two `String` clones beside it were, and they are gone. |
| `zenkey-fleet/src/query.rs` — `declare_querier(key.to_string())` | zenoh's builder takes ownership. A borrowed expr does not compile here. |
| `zenkey-fleet/src/query.rs` — the error-reply arm | `String::from_utf8_lossy(…).to_string()` into an owned `Error{name,message}`. Rare, and the bytes must be owned. |
| `zenkey-fleet/src/body.rs` — `body.to_vec()` | The write path builds an owned wire body, once per user action. Not per sample. |
| `SampleView`'s two `String`s | The floor above. `Box<str>`/`Arc<str>` would save a pointer's worth and break an all-public-fields struct. |
| `Flattened::rows` — one `Vec<TreeRow>` per flatten | Bounded by `MAX_ROWS`, and the rows are the pane's own data rather than a copy of the engine's. What it costs is #177's, not this doc's. |

---

## 6. How to check

```bash
cargo bench -p zenkey-fleet                     # everything
cargo bench -p zenkey-fleet -- stats/ monitor/  # the per-sample floor
cargo bench -p zenkey-fleet -- tree/ skeleton/  # the per-tick paths
cargo bench -p zengui                           # the frontend's own two cadences
cargo bench -p zengui -- echo/ hex/             # per frame
cargo bench -p zengui -- roster/ series/        # per tick
```

`tree/build_50k` and `skeleton/merge_10k` are tens of milliseconds an
iteration — release only, and slow even there.

`zengui/benches/frame.rs` is **renderer-free by design**: it measures the work a
`view::*` function does before it builds a widget, plus the per-tick joins that
feed it. A bench needing a GPU surface would not run in CI, and one through
`iced_test` would measure the simulator — so the widget-shaped half is held by
`zengui/tests/panes.rs` and by the compiler instead. Saying which was measured
beats a harness that implies both.

The O6 counter ledger (`zenkey-fleet/tests/ledger.rs`) is the other half of
this discipline: zero-copy is about not *spending*, the ledger is about not
*losing*. Both run from `just ci`.
