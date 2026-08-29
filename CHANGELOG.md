# Changelog

Releases of the workspace as a whole. The convention's own amendment ledger
is [`rfcs/CHANGELOG.md`](rfcs/CHANGELOG.md); `zenctl`'s command-tree cut has
its own migration table in [`zenctl/CHANGELOG.md`](zenctl/CHANGELOG.md).

Versions per crate, because they move independently:

| Crate | 0.6.0 | 0.7.0 | 0.7.1 |
|---|---|---|---|
| `zenkey` | 0.6.0 | 0.7.0 | 0.7.0 — unchanged |
| `zenkey-build` | 0.6.0 | 0.7.0 | 0.7.0 — unchanged |
| `zenkey-fleet` | 0.9.0 | 0.10.0 | **0.11.0** |
| `zenctl` | 0.4.0 | 0.5.0 | **0.5.1** |
| `zengui` | 0.2.0 | 0.3.0 | **0.3.1** |

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
