# Changelog

Releases of the workspace as a whole. The convention's own amendment ledger
is [`rfcs/CHANGELOG.md`](rfcs/CHANGELOG.md); `zenctl`'s command-tree cut has
its own migration table in [`zenctl/CHANGELOG.md`](zenctl/CHANGELOG.md).

Versions per crate, because they move independently:

| Crate | 0.6.0 | 0.7.0 | 0.7.1 | 0.7.2 | 0.8.0 |
|---|---|---|---|---|---|
| `zenkey` | 0.6.0 | 0.7.0 | 0.7.0 — unchanged | 0.7.0 — unchanged | **0.8.0** |
| `zenkey-build` | 0.6.0 | 0.7.0 | 0.7.0 — unchanged | 0.7.0 — unchanged | **0.8.0** |
| `zenkey-fleet` | 0.9.0 | 0.10.0 | **0.11.0** | **0.11.1** | **0.12.0** |
| `zenctl` | 0.4.0 | 0.5.0 | **0.5.1** | 0.5.1 — unchanged | **0.6.0** |
| `zengui` | 0.2.0 | 0.3.0 | **0.3.1** | 0.3.1 — unchanged | **0.4.0** |
| `zenwatch` | — | — | — | — | **0.1.0** (new) |

---

## Unreleased

**The exporter — a metrics surface that exports its own blind spots**
(#228, RFC 13 §3 *Exporter obligations*, v1.34). `zenkey-fleet` gains the
pure ledger `model::export` (`ExportLedger::{ingest, fold}` — series
identity `(origin, producer, declared pattern, {var} bindings, field)`,
values from a structural number, the RFC 11 `{type, value}` tag or
top-level numeric fields; every refusal counted by reason; drops taint the
series fed meanwhile; coalescing counted as the third O6 kind), the
exposition `model::prom::exposition` (a pure function of the snapshot,
names and units from the registry, deterministic bytes) and the wire shape
`report::ExportSnapshot` with `SeriesRow`/`SeriesState`/`ObserverCounters`/
`ContractCounters`/`DoctorSummary`, pinned; `report-fixtures::
export_snapshot` and `tests/fixtures/export.prom`. `zenctl export` serves it
(see `zenctl/CHANGELOG.md`); `docs/redesign-2026-07.md` §6.1's Daemon row
records it as the third of the permitted second kind.

**Consumers and blast radius — the admin space answers who reads this**
(#224). `zenkey-fleet` gains the consumers join: `consumers` and
`subject_impact` under `bus/admin.rs` (one pass over the admin space —
topology, the five declared-entity sweeps, attachments from the token
entities already in hand) and the pure `model::consumers::join_consumers`
that relates every declared subscriber and querier to a target by
`zenoh-keyexpr` and attributes it to a session on the admin `sources`;
wire shapes `ConsumersReport`, `ConsumerRow`, `AdminAnswer`, `Relation`,
`Attribution`, `SubjectImpact`, `DeprecationFact` under `report/`, pinned.
`AdminAnswer::NotAvailable` is *not asked*, never an empty set (RFC 13 §3
O4); nothing here is matching status (RFC 12 §9). `zenctl registry
consumers|impact` render it (see `zenctl/CHANGELOG.md`); zengui's
Inspector gains a Consumers section — one admin sweep per click, never
ambient. Also: `origin_attachments` goes through the pure `attach_tokens`,
`declared_entities_within` keeps the elided count, `EntityKind::ALL`.
### `zenkey-fleet`

- **The fleet timeline** (#216): `model/timeline.rs`, a pure projection
  from a window of samples — live `SampleView`s or `.zrec` lines — to one
  merged ordering on a stated clock, lanes per origin/producer, and the
  three provenances of a position kept apart. `Placed<HlcAxis>::new` is the
  only way onto the HLC axis and refuses an unstamped row (a compile-fail
  doctest pins that it cannot be promised for an arbitrary row);
  `HlcClaim` rests on the zenoh 1.10 fact the module doc cites (an HLC is
  updated on receive only where the node has one — routers by default,
  peers and clients not). `report/timeline.rs` pins the wire; every row
  carries `order_by`; the sequence-number lane is structurally
  unavailable with a fixed reason. `ZrecItem::Sample` gains `source`, so a
  replayed window classifies its stampers exactly as the live one did.
  Deliberately no edges.

### `zenctl`

- `timeline`, a new wire verb at the root — see `zenctl/CHANGELOG.md`.

## 0.8.0 — what the adopters found (2026-09-06)

**Tagged `0.8.0`** (bare, per the scheme since 0.7.1) on 2026-09-06; the three
library crates published to crates.io the same day.

The week after 0.7.2, two adopters — tcgui and zensight — filed five
findings that were all one shape: the contract could not *say* something,
so no tool could *judge* it, so the application invented a local rule and
the wire drifted. This release is the text first (RFC v1.31, v1.32, v1.33),
the code that enforces it, a third binary, and the two RFC 09 recipes that
had gone undeployed for a year because nobody could write them by hand.

`zenkey` and `zenkey-build` are **breaking** (the slug re-keys; the
framework enums grow); `zenkey-fleet` is breaking (`CheckId` grows,
`InterfaceShow` gains a field, `schemas_for_type` is gone); `zenctl` and
`zengui` gain verbs and rows without moving any. `zenwatch` is new.

### The convention (RFC v1.31 → v1.33)

* **v1.31, the adopters' batch.** [03 §2] the slug is injective — one
  reserved prefix `x-` on both sides of the passthrough boundary, `_xHH`
  escapes with no closing underscore, the decoder stated beside the encoder
  (see the `zenkey` section for the re-keying table). [05 §3.2] a bounded
  reply is an envelope (`items`, `next_cursor`, `partial`, `scanned`,
  `covers_from`), with a value cursor and "partial with no cursor" named a
  contract violation — the issue that asked for it cited a section that did
  not exist. [04 §4, 11 §2, 09 §2] telemetry history as a computed answer
  served by an ordinary host-origin producer.
* **v1.32, the declaration batch.** [08 §2] `kind = counter | gauge | text |
  bool` on a subject, and a per-producer `[budget]`; [04 §1.2] the health
  document MAY carry `self_stats`; [13 §3] both judged on the four poles —
  a producer that publishes no `self_stats` is *unobservable* and says so.
  Errata: 08 §2's `common` row had stopped at v1.25's vocabulary (#425).
* **v1.33, the generators.** [09 §3] the fifth ACL fact — interest is
  evaluated on egress against the responding face's subject, so own-origin
  grants leave a peer-mode publisher publishing to nobody; the shared
  egress-only `interest-prop` row — and the two 09 recipes become tooling.
* **The Docs lane** (#420): `docs.yml` runs the RFC gate when and only when
  prose changes, and the gate now checks that 00-index, CLAUDE.md and
  README.md name the newest ledger version (two of them were at v1.28).

### `zenkey` 0.8.0 — breaking

* **The slug is injective** (#418, RFC 03 §2 v1.31). It was not, twice
  over: the v1.4 escape produced chunks that were themselves legal values,
  so the literal `x_x5f_myns` and the escaped `_myns` shared a key (tcgui#39,
  found slugging Linux device names, where both are legal), and its leading
  marker `x` was also a byte a value could start with, so `x@b` and `@b`
  collapsed to `x_x40_b`. The v1.31 rule reserves **one prefix on both sides
  of the boundary**: a value passes through only when it is charset-legal
  *and* does not start with `x-`; everything else is `x-` plus a body in
  which every byte outside `[a-z0-9]` is `_xHH` (no closing underscore),
  `.` and `-` staying literal except as the last byte, and the empty value
  is `x-_x`. `chunk_unslug` / `Chunk::unslug` is the decoder, shipped beside
  the encoder with a round-trip test as the RFC now requires; it is the left
  inverse on the slug's image and refuses everything else.

  **This re-keys every value that was ever escaped**, and every legal value
  that starts with `x-`. Clean values, IP slugs and ULID slugs are
  byte-identical. The 0.6 → 0.7 move from the `e` sentinel to the `x`
  marker never had a changelog line; this table is the record of all three,
  and `slug_outputs_are_pinned` fails the build the next time a row moves.

  | value | 0.6 | 0.7 | 0.8 |
  |---|---|---|---|
  | `ETH0` | `e_x45__x54__x48_0` | `x_x45__x54__x48_0` | `x-_x45_x54_x480` |
  | `_myns` | `e_myns` | `x_x5f_myns` | `x-_x5fmyns` |
  | `*` | `e_x2a_e` | `x_x2a_x` | `x-_x2a` |
  | `foo@1.service` | `foo_x40_1.service` | `foo_x40_1.service` | `x-foo_x401.service` |
  | `x-foo` | `x-foo` | `x-foo` | `x-x-foo` |
* **The framework vocabulary reaches v1.30** (#425): `CommonState` gains
  `EvidenceRelation`, `CatalogIncident`, `CatalogAck`, `CatalogSilence`,
  `CatalogEdge`; `CommonFamily` gains `EvidenceRelation` (`ALL` is 8). The
  enums stay exhaustive by design, so a consumer's `match` grows.
* `SubjectDecl.kind` (`SubjectKind`), `RegistrySlice.budget` (`BudgetDecl`,
  `TableBudget`) — both round-trip byte for byte when absent.
* `alert::alert_ref` / `parse_alert_ref` (RFC 11 §3.2).

### `zenkey-build` 0.8.0 — breaking

* The five v1.29/v1.30 `common` tokens lint and generate (#425); a registry
  declaring `common = "edge"` no longer fails the consumer's build.
* `kind` is carried into `Subject::kind()`, `AnySubject`, the AsyncAPI
  export and `registry.lock` as an optional sixth column: adding one is
  *stale* (regenerate), changing or removing one is *incompatible*.
  `check_compat_lock` now compares shape columns strictly and metadata
  separately.
* `[budget]` is linted (non-negative, unique table names) and rides
  `REGISTRY_TOML` verbatim — no codegen.

### `zenkey-fleet` 0.12.0 — breaking

* **`CheckId` gains `kind-mismatch` and `budget-exceeded`** (`ALL` is 23).
  The doctor's listen phase now watches the liveliness selectors, so a
  counter reset across an `alive` cycle is not a finding; `--deep` fetches
  each budgeted producer's health documents.
* **`interface show` reports the engine's drift verdict per origin**
  (#410): the describe sweep moved into `bus/describe.rs`; `InterfaceShow`
  gains `drift: Vec<SchemaDrift>`; `schemas_for_type` — which kept the
  first reply per producer and no origin — is removed.
* `CallAnswer::page_signal()` reads the RFC 05 §3.2 envelope (#424).
* For zenwatch, none behind `decode`: `AlertTransition` / `alert_transition`,
  the catalog documents (`EntityDoc`, `AliasDoc`, `EdgeDoc`, `EdgeKind`),
  `attribute` / `entity_of` (impact, depth-capped, containment kinds only),
  `doctor_delta` (moved out of zengui).
* The storage and ACL planners (`plan_storages`, `plan_acl`, their checks
  and explains) with pinned plan shapes.
* `rpc_key` is decode-gated like both of its callers.

### `zenctl` 0.6.0

* **`storage gen`** (#393) and **`acl gen`** (#392): the RFC 09 §2 and §3
  blocks derived from the registry — `lifespan` computed and shown, the
  grant matrix expanded per principal with `interest-prop` on every
  publishing policy — each with `--json5`, `--check` and `--explain`. The
  running ACL is not readable from the zenoh 1.10 admin space, so
  `acl gen --check` reads the router's config file. Details in
  `zenctl/CHANGELOG.md`.
* `interface show --schema` names both hosts of a disagreement (#410);
  `call` says *stopped early* on a partial page and flags a null cursor
  (#424); `topic info` shows `kind`; `doctor` carries the two new checks;
  `registry export --as toml` keeps `[budget]`.

### `zengui` 0.4.0

* The registry-facts line shows a subject's `kind`; the doctor panel's
  run-over-run delta now comes from the engine.

### `zenwatch` 0.1.0 — new

The notifier daemon (#387 — #388, #389, #390): one JSON5 config, N rules
over the engine's closed watchdog vocabulary plus `alerts <SEL>` and
`liveliness-gone <SEL>`, M sinks (ntfy, webhook, exec, smtp; secrets by env
name or file path only), the three states preserved into the sink payload,
`--dry-run`, `check-config`, `test-sink`; `for` duration, dedup, grouping,
repeat, inhibition over the catalog's containment edges, a bounded
persisted ledger that survives a restart; self-publication (`health`,
`firing/{rule_id}`, `doctor`, `introspect`/`describe`, `alive`) so something
can watch the watcher; and a scheduled doctor routing run-over-run deltas.
Ships as a Forgejo release binary beside zenctl and zengui. HTTP rides
reqwest on `rustls-no-provider` with the ring provider zenoh already uses —
no second crypto provider (`cargo tree -i aws-lc-rs` is empty).

### Housekeeping

* The instance runner did not execute a single job on the day this was
  built; every merge carries the local gate list in its PR body, and CI
  runs on `main` when the runner returns.

---

## 0.7.2 — a tombstone is not a value (2026-08-30)

One fix, one crate: `zenkey-fleet` 0.11.1. Nothing else is republished.

* **The doctor no longer judges `Delete` tombstones as payloads**
  (zensight#830). A retirement rides the bus as a `Delete` sample with an
  empty payload (RFC 04 §1.2); `observe_traffic` never looked at
  `SampleView.kind`, so the empty body fell through the encoding sniff to
  CBOR and ciborium's `UnexpectedEof` surfaced as a `payload-undecodable`
  **error** against a correct retire. The alert plane is exactly where
  put-then-retire is routine, so any fleet that clears an alert inside a
  doctor window drew a manufactured error finding. Deletes now skip the
  field-intelligence parse and the decode/validate ladder; the wire facts
  about the publisher — QoS axes, registration, stamping, event rate —
  stay judged, because a tombstone rides the same declared publisher and
  can be wrong in all the same ways. The regression test had to *serve a
  schema* to reproduce the bug: without one the ladder stops at `NoSchema`,
  which is why no fixture ever hit it.

---

## 0.7.1 — what the fleet does not agree about (2026-08-29)

**Tagged `0.7.1`, not `v0.7.1`** — the tag scheme goes bare here (#412); see
the build section below.

A small release four days after a large one, and every item in it is a
correction. Six issues, filed as one audit batch the day after 0.7.0's work
settled, each recording something the code or the RFC got wrong, promised
without delivering, or deferred to work that had since shipped.

Two of them are the same defect on two planes, and the pattern is worth
naming because it will recur: **a fan-in answer that drops the origin that
gave it**. A fleet GET goes to `*/@rpc/<producer>/…`, so N hosts answer;
keep one and you have a report that says a producer is wrong and gives
nobody a host to go and look at — and worse, a producer whose two hosts
disagree collapses to one claim and stops being a finding at all. The
mid-rollout fleet, which is the case these checks exist for, read as
agreeing.

`zenkey-fleet` is breaking. `zenkey` and `zenkey-build` are untouched and
are not republished; their 0.7.0 remains current.

### The engine

* **The describe sweep keeps the origin that answered** (#398). `describe`
  fans in across every host running the producer, and the sweep kept the
  first parseable reply. `SchemaServer` gains `origin`, `schema_drift`
  compares **answers** rather than producers — so one producer on two hosts
  at two hashes is a disagreement it can see — and the finding reads
  `producer@origin (hash)`. `DescribedSchema` is the attributed value, the
  opposite number of `ServedSlice`. *Breaking: `SchemaServer` has a new
  field, `schema_drift` takes a new input type.*
* **The watchdog is a `Sipper`** (#397). `run_watchdog`'s `emit` callback
  was infallible by construction, so a caller that could fail while emitting
  had to stash the error and answer for it after the run. It is now
  `watchdog`, returning a `Straw<WatchdogSummary, Transition, Error>`:
  transitions while it runs, the summary when it stops, the acknowledged
  monitor teardown in between. `sip()` for the sequence, `await` for the
  output — nothing to forget, and a consumer that gives up mid-run still
  gets the teardown. *Breaking: renamed, no callback, returns a `Straw`. New
  dependency `sipper` 0.1, re-exported so consumers need no direct one.*
* **The registry collapse is three-state** (#399). `SliceSet::collapsed()`
  returns `Asked<&[CollapsedProducer]>`: an empty list could not tell
  *asked, and every producer had one origin* from *built from files, which
  have no origin to collapse*. `CollapsedProducer` moves to `report/` —
  it is rendered now, so it is a wire shape. *Breaking: accessor signature,
  module, and the loss of `#[non_exhaustive]` on the move.*

### The explorers

* **`zenctl registry diff` stops printing `agree` about a comparison it made
  against one of several answers** (#399). The diff carries the receipt, the
  row names both hosts and both versions, and a served side that never came
  off the bus says the question was never put rather than printing a silence
  that reads as agreement.
* **Every registry-aware verb says when its answer came from a pick**
  (#399), through a fourth `resolve::notes` sentence emitted from both slice
  loads. Worded to be unmistakable for the existing bus-versus-checkout
  note: this one is the fleet against *itself*.
* **zengui's status strip gains `· N FLEET-SPLIT`** beside `· N DISAGREE`
  (#399). The `Dirs` source gets no such field rather than a zero — files
  have no origin to disagree across, and never asked.
* **`zenctl watchdog` returns a write error where it happens** (#397). The
  stashed `io::Error` and the after-the-run `BrokenPipe` re-check are gone.

### The convention (RFC v1.28)

* **The storage history mode is per volume, not per storage** (#401).
  v1.27 added a `redb` row to 09 §2.1 ahead of the backend, with a status
  note promising to remove it on shipping. `zenoh-backend-redb` #10 and #11
  closed 2026-08-28 — and chose differently, so this is a correction rather
  than a caveat removal: Zenoh asks the *volume* for its capability, and the
  storage manager decides replication and outdated-sample dropping from that
  answer. §2.2's "a misconfiguration a deployment should be told about"
  becomes the startup refusal it is, and the row gains what the capability
  pair does not say — mandatory retention, kept tombstones, `_time`-ranged
  reads.

### The build, and how a release is tagged

Not part of the audit batch — the fleet-uniformity rollout (#412, part of
#407) landed in the same window and ships here.

* **Tags are bare `X.Y.Z` from this release on**, not `vX.Y.Z`. 0.7.1 is the
  first, and there is no retro-tagging: the `v`-prefixed tags up to and
  including `v0.7.0` stay exactly as they are. `release.yml` also gains a
  `workflow_dispatch` that re-releases an existing tag, so a lane that failed
  on a runner rather than on the code no longer needs the tag deleted and
  pushed again.
* **The toolchain is pinned**: `rust-toolchain.toml` at 1.97, `rust-version =
  "1.97"` in `[workspace.package]`, and all eight members inherit it. The
  MSRV a consumer reads and the compiler CI uses are now one number.
* **`cargo deny check` is a CI job**, with `deny.toml` as the single
  supply-chain lint. Its advisory ignores are transitive through zenoh 1.10
  and dated, to be re-checked on every zenoh bump.
* `chacha20` moves 0.10.1 → 0.10.2, off a yanked release.

### Documentation

* **Four forward references that had outlived their issues** (#400, #402).
  Three engine module docs promised a zengui surface "deferred to a later
  window"; all three had landed (#214, #223, #221). `Subject::Key`'s
  deferral to the path arena is answered rather than deferred: arena ids are
  per-flatten and a `Subject` outlives every flatten, so a `PathId` there
  would resolve to a different key than the one clicked.

---

## 0.7.0 — the API we mean to keep (2026-08-25)

**306 commits. Breaking, deliberately, across every crate.** This is the
release where the published surface stops being the surface that happened
and becomes the one that was chosen: the breaking work was gathered into one
window rather than dribbled across four, so a consumer upgrades once.

The convention moves to **RFC v1.26**, which carries the first
wire-observable change since ratification.

### The convention (RFC v1.26)

- **`frame` loses `express`; `alert` keeps it** (04 §3). Traced through the
  transport rather than argued from the labels: `express` forces the current
  batch out, batching engages *only* under back-pressure, so express is a
  no-op on an unsaturated link and spends per-message overhead exactly when
  a `drop` profile is supposed to be shedding. The axis is
  rare-and-must-arrive versus continuous-and-sheddable. `express` is one of
  the four axes `doctor`'s `qos-observed-mismatch` compares, so a `@media`
  publisher not yet rebuilt is *correctly* reported as deviating.
- **The stream control surface is what is actually served** (07 §1.1, 11 §5):
  one `stream/set` carrying a tagged command, not the three keys the prose
  named and nothing ever served. `@rpc/<producer>/stream/report` is newly
  ratified for receiver feedback.
- **Adaptation is receiver-driven** (07 §1.2, normative): a producer MUST NOT
  re-tune a shared tier from one consumer's report.
- **The frame-age clock is named** (07 §1.3): the sample's publisher HLC,
  read as observed skewed latency, negatives shown rather than clamped.
- **Browser consumers** (07 §1.4) and the two consequences the text had made
  inevitable without stating.
- **Retirement covers procedures** (08 §3): `[[deprecated]]` takes a `kind`,
  and `deprecated.lock` a leading kind field. Before this, a procedure
  removed from a `compat = "backward"` registry had **no sanctioned exit at
  all**.
- **The `streams` procedure is profile-local** (11 §5.1), and stays.

### `zenkey`

The typed core: what used to be a `String` and could have been anything is
now a type that could only ever have been what it is.

- `RegistrySlice` carries its vocabularies as types, not strings.
- The producer position is typed, like the origin beside it; position 5 is
  one field because it was always one choice.
- The schema entry is the tagged union it always was.
- An application names its two constants and the compiler checks them.
- One pattern grammar, real error chains (`KeyError` gained variants where a
  string used to carry the reason), and a full API-guideline pass:
  `#[non_exhaustive]` on every parse result, `#[must_use]`, `Display`
  before `to_string`.

### `zenkey-build`

- A lint failure has a **kind** (`Incompatible`, `Vanished`, `Stale`,
  `BadLine`, `Invalid`), so a caller can classify without matching on
  prose, and a warning is a value rather than a line on stderr.
- `[[deprecated]]` and `deprecated.lock` cover procedures (RFC 08 §3).

### `zenkey-fleet`

- **The crate root is the whole supported surface, as it says.** A module
  path is *a* spelling, never the only one.
- **The engine classifies its own failures**: `zenkey_fleet::Error` with a
  `Display`/`source()` convention (Display says *what* failed, `source()`
  says *why*) and `is_unaskable()` for the exit-2 boundary. `anyhow` is gone
  from the crate.
- The check and rung vocabularies are types, in `report/`. Judgements take
  named evidence.
- **Two producers that said nothing are not agreeing**: `schema_drift` read
  two absent hashes as agreement, which is an O4 failure — a `SchemaDrift`
  report with a `DriftVerdict` replaces the bool.
- One timer per window, bounded declarations (every `declare_*` gets a
  deadline), and a bounded observation limit shared with the GUI.
- `Duration` at the boundaries, `f64` seconds in the reports.

### `zenctl`

- **One tree, one flag per concept, one exit contract** — the command-tree
  cut, with its full old→new table in `zenctl/CHANGELOG.md`.
- The exit contract is a type and a value (`exit::Asking`), not four
  spellings of the same `match`.
- `run()` is 107 lines of arms rather than 462 lines of arms *and
  decisions*; all fourteen `#[allow(clippy::too_many_arguments)]` are gone,
  and with them the transposition hazards they were hiding.
- `doctor --transitions` and `watchdog` answer for their primary output
  instead of dropping write errors.

### `zengui`

- Subject slots and pins (#181, #257): a torn Inspector *is* a pin, and one
  bus tick fans out to every slot's recorder — a pin costs a bounded ring,
  never a second subscription.
- Windows: the process owns N of them, and a role has one home.
- Spacing off the 8pt grid by role, gated by `check-spacing.sh`.
- `services::ServiceError` keeps a failure's cause chain across the message
  seam, where `map_err(|e| e.to_string())` used to keep one sentence.
- One `kit::Viewport` for the four virtualized lists; the frame path stops
  re-allocating strings it materialised one line earlier.

---

## 0.6.0 and earlier

See the git history and `zenctl/CHANGELOG.md`. This file starts at 0.7.0
because that is the release where the workspace as a whole became worth
describing in one place — 306 commits is more than a tag message can hold.
