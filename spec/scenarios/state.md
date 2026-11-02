# State scenarios (core §4)

Common setup: an owner `ground/fleet-mgr` with a state template
`plans/{vehicle}` and a tombstone window W (60 s unless stated). Its
session's timestamping is off, and its runs come from a record unless
stated. The template declares no `freshness.ttl_s`, so core §4.3's table
does not list it. A consumer `vehicle-01/executor`, bound with
`{vehicle} = self.system`, its session's timestamping off too, zenoh's
default for a client (core §4.1).

## §1 In-band stamps (S1, S2, §4.3)

**Setup.** The scenario is played twice. The first time, the owner and
the consumer are clients of R1, whose timestamping is disabled. The second
time, they are clients of R2, whose timestamping is enabled. The owner's session is never the router
itself. There, the owner's zid and the router's are one, and the id checks
below prove nothing (core §4.2, "Observing S1"). The owner also serves
`status`, a state with `freshness.ttl_s` 60. Its contract uses
`freshness.v1`, so core §4.3's table lists it. A `freshness.v2` state's
puts carry no Zenoh timestamp.

**Steps.**
1. The owner puts v1, then v2, then deletes the key. The consumer
   subscribes before, and GETs with `All` + `Latest` after v2.
2. **The control:** a third client of the router puts on a key of its own
   without setting a timestamp, and the consumer subscribes to it.
3. The owner puts v3 and v4 back to back, with nothing between them.
4. The owner puts `status` twice, and the consumer GETs it.

**Expected.**
1. Every put's payload is an envelope of the mutation form, whose run is
   the descriptor's `run`. The delete's attachment is a delete envelope of
   the same run. Each seq is above the one before.
2. The GET after v2 returns v2's bytes, its envelope included.
3. Through R1, no put, delete or reply of `plans/{vehicle}` carries a
   Zenoh timestamp. The owner sets none, and R1 adds none. Through R2,
   each of its puts arrives with R2's zid, and its delete and its reply
   with none. The control's put arrives with no timestamp through R1, and
   with R2's zid through R2.
4. v4's seq is v3's plus 1.
5. Through either router, each put and the reply of `status` carry a
   Zenoh timestamp whose id is the owner session's zid. The second put's
   is above the first's.

*The reference (`zenkey/tests/state.rs`) plays both as
`s1_in_band_stamps_and_no_per_put_hlc`.*

## §2 Deletes inside the window, and the collection (S2, S3)

**Steps.** The owner holds `plans/a` and `plans/b`, and deletes `plans/b`.
Within W, the consumer GETs `plans/b`, then the selector `plans/*`.

**Expected.**
- For `plans/b`: a `reply_del` whose attachment is the delete's envelope.
- For `plans/*`: `plans/a` with the bytes the owner put, and a `reply_del`
  for `plans/b`.

*The reference (`zenkey/tests/state.rs`) runs it as
`s2_a_reply_del_carries_the_delete_envelope`.*

## §3 The owner is authoritative (S4)

**Setup.** As above. A storage manager is deployed covering
`zk2/ground/fleet-mgr/**`.

**Steps.**
1. A tool inspects the routers' storage admin space.
2. Without the storage, the consumer GETs its plan, through a session whose
   outgoing queries the test captures.

**Expected.**
1. The tool reports the storage as a finding: no storage may answer on an
   owner's state keys.
2. The consumer's query carries target `All` and consolidation `Latest`,
   and the owner alone answers it.

## §4 Last-known from an archive (S5, S6)

**Setup.** An archive `vehicle-01/archive` records `plans/vehicle-01`. The
link between the vehicle's router and the ground's is cut.

**Steps.** The owner puts v3 before the cut. After the cut, the consumer
GETs the owner.

**Expected.**
- The owner's GET returns no reply within the timeout.
- The consumer reads the archive's key for the same plan. It gets v3's
  bytes as the owner put them, its envelope included, with its type
  identity in the attachment.
- The consumer presents v3 as *last-known*, never as current.

## §5 Alignment after reconnect (S5)

**Steps.**
1. With the link cut, the owner deletes the plan within W. The link heals
   within W.
2. The same, but the access control refuses the archive's GETs for the
   whole alignment, its retries included (core §4.4), so every reply set
   is empty.
3. The link stays cut and the owner stops. An archive on the owner's side
   still records the plan's deletion.

**Expected.**
1. The archive re-reads the owner's collection before serving it as
   confirmed. The owner answers the plan with a `reply_del`, and the archive
   drops it. Should the owner's token arrive before the route to its state
   queryable, the first read returns empty, and a retry drops the plan.
2. The empty reply sets drop nothing, however often the archive retries.
   The archive keeps serving the plan, with `confirmed: false`.
3. When the link heals, the vehicle's archive aligns from the owner-side
   archive, reading it through the collection's archive form (core §4.4).
   That archive answers with the `reply_del`, which is positive evidence,
   and the vehicle's archive drops the plan.

*The reference (`zenkey/tests/state.rs`) refuses every read in step 2: a
refusal of one read alone would be healed by the retry.*

## §6 A window shorter than the outage (S3)

**Steps.** As §5.1, but the link stays cut for longer than 60 s.
1. The deployment has configured W to the face's maximum outage.
2. W is left at 60 s.

**Expected.**
1. After the heal, the owner still answers the deleted plan with
   `reply_del`, and the archive drops it.
2. The owner answers nothing for the plan: there is no positive evidence.
   The archive keeps serving the deleted plan, with `confirmed: false`.

The second run is the failure that rule S3's raised window prevents.

*Spike S5 measured this resurrection with a storage ("after the producer's
window": wrong without replication).*

## §7 Runs (S7)

**Steps and expected:**
1. **A record.** The owner writes rev 10 under run n. It restarts with
   its clock 5 s behind, reads its record, then writes rev 11. Rev 11
   comes under run n + 1. The record holds it before rev 11 is put, and
   the new descriptor states it. The consumer accepts rev 11 as another
   run's, and orders by seq within run n + 1 from then on.
2. **No record.** The owner starts with no record and writes rev 10.
   Then it restarts, a new epoch with a fresh zid. Its clock is 5 s
   behind, and it writes rev 11. Each start draws a 32-bit run. Each
   envelope's CH is `0x10`, the descriptor's `run` has 8 hex digits, and
   the two starts' runs differ. The consumer accepts rev 11, whatever
   the clock. Likewise with a record at 0xFFFF, which is spent. The owner
   draws a 32-bit run, and leaves the record as it is.
3. **Before the wrap.** The test sets the owner's next seq to 16,777,214
   (2^24 − 2), then the owner writes three times. The first two come at
   seqs 16,777,214 and 16,777,215 of run n. The third comes under a new
   run at seq 0. The owner puts its descriptor stating the new run before
   it puts the third. The consumer accepts the third as another run's.
4. **Late joiners across a restart.** Before the restart, a consumer and
   a tool each seeded rev 10, in §10's order. The owner restarts and
   writes rev 11, then rev 12, under its new run. The consumer orders by
   S7 from its first value on, its seed's reply included: it accepts rev
   11, then rev 12. The tool keeps §4.2's merge for as long as it follows
   (observer §4.1). It keeps the later arrival across two runs, so rev
   11, then rev 12.
5. **Inside the seed.** The owner writes rev 10. A second session holds a
   queryable on the plans that is not `complete`, and keeps each query it
   takes. A consumer and a tool each subscribe, then make S4's GET, which
   the owner answers and the second queryable holds open. While it is
   open, the owner restarts and writes rev 11 under its new run, which
   both hear live. Then the second queryable lets the GET end, and the
   owner writes rev 12.
   - `Latest` delivers the owner's reply of rev 10 at the GET's end,
     after rev 11's live sample (core §4.1).
   - The consumer accepts rev 11, then the reply, which carries the old
     run. It holds rev 10 until rev 12, which it accepts.
   - The tool keeps the later arrival too: it holds rev 10, the reply that
     arrived last, until rev 12.
6. **Ahead.** The owner's clock runs 2 s ahead, beyond the router's HLC
   delta, and it holds a subscription to a router-stamped heartbeat. It
   serves §1's `status` too, which core §4.3's table lists. It detects
   the drift, then tries a put and a delete of `status`, and of a plan.
   - `status`'s put and delete are refused: each would carry a Zenoh
     timestamp.
   - The plan's put and delete go on, under the in-band stamp alone:
     `plans/{vehicle}` is on no row of the table.
   - Where it implements `health.v1`, it reports the drift as a `faults`
     sample with code `clock_ahead`. Its status, which carries no Zenoh
     timestamp, is not held (`profiles/health/scenarios.md` §4).

*Spike S12: with a clock 2 s ahead, the next right-clocked write looked
stale. It also showed rev 14 rejected as stale after a restart without a
catch-up. Under runs that cannot happen. A restart takes a new run,
which a consumer accepts whatever its clock.*

*The reference (`zenkey/tests/state.rs`) runs steps 1 to 3 as
`s7_a_run_record_persists_across_a_restart`,
`s7_no_record_takes_a_random_32_bit_run` and
`s7_the_run_changes_before_the_seq_wraps`, and step 6 as
`s7_an_owner_ahead_stops_its_timed_state_writes`. Steps 4 and 5 are
`s7_late_joiners_across_runs` and
`s7_late_joiners_across_runs_inside_the_seed`: the consumer through
`ValueOrder`, the tool's merge beside it. The fleet's two seeded
subscriptions run the tool's on the bus (`zenkey-fleet/tests/seeding.rs`,
`s7_a_tools_late_joiner_keeps_the_merge_after_its_seed`).*

## §8 Events replay

**Setup.** An owner publishes 1,000 occurrences of an event with
`retention = "1h"`: 500 dated within the last hour, and 500 older, whose
ULIDs carry older times. A union storage runs on `zk2/*/*/*/events/**`.

**Steps.** A consumer replays with a wildcard GET bounded by the retention.

**Expected.** Exactly the 500 recent occurrences. With a backend that
ignores `_time`, the consumer filters by the ULID's time.

*Spike S5: the union replay returned 1,000 in 4 ms; the memory backend
ignored the `_time` bound.*

## §9 An archive's backend refuses an outdated put (§4.4)

**Steps.** The archive records v1 (seq 1 of run n) and a delete (seq 2 of
run n). Then:
1. an older put of the same run, v0 at seq 0 of run n, reaches it late;
2. a put of another run, v9 under run m, reaches it late.

**Expected.**
1. The archive answers the key with a tombstone, never with the older
   value.
2. The archive records v9 and answers it: nothing orders two runs (core
   §4.3). That is core §4.3's residual risk, as an archive meets it. A
   late value of an old run revives a deleted key until the next
   alignment.

*Spike S5: the storage manager 1.10.1 accepted the older put and resurrected
the key, which is why it cannot be an archive's store as released.*

## §10 A late joiner's seed (§4.2)

**Setup.** As above, every seq below one run's. The owner holds `plans/a`
(v1, seq 1), and deleted `plans/b` at seq 2, within W. A tool, the late
joiner, wants the plans and every change after them. The test holds the
owner's answer to a GET until it releases it, so that it can place the
owner's puts between the late joiner's steps. The reference plays the
owner from a session of its own, whose puts, replies and `reply_del`
carry the envelopes it writes (core §4.3).

**Steps.**
1. The late joiner subscribes to `plans/*`, then GETs `plans/*` with `All`
   + `Latest`.
2. The owner takes the GET and holds it. It puts `plans/d` (v1, seq 3).
3. The owner makes its answer: `plans/a` (v1, seq 1), a `reply_del` for
   `plans/b` (seq 2), and `plans/d` (v1, seq 3).
4. Before it sends the answer, the owner puts `plans/a` (v2, seq 4), then
   `plans/c` (v1, seq 5).
5. The owner sends its answer and ends the GET.
6. **The control:** the other order. The late joiner GETs first; the owner
   holds the GET, makes its answer (`plans/a` and the `reply_del` for
   `plans/b`), puts `plans/c` (v1, seq 5), and answers; the late joiner
   subscribes once the answer has arrived.

**Expected.** When the GET has ended, the late joiner holds:
1. `plans/a` as v2 with seq 4. The reply (seq 1) is older than the live
   put, and is dropped by the merge, never shown after v2.
2. `plans/c` as v1 with seq 5: the put the GET's answer does not hold
   arrived through the subscription.
3. `plans/b` as a deletion with seq 2: the `reply_del` reached the merge
   as a deletion, never as an absence.
4. `plans/d` as v1 with seq 3, once: the live put and the reply carry one
   stamp, and are one mutation.
5. Its seed ran to its final reply, with no error reply: complete.
6. In the control, `plans/c` is missing: it was put after the answer was
   made and before the subscription existed.

*The reference (`zenkey-fleet/tests/seeding.rs`) runs it through the
fleet's seeded subscription, the one zengui's tree seeds through, and the
control on the bare session.*

## §11 A late joiner's merge: unstamped samples, equal times, a key that is not concrete (§4.2)

**Setup.** No zk2 owner: foreign keys (§1.7), `foreign/*`, outside S4.
Their writer W is a session with no HLC, so its puts and deletes arrive
unstamped unless it sets a stamp, and it answers GETs on `foreign/**` as a
storage would. A tool, the late joiner, wants them and every change after.
The test holds W's answer to a GET until it releases it. The reference
plays W from one peer session, the late joiner a client of it.

**Steps.**
1. The late joiner subscribes to `foreign/**`, then GETs it with `All` +
   `Latest`. W takes the GET and holds it.
2. W puts `foreign/a` unstamped, then `foreign/c` unstamped twice
   (`c-1`, `c-2`).
3. W answers: `foreign/a` stamped T1, `foreign/b` stamped T2, and
   `foreign/c` unstamped (`c-0`, a value it held from before step 2). It
   ends the GET.
4. W puts `foreign/a` unstamped, deletes `foreign/b` unstamped, then puts
   `foreign/b` stamped T3 > T2.
5. **Equal times.** With one time t and two ids, `2` and `100` (hex), W
   puts `foreign/e` at (t, `2`) then (t, `100`), and `foreign/f` at
   (t, `100`) then (t, `2`).
6. **A key that is not concrete.** W puts `foreign/g` stamped T1, then
   on the wildcard key `foreign/*` stamped T3, then `foreign/g` stamped
   T2.

**Expected.**
1. —
2. Each is kept as it arrived: `a`, then `c-1`, then `c-2`.
3. `a` is replaced by the stamped reply (T1); `b` is kept (T2); `c` is
   replaced by `c-0`: two unstamped samples have no order but their
   arrival, and the reply arrived last.
4. Neither unstamped sample is kept, on `a` or `b`: each key holds a
   stamp. The stamped put on `b` (T3) is kept.
5. On `e`, only (t, `2`) is kept; on `f`, both are, the second replacing
   the first. At one time, id `100` is below id `2`: zenoh compares the
   ids' bytes from the least significant, `00` below `02`. By the ids'
   values, the other way round, `e` would keep both and `f` one.
6. A tool's seed keeps all three: the wildcard put under `foreign/*`,
   never merged with the keys it covers, so `foreign/g` at T2 is kept
   after it. R6 binds a consumer, which discards the wildcard put.

*The reference (`zenkey-fleet/tests/seeding.rs`) runs steps 1–4 as
`s11_an_unstamped_sample_ranks_below_every_stamp`, step 5 as
`s11_equal_times_order_by_the_id_as_zenoh_does` and step 6 as
`s11_a_tools_seed_keeps_a_key_that_is_not_concrete`, through the fleet's
seeded subscription. `conformance/seed/merge.json` states the same order
as a pure fixture.*

## §12 The check's reads beside a router that is declaring (§4.2)

**Setup.** As above, on one router with its admin space on, read
permitted, whose storage manager runs one storage, `plans`, with
`key_expr` `zk2/ground/fleet-mgr/**`. The owner serves its state, so it
holds a queryable ending in `**`,
`zk2/ground/fleet-mgr/mission_plan.v1/state/**`, and it holds
`plans/vehicle-01`. Another client holds 200 subscribers and 200
queryables on keys ending in `**` (`load/<i>/**`), so that a reply loop
over the router's declaration records would have work. A third client
declares and undeclares a subscriber and a liveliness token, flat out,
throughout. A tool, the reader under test, makes S4's check reads. A
session of the test's own holds a queryable on `@/**` that answers nothing
and records the selector of every GET it receives.

**Steps.** 200 times:
1. The reader reads `@/*/router`, then
   `@/*/router/status/plugins/storage_manager/storages/**`.
2. A client puts on a key another client subscribes to.
3. The consumer GETs its plan (S4).

**Expected.**
1. Every read of the first selector finds the router, and every read of
   the second finds `plans` with its `key_expr`.
2. The router keeps routing throughout: in every round the put arrives,
   and the owner answers the GET, within 2 s of being made.
3. Every selector the queryable on `@/**` heard is one of the two above:
   none intersects a node's declaration records, its `linkstate/*` or its
   `route/successor/**`. The queryable may never hear a read that freezes
   the router: a client's queryable never did, though one on the router's
   own session under an access control block did. Expected
   2 fails that reader either way. A runner that knows how many reads the
   reader makes counts them heard: two a round for S4's check.

*Measured.* The hazard this section guards against was measured with
zenoh alone, never in a suite: a reader that asks one of those keys
beside a writer freezes the router for good, and every session through
it (`docs/zk2/upstream/adminspace-reply-repro/`, zenoh 1.10.1, beside 500
subscribers and queryables and a client churning declarations). A wider
storages selector, `@/*/router/**/storage_manager/storages/**`, froze the
router in 10 of 10 trials beside records ending in `**`, at the first
GET; the declaration-record selectors and `linkstate/*`, with a record to
send, in 135 of 135 across every cell. The two selectors above froze it
in 0 of 10 (five each, the status subtree beside the same records ending
in `**`), and with the replies sent after the lock is dropped nothing
froze in 138 trials. The reference ran this section 96 times, eight at a
time, in a debug build: every round passed, the slowest put crossing in
68 ms and the slowest S4 answer in 82 ms, beside 1,692 to 2,588 rounds of
the writer per run.

*The reference (`zenkey-fleet/tests/adminspace.rs`) runs it as
`s12_the_checks_reads_beside_a_router_declaring`, through the fleet's
`routers` and `storages`, which the doctor and `zenctl storage list` read
through. An in-process router loads no plugin, so a session of the test's
plays the storage manager's document on the router's own key. It never
asks that wider selector: that read freezes the router, and the suite
with it.*

## §13 A stream key's seq wraps (§4.3)

**Setup.** An owner with a plain stream, through one router. A tool
merges it as a late joiner does (observer §4.1). For steps 1 and 2,
a session of the test's own plays the owner, writing envelopes, as §10's
does.

**Steps.**
1. The owner publishes `pose` at seqs 16,777,214, 16,777,215, 0 and 1 of
   one run: the key's seq wraps.
2. It publishes `heading` at seq 5, then at seq 8,388,613, which is
   2^23 apart.
3. A candidate of a governed interface (`redundancy.v1`) serves, and
   publishes a stream key at seqs 0, 1 and 2. It steps down, then takes
   over again within the same instance. Its run is unchanged, as its
   descriptor says. Then it publishes the key once more. Likewise, a
   templated stream member removed, then added again within the run,
   publishes once more.

**Expected.**
1. The tool's merge keeps all four samples of `pose`. Seq 0 is newer
   than seq 16,777,215, since (0 − 16,777,215) mod 2^24 is 1.
2. The tool keeps both samples of `heading`. At exactly 2^23 apart they
   are unordered, and the merge keeps the later arrival.
3. The sample after the takeover comes at seq 3 of that run. The
   re-added member's comes at the seq after its last, never at 0.

*The reference runs steps 1 and 2 through the fleet's seeded
subscription (`zenkey-fleet/tests/seeding.rs`,
`s13_a_stream_keys_seq_wraps_in_a_tools_merge`), and step 3 as
`zenkey/tests/profile_redundancy.rs`'s
`a_stream_keys_seq_continues_across_a_takeover`.
`conformance/seed/inband.json` states the order as a pure fixture.*
