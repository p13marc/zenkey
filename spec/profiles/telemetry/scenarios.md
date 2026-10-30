# `telemetry.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. The reference runtime runs §1
to §9 as tests of `zenkey` (`zenkey/tests/profile_telemetry.rs`, one test
per section, named after it: `s1_counter_across_a_restart` to
`s8_counter_across_a_face`, and `s9_state_counter_seeded`). §8 is also a
tool's: the reference tool, `zenctl telemetry --across-face`, runs it over
a real face (`zenctl/tests/live_zk2.rs`,
`telemetry_s8_a_counter_that_crosses_a_face`).

**Conventions.**
- **Section numbers** in parentheses (§2.10) are v1.md's rules; core
  sections say "core".
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. The owner O and the readers are its clients. Timeouts are 1 s
  unless stated.
- **The contract.** Every section uses `meter.v1`. The reference keeps it
  as `zenkey/tests/contracts/meter.v1.toml`, beside `meter.json`, and
  lists the profile's file by its path from there:

  ```toml
  [interface]
  name  = "meter"
  major = 1
  minor = 0
  uses  = ["telemetry.v1"]

  [schemas]
  jsonschema = ["../../../spec/profiles/telemetry/telemetry.v1.json", "meter.json"]

  [resources.frames]                       # a Point, declared a counter
  kind        = "stream"
  type        = "json:telemetry.v1.Point"
  annotations = { "telemetry.kind" = "counter", "telemetry.unit" = "1" }

  [resources.bytes]                        # a bare counter, its epoch in the attachment
  kind        = "stream"
  type        = "json:Count"
  attachment  = "json:telemetry.v1.Epoch"
  annotations = { "telemetry.kind" = "counter", "telemetry.unit" = "By" }

  [resources.drops]                        # a bare counter, no epoch
  kind        = "stream"
  type        = "json:Count"
  annotations = { "telemetry.kind" = "counter" }

  [resources.latency]                      # a Point, declared a histogram
  kind        = "stream"
  type        = "json:telemetry.v1.Point"
  annotations = { "telemetry.kind" = "histogram", "telemetry.unit" = "s", "telemetry.buckets" = [0.005, 0.01, 0.025] }

  [resources.level]                        # a bare gauge
  kind        = "stream"
  type        = "json:Level"
  annotations = { "telemetry.kind" = "gauge", "telemetry.unit" = "dBm" }

  [resources.total]                        # a bare counter in state (0.2, §9)
  kind        = "state"
  type        = "json:Count"
  annotations = { "telemetry.kind" = "counter" }
  ```

  `meter.json` is
  `{"$defs": {"Count": {"type": "integer", "minimum": 0}, "Level": {"type": "number"}}}`.
- **The owner** is `lab/meter`, implementing `meter.v1` with every
  resource exposed. Its keys are
  `zk2/lab/meter/meter.v1/stream/{frames,bytes,drops,latency,level}` and
  `zk2/lab/meter/meter.v1/state/total`, whose GETs it answers (core S2). It
  writes each Point's `timestamp` from its own clock when it takes the
  value, and its `epoch` as its instance id (§2.9). "I1", "I2" name its
  instance ids in the order it mints them.
- **R** subscribes to `zk2/lab/meter/meter.v1/stream/*`, and holds a
  liveliness subscription to `zk2/lab/meter/@zk/instance/*`, with history,
  both declared before the owner starts. Then it makes a complete
  liveliness read of the same tokens (core §8.1), on which its view of
  presence stands (§2.9): a subscription's history alone is not a
  complete read. It records each sample's arrival on its monotonic
  clock, its key, its payload and its attachment, and each token's
  appearance and withdrawal.
  - "R reads" a sample: v1.md §2.6, against the resource's declaration.
  - "R pairs": §2.9 to §2.11, over each two consecutive conforming samples
    of a member, in arrival order. Its presence is what R saw after its
    complete read: alone throughout when exactly one instance token was
    held at every instant from before the earlier arrival to after the
    later one's, with no liveliness event lost; earlier and later the only
    token held at each arrival. The read finds the tokens held when it is
    made, and the subscription's moves are folded after it, the
    subscription's word winning where the two disagree.
- **R0** subscribes to the same data keys, and reads no presence: it pairs
  with none (§2.9, 3), as a reader across a constrained face does
  (§2.14).
- **Raw puts.** A step marked *raw* puts its payload on the owner's key
  from the owner's session, bypassing any check its runtime's writer
  makes: it is what an owner the text does not allow would send. A runtime
  whose writer refuses such a value refuses it in the same step, and the
  scenario records the refusal.
- **Spacing.** Samples of one member are published at least 200 ms apart,
  and a section waits 500 ms after a token's withdrawal or appearance
  before the next put, so that every arrival falls clear of a token's
  change. Jitter on loopback is allowed 200 ms.

## §1 A counter across a restart (§2.9, §2.10, §2.11; core §1.5)

**Steps.**
1. O starts, as I1. It publishes `frames` at 0, 10 and 20, and `drops` at
   0, 5 and 9, 1 s apart.
2. O's process ends: its session closes, and R sees I1's token withdrawn.
3. O starts again, as I2. It publishes `frames` at 0 and 4, and `drops` at
   0 and 2, 1 s apart.

**Expected.**
1. Every sample conforms. Each `frames` Point carries I1 as its `epoch` in
   step 1, and I2 in step 3. I1 ≠ I2.
2. Within step 1, R pairs `frames` as `same_epoch`, established in-band,
   with a rate by the Points' timestamps (§2.11) in `1/s`: 10 over the
   difference of the two timestamps. It pairs `drops` as `same_epoch`,
   established by presence (I1 alone throughout), with a rate by the
   arrivals, and no unit.
3. Across the restart, R pairs `frames` 20 → 0 as `reset`, established
   in-band, and `drops` 9 → 0 as `reset`, established by presence (I1 alone
   at the earlier arrival, I2 at the later). Neither has a rate.
4. Within step 3, both pair `same_epoch` again, established, with rates.
5. R pairs nothing `decreased`, and derives no rate across the restart.

## §2 A counter across `new_epoch` in one process (§2.9, §2.10; core §1.5, §8.1)

**Steps.**
1. O starts, as I1. It publishes `frames` at 0, 10 and 20, and `drops` at
   0, 5 and 9.
2. O starts a new epoch while running (core §8.1, make-before-break): I2's
   descriptor and tokens are declared, then I1's withdrawn. R sees both.
3. 500 ms after I1's withdrawal, O publishes `frames` at 0 and 3, and
   `drops` at 0 and 3.

**Expected.**
1. Every `frames` Point after step 2 carries I2 as its `epoch`, the
   process unchanged.
2. R pairs `frames` 20 → 0 as `reset`, established in-band, and `drops`
   9 → 0 as `reset`, established by presence: I1 alone at the earlier
   arrival, I2 alone at the later. Neither has a rate.
3. Then each pairs `same_epoch`, established, with a rate.
4. R0 pairs `drops` 9 → 0 as `reset`, assumed: the same answer without
   the evidence.

A sample arriving while both instances' tokens are held establishes
nothing by presence (§2.9, 3). The overlap is the runtime's, so this
section publishes outside it.

## §3 A decrease within one epoch (§2.7, §2.10)

**Steps.**
1. O starts, as I1. It publishes `frames` at 10, and `drops` at 10.
2. *Raw:* O's session publishes `frames` at 7, a Point carrying I1 as its
   epoch, and `drops` at 7.

**Expected.**
1. Every sample conforms: conformance is per sample (§2.6).
2. R pairs `frames` 10 → 7 as `decreased`, established in-band, and
   `drops` 10 → 7 as `decreased`, established by presence: the finding.
   Neither has a rate.
3. R0 pairs `drops` 10 → 7 as `reset`, assumed: never a finding, and no
   rate (§2.10). It pairs `frames` 10 → 7 as `decreased`, established
   in-band: the epoch rides the Point.
4. A writer that holds its owner to §2.7 refuses both puts of step 2.

## §4 A Point whose tag disagrees (§2.5, §2.6)

**Steps.**
1. O starts, as I1, and publishes `frames` at 1.
2. *Raw:* `frames`, a Point tagged `gauge`, value 3.5.
3. *Raw:* `frames`, a Point tagged `counter`, value 2, with `unit` `By`.
4. *Raw:* `level`, the string `"x"`.
5. O publishes `frames` at 5.

**Expected.**
1. R reads step 2 as `does_not`, `tag_disagrees`. The member's kind stays
   `counter`, the declared one.
2. R reads step 3 as `does_not`, `unit_disagrees`. The member's unit stays
   `1`.
3. R reads step 4 as `does_not`, `wrong_type`.
4. None of them pairs: R pairs `frames` 1 → 5, step 1's with step 5's, as
   `same_epoch`.
5. A writer that checks its values against the declaration refuses steps
   2 to 4 before sending.

## §5 A histogram over other bounds, and one that goes backwards (§2.6, §2.8, §2.10)

**Steps.** O starts, as I1, and publishes on `latency`, each a Point
tagged `histogram` with I1 as its epoch:
1. over `[0.005, 0.01, 0.025]`: counts `[3, 10, 2, 1]`, count 16, sum
   0.19;
2. *raw:* over `[0.005, 0.01, 0.05]`, the same counts;
3. *raw:* over the declared bounds: counts `[3, 9, 2, 1]`, count 15, sum
   0.18;
4. over the declared bounds: counts `[4, 12, 2, 1]`, count 19, sum −1.0.

**Expected.**
1. Steps 1, 3 and 4 conform. R reads step 2 as `does_not`,
   `bounds_differ`.
2. R pairs step 1 with step 3 as `decreased`, established in-band: a
   bucket's count and `count` fell.
3. R pairs step 3 with step 4 as `same_epoch`: every count grew, and `sum`
   fell, which it may.
4. No pair has a rate: a histogram has none at this version (§2.11).
5. A writer that checks refuses steps 2 and 3.

## §6 Absence is not zero (§2.12)

**Steps.**
1. O starts, as I1. It publishes `level` at −70.0 once, and `drops` at 5,
   6 and 7, 1 s apart.
2. O closes its service.
3. R waits 3 s.

**Expected.**
1. R's last sample of `level` is −70.0 throughout. No step gives `level`
   another value: a reader shows −70.0 with its age, or nothing, and
   never 0.
2. R pairs `drops` 5 → 6 and 6 → 7, and nothing after 7: a member with no
   sample forms no pair, and no rate of 0 is derived from the silence.
3. After step 2, R has no new sample of any member, and reads none as 0.

## §7 An in-band epoch on a bare counter (§2.4, §2.9)

**Steps.**
1. O starts, as I1. It publishes `bytes` at 100 and 200, each with I1 as
   its attachment.
2. O starts a new epoch, as in §2 step 2. 500 ms later, it publishes
   `bytes` at 0 and 50, each with I2 as its attachment.
3. *Raw:* `bytes` at 60, with the JSON string `""` (two bytes) as its
   attachment.

**Expected.**
1. Each attachment of steps 1 and 2 is the owner's instance id when it
   published.
2. R0, with no presence, pairs `bytes` 100 → 200 as `same_epoch`,
   established in-band, with a rate in `By/s`; 200 → 0 as `reset`,
   established in-band; 0 → 50 as `same_epoch`, established in-band.
3. R reads step 3 as `does_not`, `epoch_invalid`, and pairs it with
   nothing.

## §8 A counter that crosses a face (§2.10, §2.14)

**Setup.** R0 stands where a reader across a constrained face stands
(core §8.5): it receives the data keys and nothing under `@zk`. The
reference tool, `zenctl telemetry --across-face`, stands it behind a face
whose access control denies `@zk/**` (`link.v1` §2.6); the
runtime runs it as R0 alone, which the profile reads alike (§2.14).

**Steps.**
1. O starts, as I1. It publishes `drops` at 10 and 20, and `frames` at 10
   and 20.
2. O's process ends. O starts again, as I2, and publishes `drops` at 3
   and 8, and `frames` at 3 and 8.

**Expected.**
1. R0 pairs `drops` 10 → 20 as `same_epoch`, assumed, with a rate; 20 → 3
   as `reset`, assumed, with no rate; 3 → 8 as `same_epoch`, assumed.
2. R0 pairs `frames` 20 → 3 as `reset`, established in-band: the Point's
   epoch crossed with it.
3. R0 pairs nothing `decreased`, and derives no negative rate.

## §9 A state counter seeded (§2.10; core §4.2)

**Setup.** R is a late joiner of `total` (core §4.2): it subscribes to
`zk2/lab/meter/meter.v1/state/total`, then GETs it as core S4 says (target
`All`, consolidation `Latest`), and merges the GET's replies and the
subscription's samples by the owner's stamp, whichever path brought each.
It still holds its liveliness subscription.

**Steps.**
1. O starts, as I1, and puts `total` at 10.
2. R subscribes, then GETs `total`. O answers 10, with that put's stamp;
   R holds the reply, unread.
3. O puts `total` at 12, which R's subscription delivers.
4. R reads the GET's reply, 10. 200 ms later, O puts `total` at 15.

**Expected.**
1. R received 12, then 10, then 15. Every sample conforms.
2. Through the merge, the reply's stamp is below 12's, so R does not keep
   it: R's samples of `total` are 12 and 15, and it pairs 12 → 15 as
   `same_epoch`, established by presence (I1 alone throughout), with a
   rate by the owner's stamps (§2.11): 3 over their nanoseconds' difference.
3. Paired in the order received, 12 → 10 would read `decreased`,
   established by presence: the finding, for a counter that never went
   down. R pairs nothing `decreased`.
