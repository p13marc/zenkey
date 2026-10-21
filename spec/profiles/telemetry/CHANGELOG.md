# `telemetry.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.1 — 2026-10-21: the first text, draft (#819, TA)

`telemetry.v1` is the profile that zengui's units, the fleet's
continuity reader and the doctor's `telemetry-*` checks wait on, in place
of v1's dark `kind-mismatch` (`docs/zk2/migration.md`). Before it, the
four `telemetry.*` keys were core Appendix D's interim row, which nothing
read, and `telemetry.v1.Point` was a schema file inside
`examples/zk2/zensight/`, listed by four ZenSight contracts, whose counter
had no epoch. This text is written against core 0.35, which lets a
profile's first point be a standard schema and publishes the vocabulary.

**Decided on #819, before the text** (the maintainer's defaults, not
reopened here).
- **What the profile contributes:** its vocabulary, `kind`, `unit`,
  `buckets` and `semantic`, published; a standard schema,
  `telemetry.v1.json`, moved under `spec/profiles/telemetry/`; no standard
  contract and no kind; point 4 through `uses` only.
- **Six kinds,** the Point's tags, one spelling on the annotation and the
  wire: v1's `bool` is `boolean`.
- **Two carriers:** the Point, whose tag describes a member whose resource
  declares no kind, a declared kind being authoritative; and a bare value
  declared by `telemetry.kind`. Composites and protobuf wrappers are not
  metrics at 1.x; a field-level kind map is considered, not adopted.
- **Counters:** 0 to 2^64−1, read exactly, never decreasing within one
  epoch; widened by their producer; a wrap reads as a reset.
- **The epoch:** in-band (the Point's new `epoch`, or an Epoch
  attachment), else the instance, else a decrease reads as a reset; the
  finding is a decrease within an established epoch.
- **Rates:** from two samples of one epoch, timed by the Point's
  `timestamp`, else the receipt; none across a reset; a producer's rate is
  a gauge with a rate unit.
- **Histograms:** ZenSight's shape, normative; `buckets` iff `histogram`;
  quantiles and deltas the reader's.
- **Units:** UCUM SHOULD, byte for byte, the annotation then the Point,
  else unknown; modem's `dBm` and `us` valid; a label named `unit` is
  context.
- **`semantic`:** v1's closed eight, a hint, never a finding.
- **Streams and state;** events and operations ignore the vocabulary.
- **Labels** are context; **absent is not zero.**
- **Out of scope:** export naming, the historian's rollups, a published
  quantile's window.

**Stated.**
- **The schema moves** under the profile (§3): its stem and every
  `$defs` name kept; the Point gains an optional top-level `epoch`, a
  `$ref` to the new `telemetry.v1.Epoch` (a string of 1 to 64
  characters); the descriptions cite this text. No keyword inside
  `telemetry.v1.Value`'s `oneOf` changed, which core §9.8 would classify
  review. Its id moves from `sha256:3d6276dd…` to `sha256:715ce537…`, and
  both are published. The four ZenSight contracts list it by its new path
  and are republished at minor 1, each compatible (Appendix B).
- **Metric resources and who lists the profile** (§2.1), **the
  vocabulary's values** with a closed list of reasons for an invalid
  declaration (§2.2), **the standard schema** and identity by artifact id
  (§2.3), **carriers** (§2.4), **a member's kind and unit** (§2.5), **a
  sample's conformance** with a closed list of reasons (§2.6),
  **counters** (§2.7), **histograms** (§2.8), **the epoch** (§2.9), **a
  pair's continuity** (§2.10) and **rate** (§2.11), **time, labels and
  absence** (§2.12), **producers' rates** (§2.13), **constrained faces**
  (§2.14), **device-as-service** (§2.15).
- **What a tool may conclude** (§5): a declaration's validity, a member's
  kind and unit, a sample's conformance, a pair's continuity and its rate;
  the changes against v1's RFC 08 §2, RFC 11 §4 and RFC 13 §3
  (Appendix A); the adopters' mappings (Appendix B).
- **Evidence:** `conformance/schema.json` (9 cases),
  `conformance/declarations.json` (87), `conformance/samples.json` (101),
  `conformance/continuity.json` (43), and eight scenario sections.
- **`link.v1` 0.8** cites §2.9 and §2.14 for the counter and its epoch,
  where 0.7 waited for "`telemetry.v1` defines its counter type".

**Decided here, where the record was silent.**
- **A metric resource** is a stream or state resource that holds a key of
  the vocabulary, or is typed by the Point (§0). The issue's wording named
  `telemetry.kind` alone; a unit with no kind on a type other than the
  Point would then belong to no question, so it is a metric resource
  whose declaration is invalid (`kind_missing`).
- **Each key checked alone,** a check that needs a wrong value skipped, as
  `link.v1` §2.1's lints read: one wrong kind is reported once, never as a
  carrier problem too.
- **A semantic outside the eight** makes the declaration invalid, like any
  other value that is not one. "Never a finding" is about samples: no
  sample is judged by its semantic.
- **A nullable is not a carrier,** in either spelling of core §7.3: `null`
  is no value of any kind, and absent is not zero.
- **The Histogram is a bare carrier by its artifact's id,** as the Point
  is; a bare `binary` is a raw type, since no JSON Schema type carries
  bytes but the Point's array.
- **The epoch's attachment rides only a bare counter or histogram.** The
  Point carries its own; an Epoch attachment on it, or on another kind,
  carries nothing.
- **How numbers are read** (§2.6): an integer exactly, a float as its
  double, an integral double as the integer it equals (JSON Schema's
  `integer`, `freshness.v1` §2.1), so `5.0` is a counter and
  `18446744073709551615.0`, the double 2^64, is out of range. A CBOR
  reader keeps a non-finite float, which is `not_finite` here, never
  `null`.
- **The Point's `timestamp`** is read from −2^63 to 2^63−1, ZenSight's
  `i64`; beyond it the payload is not a Point.
- **A Point's `unit`** is held to the annotation's rule (`unit_invalid`),
  so that a unit read from the wire is one a reader can compare.
- **Bounds compare as doubles,** where RFC 11 §4 said bit for bit: the
  canonical form writes `1.0` as `1`, so bits are not what a bundle
  keeps.
- **Presence evidence** needs one instance alone without a loss across
  the whole span for "the same epoch", and two different single instances
  at the two arrivals for "a different one"; anything else establishes
  nothing, with the residual risk of core §4.3 stated (§2.9).
- **The rate's formula** is stated in IEEE operations, so two readers
  agree bit for bit (§2.11), and the fixture compares exactly.
- **A pair of an undeclared Point whose tags differ,** or of histograms
  over other bounds, is `incomparable`, never a reset or a finding.

**Deliberately not stated.**
- **No lint code** for an invalid declaration. The core has no code for an
  annotation's value (core 0.35), and its own fixtures use
  `telemetry.unit` and `telemetry.buckets` as carriers for E027 and
  `ok-float-integral`. A linter SHOULD warn (§2.2).
- **No rate of a histogram,** no quantile, no delta. A reader's, and
  nothing asks for a rule yet.
- **No field-level kind map** for a composite (tcgui's states): a
  composite is not a carrier at 1.x (§2.4).
- **No unit conversion,** and no UCUM parser: a unit is a label.
- **No meaning on a requirement,** as in `freshness.v1`.
- **No change to modem's contract:** adding an Epoch attachment to
  `sdu/dropped_total` is review (core §9.8, `attachment_changed`), and its
  revision is the adopter's (Appendix B).
- **No runtime and no tool here.** `zenkey::telemetry` (an owner's
  Points carrying the instance as their epoch, a writer refusing what its
  declaration refuses) and the scenarios as tests are #819's TB; the
  fleet's continuity reader, `check conform`'s and the doctor's
  `telemetry-*` checks and zengui's units are its TC.
