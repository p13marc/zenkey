# Findings against the zk2 spec (from the Python implementation, #609)

zk2py was written from `spec/` alone: `core.md`, the two JSON schemas,
`core/`, the fixtures and the scenarios, plus `examples/zk2/` as extra
inputs. It never read the Rust implementation or `docs/zk2/`, and it runs
the Rust owner example only as a black box. Each entry below is a place
where that was not enough, or where the spec said two things.

**Six rounds.**
- F-01 to F-39 were found against `core.md` 0.2.
- F-40 to F-45 were found against 0.4.
- F-46 to F-55 come from the live half's first slice.
- F-56 to F-63 were found against 0.5.
- F-64 to F-70 were found against 0.6, with the rest of the live half.
- Amendments 0.5, 0.6 and 0.7 resolved F-01 to F-70. Each entry carries a
  status line naming its amendment.
- **F-71 to F-73 are new**, found against 0.7.

**Severities.**
- **gap:** the prose is silent. The entry says whether a fixture's expected
  value decided it, or zk2py guessed.
- **ambiguity:** the text allows two readings. A fixture or a guess decided.
- **contradiction:** the prose, read literally, and a fixture disagree, or
  two parts of the spec do.
- **blocker:** zk2py could not implement the rule. None was found.

**Counts at 0.7:** 73 entries.
- F-01 to F-55: resolved by 0.5.
- F-56 to F-63: resolved by 0.6.
- F-64 to F-70: resolved by 0.7; none left unresolved.
  - 0.7 confirmed six of zk2py's guesses: F-64, F-65, F-66, F-67, F-68 and
    F-69.
  - It overturned one, F-70 (exposed). F-67's place for the put was refined
    too.
  - The Rust owner example has not caught up with 0.7 on F-65 and F-68, nor
    on F-69's setup. The runner reports those as known deviations, not
    failures.
- F-71 to F-73: **new**, 2 ambiguity, 1 gap.

Code comments cite open entries as `SPEC-FINDINGS F-nn`, and resolved ones
by the spec section that now states the rule.

| Id | Severity | Status at 0.5 | Location | In one line |
|---|---|---|---|---|
| F-01 | ambiguity | resolved by 0.5 | §1.2 ULID | No first-character bound. |
| F-02 | ambiguity | resolved by 0.5 | §1.1 | Is `x-eth0` a valid resource chunk without a contract? |
| F-03 | ambiguity | resolved by 0.5 | §2.2 | Do `uint` parameters only match canonical decimal? |
| F-04 | gap | resolved by 0.5 | §3.3 | D000–D004, D008, D010 undefined in prose. |
| F-05 | gap | resolved by 0.5 | descriptors/ | Cascades and scope only in fixtures. |
| F-06 | ambiguity | resolved by 0.5 | §3.3, R3 | Implied descriptor checks no fixture pinned. |
| F-07 | contradiction | resolved by 0.5 | §3.3 example | Holds `imu`, yet "no IMU". |
| F-08 | ambiguity | resolved by 0.5 | §5.2 | Envelope encodings and type errors. |
| F-09 | ambiguity | resolved by 0.5 | §7.3, E037 | Refused keywords' content. |
| F-10 | ambiguity | resolved by 0.5 | §9.4, E032 | `$ref` scheme and fragments. |
| F-11 | gap | resolved by 0.5 | §9.4, §9.6 | Cross-file `$ref` in a bundle. |
| F-12 | gap | resolved by 0.5 | §9.4, E029 | Duplicate member in a schema file. |
| F-13 | ambiguity | resolved by 0.5 | E024 | Counting duplicate stems. |
| F-14 | gap | resolved by 0.5 | §9.4 | Well-known type sources. |
| F-15 | ambiguity | resolved by 0.5 | §9.4 | Names, roots, nested messages, enums, `json_name`. |
| F-16 | ambiguity | resolved by 0.5 | §9.1 | TOML's 64-bit integer limit. |
| F-17 | ambiguity | resolved by 0.5 | contract.schema.json | Bounds only in `format`. |
| F-18 | ambiguity | resolved by 0.5 | §9.1, E020 | Which TOML date/time kinds count. |
| F-19 | gap | resolved by 0.5 | §9.1, §9.5 | Floats: `nan`/`inf`, `1e16`, `1.0` ≡ `1`. |
| F-20 | ambiguity | resolved by 0.5 | E020 | Counting; requirement annotations. |
| F-21 | gap | resolved by 0.5 | §10, W105 | A profile without a vocabulary. |
| F-22 | ambiguity | resolved by 0.5 | E018, E033 | Written or resolved values. |
| F-23 | ambiguity | resolved by 0.5 | §9.2 | Small lint edges. |
| F-24 | gap | resolved by 0.5 | §9.6 | The base64 variant. |
| F-25 | gap | resolved by 0.5 | §9.6 | Entry shapes. |
| F-26 | gap | resolved by 0.5 | §9.6 extras | Source of extras; value shape; empty `extras`. |
| F-27 | ambiguity | resolved by 0.5 | §9.7 | History check order. |
| F-28 | ambiguity | resolved by 0.5 | §9.7 | Retention identity. |
| F-29 | ambiguity | resolved by 0.5 | §9.8 | "Both directions". |
| F-30 | gap | resolved by 0.5 | §9.8 | Pairing resources (see F-40). |
| F-31 | gap | resolved by 0.5 | §9.8 | The contract table's coverage. |
| F-32 | gap | resolved by 0.5 | §9.8 | The JSON Schema rules' coverage. |
| F-33 | ambiguity | resolved by 0.5 | §9.8 | `oneOf`/`anyOf` branch identity. |
| F-34 | contradiction | resolved by 0.5 | §9.8 | A renumber "is a deletion" yet no warning. |
| F-35 | gap | resolved by 0.5 | §9.8 | The protobuf rules' coverage. |
| F-36 | gap | resolved by 0.5 | compat/ | The compat input layout. |
| F-37 | gap | resolved by 0.5 | fixture documents | Meanings deferred to Rust symbols. |
| F-38 | gap | resolved by 0.5 | §9.2, sets/ | A failing member; E035 against duplicates. |
| F-39 | ambiguity | resolved by 0.5 | E021/E022 | "The first" without key order. |
| F-40 | contradiction | resolved by 0.5 | §9.8 | "Matched by kind token" against `explicit_cleared`. |
| F-41 | ambiguity | resolved by 0.5 | §9.1 | "MAY accept later TOML" against E000. |
| F-42 | contradiction | resolved by 0.5 | CHANGELOG, compat/ | Stale counts and descriptions. |
| F-43 | ambiguity | resolved by 0.5 | §9.8 | Explicit presence. |
| F-44 | ambiguity | resolved by 0.5 | compat/README.md | Wrapper location; `invalid`. |
| F-45 | gap | resolved by 0.5 | §9.7 | Where a contract's history lives. |
| F-46 | gap | resolved by 0.5 | §3.3 | The descriptor GET's parameters. |
| F-47 | gap (measured) | resolved by 0.5 | §8.1 | The handler rule binds only the GET. |
| F-48 | gap | resolved by 0.5 | §8.4 | The bundle reply's encoding. |
| F-49 | gap | resolved by 0.5 | §8.1, §3.3, §8.4 | No timeouts. |
| F-50 | gap (measured) | resolved by 0.5 | §8.4 | Consolidation `None`. |
| F-51 | ambiguity | resolved by 0.5 | §8.4 | "Nearest holder" for a client. |
| F-52 | gap | resolved by 0.5 | §3.2 | An unbound required role. |
| F-53 | ambiguity | resolved by 0.5 | §3.3 | `profiles`. |
| F-54 | gap | resolved by 0.5 | §8.1 | When a member exists. |
| F-55 | ambiguity | resolved by 0.5 | §3.2 R3 | An unbound optional role. |
| F-56 | ambiguity | resolved by 0.6 | §2.6, E026 (0.5) | "The seconds MUST fit 64 bits": signed or unsigned? |
| F-57 | ambiguity | resolved by 0.6 | §3.3 D008, D010 (0.5) | "Once for the repeat": once per repeated value, or once per descriptor? |
| F-58 | ambiguity | resolved by 0.6 | §5.2 CBOR (0.5) | "An integer outside 64 bits": i64, u64, or their union? |
| F-59 | ambiguity | resolved by 0.6 | §8.1 timeouts (0.5) | "Waits for presence after an owner starts … 1 s": from which instant? |
| F-60 | ambiguity | resolved by 0.6 | §9.8 `oneof_branch_added` (0.5) | "More branches than the earlier one's" when one side has no `oneOf`. |
| F-61 | gap | resolved by 0.6 | §3.2, presence.md §2 step 4 (0.5) | "No instance token appears" cannot be observed when the refusing owner is its own router. |
| F-62 | contradiction | resolved by 0.6 | §9.6 against §9.4 (0.5) | One id for identical files, under the *last* name, breaks a bundle `$ref` to the first file's stem. |
| F-63 | ambiguity | resolved by 0.6 | descriptor.schema.json, §3.3, §9.1 (0.5) | `uint64` is bounded for contracts but not for descriptors' `cardinality`. |
| F-64 | ambiguity | resolved by 0.7 | §5.1 O1–O3 | A `replies = "one"` call names no consolidation. |
| F-65 | gap (observed) | resolved by 0.7 | §5.2 | An `app` envelope for an operation with no `error` type, or a raw one, in JSON: what `detail` is. |
| F-66 | ambiguity | resolved by 0.7 | §4.3 minting | "Plus one tick": which unit? |
| F-67 | ambiguity | resolved by 0.7 | §3.3, §8.2 | Is the first descriptor a "change" to put, and where in the bring-up order? |
| F-68 | gap | resolved by 0.7 | §8.2, §4.2 | When an owner puts its first state value, relative to its tokens. |
| F-69 | gap | resolved by 0.7 | §4.2 S1, state.md §1 | "A router stamp would carry the router's": not observable when the owner is the router. |
| F-70 | gap | resolved by 0.7 | §8.2 step 2 | What "exposed" means for a state resource, or a templated one, at start-up. |
| F-71 | ambiguity | **new** | §7.3 the nullable form (0.7) | "The null schema, whose type is exactly null": is `{"type": ["null"]}` one? |
| F-72 | ambiguity | **new** | §9.8 inside undecided keywords (0.7) | "A $ref back to a target already being followed is compared as written": its text, or its target? |
| F-73 | gap | **new** | state.md §1 step 3 (0.7) | "Faster than its clock advances" cannot be arranged by a tester; the tick path went unexercised. |

---

## §1 Identity and grammar

### F-01 · ambiguity · §1.2, ULID chunks

**Status at 0.5: resolved by 0.5.** §1.2 now says the check is lexical and the first character has no bound (`8zzz…` is a ULID chunk); zk2py's guess stands.

> "A ULID chunk (events, §2.6) is 26 characters of Crockford's base32 in
> lowercase: digits, and lowercase letters except `i`, `l`, `o`, `u`."

A real ULID is 128 bits, so its first character is `0`–`7`. The spec does
not say whether `8zzzzzzzzzzzzzzzzzzzzzzzzz` is accepted. `keys.json`
has no such case.
**Resolved:** a guess. zk2py follows the literal text and accepts any of
the 26 characters (`lexical.ULID`).

### F-02 · ambiguity · §1.1 (position 6+), `keys.json`

**Status at 0.5: resolved by 0.5.** §1.1 now says a key is parsed lexically, so `x-eth0` is a resource chunk; zk2py's guess stands.

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

**Status at 0.5: resolved by 0.5.** §2.2 now says parameter types play no part in matching, and a tie leaves the first listed template; zk2py's guess stands.

> "A `uint` value is written in decimal without leading zeros, then slugged
> like any value."

> "A template matches a key's resource chunks when … every parameter chunk
> decodes (§1.4)…"

Resolution mentions only decoding. Does `{n}` typed `uint` match the chunk
`03` or `abc`? `templates.json` gives templates without parameter types.
**Resolved:** a guess. Types are ignored when matching (`templates.match`).

## §3.3 The descriptor

### F-04 · gap · §3.3 and `conformance/descriptors/`: most D codes have no definition

**Status at 0.5: resolved by 0.5.** §3.3 now has "The checks": a D000–D010 table with severity (D006 alone a warning) and counting.

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

**Status at 0.5: resolved by 0.5.** §3.3 "Cascades and scope" states all five cascades; zk2py now also drops an invalid `iface` from `declared_by` (cascade 2).

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

**Status at 0.5: resolved by 0.5.** §3.3 lists what is deliberately not checked; a lowered bound of 0 is D007, a profile twice is D010. Counting a repeat leaves F-57.

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

**Status at 0.5: resolved by 0.5.** The example now holds `imu` and lists covariance with cause `config`.

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

**Status at 0.5: resolved by 0.5.** §5.2 lists the decoding edges. zk2py's guess was overturned: a CBOR byte string in a detail is base64 text, not `bytes_hex`. "An integer outside 64 bits" leaves F-58.

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

**Status at 0.5: resolved by 0.5.** §7.3 defines schema positions once. zk2py's guess was overturned: a refused keyword's content is not walked, and `definitions` is refused without being walked.

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

**Status at 0.5: resolved by 0.5.** §9.4: a scheme is a `:` in the file part; the fragment is a JSON Pointer used as written. zk2py's guess was overturned: no percent-decoding.

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

**Status at 0.5: resolved by 0.5.** §9.4: in a bundle, a file part names the artifact whose `name` is the stem of its last path segment. But see F-62: §9.6's duplicate-id rule breaks the premise.

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

**Status at 0.5: resolved by 0.5.** §9.4/E029: a duplicate member makes the file not JSON; zk2py's guess stands.

> E029: "a schema file missing, unreadable, not JSON, or not compiling"

The artifact id is "the document's JCS bytes", which a document with
duplicate members does not have. The spec does not say whether such a
document is "not JSON". (§9.6 says it for bundles: "JSON with no duplicate
member".)
**Resolved:** a guess. A duplicate member is E029.

### F-13 · ambiguity · §9.2 E024, counting

**Status at 0.5: resolved by 0.5.** E024 falls once per later file with a taken stem, and that file is not loaded, so `json:stem#Name` looks in the first. zk2py now skips loading it.

> "E024 | a `json:` name defined by several listed files, or two listed
> JSON Schema files with one stem | per reference or file"

For two files sharing a stem, is that one E024 or two? And how does a
qualified reference `json:stem#Name` resolve when the stem is ambiguous?
**Resolved:** a guess. One E024 per file whose stem an earlier file
already took. A qualified reference looks only in the first file with that
stem.

## §9.4 Protobuf artifacts

### F-14 · gap · §9.4, the well-known types' source files

**Status at 0.5: resolved by 0.5.** §9.4 names protoc 3.21.12's `include/` sources for the well-known types.

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

**Status at 0.5: resolved by 0.5.** §9.4: the first import root names a file, E029 under none; a nested message is a message, an enum is E023; listed files shadow the well-known types; every field carries `json_name`.

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

**Status at 0.5: resolved by 0.5.** §9.1 "Integers": outside the 64-bit signed range is E000; zk2py's guess stands.

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

**Status at 0.5: resolved by 0.5.** `contract.schema.json` carries `maximum` on its `uint32` fields, and §9.1 makes `format` a bound for contracts. The descriptor schema's `uint64` leaves F-63.

`major`, `minor`, `deprecated.since` and `history.depth` are
`{"type": "integer", "format": "uint32", "minimum": 0}`. JSON Schema
2020-12 treats `format` as an annotation, so a standard validator accepts
`major = 4294967296`. §9.1 bounds `major` to "0 to 2^32−1".
**Resolved:** a guess. zk2py's shape checker asserts `uint32` and `uint64`,
giving E000 and D000. A `maximum` in the schema would remove the doubt.

### F-18 · ambiguity · §9.1 and E020, "a datetime"

**Status at 0.5: resolved by 0.5.** §9.1: all four TOML date/time kinds, at any depth.

**Status at 0.4: partly resolved by 0.4.** The 0.4 changelog says a time without seconds "E020 would have refused … anyway, as an annotation datetime", so a local time counts. That is said in the changelog, not in §9.1, and nothing names a local date.

> "Annotation values are any TOML value except a datetime (E020)."

TOML has four temporal types: offset date-time, local date-time, local date
and local time. `e020-datetime` uses an offset date-time.
**Resolved:** a guess. All four count, at any depth of the value.

### F-19 · gap · §9.1 and §9.5, floats and the canonical restrictions

**Status at 0.5: resolved by 0.5.** §9.5 defines the canonical domain: `nan`/`inf` and integral floats JCS writes as integers beyond ±(2^53−1) are E028, one per value; `1.0` and `1` fingerprint alike.

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

**Status at 0.5: resolved by 0.5.** E020 counts a key's value and its name apart (one key can give two), and requirement annotations are checked. zk2py's guess (one per key) was overturned.

> "per key and table; once for `[defaults]`, whatever it reaches"

When a key is malformed *and* its value holds a datetime, is that one E020
or two? Do `[requires.<role>]` annotations count as one of these tables?
§9.1 lists `annotations` among a requirement's fields, but §9.2's E020
does not name them.
**Resolved:** a guess. One E020 per key, and requirement annotations are
checked like any other table.

### F-21 · gap · §10 point 2, Appendix D, W105 without a vocabulary

**Status at 0.5: resolved by 0.5.** §10: a profile with no interim table raises no W105; zk2py's guess stands.

> "Until a profile publishes its vocabulary, the interim tables of
> Appendix D apply, and a key outside them is a warning."

A contract may `use` a profile that has no interim table, such as
`acme.v1`. Is every key of that profile then W105, or none?
**Resolved:** a guess. None.

### F-22 · ambiguity · E018 and E033, written or resolved

**Status at 0.5: resolved by 0.5.** §9.3: lints read resolved values; zk2py's guess stands.

> "E018 | `serving = "replicated"` without `idempotent = true`"

> "E033 | `summary` without `replies = "many"`"

`serving`, `idempotent` and `replies` are defaultable (Appendix D). With
`[defaults.operation] idempotent = true`, is a resource that sets only
`serving = "replicated"` an E018?
**Resolved:** a guess. zk2py judges the resolved values (§9.3).

### F-23 · ambiguity · §9.2, small lint edges

**Status at 0.5: resolved by 0.5.** Retention may have leading zeros, a rate may not; `gate = []` is no gate (zk2py's E016 guess overturned); `history = false` on an event is E019; W103 waits for resolved types (cascade 6). "Fit 64 bits" leaves F-56.

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

**Status at 0.5: resolved by 0.5.** §9.6: RFC 4648 §4, padded, strict; zk2py's guess stands.

> "it has a `data` member, base64 text for protobuf"

The prose does not say which alphabet, whether padding is required, or
whether decoding is strict.
**Resolved:** from the fixtures. zk2py's builder reproduces
`valid.bundle.json` byte for byte with RFC 4648 §4 standard, padded base64.
The verifier is strict, so unpadded or URL-safe text is `shape`.

### F-25 · gap · §9.6 steps 7, 9 and 10, entry shapes

**Status at 0.5: resolved by 0.5.** Step 9: a non-object entry or one without `kind` is `schema_kind` (zk2py's `shape` guess overturned). Entry members besides `kind`/`data` are `shape`. An extra's `media_type` must be a string and is informative. One id listed twice is one artifact.

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

**Status at 0.5: resolved by 0.5.** By deferral: where a builder finds extra documents is `views.v1`'s; meanwhile a core builder carries no extras and such a bundle fails step 11. A `views.document` value is one id string (zk2py's list guess overturned), and a bundle always writes all three members.

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

**Status at 0.5: resolved by 0.5.** §9.7 gives the check's order. `interface` and `jcs` are both reported (zk2py's one-per-file guess overturned); `jcs` compares against all three members written.

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

**Status at 0.5: resolved by 0.5.** §9.7 defines retention identity apart from the classes, with protoc's default `json_name`; zk2py implements it (`compat.identical`).

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

**Status at 0.5: resolved by 0.5.** §9.8 now says a direction is a role, never a swap.

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

**Status at 0.5: resolved by 0.5.** §9.8: resources pair by kind and template (see F-40).

**Status at 0.4: resolved by 0.3, but contradictorily.** §9.8 now says resources are "matched by kind token and template", which contradicts its own explicit rows and the fixture: see F-40.

The spec never says how two revisions' resources are paired.
- **By `(kind token, template)`:** `explicit` true → false becomes a
  removal plus an addition, so breaking.
- **By template text:** it is the review the fixture expects.

**Resolved:** from the fixture. zk2py pairs by template, which is unique
within a contract because it is the table key.

### F-31 · gap · §9.8, the contract table is not exhaustive

**Status at 0.5: resolved by 0.5.** §9.8: only artifacts a type reaches are compared; an unreferenced one changing is compatible.

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

**Status at 0.5: resolved by 0.5.** §9.8: `$ref` siblings are merged into the target and compared (zk2py's review guess overturned); a boolean schema change is review; a `required` name without a property is breaking.

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

**Status at 0.5: resolved by 0.5.** §9.8: `oneOf`/`anyOf`/`prefixItems` compare as written, in order, so a reordering is review (zk2py's multiset guess overturned); "a branch added" is more branches; the asymmetry is deliberate. An absent `oneOf` leaves F-60.

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

**Status at 0.5: resolved by 0.5.** The renumber bullet no longer calls a renumber a deletion.

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

**Status at 0.5: resolved by 0.5.** §9.8: structure from the named type only (nested types no field reaches are not compared); oneofs by name; enums by number, closed by the candidate's syntax alone; proto2 defaults (zk2py's review guess overturned), options and reserved names not compared; warnings deduplicated.

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

**Status at 0.5: resolved by 0.5.** Resolved by 0.3; nothing more.

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

**Status at 0.5: resolved by 0.5.** Fixture documents cite spec sections; only a provenance line naming `zenkey-model` remains, which defines nothing.

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

**Status at 0.5: resolved by 0.5.** §9.2: a set check runs only when every member loads, in file-name order; E035 against the first declaration. zk2py's "refuse the set" guess was overturned: a failing member's own codes stand.

> "E035 and E036 are set checks: each file is loaded on its own first."

Only `sets/expect.json` adds "and must load". The spec does not say what a
set check reports when a member does not load. Nor does it say which
declaration E035 checks a requirement against when E036 reports two
contracts declaring the same interface.
**Resolved:** a guess. A member that does not load refuses the whole set
check. E035 uses the first declaration in file-name order.

### F-39 · ambiguity · §9.2 E021 and E022, "after the first"

**Status at 0.5: resolved by 0.5.** §9.2 "Order": template order, not document order (zk2py's guess overturned); E021 does not take a resource out of W101.

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

**Status at 0.5: resolved by 0.5.** §9.8 pairs by kind (not kind token) and template; a kind change is `resource_removed`.

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

**Status at 0.5: resolved by 0.5.** §9.1: the list is closed, and a reader "MAY be a parser of later TOML, provided it reports these as E000".

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

**Status at 0.5: resolved by 0.5.** Corrected: CHANGELOG 0.3 now reads 23 new, and `compat/expect.json`'s description no longer says inputs are only loaded.

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

**Status at 0.5: resolved by 0.5.** §9.8: presence is protobuf's definition, and a oneof move reports `oneof_changed` alone (zk2py's two-rule guess overturned).

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

**Status at 0.5: resolved by 0.5.** `compat/README.md`: the wrapper sits beside its artifact; only a candidate can be `invalid`.

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

**Status at 0.5: resolved by 0.5.** §9.7: a configured history root, a contract's history found by interface id; `examples/zk2/README.md` states the examples' requirement.

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

## The live half, first slice (against 0.4; #609, #610)

All ten are resolved by 0.5; their status is in the summary table at the
top, and on each entry.

These come from `zk2py.live` and `zk2py.live_interop`, which run the Rust
owner example as a black box over zenoh-python 1.10.1. The setup: loopback,
the owner as the router, and zk2py as a client of it. The rules read are
§3.3, §8.1–§8.4, `scenarios/presence.md` and `scenarios/retrieval.md`. Two
entries rest on measurements, which are given.

| Id | Severity | Location | In one line |
|---|---|---|---|
| F-46 | gap | §3.3 | The descriptor GET: no target, consolidation, timeout or reply encoding is stated. |
| F-47 | gap (measured) | §8.1 "Reading presence", presence.md §4 | The handler rule binds only the GET. A bounded, undrained *subscriber* on the same session starves even a callback GET, which then returns 257 of 2,002 tokens at its timeout. |
| F-48 | gap | §8.4, §9.6 | The bundle reply's encoding is not stated. |
| F-49 | gap | §8.1, §3.3, §8.4 | No timeout anywhere: liveliness GET, descriptor GET, each §8.4 attempt, waiting for presence. |
| F-50 | gap (measured) | §8.4 step 2 | "Verify each reply as it arrives" needs consolidation `None`. With zenoh's default, a slow corrupt holder's reply arrives alone, at completion, and the valid bundle is consolidated away. |
| F-51 | ambiguity | §8.4 step 1, retrieval.md §1 | "The nearest holder on each router": from a client that holds a bundle itself, `BestMatching` returned two replies. |
| F-52 | gap | §3.1, §3.2 R1, §8.2 | Nothing says an owner with an unbound *required role* must not start, yet the reference owner refuses to, citing R1. |
| F-53 | ambiguity | §3.3, §10 point 4 | Is a descriptor's `profiles` the union of its contracts' `uses`? |
| F-54 | gap | §8.1 member tokens | When does a member exist, and so need its token? |
| F-55 | ambiguity | §3.2 R3 | Is an unconfigured optional role listed with `bindings: []`, or omitted? |

### F-46 · gap · §3.3, the descriptor GET

**Status at 0.5: resolved by 0.5.** §3.3 "The GET": one reply, `application/json`, no timestamp, no attachment, consolidation `None`; the target is the caller's.

> "Every instance serves a **descriptor**: a JSON document answered on GET
> at its instance key, and put on every change."

The GET's target, consolidation and timeout are not given. Nor is the
reply's `Encoding`: §7.2's encoding rule is about resource samples, and
§5.2's about the error envelope. Nor is whether the reply carries a
timestamp (S1–S2 bind state, not control keys), or how many replies to
expect.

The owner answered with one reply, encoded `application/json`, with no
timestamp and no attachment.
**Resolved:** a guess:
- zk2py GETs with `BestMatching`, consolidation `None` (see F-50) and 5 s;
- it discards a reply on a key that is not concrete (R6);
- it treats `application/json` as an expectation the spec does not state.

### F-47 · gap (measured) · §8.1 "Reading presence" and `presence.md` §4

**Status at 0.5: resolved by 0.5.** §8.1: every liveliness subscriber on the session MUST be callback-driven or drained; a GET that ended at its timeout SHOULD be read as possibly incomplete; presence.md §4 now states zenoh-python's measurement.

> "A caller or tool's liveliness GET on a session that holds a liveliness
> subscriber MUST use a callback or an unbounded handler. With zenoh's
> default 256-slot handler, such a GET hung at every measured size from
> 996 tokens (zenoh#2678)."

Measured with zenoh-python 1.10.1:
- **Setup:** 2,000 extra instance tokens, declared by a second client. The
  reading session holds a liveliness subscriber on `zk2/*/*/@zk/**`, and
  issues `liveliness().get("zk2/*/*/@zk/**", timeout=10)`.
- **Total:** 2,002 tokens, counting the owner's two.

| GET handler | the session's liveliness subscriber | result |
|---|---|---|
| `Callback` | `Callback` | complete: 2,002 in 0.34 s |
| default, drained as replies arrive | `Callback` | complete: 2,002 in 0.43 s |
| default, drained after 3 s | `Callback` | complete: 2,002 in 3.0 s |
| default | default, `history=True`, never drained | **hung**: 0 replies after 20 s |
| `Callback` | default, `history=True`, never drained | **257 of 2,002**, ended at the 10 s timeout |

Three things follow:
1. **The scenario did not reproduce as written.** `presence.md` §4's "With
   zenoh's default 256-slot handler, it hangs" did not happen for the GET's
   handler alone in zenoh-python.
2. **What starves the GET is the subscriber's handler,** when it is bounded
   and nobody drains it. Then even a callback GET is cut short, silently,
   at its timeout. A tool that trusts it under-reports presence, which is
   exactly what O5 and R7 warn against.
3. **zenoh-python has no unbounded handler.** Its default handler and
   `FifoChannel` are bounded, and `RingChannel` drops. So "an unbounded
   handler" is not an option there; only a callback is.

The rule should also bind the subscriber's handler. A tool should treat a
liveliness GET that ended at its timeout, rather than at the routers' final
reply, as possibly incomplete.

**Resolved:** how zk2py meets the rule:
- every liveliness GET passes `zenoh.handlers.Callback(on_reply, on_done)`
  (stable API, default `indirect` mode). The callback only appends to a
  Python list, and `on_done` marks completion;
- the session holds no bounded subscriber. The scale check's subscriber is
  a `Callback` too;
- `list_presence` reports `complete=False` when the GET ended at its
  timeout.

The runner checks the §4 scenario with callbacks: 2,000 of 2,000 tokens in
about 0.15 s while a subscriber is held.

### F-48 · gap · §8.4 and §9.6, the bundle reply's encoding

**Status at 0.5: resolved by 0.5.** §8.4: one reply, `application/json`, and a caller MUST NOT depend on the encoding.

§9.6 defines the bundle *bytes* (JCS). §8.4 defines how to retrieve and
verify them. Neither says what `Encoding` a holder sets on its reply. The
owner sets `application/json`.
**Resolved:** zk2py does not depend on it. Verification is by hash, as §8.4
intends ("the hash is the check"). zk2py records the encoding in its report.

### F-49 · gap · timeouts: §8.1, §3.3 and §8.4

**Status at 0.5: resolved by 0.5.** §8.1: timeouts are the caller's; a conformance run uses 1 s. The starting instant leaves F-59.

The spec sets no timeout for any of these:
- a liveliness GET;
- the descriptor GET;
- each of §8.4's two attempts. "If none was valid, retry once" depends on
  when the first attempt counts as done;
- how long a tool waits for presence after an owner starts.

S6 speaks of "the GET's timeout" as if it were given.
**Resolved:** a guess:
- 5 s per GET;
- 2 s for the attempts the runner expects to fail;
- 30 s to wait for presence, polled every 0.2 s.

### F-50 · gap (measured) · §8.4 step 2 and consolidation

**Status at 0.5: resolved by 0.5.** §8.4: consolidation `None` is a MUST on both attempts, with the measurement.

> "Verify each reply **as it arrives** (§9.6), and accept the first valid
> one, without waiting for the GET to complete."

§4.1 says `Latest` consolidation "delivers at query completion", and
zenoh's default for a GET is `Auto`, which is `Latest` on a concrete key.
§8.4 never says to set consolidation. O2 and O6 do say it, for calls.

Measured: on the `nav.v2` contract key, zk2py's session held a holder that
answers a corrupt 12-byte bundle after 2 s, and the owner's holder answers
the valid 4,800-byte bundle at once:

| Consolidation | Target | Replies delivered (arrival in s, size in bytes) |
|---|---|---|
| default (`Auto`) | `BestMatching` | (2.001, 12) |
| default (`Auto`) | `All` | (2.000, 12) |
| `None` | `BestMatching` | (0.001, 4800), (2.000, 12) |
| `None` | `All` | (0.001, 4800), (2.000, 12) |

With the default, the valid bundle never arrives: it is consolidated away,
and only the corrupt one is delivered, at completion. A tool that follows
§8.4 to the letter, with zenoh's defaults, waits 2 s, refuses the corrupt
reply, retries with `All`, and reports the contract unavailable while a
valid holder answers in 1 ms.

**Resolved:** a guess. zk2py sets consolidation `None` for every §8.4 GET,
and for the descriptor GET. §8.4 should say so.

### F-51 · ambiguity · §8.4 step 1 and `retrieval.md` §1, "nearest holder"

**Status at 0.5: resolved by 0.5.** §8.4 step 1: a holder on the caller's own session answers too; assume nothing about the count.

> "GET with target `BestMatching`. That reaches the nearest holder on each
> router the query visits."

> retrieval.md §1: "With 200 equal holders on one router and
> `BestMatching`, one reply."

Measured in the same setup as F-50: when zk2py's own client session held a
complete queryable on the contract key, a `BestMatching` GET from that
session returned **two** replies, its own and the owner's (see F-50's
table).

So the querying session's own holder counts as a holder of its own.
"Nearest" is the routers' choice, not one a client can make or observe. The
spec does not say how many replies a tool may get from `BestMatching`.
**Resolved:** zk2py assumes nothing about the count. It verifies every reply
that arrives, and accepts the first valid one. The runner's
corrupt-nearest-holder check passes either way.

### F-52 · gap · §3.1, §3.2 R1, §8.2: an unbound required role

**Status at 0.5: resolved by 0.5.** §3.2: an owner whose configuration binds a required role to nothing MUST NOT start. Observing that from outside leaves F-61.

The owner example refused to start on `examples/zk2/walkthrough/thruster.v1.toml`.
It exited with status 1 and declared no instance token, saying:
`NotExposed("role \"cmd\" (twist_cmd.v1) is required and the configuration
binds it to nothing (R1)")`.

R1 says: "a role MUST be bound by configuration, never in code".
- **Required resources:** §8.2 validates that every required *resource* is
  exposed, and `presence.md` §2 step 3 says such an owner "does not start".
- **Required roles:** nothing says an owner with an unbound required
  *role* must not start.

R5 ("A binding resolves at once") and R7 ("A binding MUST NOT require
presence") suggest bindings never block start-up. The owner's documented
contract ("it implements every contract given") does not hold for this
contract either.

**Resolved:** the runner uses only contracts the owner accepts. In place of
`thruster.v1` it uses `camera.v1`, also protobuf. The spec should say what
an unbound required role does.

### F-53 · ambiguity · §3.3 and §10 point 4, the descriptor's `profiles`

**Status at 0.5: resolved by 0.5.** §3.3: `profiles` is the union of the contracts' `uses`, sorted and deduplicated; not checked by the checker.

> descriptor.schema.json: "The profiles this instance follows, as
> `<name>.v<major>`."

The owner listed the union of its contracts' `uses`. For example,
`["freshness.v1", "telemetry.v1"]` for `zk2py_probe.v1` with `zs.snmp.v1`.
Nothing says whether `profiles` must equal, include, or be independent of
those `uses`.
**Resolved:** zk2py does not check `profiles` beyond D010's syntax.

### F-54 · gap · §8.1 member tokens: when does a member exist?

**Status at 0.5: resolved by 0.5.** §8.1: a member exists from the owner's first declaration of it; no member, no member token.

> "The owner MUST hold one member token per member, and cycle it whenever
> that member's continuity breaks."

The spec does not say when a member comes into being: when its first value
is published, when it is configured, or when its device appears.
`zs.snmp.v1` and `zk2py_probe.v1` both have an `epoch` template. The owner,
which publishes no data, held no member token for either.
**Resolved:** zk2py reports the member-token count. It asserts nothing.

### F-55 · ambiguity · §3.2 R3, an unconfigured optional role

**Status at 0.5: resolved by 0.5.** §3.2/§3.3: an unbound optional role is listed with `"bindings": []` and `"params": {}`.

> "**Owner:** the descriptor (§3.3) MUST list every requirement with its
> bindings and parameter bindings as configured."

The owner listed `zk2py_probe.v1`'s optional, unbound role as
`{"role": "upstream", "interface": "nav.v2", "declared_by":
"zk2py_probe.v1", "bindings": [], "params": {}}`.

"As configured" could also be read as "only the roles the configuration
binds". With that reading, an unbound optional role would be omitted, and
the data-flow graph (R3's "read from descriptors") would lose an edge the
contract declares.
**Resolved:** a guess. zk2py's runner requires every contract-declared role
to be listed. The owner passes.

## New at 0.5 (against amendment 0.5)

Found while making zk2py follow 0.5: `just py-conformance` passes 485 of
485, and `just py-live` 60 of 60. Five entries follow up text that 0.5
added. F-62 is a contradiction between two 0.5 rules, and was reproduced.
No fixture pins any of these.

### F-56 · ambiguity · §2.6 and E026 (0.5): "the seconds MUST fit 64 bits"

**Status at 0.6: resolved by 0.6.** §2.6/E026: the seconds are unsigned; above 2^64−1 is E026, and 2^63 s is E028 in the canonical form. zk2py's guess was the rule.

> "In a retention, `<n>` is decimal digits, leading zeros allowed, and at
> least 1 … The seconds MUST fit 64 bits (E026) and, like every canonical
> integer, ±(2^53−1) (E028)."

The spec does not say whether 64 bits means signed or unsigned. A retention
of `9223372036854775808s` (2^63) fits an unsigned 64-bit integer but not a
signed one, so it is either E028 alone or E026 and nothing else (E026 stops
the canonical form). Elsewhere 0.5 bounds integers both ways: TOML's are
signed (§9.1), and a descriptor's `minor` goes to 2^64−1 (§3.3).
**Resolved:** a guess. zk2py uses unsigned: E026 above 2^64−1, then E028
from the canonical form.

### F-57 · ambiguity · §3.3's D008 and D010 (0.5): "once for the repeat"

**Status at 0.6: resolved by 0.6.** §3.3: a repeat is one D008 or D010 for the whole list, and a malformed value counts at each occurrence. zk2py's guess (one per repeated value) was overturned.

> D008 "a capability is not `[a-z0-9][a-z0-9_.-]*`; or one is listed twice
> | per capability; once for the repeat"; D010 likewise for profiles.

Two readings, and the fixtures (`d008-twice`, `d010-twice`) have a single
repeated value, so they cannot tell them apart:
- once per repeated *value*: `["a", "a", "b", "b"]` gives two;
- once for the descriptor, however many values repeat.

Also open: a malformed value listed twice. Is that two D008 for the
malformed occurrences plus one for the repeat?
**Resolved:** a guess. zk2py counts once per repeated value, plus once per
malformed occurrence (`descriptor._repeats`).

### F-58 · ambiguity · §5.2, CBOR (0.5): "an integer outside 64 bits"

**Status at 0.6: resolved by 0.6.** §5.2: a CBOR integer decodes from −2^63 to 2^64−1. zk2py's guess was the rule.

> "A map key that is not text, an integer outside 64 bits, or a float that
> is not finite is `decode`."

CBOR's major type 0 reaches 2^64−1, and major type 1 reaches −2^64. "Outside
64 bits" could mean outside `i64`, outside `u64`, or outside both. 2^63 and
−2^63−1 fall differently under each reading. No case in `errors/cases.json`
has such an integer.
**Resolved:** a guess. zk2py refuses only what neither `i64` nor `u64`
holds, that is, below −2^63.

### F-59 · ambiguity · §8.1 (0.5): when "after an owner starts" begins

**Status at 0.6: resolved by 0.6.** §8.1: the wait starts at the later of the tool's session connecting and the owner's launch, which is the connection when the owner is the tool's router. zk2py's guess was the rule.

> "how long a tool waits for presence after an owner starts, are the
> caller's choices. The scenarios, and so a conformance run, use 1 s unless
> they say otherwise."

A tool cannot observe "starts". The second could be counted from at least
four instants:
- the process launch;
- the owner's router accepting connections;
- the tool's session connecting;
- the owner's last start-up step (§8.2).

The figure is tight enough that the choice matters: process start-up alone
can take a good part of a second.
**Resolved:** a guess. zk2py counts the 1 s from its client session's
connection to the owner's router. The owner example's presence appeared
within it in every run (2 and 4 tokens, in about 1 ms).

### F-60 · ambiguity · §9.8 `oneof_branch_added` (0.5): a side with no `oneOf`

**Status at 0.6: resolved by 0.6.** §9.8: `oneof_branch_added` needs a `oneOf` on both sides; adding or removing the keyword is `undecided_changed`. zk2py's guess (breaking) was overturned.

> "a `oneOf` branch added (`oneof_branch_added`): the candidate's `oneOf`
> has more branches than the earlier one's, whatever they hold."

The rule is open when the earlier revision has no `oneOf` at that position.
Adding a `oneOf` keyword could be:
- `oneof_branch_added`, reading the absent `oneOf` as zero branches; or
- `undecided_changed`, "any other change inside" it.

`oneof-add-branch` has a `oneOf` on both sides.
**Resolved:** a guess. zk2py reads an absent `oneOf` as zero branches, so
adding one is breaking. Removing one is review.

### F-61 · gap · §3.2 and `presence.md` §2 step 4 (0.5): observing a refusal

**Status at 0.6: resolved by 0.6.** §3.2 and presence.md §2 now watch a refusal through a router R1 that outlives the owner, against a control. The Rust owner example is still its own router, so its refusal check still rests on silence. zk2py's own owner, as a client of R1, now follows the scenario in full (`run_python_refusal`).

> §3.2: "An owner whose configuration binds a required role to nothing MUST
> NOT start, as one missing a required resource does not (§8.2 step 2): no
> instance token appears."

The live runner starts the owner on `walkthrough/thruster.v1`. The owner
example is its own router: it prints `listening`, finds the unbound role,
and exits with status 1. zk2py's client completed no presence GET before
the router was gone. So "no instance token appears" was judged on silence,
which O5 says is never a verdict, plus the missing `ready` line and the
exit.

The spec gives no observable for a refusal when the refusing participant
is also the only router. A scenario could run the owner as a client of a
separate router, where a watcher sees no token.
**Resolved:** the runner's check is named for what it observes ("no
instance token while it ran, no `ready` line"), and reports how many
presence GETs it completed: 0.

### F-62 · contradiction · §9.6 against §9.4 (0.5): one id for identical files

**Status at 0.6: resolved by 0.6.** §9.4: a later listed file with an earlier one's id is E024 and is not loaded; the classifier reads a dangling `$ref` as `schema_unreadable`. zk2py follows both.

> §9.6: "Two listed JSON Schema files with identical bytes have one id, so
> the canonical form lists it once, under the `name` of the last of them in
> `[schemas]` order."

> §9.4: "In a bundle, which keeps no paths, the file part names the
> artifact whose `name` is the stem of its last path segment. Stems are
> unique per contract, so this is the file the path named."

The two rules meet in one contract, which zk2py reproduced:
- `[schemas] jsonschema = ["a.json", "b.json", "c.json"]`, with `a.json`
  and `b.json` byte-identical;
- `c.json` holding `{"$ref": "a.json#/$defs/X"}`.

In the source tree the `$ref` resolves by path, so the contract lints clean
(no code). The canonical form lists the shared id once, as `b`. In its
bundle, the file part `a.json` names stem `a`, which no artifact has: the
`$ref` resolves to nothing. §9.8 follows `$ref`s "by stem as in a bundle",
so the classifier cannot see what `c.json` refers to. "So this is the file
the path named" is false here.
**Resolved:** a guess. zk2py follows both rules as written, and so
reproduces the dangling `$ref`: building that contract and reading its
bundle back gives the stems `b` and `c` only. Either rule could yield, for
example:
- list a shared id under every name;
- refuse identical listed files (an E024-like code);
- resolve a bundle file part by *any* name the id was listed under.

### F-63 · ambiguity · `descriptor.schema.json`, §3.3 and §9.1 (0.5): the `uint64` bound

**Status at 0.6: resolved by 0.6.** §3.3: `format` is a bound in a descriptor too, `uint64` being 0 to 2^64−1. zk2py's guess was the rule.

> §9.1: "The schema's `format` is a bound here, not an annotation: `uint32`
> is 0 to 2^32−1, and `uint64` is 0 to 2^63−1, TOML's own bound."

That sentence is scoped to the authoring format ("here").
`descriptor.schema.json` uses `"format": "uint64"`, with no `maximum`, for
`minor` and for every `cardinality` value. §3.3 gives `minor` "0 to
2^64−1". It bounds `cardinality` only by the contract's value (D007). So
whether a descriptor whose `cardinality` value is 2^64 is D000 (outside
`uint64`) or D007 (above the contract's bound) depends on whether `format`
is a bound for descriptors too.
**Resolved:** a guess. zk2py's shape checker bounds `uint64` to 0..2^64−1
in both schemas, so such a value is D000. For contracts this changes
nothing, since TOML cannot write a larger integer.

## New at 0.6: state, operations, and being an owner (#609)

These come from the rest of #609's live scope:
- `zk2py.live.get_state` and `zk2py.live.call`;
- `zk2py.owner`, a minimal owner;
- `zk2py.live_interop`, which reads the Rust owner example's state and
  calls its operations, and lets the Rust `consume` example read zk2py's
  owner.

`just py-live` passes 113 of 113. Each entry below is a rule the
implementation had to guess.

### F-64 · ambiguity · §5.1 O1–O3: the consolidation of a single-reply call

**Status at 0.7: resolved by 0.7.** §5.1 O1: a concrete call MUST set `BestMatching` and `None`. zk2py's guess was the rule.

O2 gives a fan-out call target `All` and consolidation `None`, and O6
gives a `replies = "many"` call consolidation `None`. O1 implies target
`BestMatching` for a concrete call. Nothing names the consolidation of the
common case, a concrete call to a `replies = "one"` operation.

zenoh's default (`Auto`, `Latest` on a concrete key) holds the reply until
the query completes (§4.1), the effect §8.4 measured and forbade for
retrieval. It also ranks an unstamped reply, which every operation reply
is, lowest.
**Resolved:** a guess. zk2py's `call` uses `None` for every call, so a
value or an envelope is seen as it arrives (`live.call`).

### F-65 · gap (observed) · §5.2: `app`, and what `detail` is, in a JSON envelope

**Status at 0.7: resolved by 0.7.** §5.2: any operation may refuse with `app`; with no `error` type there is no detail, with one it is optional, and a raw type's is base64 text in a JSON envelope. zk2py's owner sends `app` without a detail, as the rule says. The Rust owner example still sends an empty detail on `@op/refuse`, a fix 0.7 records for the runtime: the runner reports it as a known deviation (XFAIL).

> "`detail` | a value, bytes, or null | With `app` only: the operation's
> declared `error` type, as a value inline (JSON, CBOR) or as its encoded
> message (protobuf)"

Three cases are not covered:
- **An operation that declares no `error` type.** May it reply `app`,
  and is `detail` then null?
- **A raw `error` type.** The envelope is JSON ("A raw type: the envelope is
  JSON"), and the detail is bytes. Is that base64 text, §7.2's JSON form of
  bytes?
- **A JSON Schema operation whose `error` type is raw or absent.** The same
  question.

Observed on the reference owner example, whose documented contract
refuses "any other types … with an `app` error envelope":
- every protobuf operation got `app` in `zk2.core.v1.Error`, with an empty
  `detail`;
- every JSON-enveloped operation got `{"code": "internal", "message": "the
  app detail does not fit the envelope's encoding"}` instead. That held with
  no `error` type, with `error = "json:Status"`, with `error = { raw =
  "text/plain" }`, and with `encoding = "cbor"` (as CBOR).

So the reference has no `app` envelope for those operations, and the spec
does not say whether that is right.
**Resolved:** zk2py's owner sends `app` without a detail, and `invalid_request`
for a malformed JSON request. zk2py's runner accepts any envelope that
decodes for the Rust owner's JSON operation, and records the code.

### F-66 · ambiguity · §4.3 minting: "plus one tick"

**Status at 0.7: resolved by 0.7.** §4.3: a tick is the timestamp type's smallest step, one NTP64 unit, and any larger step, zenoh-python's 1 ns included, keeps S7. zk2py's guess was allowed.

> "An owner therefore mints each state timestamp as the greater of
> `Session::new_timestamp()` and the last timestamp it issued plus one
> tick, with its session's zid as the id."

The spec does not define a tick. NTP64's own unit is 2^−32 s (about 0.23
ns), an HLC's is whatever it increments by, and zenoh-python builds an
`NTP64` from seconds and nanoseconds only. So a Python owner's smallest step
is 1 ns, about four of NTP64's units.
**Resolved:** a guess. zk2py adds 1 ns (`Owner.mint`). Any positive step
keeps S7's "never at or below the last", which is what the rule protects.

### F-67 · ambiguity · §3.3 and §8.2: the descriptor's first put

**Status at 0.7: resolved by 0.7.** §3.3, §8.2 step 3: the first descriptor is put when its queryable is declared, before the contract queryables and any token. zk2py now puts it there; it used to come after the contract queryables.

> §3.3: "The owner MUST put the descriptor on its instance key whenever it
> changes, and MUST answer a GET there with the current one."

Two points are open:
- **The first descriptor.** Is an owner's first descriptor a "change" that
  must be put?
- **Its place in the order.** §8.2's order names the descriptor's
  *queryable* (step 3), not a put. A subscriber to instance keys, as
  presence.md §1's tool is, sees a put only if there is one, and
  a late one could land after the tokens.

**Resolved:** a guess. zk2py's owner puts the descriptor once at start,
stamped, after step 3 and before the tokens.

### F-68 · gap · §8.2 and §4.2: the first state value against the tokens

**Status at 0.7: resolved by 0.7.** §8.2 "State values": a value held at start SHOULD be put before step 4. zk2py's guess was the rule. The Rust owner example still puts after starting (a fix 0.7 records); the runner's first-sight GET reports it as a known deviation, which a race can hide (XPASS).

§8.2's order makes "alive ⇒ callable" hold for operations: queryables come
before tokens. A state resource's first *value* has no place in that order.
An owner that declares its publisher and state queryable (step 1), then its
tokens (step 4), and only then puts its first value, answers a GET made the
moment its interface token appears with silence. S6 says silence is not a
verdict, but a consumer acting on presence then reads nothing for a state
the owner was about to set.

**Resolved:** a guess. zk2py's owner puts every state value before its
tokens. The Rust owner example's value was present whenever zk2py read it.
That was after presence and the descriptor GET, though, so the runner does
not test the moment the token appears.

### F-69 · gap · §4.2 S1 and `state.md` §1: an owner stamp that cannot be told from a router's

**Status at 0.7: resolved by 0.7.** §4.2 "Observing S1" and state.md §1: the owner and the consumer are clients of a router with timestamping on, against an unstamped control put. zk2py's owner now follows it in full (`run_python_s1`). The Rust owner example is still its own router, so its S1 check proves only the process's id.

> state.md §1: "Every sample, the delete included, carries a timestamp
> whose id is the owner session's zid. A router stamp would carry the
> router's."

Both the Rust owner example and zk2py's owner are their own routers in the
runner. There, the owner session's zid is the router's, so the check
cannot tell an owner stamp from a router stamp. The runner's S1 check
passes in both cases, but it proves only that the stamp's id is the
process's.

This is F-61's situation again, for state: the scenario needs the owner as
a client of a separate router, which state.md does not say.
**Resolved:** zk2py's owner can run as a client of a router
(`Owner(connect=…)`), which `run_python_refusal` uses. The S1 check against
an owner behind a separate router is left for when the Rust owner can be
one too.

### F-70 · gap · §8.2 step 2: what "exposed" means at start-up

**Status at 0.7: resolved by 0.7.** §8.2 "Exposed": what serves a resource is declared; a state's value is not needed, and a template with no member is exposed by its template. zk2py's guess (a value needed; a required template refused) was overturned. zk2py now exposes every resource it is not told to withhold: states by their queryables and publisher (a raw one holds `ok`, others no value), streams by their publisher, events and templates by nothing more, and operations by a queryable on the key or over the template. Step 2 refuses what the rule refuses.

> §8.2: "2. validate that every required resource is exposed"; §2.3: "An
> owner MUST expose every required one, or not start (§8.2)."

"Exposed" is defined for the descriptor ("The exposed resources of an
interface are its contract's resources, minus …", §3.3), not for the
runtime check. The spec does not say what being exposed takes:
- for a state resource: a publisher, a queryable, a value;
- for a templated resource with no member yet: a queryable over its
  template.

An owner that serves only some resources cannot know whether the others
count as exposed.
**Resolved:** a guess. zk2py's minimal owner counts a parameterless raw
state, once its publisher, queryable and value exist, and a parameterless
operation, once its queryable exists. It serves nothing templated, so it
refuses to start for a contract with a required templated resource.

## New at 0.7 (#609)

Found while following 0.7: `just py-conformance` passes 514 of 514.
`just py-live` passes 129 of 129, with 2 known deviations of the Rust owner
example.

### F-71 · ambiguity · §7.3 (0.7): what "the null schema" is

> "holding two branches in either order: the null schema, whose `type` is
> exactly `null` and which holds nothing else that carries meaning, and any
> schema S"

`type` may be a string or a list (§7.3 keeps `type` as JSON Schema has it).
So `{"type": ["null"]}` admits exactly null too, but is it "exactly
`null`"? No fixture has one. A generator writing the list form would make
the same `Option` a nullable form for one implementation and a plain
`anyOf` for another, whose classes then differ (compatible or review).
**Resolved:** a guess. zk2py takes only the string `"null"`
(`compat._is_null_schema`).

### F-72 · ambiguity · §9.8 (0.7): a recursive `$ref` "compared as written"

> "A `$ref` back to a target already being followed is compared as written,
> which ends a recursive type"

"As written" could mean the `$ref`'s text, or the target it names. Two
revisions can spell one target two ways:
- `#/$defs/Node` in the defining file;
- `nodes.json#/$defs/Node` from another file.

By text they differ, so the change is review. By target nothing changed.
No fixture has a recursive type inside `oneOf`/`anyOf`/`prefixItems`.
**Resolved:** a guess. zk2py compares such a `$ref` by its target, written
as `<stem>.json#<pointer>`, so a respelling is no change
(`compat.written`).

### F-73 · gap · `state.md` §1 step 3 (0.7): a precondition a tester cannot arrange

> "3. The owner puts v3 and v4 back to back, faster than its clock
> advances. … v4's timestamp is greater than v3's, by at least one tick"

Whether two puts land within one clock reading is the owner's timing,
which a black-box tester cannot force. zk2py's owner, put through
zenoh-python, issued v3 and v4 53 µs apart. The HLC had advanced, so the
"plus one tick" branch of §4.3's minting never ran, and the check passed
without testing it. The scenario does not say how a run shows the branch
was taken, nor whether a run that did not take it counts.
**Resolved:** the runner checks what the scenario expects (v4 > v3 by at
least a tick). It reports the gap in nanoseconds, which shows the branch
was not exercised. zk2py's minting branch is exercised only by its own
logic, not by a test here.
