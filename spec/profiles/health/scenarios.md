# `health.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §5 are the runtime's, and
the reference runs them as tests of `zenkey` (`zenkey/tests/profile_health.rs`,
one test per section, named after it; #721, PE). §6 to §8 are a tool's,
and the reference tool runs them (`zenctl/tests/live_zk2.rs`, `zenctl
health`; #721, PF).

**Conventions.**
- **Section numbers** in parentheses (§2.3) are v1.md's rules; core
  sections say "core", and `freshness.v1`'s say so.
- **The bus.** R1 is a router with timestamping on, as a router's default
  is, and without `timestamping.drop_future_timestamp`. Owners, the
  subscriber S, the GET reader G and the tool T are its clients. Timeouts
  are 1 s unless stated.
- **The contract** is the standard one,
  [`health.v1.toml`](health.v1.toml), with protobuf payloads. "Status
  `OK`" is a `health.v1.Status` whose `level` is `LEVEL_OK`; "check `disk`
  `FAILED`" is the member `checks/disk` with a `health.v1.Check` at
  `LEVEL_FAILED`.
- **The owner** is `lab/svc`, implementing `health.v1` with every resource
  exposed. Its keys are `zk2/lab/svc/health.v1/state/status`,
  `zk2/lab/svc/health.v1/state/checks/<check>` and
  `zk2/lab/svc/health.v1/stream/faults`.
- **S** subscribes to `zk2/lab/svc/health.v1/**` before the owner starts,
  and records each delivery's arrival on its monotonic clock, its key,
  kind, payload and stamp. **G** GETs `zk2/lab/svc/health.v1/state/**` as
  core S4 says, and measures its clock from S's deliveries
  (`freshness.v1` §2.6, ground 2). "Judges" applies v1.md §2.11 to the
  reading at that instant, the descriptor read as core §3.3 says.
- **The horizon** is the contract's, 60 s, so the owner confirms its status
  at least every 30 s. §2, §4 and §8 wait it out, §2 and §4 for about
  70 s each and §8 for about 200 s, and a runner MAY keep them in a slow
  tier.
- **Jitter.** A gap between deliveries is measured at S, which adds the
  bus's jitter to what the owner sent. The reference allows 200 ms on
  loopback.

## §1 Bring-up (§2.3, §2.7)

**Steps.**
1. The owner starts with status `UNSPECIFIED`, reason `"starting"`. A
   tool T, holding a liveliness subscriber on `zk2/lab/*/@zk/**` with
   history, GETs the owner's `health.v1/state/**` the moment it sees the
   owner's instance token, and again the moment it sees its
   `alive/health.v1/…` token.
2. A second owner, `lab/quiet`, starts with `health.v1` in its tokenless
   set (core §8.1) and status `OK`. T GETs its descriptor and its
   `health.v1/state/**` the moment it sees its instance token.
3. The owner puts status `OK`, reason `"serving"`. G GETs.
4. The owner puts nothing for 35 s.
5. The owner closes. S listens 3 s more.

**Expected.**
1. Both of T's GETs answer the status: `UNSPECIFIED`, `"starting"`,
   stamped by the owner's session. It was put before the tokens (§2.3).
2. `lab/quiet` holds an instance token and no `alive/health.v1/…` token,
   its descriptor lists `health.v1` with `"token": false`, and T's GET at
   the instance token answers its status `OK`.
3. G's reply is `OK`, `"serving"`, with `since_ns` later than the first
   status's.
4. S receives the status again at least once, no two deliveries more than
   30 s apart: the same payload, `since_ns` included, under a later stamp
   of the same id (`freshness.v1` §2.4).
5. S receives no delete of the status, before or after the close (§2.3).

## §2 A stale status (§2.4)

**Steps.**
1. The owner puts status `OK`, and goes on confirming it. S and G judge
   (G with the descriptor read, and the owner present).
2. The owner closes its writer for the status, and keeps its service up:
   its tokens held, its GETs answered.
3. 65 s after S's last delivery of the status, S judges, and G GETs and
   judges.

**Expected.**
1. Both: **healthy**, `ok`.
3. Presence: the owner's instance and interface tokens are present.
   S: **stale**, `beyond_horizon`. G: **stale**, `beyond_horizon`, the
   reply's stamp more than 60.5 s old against its measured clock. Neither
   reports the service unhealthy or `FAILED`, and neither reports `OK` as
   current (§2.4, `freshness.v1` §2.11).

## §3 Aggregation (§2.2, §2.3)

**Steps.**
1. The owner holds status `OK`, check `disk` `OK` and check `net` `OK`.
2. The owner's application sets `disk` to `FAILED`. G GETs.
3. The application sets `disk` back to `OK`, and the owner improves its
   status to `OK`. G GETs.
4. The owner retires `net`. G GETs.

**Expected.**
2. S receives the status `FAILED` before `checks/disk` `FAILED`, and at no
   delivery is S's latest status better than its latest `disk` (§2.2, the
   order of the puts). G's reply holds status `FAILED`, `disk` `FAILED`
   and `net` `OK`, and G judges **unhealthy**, `failed`.
3. S receives `checks/disk` `OK` before the status `OK`. G judges
   **healthy**, `ok`.
4. S receives a delete of `checks/net`, and no status put for it. G's
   reply holds the status and `disk`, and a `reply_del` for `checks/net`
   (core S2, S3). No check is re-put at any point: S receives each check
   only when it changes (§2.3).

## §4 Clock ahead (§2.5)

**Setup.** `spec/scenarios/state.md` §7's step 3: an owner, `lab/ahead`,
implementing `health.v1`, its clock 2 s ahead of R1 (the reference's
`simulate_offset`), watches a router-stamped heartbeat key as its clock
reference (core §4.3). S subscribes to `zk2/lab/ahead/health.v1/**`.

**Steps.**
1. The owner puts status `OK` before any heartbeat.
2. The heartbeat is put until the owner reports itself ahead. S listens
   for 65 s from then, and judges.
3. The owner's clock is set right, and the heartbeat goes on. S listens
   for 35 s, and judges.

**Expected.**
1. S receives the status.
2. Within 1 s of the detection, S receives a `faults` sample with code
   `clock_ahead` and level `FAILED`, whose stamp is not the owner's (R1
   re-stamped a future-dated put, core §4.1). S receives no status put
   from the detection on. The reference publishes the fault again within
   31 s (§2.5, a SHOULD). The owner's tokens stay present. At 65 s, S
   judges the status **stale**, never `FAILED`, and a tool reading the
   faults answers "is its clock ahead?" yes (v1.md §5).
3. S receives the status again, re-put within a few seconds of the
   correction (`freshness.v1` §2.10), and no `clock_ahead` fault after
   it. S judges **healthy**, `ok`, and the clock question no.

## §5 A tokenless set of 100 (§2.7)

**Setup.** `spec/scenarios/presence.md` §5's, with the standard contract:
100 services `p5/dev0` to `p5/dev99`, each implementing `health.v1`, in
its tokenless set, and `nav.v2`, each with status `OK` put before it
starts.

**Steps.** A tool lists `zk2/p5/*/@zk/**`, then `zk2/p5/*/@zk/alive/health.v1/**`,
then GETs each instance's descriptor, and each listed service's
`health.v1/state/**`, and judges.

**Expected.**
- 200 tokens: an instance token and an `alive/nav.v2/…` token each, and no
  `alive/health.v1/…` token. The second read finds nothing.
- Every descriptor lists `health.v1` with `"token": false`. The tool finds
  all 100 providers, and judges each **healthy**, `ok`: none is reported as
  not implementing `health.v1` because it holds no interface token.

## §6 An absent owner (§2.1)

**Setup.** The owner holds status `DEGRADED`, reason `"upstream lost"`.
An archive, `lab/archive` (core §4.4), records
`zk2/lab/svc/health.v1/state/**`. The tool T reads the deployment.

**Steps.**
1. T reads the owner, and judges.
2. The owner's process ends.
3. T reads presence, a complete read, then GETs the owner's status, then
   the archive's form of it (core §4.4), and judges.

**Expected.**
1. **Unhealthy**, `degraded`.
3. The owner's tokens are gone, and its GET draws no reply. T judges **not
   asked**, `absent`: never `FAILED`, never stale. It shows the archived
   status, `DEGRADED`, as last-known, with its stamp and the archive's
   `confirmed`, never as current (core S6, `freshness.v1` §2.8).

## §7 An inconsistent status, for a tool (§2.2)

**Setup.** A test owner, `lab/liar`, breaks §2.2 on purpose: it holds
status `OK` and check `disk` `FAILED`. A second test owner, `lab/frank`,
holds status `FAILED` and check `disk` `OK`.

**Steps.** T reads both twice, 2 s apart.

**Expected.**
- `lab/liar`: each reading **unhealthy**, `inconsistent`, at `FAILED`,
  never healthy. T reports the break of §2.2 as a finding about the owner,
  seen in both readings.
- `lab/frank`: **unhealthy**, `failed`, and no break: a status may be
  worse than its checks.

## §8 Across a constrained face (§2.8)

**Setup.** `spec/scenarios/constrained.md`'s face, attached as a client
(its §1, step 2): a vehicle router V, and a ground session G that is a
client of V, with `zk2/**/@zk/**` denied on the face. The owner is
`vehicle-01/nav` on V, implementing `health.v1` with status `OK`. G knows
from its configuration that `vehicle-01/nav` implements `health.v1`.
- **Run A:** the face lets `zk2/vehicle-01/nav/health.v1/state/status`
  cross.
- **Run B:** the face denies `zk2/vehicle-01/*/*/state/**`, as
  zenoh-modem's `face-deny-device-state` does.

**Steps.**
1. G subscribes to the owner's status statically, without presence, before
   the owner starts, and listens 65 s, then judges.
2. The owner starts. G listens 65 s, then judges.
3. In run A, the owner closes its writer for the status. G listens 65 s
   more, and judges.

**Expected.**
- **Run A:**
  1. G receives nothing, and judges **unobservable**, `nothing_crossed`:
     it knows of no status yet, and does not call one stale (§2.8).
  2. G receives the status and its re-puts, and judges **healthy**, `ok`.
  3. G judges **stale**: never down, never `FAILED`, and never absent,
     since presence does not cross.
- **Run B:** nothing of the status crosses at any step. G judges
  **unobservable**, `face_closed`, each time, and never reports the service
  unhealthy, failed or down.
