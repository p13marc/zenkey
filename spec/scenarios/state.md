# State scenarios (core §4)

Common setup: an owner `ground/fleet-mgr` with a state template
`plans/{vehicle}` and a tombstone window W (60 s unless stated). Its
session's timestamping is off, and its runs come from a record unless
stated. The template declares no `freshness.ttl_s`, so core §4.3's table
does not list it. The owner answers a GET of it by snapshot (core §4.2):
its collection's snapshot key is
`zk2/ground/fleet-mgr/mission_plan.v1/state/plans`, and its page size
1,024 bytes. A consumer `vehicle-01/executor`, bound with
`{vehicle} = self.system`, its session's timestamping off too, zenoh's
default for a client (core §4.1). Every state GET is made with target
`All` and consolidation `None` (S4).

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
   subscribes before, and GETs its plan after v2.
2. **The control:** a third client of the router puts on a key of its own
   without setting a timestamp, and the consumer subscribes to it.
3. The owner puts v3 and v4 back to back, with nothing between them.
4. The owner puts `status` twice, and the consumer GETs it.

**Expected.**
1. Every put's payload is an envelope of the mutation form, whose run is
   the descriptor's `run`. The delete's attachment is a delete envelope of
   the same run. Each seq is above the one before.
2. The GET after v2 is a member read. It returns a snapshot of the plan
   alone, on the plan's own key: one page, whose one entry has the empty
   id, v2's seq and v2's record.
3. Through R1, no put, delete or reply of `plans/{vehicle}` carries a
   Zenoh timestamp. The owner sets none, and R1 adds none. Through R2,
   each of its puts arrives with R2's zid, and its delete and its reply
   with none. The control's put arrives with no timestamp through R1, and
   with R2's zid through R2.
4. v4's seq is v3's plus 1.
5. `status` is a per-key resource (core §4.2). Through either router,
   each put and the reply of `status` carry a Zenoh timestamp whose id is
   the owner session's zid. The second put's is above the first's. The
   reply is the second put's bytes, its envelope included.

*The reference (`zenkey/tests/state.rs`) plays both as
`s1_in_band_stamps_and_no_per_put_hlc`.*

## §2 Deletes inside the window, and the collection (S2, S3)

**Steps.** The owner holds `plans/a` and `plans/b`, and deletes `plans/b`.
Within W, the consumer GETs `plans/b`, then the selector `plans/*`, then
`plans/**`.

**Expected.**
- For `plans/b`, a member read: a snapshot on `…/plans/b` whose one entry
  is a tombstone, the empty id with the delete's run and seq.
- For `plans/*`, a member read: a snapshot of each member on its own key.
  `plans/a`'s lists its put's record and stamp, and `plans/b`'s its
  tombstone.
- For `plans/**`, a collection read: one snapshot on `…/state/plans`, one
  page. It lists `a` live, with the record the owner put, and `b` as a
  tombstone, each with its stamp.

*The reference (`zenkey/tests/state.rs`) runs it as
`s2_a_reply_del_carries_the_delete_envelope`, a name kept from before
snapshots.*

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
2. The consumer's query carries target `All` and consolidation `None`,
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
1. The archive re-reads what it records of the owner's collection, the
   one plan, by a member read of its key, before serving it as confirmed.
   The owner's snapshot of the plan holds its tombstone, and the archive
   drops it. Should the owner's
   token arrive before the route to its state queryable, the first read
   returns empty, and a retry drops the plan.
2. The empty reply sets drop nothing, however often the archive retries.
   The archive keeps serving the plan, with `confirmed: false`.
3. When the link heals, the vehicle's archive aligns from the owner-side
   archive, reading it through the collection's archive form (core §4.4).
   That archive answers with the `reply_del`, which is positive evidence,
   and the vehicle's archive drops the plan.

*The reference (`zenkey/tests/state.rs`) refuses every read in step 2: a
refusal of one read alone would be healed by the retry.*

## §6 A window shorter than the outage (S3, §4.4)

**Setup.** As §5. The owner-side archive of §5.3 keeps each tombstone for
W. The owner also serves §1's `status`, a per-key resource (core §4.2),
which the vehicle's archive records too.

**Steps.** As §5.1, but the link stays cut for longer than 60 s, and the
owner deletes `status` too while it is cut.
1. W is left at 60 s, and the owner is back at the heal.
2. W is left at 60 s, and the owner stops before the heal, as in §5.3.
3. As step 2, with W configured to the face's maximum outage.

**Expected.**
1. The owner answers the plan's key with a snapshot of no entry:
   positive evidence, whatever W. The archive drops the plan. The
   owner answers nothing for `status`, whose tombstone is gone: there is
   no positive evidence. The archive keeps serving `status`, with
   `confirmed: false`.
2. The owner-side archive answers per key, and its tombstones are gone
   with W. The vehicle's archive keeps serving the plan and `status`, with
   `confirmed: false`.
3. The owner-side archive answers each with its `reply_del`, and the
   vehicle's archive drops both.

The unconfirmed keys are the failure that rule S3's raised window
prevents: a per-key resource's, and one aligned from a peer archive.

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
5. **Inside the seed.** The owner writes rev 10. A consumer and a tool
   each subscribe, then make S4's GET. The test holds the owner's answer,
   a snapshot whose entry is rev 10, as §10 holds one. While it is held,
   the owner restarts and writes rev 11 under its new run, which both
   hear live. Then the test releases the old answer, and the owner writes
   rev 12.
   - Under `None`, the answer arrives when it is released, after rev 11's
     live sample (core §4.1).
   - The consumer accepts rev 11, then the answer's entry, which carries
     the old run. It holds rev 10 until rev 12, which it accepts.
   - The tool keeps the later arrival too: it holds rev 10, the entry that
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

## §8 Events replay (§2.6)

**Setup.** An owner `lab/journal` publishes occurrences of
`events/entries`, `retention = "1h"`. Each rides its occurrence chunk's
key (core §1.2). Its payload is an event (§7.5's form 4), with its time.
A union storage runs on `zk2/*/*/*/events/**`. Its backend ignores
`_time`, as the storage manager's memory backend does (spike S5). A
consumer replays with a wildcard GET bounded by the retention. T is the
time of step 1.

**Steps.**
1. The owner publishes 1,000 occurrences, each with a time of its own,
   as an owner writes one it relays (core §2.6, "An occurrence seen
   earlier"; the reference's `put_at`). 500 are within the hour, a
   second apart, the newest at T. 500 are older, two hours and more
   before T. The owner writes the recent ones newest first, so among
   them a later seq carries an earlier time.
2. The owner publishes one occurrence of an unknown time.
3. The owner restarts, so a new instance writes under a new run (§4.3).
   It publishes two occurrences, dated T − 100.5 s and T − 0.5 s.
4. The consumer subscribes to the event. Another session then puts two
   samples on keys of `events/entries` whose occurrence chunks no
   occurrence took. One is a mutation (§7.5's form 0) under
   `zenoh/bytes`. The other is a JSON document under `application/json`,
   with no envelope. It then deletes a third such key, its attachment a
   delete (§7.5's form 1).
5. The consumer replays the event.

**Expected.**
1. Each occurrence's key ends in its run and seq: 8 characters under a
   16-bit run, or 12 under a 32-bit one. Its payload carries its time,
   or the unknown time for step 2's.
2. The replay holds 503 occurrences: step 1's 500 recent ones, step 2's
   and both of step 3's. The backend kept all 1,005 puts, step 4's two
   among them. The consumer dropped the 500 beyond the retention, by each
   event's time.
3. The 502 with a time come in the order of their times, across the run
   change and against the seqs. So step 1's recent ones come oldest
   first, the reverse of the order the owner wrote them in. Step 3's
   first comes between step 1's at T − 101 s and T − 100 s, and its
   second between T − 1 s and T.
4. Step 2's comes last. An unknown time is never judged within the
   retention or beyond it, and comes after every known one.
5. None of step 4's three samples is delivered, by the subscription or
   by the replay. The consumer counts each undecodable, refused as
   `form`: each carries no event, in an envelope §7.5's parse accepts or
   in none (core §2.6).

*Spike S5: the union replay returned 1,000 in 4 ms; the memory backend
ignored the `_time` bound. The reference runs §8 in `zenkey/tests/state.rs`
(`s8_events_replay`). Step 4's delete is written ahead of the reference.*

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
1. The late joiner subscribes to `plans/*`, then GETs `plans/**`, a
   collection read.
2. The owner takes the GET and holds it. It puts `plans/d` (v1, seq 3).
3. The owner makes its answer, a snapshot as of seq 3: `a` (v1, seq 1),
   a tombstone `b` (seq 2), and `d` (v1, seq 3).
4. Before it sends the answer, the owner puts `plans/a` (v2, seq 4), then
   `plans/c` (v1, seq 5).
5. The owner sends its answer and ends the GET.
6. **The control:** the other order. The late joiner GETs first; the owner
   holds the GET, makes its answer (`a` and the tombstone `b`), puts
   `plans/c` (v1, seq 5), and answers; the late joiner subscribes once the
   answer has arrived.

**Expected.** When the GET has ended, the late joiner holds:
1. `plans/a` as v2 with seq 4. The entry (seq 1) is older than the live
   put, and is dropped by the merge, never shown after v2.
2. `plans/c` as v1 with seq 5: the put the snapshot does not hold arrived
   through the subscription. The omission rule keeps it: its seq is above
   the snapshot's asof, 3.
3. `plans/b` as a deletion with seq 2: the tombstone reached the merge as
   a deletion, never as an absence.
4. `plans/d` as v1 with seq 3, once: the live put and the entry carry one
   stamp, and are one mutation.
5. The snapshot complete at its one page, before the GET's final reply.
6. In the control, `plans/c` is missing: it was put after the answer was
   made and before the subscription existed.

*The reference (`zenkey-fleet/tests/seeding.rs`) runs it through the
fleet's seeded subscription, the one zengui's tree seeds through, and the
control on the bare session. `conformance/seed/snapshot.json` states the
merge and the omission rule as a pure fixture.*

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

## §14 A collection's snapshot, its pages, and a seed from it (S2, S4, §4.2, §7.5)

**Setup.** As above. The owner holds 32 plans, put under one run, each a
record of 110 bytes: `plans/v00` to `plans/v30`, and `plans/vehicle-01`,
the consumer's. Its pages then hold 8 entries each, at most 975 bytes
under a 16-bit run and 977 under a 32-bit run. A tool T, a late joiner,
subscribes to `plans/*` before each GET. For steps 3, 5 and 7, a session of the
test's own plays the owner, writing the snapshots it is given, as §10's
does.

**Steps.**
1. **A collection read.** T GETs `plans/**`.
2. **A member read.** T GETs `plans/v07`, then `plans/*`, then
   `plans/v99`, which the owner does not hold.
3. **A page lost.** The owner's answer to T's `plans/**` holds pages 0,
   1 and 3 of its 4. T holds `plans/v99` from an earlier seed, which the
   owner no longer has.
4. **The control: `Latest`.** As step 1, with T's GET under `Latest`.
5. **A delete beyond the window.** T holds the 32 plans of step 1, and
   the consumer its plan. Their links to the router are cut for longer
   than W. Meanwhile the owner deletes `plans/v05` and
   `plans/vehicle-01`. At the heal T seeds again, and the owner puts
   `plans/v31` between taking T's GET and sending its answer. The
   consumer GETs its plan again.
6. **An empty collection.** The owner deletes every plan, and W passes.
   T GETs `plans/**`.
7. **Two instances answer.** As step 1, but a second instance of the
   owner, under another run, answers too: its snapshot lists
   `plans/x1` alone.

**Expected.**
1. One snapshot on `…/state/plans`, in 4 pages with indices 0 to 3, the
   last with its more bit 0. Each page is within 1,024 bytes, and all four
   have one run, asof and time. Each live entry's id is a vehicle, its seq
   its put's, and its confirmation 0. T holds the snapshot complete at its
   last page, before the GET's final reply, and holds the 32 plans.
2. `plans/v07`: one snapshot on `…/plans/v07`, one page whose one entry
   has the empty id. `plans/*`: 32 such snapshots, each on its plan's own
   key, and none drops another plan T holds. `plans/v99`: one page with
   no entry, on `…/plans/v99`.
3. T holds the snapshot incomplete. It merges the entries of pages 0, 1
   and 3, and drops nothing: it still holds `plans/v99`. It reads the
   collection again, and that snapshot, complete, drops `plans/v99`.
4. T receives one page of the four: `Latest` keeps one reply per key, and
   the four share theirs. Its index shows the snapshot incomplete, so T
   drops nothing (core §4.2, "Why `None`").
5. The owner's tombstones went with W, and its snapshot lists the 30
   plans left. T's omission rule drops `plans/v05` and `plans/vehicle-01`:
   the deletes reach T beyond the window. `plans/v31`, heard live at a seq
   above the snapshot's asof, is kept, though the snapshot does not list
   it. The consumer's member read draws one page with no entry on
   `…/plans/vehicle-01`, and the consumer drops its plan.
6. One page with no entry, complete: the owner's word. T's omission rule
   drops every plan it held.
7. Both snapshots arrive, one for each run, and both are complete. Each
   one's omission rule spares the other run's plans, which T heard after
   it asked the GET. T holds the 32 plans and `plans/x1`, whichever
   snapshot arrived first.

*The reference runs steps 1, 2 and 4 in `zenkey/tests/state.rs`, and
steps 3 and 5 to 7 through the fleet's seeded subscription
(`zenkey-fleet/tests/seeding.rs`). `conformance/seed/snapshot.json` states
the merge, pages and the omission rule as a pure fixture.*

## §15 A `reply_del` where the owner answers by snapshot (§4.2)

**Setup.** As §14: the owner holds the 32 plans, and T reads its state.
For step 1, a session of the test's own plays the owner, as in §14.

**Steps.**
1. **A stray `reply_del`.** T GETs `plans/**`. The owner's answer holds
   the collection's snapshot, complete, and a `reply_del` on `plans/v03`
   under the owner's run, which no owner sends.
2. **An owner on an earlier revision.** The owner also serves `status`, a
   state with `freshness.ttl_s` 60, under a revision whose contract uses
   `freshness.v1`, which its descriptor names. T holds that revision, and
   a later one of the same interface whose contract uses `freshness.v2`
   instead. The owner deletes `status`, and within W T GETs its
   interface's `state/**`.

**Expected.**
1. T holds the snapshot complete and the 32 plans, `plans/v03` among
   them. It counts the `reply_del` undecodable, refused as `form`, and
   merges no deletion of `plans/v03` (core §4.2).
2. By the descriptor's revision, `status` is a per-key resource. Its owner
   answers it with a `reply_del` within W (core S2, S3), and T merges it as
   a deletion. Read by the later revision, which T holds too, it would be
   a stray `reply_del` (core §4.2).

*The reference runs step 1 as
`a_reply_del_on_a_collections_member_is_undecodable` in
`zenkey/tests/snapshot.rs`, on a snapshot of three plans. Step 2 is
written ahead of the reference.*
