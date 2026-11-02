# Frame scenarios (core §4.3, §7.7)

An owner writes a frame resource's records as frames, and a reader holds
each field in one of four states (core §7.7). These scenarios run an owner
and a reader on one bus. The frame's body, its tag and its refusals are
static, and `conformance/frame/` checks them.

**Common setup.**
- **R1** is a router. **O** is the owner at `lab/modem`, a client of R1.
  **S** is a consumer of O, on another client.
- **The contract** is `linkframes.v1`, with two frame resources. Their
  record types are `Link` and `Sample` of
  [`conformance/contracts/schemas/packed/frames.json`](../conformance/contracts/schemas/packed/frames.json):

  ```toml
  [resources."frames/link"]
  kind  = "stream"
  type  = "packed:linkframes#Link"
  frame = { keyframe_every = 15, deadband = { queue_depth = { delta = "4", max_records = 5 } } }

  [resources."frames/sample"]
  kind  = "stream"
  type  = "packed:linkframes#Sample"
  frame = { records = 3, step_ms = 10000, jitter_ms = 500, keyframe_every = 3 }
  ```

- **A Link record** is written as its four values in order: `dropped`,
  `queue_depth`, `duty_available` and `duty_wait_ms`. **The base record**
  is 7, 12, 1, `"1500"`. A step that names a record by what changed
  changes only those values from the record before.
- **S** subscribes to O's frame keys before O's first frame unless a step
  says otherwise. Each run of these scenarios starts O under a run R
  that S has not seen.
- **A lost frame.** Where a step loses a frame on its way, O takes the
  frame's seqs and puts nothing, as a put a network dropped. The
  reference's owner does so through a test hook.

## §1 Each frame decodes alone (§4.3, §7.7)

**Steps.**
1. O records the base record on `frames/link`, then the base record with
   `queue_depth` 30 and `duty_wait_ms` `"2500"`.
2. O records 14 more records on `frames/link`, each with `queue_depth` 10
   higher than the last.
3. O records three Sample records on `frames/sample`, 10 s apart on its
   clock: `"21.5"`, -80, true, `"rx"`; then `"21.5"`, -82, true, `"rx"`;
   then `"22"`, -82, true, `"tx"`.
4. O asks its runtime for a writer of mutations on `frames/link`.
5. A reader holding nothing reads each frame of steps 1 and 3 alone.

**Expected.**
1. Two frames of one record each, at seqs 0 and 1 of R. The first is a
   keyframe: one full record. The second is a delta record that includes
   `queue_depth` and `duty_wait_ms` alone. S reads the second as
   continuing the key, every field `known`: `queue_depth` 30 and
   `duty_wait_ms` `"2500"`, and the other two as the first frame held
   them.
2. Seqs 2 to 15. The records at seqs 2 to 14 are delta records. The record
   at seq 15 is full, 15 records after the last full one.
3. One frame of three records, its seq 0 for `frames/sample`: record 0
   full, and records 1 and 2 delta records, both scheduled. Record 1
   includes `rssi` alone, and record 2 `temperature` and `mode`. The
   key's next frame takes seq 3.
4. The runtime refuses it, and no mutation is put on the key.
5. Step 1's first frame reads every field `known`. Its second reads
   `queue_depth` and `duty_wait_ms` `known`, and the other two `unknown`,
   why `first`. Step 3's frame reads every field of its three records
   `known`: a field a delta record omits holds the value of the record
   before, in the same frame.

## §2 A lost delta frame (§7.7)

**Steps.**
1. O records the base record on `frames/link`, then the base record with
   `queue_depth` 30.
2. O records `duty_wait_ms` `"2500"`. The frame is lost on its way.
3. O records `queue_depth` 40.
4. O records 12 more records, each with `queue_depth` 10 higher than the
   last.

**Expected.**
1. S reads seqs 0 and 1, every field `known`.
2. Nothing reaches S.
3. S reads seq 3 as following a gap of one record. `queue_depth` is
   `known`, 40. `dropped`, `duty_available` and `duty_wait_ms` are
   `unknown`, why `gap`: S shows and uses no value for them, neither the
   last it read nor 0.
4. The three stay `unknown` until seq 15, the next full record. From
   there every field is `known` again, `duty_wait_ms` at `"2500"`.

## §3 Scheduled and measured time (§7.7)

**Steps.**
1. O records on `frames/sample` at t, t + 10.2 s and t + 20.7 s on its
   monotonic clock.
2. O records at T = t + 30.1 s and T + 9.6 s, then at T + 35 s.
3. O ends its open frame.

**Expected.**
1. One frame of three records at seqs 0 to 2. Record 0's time is O's wall
   clock at t, from the deployment epoch. Record 1 is scheduled: it lies
   within 500 ms of t + 10 s. Record 2 is measured, its offset 20,700 ms.
   S shows record 1 at t + 10 s, scheduled, ±500 ms, and never as
   measured or at t + 10.2 s. It shows record 2 at t + 20.7 s, measured.
2. The record at T + 35 s lies past the span of 30 s, so it ends the
   frame. That frame holds two records at seqs 3 and 4, record 1
   scheduled. The record at T + 35 s opens the next frame as its record
   0.
3. That frame holds the one record, at seq 5.

## §4 The deadband (§7.7)

**Setup.** Beside O, **X** is a raw writer at `lab/rogue`, putting frames
of `frames/link` by hand. S consumes both.

**Steps.**
1. O records the base record on `frames/link`.
2. O records `queue_depth` 14, 15, 9 and 16, one record each.
3. O records `queue_depth` 13.
4. O records `queue_depth` 20.
5. X puts a keyframe of the base record at seq 0, then five delta records
   at seqs 1 to 5 that omit `queue_depth`.

**Expected.**
1. S reads `queue_depth` `known`, 12.
2. O omits `queue_depth` from seqs 1 to 4, each within 4 of 12. S reads it
   `within` at each, 12 give or take 4, and shows it with its delta.
3. O includes it at seq 5, five records after seq 0. S reads it `known`,
   13.
4. O includes it at seq 6, more than 4 from 13. S reads it `known`, 20.
5. S reads X's `queue_depth` `within` at seqs 1 to 4, and `unknown` at
   seq 5, why `deadband`.

## §5 A new run (§4.3, §7.7)

**Steps.**
1. O records the base record on `frames/link`, then `queue_depth` 30.
2. O takes a new run R′ and records `queue_depth` 30 again.
3. O takes a new run R″. Its first frame, the base record with
   `queue_depth` 40, is lost on its way. O then records `queue_depth` 50.

**Expected.**
1. S reads seqs 0 and 1 of R.
2. O's first frame under R′ is a keyframe at seq 0 of R′, though nothing
   changed. S reads it at a run it does not hold, every field `known`.
3. S reads seq 1 of R″ at a run it does not hold. `queue_depth` is
   `known`, 50, and every other field `unknown`, why `run`.

## §6 A late joiner (§7.7)

**Steps.**
1. Before S subscribes, O records the base record on `frames/link`, then
   four records, each with `queue_depth` 10 higher than the last.
2. S subscribes. O records 11 more records, each with `queue_depth` 10
   higher than the last.

**Expected.**
1. Nothing reaches S.
2. S's first frame is seq 5. It reads it as a first frame: `queue_depth`
   `known`, and `dropped`, `duty_available` and `duty_wait_ms` `unknown`,
   why `first`, never 0. They stay `unknown` until seq 15, the next full
   record, from which every field is `known`.
