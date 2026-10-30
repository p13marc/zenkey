# `arbitration.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §9 and §11 are the
runtime's: the reference runs them as tests of `zenkey`, one test per
section, named after it (`zenkey/tests/profile_arbitration.rs`). §10
is a tool's, and the reference tool, `zenctl arbitration`, runs
it as a test of zenctl (`zenctl/tests/live_zk2.rs`'s
`arbitration_s10_a_tools_questions`).

**Conventions.**
- **Section numbers** in parentheses (§2.7) are v1.md's rules; core
  sections say "core", and `timing.v1`'s say so.
- **The bus.** R1 is a router. Every session is a client of R1 unless a
  section says otherwise. Timeouts are 1 s unless stated, and zenoh's lease
  is its default, 10 s.
- **The contracts.** `steer.v1`, a commander's stream with a deadline, and
  `rudder.v1`, an actuator's arbitrated role on it. The reference keeps
  them as `zenkey/tests/contracts/steer.v1.toml` and `rudder.v1.toml`:

  ```toml
  # steer.v1.toml: what a commander provides
  [interface]
  name  = "steer"
  major = 1
  minor = 0
  uses  = ["timing.v1"]

  [resources.cmd]                  # one command every 50 ms, missed 300 ms after the last
  kind        = "stream"
  type        = { raw = "text/plain" }
  reliability = "best_effort"
  congestion  = "drop"
  priority    = "real_time"
  express     = true
  annotations = { "timing.period_ms" = 50, "timing.deadline_ms" = 300 }
  ```

  ```toml
  # rudder.v1.toml: what an actuator requires
  [interface]
  name  = "rudder"
  major = 1
  minor = 0
  uses  = ["arbitration.v1"]

  [resources.angle]                # the last command delivered
  kind = "state"
  type = { raw = "text/plain" }

  [requires.cmd]
  interface   = "steer.v1"
  resources   = ["cmd"]
  cardinality = "many"
  annotations = { "arbitration.policy" = "priority" }
  ```

  The deadline is 300 ms, and `timing.v1` §2.5's detection bound 306 ms
  (300 + max(300/50, 1)): a commander is missed once more than 300 ms has
  passed since its last arrival, and the miss is seen by 306 ms. The numbers
  are a loaded runner's: a sample in six lost before a miss, and a bound
  tens of times the scheduling jitter.
- **The commanders.** `C_s`, `C_t` and `C_a` are owners of `steer.v1` at
  `lab/safety`, `lab/teleop` and `lab/autopilot`, each in a process of its
  own, its HLC on. Each puts `cmd` every 50 ms with the payload `<service>
  <n>`, `n` counting from 0 at its start (`safety 0`, `safety 1`, …). Its
  first put is **500 ms after its tokens** (an actuator hears a token on
  its watch session and a sample on another, unordered, so a first sample
  put at the token could be a withdrawn provider's arrival, as §4's X
  avoids too). Each holds its instance and interface tokens as core §8.1
  says. A commander **holds** when it stops putting, keeping its
  tokens and its session, and **resumes** when it starts putting again.
  The reference's commanders are `zenkey/examples/owner.rs` run with
  `--commander` (`--commander-lead-ms`, 500 by default; `owner --help`),
  or owners its test builds (`zenkey::ServiceBuilder`), each on a session
  of its own behind a link of its own, which the test closes for a kill.
  Where a section needs what no owner's runtime does, a raw session `X`
  plays the commander, declaring its tokens by hand at the keys core §8.1
  gives and putting by hand.
- **The actuators.** `A_l` and `A_r` are instances of `lab/rudder-l` and
  `lab/rudder-r`, implementing `rudder.v1`, each binding `cmd` to
  `["lab/safety", "lab/teleop", "lab/autopilot"]` unless a section says
  otherwise. Each runs three sessions: its service's, which holds its
  subscription; a watch session, which holds its liveliness subscriber
  (v1.md §2.7; core §8.1); and a reader, for its reads again. Each
  puts every command it delivers on `angle`, payload unchanged.
- **What an actuator prints.** Each actuator under test prints, on its
  standard output, one JSON line per switch, as it tells its application
  (§2.10):

  ```json
  {"switch": {"from": "lab/teleop", "to": "lab/safety", "cause": "sample"}, "at_ms": 2051.7, "last_ms": 2043.0}
  ```

  `from` and `to` are addresses, `null` meaning none: the dead-man.
  `cause` is `start`, `sample`, `deadline` or `withdrawn`. `at_ms` is the
  instant of the event that made the switch, and `last_ms` the instant of
  `from`'s last arrival, `null` when `from` is `null`, both in ms since the
  actuator's start on its monotonic clock. Asked, it prints one line of its
  counts (§2.5):

  ```json
  {"counts": {"providers": {"lab/safety": {"delivered": 40, "not_active": 0, "deleted": 0, "expired": 0, "not_incumbent": 0}}, "wild": 0, "unresolved": 0, "unbound": 0}}
  ```

  Every provider known has its five counts, `deleted` among them, in rank
  order. The reference's actuators are
  `zenkey/examples/owner.rs` run with `--arbitrate <entry>,…`,
  which answers `arbitrate counts` so, and whose `owner --help` is the
  interface a black-box runner drives, as for `alarms.v1`; its watch
  session connects to `--watch-connect`'s endpoint where §7 cuts it. Two
  implementations are compared on these lines and on what S sees (§2.10;
  core `presence.md` §10).
- **S** is a tool session that subscribes to `zk2/lab/**` and follows
  `zk2/lab/**/@zk/**` with a liveliness subscriber on a watch session,
  recording each delivery's arrival on its monotonic clock, its key and
  its payload. "S sees `A_l` follow teleop" means every `angle` put of
  `A_l`'s that S receives in that span carries a `teleop` payload.
- **Margins.** A switch's own instants (`at_ms − last_ms`) are the
  actuator's, so they hold on a loaded runner as far as its timer does:
  the test allows the runner 50 ms of scheduling beyond the detection
  bound, and records the largest it saw, as `timing.v1`'s scenarios do. An
  instant the test takes itself (a kill, a cut) is allowed 200 ms on
  loopback.

## §1 The dead-man at start, and each higher priority taking over (§2.9–§2.11)

**Setup.** R1 up. `C_a` runs. Nothing of `lab/rudder-*` runs.

**Steps.**
1. `A_l` and `A_r` start.
2. 2 s later, `C_t` starts.
3. 2 s later, `C_s` starts.
4. 2 s later, the test asks each actuator for its counts.

**Expected.**
1. Each actuator's first line is `{"from": null, "to": null, "cause":
   "start"}`, before any other (§2.11). Within 1 s of its start, its next
   is to `lab/autopilot`, cause `sample`, and S sees it follow autopilot.
2. Each switches to `lab/teleop`, cause `sample`, at teleop's first
   arrival (no hysteresis, §2.9): the first teleop payload S sees on its
   `angle` is the first of `C_t`'s that reached it (`teleop 0`, unless
   best effort lost it), and S sees no autopilot payload on its `angle`
   after it.
3. The same for `lab/safety`, cause `sample`: S sees each follow safety.
4. Each `delivered` count equals the number of that provider's payloads S
   saw on the actuator's `angle`, give or take one put in flight; teleop's
   and autopilot's `not_active` counts are above zero; `wild`,
   `unresolved` and `unbound` are 0. `A_l` and `A_r` decided apart: each
   printed its own `start` line and its own three switches.

## §2 A commander silent past its deadline (§2.3, §2.6, §2.9; `timing.v1` §2.5)

**Setup.** §1's end: all three commanders run, and each actuator follows
safety.

**Steps.**
1. `C_s` holds for 2 s, then resumes.
2. The test repeats step 1 five times, 3 s apart.

**Expected.**
1. Each actuator switches to `lab/teleop`, cause `deadline`, with
   `at_ms − last_ms` above 300 (never before the deadline) and at most
   306, the detection bound, within the margin. No line before it in the
   hold. At safety's first arrival after it resumes, each switches back to
   `lab/safety`, cause `sample`: a miss needs no re-admission (§2.7).
2. Every repetition the same: ten switches per actuator, none to the
   dead-man, and S sees no teleop payload on an `angle` outside a hold.

## §3 A commander killed (§2.7)

**Setup.** As §2's setup.

**Steps.**
1. The test kills `C_s`'s process (SIGKILL), at instant K: its host's
   kernel closes its links, and R1 withdraws its tokens at once.
2. 2 s later, `C_s` starts again, a new instance, putting 500 ms after its
   tokens (the conventions).

**Expected.**
1. Each actuator switches to `lab/teleop`, cause `withdrawn`, before
   safety's deadline: `at_ms − last_ms` below 300 (S11 measured 4–5 ms
   after the signal). Each reads `lab/safety`'s presence again (§2.7's
   SHOULD; the reference makes one read), finds no token held, and stays on
   teleop.
2. When S sees the new instance's interface token, the actuators switch
   nothing; at the new instance's first arrival after the actuator's watch
   heard its token, each switches to `lab/safety`, cause `sample`: with
   the commander's lead, its first, `safety 0`.

## §4 A clean stop that publishes after its withdrawal (§2.7; core §8.2)

**Setup.** R1 up; `C_t` runs. `X` plays safety: a raw session that
declares an instance token at `zk2/lab/safety/@zk/instance/<i>` and an
interface token at `zk2/lab/safety/@zk/alive/steer.v1/<i>/<fp16>` (`<fp16>`
the first 16 digits of `steer.v1`'s fingerprint), then puts `cmd` every
50 ms as a commander does. Each actuator follows safety.

**Steps.**
1. `X` undeclares its interface token, then its instance token, as core
   §8.2's stop does, and goes on putting `cmd` every 50 ms for 1 s: 20
   samples after its withdrawal. Then it stops putting.
2. 2 s later, `X` declares an instance token and an interface token under
   a new instance id `<j>`, and 500 ms later puts `cmd` every 50 ms again.

**Expected.**
1. At the withdrawal each actuator switches to `lab/teleop`, cause
   `withdrawn`. No line moves back to safety while `X`'s 20 samples
   arrive: none is delivered (S sees only teleop payloads on each
   `angle`), and each is counted `not_active` for `lab/safety`. The read
   again finds no token held, and re-admits nothing.
2. At `X`'s new tokens nothing is printed: a re-admission alone switches
   nothing. At its first sample after them, each actuator switches to
   `lab/safety`, cause `sample`.

## §5 A re-mint with no drop (§2.6, §2.7; core §8.1)

**Setup.** As §2's setup.

**Steps.**
1. `C_s` re-mints, make-before-break (core §8.1): its new descriptor and
   tokens, then the old ones undeclared, putting `cmd` throughout. The
   test repeats it ten times, 1 s apart.

**Expected.**
1. No actuator prints a line, and S sees only safety payloads on each
   `angle`. Each actuator's watch hears every old token withdrawn while
   the new one is held, so none leaves the address with no token known
   held, and the reference makes no read again for any of them (§2.7).

## §6 All silent: the dead-man (§2.11)

**Setup.** As §2's setup.

**Steps.**
1. `C_s` holds.
2. 1 s later, `C_t` holds.
3. 1 s later, `C_a` holds.
4. 2 s later, `C_a` resumes.

**Expected.**
1. To `lab/teleop`, cause `deadline`.
2. To `lab/autopilot`, cause `deadline`.
3. To none, `{"from": "lab/autopilot", "to": null, "cause": "deadline"}`,
   with `at_ms − last_ms` above 300 and at most 306 within the margin:
   never before the deadline, so no stop while any commander's samples
   still arrive. S sees no `angle` put of either actuator from then until
   step 4.
4. To `lab/autopilot`, cause `sample`, at autopilot's first arrival.

## §7 The watch session's own transport moves (§2.8; core §8.1)

**Setup.** As §2's setup, but each actuator's watch session is a client
of R1 through a link the test can cut and heal, alone: its service's
session and its reader keep their own links. The control: a watch
session `W` of the test's, a client of R1 through the same link, holding
a liveliness subscriber without history on `zk2/lab/safety/@zk/alive/**`
and listening to its own transport events, each recorded with its
instant.

**Steps.**
1. The test cuts each actuator's watch link for 1 s, then heals it. It
   repeats the step five times, 3 s apart.
2. The test cuts each actuator's watch link again; while it is cut, it
   kills `C_s`'s process; 1 s after the kill it heals the link.

**Expected.**
1. No actuator prints a line, and S sees only safety payloads on each
   `angle`: what the watch hears at the cut and at the reopen is kept,
   and applied as one change once the round trip after the reopen stands,
   which leaves safety held. An actuator that applied the cut's
   withdrawals as heard would switch to teleop, cause `withdrawn`, and
   fails. After each heal, each actuator asks one round trip on its watch
   session (core §8.1; the reference counts its own).
   - **The control:** at a cut after the first, `W` hears safety's
     interface token withdrawn **before** its transport event tells it the
     close; at the first it hears nothing, having been told no token
     (core §8.1), and at the reopen it is told the token. So
     the move begins at that withdrawal (v1.md §2.8): an actuator that
     began it at its transport event alone would apply the close's
     withdrawals as heard, switch `withdrawn`, and fail.
2. Each actuator switches to `lab/teleop`, cause `deadline`, with
   `at_ms − last_ms` within §2's bounds: safety's withdrawal is never told
   to a watch whose transport is down (core §8.1), and its deadline
   makes it ineligible. Nothing more is printed at the heal.

## §8 An invalid declaration, refused at start (§2.2; core §8.2)

**Setup.** R1 up, S watching, the three commanders running. A control:
`A_l` configured plainly prints its `start` line and serves within 3 s.

**Steps.** `A_l` starts, each time on a copy of a contract:
1. `rudder.v1` with `arbitration.policy = "freshest"`;
2. `rudder.v1` with `arbitration.policy = "vote"`;
3. `rudder.v1` with `cardinality = "one"`;
4. `steer.v1` without `timing.deadline_ms`, as the revision `A_l` holds;
5. `steer.v1` with `timing.deadline_ms = 20`, below its period of 50.

**Expected.** Each time `A_l` refuses to start (core §8.2 step 2): no
instance token of `lab/rudder-l` appears within 5 s, nothing of it is
declared or put, and it prints no switch. The refusal names the reason,
`policy_unsupported`, `value`, `cardinality`, `no_deadline` and
`no_deadline` (the reference: `not started: … invalid: <reason>`).

## §9 A wildcard binding's order (§2.4; core R1, R3)

**Setup.** R1 up. `C_t` and `C_a` run; `C_s` does not. `A_l` binds `cmd`
to `["lab/safety", "lab/*"]`.

**Steps.**
1. `A_l` starts. A tool reads its descriptor.
2. 2 s later, `C_a` holds.
3. 1 s later, `C_a` resumes.
4. 1 s later, `C_s` starts.

**Expected.**
1. `A_l`'s descriptor lists `cmd`'s bindings as `["lab/safety",
   "lab/*"]`, in that order (core R3). Within 1 s of its start `A_l`
   follows `lab/autopilot`: within the wildcard, autopilot ranks above
   teleop by byte order (§2.4), and if teleop's sample arrived first, the
   switch to autopilot came at autopilot's first, cause `sample`.
   `lab/rudder-l` itself, matched by `lab/*`, publishes no `steer.v1`,
   and is never a provider with arrivals.
2. To `lab/teleop`, cause `deadline`.
3. To `lab/autopilot`, cause `sample`.
4. To `lab/safety`, cause `sample`: the exact entry above the wildcard.

## §10 A tool's questions (§2.14, §5)

**Setup.** T is the tool under test, a client of R1, holding `rudder.v1`
and `steer.v1` offline. It reads `A_l`'s descriptor and listens, on a
subscription and a watch of its own, as §2.14 says.

**Steps.**
1. The three commanders run, and `A_l` binds as in §1. T listens 2 s.
2. All three hold. T listens 2 s.
3. `A_l` binds as in §9, with `C_t` and `C_a` running. T listens 2 s.
4. T reads a copy of `rudder.v1` whose role says
   `arbitration.policy = "freshest"`, offline, with no consumer running.
5. T holds `steer.v1` and a copy of it with `timing.deadline_ms = 200`,
   and reads `A_l` of step 1.
6. T listens to `A_l` of step 1 for 100 ms only, all three holding.

**The reference tool** is `zenctl arbitration`, with
`--contracts` holding the contracts T holds offline, `--for` its listen,
and `--format json` its answers, each selection's `whose` "the tool's own
view". `zenctl/tests/live_zk2.rs`'s `arbitration_s10_a_tools_questions`
runs the six steps against the owner example's actuator and commanders,
compiled in (`zenkey/examples/owner/arbitrate.rs`); its exits are 0, 1, 1,
1, 2 and 2.

**Expected.** T answers as `conclusions.json` does for the same reading,
and says each selection is its own view:
1. The declaration: yes, `valid`; the selection: yes, `selected`,
   `lab/safety`; the binding: no, `exact`.
2. The selection: no, `dead_man`.
3. The selection: yes, `selected`, `lab/autopilot`; the binding: yes,
   `wildcard`, naming entry 1.
4. The declaration: no, `invalid`, `policy_unsupported`; the selection
   and the binding: not asked, `no_descriptor`: with no
   consumer, T read no descriptor, so no binding and no view.
5. The declaration and the selection: unobservable, `revisions_disagree`.
6. The selection: unobservable, `listening`.

## §11 A governed commander's standby (§2.5's `not_incumbent`; `redundancy.v1` §2.10)

A sample `redundancy.v1` §2.10 does not accept is no
arrival, so a standby's writes cannot keep its address eligible.

**Setup.** R1 up. The commanders' contract is `steer.v1` minor 1, which
lists `redundancy.v1` in `uses`, its election `claim` on `[defaults]`,
`cmd` as minor 0 declares it (the reference keeps it as
`zenkey/tests/contracts/steer-governed.v1.toml`). `C_t` serves
`lab/teleop`. `C_s1` and `C_s2` are two instances of `lab/safety`, each a
candidate (`redundancy.v1` §2.5) with its HLC on, `C_s1` of the lower
instance id: `C_s1` serves, `C_s2` stands by, holding its instance token
and no interface token. Then `A_l` and `A_r` start, each holding `steer.v1`
minor 1 as its revision of the required interface (v1.md §2.3), `cmd`
bound to `["lab/safety", "lab/teleop"]`.

**Steps.**
1. `X`, `C_s2`'s own session, puts `cmd` every 50 ms raw, round its
   runtime's writer, payload `standby <n>`, stamped by its own HLC (the
   reference's `commander raw on`).
2. 1 s later, `C_s1` holds; `X` goes on putting.
3. 2 s later, `C_s1` resumes.

**Expected.**
1. Each actuator follows `lab/safety` within 1 s of its start, its
   incumbent `C_s1`, learned by its read at start (its watch was told no
   token declared before it) and `C_s1`'s descriptor's `meta.zid`
   (v1.md §2.5). Through step 1 it prints no line; S sees no `standby`
   payload on any `angle`; and its `not_incumbent` count for
   `lab/safety` grows with `X`'s puts, about 20 a second.
2. Each switches to `lab/teleop`, cause `deadline`, with `at_ms −
   last_ms` within §2's bounds: `X`'s samples, counted `not_incumbent`,
   keep nothing eligible.
3. At `C_s1`'s first arrival, each switches to `lab/safety`, cause
   `sample`.
