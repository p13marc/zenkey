# State scenarios (core §4)

Common setup: an owner `ground/fleet-mgr` with a state template
`plans/{vehicle}`, HLC enabled, a tombstone window W (60 s unless stated).
A consumer `vehicle-01/executor`, bound with `{vehicle} = self.system`.

## §1 Stamped mutations (S1, S2, §4.3 minting)

**Setup.** The owner and the consumer are clients of R1, whose
timestamping is enabled. The owner's session is never R1 itself: there,
the owner's zid and the router's are one, and the id check below proves
nothing (core §4.2, "Observing S1").

**Steps.**
1. The owner puts v1, then v2, then deletes the key. The consumer
   subscribes before, and GETs with `All` + `Latest` after v2.
2. **The control:** a third client of R1 puts on a key of its own without
   setting a timestamp, and the consumer subscribes to it.
3. The owner puts v3 and v4 back to back, with nothing between them.

**Expected.**
1. Every sample, the delete included, carries a timestamp whose id is the
   owner session's zid, not R1's. Each timestamp is greater than the
   previous one. The GET after v2 returns v2 with v2's timestamp.
2. The control's sample carries R1's zid: the router stamps what is
   unstamped, so the check in 1 can tell the two apart.
3. v4's timestamp is greater than v3's.

**What step 3 cannot show** (0.8, F-73). Two puts that fall within one
reading of the owner's clock are where the minting rule (core §4.3) earns
its keep, by one tick. A tester can neither arrange that from outside nor
tell from the stamps whether it happened, so the scenario asks only for
the order. An implementation checks the tick with a clock it controls: the
reference does it in §7's catch-up, where its clock reads behind the
record, and the next stamp is the record plus one NTP64 unit.

*Measured (#609): the Rust owner example and zk2py's owner were each their
own router, so their S1 check proved only that the stamp's id was the
process's (F-69). zk2py's owner also runs as a client of a router, and
steps by 1 ns, its smallest; the reference steps by one NTP64 unit
(2^−32 s).*

## §2 Deletes inside the window, and the collection (S2, S3)

**Steps.** The owner holds `plans/a` and `plans/b`, and deletes `plans/b`.
Within W, the consumer GETs `plans/b`, then the selector `plans/*`.

**Expected.**
- For `plans/b`: a `reply_del` carrying the deletion's timestamp.
- For `plans/*`: `plans/a` with its value, and a `reply_del` for `plans/b`.

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
- The consumer reads the archive's key for the same plan, and gets v3 with
  v3's timestamp and its type identity in the attachment.
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

## §7 Clocks (S7)

**Steps and expected:**
1. **Catch-up.** The owner writes rev 10, restarts with its clock 5 s behind,
   reads its persistent record, then writes rev 11. Rev 11's timestamp is
   above rev 10's, and the consumer applies it.
2. **New epoch.** The owner restarts with no record and no owner-side
   archive, with a fresh zid and its clock 5 s behind, and writes rev 11.
   The consumer sees a new timestamp id, accepts rev 11, and restarts its
   ordering.
3. **Ahead.** The owner's clock runs 2 s ahead, beyond the router's HLC
   delta, and it holds a subscription to a router-stamped heartbeat. It
   detects the drift and stops writing state. Where it implements
   `health.v1`, it reports the drift as a `faults` sample with code
   `clock_ahead`, not through its status, which is state (0.23;
   `profiles/health/scenarios.md` §4).

*Spike S12: without the catch-up, rev 14 was rejected as stale and never
converged; with a clock 2 s ahead, the next right-clocked write looked
stale.*

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

**Steps.** The archive records v1 (T1) and a delete (T2). Then an older put
(T0 < T2) reaches it late.

**Expected.** The archive answers the key with a tombstone, never with the
older value.

*Spike S5: the storage manager 1.10.1 accepted the older put and resurrected
the key, which is why it cannot be an archive's store as released.*

## §10 A late joiner's seed (§4.2, 0.26)

**Setup.** As above. The owner holds `plans/a` (v1, stamp T1), and deleted
`plans/b` at T2, within W. A tool, the late joiner, wants the plans and
every change after them. The test holds the owner's answer to a GET until
it releases it, so that it can place the owner's puts between the late
joiner's steps: the reference plays the owner from a session of its own,
whose puts, replies and `reply_del` carry the stamps it mints (§4.3).

**Steps.**
1. The late joiner subscribes to `plans/*`, then GETs `plans/*` with `All`
   + `Latest`.
2. The owner takes the GET and holds it. It puts `plans/d` (v1, T3).
3. The owner makes its answer: `plans/a` (v1, T1), a `reply_del` for
   `plans/b` (T2), and `plans/d` (v1, T3).
4. Before it sends the answer, the owner puts `plans/a` (v2, T4), then
   `plans/c` (v1, T5).
5. The owner sends its answer and ends the GET.
6. **The control:** the other order. The late joiner GETs first; the owner
   holds the GET, makes its answer (`plans/a` and the `reply_del` for
   `plans/b`), puts `plans/c` (v1, T5), and answers; the late joiner
   subscribes once the answer has arrived.

**Expected.** When the GET has ended, the late joiner holds:
1. `plans/a` as v2 with T4. The reply (T1) is older than the live put,
   and is dropped by the merge, never shown after v2.
2. `plans/c` as v1 with T5: the put the GET's answer does not hold arrived
   through the subscription.
3. `plans/b` as a deletion with T2: the `reply_del` reached the merge as a
   deletion, never as an absence.
4. `plans/d` as v1 with T3, once: the live put and the reply carry one
   stamp, and are one mutation.
5. Its seed ran to its final reply, with no error reply: complete.
6. In the control, `plans/c` is missing: it was put after the answer was
   made and before the subscription existed.

*The reference (`zenkey-fleet/tests/seeding.rs`) runs it through the
fleet's seeded subscription, the one zengui's tree seeds through, and the
control on the bare session.*

## §11 A late joiner's merge: unstamped samples, equal times, a key that is not concrete (§4.2, 0.27)

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
