# zk2 conformance fixtures

Seeded by `zenkey-model` (#608), completed by #607. The normative text they
check is [`../core.md`](../core.md), and every meaning a fixture relies on is
defined there. An implementation conforms when it produces every `expect`
here. The reference implementation and the Python implementation (#609) run
the same files.

| File | Input → expected |
|---|---|
| [`keys.json`](keys.json) | A key → its parse (form, system, service, interface, kind token, resource chunks; or the control-key fields), or `null` when the key is refused (§1.1, §1.2). Every accepted key builds back to the same string. |
| [`slugs.json`](slugs.json) | A value → its canonical chunk (`slug`), which unslugs back to the value. A chunk → its value, or `null` when it is not a canonical slug (`unslug`) (§1.4). |
| [`templates.json`](templates.json) | Templates + resource chunks → the winning template (match, then rank, most literal first, §2.2) and its unslugged bindings, or `null`. |
| [`contracts/`](contracts/) | `<stem>.toml` → the sorted diagnostic codes and the fingerprint (`expect.json`), plus the canonical JCS bytes of a valid contract (`<stem>.canonical.json`) (§9.1–§9.5). |
| [`bundles/`](bundles/) | `<name>.bundle.json` → verified with its fingerprint, or refused with a tag (`expect.json`) (§9.6). Built from `seed.toml`. |
| [`sets/`](sets/) | A directory of contracts → the set checks' codes (E035, E036; §9.2). Hand-written. |
| [`history/`](history/) | A history root → its problems as `[at, tag]` (§9.7). Hand-written. |
| [`descriptors/`](descriptors/) | A descriptor document → its `D…` codes, checked against `contracts/nav.v2.toml` (`expect.json`) (§3.3). |
| [`errors/`](errors/) | An error envelope and its Zenoh encoding → the decoded envelope, or a refusal tag (§5.2). Written by an encoder independent of the reference implementation. |
| [`compat/`](compat/) | Old/new payload schemas, contract pairs, and transitive histories → the FULL_TRANSITIVE class (`expect.json`) (§9.8). Layout and the departures from `buf` in [`compat/README.md`](compat/README.md). |

## What the contract fixtures pin

- **Codes, not messages.** The codes and their meanings are the table of
  `core.md` §9.2 (and §3.3 for `D…`); `E…` makes a contract invalid, `W…`
  does not. The file stem names the code a fixture exercises; each fixture
  file opens with a comment saying what it pins, where one is needed.
- **Every finding counts.** A fixture's `codes` is every diagnostic, sorted,
  with repeats. To keep that independent of the order lints run in, §9.2's
  cascades apply:
  - a template that does not parse (E010) stops the other checks of its resource;
  - a schema file that fails to load (E029) suppresses E023 for its schema kind;
  - shapes (E021) and `deprecated` (E031) are checked over every template that parses;
  - overlaps (W101) need types, so they are checked over resources whose own checks found no error;
  - W103 is judged on resolved types only.
- **Files load from the fixture's directory**, without the file-name
  convention check (W107).
- **Canonical bytes are portable for JSON Schema and raw types.** A
  protobuf artifact is the `FileDescriptorSet` that protoc 3.21.12 writes
  (§9.4, §9.5), and another compiler may encode the same file differently.
  So a contract with protobuf types fingerprints the same across
  implementations only when they share that encoding. Bundles carry the
  bytes, so *verifying* a bundle is portable for every kind (§9.5: bundles
  are built once by the contract's CI and only verified elsewhere).

## The canonical form (draft 1)

`core.md` §9.5 states it normatively. In brief:
- **Fully explicit:** every field of every resource is present, `null` when
  absent, with defaults expanded (§9.3).
- **Documentation and `minor` are excluded.**
- **Format tag:** `"format": "zk2-contract/draft-1"`.
- **Ordering:** resources sorted by (kind token, template); gates and
  required resources sorted and deduplicated; objects ordered by JCS.
- **Every schema artifact is listed** as `{id, kind, name}`, so that a change to any listed
  file, even one that is only `$ref`'d, changes the fingerprint.
- **Fingerprint:** `sha256:` + the hex sha256 of the JCS bytes (RFC 8785).
- **Restrictions:** every string is printable ASCII (E027), and every
  number is in the canonical domain: an integer within ±(2^53−1), a float
  finite and, where JCS writes it as an integer, within the same range
  (E028).
