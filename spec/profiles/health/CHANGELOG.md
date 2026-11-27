# `health` changelog

Versions of the texts of [`v1.md`](v1.md) (`health.v1`) and
[`v2.md`](v2.md) (`health.v2`). Each entry records what changed, what
deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)). An entry
headed `health.v2 0.N` is `v2.md`'s, and one headed `0.N` alone is
`v1.md`'s.

## health.v2 0.3 — 2026-11-27: zk2py's rounds 120 and 123: which owner heartbeats answer the clock, a record whose time no catalog in hand dates, `not_used` under `health.v1`, and what a linter reports (#986)

zk2py's round 120 read 0.1 cold, and its round 123 ran it against the
reference (F-593 to F-602, F-610, F-611). 0.2 resolved F-595, F-596 and
F-602. This version answers the rest. F-597 is core §2.6's, answered in
core 0.115.

**Decided where the two implementations differ.**
- **A record whose time no catalog in hand dates (F-610).** Core §8.6
  refuses such a time, `unknown_catalog`. §2.6 now says the record is
  refused whole, as core §7.6 refuses a record for one field. So its
  status is unknown, `undecodable`. Both readers do so. A record with no
  time needs no catalog, and is read. zk2py reads it. The reference
  refuses every record of such an owner, and moves (the code half).
- **Owner heartbeats, and a reader with no catalog (F-598).** An owner
  heartbeat counts whatever became of its groups, as `freshness.v2` §2.5
  says of a read one. Its time is read with no catalog. But such a
  reader need not subscribe to an owner heartbeat key. zk2py subscribes
  and answers yes, `ahead`. `zenctl health` does not, and answers
  `nothing_heard`. Both are now as §2.5 says, and neither moves.
- **The `link` class (F-598)** does not count. A reader subscribes to
  the status's class, `host`. 0.2 already read so.

**Corrected, with no rule changed.**
- **`not_used` (F-593).** "A contract that loads cannot hold it (core
  E020)" was wrong of a contract that lists `health.v1`. That contract
  loads, with no W105 (core Appendix D), and holds no table. The
  sentence now says so, as #984's `alarms.v2` draft says of a contract
  that lists `alarms.v1`, where W105 does read.
- **What a linter reports (F-594).** In a contract that lists
  `health.v2`, `value` and `disagrees` are a linter's errors, as
  `link.v2`'s `packed_required` is. The contract still loads. The
  reference's `zk2 contract lint` does so, and zk2py's linter moves.
  `not_used` under `health.v1` draws no core lint, and a linter MAY warn
  of it.
- **"In every other case" (F-600)** now says "under this major". A
  descriptor that lists `health.v1` alone is asked `health.v1`'s
  question, as §2.12 and `clock.json`'s case 29 say. Both readers read
  it so.
- **§9's T (F-611).** Expected 4 asks what a reader of records shows. T
  and T2 are such readers, as §9 is the runtime's. A health tool
  answers questions of now, and need not show a status since replaced.
  The reference's `zenctl health` meets Expected 4 but for that.

**The fixtures (F-599).**
- `tables.json`, instance 9: an empty list beside another list is
  `differ`. Both owners wrote `out_of_range` so, live.
- `judgements.json`, case 60: no status read reaches step 3 with no
  observation, unobservable.
- Each was written by hand from the text, and read against the
  reference's model. Their counts move in the reference's tests (the
  code half).
- **Not added.** `clock.json`'s owner heartbeat payloads, and a
  `records.json` read under a catalog the reader does not hold. Each
  needs its runner to read a new shape (the code half).

**0.1's "Not verified" (F-601).** It said "The contract is not yet in
`spec/profiles/.history/`". It was published there with Q1:
`.history/health.v2/b447f613….bundle.json`, as §3 says. 0.1's entry
stands as a record.

**Deliberately not changed.** No fixture's reading, and no scenario
under a catalog. The header is still written against core 0.113 and
`freshness.v2` 0.12.

## health.v2 0.2 — 2026-11-27: editorial: three scenario texts zk2py's round 120 found wrong, and who runs the scenarios, no rule changed (#986)

0.1's `scenarios-v2.md` had three steps no implementation could meet as
written. zk2py's round 120 ran them first (F-595, F-596, F-602), and the
reference's Q2 corrected them, with no entry at the time. This is that
entry.

**What changed, in `scenarios-v2.md` alone.**
- **§8's runs name what they deny of the streams (F-596).** The setup's
  face denies presence alone, and run B added a deny of the state. A
  heartbeat and a fault are stream samples, so on those denies both
  crossed: run B's "nothing crosses", run C's "no heartbeat crosses" and
  run A's "no fault crosses" could not hold. Run A now denies the faults'
  key, run B the heartbeats (`zk2/vehicle-01/*/freshness.v2/stream/**`)
  and the faults' key, and run C the heartbeats.
- **§9's tools subscribe before step 1 (F-602).** T and T2 read at step 4,
  after step 2 replaced step 1's status, and no read answers a fault, yet
  Expected 4 had them show both. They now subscribe to both owners'
  `health.v2/**` before step 1, and each shows what it heard and what it
  read.
- **§1's and §8's byte figures state their run (F-595).** "12 bytes: CH
  `0x03` or `0x13`" is a 16-bit run's figure alone. A 32-bit run's put is
  14 bytes, as `v2/records.json` has it. Each figure now says which run it
  is under.
- **The intro says who runs them.** The reference runs §1 to §5 and §9 in
  `zenkey/tests/profile_health_v2.rs`, and §6 to §8 in
  `zenctl/tests/live_zk2.rs`, every step and Expected.

**Deliberately not changed.**
- **No rule** of `v2.md`, and no fixture of `conformance/v2/`. The
  scenarios now say what the rules already did: a deny of the state
  leaves the heartbeats and the faults crossing (core §8.5), and a later
  read answers no status since replaced and no fault (§2.4).
- **No scenario runs under a catalog.** The conventions still give the
  owners none, so every time counts from 0. The reference also checks an
  owner under a catalog's later epoch, outside the scenarios: its records'
  times and its snapshots' time count from it, and a reader given the
  catalog reads them so. `v2/records.json` already holds writes and reads
  under such an epoch.
- **The header** is still written against core 0.113 and `freshness.v2`
  0.12.

## health.v2 0.1 — 2026-11-22: a new major: packed records sent on change only, words by table, and a clock question a heartbeat answers alone (#986)

**What it saves is small, and that is said first.** `health.v1`'s
revision 1.1 already stopped putting its status again: its owner's
heartbeat confirms it (#978). So on W2, four status changes a day cost
232 B under `health.v1` and 148 B here. `health.v2` saves about 84 B a
day. Health's periodic bytes are its heartbeat's, 118 KB a day, which
this major does not touch.

**What it buys is one wire.**
- **Packed records of one size.** A status and a check are 6 bytes, a
  fault 8, where `health.v1`'s measured 27, 37 and 50 before their
  envelope.
- **No protobuf and no text on a small owner.** A reason, a detail and
  a fault's code are 6-bit numbers.
- **A clock question a heartbeat answers alone,** with core 0.113's
  ahead time. The owner publishes its `clock_ahead` fault once.

`health.v2` is a new file, [`v2.md`](v2.md), and no entry of `v1.md`'s:
it changes what a conforming owner puts on the wire. It restates
`health.v1` 0.11 by reference, and is written against core 0.113 and
`freshness.v2` 0.12. Its status is `experimental`: no adopter's mapping
uses it yet.

**What it replaces.**
- **§2.3 and §3, the records.** `Status`, `Check` and `Fault` are packed
  types of a standard schema, `health.v2.json`. `since` and `at` are
  whole seconds from the deployment epoch, and optional. A fault's
  detail is a 16-bit `argument`.
- **§2.5, the clock.** The heartbeat's ahead time says it. The fault is
  published once per hold, and carries no Zenoh timestamp. §5's
  question reads the last heartbeat read, and no fault.
- **§2.6, unknown.** `unknown` is the record's code 0, and a record core
  §7.6 refuses is unknown too.
- **§2.10, words.** Three kinds of word, each a number: 0 none, 1 to 31
  the profile's, 32 to 62 the service's own, and `out_of_range`.
- **§2.2, the order,** is restated by seq, and a reading of two
  snapshots under one run and one asof is one instant.

**New keyword rules, each with its evidence.**
- §2.2: an owner MUST order its puts so that at each seq the status is
  not better than the worst current check. `[Sc: §3]`
- §2.2: an owner SHOULD build both snapshots of one GET at one asof. A
  tool MAY report a break from one reading that is one instant, and
  SHOULD see it in two readings otherwise. `[F: v2/snapshots.json]`
  `[Sc: §7]`
- §2.3: an owner MUST NOT implement both majors in one instance.
  `[Sc: §1]`
- §2.5: an owner MUST publish one `clock_ahead` fault when its guard
  begins to hold, and SHOULD NOT publish it again while it holds.
  `[Sc: §4]`
- §2.5: a reader MUST answer the clock question as the fixture does.
  `[F: v2/clock.json]` `[Sc: §4]`
- §2.6: a reader MUST read `unknown`, and a record it refuses, as
  unknown, never as `ok`. `[F: v2/records.json, v2/judgements.json]`
- §2.10: a reader MUST read a contract's tables as the fixture does. An
  owner MUST write, and a reader MUST read, each word as the fixture
  does. A reader MUST NOT show a name from a revision the descriptor
  does not name. A later revision SHOULD append to a table.
  `[F: v2/tables.json]` `[Sc: §9]`
- §2.11, §2.12: a reader MUST judge as the fixture does, by the major
  the descriptor lists, `health.v2` where it lists both.
  `[F: v2/judgements.json]` `[Sc: §1]`
- Restated from v1 with this major's evidence: absent is never a level
  (§2.1), a stale status is never a level (§2.4), and providers are
  found by descriptor, never by token (§2.7).

**The three guarantees hold.**
- **The status is never better than its worst current check:** the same
  order of puts, now checkable from one GET.
- **Absent is never `failed`:** presence's word, and no record takes
  part.
- **Unknown is never `ok`:** `unknown`, a refused record, no status
  held, a status not confirmed within 60 s, and nothing crossed, each as
  v1 had it.

**What is narrowed.**
- **A reason and a detail in words.** A word no table names is
  `out_of_range` on the wire, and stays in the owner's log. `health.v1`
  carried any text.
- **A fault's detail** is one 16-bit number.
- **A time** is to the second.

**Settled here, where the design left a choice.**
- **The profile's first tables** are the design's proposal: eight
  reasons, five details, and the code `clock_ahead`. They are drawn from
  the words the reference and the scenarios use, not from a survey of
  adopters. A later text adds words without a new contract.
- **`clock_ahead`'s argument** is the offset in whole seconds, rounded
  up, so that an offset above 0 never reads 0. It is 65,534 for that or
  more, and `not_available` where none was measured.
- **The fault answers nothing of the clock question.** It is published
  once, so it says that a hold began, never that it lasts. `health.v1`
  answered yes from a fault. Here a reader shows the fault as an
  occurrence, and answers from the last heartbeat it read. A face that
  lets the faults cross alone gives the occurrences, and the question
  is unobservable there.
- **Which heartbeats count:** a read heartbeat covering `health.v2`, or
  a read owner heartbeat of the `host` class, whatever its sections say.
  `health.v1` asked that the status's section be in step with the view.
  A late joiner that only subscribes has no view yet.
- **A fault carries no Zenoh timestamp.** Core §4.3's table lists
  `health.v1`'s fault alone, and this text leaves it so.
- **A table's names** are identifiers, `[a-z][a-z0-9_]*`, in all three
  kinds. A contract's table is the one list its resources carry, and an
  instance's the one list among its contracts. Two that differ are none,
  and the owner then writes `out_of_range`.
- **A descriptor that lists both majors** is read as `health.v2`.
- **One instant** needs both snapshots complete, one run, one asof, and
  no entry or tombstone of that run above the asof.

**Evidence.** [`conformance/v2/`](conformance/v2/) holds six files.
- `records.json`: 3 layouts, 42 writes, 49 reads and 8 arguments.
- `tables.json`: the profile's 3 tables, 23 declarations, 9 instances,
  20 writes, 23 reads and 8 growths.
- `snapshots.json`: 25 readings. `judgements.json`: 60.
- `clock.json`: 16 heartbeats and 37 cases. `schema.json`: 4.
- **Computed twice.** One script read the text into code and wrote the
  files. A second, written from the finished text, computed every
  expectation again. They share no code, and agree on all 327 cases.
  Both are the drafter's, kept out of the repository. No file was
  blessed.
- **Checked against what the repository holds.** The first script's
  packed records agree with the core's `packed/vectors.json` for the
  kinds these types use, its digests with `heartbeat/digests.json`, and
  its pages with 26 writes of `snapshot/bodies.json`. The schema's
  artifact id is the one the reference's `zk2 contract bundle` states.
- [`scenarios-v2.md`](scenarios-v2.md) holds 9 sections: v1's eight
  restated, and §9 for the words.
- **`judgements.json` takes the status's freshness as judged.** Its
  cases state `freshness.v2`'s verdict and reason, where `health.v1`'s
  state the observations. The freshness judgement has its own fixtures.

**The bytes, recomputed** from these layouts through the wire
baseline's model. Each matches the design's figure.
- A status and a check are 42 bits, 6 bytes, a 12-byte payload in a
  37-byte SDU. A fault is 58 bits, 8 bytes, 14 and 39.
- W2's four status changes a day are 148 B, where `health.v1`'s are
  232 B.
- A late joiner's status snapshot is 28 bytes, where it was 49.
- The heartbeat covering `health.v2` is a 16-byte payload: 118 KB a day
  at 28.5 s, as `health.v1`'s.
- An hour of a clock ahead is one fault, 39 B, where `health.v1`
  published 10.2 KB of them.

**What a reader built before this major does.** It finds no `health.v1`
in the descriptor, and answers not asked, `not_listed`. That is safe,
and blind. Readers move first.

**Not verified.**
- **No implementation has run a fixture or a scenario.** The reference
  implements none of this yet.
- **No byte crossed a wire.** Every `health.v2` figure is computed.
- **The contract is not yet in `spec/profiles/.history/`.** §3 states
  its fingerprint, which the reference's `zk2 contract fingerprint`
  computed. Publishing it is the code half's.
- **That an owner builds both snapshots of one GET at one asof** was
  seen in two runs of the reference and read in its comments, as the
  design says. §2.2 asks it as a SHOULD.

**What deliberately did not change.** Three resources and their QoS,
`faults` a stream. The status's horizon, 60 s, and its heartbeat. Stale
is not a level, and a fresh status vouches for the checks. §2.11's
steps, verdicts and reasons, and the roll-up. The standard contract
exposes nothing `link`.

## 0.11 — 2026-11-22: editorial: a heartbeat's time while the guard holds is the ahead time, read as the unknown time was, no rule changed (#986)

`freshness.v2` 0.12 has an owner write core 0.113's ahead time as its
heartbeat's time while its clock guard holds, where it wrote all ones.
This text said "its time unknown" of that heartbeat in five places.

**Changed: wording.**
- **§2.5** says the heartbeat's time is the ahead time while the guard
  holds. Its "Nothing else does" names a heartbeat whose time is no
  instant: the ahead time or the unknown time.
- **§5's clock question** reads "no instant, ahead or unknown" where it
  read "unknown". The reason keeps its token, `time_unknown`.
- **`scenarios.md` §4**, Expected 2 and 4, names the ahead time.
- **`clock.json`**: `heartbeat_held` is a heartbeat whose time is no
  instant, ahead or unknown. Four notes follow. No case's `heard` and no
  expectation moved: 29 cases.

**Deliberately not changed: every answer.** Under this major a
heartbeat's ahead time answers nothing alone, as the unknown time it
replaced answered nothing. The fault answers, and the owner still
publishes it again each period (§2.5).
- **`health.v2` reads it as a yes** (its §2.5), and has its owner
  publish the fault once. That is a new major's rule, not this one's.
- **Not taken here:** answering yes from the ahead time under
  `health.v1` too. It would change `clock.json`'s cases 21 and 25 and
  what a `health.v1` reader derives.

**Written against** core 0.100 still. The entry restates one sentence
over core 0.113 and `freshness.v2` 0.12.

## 0.10 — 2026-11-07: editorial: the scenarios read a GET's answer as snapshots, no rule changed (#977, #609)

Core 0.100 (#977's E3) answers a state GET by snapshot, whose entries
carry no Zenoh timestamp. zk2py round 105's F-503 found two of these
scenarios' rows still expecting the owner's stamp on a GET's reply. This
version re-keys them, and two rows beside them, written against core
0.100.

**Changed: the scenarios.**
- **§1, Expected 1 (F-503):** T's GETs answer the status in a snapshot
  under the run the owner's descriptor states, its entry with its
  confirmation. It said "stamped by the owner's session".
- **§4, Expected 4 (F-503):** G's answer lists the status under the run
  the owner's descriptor states, its times from the owner's clock, ahead,
  which no router reads. It said "under the owner's stamp".
- **§2, Expected 1 and 3:** G's entry carries the status's confirmation,
  and goes stale when that is more than 60.5 s old. They said "stamp".
- **§3, Expected 4:** `checks/net` is a tombstone in G's answer. It said
  a `reply_del`, which core 0.102 calls a stray one for a collection's
  member.

**The reference** asserts each entry's confirmation on the owner's
clock already (`zenkey/tests/profile_health.rs`). That the snapshot is
under the descriptor's run is its code half (#977).

**Deliberately not changed.**
- **No rule.** `v1.md` moves only its header: its version, and core
  0.100 as what it is written against.

## 0.9 — 2026-11-02: a clock ahead holds no status, and the clock question reads the heartbeat's time (#978)

Core 0.88 (#978's F6) narrows its clock guard to the state writes that
carry a Zenoh timestamp. The status carries none since 0.8, so the guard
no longer holds it, and `freshness.v2` 0.4's heartbeat goes on vouching
for it, its time unknown. This text, written against core 0.88, follows.

**Changed: §2.5.**
- **Before:** the guard held the status's puts and its heartbeat's bit.
  The status went stale, by design, 60 s after its last confirmation,
  and the next heartbeat after the guard released vouched for it again.
- **Now:** the status is not held. The owner puts it as at any other
  time, and its heartbeat vouches for it, its time unknown. `[Sc: §4]`
  An owner that the drift leaves worse off, its timed writes held, puts
  its status as for any occurrence. §2.3 drops its "unless its clock is
  ahead".
- **The fault stays:** the MUST at the guard's first hold is 0.8's.
- **The repeat** follows the heartbeat covering `health.v1`: at least
  once per its period, 30 s at most, where 0.8 said once per status
  interval. `[Sc: §4]` In revision 1.1 the two are one: the status is
  the interface's one collection, so its heartbeat beats at 28.5 s. The
  heartbeat is what a reader's "no" now rests on, so a reader that
  joined during the hold hears a fault within the period it hears the
  time unknown in. A minor that added a collection with a shorter horizon
  would quicken both.
- **Once the guard releases,** the next heartbeat's time is known again,
  the evidence that the drift ended, where 0.8's was a heartbeat vouching
  again.
- **A deployment under `drop_future_timestamp`** loses the fault, and
  its readers see the heartbeat's time unknown, where 0.8's saw the
  status go stale.

**Changed: §5's clock question, and `clock.json`.** A confirmation of
the status no longer tells a held guard from a released one.
- **Yes:** a `clock_ahead` fault, with no heartbeat whose time is known
  heard since.
- **No:** the last heartbeat heard has its time known, after the last
  fault or with none heard.
- **Unobservable:** `time_unknown`, a new reason, when the last
  heartbeat's time is unknown with no fault since one whose time is
  known; `nothing_heard` when neither a fault nor a heartbeat was heard.
- **A heartbeat counts** where the reader read the status's section in
  step with its view, its bit set or clear (§2.5). A put of the status
  says nothing of the clock.
- **The fixture's tokens.** `status`, a confirmation, is gone. Its cases
  read `heartbeat`, a heartbeat whose time is known. `heartbeat_held`
  (its time unknown) and `status_put` (a put of the status) are new, with
  8 cases, 29 in all.

**Changed, no rule:** §2.10's `clock_ahead` names the writes that carry
a Zenoh timestamp; §5's "Which cause" drops the clock ahead from a stale
status's causes; Appendix A's clock row.

**`scenarios.md` §4.** Step 2 puts the status once the guard holds, and
S receives it. Every heartbeat from the detection on vouches for it, its
time unknown, and S judges **healthy** at 65 s, where 0.8's judged stale.
Step 3's evidence is a heartbeat whose time is known again. Step 4's
readers have the heartbeat's time unknown to go on.

**Deliberately not changed.**
- **The contract:** revision 1.1, `sha256:60a43745…`, as 0.8's. No
  fingerprint moves.
- **The fault's level** stays `FAILED`, and its stamp as §2.5 says.
- **Revision 1.0's owners** re-put their status with a Zenoh timestamp,
  which the guard holds, and send no heartbeat. A reader that heard one's
  fault answers yes until its reading ends, the drift over or not. No
  owner of this repository serves 1.0, and zk2py's move in its round.

## 0.8 — 2026-11-01: the status confirmed by `freshness.v2`'s heartbeat, never re-put (#978)

Epic #975 makes the wire bandwidth-first. Under `freshness.v1` an owner
re-put its status, unchanged, every 30 s: about 240 KB a day on W2
(#989). `freshness.v2`, text 0.1 (#978's F3), confirms a state member by
a heartbeat per interface instead, which vouches for it by a bit. This
text, #978's F4, moves `health.v1` onto it.

**The contract: revision 1.1,** `sha256:60a43745…`, published in
`../.history/health.v1/`. Its `uses` is `freshness.v2`, where 1.0's was
`freshness.v1`: a review (core §9.8, `uses_changed`), no resource
changed. Revision 1.0 stays defined, and its owners re-put.

**Changed: §2.3's "Confirmed every 30 s".**
- **Before:** the owner confirms its status at least every 30 s, by a
  change or a re-put of the value unchanged (`freshness.v1` §2.4).
- **Now:** by a change or by its heartbeat, as `freshness.v2` §2.4
  requires. The heartbeat sets the status's bit while the owner vouches
  for its level, and the owner never puts the status again unchanged.
- **§2.3's table** names its last column "Confirmed": the status by its
  heartbeat, at least every 30 s, never re-put.

**Restated over the heartbeat, no rule changed.**
- **§2.4:** freshness is `freshness.v2`'s, whose judgement is v1's with a
  heartbeat counted as a confirmation. "Why one heartbeat" is "Why one
  bit".
- **§2.5:** the guard holds the heartbeat's bits too, and once it
  releases, the next heartbeat vouches again, where the status was
  re-put. §2.5's rules move with #978's F6, not here.
- **§2.8:** a status that crosses brings its heartbeat with it
  (`link.v1` §2.9), which is the liveness evidence. The face's cost is
  that heartbeat, about 10 B every 28.5 s.
- **§2.7:** each owner implements `freshness.v2` beside `health.v1`, an
  interface token more, which the tokenless set holds too (core §8.1).
- **§1, §3, Appendix A and Appendix B** name `freshness.v2`.

**`scenarios.md`.** S hears the heartbeat key too, and G and T take the
deployment's word for their clock: a status put carries no Zenoh
timestamp of the owner's, so G can no longer measure it from S's
deliveries. §1's `lab/quiet` and §5's 100 services hold `freshness.v2` in
their tokenless set too. §1 waits 65 s, past the horizon, and S judges the status
fresh with no re-put. §2 confirms for 65 s first, then stops vouching:
`set_confirming(false)` clears the status's bit, where the owner closed
its writer. §4's step 3 is a heartbeat that vouches again. §4's step 4 hears the
status put while ahead: it carries no Zenoh timestamp of the owner's, so a
router under `drop_future_timestamp` stamps it and lets it through, and
drops only the fault. §8's run A hears the heartbeats that cross with the
status.

**`link.v1`'s `plan.json`.** zenoh-modem's RF face denies the
heartbeats covering `health.v1` and `config.v1`, `stream:freshness.v2`,
as `link.v1` §2.9 says: none of their collections crosses that face. The
case's expectation follows, and no rule of `link.v1` changed.

**Written against core 0.82,** where it said 0.73: the heartbeat is core
0.82's §7.5 form 6, and its stamps §4.3's table. The index's row follows.

**Deliberately not changed.**
- **The horizon:** 60 s. **Every judgement:** §2.11's steps, the
  verdicts, the reasons and `judgements.json`.
- **Checks:** still no horizon, so no heartbeat covers them, and the
  status vouches for them.
- **§2.5's fault cadence:** once per status interval, 30 s.

## 0.7 — 2026-11-01: editorial: §2.5's fault stamp read under core 0.73's session posture, no rule changed (#976)

Core 0.73 orders state by an owner's run and seq, carried in band. It
asks an owner's session to run without timestamping (its §8.2, "Session
posture"). §2.5 said that a session with its HLC enabled stamps the
fault, and that a deployment's clock is its HLC. Neither describes an
owner under 0.73.

**Editorial: §2.5, "The routers and the fault's stamp".**
- **Before:** a session with its HLC enabled stamps every put, so the
  fault carries a stamp from the owner's clock.
- **Now:** an owner's session runs without timestamping (core §8.2). So
  the fault carries a stamp only where the owner sets one, as the
  reference does. A fault left unstamped arrives under a router's stamp,
  or none.
- **"An owner that sets the stamp itself":** its clock is "its wall clock
  with a bump (core §4.3), which stamps its status too". It was "in a
  deployment that clock is its HLC".

**Editorial: `scenarios.md` §4's setup** cites `spec/scenarios/state.md`
§7's step 6, where it cited step 3. Core 0.73's `state.md` moves "Ahead"
there, unchanged, after the runs' steps.

**Written against core 0.73,** where it said 0.23: the text now cites
core §8.2's session posture, which 0.73 adds. The index's row follows.

**Deliberately not changed.**
- **No rule.** §2.5's MAY to set the fault's stamp stands. Core 0.73's
  table lists a fault among the resources that may carry a Zenoh
  timestamp, for that MAY.
- **"A simulated offset is not a drift"** stays: it says what a session
  with an HLC does, which is still zenoh's.
- **No fixture, scenario step, section number or label.**

## 0.6 — 2026-10-30: editorial: round 86's F-327, no rule changed (#935)

zk2py round 86 (PR #959) read 0.5 cold, beside core 0.62 and
`observer.md` 0.6. It found that 0.5 changed no rule, and filed one
finding against this text, F-327, hard to read. This is #935's fourth
part.

**Editorial: F-327, the scenarios' "T's clock".**
- **The gap.** "Without it, a tool measures its clock from puts it hears
  during its reading, and the status is re-put every 30 s, so the
  reading lasts 30 s, or its subscription is declared before the owners
  start" stated a requirement as a fact. Its "or" joined a duration to a
  precondition.
- **Now.** Three sentences, the finding's rewrite. The tool measures its
  clock from the puts it hears. The status is re-put every 30 s. So such
  a tool's reading lasts 30 s, unless its subscription is declared
  before the owners start.

**How "no rule changed" was checked**, as for 0.5. The scenarios' 44
keyword sentences and Expected lines are unchanged. The rewrite holds
no keyword, and was read beside its old text: the tool, the 30 s and
the precondition are the same. `v1.md` is unchanged but for its header.
No fixture moved.

**The figures.** `scenarios.md`'s mean went from 13.70 words to 13.50.
`spec/.prose-baseline.json` is lowered to it.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Written against core 0.23**, still.

## 0.5 — 2026-10-30: editorial: inline history taken out of the rules and long sentences split, no rule changed (#935)

The rules carried their own history, as the core's did before core 0.60
(#935, epic #921 Track B). A rule named the version that made it, as in
"(0.2)". This changelog and the core's already hold that history, so the
text now states each rule and cites its evidence, and nothing else. This
is #935's third part, beside `freshness.v1`, `hostid.v1`, `config.v1`
and `link.v1`.

**Editorial: inline history taken out of the rules, no rule changed.**
- **This text's versions.** "(0.2)" is gone from three bullets of §2.5:
  the fault's stamp, "An owner that sets the stamp itself", and "A
  simulated offset is not a drift".
- **The core's version.** §2.3's "the core keeps its SHOULD (core 0.23)"
  is "the core keeps its SHOULD".
- **An issue.** Appendix B's zenwatch row said that zenwatch implements
  `zenwatch.v1` "since #614's FL3". It now says that zenwatch implements
  it.
- **The scenarios.** Their conventions' "T's clock (0.2)" and §4's
  "Without root (0.2)" are run-in labels alone. §5 to §7 cite "the
  conventions" without "0.2", and the opening paragraph names the
  reference's tests without "#721, PE" and "#721, PF".

**The long sentence, split.** §2.2's SHOULD, that a tool reports a
status better than a check from two readings a grace apart, ended in its
reason after a colon. The reason is a sentence of its own, "In one
reading, a reader that pairs …". The SHOULD keeps its words.

**Nothing moved to an appendix.** §2.5's measurement, a stamp 2 s ahead
re-stamped on zenoh 1.10.1, carried no tag and states the ground of its
bullet, which holds no keyword. This text has no appendix of
measurements, so it stays where it is.

**How "no rule changed" was checked**, as for core 0.60 and 0.61. Every
sentence holding MUST, MUST NOT, SHOULD, SHOULD NOT, MAY or RECOMMENDED
was extracted before and after, keyed by section, and the two sets
diffed. Both hold 27. 24 are unchanged, two lost a tag only (§2.3's and
§2.5's MAY), and §2.2's SHOULD lost its reason to a sentence of its own.
The scenarios' 44 such sentences and Expected lines are unchanged. No
fixture moved.

**The figures.** This text's tags went from 5 to none, its sentences
over 50 words from 1 to none, and its mean from 15.02 words to 14.95.
`scenarios.md`'s tags went from 4 to none, and its mean from 13.75 to
13.70. `spec/.prose-baseline.json` is lowered to them.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history.
- **Written against core 0.23**, still, and the header's "uses
  `freshness.v1`, text 0.2".

## 0.4 — 2026-10-10: zenwatch's row cites its v1 source and its zk2 home (#614)

#614's FL3 made zenwatch a zk2 service: `zenwatch/src/publish.rs` on
`main` is now its zk2 self-publication, implementing this profile beside
an interface of its own. Appendix B's zenwatch row cited "its
`publish.rs`" for the v1 health document it maps.

**Changed: wording, in v1.md.**
- **Appendix B's zenwatch row** cites its v1 `zenwatch/src/publish.rs` on
  the repository's `v1` branch, and names the interface of its own the
  row foresaw: `zenwatch.v1` (`zenwatch/contracts/zenwatch.v1.toml`).

**Deliberately not changed.**
- **No rule.** The mapping is the one 0.1 wrote: the status, beside an
  interface of its own for its rules' state.
- **Written against core 0.23**, still: a citation reads nothing new
  from the core.

## 0.3 — 2026-10-10: §5's clock question has its fixture (#721, PF)

The reference tool (`zenctl health`, #721's PF) answers §5's last
question, "is this service's clock ahead?", and found it pinned by the
text and checked by no fixture: the runtime's §4 test computed it by hand.

**Changed.**
- **`conformance/clock.json`** (21 cases), cited from §5's row: what one
  subscriber heard of one service in its window, in arrival order (the
  status's puts and deletes, `clock_ahead` faults, other faults), and its
  presence as `judgements.json` writes it → yes, no, unobservable or not
  asked, with a reason. `zenkey_model::health::clock_ahead` runs it.

**Deliberately not changed.**
- **The rule.** Every case is §5's row as 0.2 states it. Where the row is
  silent the fixture reads it so, and says so in its notes:
  - a status delete is no confirmation (§2.3), so it neither answers no nor
    clears a fault;
  - a fault's level and stamp play no part, only its code (§2.5: nothing a
    reader concludes rests on the stamp);
  - "not asked, as the first row's" is absent and not listed. A service
    whose descriptor was not read, or whose token a read possibly
    incomplete missed, is still answered by a fault heard on its key: the
    fault is its own word.
- **§5's second question** ("does its status agree with its checks?")
  stays without a fixture of its own: its answer is the first question's
  reason for one reading (`inconsistent` is the no), and `judgements.json`
  pins that reason.

## 0.2 — 2026-10-10: a clock ahead without root, and the tool's clock (#721, PE)

Two findings of the Python implementation's cold read of 0.1 (#609),
resolved with the runtime that implements the text (`zenkey::health`, PE)
and measured where zenoh decides.

**Changed.**
- **F-103: what `scenarios.md` §4 checks without root.** Its expected 2
  wanted the `clock_ahead` fault re-stamped by R1, which needs the fault to
  reach R1 dated ahead. A runner without root cannot move a host's clock,
  and zenoh lets no program move its session's HLC, so the offset is
  simulated in the owner's runtime. Measured on zenoh 1.10.1, from Rust
  (`zenkey/tests/profile_health.rs`) and from Python: a session whose HLC
  runs re-stamps a put given a stamp 2 s ahead of that HLC before the put
  leaves it, so the fault reaches R1 under the owner's honest stamp, as an
  unstamped one does.
  - §2.5 now states that fact: a simulated offset is not a drift. It says
    that nothing a reader concludes rests on the fault's stamp, and that
    an owner MAY set the stamp itself from the clock it mints state stamps
    with, which the reference runtime does.
  - §4 marks the stamp check **[moved clock]**, with a new step 4 under
    `timestamping.drop_future_timestamp`: the fault and the status put
    while ahead are dropped, and the owner's GET still answers. Every
    other expectation of §4 holds without root.
  - A runner meets the [moved clock] tier by moving the owner host's
    clock, or with an owner that stamps from its offset clock on a session
    without an HLC. The reference does the second: in-process client
    owners, whose HLC is off by default, and its owner example under
    `--clock-offset-ms`.
- **F-104: the tool's clock in `scenarios.md` §5 to §7.** T reads by GET
  alone, and `freshness.v1` §2.6 ages a reply only with a trusted clock,
  so read literally every status there was unobservable,
  `clock_untrusted`. T now takes the deployment's word (`freshness.v1`
  §2.6, ground 1), stated in the conventions and in each setup. Every
  session of a scenario runs on one host, so the word holds.
- **The conventions' waits:** §4 waits about 100 s (65 s, then 35 s), not
  70 s.
- **Uses `freshness.v1` text 0.2.** §2.8 already cited its §2.5, "a
  member it knows", and F-104 reads its §2.6.

**Deliberately not changed.**
- **The rule.** An owner whose guard holds still publishes `clock_ahead`
  at `FAILED`, at once, then once per status interval, and none after
  release. Only what a runner can observe of the stamp moved.
- **Measuring T's clock** was the other way. T would subscribe to the
  statuses before the owners start, and measure from their first puts.
  It was not chosen: it ties §5–§7 to a subscription and a start order,
  and a tool reaching a running deployment would hear a re-put only every
  30 s. What those sections check is health's reading. The grounds of a
  clock's trust are `freshness.v1`'s, which its scenarios §4 and §6 check.
  G in §1 to §4 still measures from S's deliveries.
- **Core 0.23 §4.1 and Appendix B** are not amended here. The session's
  re-stamp is a zenoh fact, recorded in §2.5 where this text relies on it.
  The core may list it among its §4.1 facts at its next version.

## 0.1 — 2026-10-10: the first text, draft (#721, PD)

`health.v1` is the third profile of #613's first tier, after `hostid.v1`
and `freshness.v1`, which it uses. The maintainer decided on 2026-10-09
that it comes before zengui and zenwatch move onto zk2 (FL, #614). Its
standard contract was written with the walkthrough, published as revision
1.0 (`sha256:e9dbcdcb…`), and implemented by name in the tcgui, ZenSight
and zenoh-modem mappings. This text gives it rules, and is written against
core 0.23, which names the `faults` stream where §4.3 said to report a
clock ahead through `health.v1`.

**Stated.**
- **The contract moves** under the profile (§3), unchanged:
  `spec/profiles/health/health.v1.toml` and its proto, the same
  fingerprint before and after, revision 1.0 copied byte for byte into
  `spec/profiles/.history/health.v1/`. Doc strings and proto comments were
  added, and are outside the fingerprint.
- **The levels** (§2.1): `OK`, `DEGRADED`, `FAILED`, ordered; absent is
  presence's word, never `FAILED`.
- **Aggregation** (§2.2): the status never better than its worst current
  check, with the puts ordered so that no intermediate breaks it; no
  roll-up across services; a tool's roll-up is the worst established level,
  with stale, unobservable and not-asked services counted apart.
- **Who puts what, and when** (§2.3): the status before the tokens, a MUST
  where core §8.2 says SHOULD; re-put by `freshness.v1`; never deleted
  while the service runs. Checks put on change and deleted when retired.
  Faults are occurrences.
- **Freshness** (§2.4) by `freshness.v1`, and **a fresh status vouches for
  the owner's checks**, which carry no freshness of their own.
- **A clock ahead** (§2.5) is a `clock_ahead` fault at `FAILED`, and the
  status goes stale by design.
- **Unknown levels** (§2.6), **the tokenless set** (§2.7), **constrained
  faces** (§2.8), **device-as-service** (§2.9), **fault codes** (§2.10),
  and **the reader's procedure** (§2.11).
- **What a tool may conclude** (§5): healthy, unhealthy, stale,
  unobservable, not asked; the status against its checks; a roll-up; a
  clock ahead. The changes against v1's health document (Appendix A), and
  the adopters' mappings (Appendix B).
- **Evidence:** `conformance/judgements.json` (61 cases),
  `conformance/rollups.json` (8), `conformance/codes.json` (22), and eight
  scenario sections, five the runtime's and three a tool's.

**Decided here, where the record was silent.**
- **One heartbeat.** The contract gives `checks` no horizon. Rather than
  give each check a freshness of its own, a fresh status vouches for every
  check the owner holds, and the owner deletes what it retires and leaves no
  check it cannot run. 64 checks re-put every 30 s would cost 64 times the
  status's one put.
- **The order of the puts.** v1 said a writer orders a transition across
  keys so that every intermediate is safe (RFC 04 §1.2); here, the status
  first when a check gets worse, the check first when it gets better.
- **A *no* can rest on a check, a *yes* only on the status.** A status at
  an unknown level, with a current check `FAILED`, reads unhealthy: the
  check is vouched for by the fresh status. Checks all `OK` never make an
  unknown status healthy, since a service can fail in ways no check names.
- **A status better than a check reads at the check's level,** reason
  `inconsistent`, rather than unobservable: the evidence for *no* is there.
- **The fault for a clock ahead.** `docs/zk2/architecture.md`'s S7 row and
  core §4.3 said to report through `health.v1`, but the status is state,
  which an owner ahead stops writing. A `faults` sample is a stream sample,
  which the guard does not hold.
- **Fault codes are a vocabulary of the form `[a-z][a-z0-9_]*`,** the
  profile's table first, an application's codes beside it, prefixed by
  recommendation.
- **A deleted status reads unobservable,** reason `deleted`: the owner
  broke §2.3, and nothing answers whether it is healthy.
- **Silence ages a status only when one is known to exist.** A present,
  listed owner holds one (§2.3), so a subscriber's silence makes it stale.
  Across a constrained face nothing says so until a delivery crosses
  (`freshness.v1` §2.5, "a member it knows"), so a status of which nothing
  has crossed is unobservable, `nothing_crossed`, which is what core R7
  says of a face where nothing crosses.
- **zenoh-modem's RF face lets nothing of `health.v1` cross**: its
  `link.md` denies the device's state and `health.v1`'s streams. The text
  records it as a gap of that mapping (Appendix B) and leaves `link.md` as
  it is.

**Deliberately not stated.**
- **No application field on `Status`** (the maintainer's decision for
  #721): ZenSight's `HealthSnapshot` and zenwatch's counters stay in their
  own interfaces.
- **No horizon on checks or faults.** The contract is published, and
  adding one would be a new revision; the vouching rule makes one
  unnecessary.
- **No resolution of a fault.** A fault is an occurrence; a confirmed
  status is the evidence that a drift ended.
- **No `health.v1` in the descriptor's `profiles`.** Core §3.3 defines the
  list as the contracts' `uses` and the derivation-only profiles, and
  `health.v1` is neither: its `interfaces` carry it.
- **No runtime and no tool here.** `zenkey::health` and the owner example
  are #721's PE, the fleet's checks and `zenctl health` its PF. They run
  `scenarios.md` §1–§5 and §6–§8.
