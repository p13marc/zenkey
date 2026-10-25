# `redundancy.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §11 are the runtime's:
the reference runs them as tests of `zenkey` (`zenkey/tests/profile_redundancy.rs`,
one test per section, named after it; #841, RB). §12 is a tool's: the
reference tool runs it as a test of zenctl and of the doctor
(`zenctl/tests/live_zk2.rs`, `redundancy_s12_…`; #841, RC).

**Conventions.**
- **Section numbers** in parentheses (§2.5) are v1.md's rules; core
  sections say "core".
- **The bus.** R1 is a router. Where a section cuts a link, R2 is a second
  router linked to R1 through a path the test can cut and heal (spike S12
  cut a proxy). Everything else is a client of the router its section
  names, R1 unless stated. Timeouts are 1 s unless stated, and zenoh's
  lease is its default, 10 s, unless a section sets it short.
- **The contracts.** `ledger.v1`, governed under `claim`, and `valve.v1`,
  governed under `external`. The reference keeps them as
  `zenkey/tests/contracts/ledger.v1.toml` and `valve.v1.toml`:

  ```toml
  # ledger.v1.toml: a single writer elected by claim
  [interface]
  name  = "ledger"
  major = 1
  minor = 0
  uses  = ["redundancy.v1"]

  [defaults]
  annotations = { "redundancy.election" = "claim" }

  [resources.balance]
  kind = "state"
  type = { raw = "text/plain" }

  [resources.entries]
  kind = "stream"
  type = { raw = "text/plain" }

  [resources.append]
  kind     = "operation"
  request  = { raw = "text/plain" }
  response = { raw = "text/plain" }
  ```

  ```toml
  # valve.v1.toml: an actuator locked outside the bus
  [interface]
  name  = "valve"
  major = 1
  minor = 0
  uses  = ["redundancy.v1"]

  [defaults]
  annotations = { "redundancy.election" = "external" }

  [resources.position]
  kind = "state"
  type = { raw = "text/plain" }

  [resources.set]
  kind     = "operation"
  request  = { raw = "text/plain" }
  response = { raw = "text/plain" }
  ```
- **The instances.** A, B and C are instances of `lab/ledger`, each
  implementing `ledger.v1`, and the test chooses their instance ids, A's
  lowest, then B's, then C's (the reference: a test hook of
  `ServiceBuilder`). Each runs on three sessions of its own, clients of
  the router its section names (§2.5 step 1, 0.2, F-204): the claim's,
  holding its descriptor and its tokens, its HLC on (§2.10); a watch
  session, holding its liveliness subscriber (§2.7); and a reader, where
  it makes every read. Each answers `append` with its own name and counts
  its executions, and puts `balance` with its own name as the value when
  it takes over. The retry interval is 2 s and the grace 2 s.
- **S** is a tool session, a client of R1, that follows
  `zk2/lab/**/@zk/**` with a liveliness subscriber on a watch session
  (core §8.1, 0.37) and subscribes to `zk2/lab/**`, recording each
  delivery's arrival on its monotonic clock, its key, its kind, its
  payload and its stamp. Where a section cuts a link, **S2** does the same
  as a client of R2. **"Before the token"** is observed by a GET of the
  instance's descriptor that S makes the moment it hears each of the
  instance's tokens, as core `presence.md` §1 observes core §8.2's order:
  the descriptor read at the instance token, and at the interface token.
  S's arrival order of a put and a token is noted, never judged: zenoh
  carries a token's declaration and a put on two priorities (0.2,
  F-202). **G** GETs as core S4 says, and reads presence as
  core §8.1 says, on a session that has never held a liveliness
  subscriber (0.38).
- **A crash.** Where a section kills an instance's process, the test
  either makes the instance's links silent first (a proxy between it and
  its router that stops forwarding and closes nothing: a lost host, seen
  at the lease, which the instance's configuration sets short), or kills
  it as it runs (its host's kernel closes its links: seen at once). §1
  and §10 run both (0.2, F-201).
- **"A serves"** means: A holds `zk2/lab/ledger/@zk/alive/ledger.v1/<A>/<fp16>`,
  its descriptor lists `ledger.v1` without the mark, and its `append`
  queryable answers. **"A stands by"** means: A holds its instance token
  and no `alive/ledger.v1/…` token, its descriptor lists `ledger.v1` with
  `"standby": true` and `"token": true`, and an `All` GET on
  `zk2/lab/ledger/ledger.v1/**`, and a concrete call to `append`, are
  answered by no queryable of A's.
- **The owner example** (#841, RB). The reference's instances are
  `zenkey/examples/owner.rs` run with `--standby`, whose help is the
  interface a black-box runner drives, as for `alarms.v1`. With
  `--wait-go` it starts only at a `go` line on stdin, so that a runner
  holds two starts together (§3), reading its standard input from its
  own start (0.4, F-220); `redundancy zid` answers its claim's, reader's
  and watch's session zids, and `redundancy reads` what each of its
  poll's reads found (0.3, F-216).
- **`ledger-appendall`** (0.4, F-221): `ledger.v1` with `entries` left
  out and a second operation whose name begins with the first's, its
  fan-out allowed, so that `append`'s probe, `append$*`, would reach it.
  The reference keeps it as `zenkey/tests/contracts/ledger-appendall.v1.toml`:

  ```toml
  [resources.appendall]   # beside ledger.v1's balance and append
  kind       = "operation"
  request    = { raw = "text/plain" }
  response   = { raw = "text/plain" }
  fanout     = "allowed"
  idempotent = true
  ```

## §1 Takeover after a crash, seen at the lease (§2.4–§2.8; core §6, §8.1, §8.2)

**Setup.** R1 up. Nothing of `lab/ledger` runs.

**Steps.**
1. A starts.
2. B starts, 2 s later.
3. A client calls `append` 20 times, `BestMatching`, each after the last
   reply.
4. The test makes A's links silent, then kills A's process (SIGKILL), at
   instant K. A second run of the section kills A without the silence.
5. The client calls `append` once, as soon as S sees B's interface token.

**Expected.**
1. A claims before it reads: S's GET at A's instance token answers A's
   descriptor marked standby, unless A took over before that GET was
   answered: its interface token at S within a round trip of the answer,
   the two riding two priorities (§2.5 step 1, core §8.2 step 3; F-202;
   0.4, F-223); its GET at A's interface token answers
   it unmarked, the token last (§2.6, core §8.2, 0.39). A serves within 3 s
   of its start.
2. B stands by (§2.4), and stays so: S's GET at B's instance token answers
   it marked, no interface token of B's appears at any step before 4, and
   every descriptor of B's that S receives marks the interface.
3. 20 executions on A, none on B (spike S6's standby).
4. S hears A's tokens withdrawn when R1's lease for A's sessions runs
   out, A's lease after K, give or take a keep-alive; in the second run,
   at once (§2.8, 0.2). B, standing by, reads, finds no holder in a
   complete read, and takes over (§2.8): S's GET at B's interface token
   answers B's descriptor unmarked, within 3 s of A's withdrawal. B made
   no read between K and the withdrawal, and never polled: its reads
   follow its watch's events (§2.7).
5. The call executes on B.

## §2 Takeover after a clean stop (§2.6, §2.8)

**Setup.** A serves; B stands by.

**Steps.**
1. A closes cleanly, at instant Q.

**Expected.**
1. A's interface token is withdrawn at once, without a lease (§2.8); B
   serves within 2 s of Q, in §2.6's order. A puts and deletes nothing of
   `ledger.v1`'s keys while it closes.

## §3 A simultaneous start (§2.5, §2.9)

**Setup.** R1 up. Nothing of `lab/ledger` runs.

**Steps.**
1. A and B start within 10 ms of each other, measured at S as the arrival
   of each instance's instance token (0.3, F-216). The test runs this step
   20 times, each with fresh ids, A's always the lower, stopping both
   cleanly in between; a run whose two instance tokens arrive more than
   10 ms apart is made again, and counted apart.

**Expected.**
1. In every run, one serves and the other stands by within 5 s of the
   later start plus two graces: A, unless B took over before A's read, A
   then standing by on B's token, as §2.5 step 2 says (0.2, F-203). S's
   GET at each instance's instance token answers its descriptor marked
   standby, unless it took over before that GET was answered: its
   interface token at S within a round trip of the answer, the two riding
   two priorities (0.4, F-223; F-202), and each
   instance's first read follows its own claim (§2.5 step 1). In a run
   where both read before either claim reached the other, both take over;
   then B, the higher, steps down as §7 expects, within two graces of
   seeing A's token. The test records how many runs needed the heal, and
   in how many B kept serving, and judges only the end state.

## §4 A live incumbent, and a lower newcomer (§2.5 step 2)

**Setup.** B serves, alone.

**Steps.**
1. A starts, its id lower than B's.
2. The test waits 10 s.

**Expected.**
1. A stands by. No interface token of A's appears at any time.
2. B still serves, and never stepped down: a live incumbent wins whatever
   the newcomer's id (ZenSight #1104).

## §5 An unreadable claim set (§2.5 step 3)

**Setup.** R1 down at first; up, it gossips, zenoh's default. A's
sessions are peers whose only endpoint is R1's, listening on loopback and
gossiping, so that they link to each other through R1, since a peer reads
only what its routers' clients and its linked peers hold (§2.5 step 1;
core §8.1, 0.40; 0.2, F-206), and their connect retry period is 1 s at
most (0.2, F-205). Separately, R3, a router whose access control denies
A's principal `liveliness_query` on the query's ingress (core §8.1, 0.8:
such a read is answered complete and empty).

**Steps.**
1. A starts, its sessions peers of R1 (the Setup's), while R1 is down: its
   sessions reach no node.
2. R1 starts.
3. Separately, A starts, its sessions clients of R3. The test waits 10 s.

**Expected.**
1. A makes no read through its cut-off session (core §8.1, 0.37), and
   stands by: no interface token, no queryable of `ledger.v1`. It reports
   its reads not made, never "the only candidate".
2. Within the retry interval, plus its reconnect (1 s at most, the
   Setup's), plus 2 s, of R1's start, A's next campaign reads complete,
   finds its own claim, and A serves (0.2, F-205).
3. Every read A makes is complete and empty, its own claim missing, so A
   stands by and campaigns again at each retry interval, and never serves
   within the 10 s. The same read on A's claim's own session returns the
   claim (core §8.1, 0.40), which is why A reads on its reader (§2.5 step
   1, 0.2, F-204). A control, the same instance as a client of a router
   with no access control, serves within 3 s.

## §6 A withdrawal that is no vacancy (§2.7, §2.8; core §8.1)

**Setup.** A serves, a client of R1; B stands by, a client of R2; R2 is
linked to R1. The test can cut B's own link to R2.

**Steps.**
1. The test cuts B's link to R2 for 1 s, then restores it.

**Expected.**
1. B's watch hears A's tokens withdrawn when its link drops (its own
   session's view). B makes no read while its session reaches no node;
   once restored, its read shows A's interface token, and B stands by.
   B never takes over, and no interface token of B's appears at R1 (S)
   at any time.

## §7 The heal of a split (§2.9; core §6)

**Setup.** A, a client of R1, serves; B, a client of R2, stands by. R2 is
linked to R1 through a path the test can cut and heal.

**Steps.**
1. The test cuts the link between R1 and R2.
2. The test waits until S2 sees B's interface token.
3. A puts `balance` `a1`; B puts `balance` `b1`.
4. The test heals the link, at instant H.

**Expected.**
1. Through S2: R2 withdraws A's tokens, at the link's loss or at A's lease;
   B reads, finds no holder, and takes over (§2.8). Through S: A still
   serves, and sees nothing of B's interface token.
2. Two incumbents, one on each side: the partition's split, by design.
3. Each side's subscribers receive their own incumbent's put.
4. After H, S and S2 each see both interface tokens. B, the higher, steps
   down within two graces plus 2 s of H, in §2.9's order: its writes
   stopped, then its interface token withdrawn, then its queryables gone,
   then its descriptor put marked standby. Observed (0.2, as F-202 asks
   of an order): every put or delete stamped with B's `meta.zid` that S or
   S2 records is dated before the withdrawal of B's token; afterwards an
   `All` GET on `zk2/lab/ledger/ledger.v1/**` is answered by A alone, B's
   descriptor reads marked, and a write by B is refused (§2.4). A never
   steps down. A stands alone as the incumbent from then on.

## §8 A call finds no standby (§2.4; core O1, O3)

**Setup.** A serves; B stands by; a caller X, all on R1.

**Steps.**
1. X calls `append` 20 times with `BestMatching` and consolidation `None`.
2. X GETs `zk2/lab/ledger/ledger.v1/@op/append` with target `All` and
   consolidation `None`, with a concrete key, as a fan-out caller would.
3. A closes cleanly. X calls `append` at once, before B serves.

**Expected.**
1. 20 executions on A, none on B: B declares no operation queryable, and
   no `unavailable` one.
2. One reply, A's.
3. No value and no envelope from B: the call ends silent, or with zenoh's
   own error reply, which X reads as silence (core O5).

## §9 A consumer following the incumbent during a split (§2.10; core S7)

**Setup.** As §7, the split established (§7 steps 1 and 2), each incumbent
stating its `meta.zid` and stamping every put with its own HLC (§2.10). K
is a consumer of `ledger.v1`, a client of R1, bound to `lab/ledger`, that
follows the incumbent (§2.10). P is a consumer on R1 that reads by core S7
alone.

**Steps.**
1. The test heals the link. Before B steps down, A and B each put
   `balance` 5 times, alternately, 100 ms apart, A's values `a<n>` and B's
   `b<n>`; the test holds B's step-down for this step (the reference: a
   test hook delaying B's heal reads).
2. K restarts during step 1, and seeds (core §4.2).
3. B steps down.

**Expected.**
1. P's value moves at each put, between A's and B's: each is the first
   value under its id since the other's (core S7), the flapping §2.10
   ends. K's value moves to A's values only, in A's order, and K discards
   every sample stamped with B's `meta.zid`.
2. K's seed holds A's reply, or, where `Latest` kept B's later stamp, no
   value for `balance` until A's next put: never B's (§2.10).
3. K's value stays A's. A sample of B's that was in flight when B stepped
   down, arriving after, is discarded by K.

## §10 `external` (§2.11)

**Setup.** V1 and V2 are instances of `lab/valve` on one host, each
implementing `valve.v1`, sharing the lock file `/run/lab/valve.lock`
(`flock(LOCK_EX | LOCK_NB)`), each standing by while it cannot take the
lock (§2.11). V2's id is lower than V1's.

**Steps.**
1. V1 starts and takes the lock.
2. V2 starts. The test waits 10 s.
3. The test makes V1's links silent, then kills V1's process (SIGKILL), at
   instant K. The kernel releases the lock. A second run of steps 1 to 3
   kills V1 without the silence (0.2, F-201).
4. Separately, V1 and V2 start with two lock files (a misconfiguration),
   V2 first; the test waits 3 s.
5. Separately, V1 and V2, with two lock files, start behind two routers
   whose link is cut, V1 a client of R1 and V2 of R2. Once both serve, the
   link is restored, and the test waits 10 s (0.2, F-200).
6. Separately, V3, configured to refuse rather than stand by, starts while
   V1 holds the lock.

**Expected.**
1. V1 serves, in §2.6's order.
2. V2 stands by, its descriptor marked: no election by ids moves it,
   though its id is lower.
3. V2 takes the lock at once (within its lock period), waits for V1's
   interface token to go (V1's lease after K; at once in the second run),
   and serves within 3 s of that withdrawal; at no time do G's two reads
   a grace apart both show V1 and V2 holding.
4. V2 serves; V1, holding its own lock, stands by on V2's token (§2.5 step
   2, §2.11; 0.2, F-200): two locks make a split only when the reads miss
   each other.
5. Both serve: two interface tokens, and neither steps down within the
   10 s, since `external` never heals (§2.9, §2.11). G's two reads a grace
   apart read a split (§12's first question, no, `split`).
6. V3 refuses to start: no instance token of V3's appears within 5 s,
   watched through R1 against a control (core §3.2's way of observing a
   refusal).

## §11 The tokenless set, and an invalid declaration (§2.2, §2.14; core §8.2)

**Setup.** R1 up, S watching. A control: A configured plainly serves
within 3 s.

**Steps.**
1. A starts with `ledger.v1` in its tokenless set.
2. A starts with a copy of `ledger.v1` whose `[defaults]` says
   `redundancy.election = "vote"`.

**Expected.**
1. A refuses to start (core §8.2 step 2, 0.39): no instance token of A's
   appears within 5 s, nothing of `lab/ledger` is declared or put.
2. The same refusal: the declaration is invalid, `value` (§2.2). The
   declaration is checked before anything of the interface is written:
   the owner example says `not started: … invalid: value`, never a
   standby's refused write (0.3, F-214).

## §12 A tool's questions (§2.15, §5; core §6; hostid.v1 §2.12)

**Setup.** T is the tool under test, reading `lab/ledger` twice, 2 s apart,
on a session that has never held a liveliness subscriber, and holding
`ledger.v1` offline: in steps 3 and 5 no candidate serves its bundle, and
without it T answers unobservable, `no_contract` (§2.15; 0.3). X is a test
session that can declare tokens under `lab/ledger`, serve a descriptor it
writes, and declare a queryable, to build each shape.

**Steps.**
1. A serves and B stands by, as §1 step 2.
2. The split of §7 step 2, T a client of R1 after the heal and before B's
   step-down (the test holding the step-down, as §9).
3. B stands by, and no instance serves: X declares an instance token and
   serves a descriptor marking `ledger.v1` standby, B having stopped, A
   never started.
4. A serves; X declares an instance token and a descriptor listing
   `ledger.v1` unmarked, and no interface token.
5. No instance of `lab/ledger` runs; X declares an instance token and an
   interface token of `ledger.v1`, and a descriptor marking it standby.
6. A serves and B stands by, on a system minted by `hostid.v1`, each
   descriptor listing `hostid.v1`.
7. The doctor (`zenctl doctor`) runs against step 1's shape and step 2's.
8. A serves and B stands by; T probes their exposure (§2.15; the
   reference: `zenctl check conform`). Then X declares an instance token,
   serves a descriptor marking `ledger.v1` standby, and declares a
   queryable on `append`'s key, and T probes again. Then, apart, A and B
   run `ledger-appendall`, T holding it, and T probes them (0.4, F-221).
9. A serves and B stands by, both running `ledger.v1`, which is all the
   address's descriptors name. T holds `ledger-appendall` offline (the
   reference: `--contracts`), a revision of the same interface no
   instance runs, and probes them (0.5, F-226).

**Expected.** T answers as `conclusions.json` does for the same reading:
1. Exactly one incumbent: yes, `one`; a vacancy: no, `held`; standbys:
   yes, 1, exact; A: no, `incumbent`; B: no, `standby`.
2. Exactly one: no, `split`; A: yes, `split_lowest`; B: yes, `split`.
3. Exactly one: no, `vacant`; a vacancy: yes, `lasting`.
4. X: yes, `unmarked`.
5. X: yes, `marked_holding`; exactly one incumbent: yes, `one`, since a
   holder is an incumbent whatever its mark (§2.3).
6. `hostid.v1` §2.12's second process is B, recognised as a standby and
   not counted: no finding, and the tool names B a recognised standby
   beside its answer (§2.15; 0.3).
7. Step 1: no `split-brain` finding and no `token-missing` finding for B
   (core §6, 0.39: its listing claims nothing exposed), and no lasting
   vacancy. Step 2: one `split-brain` finding, naming A and B, as this
   profile's `split` (§2.15, "One split, one finding"; 0.3).
8. The probe over `append`'s wildcard is answered by A's session alone,
   attributed by its zid: B exposes nothing. Then X's session answers it
   too: X, marked standby with no token in both reads, holds a queryable
   of the interface, the finding (§2.4, §2.15; 0.3). On
   `ledger-appendall`, T asks neither `append$*`, which reaches
   `appendall`, nor `appendall`, and says why: its exposure is
   unobservable, and `appendall`'s handler never runs (0.4, F-221).
9. T asks no `append$*` either: the revision it holds offline declares
   `appendall`, fan-out allowed, which the wildcard reaches, so the call
   could run it on an instance that runs that revision (§2.15, "Never a
   call that would run"). T says so, naming `appendall` and the revision
   held offline (the reference: `@op/append not probed: its probe
   zk2/lab/ledger/ledger.v1/@op/append$* would reach
   zk2/lab/ledger/ledger.v1/@op/appendall, an operation whose fanout is
   "allowed" (revision sha256:…), and run it`), and the standbys'
   exposure is unobservable: `redundancy-standby` reads unobservable,
   exit 2. No instance at the address has an `appendall` to run (0.5,
   F-226).
