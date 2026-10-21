# `telemetry.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.3 — 2026-10-21: the tools' reading, and two findings of round 46 (#819, TC; #609)

The fleet's continuity reader (`zenkey_fleet::model::telemetry`, #819's
TC) reads §2.9 to §2.11 over a live bus, behind `zenctl telemetry`,
`check conform`'s three `telemetry-*` cases and the doctor's two checks,
and runs `scenarios.md` §8 across a real face. Reading presence as a tool
reads it found two places the text left to the reader. zk2py read 0.2
again (round 46) and filed two findings, F-150 and F-151, and the review
of its code found a third gap, a data item's depth. Each answer is the
Rust model's and zk2py's reading already, so this is text and fixtures;
written against core 0.35, unchanged. The standard schema, its ids,
`declarations.json` (87), `continuity.json` (45) and `schema.json` (9)
are unchanged; `samples.json` gains twelve cases (137), written by hand,
which pass as written. No case that existed before changes.

**Answered: round 46's findings.**
- **F-150, "past the double's finite range" at its edge** (§2.6, Strict
  JSON). 0.2 made `1e400` and 10^400 undecodable, and left the band
  between the largest double and the halfway point to the next power to
  two readings: an integer's text one past the largest double is "past the
  range" read literally, and the largest double read by rounding.
  - Now "finite" is by rounding: a number's text, an integer's included,
    is finite when its exact value rounded to the nearest double (ties to
    even) is not an infinity. The edge is a magnitude of 2^1024 − 2^970,
    the largest double and half its last unit, a tie that rounds to even,
    2^1024: undecodable, and so is any text past it. Text below it
    decodes. zk2py's reading (Python's `float()`) stands, and so does the
    model's, `str::parse::<f64>`, correctly rounded.
  - `samples.json`, six cases in a gauge: `1.7976931348623157e308`
    (conforms), `1.7976931348623158e308` (rounds down, conforms),
    `1.7976931348623159e308` (undecodable), the largest double's integer
    text (conforms), 2^1024 − 2^970's (the tie, undecodable), and one
    below it (conforms).
- **F-151, an Epoch attachment's strictness** (§2.6 check 2). 0.2 made a
  sample's JSON strict and said only that an attachment decodes "as its
  encoding", so `"\ud800"`, a lone surrogate escape, was an epoch of one
  code point to one reader and `epoch_invalid` to another.
  - Now an Epoch attachment is read as §2.6 reads a sample of its
    encoding, strict JSON or CBOR, depth included; one that does not
    decode so is `epoch_invalid` (F-147). zk2py's reading stands; the
    model's `Datum::attachment` was strict already, and the runtime's
    writer, which reads an attachment through it, refuses one so.
  - `samples.json`: an `attachment_text` case, the lone surrogate.

**Answered: a data item's depth** (§2.6, from the review of round 46's
code, not a finding). The model's decoders stopped at 256 levels and
0.2 said nothing of depth, so a reader with no limit read a deeper sample
as decoded, `does_not`, a finding, where the model read it `undecodable`,
unobservable.
- Now a data item nests at most 256 arrays and maps deep: an array or a
  map at the top is at depth 1, one inside another one deeper; a scalar
  adds no depth, nor does a CBOR tag, read as its content. An array or a
  map at depth 257 is undecodable, in JSON as in CBOR.
- The model's JSON reader counted so already. Its CBOR reader counted
  every item from 0, tags and scalars included, so it read 257 nested
  arrays and stopped short of 256 under a tag; it now counts arrays and
  maps alone, as the JSON reader does, and reads a chain of tags without
  recursion (`telemetry::decode`, `both_readers_count_depth_alike`).
- `samples.json`: arrays nested 256 deep and 257 deep, as `json_text`
  and as `cbor_hex`, and a tag around 256 (still 256 deep).
- The bound is the profile's reading of a sample alone: core D000, which
  reads a descriptor, is not changed.

**Answered: what a tool's reading of presence found** (§2.9).
- **A loss between two arrivals establishes nothing.** 0.2 said a loss on
  the presence view establishes nothing, and required "without a loss"
  only of the same epoch; read literally, the only instance seen at each
  arrival, with a loss between, still established another epoch. A
  reader that lost deliveries between the two arrivals may have lost the
  very token moves that would place the pair, so now neither the same
  epoch nor a different one is established across a loss, whatever the
  view showed at each arrival. `continuity.json`'s `presence` is what a
  reader saw: across a loss a reader gives none, as the fleet's reader
  does. The case "the same instance at both arrivals, with a loss
  between" (unchanged) reads the same either way.
- **What the view stands on.** A reader's view of the tokens stands on a
  complete read of them (core §8.1), never on a subscription's history
  alone, which has no end mark, and on nothing lost since; it folds the
  tokens' moves and the samples in the one order it received them. The
  fleet's reader seeds presence after subscribing, reads it again after a
  loss, and gives no presence to a sample before a complete seed.

**Also stated: a tool over many pairs** (§5). A pair whose epoch was
assumed answers `same_epoch` or `reset`, never `decreased`: a no without
the evidence for a yes. A tool asked "did anything go backwards?" over
pairs whose every epoch was assumed SHOULD say it could not tell, never
that nothing went backwards; `zenctl telemetry` exits 2 there.

**Deliberately not changed.**
- **The per-pair answers of §2.10 and §5's table:** assumed stays a no,
  and a decrease without an established epoch stays a reset.
- **No clock is trusted** by §2.11: two timestamps of the owner's, two
  stamps of the owner's, or two receipts of the reader's, never one clock
  against another. The tools take no word of the deployment's for one.
- **Core D000** and every other profile's reading of JSON.
- **The fixtures' presence input** stays a triple: what a reader saw,
  which a loss leaves it without.

## 0.2 — 2026-10-21: the cold reader's findings (#819, TB; #609)

The spec-only implementation read 0.1 cold (zk2py round 45, PR #821):
every fixture passed, 243 cases of `telemetry`'s, and `scenarios.md` §1 to
§8 ran against its own owner. It filed eight findings, F-142 to F-149,
each a place where two readers following 0.1 could read two answers. The
runtime (`zenkey::telemetry`, #819's TB) was built beside them: its
writers check what they put through the model's `check`, so every answer
that changes what `check` reads changes what an owner may put. Building it
found nothing more. Written against core 0.35, unchanged; the standard
schema, its ids and every declaration are unchanged.

Each answer is stated in the text and pinned by a fixture case where a
fixture can pin one. No case that existed before changes: the new ones
were written by hand and pass as written. `samples.json` gains three input
forms (`json_text`, `cbor_hex`, `attachment_text`: the bytes as they came)
and `continuity.json` one (`stamp_ntp64`). The evidence is now
`samples.json`, 125 cases, `continuity.json`, 45, and nine scenario
sections; `declarations.json` (87) and `schema.json` (9) are unchanged.

**Answered: the eight findings.**
- **F-142, a stamp's nanoseconds** (§2.11). 0.1 said "a stamp's time in
  nanoseconds as zenoh reads it" and stated no conversion. A stamp is a
  whole number of 2^−32 s, so converting each stamp and subtracting, or
  subtracting and converting once, gave two `e` in about half the pairs
  zk2py measured, and two rates in their last bits.
  - Now §2.11 states zenoh's conversion, uhlc's `NTP64::as_nanos`: the
    seconds times 10^9, plus the fraction times 10^9 divided by 2^32 and
    rounded up, each stamp alone, `e` the difference of the two. zk2py's
    reading stands.
  - `zenkey_model::telemetry::stamp_ns` states it, and
    `zenkey::telemetry::stamp_ns` reads a zenoh `Timestamp` through
    `as_nanos`, tested equal. Nothing in the fleet timed a pair then: its
    continuity reader came with TC, at 0.3.
  - `continuity.json`: two cases by raw NTP64 values, one whose per-stamp
    `e` is 1000000001 ns where the converted difference's floor is
    1000000000, one whose two stamps are 0 ns apart (`elapsed`).
- **F-143, a JSON sample is strict JSON** (§2.6). 0.1 read "JSON's data
  model (RFC 8259)", which leaves duplicate members, the numbers' range
  and lone surrogates to the parser: two `timestamp`s could give two
  rates, and `1e400` read conforming, `not_finite` or undecodable.
  - Now a JSON sample is read as `alarms.v1` §2.4 reads a payload, core
    §3.3's D000: UTF-8, RFC 8259 with nothing after the value, no
    duplicate member at any depth, every string with a UTF-8 form, every
    number finite. Anything else is `undecodable`, `1e400` and 10^400
    written out as an integer included. An integer within the double's
    range is still read exactly, however long its text.
  - zk2py read `1e400` as +Infinity (`not_finite`) and 10^400 as a
    conforming gauge; both readings change. Its strict members stand.
  - The model's `Datum::from_json_bytes` read through `serde_json`, where
    the last duplicate wins and an integer past 64 bits is a double. It is
    now `telemetry::decode::json`, a strict reader whose integers are
    exact (`Datum::BigInt` past i128), and the runtime's writers and the
    readers go through it.
  - `samples.json`: eight cases as `json_text`: a duplicate member,
    `1e400`, 10^400, a lone surrogate, text after the value, whitespace
    around one, an integer's text past 2^64 in a gauge, and a timestamp of
    −2^63−1 read exactly (`not_a_point`, never the double −2^63).
- **F-144, CBOR's bignums and tags** (§2.6). 0.1's "CBOR's major types 0
  and 1" made a bignum no integer, so 2^64 in a counter was `wrong_type`
  in CBOR and `out_of_range` in JSON; the model, through `ciborium`, read
  a bignum of up to 16 bytes as an integer and a longer one as bytes.
  - Now an integer is major type 0 or 1, or a bignum, tag 2 or 3 around a
    byte string of any length, definite or not, read as the integer it
    denotes: one value, one reason, in either encoding. Every other tag,
    and a tag 2 or 3 around anything else, is its content, and a map is
    read as core §5.2 reads the envelope's: text keys, none twice,
    `undefined` as `null`.
  - zk2py read a bignum as its content, a byte string; that reading
    changes. Its tag-1 timestamp, read as the integer, stands.
  - The model reads CBOR from its headers (`telemetry::decode::cbor`, on
    `ciborium-ll`), so a bignum's length never decides, and one past i128
    is `Datum::BigInt`, which reads `out_of_range` as an integer.
  - `samples.json`: seven cases as `cbor_hex`: the bignum 2^64, a small
    one, one of 17 bytes, an indefinite one, a negative one of 17 bytes in
    a gauge, a tag-1 timestamp, and a map with a key twice.
- **F-145, when `counts_length` is checked** (§2.6 check 11). 0.1 said
  what checks 10 and 12 need, not 11. Now it is checked whenever the value
  is a Histogram: it needs the two lengths alone. That is the model's and
  zk2py's reading; nothing changes but the text. `samples.json`: no bounds
  with two counts, and an infinite bound with four counts for two bounds.
- **F-146, `incomparable` as a no-rate reason** (§2.11, §5). Two counters
  are always comparable, so an incomparable pair is never of two counters,
  and the fixture already said `kind`. The reason is dropped from §5's
  closed list and from the model's `NoRate`; `incomparable` stays as a
  continuity reading (§2.10). The fixture's description now says so.
- **F-147, an Epoch attachment that does not decode** (§2.6 check 2).
  0.1's fixtures gave attachments decoded, and `undecodable` named "its
  JSON or its CBOR", so zero bytes or `x` read as `epoch_invalid` or as an
  undecodable sample.
  - Now an Epoch attachment whose bytes do not decode is an epoch present
    and not a string: `epoch_invalid`, zero bytes included, and the sample
    is still read. `undecodable` names the payload alone. zk2py's reading
    stands.
  - The model's `Datum::attachment` reads one so; the runtime's writer,
    which refused such an attachment as undecodable, now refuses it
    `epoch_invalid`, as a reader reads it.
  - `scenarios.md` §7 step 3 now says "the JSON string `""` (two bytes)".
  - `samples.json`: four cases as `attachment_text`: zero bytes, `x`,
    `""`, and an instance id.
- **F-148, a state member's pairs** (§2.10). 0.1 paired "in the order the
  reader received them", and a late joiner receives a state member by a
  GET and a subscription: a reply of 10 read after a live 12 paired 12 →
  10, `decreased` and established, for a counter that never went down.
  - Now a state member's pairs are taken in the order a late joiner's
    merge keeps its samples (core §4.2), and a sample the merge does not
    keep is no sample of a pair. A stream's are still taken as received.
    zk2py's reading stands.
  - `scenarios.md` §9, a state counter seeded, runs it, and `meter.v1`
    gains `total`, a bare counter in state, for it.
- **F-149, a Point member written `null`** (§2.6 checks 1 and 2). Present:
  `null` is no string and no object, so `"epoch": null` is
  `epoch_invalid`, and a `null` `unit` or `labels` is `not_a_point`, as
  the standard schema also says. Core §5.2's "`null` is absent" is the
  envelope's own rule, and the text now says so. The model and zk2py
  already read it so. `samples.json`: one case per member.

**Also stated, from the round's "read and not filed".**
- **§2.1 against §0:** "a Point-typed resource … is a metric resource" is
  now "a stream or state resource typed by the Point", so an event typed
  by the Point is not one (`declarations.json` case 79, unchanged).
- **The pre-profile Point** (§2.4, §3): it is the Point, and a reader reads
  an in-band epoch in its `epoch` member, which its file does not define
  and every reader tolerates; only the fixture said so
  (`declarations.json` case 15, unchanged).

**Deliberately not changed.**
- **The schema** and its published ids: the answers are about reading
  bytes, never about the Point's shape.
- **`Datum::from_json` and `Datum::from_cbor`,** which read values already
  decoded, stay for a reader that holds one (the second now reads a
  bignum's tag as its integer too); a sample's bytes are read by the new
  decoders.
- **CBOR's non-finite floats** stay `not_finite`: CBOR carries them, and
  strict JSON's "every number finite" is JSON's rule.
- **`undecodable` stays unobservable,** never a finding: the core's
  verdict on the bytes stands (§2.6).
- **A stream's pairs** stay in receipt order: a stream has one path, and
  no merge.

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
  declaration refuses) and the scenarios as tests were #819's TB (0.2);
  the fleet's continuity reader, `check conform`'s and the doctor's
  `telemetry-*` checks were its TC (0.3), and zengui's units its TD.
