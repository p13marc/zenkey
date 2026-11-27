# `health.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §8 restate
[`health.v1`'s](scenarios.md) where a record or the confirmation
changes, and §9 is new. §1 to §5 and §9 are the runtime's. §6 to §8 are
a tool's.

The reference runs §1 to §5 and §9
(`zenkey/tests/profile_health_v2.rs`), and §6 to §8
(`zenctl/tests/live_zk2.rs`).

**Conventions.**
- **Section numbers** in parentheses (§2.3) are v2.md's rules. Core
  sections say "core", `freshness.v2`'s say so, and `v1 §n` is
  `health.v1`'s text.
- **The bus.** R1 is a router with timestamping on, as a router's
  default is, and without `timestamping.drop_future_timestamp`. Owners,
  the subscriber S, the GET reader G and the tool T are its clients. An
  owner's session runs with timestamping off (core §8.2). Timeouts are
  1 s unless stated.
- **The contract** is the standard one, [`health.v2.toml`](health.v2.toml),
  with the packed records of [`health.v2.json`](health.v2.json). "Status
  `ok`, `serving`" is a `Status` whose `level` is `ok` and whose
  `reason` is 2. "Check `disk` `failed`" is the member `checks/disk`
  with a `Check` at `failed`.
- **The epoch.** No owner is given a catalog, so the deployment epoch is
  0: a record's `since` and `at` are Unix seconds (core §7.6).
- **The owner** is `lab/svc`, implementing `health.v2` with every
  resource exposed. Its keys are `zk2/lab/svc/health.v2/state/status`,
  `zk2/lab/svc/health.v2/state/checks/<check>` and
  `zk2/lab/svc/health.v2/stream/faults`.
- **S** subscribes to `zk2/lab/svc/health.v2/**` before the owner
  starts, and to its heartbeat key covering `health.v2`,
  `zk2/lab/svc/freshness.v2/stream/heartbeat/health.v2`. It records each
  delivery's arrival on its monotonic clock, with its key, its kind, its
  payload and its in-band stamp. It reads each heartbeat as
  `freshness.v2` §2.5 says. **G** GETs `zk2/lab/svc/health.v2/state/**`
  as core S4 says. "Judges" applies v2.md §2.11 to the reading at that
  instant, the descriptor read as core §3.3 says.
- **G's and T's clock.** G and T read by GET, and take the deployment's
  word for their clock, as `health.v1`'s scenarios do: every session of
  a scenario runs on one host.
- **The horizon** is the contract's, 60 s. The owner's heartbeat
  confirms its status at least every 30 s. §1, §2, §4 and §8 wait it
  out. A runner MAY keep them in a slow tier.
- **Jitter.** A gap between deliveries is measured at S, which adds the
  bus's jitter to what the owner sent.

## §1 Bring-up, and both majors (§2.3, §2.4, §2.7, §2.12)

**Steps.**
1. The owner starts with status `unknown`, `starting`. A tool T, holding
   a liveliness subscriber on `zk2/lab/*/@zk/**` with history, GETs the
   owner's `health.v2/state/**` the moment it sees the owner's instance
   token, and again the moment it sees its `alive/health.v2/…` token.
2. A second owner, `lab/quiet`, starts with `health.v2` and
   `freshness.v2` in its tokenless set (core §8.1) and status `ok`. T
   GETs its descriptor and its `health.v2/state/**` the moment it sees
   its instance token.
3. The owner puts status `ok`, `serving`. G GETs.
4. The owner puts nothing for 65 s, and S judges.
5. A third owner, `lab/old`, implements `health.v1`, its standard
   contract's revision 1.1, with status `OK`. T reads the deployment: one
   GET over `zk2/lab/*/health.v2/state/**`, one over
   `zk2/lab/*/health.v1/state/**`, and each instance's descriptor. It
   judges each service.
6. The owner closes. S listens 3 s more.

**Expected.**
1. Both of T's GETs answer the status's snapshot: one entry, `unknown`
   with the reason 1, under the run the owner's descriptor states (core
   §3.3), with its confirmation (`freshness.v2` §2.4). It was put before
   the tokens (§2.3). Its put, as S heard it, is CH, the run, the seq
   and a record of 6 bytes. Under a 16-bit run it is 12 bytes, with CH
   `0x03`. Under a 32-bit run it is 14, with CH `0x13`.
2. `lab/quiet` holds an instance token and no `alive/health.v2/…` or
   `alive/freshness.v2/…` token. Its descriptor lists `health.v2` with
   `"token": false`, and T's GET at the instance token answers its
   status `ok`.
3. G's entry is `ok` with the reason 2, its `since` not before the
   first status's.
4. S receives no put of the status (`freshness.v2` §2.4). It receives a
   heartbeat at least every 30 s, of form 6. Its tag is 1, that of
   `health.v2`'s revision 2.0, and its section vouches for the status.
   Its payload is 16 bytes under a 16-bit run. At 65 s, past the
   horizon, S judges **healthy**, `ok`.
5. `lab/svc`'s descriptor lists `health.v2` and not `health.v1`, and
   `lab/old`'s the reverse (§2.3). T judges `lab/svc` and `lab/quiet`
   under this text and `lab/old` under `health.v1`'s, each **healthy**,
   `ok` (§2.12). Its roll-up counts three healthy services.
6. S receives no delete of the status, before or after the close.

## §2 A stale status (§2.4)

**Steps.**
1. The owner puts status `ok`, and goes on confirming it for 65 s. S and
   G judge (G with the descriptor read, and the owner present).
2. The owner stops vouching for its status. Its service stays up: its
   tokens held, its heartbeats sent, its GETs answered.
3. 65 s after the last heartbeat that vouched for the status, S judges.
   G GETs and judges.

**Expected.**
1. Both: **healthy**, `ok`, past the horizon. S has received no put of
   the status since step 1's. G's entry carries confirmation 1, vouched
   for at the snapshot's time.
2. S receives heartbeats, each with the status's bit clear, and no put.
3. The owner's tokens are present. S: **stale**, `beyond_horizon`. G:
   **stale**, `beyond_horizon`: its entry carries confirmation 2, a last
   confirmation more than 60.5 s old. Neither reports the service
   unhealthy or `failed`, and neither reports `ok` as current (§2.4).

## §3 Aggregation, by seq (§2.2, §2.3)

**Steps.**
1. The owner holds status `ok`, check `disk` `ok` and check `net` `ok`.
2. The owner's application sets `disk` to `failed`, with the detail
   `timeout`. G GETs.
3. The application sets `disk` back to `ok`, and the owner improves its
   status to `ok`. G GETs.
4. The owner retires `net`. G GETs.

**Expected.**
2. S receives the status `failed`, with the reason `check`, before
   `checks/disk` `failed`. Both puts are of one run, and the status's
   seq is the lower (§2.2). At no delivery is S's latest status better
   than its latest `disk`.
   - G's answer is two snapshots under one run and one asof, with no
     entry above that asof: one instant (§2.2). It holds status
     `failed`, `disk` `failed` and `net` `ok`, and G judges
     **unhealthy**, `failed`.
3. S receives `checks/disk` `ok` before the status `ok`, the check's seq
   the lower. G judges **healthy**, `ok`.
4. S receives a delete of `checks/net`, and no status put for it. G's
   answer holds the status and `disk`, and `net` as a tombstone in the
   checks' snapshot. No check is put again at any point: S receives each
   check only when it changes (§2.3).

## §4 Clock ahead, answered by a heartbeat (§2.5)

**Setup.** `spec/scenarios/state.md` §7's step 6: an owner, `lab/ahead`,
implementing `health.v2`, its clock 2 s ahead of R1, watches a
router-stamped key as its clock reference (core §4.3). S subscribes to
`zk2/lab/ahead/health.v2/**` and to its heartbeat key. A late joiner L
subscribes to the heartbeat key alone.
- **Without root.** A runner without root offsets the owner's clock in
  its runtime, as `health.v1`'s scenario §4 says. Every expectation
  below holds so: none rests on a fault's stamp, and a fault of this
  major carries none (§2.5).

**Steps.**
1. The owner puts status `ok`, `serving`, before any put of its
   reference.
2. The reference is put until the owner reports itself ahead. The owner
   then puts status `ok` with the reason `fallback`. S listens for 65 s
   from the detection, and judges.
3. L subscribes 35 s after the detection, and listens 30 s.
4. The owner's clock is set right, and the reference goes on. S listens
   for 35 s, and judges.
5. The owner's clock is set 2 s ahead again. S listens for 35 s.
6. Steps 1 and 2 again, with R1 running
   `timestamping.drop_future_timestamp`. S listens for 35 s from the
   detection.

**Expected.**
1. S receives the status. Each heartbeat it reads carries an instant,
   and S answers the clock question **no**, `confirmed`.
2. Within 1 s of the detection, S receives one `faults` sample: the code
   1, `clock_ahead`, the level `failed`, and an `argument` of 2 or 3,
   the offset the owner measured, rounded up. Its put carries no Zenoh
   timestamp of the owner's.
   - The guard does not hold the status: the owner's put succeeds, and S
     receives it.
   - S receives **no second `clock_ahead` fault** in the 65 s (§2.5).
   - Every heartbeat from the detection on carries the ahead time, never
     all ones, and vouches for the status (`freshness.v2` §2.10).
   - From its first such heartbeat, within 30 s of the detection, S
     answers the clock question **yes**, `ahead`.
   - At 65 s, S judges **healthy**, `ok`: the heartbeats confirmed the
     status. The owner's tokens stay present.
3. L hears no fault: it was published before L joined. From the first
   heartbeat it reads, within 30 s, L answers **yes**, `ahead` (§2.5).
4. Within 30 s of the correction, S receives a heartbeat whose time is
   an instant again. It receives no `clock_ahead` fault after step 2's.
   S judges **healthy**, `ok`, and the clock question **no**,
   `confirmed`.
5. Within 1 s of the new detection, S receives one more `clock_ahead`
   fault: a new hold (§2.5). Its heartbeats carry the ahead time again.
6. S receives the fault: it carries no Zenoh timestamp of the owner's,
   so R1 has no future-dated put to drop. S receives the status and the
   heartbeats as in step 2, and answers **yes**, `ahead`.

## §5 A tokenless set of 100 (§2.7)

`health.v1`'s scenario §5, with `health.v2` for `health.v1`: 100
services `p5/dev0` to `p5/dev99`, each implementing `health.v2` and
`nav.v2`, with `health.v2` and `freshness.v2` in its tokenless set and
status `ok` put before it starts.

**Steps.** T lists `zk2/p5/*/@zk/**`, then
`zk2/p5/*/@zk/alive/health.v2/**`, then GETs each instance's descriptor,
and each listed service's `health.v2/state/**`, and judges.

**Expected.**
- 200 tokens: an instance token and an `alive/nav.v2/…` token each, and
  no `alive/health.v2/…` or `alive/freshness.v2/…` token. The second
  read finds nothing.
- Every descriptor lists `health.v2` with `"token": false`. T finds all
  100 providers, and judges each **healthy**, `ok`.

## §6 An absent owner (§2.1)

**Setup.** The owner holds status `degraded`, `upstream_lost`. An
archive, `lab/archive` (core §4.4), records
`zk2/lab/svc/health.v2/state/**`. The tool T reads the deployment.

**Steps.**
1. T reads the owner, and judges.
2. The owner's process ends.
3. T reads presence, a complete read, then GETs the owner's status, then
   the archive's form of it (core §4.4), and judges.

**Expected.**
1. **Unhealthy**, `degraded`. T shows the reason as `upstream_lost`.
3. The owner's tokens are gone, and its GET draws no reply. T judges
   **not asked**, `absent`: never `failed`, never stale. It shows the
   archived status, `degraded`, as last-known, never as current (core
   S6).

## §7 An inconsistent status, from one reading (§2.2)

**Setup.** A test owner, `lab/liar`, breaks §2.2 on purpose: it holds
status `ok` and check `disk` `failed`. A second, `lab/frank`, holds
status `failed` and check `disk` `ok`. A third, `lab/flip`, follows
§2.2: every 100 ms it sets its check `disk` to `failed` and back, its
status following in §2.2's order. Each builds the snapshots of one GET
at one asof (§2.2).

**Steps.**
1. T reads `lab/liar` and `lab/frank` once each.
2. T reads `lab/flip` 50 times, 37 ms apart.
3. A fourth test owner, `lab/two`, answers the status's snapshot at one
   asof and the checks' at a later one, with status `ok` and check
   `disk` `failed`. T reads it once, then again 2 s later.

**Expected.**
1. Each reading's two snapshots are under one run and one asof, with no
   entry above it: one instant.
   - `lab/liar`: **unhealthy**, `inconsistent`, at `failed`, never
     healthy. T reports the break of §2.2 as a finding about the owner,
     from this one reading.
   - `lab/frank`: **unhealthy**, `failed`, and no break: a status may be
     worse than its checks.
2. Every reading of `lab/flip` is one instant, and none shows a status
   better than its check. T reports no break.
3. T's first reading of `lab/two` is not one instant. T judges it
   **unhealthy**, `inconsistent`, at `failed`, and reports no break from
   it alone. It reports the break after the second reading, a grace
   apart (§2.2).

## §8 Across a constrained face (§2.8)

**Setup.** `health.v1`'s scenario §8: a vehicle router V, and a ground
session G that is a client of V, with `zk2/**/@zk/**` denied on the
face. The owner is `vehicle-01/nav` on V, implementing `health.v2` with
status `ok`. G knows from its configuration that `vehicle-01/nav`
implements `health.v2`.
The faults' key is `zk2/vehicle-01/nav/health.v2/stream/faults`. A
heartbeat and a fault are stream samples. A deny of the state leaves
them crossing. So each run names what it denies of them.
- **Run A:** the face lets `zk2/vehicle-01/nav/health.v2/state/status`
  cross, and the heartbeat covering `health.v2` with it. It denies the
  faults' key.
- **Run B:** the face denies `zk2/vehicle-01/*/*/state/**`. It denies
  the heartbeats, `zk2/vehicle-01/*/freshness.v2/stream/**`, and the
  faults' key.
- **Run C:** the face denies the state and the heartbeats as run B
  does. It lets the faults' key cross.

**Steps.**
1. G subscribes to the owner's status, its heartbeat key and its faults
   statically, without presence, before the owner starts. It listens
   65 s, then judges.
2. The owner starts. G listens 65 s, then judges.
3. The owner's clock is set 2 s ahead, and its guard holds. G listens
   35 s, and answers the clock question.
4. In run A, the owner's clock is set right, and it stops vouching for
   its status. G listens 65 s more, and judges.

**Expected.**
- **Run A:**
  1. G receives nothing, and judges **unobservable**, `nothing_crossed`.
  2. G receives the status and the heartbeats that vouch for it.
     Under a 16-bit run they are 12 bytes, and 16 bytes each. G judges
     **healthy**, `ok`.
  3. No fault crosses. G reads a heartbeat whose time is ahead, and
     answers **yes**, `ahead`.
  4. G judges **stale**: never down, never `failed`, and never absent.
     Its heartbeats carry instants again, and it answers the clock
     question **no**.
- **Run B:** nothing crosses at any step. G judges **unobservable**,
  `face_closed`, each time, and the clock question **unobservable**,
  `nothing_heard`.
- **Run C:** at step 3, G receives one `clock_ahead` fault, and shows
  it as an occurrence. It is 14 bytes under a 16-bit run. G answers the
  clock question **unobservable**, `nothing_heard`: no heartbeat
  crosses, and a fault
  published once cannot say that the hold lasts (§2.5). G judges health
  **unobservable**, `face_closed`.

## §9 Words, by table (§2.10)

**Setup.** The owner `lab/tc` implements `health.v2` and a contract of
its own, `tc.v1`, which lists `health.v2` in `uses` and declares on
`[defaults]`:

```toml
[defaults.annotations]
"health.reasons" = ["tc_netns_gone", "tc_qdisc_missing"]
"health.codes"   = ["tc_netns_restarted"]
```

A second owner, `lab/plain`, implements `health.v2` and declares no
table. T holds `tc.v1`'s revision, the one `lab/tc`'s descriptor names.
T2 is a tool that holds an earlier revision of `tc.v1` alone, whose
`health.reasons` is `["tc_qdisc_missing", "tc_netns_gone"]`. T and T2
each subscribe to both owners' `health.v2/**` before step 1. A later
read answers no status since replaced, and no fault.

**Steps.**
1. `lab/tc` puts status `degraded` with its own reason `tc_netns_gone`,
   then publishes a fault with its own code `tc_netns_restarted` and the
   argument 3.
2. `lab/tc` puts status `degraded` with a reason its table does not
   hold, "kernel too old for this qdisc".
3. `lab/plain` puts status `degraded` with the same reason as step 1's.
4. T and T2 read both owners by GET. Each shows what it heard and
   what it read.

**Expected.**
1. The status's record carries the reason 32, and the fault's the code
   32 and the argument 3. Neither payload holds a byte of text.
2. The record carries `out_of_range`, the code 63. The words are in
   `lab/tc`'s log.
3. `lab/plain` has no table, so its record carries `out_of_range` too.
4. T shows `lab/tc`'s reason as `tc_netns_gone` and its fault's code as
   `tc_netns_restarted`, with the argument 3. It shows step 2's status
   as a word no table names, never as a name and never as no word.
   - T2 does not hold the revision `lab/tc`'s descriptor names. It shows
     the reason as the number 32, never as `tc_qdisc_missing`, the name
     its own revision has at that index.
   - Both show `lab/plain`'s reason as a word no table names.
   - Each judges both services **unhealthy**, `degraded`: a word plays
     no part in the verdict.
