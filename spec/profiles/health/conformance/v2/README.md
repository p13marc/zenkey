# `health.v2`'s fixtures

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
| [`schema.json`](schema.json) | §3 | The standard packed schema's artifact id, and its three types' tags |
| [`records.json`](records.json) | §2.3, §2.5, §2.6, §3 | A status, a check and a fault written as records, their bits and bytes, and their puts; records read back or refused; a `clock_ahead` fault's argument from the offset measured |
| [`tables.json`](tables.json) | §2.10, §4 | The profile's three tables; a contract's tables read from its resolved annotations; an instance's from the revisions its descriptor names; a word written as a number and read back as a name; a table's growth |
| [`snapshots.json`](snapshots.json) | §2.2, §2.4 | A late joiner's reading of the status's and the checks' snapshots, entry by entry, and whether the reading is one instant |
| [`judgements.json`](judgements.json) | §2.1, §2.4, §2.6 to §2.8, §2.11, §2.12 | "Is this service healthy?", from presence, the descriptor, the status's freshness and the records read; and the major a reading is judged under |
| [`clock.json`](clock.json) | §2.5, §5 | A heartbeat's bytes read as an instant, ahead, unknown or not read; and "is this service's clock ahead?" from what one reader heard |

**What each computation was checked against.** The first script's packed
records agree with the core's
[`packed/vectors.json`](../../../../conformance/packed/vectors.json) for
the kinds these types use. Its digests agree with
[`heartbeat/digests.json`](../../../../conformance/heartbeat/digests.json),
and its pages with the writes of
[`snapshot/bodies.json`](../../../../conformance/snapshot/bodies.json).
The schema's artifact id is the one the reference's `zk2 contract
bundle` states for the standard contract.

**`health.v1`'s fixtures** ([`../`](../)) still hold what this text keeps
by reference: a tool's roll-up (`rollups.json`), which reads verdicts and
levels of either major. `health.v1`'s `judgements.json` judges a service
whose descriptor lists `health.v1`.
