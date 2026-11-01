# `catalog/`: the deployment catalog

These fixtures check [`core.md`](../../core.md) §8.6. Each file states its
format in its `description` member. They were written by hand from the
text, and are checked, never blessed.

| File | Input → expected |
|---|---|
| [`documents.json`](documents.json) | A catalog's text → its id, or the reader's refusal: the first that applies, in §8.6's order. |
| [`ids.json`](ids.json) | An accepted catalog's text → its JCS bytes, their sha256 and its id. Spacing, member order and escapes move none of them. |
| [`succession.json`](succession.json) | Named catalogs. Two of them → whether the second succeeds the first, or the checker's refusal (`successions`). The catalogs a reader holds, the id an owner names, and an index → the epoch and the entry, or the reader's refusal (`reads`). |

**What they cannot reach.** A catalog whose own id is one of its
lineage's is refused as `lineage`. Making one takes a 32-bit collision, so
no case holds one.

**Where the fingerprints come from.** The entries name `modem.v3`,
`config.v1` and `health.v1` at their revisions in `examples/zk2/` and
`profiles/`. The others are made up. No fixture reads a contract: a
catalog lists a fingerprint, and the reading checks only its form.
