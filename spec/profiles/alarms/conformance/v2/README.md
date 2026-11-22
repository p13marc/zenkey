# `alarms.v2`'s fixtures

The fixtures of [`../../v2.md`](../../v2.md). Each file states its format
in its `description`. Every expectation in them was computed twice, by
two scripts of the text's drafter that share no code. The first read the
text into code and wrote the files. The second was written from the
finished text, and computed each expectation again from the files'
inputs. It reads where the first writes: it parses each heartbeat and
each snapshot, and types the record's layout in from the text's listing.
The two agree on every case. Both are kept out of the repository, and no
file here was blessed.

Both scripts are one drafter's. So they catch a slip of computation, and
not a reading of the text that both share.

No implementation has run them yet.

| File | Rules | What it pins |
|---|---|---|
| [`schema.json`](schema.json) | §3 | The two standard schemas' artifact ids, and the packed type's tag |
| [`records.json`](records.json) | §2.4, §3 | An `Alarm` written as a record, its bits and bytes, and its put; what a writer refuses; records read back, refused by core §7.6, or undecodable by their `key` |
| [`tables.json`](tables.json) | §2.4, §4 | A contract's table read from its resolved annotations, with each finding; an instance's from the revisions its descriptor names; an alarm written as a rule's index, label codes and an argument, or by words; a record read back into words, or unnamed; a table's growth |
| [`keys.json`](keys.json) | §2.2, §2.12, §4 | The alert key of an alarm's words, as digits and as the integer a record carries; the alert key a table's rule derives; a key read as a slot; the collection a reference is read from |
| [`slots.json`](slots.json) | §2.2, §2.6, §2.8, §2.9 | The slot each identity takes, event by event: a re-raise, a resolved slot retaken, the bound's refusal, a restart's adoption; and which of two slots a reader takes for one identity |
| [`words.json`](words.json) | §2.4, §5 | Whether a reader takes a slot's words as its alarm's, and why not; what a tool shows as words and records arrive |
| [`transitions.json`](transitions.json) | §2.6, §2.16 | One slot's samples read by identity: `rebound`, `replaced` and `dropped` among them, and what a notifier reports |
| [`confirmations.json`](confirmations.json) | §2.7, §3 | A heartbeat's bytes read over a reader's view: the live count beside the firing count, the slots confirmed by rank, and each slot's answer past the horizon; a snapshot's page read entry by entry |
| [`acks.json`](acks.json) | §2.5 | The `at` each act writes, with its one-second step and its absence; whether an acknowledgement applies; an `alarms.v1` timestamp adopted |
| [`conclusions.json`](conclusions.json) | §2.1, §2.11, §2.13 to §2.15, §5 | The major a descriptor is read by; "is this alarm firing?" and "is it past its horizon?"; whether an owner's alarms are established, by one snapshot; "is anything firing?" |

**What each computation was checked against.**
- The first script's packed records, snapshot pages, heartbeat bodies
  and digests agree with the core's
  [`packed/vectors.json`](../../../../conformance/packed/vectors.json),
  [`snapshot/bodies.json`](../../../../conformance/snapshot/bodies.json),
  [`heartbeat/bodies.json`](../../../../conformance/heartbeat/bodies.json)
  and
  [`heartbeat/digests.json`](../../../../conformance/heartbeat/digests.json),
  for the kinds this profile uses. Its alert keys agree with
  `alarms.v1`'s [`keys.json`](../keys.json).
- The two schemas' artifact ids are those the reference's `zk2 contract
  bundle` states for the standard contract.
- The second script's steps for one alarm and for "anything firing"
  answer `alarms.v1`'s own [`conclusions.json`](../conclusions.json) as
  it expects: its 22 `any_firing` cases, and the 19 readings that no
  freshness verdict decides.
- Each of the 473 cases was changed once, and the second script
  disagreed with every changed one.

**`alarms.v1`'s fixtures** ([`../`](../)) still hold what this text
keeps by reference: the recipe and its sanitizing step (`keys.json`),
and "anything firing" over verdicts of either major
(`conclusions.json`). `alarms.v1`'s other cases judge a service whose
descriptor lists `alarms.v1`.
