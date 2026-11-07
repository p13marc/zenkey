# zk2 conformance fixtures

Seeded by `zenkey-model` (#608), completed by #607. The normative text they
check is [`../core.md`](../core.md), and every meaning a fixture relies on is
defined there. An implementation conforms when it produces every `expect`
here. The reference implementation and the Python implementation run the
same files.

| File | Input → expected |
|---|---|
| [`keys.json`](keys.json) | A key → its parse (form, system, service, interface, kind token, resource chunks; or the control-key fields), or `null` when the key is refused (§1.1, §1.2). Every accepted key builds back to the same string. |
| [`slugs.json`](slugs.json) | A value → its canonical chunk (`slug`), which unslugs back to the value. A chunk → its value, or `null` when it is not a canonical slug (`unslug`) (§1.4). |
| [`templates.json`](templates.json) | Templates + resource chunks → the winning template (match, then rank, most literal first, §2.2) and its unslugged bindings, or `null`. |
| [`contracts/`](contracts/) | `<stem>.toml` → the sorted diagnostic codes and the fingerprint (`expect.json`), plus the canonical JCS bytes of a valid contract (`<stem>.canonical.json`) (§9.1–§9.5). |
| [`bundles/`](bundles/) | `<name>.bundle.json` → verified with its fingerprint, or refused with a tag (`expect.json`) (§9.6). Built from `seed.toml`, and the packed ones from `contracts/ok-packed.toml`. |
| [`sets/`](sets/) | A directory of contracts → the set checks' codes (E035, E036; §9.2). Hand-written. |
| [`history/`](history/) | A history root → its problems as `[at, tag]` (§9.7). Hand-written. |
| [`descriptors/`](descriptors/) | A descriptor document → its `D…` codes, checked against `contracts/nav.v2.toml` (`expect.json`) (§3.3). |
| [`errors/`](errors/) | The bare wire's error envelope and its Zenoh encoding → the decoded envelope, or a refusal tag (`cases.json`, §5.2). Written by an encoder independent of the reference implementation. §7.5's error form (`forms.json`): every error byte → its code, its cause and its two bits, or the refusal; an error and its stamp → the reply error's bytes, a message over 255 bytes cut, or the writer's refusal; a reply error's Encoding and payload → the error it carries, or the refusal, which makes it the transport's, a router's `Timeout` among them. Written by an independent reading of the text, checked, never blessed. |
| [`budget/`](budget/) | A contract's `cardinality` and each instance's stated one → the bound (`bounds.json`); a kind, a bound and one reading (an owner's GET, or a window's members and instants, how long it listened, whether it lost no delivery, whether the owner was present throughout) → the population verdict and its reason (`population.json`); an event's rate and a window's occurrences, with the same window premises → the rate verdict and its reason (`rate.json`) (§2.7; the readings and verdicts are [`observer.md`](../observer.md) §5's); a resource's declared QoS, the reader's own loss, what it saw of its own receive path (its ticker's greatest gap against the bound, and whether a transport of its session closed) and miss detection (declared, used, misses detected) → whether the window lost no delivery, and why, and `wait_before_close`, the lease and the keep-alive count → the stall bound (`lossless.json`). Written by hand from the text, checked, never blessed. |
| [`seed/`](seed/) | Two Zenoh timestamps → their order, time then the id's bytes as zenoh compares them; one key's arrivals, each a stamp or none and a kind → which a late joiner keeps (`merge.json`, §4.2). A key's kind and two in-band stamps, each a run and a seq → their order, plain for state and events, serial for a stream, and unordered across two runs; one key's arrivals, each with an in-band stamp, a Zenoh timestamp, both or neither → which a tool's merge keeps, and for state which a consumer accepts (`inband.json`, §4.2, §4.3). Written by hand from the text, checked, never blessed. A reader's collections, what it holds when it asks a GET, and the GET's snapshots' pages, per-key replies and live samples after it → what it holds, and each snapshot's pages, completeness and omissions (`snapshot.json`, §4.2): the omission rule, asof and pages. Written by an independent reading of the text, checked by a second, never blessed. |
| [`envelope/`](envelope/) | One byte → CH's form, run width and tag (`headers`). An envelope's or a call's form's bytes in hex, and their sample: a put's payload (a snapshot's among them), a delete's attachment, or a call's query, reply or reply error → the parse (form, run width, run, seq, tag, the record in hex, and an error's code, cause, message and detail) or the refusal's token, the first in §7.5's order (`cases`). Every header and every accepted envelope writes back to its bytes (`vectors.json`, §7.5). A key and its Encoding → whether the sample carries an envelope, and a key and a put's payload → how it reads, `key_kind` among the refusals, a frame's and a snapshot's (`carries.json`). Written by hand from the text, checked, never blessed. The operations of `contracts/ok-packed-operation.toml` → each form's CH by their types' tags; an operation, a form and its record or error → the payload, or the writer's refusal; an operation, a sample and a payload → the record read as the operation's type, or the refusal (`calls.json`, §5.1, §5.2). Written by an independent reading of the text, checked by a second reader, never blessed. A key, its CH and its record's size, the revisions held (bundles of modem.v3's history) and the catalog given → the revision a tool reads the sample with and the step that chose it, or why none is left, and what it then shows: a packed record by its tag and size, a sniffed one, or an envelope (`tags.json`, [`observer.md`](../observer.md) §7.3, §7.5). Written by hand from the text, checked by a second reading over the bundles, never blessed. |
| [`heartbeat/`](heartbeat/) | A contract revision → its collections in order, each one's member widths and horizon, and its fingerprint and tag (`layouts.json`). A layout and a value → the body's bits, pad and bytes; a layout and bytes → the value, or the refusal, the first in §7.5's order (`bodies.json`). Members, each a key, a run and a seq → each one's value and the collection's digest (`digests.json`) (§7.5). Written by an independent reading of the text, checked, never blessed. |
| [`packed/`](packed/) | One packed document → each type's layout form, its sha256 and tag, each item's width and start, and its shortest and longest record (`layouts`). A type, a deployment epoch and a value → the record's bits, pad and bytes, or the writer's refusal (`writes`). A type, a tag and bytes → the JSON reading, or the reader's refusal, the first in §7.6's order (`reads`) (§7.6). Written by an independent reading of the text, checked, never blessed. |
| [`snapshot/`](snapshot/) | A page of a collection's snapshot → its bytes and their size, or the writer's refusal (`writes`). A page's bytes → the page: its run, asof, time, index and collection, then its live entries and tombstones, each an id, a stamp and, for a live entry, its confirmation and record; or the reader's refusal, the first in §7.5's order (`reads`). A snapshot and a page size → its pages, as the paging rule cuts them, or the writer's refusal (`pages`) (`bodies.json`, §7.5). Written by an independent reading of the text, checked by a second, never blessed. [`snapshot/README.md`](snapshot/README.md) says what they cannot reach. |
| [`frame/`](frame/) | A record type and a `frame` table → the frame form, its sha256 and tag, the count's and the offset's widths, and the span (`layouts.json`). A layout and a frame's value → the body's bits, pad and bytes, or the writer's refusal (`writes`). A layout, a tag and a body → the value, or the reader's refusal, the first in §7.7's order (`reads`) (`bodies.json`, §7.7). One key's frames → each record's field states (`apply.json`, §7.7's reader). Written by an independent reading of the text, checked, never blessed. [`frame/README.md`](frame/README.md) says what they cannot reach. |
| [`catalog/`](catalog/) | A deployment catalog's text → its id, or the reader's refusal, the first in §8.6's order (`documents.json`). Its JCS bytes and id (`ids.json`). Two catalogs → whether the second succeeds the first, and the catalogs a reader holds, an id and an index → the epoch and the entry, or the refusal (`succession.json`). Written by hand from the text, checked, never blessed. [`catalog/README.md`](catalog/README.md) says what they cannot reach. |
| [`scope/`](scope/) | The contracts in hand, the addresses across a face, a face policy and its face or none, what the tool reads (a subscription or a GET) and the selectors given → the selectors it declares, the selectors given it refuses, or the plan's refusal (`faces.json`, [`observer.md`](../observer.md) §7.5). Computed by a reading of the text over zenoh-modem's contracts and faces, checked by a second reading over `link.v2`'s `plan.json` and the bundles, never blessed. |
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
