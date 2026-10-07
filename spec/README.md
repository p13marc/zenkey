# zk2 spec

The language-neutral half of zk2: what any implementation, in any language,
reads and must agree on. The design of record is
[`docs/zk2/architecture.md`](../docs/zk2/architecture.md) (r3.3). The
normative spec text arrives with #606, after the spike report and r4
(#605).

| Path | What it is | Source |
|---|---|---|
| [`contract.schema.json`](contract.schema.json) | JSON Schema (2020-12) of the contract authoring format, draft 1. It is generated from `zenkey-model`'s authoring types and checked in CI. | `zenkey-model/src/authoring.rs` |
| [`conformance/`](conformance/) | Fixtures: keys, slugs, templates, contract lints, canonical forms, fingerprints and bundles. Seeded by `zenkey-model` (#608), extended by #607. | `zenkey-model/tests/conformance.rs` |

To regenerate both after an intended change (then read the diff, because a
bless claims that every changed expectation is right):

```bash
ZK2_BLESS=1 cargo test -p zenkey-model --test conformance
```
