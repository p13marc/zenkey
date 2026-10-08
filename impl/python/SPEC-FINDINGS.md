# Findings against the zk2 spec (from the Python implementation, #609)

zk2py was written from `spec/` alone: `core.md`, the two JSON schemas,
`core/`, the fixtures and the scenarios, plus `examples/zk2/` as extra
inputs. It never read the Rust implementation or `docs/zk2/`. Each entry
below is a place where that was not enough, or where the spec said two
things.

**Two rounds.**
- **F-01 to F-39** were found against `core.md` 0.2.
- After amendments 0.3 (the classifier's rule set) and 0.4 (TOML 1.0
  enforced), each of those entries carries a status line where the
  amendment touches it.
- **F-40 to F-45** are new, found against 0.4.

**Severities.**
- **gap:** the prose is silent. The entry says whether a fixture's expected
  value decided it, or zk2py guessed.
- **ambiguity:** the text allows two readings. A fixture or a guess decided.
- **contradiction:** the prose, read literally, and a fixture disagree, or
  two parts of the spec do. Against a fixture, the fixture won, because §9
  says so: "Where it and a fixture disagree, the fixture is right and this
  text has a bug."
- **blocker:** zk2py could not implement the rule. None was found.

**Counts at 0.4:** 45 entries.
- **By severity:** 18 gap, 23 ambiguity, 4 contradiction, 0 blocker.
- **By status:**
  - 4 resolved: F-29, F-30, F-34, F-36;
  - 5 partly resolved: F-16, F-18, F-31, F-32, F-35;
  - 30 still open from the first round;
  - 6 new: F-40 to F-45.

Code comments cite these as `SPEC-FINDINGS F-nn`.

| Id | Severity | Status at 0.4 | Location | In one line |
|---|---|---|---|---|
| F-01 | ambiguity | open | §1.2 ULID | No first-character bound: is `8zzz…` (beyond 128 bits) a ULID chunk? |
| F-02 | ambiguity | open | §1.1, keys.json | Without a contract, is a plain chunk that is neither a literal nor a canonical slug (`x-eth0`) a valid resource chunk? |
| F-03 | ambiguity | open | §2.2 | Does a `uint` parameter only match canonical decimal chunks? |
| F-04 | gap | open | §3.3, descriptors/ | D000–D004, D008 and D010 are defined nowhere in the prose. |
| F-05 | gap | open | descriptors/ | The checker's cascades and scope appear only in fixtures. |
| F-06 | ambiguity | open | §3.3, R3 | Descriptor checks the prose implies but no fixture pins. |
| F-07 | contradiction | open | §3.3 example | The example holds `imu` yet says covariance is absent for "no IMU". |
| F-08 | ambiguity | open | §5.2 | Encoding strings and type errors in the envelope decoder. |
| F-09 | ambiguity | open | §7.3, E037 | Is a refused applicator's content a "schema position"? |
| F-10 | ambiguity | open | §9.4, E032 | What "a `$ref` with a scheme (`:`)" covers; fragments that are not pointers. |
| F-11 | gap | open | §9.4, §9.6 | How a reader of a bundle resolves a cross-file `$ref` (bundles carry no paths). |
| F-12 | gap | open | §9.4, E029 | Is a JSON Schema file with a duplicate member "not JSON"? |
| F-13 | ambiguity | open | §9.2 E024 | How many E024 for duplicate stems; a qualified reference to an ambiguous stem. |
| F-14 | gap | open | §9.4 | Which source files the well-known types compile from. |
| F-15 | ambiguity | open | §9.4 | Artifact names and roots, nested messages, enums as types, `json_name`. |
| F-16 | ambiguity | partly resolved by 0.4 | §9.1 | TOML 1.0's 64-bit integer limit: E000 or E028? (The 1.1 half is settled.) |
| F-17 | ambiguity | open | contract.schema.json | Integer bounds exist only as `format`, which JSON Schema treats as an annotation. |
| F-18 | ambiguity | partly resolved by 0.4 | §9.1, E020 | Which of TOML's four date/time types count as "a datetime". |
| F-19 | gap | open | §9.1, §9.5 | Floats: `nan`/`inf`; `1e16` passes the lints but fails bundle verification; `1.0` ≡ `1`. |
| F-20 | ambiguity | open | §9.2 E020 | One E020 per key or per condition; requirement annotations. |
| F-21 | gap | open | §10, App. D, W105 | W105 for a profile with no interim vocabulary. |
| F-22 | ambiguity | open | §9.2 E018, E033 | Written or resolved values (with `[defaults.operation]`). |
| F-23 | ambiguity | open | §9.2 | Small lint edges: retention leading zeros, `gate = []`, `history = false` on an event, W103 beside E023. |
| F-24 | gap | open | §9.6 step 9 | The base64 variant. |
| F-25 | gap | open | §9.6 steps 7, 9, 10 | Entry shapes the verification steps leave open. |
| F-26 | gap | open | §9.6 extras | No source for extra documents; the value's shape; whether `"extras": {}` is always written. |
| F-27 | ambiguity | open | §9.7 | History check: one problem or all per file, check order, odd entries. |
| F-28 | ambiguity | open | §9.7 retention | "Identical" is not a classifier class; identity is defined only for protobuf; the default `json_name` rule. |
| F-29 | ambiguity | **resolved by 0.3** | §9.8 | "In both directions": swapped old/new would contradict `explicit-true-to-false`. |
| F-30 | gap | **resolved by 0.3**, contradictorily (F-40) | §9.8 | How resources of two revisions are paired. |
| F-31 | gap | partly resolved by 0.3 | §9.8 table | The contract-metadata table is not exhaustive. |
| F-32 | gap | partly resolved by 0.3 | §9.8 JSON Schema | The JSON Schema rules are not exhaustive. |
| F-33 | ambiguity | open | §9.8 JSON Schema | `oneOf`/`anyOf` branch identity, and the asymmetry between them. |
| F-34 | contradiction | **resolved by 0.3** | §9.8, renumber-field | A renumber "is a deletion" without a reserved number, yet the fixture reports no warning. |
| F-35 | gap | partly resolved by 0.3 | §9.8 protobuf | The protobuf rules are not exhaustive. |
| F-36 | gap | **resolved by 0.3** | compat/, App. E | The compat input layout is described only by the files themselves. |
| F-37 | gap | open | conformance/README.md, fixture descriptions | Meanings are deferred to Rust symbols and design documents. |
| F-38 | gap | open | §9.2, sets/ | A set member that does not load; E035 against duplicate declarations. |
| F-39 | ambiguity | open | §2.2, §9.2 E021/E022 | "After the first" relies on document order, which TOML does not define. |
| F-40 | contradiction | **new** | §9.8 "Resources" | "Matched by kind token and template" contradicts `explicit_cleared` and `compat/contract/explicit-true-to-false`. |
| F-41 | ambiguity | **new** | §9.1 (0.4) | "A reader MAY accept later TOML", yet 1.1-only syntax "is E000"; is the list of constructs closed? |
| F-42 | contradiction | **new** | CHANGELOG 0.3, compat/expect.json | Stale statements: "70 cases, 11 of them new" (23 are); "until it exists, the runner checks that every input loads". |
| F-43 | ambiguity | **new** | §9.8 protobuf `presence_changed` | What "explicit presence" covers; overlap with `oneof_changed`. |
| F-44 | ambiguity | **new** | compat/README.md | Where the wrapping contract sits (it decides `same_revision`), and when a case is `invalid`. |
| F-45 | gap | **new** | §9.7, examples/zk2/.history | How a contract finds its history directory; §9.7 names `contracts/.history`, the examples use one root for many directories. |

---

## §1 Identity and grammar

### F-01 · ambiguity · §1.2, ULID chunks

> "A ULID chunk (events, §2.6) is 26 characters of Crockford's base32 in
> lowercase: digits, and lowercase letters except `i`, `l`, `o`, `u`."

A real ULID is 128 bits, so its first character is `0`–`7`. The spec does
not say whether `8zzzzzzzzzzzzzzzzzzzzzzzzz` is accepted. `keys.json`
has no such case.
**Resolved:** a guess. zk2py follows the literal text and accepts any of
the 26 characters (`lexical.ULID`).

### F-02 · ambiguity · §1.1 (position 6+), `keys.json`

> "6+ | resource chunks | Built from the resource's template (§2.2)."

`keys.json` parses keys without any contract, so a parser cannot know the
template. Is `zk2/s/svc/nav.v2/stream/x-eth0` a zk2 key?
- `x-eth0` is a plain chunk.
- It cannot be a literal (§2.2: a literal "does not start with `x-`").
- It is not a canonical slug (§1.4; `slugs.json` refuses it).

So no template could ever produce it.
**Resolved:** a guess. A resource chunk only has to be a plain chunk
(`keys._data`).

### F-03 · ambiguity · §2.2, `uint` parameters in resolution

> "A `uint` value is written in decimal without leading zeros, then slugged
> like any value."

> "A template matches a key's resource chunks when … every parameter chunk
> decodes (§1.4)…"

Resolution mentions only decoding. Does `{n}` typed `uint` match the chunk
`03` or `abc`? `templates.json` gives templates without parameter types.
**Resolved:** a guess. Types are ignored when matching (`templates.match`).

## §3.3 The descriptor

### F-04 · gap · §3.3 and `conformance/descriptors/`: most D codes have no definition

> "a checker MUST report exactly the `D…` codes that
> `conformance/descriptors/expect.json` lists for each document, checked
> against the fixture contract."

The prose names four codes:
- D005: "A listed resource that is not an optional resource of the contract is an error";
- D006: "One that a missing capability already implies is a warning";
- D007: `[F: descriptors/d007-*]` after the cardinality rule;
- D009: `[F: descriptors/d009-*]` after the `requires` rule.

D000, D001, D002, D003, D004, D008 and D010 are defined nowhere. There is
no table of D codes like §9.2's table of E and W codes. Nothing says which
D codes are warnings besides D006. `descriptors/expect.json` describes
itself as "the sorted D… codes `check()` reports": `check()` is a Rust
function.

**Resolved:** from the fixtures. Each meaning was reverse-engineered from
the file names (`d002-instance`, `d008-twice`, `d010-profile`, …) and the
documents' single differences. The derived table is in the docstring of
`zk2py/descriptor.py`.

This is where the urge to read the Rust checker was strongest.

### F-05 · gap · `descriptors/`: cascades and scope stated only by fixtures

Several rules about how the descriptor checks interact exist only as
expected values:
- **`d003-fingerprint`** expects `["D003"]` for `"contract": "sha256:FEA2"`. A
  malformed fingerprint is D003 *alone*: the revision check (D004) must not
  also run.
- **`d003-iface`** expects `["D003", "D009"]` for `"iface": "nav.2"`. The
  D009 comes from `declared_by: "nav.v2"` no longer naming an interface the
  descriptor lists. The prose says only "A role declared in a contract
  names that contract's interface in `declared_by`".
- **"A document whose interface names another contract is checked for
  syntax only"** appears only in `descriptors/expect.json`'s description.
  - With it, `ok-unknown-revision` (interface `geo.v1`) is clean.
  - `d004-revision` (`nav.v2` with another fingerprint) is D004.

  The case names suggest the reverse. "Unknown revision" is in fact an
  unknown *interface*, and D004 is the unknown revision.

**Resolved:** from the fixtures.

### F-06 · ambiguity · §3.3 and R3: checks the prose implies but no fixture pins

These are left unimplemented, except the two duplicate rules at the end,
which are guesses:
- **R3:** "the descriptor MUST list every requirement". Is a role that the
  checked contract declares in `[requires]`, but that is missing from the
  descriptor's `requires`, a D009?
- **`declared_by: "nav.v2"`:** must the role exist in that contract's
  `[requires]` with the same `interface`?
- **`cause`:** must it agree with the resource's gate kind? For example,
  `cause: "capability"` on a resource gated only by `config:`.
- **A lowered bound of `0`:** is it allowed? The schema says `minimum: 0`;
  §2.2 says a contract's cardinality is "a positive integer".
- **`params` values:** are they checked against the required interface's
  templates?
- **`minor`:** the schema has it as `uint64`, while the contract's is
  `uint32`.
- **Duplicates (guesses):** zk2py reports a capability listed twice as
  D008, which the fixture `d008-twice` pins, and a profile listed twice as
  D010, which nothing pins.

### F-07 · contradiction (informative text) · §3.3, the descriptor example

The example has `"capabilities": ["imu", "gnss"]`. It also has
`"unavailable": [{"resource": "state/covariance", "cause": "capability",
"reason": "no IMU"}]`.

The instance holds `imu`, yet it gives "no IMU" as the capability reason
for covariance's absence. If it lacked the IMU, `imu` would not be listed,
and the entry would then be D006 ("a missing capability already implies"
it). The example is informative, but a reader checking it against the
rules finds it inconsistent.
**Resolved:** nothing to implement. Reported here.

## §5.2 The error envelope

### F-08 · ambiguity · §5.2, encodings and type errors

> "The reply's `Encoding` MUST say which: `application/json` or
> `application/cbor`; or `application/protobuf` with the schema suffix
> `zk2.core.v1.Error`."

> "It MUST refuse: an unknown encoding (`encoding`); malformed bytes, a
> duplicate or unknown member, or a missing `code` or `message`
> (`decode`); …"

The spec leaves these open:
- **Variant encodings:** `application/json;charset=utf-8`, plain
  `application/protobuf`, or `application/protobuf;other.Error`. zk2py
  refuses them all with `encoding`, because §7.2 forbids schema suffixes
  except on the envelope.
- **Wrong JSON/CBOR member types:** for `"code": 3` or `"cause": 3`, the
  tag could be `decode`, `code` or `cause`. zk2py uses `decode`, as a shape
  error.
- **A known protobuf field with the wrong wire type:** zk2py uses `decode`.
- **CBOR features:** tags (zk2py decodes the content), `undefined` (read as
  null), indefinite lengths (accepted).
- **CBOR byte strings inside a `detail`:** how to show them. zk2py uses
  `{"bytes_hex": …}`, as the fixtures show a protobuf detail.
- **A protobuf `cause` present but empty:** zk2py refuses it with `cause`.

**Resolved:** guesses, as listed.

## §7.3 and §9.4 JSON Schema artifacts

### F-09 · ambiguity · §7.3, E037 inside refused keywords

> "Refused: every other keyword in a schema position (`pattern`,
> `patternProperties`, `allOf`, `not`, `if`/`then`/`else`, …)"

In JSON Schema 2020-12, the values of `allOf`, `not`, `if`,
`patternProperties` and the other applicators are themselves schema
positions. So in `{"allOf": [{"pattern": "x"}]}`, does `pattern` count as
an E037 too? `e037-subset` cannot tell, because its nested keywords are all
allowed ones.

A related case: `definitions` (draft-07) is a refused keyword, but is its
content walked?
**Resolved:** a guess. zk2py walks every 2020-12 applicator position,
refused ones included, and reports each keyword once per file. It does not
walk `definitions`.

### F-10 · ambiguity · §9.4, E032 and "a scheme"

> "A `$ref` with a scheme (`:`) is refused. Its pointer MUST resolve."

Open points:
- Is any colon refused, so that `#/$defs/a:b` fails? Or only a URI scheme
  before the fragment?
- Are non-pointer fragments (`#anchor`) refused? zk2py refuses them; the
  subset has no `$anchor` keyword anyway.
- Is the pointer percent-decoded first? The fragment of a URI normally is.

**Resolved:** a guess. A colon in the file part (before `#`) is a scheme.
Pointer fragments only, percent-decoded (`schemas.SchemaSet.resolve_ref`).

### F-11 · gap · §9.4 and §9.6, cross-file `$ref` once bundled

**Status at 0.4: open.** 0.3 adds "`$ref`s are followed, across the revision's artifacts" to §9.8, which confirms that a bundle's `$ref`s must be followed, but still not how a path maps to an artifact that has only a stem.

> "A `$ref`'s file part resolves as a path relative to the referencing
> file, lexically normalized, and MUST name a listed file."

A bundle carries each JSON Schema artifact with only its `name` (the stem).
It keeps no path. Two parts of the spec need those `$ref`s resolved from a
bundle:
- §7.2: "generic tools MUST decode";
- §9.8: the classifier reads earlier revisions from the history.

Neither can resolve `../common/types.json#/$defs/X` by path. Only
`examples/zk2/README.md`, which is not normative, says "Cross-file `$ref`
in a bundle resolves by the referenced file's stem".
**Resolved:** a guess. zk2py takes the stem of the file part's last path
component (`compat.JsonWorld`). Stems are unique per contract (E024).

### F-12 · gap · §9.4 and E029, duplicate members in a JSON Schema file

> E029: "a schema file missing, unreadable, not JSON, or not compiling"

The artifact id is "the document's JCS bytes", which a document with
duplicate members does not have. The spec does not say whether such a
document is "not JSON". (§9.6 says it for bundles: "JSON with no duplicate
member".)
**Resolved:** a guess. A duplicate member is E029.

### F-13 · ambiguity · §9.2 E024, counting

> "E024 | a `json:` name defined by several listed files, or two listed
> JSON Schema files with one stem | per reference or file"

For two files sharing a stem, is that one E024 or two? And how does a
qualified reference `json:stem#Name` resolve when the stem is ambiguous?
**Resolved:** a guess. One E024 per file whose stem an earlier file
already took. A qualified reference looks only in the first file with that
stem.

## §9.4 Protobuf artifacts

### F-14 · gap · §9.4, the well-known types' source files

> "The well-known types are always available, each compiled on demand as
> its own artifact named `google/protobuf/<file>.proto`"

The artifact is the compiled bytes, so its id depends on the *text* of
`empty.proto` and the other files. That text has changed across protobuf
releases (`go_package`, `csharp_namespace`, `objc_class_prefix`, …). The
spec pins the compiler, protoc 3.21.12 in §9.5, but not the source of these
files.

`ok-protobuf-well-known` passes with the files protoc 3.21.12 ships, in its
release archive's `include/` and in Debian's `/usr/include/google/protobuf`.
The two agree byte for byte.

A related point: a user file that imports `google/protobuf/timestamp.proto`
relies on the compiler's built-in include path. The spec does not say so.
**Resolved:** from the fixture. zk2py uses the compiler's own include
directory, with protoc pinned (`protoc.compile_well_known`).

### F-15 · ambiguity · §9.4, names, roots, nested messages, enums, `json_name`

> "Import roots are `proto_include`, relative to the contract. … Its
> `name` is the file's path relative to its import root."

Open points:
- **Listed paths and roots.** Listed paths are relative to the contract's
  directory. A file under no import root has no name; a file under several
  roots has several. zk2py takes the first root, and reports E029 when
  there is none.
- **Nested messages.** "A message reference resolves to the one listed
  file that defines it." Is `pkg.Outer.Inner` such a message? zk2py says
  yes.
- **Enums.** A reference to an enum is E023 in zk2py.
- **Shadowing.** A listed file could define `google.protobuf.Empty` itself.
  zk2py resolves to the listed file first.
- **`json_name`.** The spec does not say that the artifact includes a
  `json_name` for *every* field, as `protoc --descriptor_set_out` writes it.
  It matters for byte portability, and for §9.7's "every default
  `json_name` dropped".

**Resolved:** the fixtures decide `json_name` (protoc's output matches
every id). The rest are guesses.

## §9.1 TOML and the authoring shape

### F-16 · ambiguity · §9.1, the TOML integer range

**Status at 0.4: partly resolved by 0.4.** TOML 1.1-only syntax is now E000 (§9.1, five `e000-toml11-*` fixtures), which settles the 1.1 half. The 64-bit integer limit inside TOML 1.0 is still unaddressed.

TOML 1.0 says: "If an integer cannot be represented losslessly [in 64
bits], an error must be thrown." Python's `tomllib` accepts
`9223372036854775808` regardless.

A reader that keeps the value goes on to E028 ("an integer outside
±(2^53−1) in the canonical form"). A reader that follows TOML reports E000
("text that is not TOML"). §9.1 already acknowledges the same kind of
divergence for TOML 1.1 ("a 1.1-only contract can load in Rust and fail in
a 1.0 reader"). The 64-bit limit is a second case, inside TOML 1.0.
**Resolved:** a guess. zk2py refuses integers beyond 64 bits as E000.

### F-17 · ambiguity · `contract.schema.json` and `descriptor.schema.json`, integer bounds

`major`, `minor`, `deprecated.since` and `history.depth` are
`{"type": "integer", "format": "uint32", "minimum": 0}`. JSON Schema
2020-12 treats `format` as an annotation, so a standard validator accepts
`major = 4294967296`. §9.1 bounds `major` to "0 to 2^32−1".
**Resolved:** a guess. zk2py's shape checker asserts `uint32` and `uint64`,
giving E000 and D000. A `maximum` in the schema would remove the doubt.

### F-18 · ambiguity · §9.1 and E020, "a datetime"

**Status at 0.4: partly resolved by 0.4.** The 0.4 changelog says a time without seconds "E020 would have refused … anyway, as an annotation datetime", so a local time counts. That is said in the changelog, not in §9.1, and nothing names a local date.

> "Annotation values are any TOML value except a datetime (E020)."

TOML has four temporal types: offset date-time, local date-time, local date
and local time. `e020-datetime` uses an offset date-time.
**Resolved:** a guess. All four count, at any depth of the value.

### F-19 · gap · §9.1 and §9.5, floats and the canonical restrictions

> §9.1: "Floats are allowed."

> §9.5: "every integer is within ±(2^53−1). … On that domain every JCS
> implementation agrees."

Three consequences go unaddressed:
1. **`nan` and `inf` are valid TOML.** No lint refuses them, and RFC 8785
   cannot serialize them, so the contract has no canonical bytes.
2. **`1e16` is a float**, so the lints never see "an integer". But JCS
   writes it as the integer literal `10000000000000000`. A verifier parses
   it back as an integer beyond 2^53−1 and refuses the bundle at §9.6 step
   6 (`restrictions`). A contract that lints clean can thus build a bundle
   that no implementation verifies.
3. **`1.0` and `1` have the same canonical bytes.** Changing an annotation's
   type keeps the fingerprint.

**Resolved:** a guess. An integral float beyond ±(2^53−1) is counted as
E028; zk2py stops at 1e21, where ECMAScript switches to exponent form.
`nan` and `inf` are reported as E028 too (`jcs.restriction_violations`,
`contract.canonicalize`).

## §9.2 The lints

### F-20 · ambiguity · E020, counting

> "per key and table; once for `[defaults]`, whatever it reaches"

When a key is malformed *and* its value holds a datetime, is that one E020
or two? Do `[requires.<role>]` annotations count as one of these tables?
§9.1 lists `annotations` among a requirement's fields, but §9.2's E020
does not name them.
**Resolved:** a guess. One E020 per key, and requirement annotations are
checked like any other table.

### F-21 · gap · §10 point 2, Appendix D, W105 without a vocabulary

> "Until a profile publishes its vocabulary, the interim tables of
> Appendix D apply, and a key outside them is a warning."

A contract may `use` a profile that has no interim table, such as
`acme.v1`. Is every key of that profile then W105, or none?
**Resolved:** a guess. None.

### F-22 · ambiguity · E018 and E033, written or resolved

> "E018 | `serving = "replicated"` without `idempotent = true`"

> "E033 | `summary` without `replies = "many"`"

`serving`, `idempotent` and `replies` are defaultable (Appendix D). With
`[defaults.operation] idempotent = true`, is a resource that sets only
`serving = "replicated"` an E018?
**Resolved:** a guess. zk2py judges the resolved values (§9.3).

### F-23 · ambiguity · §9.2, small lint edges

None of these is covered by a fixture:
- **Leading zeros in `retention`.** E026: "`burst(<n>/h)` (n ≥ 1, decimal,
  no leading zero); `retention` not `<n>` + `s`/`m`/`h`/`d`/`w` (n ≥ 1)".
  `07d` is accepted, because the canonical form stores seconds anyway.
- **`gate = []`.** zk2py treats the field as present, so it is E016 without
  `optional = true`.
- **`history = false` on an event.** zk2py reports E019, because the field
  is written.
- **W103 beside E023.** "written on a resource where no JSON Schema type
  takes it" is unknowable when the type does not resolve. zk2py skips W103
  there.

**Resolved:** guesses, as listed.

## §9.6 Bundles

### F-24 · gap · §9.6 step 9, the base64 variant

> "it has a `data` member, base64 text for protobuf"

The prose does not say which alphabet, whether padding is required, or
whether decoding is strict.
**Resolved:** from the fixtures. zk2py's builder reproduces
`valid.bundle.json` byte for byte with RFC 4648 §4 standard, padded base64.
The verifier is strict, so unpadded or URL-safe text is `shape`.

### F-25 · gap · §9.6 steps 7, 9 and 10, entry shapes

The verification steps leave several shapes open:
- **A schema entry that is not an object.** zk2py: `shape`.
- **A schema entry with no `kind`.** `schema_kind` or `shape`? zk2py:
  `schema_kind`.
- **Unknown members** inside a schema entry or an extra entry, such as a
  stray `"sig"`. Step 3 refuses unknown top-level members, but nothing says
  what happens at this level. zk2py ignores them.
- **An extra's `media_type`.** Shown in the example, never checked. zk2py
  ignores it.
- **The same id listed twice** in `contract.schemas`, for two listed files
  with identical bytes. Not addressed.
- **A JSON Schema `data` with no JCS bytes** (an integer beyond 2^53−1).
  zk2py: `schema_hash`.

**Resolved:** guesses, as listed.

### F-26 · gap · §9.6, extras

> "Extras are exactly the documents that the contract's `views.document`
> annotations reference. … The reference builder does not carry extras
> yet"

Open points:
- **No source.** The spec never says where a builder finds the document for
  an id: not a path, a directory, or a convention.
- **The value's shape.** Is a `views.document` value one `sha256:…` or a
  list? Appendix D lists only the key.
- **Which annotations.** Step 11 says "of the contract's resources", which
  leaves out requirement annotations.
- **An empty `extras`.** A built bundle always writes `"extras": {}`. That
  changes the bytes, and §9.6 makes one revision one bundle. Only
  `valid.bundle.json` shows it.

**Resolved:** the builder refuses a contract that uses `views.document`.
The verifier accepts a string or a list of strings. `"extras": {}` is
written, from the fixture.

## §9.7 History and retention

### F-27 · ambiguity · §9.7, the history check

> "The history check MUST verify: every bundle (§9.6); its fingerprint
> against its file name; its interface against its directory; that it is
> in JCS form. It reports `directory` …, `file_name`, `io`, `interface`,
> `jcs`, or a bundle tag."

Open points:
- **One problem per file, or every problem?** For example, a wrong
  directory *and* not JCS.
- **In which order** are the four checks made?
- **A plain file** at the root of the history.
- **A subdirectory** inside an interface directory.
- **"In directory then file order"** (`history/expect.json`): by bytes, or
  by locale?
- **Is a bad directory's content checked?** `bad-directory` shows it is not.

Each fixture has exactly one problem, so the order is untested.
**Resolved:** a guess, consistent with every fixture:
- the first problem per file, in this order: verify (with the fingerprint
  the file name implies, so `wrong-name` gives the bundle tag
  `fingerprint`), then interface, then JCS;
- entries sorted by name;
- a non-interface entry at the root is `directory`, and its content is
  skipped.

### F-28 · ambiguity · §9.7, retention identity

> "A rebuild that the classifier judges identical to the newest published
> revision keeps that revision's bundle … For protobuf, identity compares
> the `FileDescriptorSet`s with source info dropped and every default
> `json_name` dropped."

Open points:
- **"Identical" is not one of the classifier's classes.** Those are
  compatible, review and breaking.
- **Identity for JSON Schema and raw types** is not defined. Canonical-byte
  equality is the obvious reading.
- **The default `json_name`** is protoc's `ToJsonName` (underscores
  removed, the next letter upper-cased), which the spec does not state.
- **What is compared:** messages or bytes.

**Resolved:** the fixtures decide `same_revision` for the protobuf payload
cases:
- the normalized sets are compared as messages;
- the default `json_name` follows protoc's rule.

Identity for JSON Schema and raw types is not exercised.

## §9.8 Compatibility

### F-29 · ambiguity (fixture decided) · §9.8, "in both directions"

**Status at 0.4: resolved by 0.3.** The tables now list both transitions of a member with different classes (`explicit_set` breaking against `explicit_cleared` review; `idempotent_cleared` against `idempotent_set`; `fanout`, `replies` and `reliability` each way). A rule is therefore a directed transition, and "both directions" can only mean the reader and writer roles. The phrase itself is unchanged.

> "a candidate against every revision in the history, in both directions.
> Each rule is judged per direction: for streams, state, events and
> responses, the owner writes and the consumer reads; for requests, the
> caller writes and the owner reads."

One reading compares old→new *and* new→old. Under it, every directional
row of the table becomes symmetric. `explicit` true → false (review) would
also be false → true (breaking), so breaking overall, but
`compat/contract/explicit-true-to-false` says review. The same reading
would raise a spurious `field_deleted_unreserved` for every added protobuf
field.

**Resolved:** from the fixture. "Direction" means the reader and writer
roles, and every rule's class already accounts for both. zk2py classifies
only earlier → candidate.

### F-30 · gap (fixture decided) · §9.8, pairing resources across revisions

**Status at 0.4: resolved by 0.3, but contradictorily.** §9.8 now says resources are "matched by kind token and template", which contradicts its own explicit rows and the fixture: see F-40.

The spec never says how two revisions' resources are paired.
- **By `(kind token, template)`:** `explicit` true → false becomes a
  removal plus an addition, so breaking.
- **By template text:** it is the review the fixture expects.

**Resolved:** from the fixture. zk2py pairs by template, which is unique
within a contract because it is the table key.

### F-31 · gap · §9.8, the contract table is not exhaustive

**Status at 0.4: partly resolved by 0.3.** The six tables now class every canonical member, in both directions. zk2py follows them, which changed twelve of its guesses (the list is in the README and `zk2py/compat.py`'s docstring). One remainder: a listed artifact that no type references can change with no rule firing (zk2py: compatible, because nothing reads it).

The table gives no class for any of these changes:
- **Resources:**
  - a resource *added*;
  - a resource's `kind` changed;
  - its `params`, `cardinality`, `epoch`, `gate`, `history`, `rate`,
    `retention_s`, `timeout_ms`, `serving`, `encoding`,
    `attachment_encoding`, `annotations` or `deprecated` changed.
- **The reverse transitions:**
  - `idempotent` false → true;
  - `fanout` forbidden → allowed;
  - best_effort → reliable;
  - block → drop;
  - many → one;
  - required → optional.
- **Roles:**
  - a role *removed*;
  - optional → required;
  - its `resources` or `annotations` changed.
- **Uses:** `uses` changed.
- **Types:**
  - a type changing kind (raw → jsonschema);
  - a raw media type changed;
  - an `error` or `attachment` type added or removed.

**Resolved:** guesses. Every unlisted change is **review**, with these
exceptions:
- a `kind` change, a type changing kind, and a raw media-type change are
  breaking;
- an optional resource added is compatible, by analogy with "An optional
  role added";
- a role going optional → required is breaking, by analogy with "A
  required role added".

### F-32 · gap (partly fixture decided) · §9.8, the JSON Schema rules are not exhaustive

**Status at 0.4: partly resolved by 0.3.** §9.8 now states `type_changed` for any type set change, `required_removed`, `const_changed`, the `additionalProperties`/`items` toggles (compatible) and schema gains (review), and enum reordering (compatible). Still unlisted: a `$ref` beside other keywords, a boolean schema in a property position, and a `required` name with no property.

The rules list "integer ↔ number" as the only type change.
`compat/transitive/jsonschema-property-retyped` needs `string → integer` to
be breaking (v3 against v1), and only that fixture says so.

There is also no rule for:
- a *required* property removed;
- `const` changed;
- `items` changed;
- `additionalProperties` toggled between `true`/absent and `false`, or
  between a boolean and a map schema;
- a `required` name with no matching property;
- a `$ref` beside other keywords.

**Resolved:** the fixture decides that any `type` change is breaking. The
rest are guesses:
- a required property removed, `const` changed and `items` changed are
  breaking;
- `additionalProperties` boolean toggles are compatible ("Readers tolerate
  unknown properties");
- boolean ↔ schema and a `$ref` beside other keywords are review.

### F-33 · ambiguity · §9.8, `oneOf` versus `anyOf`

> "Breaking: … a `oneOf` branch added." "Review: any other change inside
> `oneOf`, `anyOf` or `prefixItems`."

Two questions follow:
- **Branch identity.** Is it textual? Does order or an annotation count?
- **The asymmetry.** An `anyOf` branch added and a `oneOf` branch
  *removed* are only review. That is intended by the text, but stated
  nowhere as intended.

**Resolved:** a guess. Branches compare as a multiset of normalized schemas,
with annotations dropped and `$ref`s inlined. "Added" means the old
multiset is a strict subset of the new one. `prefixItems` compares in
order.

### F-34 · contradiction · §9.8 against `compat/payload/protobuf/renumber-field`

**Status at 0.4: resolved by 0.3.** §9.8 now says "Fields are matched by number. A field missing by number but present by name is renumbered": the same field is the same name, and a renumbered field is not a missing one, so it raises no warning. The bullet still calls a renumber "a deletion plus an addition", which is what made the old reading possible.

> "it is renumbered: a deletion plus an addition of the same field"

> "Warning `field_deleted_unreserved` (reported alongside the class): a
> field deleted without reserving its number."

`renumber-field` moves `string frame` from 3 to 10 without reserving 3. By
the prose's own definition that includes "a deletion" of a field without
reserving its number. Yet `expect.json` gives `"warnings": []`.

The prose also does not say what "the same field" means: the same name,
the same name and type, or something else.
**Resolved:** from the fixture. A renumbered field (same name, new number)
is breaking, and its deletion raises no warning.

### F-35 · gap · §9.8, the protobuf rules are not exhaustive

**Status at 0.4: partly resolved by 0.3.** §9.8 now states presence (`presence_changed`), map cardinality, `reserved_reused`, proto2 `required` added/removed/toggled, and that "nested and referenced" messages are compared "each pair once". Still open: proto2 defaults, options such as `packed`, an enum's openness when the two revisions' syntaxes differ, a field moved *between* oneofs, nested types added or removed, reserved *names* reused, and how warnings accumulate over a history.

None of these is covered:
- a proto3 `optional` toggled, which is wire-compatible but adds a
  synthetic oneof;
- a field moved *between* two oneofs;
- a proto2 `default` changed;
- options other than `json_name` (`packed`, `deprecated`);
- a message-typed field whose type is renamed but keeps the same
  structure;
- which types are compared: the root only, or every type reachable from
  it;
- an enum value renumbered;
- a proto3 message that uses a proto2 enum, which is open or closed;
- how `field_deleted_unreserved` accumulates over a history: once per
  earlier revision, or deduplicated.

**Resolved:** guesses:
- types are compared structurally from the root, through fields;
- a proto3 `optional` toggle is review;
- a move between oneofs is breaking (by name);
- a default change is review;
- options other than `json_name` are ignored;
- an enum is closed if either revision's file is proto2;
- warnings are listed once per deleted field per comparison.

### F-36 · gap · `compat/` and Appendix E, the input layout

**Status at 0.4: resolved by 0.3.** `compat/README.md` now gives the case layout, the one-resource wrapper and the meaning of each `expect.json` member (but see F-44 on the wrapper's location).

Appendix E says only "`compat/`: the class, warnings and `same_revision`".
The rest is learned from the files:
- old/new directories against `v1…vN` with `history` and `candidate`;
- payload cases are bare schema files or `.proto` directories with no
  contract;
- the protobuf file is always `m.proto`, and its import root is the
  revision directory;
- `type` names the type to compare;
- an invalid revision is detected by loading the file alone.

**Resolved:** from the fixture files.

## Fixtures and documents

### F-37 · gap · `conformance/README.md` and fixture descriptions: meanings deferred to Rust and design documents

The spec claims to stand without the reference implementation. Its fixture
documents point at it, and at design documents, for meanings:

| Pointer | Where | What it names |
|---|---|---|
| `zenkey_model::diag::CODES` | conformance/README.md, contracts/expect.json | the meaning of codes |
| `zenkey_model::bundle::BundleError::tag` | bundles/expect.json | refusal tags |
| `check()` | descriptors/expect.json | the descriptor checker |
| `check_set` | sets/expect.json | the set checks |
| `r3.3 D1` | templates.json, conformance/README.md | design documents |
| `r3 §3.11` | conformance/README.md | design documents |
| `docs/zk2/spike-results/s7/` | compat/expect.json | spike results |

For E/W codes and bundle tags, §9.2 and §9.6 were enough. For the D codes
they were not (F-04).
**Resolved:** nothing to implement. Reported because each pointer is an
invitation to cross the information barrier.

### F-38 · gap · §9.2 and `sets/`, set loading

> "E035 and E036 are set checks: each file is loaded on its own first."

Only `sets/expect.json` adds "and must load". The spec does not say what a
set check reports when a member does not load. Nor does it say which
declaration E035 checks a requirement against when E036 reports two
contracts declaring the same interface.
**Resolved:** a guess. A member that does not load refuses the whole set
check. E035 uses the first declaration in file-name order.

### F-39 · ambiguity · §9.2 E021 and E022, "after the first"

> E021: "once per template after the first of its shape"; E022: "once per
> `epoch` template after the first"

TOML 1.0 does not define an order for a table's keys, and resources are
table keys. The *codes* are unaffected, since they are counts. Which
resource carries the error is affected, and so is W101's "resources without
errors" (cascade 4). A different "first" can change which pairs W101
considers.
**Resolved:** a guess. Document order, as Python's `tomllib` and most
readers preserve it.

## New at 0.4 (after amendments 0.3 and 0.4)

### F-40 · contradiction · §9.8 "Resources" against `compat/contract/explicit-true-to-false`

> "**Resources,** matched by kind token and template:" … "`explicit` false
> → true | breaking for ambient consumers | `explicit_set`" … "`explicit`
> true → false | review (link budgets) | `explicit_cleared`"

Toggling `explicit` changes a resource's kind token (`stream` ↔ `@stream`,
`state` ↔ `@state`, §1.3). Paired "by kind token and template", the old
and new resources never pair, and `explicit_cleared` can never fire:
- `explicit` true → false would be `resource_removed`, which is breaking;
- `explicit-true-to-false` expects review.

The table also names `token_changed` beside `kind_changed`. Under that
pairing a token cannot change between paired resources either.

**Resolved:** from the fixture. zk2py pairs resources by template, which is
unique within a contract because it is the table key, and compares tokens
inside the pair. This replaces F-30, whose silence 0.3 filled with this
sentence.

### F-41 · ambiguity · §9.1 (0.4): "MAY accept later TOML" against "is E000"

> "A reader MAY accept later TOML, but a contract MUST NOT need it, so
> syntax that only TOML 1.1 has is **E000**:" (five constructs follow)

> CHANGELOG 0.4: "A reader MAY still accept later TOML. The rule binds what
> a contract needs, not what a reader parses."

Three things are unclear:
- **"Accept".** A 1.1 reader may parse these constructs, yet must still
  report E000 for them. So "accept" can only mean "parse, then refuse",
  which is not what the word usually says.
- **Is the list closed?** It reads as the whole TOML 1.1 delta, but §9.1
  does not say so. Is it the rule ("syntax that only TOML 1.1 has"), or
  are these five constructs the rule?
- **Later versions.** A future TOML 1.2 construct is not covered by
  either reading.

zk2py's E000 for these constructs is Python's `tomllib` refusing them,
because it is a strict 1.0 reader. A later `tomllib` that reads 1.1 would
silently stop reporting them. zk2py therefore probes its reader with the
five constructs at load, and refuses to run if any parses
(`contract._require_toml_1_0_reader`).

**Resolved:** the five fixtures pass. The guard is zk2py's own choice.

### F-42 · contradiction (documentary) · CHANGELOG 0.3 and `compat/expect.json`'s description

Two statements are stale:
- **CHANGELOG 0.3:** "`compat/` is evaluated. 70 cases, 11 of them new."
  Against 0.2's 47 cases, 23 are new:
  - 12 contract cases;
  - 5 JSON Schema cases;
  - 6 protobuf cases.
- **`compat/expect.json`'s `description`** still says "Evaluated by the
  classifier (#618); until it exists, the runner checks that every input
  loads." 0.3 removed exactly that caveat from §0 ("§0's caveat on
  `[F: compat/]` is removed").

**Resolved:** nothing to implement. Reported here.

### F-43 · ambiguity · §9.8 protobuf, `presence_changed`

> "explicit presence toggled, such as proto3 `optional`
> (`presence_changed`): a reader stops telling a default from an absent
> value"

The spec does not define which fields have explicit presence. The usual
protobuf definition gives presence to:
- every proto2 singular field;
- proto3 `optional` fields;
- message-typed fields;
- oneof members.

Under that definition, two more changes toggle presence besides the
`presence-toggled` case:
- moving a proto3 scalar into a oneof, which is already `oneof_changed`
  (breaking). Does it also raise `presence_changed`?
- changing a file's `syntax` between proto2 and proto3.

The class is unaffected, since the worst wins. Only the reasons a
classifier reports differ.

**Resolved:** a guess. zk2py uses the definition above
(`compat._presence`), so `move-into-oneof` reports both rules.

### F-44 · ambiguity · `compat/README.md`: the wrapper's location, and `invalid`

> "`payload/protobuf/<case>/` | `old/m.proto`, `new/m.proto` | Each wrapped
> in a one-resource contract (below) … with `<kind>` `protobuf` or
> `jsonschema` and `<artifact>` the case's file"

Two points are open:
- **Where the wrapping contract sits.** It is not stated, and it decides
  the protobuf artifact's import root, and with it the
  `FileDescriptorProto.name`.
  - A wrapper at the case root, listing `old/m.proto` and `new/m.proto`,
    gives two different file names. `doc-only`'s `same_revision` is then
    `false`, against `true` expected.
  - Only a wrapper inside each revision's directory, listing `m.proto`,
    reproduces the fixture.
- **When a case is `invalid`.** "`invalid` when the new revision must not
  load (its only error is E037)" leaves open what happens when the old
  revision, or a history revision of a transitive case, does not load, and
  when the new one fails with another error.

**Resolved:** the fixture decides the location. zk2py places the wrapper in
the revision's directory for protobuf, and in the case directory for JSON
Schema. For `invalid`, the guess is that any revision of the case failing
to load makes it `invalid`.

### F-45 · gap · §9.7 and `examples/zk2/.history`: where a contract's history lives

> "A contract's CI keeps every published bundle at
> `contracts/.history/<iface>/<hex>.bundle.json`"

`examples/zk2/` keeps one `.history` root for contracts in four
subdirectories (`walkthrough/`, `tcgui/`, `zenoh-modem/`, `zensight/`). The
spec does not say how a tool finds the history of a given contract file:
- the nearest ancestor `.history`;
- a configured root;
- a sibling of `contracts/`.

The requirement that every example be published there, and be compatible
with its history, is not written in `spec/` or in `examples/zk2/README.md`.
It came to zk2py from the coordinator of #609.

**Resolved:** a guess. zk2py's examples check uses `examples/zk2/.history`
for every example. All 24 are:
- published there;
- byte-identical to the bundle zk2py builds, protobuf ones included;
- `compatible` against their history.
