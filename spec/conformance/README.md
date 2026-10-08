# zk2 conformance fixtures

Seeded by `zenkey-model` (#608), completed by #607. The normative text they check is [`../core.md`](../core.md). An
implementation conforms when it produces every `expect` here. The Rust
runner is `zenkey-model/tests/conformance.rs`; the Python implementation
(#609) runs the same files.

| File | Input → expected |
|---|---|
| [`keys.json`](keys.json) | A key → its parse (form, system, service, interface, kind token, resource chunks; or the control-key fields), or `null` when the key is refused. Every accepted key builds back to the same string. |
| [`slugs.json`](slugs.json) | A value → its canonical chunk (`slug`), which unslugs back to the value. A chunk → its value, or `null` when it is not a canonical slug (`unslug`). |
| [`templates.json`](templates.json) | Templates + resource chunks → the winning template (most-literal-first, r3.3 D1) and its unslugged bindings, or `null`. |
| [`contracts/`](contracts/) | `<stem>.toml` → the sorted diagnostic codes and the fingerprint (`expect.json`), plus the canonical JCS bytes of a valid contract (`<stem>.canonical.json`). |
| [`bundles/`](bundles/) | `<name>.bundle.json` → verified with its fingerprint, or refused with a tag (`expect.json`). Built from `seed.toml`. |
| [`sets/`](sets/) | A directory of contracts → the set checks' codes (E035, E036). Hand-written. |
| [`history/`](history/) | A `.history` root → its problems as `[at, tag]`. Hand-written. |
| [`descriptors/`](descriptors/) | A descriptor document → its `D…` codes, checked against `contracts/nav.v2.toml` (`expect.json`). |
| [`errors/`](errors/) | An error envelope and its Zenoh encoding → the decoded envelope, or a refusal tag. Written by an encoder independent of the reference implementation. |
| [`compat/`](compat/) | Old/new payload schemas, contract pairs, and transitive histories → the FULL_TRANSITIVE class (`expect.json`). Evaluated by the classifier (#618); layout and the departures from `buf` in [`compat/README.md`](compat/README.md). |

## What the contract fixtures pin

- **Codes, not messages.** The codes are listed with their meaning in
  `zenkey_model::diag::CODES`; `E…` makes a contract invalid, `W…` does not.
  The file stem names the code a fixture exercises.
- **Every finding counts.** A fixture's `codes` is every diagnostic, sorted,
  with repeats. To keep that independent of the order lints run in:
  - a template that does not parse (E010) stops the other checks of its resource;
  - a schema file that fails to load (E029) suppresses E023 for its schema kind;
  - shapes (E021) and `deprecated` (E031) are checked over every template that parses;
  - overlaps (W101) need types, so they are checked over resources without errors.
- **Files load from the fixture's directory**, without the file-name
  convention check (W107).
- **Canonical bytes are portable for JSON Schema and raw types.** A
  protobuf artifact is the `FileDescriptorSet` that `protox` encodes, and
  another compiler may encode the same file differently. So a contract with
  protobuf types fingerprints the same across implementations only when
  they share that encoder. Bundles carry the bytes, so *verifying* a bundle
  is portable for every kind (r3 §3.11: bundles are built once by the
  contract's CI and only verified elsewhere).

## The canonical form (draft 1)

`zenkey_model::canonical` is the reference; #606 states it normatively.
- **Fully explicit:** every field of every resource is present, `null` when
  absent, with defaults expanded (D2).
- **Documentation and `minor` are excluded.**
- **Format tag:** `"format": "zk2-contract/draft-1"`.
- **Ordering:** resources sorted by (kind token, template); gates and
  required resources sorted and deduplicated; objects ordered by JCS.
- **Every schema artifact is listed** as `{id, kind, name}`, so that a change to any listed
  file, even one that is only `$ref`'d, changes the fingerprint.
- **Fingerprint:** `sha256:` + the hex sha256 of the JCS bytes (RFC 8785).
- **Restrictions:** every string is printable ASCII (E027), and every
  integer is within ±(2^53−1) (E028).
