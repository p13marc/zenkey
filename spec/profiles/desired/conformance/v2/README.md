# `desired.v2`'s fixtures

The fixtures of [`../../v2.md`](../../v2.md). Each file states its format
in its `description`. Every expectation in them was computed twice, by
two scripts of the text's drafter that share no code. The first read the
text into code and wrote the files. The second was written from the
finished text, and computed each expectation again from the files'
inputs. The two agree on every case. Both are kept out of the
repository, and no file here was blessed.

No implementation has run them yet.

| File | Rules | What it pins |
|---|---|---|
| [`schema.json`](schema.json) | §3 | The standard packed schema's artifact id, and the `Report`'s tag |
| [`reports.json`](reports.json) | §2.10, §3 | A report written as a record, its bits and bytes, and its put; records read back, refused, or read with their findings; a refusal's place and line written as indices, and read back as words |
| [`order.json`](order.json) | §2.4, §2.6 to §2.10 | One target's steps: versions of one run and of several, an author's restart, sixteen remembered runs, deletes, a snapshot with no entry under the held run and under another, last-known, a call, a restart, and the report after the last |
| [`watch.json`](watch.json) | §2.5 | The watch: each heartbeat's and each answer's decision, and the section the target expects after it |
| [`declarations.json`](declarations.json) | §2.2, §2.10 | A desired resource's declaration with its horizon, and a report resource's with its two tables |
| [`slices.json`](slices.json) | §2.10 | Which reply is a role's report, found by `desired.report` |
| [`conclusions.json`](conclusions.json) | §2.17, §5 | A tool's four answers, the versions compared by run and seq |

**What each computation was checked against.** The first also reads
`desired.v1`'s [`declarations.json`](../declarations.json), the core's
[`heartbeat/digests.json`](../../../../conformance/heartbeat/digests.json)
and, for its packed records, the core's
[`packed/vectors.json`](../../../../conformance/packed/vectors.json). It
agrees with each.

**`desired.v1`'s fixtures** ([`../`](../)) still hold the rules this text
keeps by reference: the binding (`bindings.json`), and v1 §2.7's
comparison of two documents and where a refusal points (`order.json`).
