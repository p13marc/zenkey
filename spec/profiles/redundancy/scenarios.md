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
  lease is its default, 10 s.
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
  implementing `ledger.v1` in its own session, and the test chooses their
  instance ids, A's lowest, then B's, then C's (the reference: a test hook
  of `ServiceBuilder`). Each answers `append` with its own name and counts
  its executions, and puts `balance` with its own name as the value when
  it takes over. The retry interval is 2 s and the grace 2 s.
- **S** is a tool session, a client of R1, that follows
  `zk2/lab/**/@zk/**` with a liveliness subscriber on a watch session
  (core §8.1, 0.37) and subscribes to `zk2/lab/**`, recording each
  delivery's arrival on its monotonic clock, its key, its kind, its
  payload and its stamp. Where a section cuts a link, **S2** does the same
  as a client of R2. **G** GETs as core S4 says, and reads presence as
  core §8.1 says, on a session that has never held a liveliness
  subscriber (0.38).
- **"A serves"** means: A holds `zk2/lab/ledger/@zk/alive/ledger.v1/<A>/<fp16>`,
  its descriptor lists `ledger.v1` without the mark, and its `append`
  queryable answers. **"A stands by"** means: A holds its instance token
  and no `alive/ledger.v1/…` token, its descriptor lists `ledger.v1` with
  `"standby": true` and `"token": true`, and an `All` GET on
  `zk2/lab/ledger/ledger.v1/**`, and a concrete call to `append`, are
  answered by no queryable of A's.
- **The owner example** (#841, RB). The reference's instances are
  `zenkey/examples/owner.rs` run with `--standby`, whose help is the
  interface a black-box runner drives, as for `alarms.v1`.

## §1 Takeover after a crash, seen at the lease (§2.4–§2.8; core §6, §8.1, §8.2)

**Setup.** R1 up. Nothing of `lab/ledger` runs.

**Steps.**
1. A starts.
2. B starts, 2 s later.
3. A client calls `append` 20 times, `BestMatching`, each after the last
   reply.
4. The test kills A's process (SIGKILL), at instant K.
5. The client calls `append` once, as soon as S sees B's interface token.

**Expected.**
1. A claims before it reads: S receives A's first descriptor, marked
   standby, then its instance token (§2.5 step 1, core §8.2 step 3), and
   later, once A takes over, its descriptor without the mark, then its
   interface token, the token last (§2.6, core §8.2, 0.39). A serves
   within 3 s of its start.
2. B stands by (§2.4), and stays so: no interface token of B's appears at
   any step before 4, and B's descriptor never loses the mark.
3. 20 executions on A, none on B (spike S6's standby).
4. S hears A's tokens withdrawn when R1's lease for A's session runs out,
   10 s after K by default, give or take a keep-alive; B, standing by,
   reads, finds no holder in a complete read, and takes over (§2.8): S
   receives B's descriptor without the mark, then B's interface token,
   within 3 s of A's withdrawal. B made no read between K and the
   withdrawal that concluded a vacancy, and never polled: its reads follow
   its watch's events (§2.7).
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
1. A and B start within 10 ms of each other. The test runs this step 20
   times, each with fresh ids, A's always the lower, stopping both cleanly
   in between.

**Expected.**
1. In every run, A serves and B stands by within 5 s of the later start
   plus two graces. Each instance's descriptor, marked standby, reaches S
   before its instance token, and each instance's first read follows its
   own claim (§2.5 step 1). In a run where both read before either claim
   reached the other, both take over; then B, the higher, steps down as §7
   expects, within two graces of seeing A's token. The test records how
   many runs needed the heal, and judges only the end state.

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

**Setup.** R1 down at first. Separately, R3, a router whose access control
denies A's principal `liveliness_query` on the query's ingress (core §8.1,
0.8: such a read is answered complete and empty).

**Steps.**
1. A starts as a peer whose only endpoint is R1's, while R1 is down: its
   session reaches no node.
2. R1 starts.
3. Separately, A starts as a client of R3. The test waits 10 s.

**Expected.**
1. A makes no read through its cut-off session (core §8.1, 0.37), and
   stands by: no interface token, no queryable of `ledger.v1`. It reports
   its reads not made, never "the only candidate".
2. Within the retry interval plus 2 s of R1's start, A's next campaign
   reads complete, finds its own claim, and A serves.
3. Every read A makes is complete and empty, its own claim missing, so A
   stands by and campaigns again at each retry interval, and never serves
   within the 10 s. A control, the same instance as a client of a router
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
   down within two graces plus 2 s of H, in §2.9's order: its interface
   token withdrawn first, then its queryables gone (an `All` GET on
   `zk2/lab/ledger/ledger.v1/**` is answered by A alone), then its
   descriptor put marked standby. B puts and deletes none of `ledger.v1`'s
   keys from the withdrawal of its token on (S records no put or delete
   stamped with B's `meta.zid` after it). A never steps down. A stands
   alone as the incumbent from then on.

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
3. The test kills V1's process (SIGKILL), at instant K. The kernel
   releases the lock.
4. Separately, V1 and V2 start with two lock files (a misconfiguration),
   V2 first; the test waits 10 s.
5. Separately, V3, configured to refuse rather than stand by, starts while
   V1 holds the lock.

**Expected.**
1. V1 serves, in §2.6's order.
2. V2 stands by, its descriptor marked: no election by ids moves it,
   though its id is lower.
3. V2 takes the lock at once, waits for V1's interface token to go (V1's
   lease, 10 s after K), and serves within 3 s of that withdrawal; at no
   time do S's two reads a grace apart both show V1 and V2 holding.
4. Both serve: two interface tokens, and neither steps down. G's two reads
   a grace apart read a split (§12's first question, no, `split`).
5. V3 refuses to start: no instance token of V3's appears within 5 s,
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
2. The same refusal: the declaration is invalid, `value` (§2.2).

## §12 A tool's questions (§2.15, §5; core §6; hostid.v1 §2.12)

**Setup.** T is the tool under test, reading `lab/ledger` twice, 2 s apart,
on a session that has never held a liveliness subscriber. X is a test
session that
can declare tokens under `lab/ledger` and serve a descriptor it writes, to
build each shape.

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

**Expected.** T answers as `conclusions.json` does for the same reading:
1. Exactly one incumbent: yes, `one`; a vacancy: no, `held`; standbys:
   yes, 1, exact; A: no, `incumbent`; B: no, `standby`.
2. Exactly one: no, `split`; A: yes, `split_lowest`; B: yes, `split`.
3. Exactly one: no, `vacant`; a vacancy: yes, `lasting`.
4. X: yes, `unmarked`.
5. X: yes, `marked_holding`; exactly one incumbent: yes, `one`, since a
   holder is an incumbent whatever its mark (§2.3).
6. `hostid.v1` §2.12's second process is B, recognised as a standby: the
   tool says so beside that finding (§2.15).
7. Step 1: no `split-brain` finding and no `token-missing` finding for B
   (core §6, 0.39: its listing claims nothing exposed). Step 2: one
   `split-brain` finding, naming A and B.
