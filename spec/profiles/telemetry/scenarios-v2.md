# `telemetry.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. What R computes from each
section's frames is also a case of [`conformance/v2/`](conformance/v2/),
which `zenkey-model` runs. The owner's half needs a runtime that writes a
frame resource's series, and the reference's is `zenkey`'s. Its tests run
§1 to §7, one per section. Each is named after its section, in
`zenkey/tests/profile_telemetry_v2.rs`.

**Conventions.**
- **Section numbers** in parentheses (§2.19) are v2.md's rules. `v1 §n` is
  [`telemetry.v1`](v1.md)'s, and core sections say "core".
- **The bus.** R1 is a router. The owner O and the readers are its
  clients. O's session runs with timestamping off (core §8.2). Timeouts
  are 1 s unless stated.
- **The contract.** Every section uses `meterframe.v1`,
  [`conformance/contracts/meterframe.v1.toml`](conformance/contracts/meterframe.v1.toml),
  beside v1's `meter.json` and the packed file `meterframe.json`. The
  fixtures read it too ([`declarations.json`](conformance/v2/declarations.json)).

  ```toml
  [interface]
  name  = "meterframe"
  major = 1
  minor = 0
  uses  = ["telemetry.v2", "link.v1"]

  [schemas]
  jsonschema = ["meter.json"]
  packed     = ["meterframe.json"]

  [resources."sdu/dropped_total"]
  kind        = "stream"
  type        = "json:Count"
  annotations = { "telemetry.kind" = "counter", "telemetry.unit" = "1" }

  [resources."frames/link"]
  kind  = "stream"
  type  = "packed:meterframe#Link"
  frame = { records = 1 }

  [resources."frames/link".annotations."telemetry.series"]
  dropped        = "sdu/dropped_total"
  queue_depth    = { kind = "gauge", unit = "1" }
  duty_available = { kind = "gauge", unit = "1" }
  duty_wait_ms   = { kind = "gauge", unit = "ms" }

  [resources."frames/totals"]
  kind  = "stream"
  type  = "packed:meterframe#Totals"
  frame = { records = 1, keyframe_every = 4 }

  [resources."frames/totals".annotations]
  "link.exposure" = "link"

  [resources."frames/totals".annotations."telemetry.series"]
  frames_total = { kind = "counter", unit = "1" }
  temperature  = { kind = "gauge", unit = "Cel", semantic = "temperature" }

  [resources."frames/bytes"]
  kind  = "stream"
  type  = "packed:meterframe#Bytes"
  frame = { records = 3, step_ms = 1000, jitter_ms = 100 }

  [resources."frames/bytes".annotations."telemetry.series"]
  rx = { kind = "counter", unit = "By" }
  ```

  `meterframe.json` declares three record types:
  - `Link`, core §7.7's fixtures' `Link`: `dropped`, a 32-bit counter;
    `queue_depth`, an `int` from 0 to 1022 with `out_of_range`;
    `duty_available`, an `int` from 0 to 1; and `duty_wait_ms`, a `fixed`
    from 0 to 3,600,000 by 100 with `out_of_range`. Each is optional;
  - `Totals`: `frames_total`, a 64-bit counter, and `temperature`, a
    `fixed` from −40 to 85 by 0.5 with `not_available` and `error`;
  - `Bytes`: `rx`, a 24-bit counter.
- **The owner** is `lab/meterframe`, implementing `meterframe.v1` with
  every resource exposed. Its frame keys are
  `zk2/lab/meterframe/meterframe.v1/stream/frames/{link,totals,bytes}`.
  "I1", "I2" name its instance ids in the order it mints them, and "r1",
  "r2" their runs. It writes each count unwrapped, and its frames carry
  each count modulo its field's width (core §7.6).
- **R** subscribes to `zk2/lab/meterframe/meterframe.v1/stream/**` before
  O starts, and reads no presence.
  - It keeps one frame reader per frame key (core §7.7), and hands it
    each frame in arrival order, with its arrival on R's monotonic clock.
  - "R reads" a record: each series field's reading (§2.18).
  - "R pairs": each two consecutive samples of one counter series, by
    §2.19 to §2.21.
- **Raw puts.** A step marked *raw* puts a frame on the owner's key from
  the owner's session, under its run and its next seq. It bypasses any
  check the owner's runtime makes: it is what an owner the text does not
  allow would send.
- **Spacing.** Frames are put at least 1 s apart unless a section says
  otherwise. Jitter on loopback is allowed 50 ms.

## §1 A counter's rate over frames, measured and scheduled (§2.18, §2.21)

**Steps.**
1. O starts, as I1. It puts `frames/link` twice, 2 s apart: `dropped` 10
   and then 20, `queue_depth` 3, `duty_available` 1 and `duty_wait_ms` 0.
2. O writes `rx` 1000, 1500 and 2100 into `frames/bytes`, measured 1 s
   apart. Each lies within 100 ms of its scheduled instant, so O marks
   records 1 and 2 scheduled, and puts the frame.
3. O writes `rx` 2600 a second later, as the next frame's record 0, and
   puts that frame.

**Expected.**
1. R reads every series field of `frames/link` as a sample. It pairs
   `dropped` 10 → 20 as `same_epoch`, established in band. The rate is 5
   in `1/s`, over 2,000 ms, its basis `measured`.
2. R pairs `rx` within the frame, by the records' offsets: 1000 → 1500 at
   500 `By/s`, and 1500 → 2100 at 600 `By/s`. Each is over 1,000 ms, its
   basis `scheduled`: ±100 ms for the first, whose earlier record is
   measured, and ±200 ms for the second.
3. R pairs 2100 → 2600 across the two frames by their instants: 500
   `By/s` over 1,000 ms, `scheduled` ±100 ms.
4. R shows each scheduled rate as scheduled, with its bound. It never
   shows one as a measured rate.
5. `queue_depth`, `duty_available` and `duty_wait_ms` are gauges: samples,
   and no pair.

## §2 A restart: a new run, a reset (§2.19)

**Steps.**
1. O, as I1 under r1, puts `frames/totals` with `frames_total` 100 and
   then 140, `temperature` 21.5.
2. O's process ends. O starts again, as I2 under r2, and its count starts
   again from 0. It puts `frames_total` 3 and then 9.

**Expected.**
1. R pairs 100 → 140 as `same_epoch`, established in band, with a rate.
2. R reads I2's first frame as another run's. Every field turns unknown
   there, and that frame, a keyframe, makes each known again.
3. R pairs 140 → 3 as `reset`, established in band. It has no rate, and
   it is no finding. No presence was read.
4. R pairs 3 → 9 as `same_epoch` again, with a rate.

## §3 A 24-bit counter's wrap (§2.20)

**Steps.**
1. O writes `rx` 16,777,000, then 16,777,316, then 25,777,316 into one
   `frames/bytes` frame, 1 s apart, and puts it. The last step breaks
   §2.20's SHOULD: it is more than 2^23 in one second.

**Expected.**
1. The frame carries `rx` as 16,777,000, 100 and 9,000,100: each count
   modulo 2^24.
2. R pairs 16,777,000 → 100 as `same_epoch`: an increase of 316, the wrap
   included, at 316 `By/s`, `scheduled`.
3. R pairs 100 → 9,000,100 as `unobservable` (`wrap_or_decrease`), with
   no rate (`wrap_or_decrease`). It reads no decrease, no finding and no
   rate of billions.

## §4 A lost frame: unknown, never zero (§2.18)

**Steps.**
1. O puts four `frames/totals` frames, one record each, 1 s apart:
   1. `frames_total` 10, `temperature` 20: a keyframe;
   2. `frames_total` 10, `temperature` 20.5: a delta, `frames_total`
      omitted;
   3. `frames_total` 15, `temperature` 20.5: O takes its seq and puts
      nothing, a lost frame (core's `frames.md` §2);
   4. `frames_total` 15, `temperature` 21: a delta, `frames_total`
      omitted, since it did not change since record 3.
2. O puts a fifth, `frames_total` 18 and `temperature` 21: a keyframe,
   since four records passed.

**Expected.**
1. R reads record 2's `frames_total` as held, `known` 10: no sample.
2. R reads record 4 after a gap of one record. Its `frames_total` is
   unknown (`gap`). R shows unknown: never 10, never 0, and no sample.
   Its `temperature` is a sample.
3. R reads record 5's `frames_total` 18, a sample, and pairs it with
   record 1's 10: `same_epoch`, established in band. The rate is 2 in
   `1/s`: an increase of 8 over the 4 s between the two records'
   instants, `measured`. Rates resume at the next sample.

## §5 A 64-bit counter's decrease across a face is the finding (§2.19; v1 §2.14)

**Setup.** R sits across a constrained face from O, whose policy denies
`@zk` (`link.v1` §2.6). `frames/totals` crosses, as its `link.exposure`
says. R reads no presence, and there is none to read.

**Steps.**
1. O, as I1, puts `frames/totals` with `frames_total` 500.
2. *Raw:* a frame with `frames_total` 480, under r1 at O's next seq.

**Expected.**
1. R pairs 500 → 480 as `decreased`, established in band: the finding,
   with no presence.
2. An owner that follows §2.19 never writes it. Its runtime refuses the
   count, and the step records the refusal.
3. The same decrease of a bare JSON counter with no in-band epoch reads
   as an assumed reset across a face (v1 §2.14, its scenarios' §8).

## §6 A rewind under a restored run file is a reset (§2.19)

**Setup.** O takes its runs from a run file (core §4.3). A copy of the
file is kept before O starts.

**Steps.**
1. O, as I1 under run r, puts `frames/totals` with `frames_total` 1000
   and then 1010.
2. O's process ends, and its run file is restored from the copy.
3. O starts again, as I2. It takes r again from the restored file, which
   core §4.3 forbids. Its count starts again: it puts `frames_total` 4, at
   seq 0 of its key.

**Expected.**
1. R reads I2's first frame as a rewind (core §7.7): a seq of the run it
   holds, older than the one it expected. Every field turns unknown
   (`rewind`) there, and that keyframe makes each known again.
2. R pairs 1010 → 4 as `reset`, established in band. It is never
   `decreased`, and no finding.

## §7 One counter, two series (§2.17)

**Steps.**
1. O puts `sdu/dropped_total` 7, a bare JSON counter, and `frames/link`
   with `dropped` 7, at the same moment.
2. 1 s later, it puts both again with 9.

**Expected.**
1. R holds two series: the key
   `zk2/lab/meterframe/meterframe.v1/stream/sdu/dropped_total`, and
   `dropped` of `…/stream/frames/link`. The second is labelled with its
   mirror, "`sdu/dropped_total` (`frames/link`)".
2. The frame's series has the mirror's kind and unit, `counter` in `1`.
3. R pairs the bare counter 7 → 9 by v1's rules, its epoch assumed with
   no presence read (v1 §2.9). It pairs the frame's 7 → 9 established in
   band. It never pairs a sample of one with a sample of the other.
4. `frames/link`'s record type is core §7.7's fixtures' `Link`, so its
   bodies are `frame/bodies.json`'s. Every implementation that reads
   those reads these frames alike.
