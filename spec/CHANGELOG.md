# zk2 spec changelog

Amendments to [`core.md`](core.md). Each entry records what changed, what
deliberately did not, and why.

## 0.5 — 2026-10-08: the second implementation's findings (#609, #607)

The Python implementation (#609) was written from `spec/` alone, and passes
every fixture. It recorded 45 places, F-01 to F-45, where the spec was
silent, ambiguous or said two things. Each is resolved here, in one of two
ways:
- **the reference's behaviour becomes the rule**, stated where a reader
  looks for it; or,
- **where that behaviour was a bug**, `zenkey-model` is fixed, and the
  corrected rule is stated.

A fixture pins every rule a fixture can check: 21 contract cases, 4
descriptors, 1 set, 3 histories, 6 bundles, 11 error envelopes, 17
compatibility cases, 2 keys and 1 template case. No existing expectation
changed.

**Changed: the reference was wrong, and is fixed.**
- **Floats (F-19).** `nan` and `inf` in an annotation became `null` without a
  word, so a contract changed meaning and linted clean. A float that JCS
  writes as an integer beyond ±(2^53−1), such as `1e16`, linted clean and
  built a bundle no implementation verifies (step 6). §9.5 now defines the
  **canonical domain**: an integer within ±(2^53−1); a float finite and,
  when integral and below 10^21, within the same range. Outside it is E028,
  in the canonical form and in a JSON Schema artifact.
- **TOML integers (F-16).** The reference reader accepted 2^63 to 2^64−1,
  which TOML 1.0 does not have, and reported E028 where the value reached
  the canonical form. An integer outside the 64-bit signed range is now
  E000 (§9.1), as in a 1.0 reader.
- **W103 beside E023 (F-23).** W103 fired when the type did not resolve,
  with a message calling it protobuf or raw. It is now judged on resolved
  types only (cascade 6).
- **E035 under E036 (F-38).** E035 checked the *last* declaration of a
  duplicated interface, an accident of a map insert. It checks the first,
  and E036 falls on each later one.
- **D010 (F-06).** A profile listed twice was silent, unlike a capability
  (D008) or an interface (D003). It is D010.
- **Schema positions (F-09, F-10).** `$ref`s were collected anywhere in a
  JSON Schema document, data included, so `"const": {"$ref": "x"}` was an
  E032. §7.3 defines schema positions once; E037, E032 and the classifier
  read only them.
- **What the classifier could not see (F-32, F-33).** Three changes were
  invisible, so a breaking change could pass as compatible:
  - keywords beside a `$ref` were dropped; they are now added to the
    target and compared;
  - `true` → `false` in a property compared as two empty schemas; any
    change to or from a boolean schema is now review;
  - a `required` name with no property was never compared; adding or
    removing one is now breaking.

  Annotations were also stripped by key at any depth inside `oneOf`, so a
  property named `title` there was invisible; they are now stripped at
  schema positions only.
- **The error envelope (F-08).** A CBOR byte string in a `detail` decoded
  as an array of numbers; it is now base64 text, the JSON form of bytes
  (§7.2). Bytes after the CBOR item were ignored; they are `decode`.
- **Bundle entries (F-25).** A schema or extra entry could carry members no
  hash covers, which contradicted "the whole bundle is verified". An entry
  now holds only its members (`shape`). A JSON document holding a number
  outside the canonical domain matches no id, by rule rather than by
  whatever a JCS library writes for it.
- **`contract.schema.json` (F-17)** carries `maximum` on its `uint32`
  fields, so a standard validator bounds them.

**Changed: rules stated, by section.**
- **Grammar (§1, §2.2).** A key is parsed lexically, so a resource chunk is
  any plain chunk, `x-eth0` included (F-02); a ULID chunk's first character
  has no bound (F-01). Parameter types play no part in matching (F-03). A
  tie in rank, which E021 makes impossible in a contract, leaves the first
  listed template the winner.
- **The descriptor (§3.3).** A D-code table, with severity and counting
  (F-04), and the cascades and scope that only fixtures showed (F-05): an
  invalid `iface` is skipped and no longer counts for `declared_by`; a
  malformed fingerprint stops the entry before D004; an interface none of
  the given contracts declares is checked for syntax only. What the checker
  deliberately does not check is listed: `cause` against gates, R3's
  completeness, `declared_by` against the contract's `[requires]`, `params`
  values, `minor` (F-06). A lowered bound of 0 is D007 (F-06). The example
  held `imu` yet gave "no IMU" as a cause; it now shows a held capability
  with a `config` cause (F-07).
- **The error envelope (§5.2).** The decoding edges (F-08): exact encoding
  strings, member types, CBOR tags, `undefined` and indefinite lengths, a
  protobuf wire-type mismatch, a missing protobuf `message`, an empty
  protobuf `cause`.
- **JSON Schema (§7.3, §9.4).** A refused keyword's content is not walked,
  and `definitions` is refused (F-09). A `$ref`'s scheme is a `:` in its
  file part; its fragment is a JSON Pointer used as written, never
  percent-decoded, and `#anchor` is not one (F-10). In a bundle, a file part
  names an artifact by stem (F-11). A duplicate member is E029 (F-12). E024
  falls once per later file with a taken stem, which is not loaded, so
  `json:stem#Name` looks only in the first (F-13).
- **Protobuf (§9.4).** The well-known types compile from protoc 3.21.12's
  `include/` sources, which fix their ids; a listed file's own import of
  them resolves through the import roots first (F-14). A listed file is
  named by the first import root containing it, E029 under none; a nested
  message is a message, an enum is E023; listed files shadow the well-known
  types; every field carries its `json_name`, as protoc writes it (F-15).
- **Authoring and lints (§9.1–§9.3).** TOML datetimes of all four kinds, at
  any depth (F-18). E020 counts a key's value and its name apart, so one key
  can give two, and requirement annotations are checked (F-20). A profile
  without an interim table raises no W105 (F-21). E018 and E033 judge
  resolved values (F-22). A retention may have leading zeros, a rate may
  not; `gate = []` is no gate; `history = false` on an event is E019
  (F-23). "The first" of E021 and E022 is the template that sorts first,
  since TOML gives a table's keys no order; codes do not depend on it, and
  E021 does not take a resource out of W101 (F-39). The 1.1 list is closed,
  and "MAY accept later TOML" now reads "MAY be a parser of later TOML,
  provided it reports these as E000" (F-41). A set check runs only when
  every member loads (F-38). Integers are bounded by the schema's `format`,
  `uint64` to TOML's 2^63−1 (F-17).
- **Bundles (§9.6).** Base64 is RFC 4648 §4, padded, strict (F-24). A
  non-object entry or one without `kind` is `schema_kind`; an extra's
  `media_type` is informative; one id listed twice is one artifact (F-25).
  A `views.document` value is one id string; where a builder finds the
  documents is `views.v1`'s (#613), and a bundle file always writes all
  three members (F-26).
- **History and retention (§9.7).** The check's order: entries in bytewise
  order, stray root entries `directory` and not looked into, then per file
  `io`, the bundle tag, `interface`, `jcs`, the last two both reported
  (F-27). Retention identity is defined apart from the classifier's
  classes, with protoc's default `json_name` (F-28). A history root is
  configured, and a contract's history found by interface id; the examples
  share `examples/zk2/.history` (F-45).
- **Compatibility (§9.8).** "Both directions" means the writer and reader
  roles, never a swap (F-29). Resources pair by kind and template, so a
  toggled `explicit` pairs and a changed kind does not: the
  `kind_changed`/`token_changed` row, which could never fire, now reads
  `resource_removed` (F-30, F-40). An artifact no type reaches is not
  compared (F-31). `oneOf`/`anyOf`/`prefixItems` compare as written, in
  order, annotations dropped; "a branch added" is more branches; the
  asymmetry is deliberate (F-33). The renumber bullet no longer calls a
  renumber a deletion (F-34). Protobuf: messages are compared by structure
  from the named type, oneofs by name, enums by number and closed when the
  candidate's file is proto2; defaults, options other than `json_name`, and
  reserved names are not compared; warnings are names, deduplicated (F-35).
  Explicit presence is protobuf's definition, and a oneof move reports
  `oneof_changed` alone (F-43). `compat/README.md` says where the wrapper
  sits and when a case is `invalid`; only a candidate can be (F-44). F-36
  was resolved by 0.3, and needs nothing more.
- **Documents (F-37, F-42).** Fixture documents cite spec sections instead
  of Rust symbols and design documents. 0.3's case count is corrected (23
  new, not 11), and `compat/expect.json` no longer says the runner only
  checks that inputs load.

**Where the Python implementation's guess and the stated rule differ** (it
was right to guess; these are now decided): CBOR bytes read as base64, not
`bytes_hex` (F-08); a refused keyword's content is not walked (F-09); a
pointer is not percent-decoded (F-10); one key can give two E020 (F-20);
`gate = []` is no gate, not E016 (F-23); a non-object schema entry is
`schema_kind`, unknown entry members are `shape`, an extra's `media_type`
must be a string (F-25); a `views.document` list references nothing
(F-26); `interface` and `jcs` are both reported for one file (F-27); `$ref`
siblings are compared rather than flagged review (F-32); `oneOf` branches
compare in order, so a reordering is review (F-33); a proto2 default change
is compatible, and an enum is closed by the candidate's syntax only (F-35);
template order, not document order (F-39); a oneof move reports one rule
(F-43).

**Deliberately not changed:**
- **No existing expectation.** Every fixture of 0.4 keeps its expected
  value; the fixes changed behaviour only on inputs no fixture had.
- **R3 in the descriptor checker.** That a descriptor lists every
  requirement its contracts declare stays a scenario rule
  (`bindings.md §3`): the checker sees the contracts it is given, not the
  deployment's.
- **Extras.** The core still defines no source for extra documents. That is
  `views.v1`'s (#613), and the core says so instead of guessing.
- **Rule names stay informative.** `expect.json` pins classes and warning
  names. The reference keeps `kind_changed` and `token_changed` as
  unreachable branches; the table no longer lists them.
- **buf.** The protobuf cases added here were not measured with buf;
  `compat/README.md` says so.
- **Protobuf portability.** Canonical bytes stay portable only across
  encoders that match protoc 3.21.12. Naming that release's well-known
  sources pins their ids, and adds no new mechanism.

| Id | Resolution |
|---|---|
| F-01 | Rule stated (§1.2): no first-character bound; fixture `keys.json` |
| F-02 | Rule stated (§1.1): parsing is lexical, a resource chunk is any plain chunk; fixture `keys.json` |
| F-03 | Rule stated (§2.2): types play no part in matching; tie rule pinned in `templates.json` |
| F-04 | Rule stated: §3.3's D-code table |
| F-05 | Rule stated: §3.3's cascades and scope; `ok-unknown-revision` renamed `ok-unknown-interface` |
| F-06 | Rust fixed (D010 twice); rest stated, with what is not checked; fixtures `d007-zero`, `d010-twice`, `ok-unchecked` |
| F-07 | Example fixed (§3.3) |
| F-08 | Rust fixed (CBOR bytes, trailing bytes); edges stated (§5.2); 11 `errors/` cases |
| F-09 | Rust fixed (one schema-position walker); defined (§7.3); fixture `e037-nested` |
| F-10 | Rule stated (§9.4); fixture `e032-fragment` |
| F-11 | Rule stated (§9.4: by stem in a bundle); fixture `compat/contract/cross-file-ref-retyped` |
| F-12 | Rule stated (E029); fixture `e029-duplicate-member` |
| F-13 | Rule stated (E024 counting, first file wins); fixture `e024-stem` |
| F-14 | Rule stated (§9.4: protoc 3.21.12's `include/` sources) |
| F-15 | Rule stated (§9.4); fixtures `ok-protobuf-nested`, `e023-enum`, `e029-outside-include` |
| F-16 | Rust fixed (E000 beyond 64 bits); fixture `e000-integer-range` |
| F-17 | Rust fixed (`maximum` in the schema); `format` stated as a bound (§9.1); fixture `d000-minor-float` |
| F-18 | Rule stated (§9.1: four kinds, any depth); fixture `e020-twice` |
| F-19 | Rust fixed (E028 for `nan`/`inf` and integral floats); canonical domain stated (§9.5); fixtures `e028-float`, `e028-non-finite`, `e028-schema-float`, `ok-float-integral` |
| F-20 | Rule stated (E020 counting, requirements); fixtures `e020-twice`, `e020-requirement` |
| F-21 | Rule stated (§10, W105); fixture `ok-profile-without-vocabulary` |
| F-22 | Rule stated (resolved values); fixture `ok-resolved-operation` |
| F-23 | Rust fixed (W103); rest stated; fixtures `e023-encoding`, `ok-gate-empty`, `e019-history-false`, `ok-retention-leading-zero` |
| F-24 | Rule stated (§9.6); fixture `base64-unpadded` |
| F-25 | Rust fixed (entry members, unportable numbers); rest stated; fixtures `schema-entry-member`, `schema-entry-not-object`, `schema-data-big-integer`, `extra-entry-member` |
| F-26 | Left to a profile (`views.v1`, #613), with what a core implementation does meanwhile; value shape and `"extras": {}` stated; fixtures `no-extras` (bundle and history) |
| F-27 | Rule stated (§9.7's order); fixtures `wrong-directory-not-jcs`, `no-extras`, `stray-entries` |
| F-28 | Rule stated (§9.7 identity, default `json_name`) |
| F-29 | Wording fixed (§9.8: roles, not swaps) |
| F-30 | Superseded by F-40 |
| F-31 | Rule stated (unreferenced artifact); fixture `compat/contract/unreferenced-artifact-changed` |
| F-32 | Rust fixed (`$ref` siblings, booleans, required names); fixtures `ref-sibling-changed`, `boolean-property`, `required-without-property` |
| F-33 | Rule stated, Rust fixed (strip at schema positions); fixtures `oneof-reordered`, `oneof-annotation-only`, `oneof-property-named-title`, `anyof-add-branch` |
| F-34 | Wording fixed (§9.8 renumber bullet) |
| F-35 | Rules stated (§9.8); fixtures `move-between-oneofs`, `proto2-default-changed`, `packed-option`, `message-renamed`, `enum-value-renumbered`, `nested-type-added` |
| F-36 | Resolved by 0.3; nothing more |
| F-37 | Pointers replaced by spec sections (`conformance/README.md`, every `description`) |
| F-38 | Rust fixed (E035 against the first); set loading stated; fixture `sets/e036-first-declaration` |
| F-39 | Rule stated (template order); fixture `e021-shape-types` |
| F-40 | Wording fixed (§9.8 pairing); fixture `compat/contract/kind-changed` |
| F-41 | Wording fixed (§9.1: closed list, "a parser of later TOML") |
| F-42 | Fixed (0.3's count; `compat/expect.json`'s description) |
| F-43 | Rule stated (§9.8 presence); fixture `proto2-to-proto3` |
| F-44 | Rule stated (`compat/README.md`: wrapper location, `invalid`) |
| F-45 | Rule stated (§9.7 history root); `examples/zk2/README.md` states the examples' requirement |

## 0.4 — 2026-10-08: TOML 1.0, enforced (#607)

**Changed:** §9.1's "a contract MUST NOT need TOML 1.1" gains its lint.
Syntax that only TOML 1.1 has is E000, which stops the load:
- a newline, a comment or a trailing comma inside an inline table;
- the `\e` and `\xHH` escapes;
- a time without seconds.

The reference reader (the `toml` crate, which reads 1.1) now finds these
on `toml_parser`'s event stream. Before, a 1.1-only contract loaded in
Rust and failed in a 1.0 reader such as Python's `tomllib`. Five fixtures
(`contracts/e000-toml11-*`) cover one construct each, and `tomllib`
refuses all five. §9.2's E000 row now reads "not TOML 1.0".

**Deliberately not changed:**
- A reader MAY still accept later TOML. The rule binds what a contract
  needs, not what a reader parses.
- Bare keys outside ASCII are not on the list: TOML 1.1.0 did not adopt
  them, and both readers refuse them already.
- In `e000-toml11-time`, the time stops the load as E000 before E020 is
  ever reached. E020 would have refused it anyway, as an annotation
  datetime.

## 0.3 — 2026-10-08: the classifier's rule set (#618)

**Changed:** §9.8 states every rule the classifier applies, each with its
name, in six tables: interface, resources, delivery, operations, roles and
types.
- **Rows that were missing,** chiefly the reverse directions:
  - required → optional, breaking;
  - `idempotent` false → true, review;
  - `fanout` forbidden → allowed, `replies` many → one, a role removed: all
    compatible;
  - `congestion` either way, review;
  - an optional resource added, compatible; a required one, breaking;
  - `deprecated` added, compatible.
- **Protobuf rules added:**
  - a reserved number reused, breaking;
  - explicit presence toggled, review;
  - proto2 `required`: added, deleted or toggled, all breaking.
- **JSON Schema rules added:**
  - `const` changed, and a required property removed: breaking;
  - `additionalProperties` or `items` gaining or losing a schema: review;
  - toggling them between absent, `true` and `false`, and reordering
    `enum` values: compatible.
- **`compat/` is evaluated.** 70 cases, 23 of them new since 0.2: 12
  contract, 5 JSON Schema and 6 protobuf. (This entry said "11 of them
  new" until 0.5 corrected it.) §0's caveat on `[F: compat/]` is removed.
- **`compat/README.md`** gives the case layout and the departures from
  `buf breaking --use WIRE`, measured in both orders.

**Deliberately not changed:**
- No existing case changed its class.
- Rule names stay informative: `expect.json` pins classes and warning
  names, so a second implementation is free to name its findings.

## 0.2 — 2026-10-08: U23 settled

**Changed:** §8.5's attachment rule gains a second shape.
- **The shape:** a far router in a `gateway.south` region of the near
  router, with the `@zk` deny. The near router keeps its own clients and
  peers south.
- **Measured** (spike S3, U23 addendum): 506 B and no denied key string for
  200 tokens, against 11.3 KB and all 201 router to router. Data crosses.
- **Also updated:** §12's faces row, constrained.md §6, and §13's U23 row.

**Deliberately not changed:**
- The client attachment stays a valid shape.
- A router-to-router link with a deny is still not recommended: the deny
  hides the declarations, but they cross anyway.

## 0.1 — accepted 2026-10-08 (#606)

The first accepted version, after two independent review passes (PR #639).
The second pass reproduced every non-protobuf canonical fixture byte for
byte from §9 alone.

**Decided at acceptance, and folded into 0.1:**
- **U22:** a deployment-configured tokenless set. Interfaces every service
  implements carry no interface token, and the descriptor records it with
  `"token": false` (§8.1, §3.3).
  - *Deliberately not done:* a contract-level flag, which would have
    changed every canonical form and fingerprint.

**Recorded as open:**
- **U23,** to be measured (`gateway.south`) before `link.v1`: settled in 0.2.
- **A TOML 1.1 lint:** done in 0.4.
- **Extras in the reference bundle builder.**
- **`compat/`,** evaluated once the classifier (#618) lands: done in 0.3.
