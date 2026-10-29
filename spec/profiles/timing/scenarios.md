# `timing.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §5 are the runtime's:
the reference runs them as tests of `zenkey`
(`zenkey/tests/profile_timing.rs`, one test per section, named after it;
#908, MB). §6 is a tool's, and the reference tool runs it (#908, MC).

**Conventions.**
- **Section numbers** in parentheses (§2.5) are v1.md's rules; core
  sections say "core".
- **The bus.** R1 is a router with timestamping on, as a router's default
  is, and its HLC's maximum delta the default, 500 ms. The owner, the
  consumer C and the tool T are its clients unless a section says
  otherwise. Timeouts are 1 s unless stated.
- **The contract.** Every section uses `pace.v1`. The reference keeps it
  as `zenkey/tests/contracts/pace.v1.toml`:

  ```toml
  [interface]
  name  = "pace"
  major = 1
  minor = 0
  uses  = ["timing.v1"]

  [resources.cmd]                    # every 100 ms; missed after 500 ms, seen within 510 ms; expired past 300 ms
  kind        = "stream"
  type        = { raw = "text/plain" }
  annotations = { "timing.period_ms" = 100, "timing.deadline_ms" = 500, "timing.lifespan_ms" = 300 }

  [resources."axis/{axis}"]          # one member token per axis
  kind        = "stream"
  type        = { raw = "text/plain" }
  params      = { axis = "string" }
  cardinality = 4
  epoch       = "axis"
  annotations = { "timing.period_ms" = 100, "timing.deadline_ms" = 500 }
  ```

  The deadline's detection bound is max(500/50, 1) = 10 ms (§2.5).
- **The owner** is `lab/pace`, implementing `pace.v1` with every resource
  exposed. Its keys are `zk2/lab/pace/pace.v1/stream/cmd` and
  `zk2/lab/pace/pace.v1/stream/axis/<axis>`. Its session's HLC is on, and
  its descriptor states that session's zid as `meta.zid` (§2.6), unless a
  section says otherwise. It holds a member token for each axis it
  publishes, `zk2/lab/pace/@zk/member/pace.v1/<axis>/<epoch>` (core
  §8.1). Each sample's payload is its sequence number, as text.
- **C** is a consumer of `pace.v1`: its role binds `lab/pace` (core R1),
  and it subscribes to the resource its section names. It follows the
  owner's presence as core §8.1 says (0.37): its liveliness subscribers on
  a watch session, its reads on a session that never held one, and a read
  again 100 ms after a withdrawal it hears. It reads the owner's
  descriptor for `meta.zid`. It records each sample's arrival on its
  monotonic clock, its wall clock then, its stamp, its lifespan's
  judgement, and whether it handed the sample to its application; and each
  deadline report, `missed` with its due instant, `met` and `ended`, with
  the instant it reported it. Its ε, where it states one, is its section's.
  **C's sessions have no HLC** (text 0.2, F-265): a session whose HLC is
  enabled stamps an unstamped put it receives (core §4.1, 0.55), so §5
  step 2's `unstamped` holds only for a reader whose sessions have none;
  one with an HLC reads it `foreign_stamp`.
- **Two instants the runner keeps** (text 0.2, F-266):
  - **C stops a deadline or less after the owner's last sample** of a
    step, or takes its last reading before then: a deadline later the
    member is missed, owner present or gone, and no section's expected
    observations count that miss.
  - **The owner publishes once C holds its descriptor,** where C judges a
    lifespan (§4, §5): C learns `meta.zid` from the descriptor's put and
    its GET (v1.md §2.6), and a sample handled before it reads
    `unattributed`, rightly, which no expected observation counts.
- **The black box** (#908, MB). The reference's owner is
  `zenkey/examples/owner.rs` run with `--timing`, its reader
  `zenkey/examples/consume.rs` run with `--timing`, and its tool `zenctl
  timing` (§6, #908, MC); each one's `--help` is the interface a
  black-box runner drives and reads, every answer and report one JSON
  line. The owner's steps are its standard input's: `timing resume cmd`
  and `timing pause cmd` start and stop `cmd`, `timing every <member>
  <ms>` publishes a member at another interval than its period (§6 step
  3, text 0.3), `timing add <axis>`
  declares an axis's member token and publishes it, `timing declare
  <axis>` declares the token alone and publishes nothing (§3 step 5, text
  0.3, F-276), `timing remove <axis>` stops it and withdraws its token,
  `timing offset <ms>` offsets its stamping clock, and `timing stamp none`
  puts unstamped (§5). For §4 and §5's step 1 and 2 it runs with
  `--no-hlc` (text 0.3, F-273).
- **The host.** A gap between two arrivals carries the bus's jitter, and
  the owner's 100 ms period is a fifth of the deadline, so no step expects
  a miss from jitter, even on a loaded runner. A report's due instant is
  checked exactly, from C's recorded arrivals. Its report instant is
  checked against the due plus the 10 ms bound, and the reference allows
  the runner 50 ms beyond that, since a loaded host delays a timer as it
  delays anything; it records the lateness apart.
- **The owner's clock.** Where a section offsets the owner's stamping
  clock from C's wall clock, the runner sets the owner's, as
  `freshness.v1`'s scenarios §4 does (the reference's `simulate_offset`):
  what §2.6 bounds is the disagreement between the two. Every offset here
  is within R1's delta, so R1 keeps each stamp (core §4.1).

## §1 A commander paused, then resumed (§2.4, §2.5)

**Steps.**
1. C declares its subscription to `cmd`. The owner starts, and publishes
   `cmd` every 100 ms for 2 s.
2. The owner publishes nothing for 2 s, keeping its service, its
   publisher and its tokens.
3. The owner publishes `cmd` every 100 ms for 1 s.
4. At the end of step 2, T reads `lab/pace`'s presence.

**Expected.**
1. C reports nothing: `cmd` is known, its owner present before its first
   sample, and every gap at most 500 ms. Its first arrival reports
   nothing either.
2. C reports `missed` for `cmd` once, its due exactly 500 ms after the
   last arrival of step 1, at most 10 ms after the due. C receives no
   sample of `cmd` in step 2: no runtime publishes on the owner's behalf
   (§2.4). Nothing more is reported of `cmd` in step 2.
3. C reports `met` once, at the first arrival of step 3, and nothing
   after.
4. T reports the owner present, its instance and interface tokens held,
   and C's miss stands beside it: neither is folded into the other (§2.5,
   a miss is never "down").

## §2 A member gone, and a member silent (§2.5)

**Steps.**
1. C declares its subscription to `zk2/lab/pace/pace.v1/stream/axis/*`.
   The owner publishes `axis/x` and `axis/y` every 100 ms each, holding a
   member token for each.
2. After 1 s, the owner stops publishing `axis/y`, and withdraws `y`'s
   member token. It goes on with `axis/x` for 2 s.
3. The owner stops publishing `axis/x`, keeping `x`'s member token, its
   service and its tokens, for 1 s.

**Expected.**
1. C reports nothing: neither member is known, since the template's
   parameter is unbound, so each is judged from its first arrival.
2. C hears `y`'s member token withdrawn, reads again, and the read is
   complete and shows no member token of `y`. C reports `ended` for
   `axis/y` at that read, and never `missed` for it: the read ends long
   before the 500 ms due after `y`'s last arrival. Nothing is reported of
   `axis/x`.
3. C reports `missed` for `axis/x`, due 500 ms after its last arrival,
   within the bound: a member whose token is held is still judged.

## §3 A member never heard (§2.5)

**Steps.**
1. C declares its subscription to `cmd` at instant S, with no owner
   running, and waits 1 s.
2. The owner starts. C first sees its interface token at instant P. The
   owner publishes nothing on `cmd`.
3. 1 s after P, the owner publishes `cmd` once.
4. C2, a second consumer, declares its subscription to
   `zk2/lab/pace/pace.v1/stream/axis/*` while the owner runs and publishes
   no axis, and listens 2 s.
5. C3, a third consumer, binds the template's parameter to `x` (core R2),
   so `axis/x` is known, and listens 1 s while the owner runs and declares
   no axis. Then the owner declares `x`'s member token, at instant M as C3
   first sees it, and publishes nothing on it (text 0.2, F-261; the
   reference's owner: `timing declare x`, text 0.3, F-276).

**Expected.**
1. C reports nothing: `cmd` is known, but its owner is not present, so
   nothing is waited for.
2. C reports `missed` for `cmd`, due at P + 500 ms, the later of S and P
   plus the deadline, within the bound.
3. C reports `met` at the arrival.
4. C2 reports nothing: no axis member is known, and none arrived.
5. C3 reports nothing in its first second, its owner present: an epoch
   template's known member is waited for from its member token, never
   from its owner's presence. Then `missed` for `axis/x`, due at M + 500
   ms, within the bound.

## §4 The lifespan on a stated bound (§2.5, §2.6)

**Setup.** Each step runs a new instance of the owner and a new C, whose
subscription to `cmd` is declared before the owner starts. The owner's
session has no HLC, so the runner's offset moves its stamps alone (v1.md
§2.6, "Without an HLC too"): an HLC follows the first stamp of its own
that runs ahead (core §4.1, 0.53), and its clock would then stand still
(text 0.3, F-273). The owner publishes `cmd` every 100 ms for 2 s. C
states ε = 100 ms unless the step says otherwise.

**Steps.**
1. The owner's clock agrees with C's.
2. The owner's clock runs 700 ms behind C's.
3. The owner's clock runs 400 ms ahead of C's for the first second, and
   agrees with it for the second.
4. The owner's clock runs 700 ms behind C's, and C states no ε.

**Expected.**
1. Every sample's stamp carries the owner's `meta.zid`, every sample is
   live, and C hands every one to its application. C reports no deadline
   transition.
2. Every sample is expired: its age is about 700 ms, and 700 − 100 is
   above 300. C hands none to its application, and reports `missed` for
   `cmd`, due 500 ms after the instant C first saw its owner present, since
   no sample is an arrival; and no `met` in the step. The deployment's
   word was wrong here, and so is every verdict C draws from it: the
   error is the deployment's (§2.6).
3. Every sample of the first second is `clock_disagrees`: its stamp is
   about 400 ms ahead of C's wall clock, more than ε. Every sample of the
   second second is `clock_disagrees` too: the clock's word was falsified
   for the reading, though its stamps now agree. C hands every sample to
   its application, and reports no deadline transition.
4. Every sample is unobservable, `no_bound`, though R1's delta is 500 ms:
   ε is never the delta. C hands every one to its application, and
   reports no deadline transition.

In steps 1, 3 and 4 the deadline is met whatever the offset: it runs on
C's monotonic clock (§2.5).

## §5 An unstamped owner (§2.6)

**Setup.** The owner's session has no HLC, and sets no stamp on its puts:
it breaks §2.6's MUST, as this section means it to. Its descriptor still
states its session's zid as `meta.zid`. Its payload is its own wall clock
minus 10 s, in nanoseconds since the Unix epoch, as text: a time
`twist_cmd.v1`'s `issued_at_ns` could carry, which would read every
sample expired if a reader aged by it. C states ε = 100 ms.

**Steps.**
1. Through R1, the owner publishes `cmd` every 100 ms for 2 s.
2. R2 is a router with timestamping off, and the owner and C are its
   clients. The owner publishes `cmd` every 100 ms for 2 s.
3. **Control.** Through R1, the owner with its session's HLC on publishes
   `cmd` every 100 ms for 2 s, the same payloads.

**Expected.**
1. Every sample arrives stamped by R1: its id is R1's zid, not the owner's
   `meta.zid`. Each is unobservable, `foreign_stamp`. C hands every one to
   its application, and reports no deadline transition.
2. Every sample arrives with no stamp: unobservable, `unstamped`. C hands
   every one to its application.
3. Every sample's stamp carries the owner's `meta.zid`, and every sample
   is live. In no step does C read the payload's time.

## §6 A tool's window (§2.7, §5)

**Setup.** T, a tool, reads `lab/pace` over a 5 s window. Before the
window starts it follows the owner's presence as C does and holds its
descriptor (v1.md §2.7, text 0.2, F-263); then it subscribes to `cmd` and
to `axis/*`, the window's start its subscription's start, and states ε =
100 ms. It answers §5's three questions of each member it held in the
window, at any instant of it (F-267). The owner publishes `axis/x` every
100 ms in every step.

**The reference** (#908, MC). T is `zenctl timing 'zk2/lab/pace/**' --for
5 --clock-bound 100 --format json`, a new run per step, its stderr's
`timing: listening` line the window's start; the owner is the owner
example's drive (`timing resume cmd`, `timing add x`, `timing pause cmd`
and `timing resume cmd` for step 2, `timing every cmd 250` for step 3).
The reference runs the section as `zenctl/tests/live_zk2.rs`'s
`timing_s6_a_tools_window`.

**Steps.**
1. The owner publishes `cmd` every 100 ms throughout. T reads.
2. T reads again. 2 s into the window, the owner stops `cmd` for 1 s,
   keeping its tokens, then resumes it.
3. T reads again. The owner publishes `cmd` every 250 ms throughout:
   slower than its period, within its deadline.
4. T reads again. 1 s into the window, the owner closes cleanly: its
   tokens are withdrawn at once.

**Expected.**
1. `cmd`: deadline yes (`met`), period yes (`kept`), lifespan no
   (`live`). `axis/x`: deadline yes, period yes, lifespan not asked
   (`no_lifespan`).
2. `cmd`: deadline no (`missed`), the owner present when T reported it;
   period no (`not_kept`): about 40 arrivals in a 5 s span, and
   (40 + 2) × 100 = 4,200 is below 5,000; lifespan no. The run's verdict
   is the finding.
3. `cmd`: deadline yes; period no (`not_kept`): about 20 arrivals, and
   (20 + 2) × 100 = 2,200 is below 5,000.
4. T hears the owner's tokens withdrawn, and its read after them is
   complete and shows none of them, long before either member's miss is
   due. `cmd`: deadline unobservable (`owner_not_present`): its miss came
   after the owner was gone; period unobservable (`owner_not_present`):
   not kept, and the owner not present throughout; lifespan no. `axis/x`:
   its member token went with the owner, so T reports it `ended` at that
   read: deadline not asked (`ended`); period yes (`kept`), over its span,
   which ended with it.
