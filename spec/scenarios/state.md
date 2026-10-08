# State scenarios (core §4)

Common setup: an owner `ground/fleet-mgr` with a state template
`plans/{vehicle}`, HLC enabled, a tombstone window W (60 s unless stated).
A consumer `vehicle-01/executor`, bound with `{vehicle} = self`.

## §1 Stamped mutations (S1, S2)

**Steps.** The owner puts v1, then v2. The consumer subscribes, then GETs
with `All` + `Latest`.

**Expected.** Every sample and every GET reply carries a timestamp set by
the owner. The GET returns v2 with v2's timestamp.

## §2 Deletes inside the window (S3)

**Steps.** The owner deletes the key. Within W, the consumer GETs.

**Expected.** A `reply_del` carrying the deletion's timestamp.

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

**Steps.** With the link cut, the owner deletes the plan within W. The link
heals within W.

**Expected.**
- The archive re-reads the owner's collection before serving it again. The
  owner's GET completes without a timeout and returns a `reply_del`.
- The archive drops the plan.
- A GET whose reply set timed out MUST NOT make the archive drop any key.

## §6 A window shorter than the outage (S3)

**Steps.** As §5, but the link stays cut for longer than W, with W raised
for this key by the face policy to the face's maximum outage.

**Expected.** After the heal, the owner still answers the deleted key with
`reply_del`, and the archive drops it. Run again with W *not* raised: the
archive keeps serving the deleted plan. That second run is the failure the
rule exists to prevent.

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
   detects the drift, stops writing state, and reports through `health.v1`.

*Spike S12: without the catch-up, rev 14 was rejected as stale and never
converged; with a clock 2 s ahead, the next right-clocked write looked
stale.*

## §8 Events replay

**Setup.** An owner publishes 1,000 occurrences of an event with
`retention = "1h"`, and a union storage runs on `zk2/*/*/*/events/**`.

**Steps.** A consumer replays with a wildcard GET bounded by the retention.

**Expected.** Exactly the occurrences within the retention. With a backend
that ignores `_time`, the consumer filters by the ULID's time.

*Spike S5: the union replay returned 1,000 in 4 ms; the memory backend
ignored the `_time` bound.*
