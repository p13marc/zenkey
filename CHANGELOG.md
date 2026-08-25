# Changelog

Releases of the workspace as a whole. The convention's own amendment ledger
is [`rfcs/CHANGELOG.md`](rfcs/CHANGELOG.md); `zenctl`'s command-tree cut has
its own migration table in [`zenctl/CHANGELOG.md`](zenctl/CHANGELOG.md).

Versions per crate, because they move independently:

| Crate | 0.6.0 | 0.7.0 |
|---|---|---|
| `zenkey` | 0.6.0 | **0.7.0** |
| `zenkey-build` | 0.6.0 | **0.7.0** |
| `zenkey-fleet` | 0.9.0 | **0.10.0** |
| `zenctl` | 0.4.0 | **0.5.0** |
| `zengui` | 0.2.0 | **0.3.0** |

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
