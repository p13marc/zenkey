# State scenarios (core §4)

Common setup: an owner `ground/fleet-mgr` with a state template
`plans/{vehicle}`, HLC enabled, a tombstone window W (60 s unless stated).
A consumer `vehicle-01/executor`, bound with `{vehicle} = self`.

## §1 Stamped mutations (S1, S2, §4.3 minting)

**Steps.** The owner puts v1, then v2, then deletes the key. The consumer
subscribes before, and GETs with `All` + `Latest` after v2.

**Expected.**
- Every sample, the delete included, carries a timestamp whose id is the
  owner session's zid. A router stamp would carry the router's.
- Each timestamp is greater than the previous one.
- The GET after v2 returns v2 with v2's timestamp.

## §2 Deletes inside the window, and the collection (S2, S3)

**Steps.** The owner holds `plans/a` and `plans/b`, and deletes `plans/b`.
Within W, the consumer GETs `plans/b`, then the selector `plans/*`.

**Expected.**
- For `plans/b`: a `reply_del` carrying the deletion's timestamp.
- For `plans/*`: `plans/a` with its value, and a `reply_del` for `plans/b`.

## §3 The owner is authoritative (S4)

**Setup.** As above. A storage manager is deployed covering
`zk2/ground/fleet-mgr/**`.

**Steps.** A tool inspects the routers' storage admin space.

**Expected.** The tool reports the storage as a finding: no storage may
answer on an owner's state keys. Without the storage, a consumer's GET is
answered by the owner alone.

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
2. The same, but the access control refuses the archive's GET once, so the
   reply set is empty.
3. The link stays cut and the owner stops. An archive on the owner's side
   still records the plan's deletion.

**Expected.**
1. The archive re-reads the owner's collection before serving it as
   confirmed. The owner answers the plan with a `reply_del`, and the archive
   drops it.
2. The empty reply set drops nothing. The archive keeps serving the plan,
   with `confirmed: false`.
3. When the link heals, the vehicle's archive aligns from the owner-side
   archive, which answers with the `reply_del`, and drops the plan.

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
   detects the drift and stops writing state; it reports the drift (through
   `health.v1`, where it implements it).

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
