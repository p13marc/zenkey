# `redundancy.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §12 are
[`redundancy.v1`'s scenarios](scenarios.md), restated where a fence, a
stamp or an answer changes. §13 to §15 are new. §12 is a tool's, and the
others the runtime's.

**Who runs them.** The reference runs §1 to §11 and §13 to §15 as tests
of `zenkey`, one per section, named after it. They are in
`zenkey/tests/profile_redundancy_v2.rs`. §12 is to run as a test of
`zenctl` and of the doctor (`zenctl/tests/live_zk2.rs`). No tool runs it
yet.

**Conventions.**
- **Section numbers** in parentheses (§2.10) are v2.md's rules; `v1 §n`
  is a section of `redundancy.v1`'s text, and core sections say "core".
- **By reference.** A section that says "as v1's" is
  [`scenarios.md`](scenarios.md)'s section of the same number, with the
  contracts, the sessions and the readings below. Where v1's names a
  stamp's id or `meta.zid`, it reads a run.
- **The bus** is v1's: R1, and R2 where a section cuts a link. Timeouts
  are 1 s unless stated. R1 and R2 run zenoh's defaults, so each stamps
  a put that arrives unstamped (core §4.1).
- **The contracts.** `ledger-run.v1` is `ledger.v1` at minor 1, its
  `uses` moved to this major. `valve-run.v1` is `valve.v1` at minor 1,
  moved the same way. The reference keeps them as
  `zenkey/tests/contracts/ledger-run.v1.toml` and `valve-run.v1.toml`,
  beside v1's:

  ```toml
  # ledger-run.v1.toml: ledger.v1's revision 1.1, a single writer fenced by run
  [interface]
  name  = "ledger"
  major = 1
  minor = 1
  uses  = ["redundancy.v2"]

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

  `valve-run.v1.toml` is `valve.v1.toml` with `minor = 1` and `uses =
  ["redundancy.v2"]`.
- **The instances.** A, B and C are instances of `lab/ledger` as in
  v1's, implementing `ledger-run.v1` unless a section says otherwise. Each
  runs v1's three sessions, and none of them timestamps (§2.10). Each
  draws a 32-bit run and states it in its descriptor from its claim on.
  A test hook can choose an instance's first run, and make it take a new
  run on demand (core §4.3).
- **S** is v1's tool session. It records each delivery's payload, and
  whether it carries a Zenoh timestamp and whose id that is: an
  instance's `meta.zid`, a router's, or none.
- **K** is a follower of `ledger.v1` (§2.10), a client of R1, bound to
  `lab/ledger`. It reads both majors (§2.17). It seeds by core S4, and
  reports each sample it accepts or discards, with the rule's reason.
- **The owner example** is v1's, given `ledger-run.v1`. Run with
  `--standby`, it takes part in this major's election as it does in v1's.

## §1 Takeover after a crash, seen at the lease (§2.6, §2.10; v1 §2.4–§2.8)

**As v1's,** with these added to its Expected:
- **To Expected 1.** S's GET at A's instance token answers A's
  descriptor with A's run, and its GET at A's interface token the same
  run, unmarked.
- **To Expected 2.** Every descriptor of B's that S receives, marked,
  states one run, B's.
- **To Expected 4.** B's descriptor put unmarked at its takeover states
  the run it stated while it stood by. B's put of `balance` at its
  takeover is under that run, at seq 0. It carries no Zenoh timestamp of
  B's: S records R1's id on it.

## §2 Takeover after a clean stop (v1 §2.6, §2.8)

As v1's.

## §3 A simultaneous start (v1 §2.5, §2.9)

As v1's.

## §4 A live incumbent, and a lower newcomer (v1 §2.5 step 2)

As v1's.

## §5 An unreadable claim set (v1 §2.5 step 3)

As v1's.

## §6 A withdrawal that is no vacancy (v1 §2.7, §2.8)

As v1's.

## §7 The heal of a split (§2.10; v1 §2.9)

**As v1's,** with Expected 4's observation by run. S and S2 record no
put or delete under B's run after B's descriptor put marked standby.
B makes that put after its writes ended (v1 §2.9, the step-down's step
3), and one session's puts on one priority arrive in order.

## §8 A call finds no standby (§2.10; v1 §2.4; core O1, O3)

**As v1's,** with Expected 2 by run: the one reply is A's, and its
value's run is A's run (core §5.1). X reads it so (§2.10, "Callers").

## §9 A follower during a split (§2.10; core S4, S7)

**Setup.** As v1's §7, the split established (v1's §7 steps 1 and 2):
A serves behind R1 and B behind R2, each stating its own run. K follows
`lab/ledger` (§2.10). P is a consumer on R1 that reads by core S7 alone.

**Steps.**
1. The test heals the link. Before B steps down, A and B each put
   `balance` 5 times, alternately, 100 ms apart, A's values `a<n>` and
   B's `b<n>`. The test holds B's step-down for this step (the
   reference: a test hook delaying B's heal reads, the owner example's
   `--hold-heal`).
2. K restarts during step 1, and seeds (core S4).
3. B steps down.

**Expected.**
1. P's value moves at each put, between A's and B's: each is the first
   value under its run since the other's (core S7). K's moves to A's
   values only, in A's order. K discards every sample under B's run,
   `not_incumbent`.
2. No put of A's or B's carries a Zenoh timestamp of its own. S records
   each with its router's id, never A's `meta.zid` nor B's (§2.10).
3. K's seed: under S4's `None` both snapshots of `balance` arrive, A's
   under A's run and B's under B's. K merges A's and discards B's,
   `not_incumbent`. Where A's did not arrive, K holds no value for
   `balance` until A's next put: never B's.
4. K's value stays A's. A sample of B's that was in flight when B
   stepped down, arriving after, is discarded by K, `not_incumbent`.

## §10 `external` (v1 §2.11)

As v1's, with `valve-run.v1` for `valve.v1`.

## §11 The tokenless set, and an invalid declaration (§2.2, §2.13; v1 §2.14; core §8.2)

**Setup.** As v1's.

**Steps.**
1. As v1's step 1, with `ledger-run.v1`.
2. As v1's step 2, with a copy of `ledger-run.v1`.
3. A starts with a copy of `ledger-run.v1` whose `entries` declares
   `explicit = true`.
4. A starts configured for a first hop (core §8.1), its every interface
   in its tokenless set, as core `constrained.md` §10 step 6 starts
   `zs.catalog.v1`.

**Expected.**
1. and 2. As v1's.
3. The same refusal: the declaration is invalid, `explicit_stream`
   (§2.2). The owner example says `not started: … invalid:
   explicit_stream`.
4. The same refusal: `ledger-run.v1` in a tokenless set (§2.13, v1
   §2.14). No instance token of A's crosses to R1.

## §12 A tool's questions (§2.15, §5; v1 §2.15; core §6)

**As v1's,** T reading `ledger-run.v1` offline, with this step and
Expected added:
- **Step 10.** A serves and B stands by, as step 1. T shows the
  address.
- **Expected 10.** T names each candidate's run beside its instance, as
  its descriptor states it: A's beside A, B's beside B (§2.15).

v1's step 6 reads `hostid.v1` §2.12's recognised standby, which names a
standby of `redundancy.v1` alone today. Under this major it runs once
that section names either major.

## §13 A takeover seen by a follower (§2.6, §2.10)

**Setup.** A serves and B stands by, on R1. K follows `lab/ledger`, and
has seeded: it holds A's `balance`, `a1`.

**Steps.**
1. A puts `balance` `a2`.
2. The test kills A's process. B takes over, and puts `balance` `b1`.
3. A second run of steps 1 and 2: B takes over holding no value of
   `balance`, and puts none (the reference: an option of the owner
   example).

**Expected.**
1. K holds `a2`, under A's run.
2. K hears A's tokens withdrawn, and sees no incumbent.
   - **B's put of `b1`** arrives before or after B's interface token,
     since the two ride two priorities. Before the token, K discards it,
     `no_incumbent`. After it, K accepts it under B's run, which B's
     descriptor stated from its claim on.
   - **K seeds again** when B's token appears (§2.10): one GET by core
     S4. B answers it with one snapshot of `balance`, under B's run,
     never a reply per key (§2.6).
   - K holds `b1` either way, within a round trip of B's token.
3. B's snapshot of `balance` is complete and lists no entry. K applies
   core §4.2's omission rule: it drops `a2`, and holds no value for
   `balance`. Under v1, K kept `a2` until B's next put.

## §14 An incumbent's new run, and a run another instance states (§2.10)

**Setup.** A serves alone on R1. K follows `lab/ledger`, and holds A's
`balance`. K's subscriber on the address's descriptors is delayed by
500 ms (the reference: a test hook of K's), and its GETs are not.

**Steps.**
1. A takes a new run (core §4.3), puts its descriptor stating it, then
   puts `balance` `a2` under it.
2. X, a test session, puts `balance` `x1` under a run no descriptor
   states.
3. Separately, no instance of `lab/ledger` runs. X declares an instance
   token whose id is lower than C's, and serves a descriptor marking
   `ledger.v1` standby whose run is R. C starts, its first run chosen R.
4. X withdraws its instance token.

**Expected.**
1. K receives `a2` before A's delayed descriptor put. K holds it,
   unattributed, and asks one read of the address's descriptors. When
   that read ends, K accepts `a2` under A's new run, before the delayed
   put arrives. K never discards it.
2. K holds `x1`, asks a read, and discards `x1` when that read ends,
   `unattributed`. K's value stays `a2`.
3. C's first campaign reads X's descriptor, a lower id's (v1 §2.5 step
   3), and finds its own run there. C draws a new run, and puts its
   descriptor stating it, before it serves. It stands by, X being a
   lower candidate (v1 §2.5 step 4).
4. C takes over under its new run, never under R.

## §15 Both majors side by side (§2.17)

**Setup.** On R1, A implements `ledger.v1` (`redundancy.v1`), its
sessions stamping as v1 §2.10 asks. B implements `ledger-run.v1`
(`redundancy.v2`). A's id is lower. K follows `lab/ledger`, reading both
majors. K1 is a follower built for `redundancy.v1` alone: it follows the
incumbent by `meta.zid` (v1 §2.10).

**Steps.**
1. A starts, then B. A puts `balance` `a1`.
2. The test kills A's process. B takes over, and puts `balance` `b1`.
3. A starts again, a new instance, its id lower than B's.

**Expected.**
1. A serves and B stands by: one election (v1 §2.5). A tool reads the
   address governed, `claim`, never `revisions_disagree` (§2.2).
   - A's put carries A's Zenoh timestamp, its id A's `meta.zid`. K
     follows A by `meta.zid` and accepts `a1`. So does K1.
2. B's puts carry no Zenoh timestamp of B's. K follows B by run from
   B's token on: it seeds again, and holds `b1`.
   - K1 finds no stamp of B's `meta.zid` on any of B's samples. It
     discards each, and holds `a1`: safe, and inert (§2.17).
3. The new A stands by on B's token (v1 §2.5 step 2). B keeps serving,
   and K's value stays B's.
