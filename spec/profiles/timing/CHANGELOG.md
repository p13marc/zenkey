# `timing.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.3 — 2026-10-28: round 78's answers, and the tools (#908, MC)

The second implementation ran 0.2 against the reference runtime (zk2py
round 78) and filed F-273, F-275 and F-276 against `timing.v1`, all
accepted on #908. The reference's tools (`zenctl timing`, the doctor's
`timing-declaration`, `check conform`'s `timing-period` and
`timing-deadline`, and zengui's Inspector, #908's MC) ran §6, `zenctl
timing` three times in a row. This text answers the findings and says what
the tools found. It is written
against core 0.55, as 0.2 was.

**Changed: rules (round 78).**
- **An owner absent, beside an epoch template's known member** (§2.5;
  F-275). "If its owner is absent before the miss is due" stops only a
  wait that began at the owner's presence: an epoch template's known
  member is waited for from its member token, and waits on until a read
  shows that token gone. The reference's `Tracker` already read it so,
  and so did zk2py; 0.2's bullet, under "a member never heard", read as
  though it stopped both. Changes a verdict in that sequence alone.
  `deadlines.json` gains its case (start 0, its token seen 0, the owner
  present 0 and absent 50, a tick at 101: missed, due 100), and its
  description says what `owner absent` does with `epoch`.

**Changed: the scenarios (round 78).**
- **§4's owner has no HLC** (F-273): the runner's offset moves its stamps
  alone, as §2.6's "Without an HLC too" lets an owner stamp. An owner whose
  HLC is on follows its own first stamp ahead (core §4.1, 0.53), so its
  clock stands still and its stamps cannot run 400 ms ahead and then
  agree. No verdict moves: every sample of step 3 reads `clock_disagrees`
  either way. The reference's §4 test states the owner's HLC off rather
  than leaving it to a client's default.
- **The black box's commands** (F-276): the conventions name the owner's
  standard input for each step, `timing declare <axis>` among them, which
  declares a member's token and publishes nothing (§3 step 5), where
  `timing add` publishes at once; and `consume --timing` prints an expired
  arrival's `wall_ns`, the wall clock it read at the arrival, as its
  `--help` promised, where 0.2's printed null.

**Changed: what the tools found (MC).**
- **A loss the tool cannot attribute is every member's** (§2.7). A queue's
  refusal, or a stall of the tool's receive path, names no member, and
  0.2's "a delivery of the member" could be read as excusing a member none
  of whose deliveries is known lost. It is lost for every member the tool
  holds: any of them may have been the one. `conclusions.json`'s
  description says what sets `reading.lost`. No verdict of a fixture
  moves.
- **A tool's known members** (§2.7). A tool holds no binding, so a member
  is known to it only when its template has no parameter, and a selector
  naming a concrete key binds nothing. Scenarios §6's `cmd` is known and
  `axis/x` is not, as 0.2's expected observations already read them.
  `periods.json`'s description says so.
- **The lifespan across a face** (§2.8). 0.2 said it was judged "as
  anywhere", and a descriptor does not cross a face either: a reader there
  holds an owner's `meta.zid` only on its deployment's word, as it holds
  ε, and without one reads every sample `unattributed` (§2.6 step 6), the
  owner's own included. `lifespans.json` gains two cases: the same samples
  with no `meta.zid` (unattributed) and with the deployment's (live, then
  expired). The reference tool takes it as `--owner-zid`.
- **Scenarios §6's step 3 on the black box**: the owner example gains
  `timing every <member> <ms>`, a member published at another interval
  than its period, since none of its commands could publish `cmd` every
  250 ms; the conventions name it, and §6 names the reference tool and its
  runs.

**Stated: what the tools do** (the reference, not a rule).
- **`zenctl timing`** follows presence on a watch session in no namespace,
  reads presence and descriptors on its namespaced session, and starts its
  window once a presence read after the round trip and every descriptor it
  shows stand, trying once more when they did not (F-263); its opening is
  in the report. Each arrival is read in the subscriber's callback, and
  samples, token moves, owed reads and its timer are folded in one order
  whose instants never go back. A withdrawal leaving none owes a read
  100 ms later; a loss, a stall or a move of its watch session's transport
  holds every owner unconfirmed until presence is read again. Every
  verdict is `zenkey_model::timing`'s, rolled up by `roll_up`.
- **`check conform`'s `timing-period` and `timing-deadline`** run the same
  reader over the suite's window, one case per exposed timed stream, read
  over its members by finding; both skipped, it sends nothing.
- **The doctor's `timing-declaration`** is the linter's warnings of §2.2,
  §2.3 and §2.5 read from the contracts the descriptors name.
- **zengui's Inspector** reads a timed stream's declared values beside its
  type, and its deadline and period over the slot's recorded history, the
  window's own view: arrivals stamped at the data pump's callback, the
  owner's state as the presence link landed it, and the lifespan not
  judged for want of ε.

**Deliberately not changed.**
- **No verdict of a lifespan or a period,** and no tracker's but F-275's
  sequence.
- **The core version.** Nothing here needs core 0.56.
- **No zenwatch rule.** `silent-for` watches a selector's silence; a
  deadline rule waits.

## 0.2 — 2026-10-27: what the runtime showed, and round 76's answers (#908, MB)

The reference runtime (`zenkey::timing`, #908's MB) ran scenarios §1 to
§5, and the second implementation read 0.1 cold (zk2py round 76: 152 of
152 fixture cases, 26 of 26 live checks in each of three runs, zk2py
alone). Round 76 filed F-259 to F-267, all accepted on #908; this text
answers them, and says what writing the runtime found. It is written
against core 0.55, whose §4.1 F-265 extends.

**Changed: rules (round 76).**
- **No tracker under an invalid declaration, or a deadline below the
  period** (§2.2, §2.5; F-259). A consumer holds no tracker and reports
  nothing. `deadlines.json` gains five cases that give the resource in
  place of a deadline, its `tracker` in the expectation.
- **`beyond_bound` is a MUST, once** (§2.5; F-260): at a member's first
  arrival past the bound, and never again while it stays past. 0.1's MAY
  sat beside a fixture that expected the report at each such arrival; the
  fixture now expects it once (the bound's case loses its second report).
- **A known member of an epoch template** (§2.5; F-261) is waited for
  from the instant a member token of it is first seen, never from its
  owner's presence, and its tokens gone end it only after one was seen: a
  read that shows none before any was seen ends nothing. 0.1 left a known
  member silent forever when the reader's first read came before the
  owner declared it, and judged it when the read came after. A token is
  seen when heard declared or withdrawn, or shown by a read. Changes a
  verdict. `deadlines.json` takes `epoch` per case and a `member_seen`
  event; its member-token cases become epoch templates with their tokens
  seen, and five cases are added. Scenarios §3 gains step 5.
- **A tool's window starts once it follows presence and holds the
  descriptors** (§2.7; F-263), the first instant both stand; §0 names the
  state before any read, `unseen`. Changes a verdict: a window opened in
  0.1's order read its owner unseen for its first milliseconds. Scenarios
  §6's setup takes the order.
- **The roll-up is read by each question's finding** (§5; F-264): 0.1's
  sentence fitted the two questions whose finding is no, and hid an
  expired member beside a live one. Changes a verdict.
  `conclusions.json` gains twelve roll-up cases.

**Changed: wording and evidence (round 76).**
- **`deadlines.json`'s description** says "never heard, not missed and
  not ended" (F-262): a miss stands until an arrival, as §2.5 already
  said. A case pins it.
- **The scenarios' two instants** (F-266): C stops a deadline or less
  after the owner's last sample, and the owner publishes once C holds its
  descriptor where C judges a lifespan.
- **Each member held in the window** (§2.7, scenarios §6; F-267).
- **C's sessions have no HLC** (scenarios' conventions; F-265), with core
  0.55: a session whose HLC is enabled stamps an unstamped put it
  receives, so a reader with one reads §5 step 2's samples
  `foreign_stamp`, never `unstamped`. The pole does not move.

**Stated: what the runtime found.**
- **How a reader holds `meta.zid`** (§2.6): the descriptors' puts, a GET
  at the subscription's start, and a GET again at a stamp of an id not
  held; until then a sample reads `unattributed` or `foreign_stamp`,
  never discarded.
- **Following presence** (§2.5): on a watch session, read on another,
  seeded after the round trip; a watch session's transport move read
  again; a reader that follows none holds every owner unseen, and loses
  the never-heard wait and the member token's end, keeping every deadline
  from its arrivals.
- **A known member needs an exact provider address** (§0): a wildcard
  binding names no address.
- **A tokenless owner's token** (§0) is read only with its descriptor in
  hand; the reference follows interface tokens alone, so it reads such an
  owner unseen.
- **The owner's stamp without an HLC** (§2.6): `Session::new_timestamp()`
  carries the session's zid with or without one; the reference's writer
  of a lifespan resource stamps each put so, and refuses to start an owner
  whose configuration states another `meta.zid`.
- **ε's unit** (§2.6): milliseconds; the reference's binding states it as
  a whole number, `clock_bound_ms`.
- **`arbitration.v1`** is written (#909, its text 0.1), and §2.5 links
  it.

**Deliberately not changed.**
- **No verdict of a lifespan or a period.** `lifespans.json` and
  `periods.json` are as 0.1 wrote them.
- **The detection bound and its allowance.** The reference's misses came
  within 2 ms of their due on loopback, and zk2py's within 0.3 ms; the
  scenarios keep the 50 ms allowance for a loaded runner, and say so.
- **A tokenless owner followed by its instance token** stays the text's
  rule (§0); the reference does not yet read it, and says so above.
- **No tool here.** The doctor's `timing-declaration`, `check conform`'s
  cases, `zenctl timing` and zengui's Inspector are #908's MC, and run
  §6.

## 0.1 — 2026-10-27: the first text, draft (#908, MA)

`timing.v1` is the first profile of use case B, control: setpoints into
actuators, deadlines, the dead-man stop. Two contracts already wrote its
keys and nothing read them: `examples/zk2/walkthrough/twist_cmd.v1.toml`'s
`cmd` (`period_ms` 20, `deadline_ms` 100, `lifespan_ms` 100) and
`nav.v2.toml`'s `position` (`period_ms` 100, `lifespan_ms` 500), both
published in `examples/zk2/.history`, and `spec/conformance/contracts/ok-kinds.toml`.
Core Appendix D carried the keys as the interim `timing` row. This text is
written against core 0.53, which publishes that row unchanged and says that
a session's HLC moves to a later stamp it receives. `arbitration.v1` waits
on it: an arbitrated role's deadline is this profile's.

**Decided on #908, before the text** (the maintainer's defaults, not
reopened here).
- **What it contributes:** point 2, a published vocabulary, the interim
  row unchanged; point 4, through `uses`. No standard contract or schema.
- **Where the keys mean something:** a `stream` resource, `[defaults]` or
  `[defaults.stream]`. On a requirement, nothing at 1.0: the descriptor's
  requirement entry carries no annotations (core §3.3, R3), and the record
  puts a tightened deadline in the binding, which has no field for it. On
  state, event or operation, nothing, and a linter warns: `freshness.v1`
  covers state, and core `rate` events.
- **`period_ms`** is the owner's nominal interval per member, a SHOULD,
  and a runtime never re-publishes a sample on the owner's behalf. A
  deadline below the period is no deadline: unobservable, and a linter
  warns; the owner is not refused.
- **`deadline_ms`** is judged per provider address and key, on the
  reader's monotonic clock, from arrival to arrival; a miss is seen within
  deadline + max(deadline/50, 1 ms), the floor relaxing E14 below 50 ms;
  `missed` and `met` are edge-triggered; tracking is bounded by the
  declared cardinality, and ends for a member whose member token is
  withdrawn; a miss is never "down"; a member never heard is judged only
  when known another way, concrete with its owner's interface token held,
  from the later of the subscription's start and that token, and nothing
  is reported before a first arrival otherwise.
- **`lifespan_ms`** is kept, since both contracts that write it are in the
  append-only history, and enforced only on ε, the deployment's stated
  bound on clock disagreement, never the HLC's delta: expired when (the
  reader's wall now − the stamp) − ε > lifespan; a stamp more than ε ahead
  falsifies the word, so its clock is unobservable; with no bound stated
  nothing is discarded. The owner MUST stamp, the stamp must be the
  owner's (its id the descriptor's `meta.zid`), never a payload field, and
  an expired sample is not an arrival.
- **`freshness.v1` §2.2's sentence** that stream samples carry no owner's
  stamp is corrected, in its text 0.4.

**Stated.**
- **Who implements it** (§2.1); **the values**, whole milliseconds from 1
  to 2^53−1, with three reasons a declaration is invalid (§2.2); **where
  they mean something** (§2.3); **the period** (§2.4); **the deadline**
  (§2.5); **the lifespan** (§2.6); **a tool's window** (§2.7);
  **constrained faces** (§2.8); **not at 1.0** (§2.9).
- **No contract** (§3), and **the vocabulary** (§4), the interim row's
  three keys unchanged.
- **What a tool may conclude** (§5), per member over a window: the
  deadline met, the period kept, a sample expired; and the changes
  against v1 (Appendix A), which had no timing profile.
- **The core's part** is 0.53 ([`../../CHANGELOG.md`](../../CHANGELOG.md)):
  Appendix D's published row, W105's fixture
  `contracts/w105-timing-max-rate`, and §4.1's HLC moved by a stamp it
  receives.
- **Evidence:** `conformance/declarations.json` (42 cases),
  `conformance/deadlines.json` (33), `conformance/lifespans.json` (30),
  `conformance/periods.json` (20) and `conformance/conclusions.json` (27),
  152 in all. Each expectation was written from this text by hand before
  `zenkey_model::timing` ran it, and every one held on its first run. Six
  scenario sections, five the runtime's and one a tool's, over one test
  contract, `pace.v1`, which loads with no finding.

**Decided here, where the defaults left room.**
- **A declaration is invalid as a whole.** One wrong value makes every
  question of the resource unobservable, as `telemetry.v1` reads a member
  under an invalid declaration: the contract was not written as the
  profile reads it, and judging the other two keys would read a contract
  half right.
- **`0` is no value.** There is no "never" for a period, a deadline or a
  lifespan: a resource without one omits the key. `freshness.ttl_s`'s 0
  means never stale; a deadline of 0 would be missed at once.
- **Kinds before keys.** A state, event or operation resource is not this
  profile's whatever it writes, and §5 says so first; then a stream with
  no key for a question is not asked it.
- **"Its owner present"** reads the tokenless set: for an interface its
  descriptor marks `"token": false`, the instance token and a descriptor
  listing it, since such an owner holds no interface token (core §8.1).
  Absence only by a complete read (core 0.37).
- **Withdrawn means gone by a complete read.** A member token's withdrawal
  heard live owes a read, and the member ends only when the read shows no
  token of it (core §8.1, 0.37): a router's restart withdraws every token
  at once, and ending members on it would hide the misses it causes.
- **`ended`, a third event.** A reader reports `ended` when it stops
  judging a member it had heard or reported missed, so that a consumer
  holding a miss is told it will hear no `met`. `met` follows only
  `missed`, and a first arrival reports nothing.
- **Forgetting.** A member that is not known is forgotten an hour after
  its last arrival, once missed, which core §2.7's liveness span allows:
  otherwise an owner whose members churn within its bound would fill a
  long-running reader's bound, and new members would go unjudged. A known
  member is never forgotten.
- **Judged at every event.** A reader judges every member it holds at
  each arrival and at each instant it sets, so an arrival after an unseen
  silence reports the miss, then `met`, and the outcome depends on the
  instants alone, never on where the timer fell.
- **A never-heard member's wait restarts.** When its owner goes absent
  before the miss is due, the wait stops, and starts again when the owner
  is present again; a miss already reported stands until an arrival.
- **The lifespan is judged at arrival,** by the reader's wall clock then,
  never its HLC: the HLC follows the stamps it receives (core 0.53). A
  held sample is not judged again: that is the deadline's.
- **The order of a lifespan's checks,** and **falsified for the reading:**
  a stamp of the owner's clock more than ε ahead makes every later sample
  of that clock unobservable for the subscription's lifetime or the
  tool's window, as `freshness.v1` §2.6 reads a measured clock. A router's
  stamp ahead falsifies nothing of the owner's.
- **The period over a window** is a rate: kept when (arrivals + 2) ×
  period is at least the span, the span starting at the window for a known
  member, else at the first arrival, a span under three periods too short.
  Two samples of slack absorb the phase and an end's jitter; a pause is
  the deadline's finding.
- **A tool's window** reads a miss as the finding only while the owner was
  present and nothing was lost, so a miss after a clean stop, which
  presence shows before the deadline, is no finding, and one while a hung
  owner still held its token is.
- **Across a constrained face,** no member is waited for, a member ends
  only by being forgotten, and a tool answers the deadline's and the
  period's questions unobservable: presence does not cross, and a face
  that downsamples sets the rate (`link.v1` §2.8).
- **The HLC fact belongs to the core.** That a session moves its HLC to a
  later stamp it receives is a zenoh fact, which core §4.1 now states
  (0.53), as it states that routers stamp; the profile cites it for what
  ε bounds.
- **The owner's stamps stay the profile's,** as `redundancy.v1` §2.10
  kept its own: core §4.3 still asks the HLC only of a session that serves
  state, and the core depends on no profile.

**Deliberately not stated.**
- **A sequence check** (architecture §3.12's "where staleness in transit
  matters"): `twist_cmd.v1` has no sequence field, and the core numbers a
  stream's samples only through advanced publication's miss detection.
- **`clock.v1`'s simulated time.** Every clock here is a host's.
- **A tightened deadline**, by the binding or the requirement: no wire
  field carries one at this version.
- **The dead-man value and arbitration** are `arbitration.v1`'s; §2.5
  says only where its deadline comes from.
- **A tool's own finding of an unstamped owner.** The owner's MUST is
  observable, as core §4.2's S1 check observes a state's, and the lifespan
  question reads it unobservable; a check of its own is the tools'
  chunk's.
- **No runtime and no tool here.** `zenkey`'s deadline events, its
  lifespan on a stated bound, an owner's stamps and the owner example's
  commander are #908's MB, and run §1 to §5; the doctor's
  `timing-declaration`, `check conform`'s `timing-period` and
  `timing-deadline`, `zenctl timing` and zengui's Inspector are its MC,
  and run §6.
