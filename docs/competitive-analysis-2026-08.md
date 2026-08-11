# Competitive analysis: zenctl + zengui vs. the Zenoh explorer field

*2026-08-11. Sources: full source review of shallow clones of all five competitors
(agents read every repo end-to-end), GitHub API metadata, and a fresh feature
inventory of our own `zenctl` 0.3.1 / `zengui` 0.1.0 / `zenkey-fleet` 0.8.0.
Competitor clones live in the session scratchpad; nothing here is from README
claims alone — every feature statement was verified in code.*

---

## 1. Verdict

**We are already ahead of all five tools on the "bus explorer" core, by a wide
margin, on any bus — not just a keyspace-v2 one.** No competitor has *any* of:
schema-driven payload decode, reply attribution discipline, honesty accounting
(`Dropped(n)`, non-verdicts, eviction counters), admin-space state-coverage,
a doctor, a blob plane, shared CLI/GUI contexts, dynamic shell completion,
diff-aware watch, or a real test suite (they range from 0 to 30 meaningful
tests; we have ~337). Three of the five have no CI that runs tests at all.

Where the field beats us is **not** in exploring — it is in the *swiss-army*
verbs (generic GET/DELETE/queryable/attachments), **topology visualization**
(two competitors draw the router graph; we render a flat table), **media
payload preview**, **TLS/auth reachability** (we cannot open a secured bus at
all), and **packaging breadth**. Section 6 turns those into a prioritized
backlog. None of the gaps is structural; several are afternoon-sized because
the engine already carries the data (e.g. end-to-end latency: `SampleView`
already holds both the publisher HLC and our arrival clock — nobody else has
both).

---

## 2. The field at a glance

| | **zenctl + zengui (ours)** | zenoh-explorer (dad-io) | zenoh-hammer (sanri) | nuze (ZettaScaleLabs) | zenoh-cli (RISE-Maritime) | zsak (kydos) |
|---|---|---|---|---|---|---|
| Form | CLI + GUI over shared engine | egui desktop GUI | egui desktop GUI | Nushell REPL | Python CLI | CLI (+ new iced GUI) |
| Language / UI | Rust, Iced 0.14 | Rust, egui 0.29 | Rust, egui 0.34/wgpu | Rust, Nushell 0.114 | Python, argparse | Rust, iced 0.13 |
| Zenoh | 1.9 | 1.7.2 | 1.9.0 | 1.9.0 (uses `internal`) | eclipse-zenoh ≥1.2.1 (py) | git `main` (lock: 1.4.0) |
| Stars / created | — | 17 / 2025-11 | 24 / 2022-11 | 22 / 2025-07 | 18 / 2023-09 | 20 / 2025-04 |
| Last activity | active | 2026-06 | 2026-05 | 2026-07 | **2026-01** | 2026-07 (one burst) |
| Product LOC | ~large, 3 crates | ~5.3k | ~5.9k | ~4.9k Rust | **715** (one file) | ~2.9k |
| Tests | **~337** (unit + integration + headless GUI + soak) | 30 unit (pure logic) | 3 (2 assertion-free) | 0 Rust; 8 Nu scripts **not run in CI** | ~5 files incl. real e2e | **0** |
| CI | fmt+clippy(-D)+test+MSRV+docs | fmt+clippy+test+5-target release | `cargo check`/`build` only | lint-only, **never builds or tests** | black + pytest | **none** |
| Distribution | Forgejo binaries (Linux x86_64) | GH binaries, 5 targets | **source-only** (0 releases, 13 tags) | crates.io source build | **PyPI** | git clone only; binary named `zenoh` |
| License | Apache-2.0 | Apache-2.0 | MIT | EPL-2.0 OR Apache-2.0 | Apache-2.0 | Apache-2.0 |
| One-line read | RFC-disciplined observability pair | pivoting into a file-transfer tool ("send_it") | polished GUI z_sub/z_put/z_get | scriptable shell, wire decoder; not an explorer | unix-pipe basics + topology PNG | creator's demo-grade swiss knife |

Community traction is uniformly small (17–24 stars); nobody has won this
space. The two first-party entries (nuze — ZettaScale; zsak — Angelo Corsaro)
are one-person side projects, not blessed eclipse-zenoh deliverables.

---

## 3. Feature matrix

Legend: ✅ solid · 🟡 partial/crude · ❌ absent · 🚫 deliberate refusal (documented).

| Capability | ours | zenoh-explorer | zenoh-hammer | nuze | zenoh-cli | zsak |
|---|---|---|---|---|---|---|
| Live subscribe view | ✅ lazy, bounded, honest | ✅ `**` firehose + dedup | ✅ | ✅ streams into Nu | ✅ | 🟡 **silently drops non-string samples** |
| Generic GET (any selector) | 🟡 `admin get` works but is named/JSON-biased | ✅ | ✅ full options | ✅ full options | ✅ | ✅ |
| PUT / publish | ✅ declared pub, QoS enum, validate-against-schema, matching badge | ✅ + file import | ✅ 53 encodings, QoS | ✅ full QoS | ✅ + stdin streaming | ✅ `{N}` macro, count/period |
| DELETE | ❌ observe-only | ❌ | ❌ | ✅ | ✅ | ✅ |
| Queryable (act as responder) | ❌ | ✅ kvstore-backed | ❌ | ✅ **Nu-closure logic** | ❌ | ✅ **Python-scripted** |
| Attachments | ❌ (read or write) | 🟡 filename channel | 🟡 send-on-GET, UTF-8 only | ✅ put/get flags | ❌ | ✅ pub/query/reply |
| Admin space (`@/**`) | ✅ browse + routers + storages + **state coverage** | ❌ | ❌ | ❌ (zero hits in repo) | 🟡 `@/*/router` internally | 🟡 one linkstate key |
| Topology graph | ❌ flat tables only | ❌ (two counters) | ❌ | ❌ (decoder exists, unused) | ✅ **matplotlib graph** | ✅ **DOT dump** (+circle GUI canvas) |
| Liveliness | ✅ roster is load-bearing (Alive/Suspect), watch events | ❌ | ❌ | ✅ (experimental) | ✅ get/sub/token, NDJSON | ✅ declare/sub/query |
| Storage awareness | ✅ list + Covered/Partial/Uncovered join | ❌ | ❌ | ❌ | ❌ | 🟡 spawns a `zenohd` child |
| Scout (Hello listing) | ❌ (`base list` is liveliness/storage-based) | ❌ | ❌ | ✅ | 🟡 flags accepted but ignored | ✅ + `list -r/-p/-c` |
| Schema-driven decode | ✅ served `describe`: JSON Schema / protobuf / CDR | ❌ | ❌ | ❌ | ❌ | ❌ |
| Structural decode (JSON/CBOR) | ✅ + sniff, tagged how it decoded | 🟡 JSON only | ✅ JSON/JSON5 trees | ❌ UTF-8 or bytes | 🟡 json codec | ❌ UTF-8 only, **panics on binary** |
| Hex view | ✅ (GUI cap 1024 B noted) | 🟡 256 B dump | ✅ paged viewer | 🟡 Nu binary | ❌ | ❌ |
| Image/media preview | ❌ | ❌ | ✅ PNG/JPEG/GIF/BMP/WebP | ❌ | ❌ | ❌ (dead video code) |
| Per-sample QoS metadata shown | 🟡 kind/encoding/both timestamps; **not** priority/CC/express | ❌ all discarded, mislabeled `text/plain` | ✅ **everything incl. SHM flag** | ✅ full record | ❌ all discarded | ❌ |
| Rate / bandwidth | ✅ `hz`, `bw`, per-key, EWMA | ❌ | 🟡 crude Hz (can print `inf`) | ❌ | ❌ | ❌ |
| Loss detection | ✅ SourceInfo seq gaps | ❌ | ❌ | ❌ | ❌ | ❌ |
| Latency / bench | 🟡 `bench rpc` per-reply; no pub→sub e2e | ❌ | ❌ | ❌ | ❌ | ❌ |
| History / diff | ✅ per-key ring, HLC-vs-arrival labeled, structural diff | 🟡 last-50 list | 🟡 per-key deque | ❌ | ❌ | ❌ |
| Recording / replay | ❌ (clipboard/stdout ndjson only) | ❌ | ❌ | 🟡 Nu `\| save` | 🟡 shell redirect | ❌ |
| Machine-readable output | ✅ json/ndjson, one report model CLI+GUI | ❌ | n/a (GUI) | ✅ Nu structured data | ✅ line templates, NDJSON liveliness | ❌ none at all |
| Scripting / e2e testing | 🟡 ndjson + shell | ❌ | ❌ | ✅ **best in field** (multi-session, closures, jobs) | ✅ pipes | 🟡 pyo3 queryable only |
| Wire-protocol decode | ❌ | ❌ | ❌ | ✅ transport/scouting msgs, LinkState | ❌ | ❌ |
| Keyexpr algebra tooling | ❌ (internal only) | ❌ (hand-rolled, wrong) | ❌ | ✅ `includes`/`intersects` | ❌ | ❌ |
| Multi-session | ❌ (one) | 🟡 dual (internal trick) | ❌ one at a time | ✅ named sessions + shared runtimes | ❌ | ❌ |
| Zenoh config file / TLS / auth | ❌ **cannot reach a secured bus** | ❌ (dead `config_json`) | ✅ user JSON5 (display-only) | ✅ record or file | ✅ `--config` + `--cfg path:value` | ✅ `-c` JSON5 |
| Saved profiles / contexts | ✅ shared CLI+GUI, editable, validated | ❌ nothing persists | ✅ workspace archive | ❌ | ❌ | ❌ |
| Shell completion | ✅ dynamic, cache-fed, 5 shells | n/a | n/a | ✅ Nu-native | ❌ | ❌ |
| Registry / contract awareness | ✅ bus+dirs union, lint, diff, export, drift | ❌ | ❌ | ❌ | ❌ | ❌ |
| Fleet health tooling | ✅ doctor (11 checks, deltas), `node info` | ❌ | ❌ | ❌ | ❌ | 🟡 `doctor` checks 2 things |
| Blob / artifact plane | ✅ list/probe/fetch, verify-before-disk | 🟡 own chunking protocol | ❌ | ❌ | ❌ | ❌ |
| i18n | ❌ | ❌ | ❌ (abandoned stub) | ❌ | ❌ | ❌ |

---

## 4. Per-competitor profiles

### 4.1 zenoh-explorer (dad-io) — GUI, the closest *shape* to zengui

MQTT-Explorer-style topic tree + always-on `**` monitor session. Its real
engineering went into **large-file chunked transfer** (64 MB chunks, validated
reassembly, progress bars) — and the repo's own plan says it is being renamed
into a file-transfer tool ("send_it"). As an explorer it is damaged at the
foundation: **every received sample's encoding, kind, and HLC timestamp are
discarded** and replaced with `text/plain` / `Utc::now()` (`zenoh_worker.rs:246,407,724`),
so a DELETE is indistinguishable from an empty PUT. No admin space, no
liveliness, discovery is two integers ("3R 5P"). Hand-rolled (wrong) keyexpr
matching in its queryable; monitor-session failure still reports "Connected";
`max_message_size` forced to 100 GB. Best-in-field distribution though:
5-target binary releases with checksums.

**Worth stealing:** per-topic pause with honest counters (we have this),
content dedup + rate limiting for firehose viewing, chunk-metadata sanity
checking. **Threat level: low** — it is leaving the category.

### 4.2 zenoh-hammer (sanri) — GUI, the sample-inspection benchmark

The accurate self-description is "GUI z_sub/z_put/z_get". Narrow but polished
and *current* (Zenoh 1.9, egui 0.34, edition 2024, ~1 tag per Zenoh minor
since 2022). Two things it does better than anyone including us:
**per-sample fidelity** — key, kind, both timestamp halves, encoding,
attachment, source zid/eid/sn, congestion control, priority, reliability,
express, and whether the buffer arrived via **SHM** — and **payload
composition**: a 53-encoding editor with string/hex/image-file modes, plus
decode-side JSON/JSON5 trees and image rendering. Also has a workspace
archive (save/load all four pages).

But: no admin space, no discovery/scout/liveliness/storages, no DELETE, no
queryable, one session, connection = hand-written JSON5 file it can only
display, key tree rebuilt from scratch every frame, a failed subscriber
declaration **kills the whole session** (`task_zenoh.rs:150-154`), `.unwrap()`
on the GET future panics the Zenoh thread. 3 tests (2 without assertions),
compile-only CI, zero releases ever published. **Threat level: low-medium** —
it owns "inspect one sample deeply"; we should take that crown (§6).

### 4.3 nuze (ZettaScaleLabs) — the scriptable shell, not an explorer

A custom Nushell binary with Zenoh commands compiled in. Its center of gravity
is (a) **multi-session e2e testing** — named sessions, queryables whose reply
logic is a Nu closure, matching listeners, shared runtimes via `zenoh::internal`
— and (b) a **594-line wire-protocol decoder** (transport + scouting messages,
including a hand-rolled LinkState decoder) that only a first party would
attempt. Full QoS surface on put/get/delete; keyexpr `includes`/`intersects`;
live config dump.

As an explorer it is structurally hollow: **zero admin-space integration**
(literally no `@/` in the repo), no topology assembly despite the LinkState
decoder, payloads are UTF-8-or-bytes with no decoding, no recording, no
measurement, and the CI **lints but never builds or tests** — its own test
runner swallows failures. Requires Nushell fluency plus a ~834-crate source
build. Panics on publish errors mid-pipeline. **Threat level: medium long-term**
— ZettaScale brand + genuinely the best scripting story; if they ever point it
at the admin space it gets interesting. It is complementary to us more than
competitive.

### 4.4 zenoh-cli (RISE-Maritime) — unix pipes + the topology PNG

715 lines of Python. Verbs: info/scout/network/put/subscribe/get/delete/
liveliness(get/sub/token). Two genuinely good ideas: **stdin/stdout line
templating** (`put --line "{key}: {value}"` parses piped lines; subscribe
formats out; liveliness emits NDJSON) and the **`network` command**: scout +
`@/*/router` admin queries → networkx graph → matplotlib rendering with
router/peer/client coloring — the only shipped topology picture in the field.
Plugin codecs via Python entry points.

Everything else is thin: scout's flags are parsed then ignored, QoS is
commented out, **all sample metadata is discarded** (a DELETE looks like an
empty PUT), no queryable, no query tuning, default decoder is base64
(surprising), matplotlib+networkx are hard deps of a CLI. Last commit
2026-01; effectively dormant. **Threat level: low.** Our `--format ndjson`
already beats its pipe story except for stdin-publish; its topology graph is
the one feature to take.

### 4.5 zsak (kydos) — broadest API touch, zero engineering floor

Angelo Corsaro's personal utility. Touches nearly every primitive most tools
miss: liveliness (declare/sub/query), **`graph`** (queries
`@/<zid>/router/linkstate/routers`, emits DOT for `dot -Tpng`), a
`zenohd`-spawning `storage` command, a **pyo3-scripted queryable**, publish
with count/period/`{N}` macros. The `-a/--admin` flag and linkstate query show
where first parties look: **the admin space is the topology source of truth.**

But it is demo-grade: **0 tests, no CI, not publishable to crates.io**
(manifest lacks license/description), the binary squats the name `zenoh`,
subscribe **silently drops every non-string sample** (`action.rs:144`) —
the worst possible failure for an explorer — `try_to_string().unwrap()`
panics on binary, `--priority` is parsed and never used, `--help` prints
literal `"ADD HERE FEW EXAMPLES"`, every session force-requires the
storage-manager plugin, and the month-old GUI has a guaranteed
`block_on`-in-update panic. **Threat level: low** — but watch it as a signal
of what Zenoh's creator thinks the tool should do: graph, storage, scripting.

---

## 5. Where we win (and should keep widening the lead)

1. **Truthfulness as a feature.** Three of five competitors *silently corrupt
   or drop data* (explorer relabels everything `text/plain`; zsak drops
   non-string samples; zenoh-cli discards kind/timestamp/encoding). Our whole
   stack is built on the opposite: `Dropped(n)`, eviction counters, seed
   coverage reports, non-verdicts, "declared vs. seen", clocks never mixed.
   This is the moat — no competitor can retrofit it without a rewrite.
2. **Attribution discipline.** Nobody else does target-All/consolidation-None
   fan-in with reply-key attribution. Every competitor's GET can silently
   collapse a fleet to one reply (zenoh-explorer at least hardcodes All/None
   for queries, but attributes nothing).
3. **Contract awareness.** Registry union (served > dirs, disagreements
   reported), schema-driven decode with drift detection, lint/diff/export,
   deprecation ledgers. The entire field renders payloads by sniffing at best.
4. **Fleet operations.** Doctor with stable check ids and run-over-run deltas,
   state-coverage joins, node info, blob plane with verify-before-disk. No
   competitor has any operational layer at all.
5. **Two frontends, one engine, one report model.** GUI panes render the same
   serde types the CLI emits; echo-pane export is CLI-shaped ndjson. Every
   competitor GUI is a dead end for automation.
6. **Engineering floor.** ~337 tests incl. headless GUI simulation and an
   ignored soak; 5-job CI with MSRV and doc gates; benches as compile gates;
   867-line design ledger. Field median: ~3 tests, compile-only CI.
7. **Scale honesty.** Virtualized 50k-row tree with bounded memory and counted
   retirement; ArcSwap snapshots so a hot bus can't melt the render loop.
   zenoh-explorer deep-clones its tree every frame; hammer rebuilds its tree
   every frame.

---

## 6. Where they beat us today — the backlog

### P1 — table-stakes verbs (cheap; every CLI competitor has them)

| Gap | Who has it | Note |
|---|---|---|
| **Generic `zenctl get <selector>`** | all five | `admin get` already *is* it (plain fan-in GET on any selector) but is named as admin browse and JSON-biased. Surface it as a first-class verb sharing the echo rendering ladder. Highest value-per-line in this document. |
| **DELETE** | nuze, zenoh-cli, zsak | We can observe tombstones but not produce one. Needs an RFC-conscious shape (`topic retire`? gated `--i-know`?) since bare deletes interact with RFC 04 §1.2 LWW/tombstone semantics — but "the explorer cannot retire a test key" is a real hole, and spray proves we already publish deletes from the demo. |
| **Scout command** | nuze, zsak, (zenoh-cli broken) | Raw Hello listing (`zid / whatami / locators`). Complements `base list` (liveliness-based) for the "is anything even out there / is multicast working" question. Both first-party tools ship it. |
| **Attachment display** | hammer, nuze, zsak | We have zero hits for `attachment` in three crates. At minimum render them in echo/detail (read side); write side can wait. |

### P2 — differentiators worth taking from the field

| Gap | Who proves demand | Note |
|---|---|---|
| **Topology graph** | zenoh-cli (matplotlib), zsak (DOT), nuze (decoder) | Both first-party-adjacent CLIs read router linkstate from the admin space — exactly the plane we already query for `admin routers`/storages. A zengui pane (iced canvas) + `zenctl admin graph --dot` for piping would leapfrog both: they draw unlabeled circles; we can overlay origins, bases, and storage coverage on the mesh. |
| **Pub→sub end-to-end latency** | nobody | `SampleView` already carries publisher HLC *and* arrival monotonic — we are the only tool holding both halves. An `hz --latency`-style readout (with the HLC-trust caveat stated) would be a first-in-field measurement. |
| **Per-sample QoS fidelity** | hammer | Hammer shows priority/CC/express/reliability/SHM per sample; our `SampleView` stops at kind/encoding/timestamps. Extend it and the detail pane — this takes hammer's one crown. |
| **Recording to file / replay** | (weakly: nuze `\| save`) | Echo already exports CLI-shaped ndjson to clipboard; `zenctl topic echo --output FILE` plus a `replay` that re-publishes with original pacing would beat everything shipped. Replay must go through declared publishers (our write path already enforces this). |
| **Mock queryable** | zenoh-explorer, nuze, zsak | "Stand up a responder to test a consumer" is a real dev-loop need (three competitors ship it). A `zenctl serve <key> <reply>` with static/`@file` bodies would cover 90% without embedding a scripting language. |
| **Image/media preview** | hammer | Our `**`-never-crosses-`@` scope excludes `@media` by design, but the detail pane could still decode image-encoded payloads when explicitly selected. Low priority; note the deliberate-refusal boundary in the pane if we skip it. |

### P3 — reachability and reach

| Gap | Note |
|---|---|
| **Zenoh JSON5 config passthrough** | We cannot connect to a TLS/QUIC-with-certs or usrpwd-secured bus *at all* — three knobs is the whole surface. A `--zenoh-config FILE` (merged under our three inserts, precedence documented) unlocks every transport without us growing a TLS UI. hammer/nuze/zenoh-cli/zsak all have this. |
| **Packaging breadth** | zenoh-explorer ships 5-target binaries; we ship Linux x86_64. aarch64 + macOS lanes in `release.yml` are the cheap 80%. zengui additionally needs a wgpu-capable host — note it in the README. |
| **Keyexpr algebra verbs** | nuze's `keyexpr includes/intersects` is ~50 lines over `zenoh-keyexpr` and genuinely useful when debugging D2/D4 scope surprises — we of all tools should have it. |
| **stdin publish streaming** | zenoh-cli's `put --line` pattern; our `topic pub - --repeat` reads stdin once. NDJSON-in (`--from ndjson`) would make replay and piping symmetric with our output. |

### Deliberate refusals to keep (and keep documenting)

No TUI (the GUI is the TUI), no silence-as-verdict, no bare-put helper, no
hostname identities, no `--origin` on blob probe, no client-side filtering of
what the selector can express server-side, no unbounded buffers. The
competitors' bug lists (§4) are one long argument for every one of these.

---

## 7. Bottom line

The five tools split into three categories: **generic sample pokers**
(hammer, zenoh-explorer), **scripting shells** (nuze, zenoh-cli), and a
**first-party sketchpad** (zsak). We are the only entry that is an
*observability system* — attribution, contracts, honesty, health — and the
GUI's key-agnostic core plus `--registry`-less operation means we compete on
their home turf (arbitrary buses) too, not only on keyspace-v2 fleets.

To be strictly better than the union of the field, close P1 (four small
verbs), then take the two crowns competitors actually hold — hammer's
per-sample fidelity and the zenoh-cli/zsak topology graph — and add the
config-file passthrough so a secured bus stops being unreachable. Everything
else in §6 extends leads we already have.
