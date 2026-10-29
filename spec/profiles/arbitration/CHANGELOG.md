# `arbitration.v1` changelog

Amendments to [`v1.md`](v1.md). Each entry records what changed, what
deliberately did not, and why.

## experimental — 2026-10-29: no adopter yet (#938)

The text's status moves from `draft` to `experimental`, the status of a
profile that no adopter's mapping and no shipped service uses
([`../README.md`](../README.md), "The process"). No contract of
ZenSight's, zenoh-modem's or tcgui's mapping lists `arbitration.v1` in
`uses`, and zenwatch does not follow it: only the walkthrough's
`thruster.v1` does. The profile leaves `experimental` when an adopter's
mapping or a shipped service uses it.

**Changed: the status alone.** The index lists `arbitration.v1` in its
second table, "Experimental: no adopter yet", and the header's status
reads `experimental`.

**Deliberately not changed.** No rule. The text stays 0.3, and its
fixtures, scenarios, runtime and tools are as they were.

## 0.3 — 2026-10-29: round 78's answers, and the tools (#909, BC)

The second implementation ran 0.2 against the reference runtime (zk2py
round 78: every section in four pairings, both implementations' actuators
beside both runners) and filed F-274 and F-276's arbitration part, both
accepted on #909 (issuecomment-37441). Neither changes a rule: each was the
reference's. The reference's tools (`zenctl arbitration`, the doctor's
`arbitration-declaration` and zengui's Workbench row, #909's BC) ran §10,
`zenctl arbitration` three times in a row. This text answers the findings
and says what the tools found. It is written against core 0.56, as 0.2
was, and `timing.v1` text 0.3, whose changes (F-275's epoch member, its
tools) touch nothing this text reads.

**Changed: the reference (round 78), no rule.**
- **A refused actuator put `angle`** (§2.2; F-274). The owner example put
  each raw state's value before its start, so §8.2's "State values" landed
  before the tokens, and the arbitration check ran only in `start()`: at
  20 of 20 refusals S heard `angle` = `ok`, which §2.2's "nothing of the
  instance is declared or put" forbids, `[Sc: §8]`. The runtime's
  `ServiceBuilder::arbitrate` now reads the declaration at once over the
  revision given and refuses an invalid one with the start's own error,
  `start()` reading it again; the example calls it as soon as the
  declaring contract is implemented, before any writer is declared or put.
  A valid one's value still lands before its tokens. The text needed
  nothing.
- **The commander's raw payload** (§11 step 1; F-276). `commander raw on`
  put the commander's own `<service> <n>`, so Expected 1's "S sees no
  `standby` payload" could not be read by the payload; it puts `standby
  <n>` now, and §11 names it as the reference's `X`.

**Changed: what the tools found (BC).**
- **What a tool's view reads that no descriptor carries** (§2.14). A
  descriptor lists a role's bindings and parameter bindings, never its
  binding's `clock_bound_ms` (core §3.3), so a tool's view states no ε,
  and §2.5's outcome 5 makes nothing expired: its view may follow a
  provider whose samples the consumer discards, which "another path, with
  another latency and another loss" already allows. And a descriptor names
  no revision of the required interface, so the view reads outcome 6 with
  the interface governed when every revision of it the tool holds lists
  `redundancy.v1`, learning the incumbents as a consumer does (§2.5). 0.2
  said neither. No verdict of a fixture moves: `conclusions.json` takes a
  view as given.
- **Listening counts from the stream's subscriptions** (§2.14). "Before the
  tool has listened for one deadline" did not say from when. A tool that
  counted from its first act, its watch's declaration, would count its
  round trip, which can take seconds after a reopen beside a large
  population (core §8.1, 0.44), as listening while no arrival could reach
  it, and answer `dead_man` of a role whose commanders publish. It counts
  from the declaration of its subscriptions to the role's stream, the
  last of them. `conclusions.json`'s description says so; no case moves.

**Stated: what the tools do** (the reference, not a rule).
- **`zenctl arbitration [SYSTEM/SERVICE] [--role R] [--for SECS]`** reads
  presence and every descriptor of the consumers in scope on its
  namespaced session, the revisions they name, and the bound providers'
  descriptors and revisions where that read did not reach them; each role
  a descriptor requires that carries a policy, or whose declaring
  contract is not in hand (`no_contract`, never "not this profile's"), or
  the one `--role` names. With `--role` and no descriptor listing it,
  each contract in hand declaring the role is read with no descriptor
  (F-272). Its view runs `zenkey_model::arbitration`'s `Arbiter`: one
  liveliness subscriber per entry on a watch session in no namespace on
  the wire keys, its round trip, one subscriber per entry on the
  namespaced session, a sample two entries cover handed over once by the
  first, every arrival's instant read in its callback, one bounded queue,
  a read again 0.1 s after a withdrawal leaving none known held, a move
  kept until the round trip asked 100 ms after it stands (core 0.56), a
  withdrawal heard with no link beginning one (F-268), and a one-shot timer
  at the active provider's due. A refusal of its queue, or a stall of its
  receive path beyond core §2.7's bound, within the last deadline is
  `lost`. Every answer is `judge_declaration`'s, `judge_selection`'s and
  `judge_binding`'s; the selection is labelled the tool's own view in
  every medium; exit 1 on any question's finding, 0 every question asked
  on its other pole (`unbound` informative), 2 otherwise.
- **The doctor's `arbitration-declaration`** asks the first and third
  questions of every running consumer's role carrying a policy, with no
  view: an invalid declaration an error, a wildcard or shadowed entry a
  warning.
- **zengui's Workbench** reads the selected address's arbitrated roles,
  300 ms after the selection settles, the latest's only, with a listen of
  two deadlines; the selection shown as the tool's own view.

**Deliberately not changed.**
- **No fixture for the view's inputs.** `conclusions.json` takes a view as
  given, and `arbitrations.json` each sample's `expired` and
  `not_incumbent` as given: which a tool can read is its descriptor's
  reach, which only a tool's run shows (§10).
- **No ε flag for a tool.** A tool told the consumer's ε would still read
  its own clock against the owners', not the consumer's; the view stays a
  view.
- **§2.14's `lost`** keeps core §2.7's stall bound: a stall shorter than it
  delays arrivals and decides them late, which the view's instants already
  carry.

**Evidence:** `conformance/declarations.json` (41 cases),
`conformance/rankings.json` (14), `conformance/arbitrations.json` (41),
`conformance/conclusions.json` (24), 120 in all, none changed. Eleven
scenario sections, ten the runtime's and one the tool's, which now runs.

## 0.2 — 2026-10-28: what the runtime showed, and round 77's answers (#909, BB)

The reference runtime (`zenkey::arbitration`, #909's BB) ran scenarios §1
to §9 and the new §11, three times each, and the second implementation
read 0.1 cold (zk2py round 77: 115 of 115 fixture cases, every live
section on zenoh-python 1.10.1, zk2py alone). Round 77 filed F-268 to
F-272, all accepted on #909 (issuecomment-37069); this text answers them,
and says what writing the runtime found. It is written against core 0.56,
which states F-268's order, and `timing.v1` text 0.2.

**Changed: rules (round 77).**
- **Where a move begins** (§2.8; F-268). A move begins at the watch
  session's transport event, or at a withdrawal the watch hears while the
  watch session holds no link, whichever comes first. zenoh 1.10.1 tells a
  close's withdrawals before the close's transport event: zk2py measured
  0.15–0.5 ms on zenoh-python, the reference 0.025–0.51 ms in Rust, at 40
  of 40 closes, with no link held when each was heard (core 0.56, §8.1 and
  Appendix B). An actuator that began the move at the event alone applied
  them as heard, and switched `withdrawn` while safety never stopped, once
  to the dead-man: the reference's did so too with the rule switched off,
  and passes §7 with it. Changes a verdict (§7 Expected 1). Scenarios §7
  gains the control that shows the order.
- **A tool with no consumer's descriptor** (§2.14, §5; F-272). After the
  premises, the selection and the binding are not asked, `no_descriptor`:
  never "no, `exact`" of a binding nobody read. `conclusions.json` takes
  `bindings` null for it, and gains five cases (§10 step 4's reading, a
  valid declaration, an unread one, and the two premises before it), 24
  in all; §10 Expected 4 says so. Changes a tool's test.

**Changed: scenarios (round 77).**
- **The commanders put 500 ms after their tokens** (conventions, §3; F-269).
  An actuator hears a commander's token on its watch session and its first
  sample on another, unordered, so a first sample put at the token could
  be counted `not_active` as a withdrawn provider's. §3 Expected 2 says
  the switch comes "at the new instance's first arrival after the
  actuator's watch heard its token". §2.7's rule stands.
- **A governed commander's standby**, §11 (§2.5; F-270): the commanders'
  contract uses `redundancy.v1`; a standby puts raw samples on its
  address's key; the actuator counts them `not_incumbent`, delivers none,
  and leaves the address at its incumbent's deadline. The reference keeps
  its contract as `zenkey/tests/contracts/steer-governed.v1.toml`
  (`steer.v1` minor 1).
- **The counts line gains `deleted`** (conventions; F-271), as §2.5 counts
  it: five counts per provider.

**Changed: what writing the runtime found.**
- **A governed interface's incumbents need a read at start** (§2.5,
  outcome 6; §2.8's "at start"). `redundancy.v1` §2.10 accepts nothing
  with no incumbent seen, and a watch without history is told no token
  declared before it (core §8.1, 0.41). A consumer that relied on its
  watch alone would count every sample of a commander up before it
  `not_incumbent`, and never follow it. A consumer of a governed
  interface SHOULD read each entry's presence once at its start, after its
  watch's round trip, and get each holder's descriptor whose `meta.zid` it
  does not hold. §11 starts its actuators after its commanders, so the
  read at start is what it shows.
- **One order, instants that never go back** (§2.9). Samples of several
  subscribers, the watch's tokens and the timer reach a consumer on
  several threads, each reading the clock before the events are ordered;
  an event decided after a later reading is decided at the last one's
  instant. Stated, with no rule of its own: `zenkey_model::arbitration`'s
  `Arbiter` now asserts it in debug builds.
- **A sample two entries cover is one sample** (§2.4's "an address matched
  twice … is one provider"). A consumer subscribing per entry hears a
  sample of `lab/safety` bound as `["lab/safety", "lab/*"]` on both
  subscriptions; the reference hands its arbiter each sample once, from
  the first entry's, and §9 checks no delivery is doubled. Stated by 0.1
  already; no rule changed.
- **The round trip that ends a move begun at a withdrawal** (§2.8) is asked
  once the watch session's transport is up again: none is asked through a
  session cut off (core §8.1, 0.42). The reference's barrier counts moves
  by transport events, so a round trip asked before the close's event
  would have read as standing; it waits for the event's.
- **And not at once at the open** (§2.8; core §8.1, 0.56). §7 failed two
  of three runs of the reference under nextest: at a heal its actuators
  applied the cut's withdrawals without the reopen's declarations, and
  switched `withdrawn`, to the dead-man once. zenoh 1.10.1 tells an open
  before its router declares the watch's interests again, so a round trip
  asked at the event can stand with nothing told (6 of 60 measured); and a
  telling heard before the reply can still wait in the consumer's queue
  when the reply wakes it. The reference asks the round trip 100 ms after
  the open, and takes what is queued first; §7 then passed six of six.

**Deliberately not changed.**
- **No fixture for where a move begins.** The fixture's events are the
  arbiter's (`moved`, `withdrawn`), and a withdrawal heard with no link is
  the runtime's to turn into `moved` then `withdrawn`; the scenario's
  control shows it.
- **§2.7's re-admission** stands: F-269 is a test's ordering, not a rule's.
- **`not_incumbent` stays outcome 6,** after `deleted` and `expired`.

**Evidence:** `conformance/declarations.json` (41 cases),
`conformance/rankings.json` (14), `conformance/arbitrations.json` (41),
`conformance/conclusions.json` (24), 120 in all. Eleven scenario sections,
ten the runtime's and one a tool's.

## 0.1 — 2026-10-27: the first text, draft (#909, BA)

`arbitration.v1` is the profile zk2 left the choice between bound
providers to: P3 moved arbitration into the consumer, "an actuator chooses
among its commanders, which is the `twist_mux` pattern" (architecture r4
§2), and the core says "when several providers are bound, choosing
between them is the consumer's (`arbitration.v1`)" (core §3.2). Nothing
chose. `thruster.v1`'s `[requires.cmd]` writes `arbitration.policy =
"priority"` with cardinality `many`, and the walkthrough's enrollment binds
it to `[safety, teleop, autopilot]` "in priority order"; the runtime's
consumer merged every commander's samples into one stream, and silence
stopped nothing. Core Appendix D's interim row, `arbitration | policy`,
was all the core said. This text is written against core 0.54, which
makes a binding ordered and publishes the vocabulary, and uses `timing.v1`
text 0.1 (#908) for the deadline.

**Decided on #909, before the text** (the maintainer's defaults, not
reopened here).
- **What the profile contributes:** point 2, `policy` on a requirement;
  point 4, through the consumer's contract's `uses`; no standard contract,
  since a consumer puts nothing of its own on the bus (`redundancy.v1`
  §3's precedent). Two implementations are compared on what a black box's
  output shows; a tool shows its own view, labelled as its own.
- **1.0 defines `priority` only.** `freshest` and `lease` are reserved,
  invalid at 1.0 (`policy_unsupported`); `acquire`/`release` are out.
- **An arbitrated role:** its contract uses `arbitration.v1` and its
  requirement carries the policy; cardinality `many`; exactly one stream
  resource, every template parameter bound; contract-declared roles only.
  A `many` role without a policy keeps the merged stream.
- **Priority is the binding's order,** made a core rule (0.54): a
  wildcard ranks its addresses at its position, among themselves by
  address; an address matched twice counts at its first entry; a tool
  warns of a wildcard.
- **The deadline** is the provider resource's `timing.deadline_ms`, from
  the revision the consumer was built against. None, or one below the
  period, is invalid (`no_deadline`), and the consumer refuses to start.
- **Eligible:** the last arrival within the deadline on the consumer's
  monotonic clock, and the interface token not withdrawn since. **Active:**
  the first eligible by priority, decided on every receive and within
  `timing.v1`'s detection bound. **No hysteresis.**
- **A withdrawal makes a provider ineligible, never absent;** per address,
  so a re-mint drops nothing; re-admission only by the token heard
  declared again or a read showing one held, never by a sample, because of
  core §8.2's stop order; a withdrawal heard after the watch session's
  transport moved not applied until its round trip stands; tokenless
  interfaces and faces by the deadline alone; withdrawals heard on a watch
  session, never polled.
- **Identity** from the key, base-relative, R6 first; a provider under
  `redundancy.v1` counting only the samples its §2.10 accepts.
- **Output:** only the active provider's samples delivered, the others
  counted; each switch reported with its cause, `start`, `sample`,
  `deadline` or `withdrawn`; the dead-man (no eligible provider) also the
  state at start; the safe value the application's.

**Stated.**
- **Who implements it** (§2.1); **the declaration**, with a closed list of
  eight reasons in their order, and **unread** for a reader holding no
  revision of the required interface (§2.2); **the deadline** (§2.3);
  **the ranking** (§2.4); **samples and who sent them**, seven outcomes in
  their order (§2.5); **eligibility** (§2.6); **withdrawals and
  re-admission** (§2.7); **a move of the watch session** (§2.8);
  **selection and delivery** (§2.9); **switches** (§2.10); **the
  dead-man** (§2.11); **tokenless interfaces and constrained faces**
  (§2.12); **cost** (§2.13); **what a tool concludes** (§2.14); **not at
  1.0** (§2.15).
- **No contract** (§3), and **the vocabulary** (§4), `policy`, the
  interim row's key unchanged.
- **What a tool may conclude** (§5): is the role's declaration valid;
  which provider does the tool's view select; does the binding leave a
  priority to an order nobody chose. The record against v1 (Appendix A),
  and where each item of architecture E14, §3.12's row and spike S11 went
  (Appendix B).
- **The core's part** is 0.54 ([`../../CHANGELOG.md`](../../CHANGELOG.md)):
  a binding ordered (R1), a descriptor keeping the configured order (R3,
  §3.3, `descriptors/ok-binding-order`), a reader that may act on a
  withdrawal heard live without concluding absence (§8.1), Appendix D's
  published row, and W105's fixture `contracts/w105-arbitration-deadline`.
- **Evidence:** `conformance/declarations.json` (41 cases),
  `conformance/rankings.json` (14), `conformance/arbitrations.json` (41),
  `conformance/conclusions.json` (19), 115 in all. Each expectation was
  computed by an independent transcription of this text before
  `zenkey_model::arbitration` ran it, and the reference agreed with every
  one at its first run. Ten scenario sections, nine the
  runtime's and one a tool's, over two test contracts, `steer.v1` (a
  stream with a 300 ms deadline) and `rudder.v1` (its arbitrated role).
- **The examples need no new revision.** `thruster.v1`'s `cmd` over
  `twist_cmd.v1`'s `cmd` reads arbitrated, `priority`, deadline 100 ms,
  as the reference checks.

**Decided here, where #909 left room.**
- **"By address in byte order"** is by system, then by service, each
  compared as bytes, never the text `<system>/<service>` whole: the two
  differ where one system is a prefix of another (`a/x` and `a-b/x`), and
  a pair's order is how an address compares everywhere else, as a
  descriptor's `profiles` sort by name, then major. `rankings.json` pins
  it.
- **Re-admission forgets the arrivals.** A withdrawal forgets the
  provider's last arrival, and only an arrival after its re-admission
  makes it eligible again. So a re-admission alone never switches, every
  switch has one of the four causes, and a clean stop's tail cannot make a
  provider eligible the instant its successor's token appears.
- **What is known held,** and so when a withdrawal leaves an address
  with nothing: declarations the watch heard and reads, a complete read
  replacing and one not complete adding. A withdrawal of a token never
  known held leaves none known held: the consumer fails toward a lower
  priority, and its read again re-admits a provider that still holds one.
  So a consumer needs no presence seed at start.
- **A read never makes a provider ineligible.** Only a withdrawal heard
  does, so a tokenless provider is never dropped by a read showing no
  token, and a poll would change no selection: "never polled" holds by
  construction.
- **A move keeps every presence event,** declarations and reads with the
  withdrawals, and applies them as one change when the round trip after
  the last move stands. Keeping the withdrawals alone would let the
  reopen's telling of a token come first and its close's withdrawal
  second; applying them one by one would forget a provider's arrivals
  between its close's withdrawal and its reopen's declaration.
- **Time first.** Each event first decides the active provider's miss at
  or before its instant, as the timer would have, cause `deadline`, then
  the event itself; one event can so make two switches. The cause of a
  switch then never depends on whether the timer or a sample came first.
- **The deadline is `timing.v1`'s, read and judged by its rules.** No
  deadline is the stream's `timing.v1` declaration invalid (any value not
  a whole number of milliseconds from 1 to 2^53−1, a lifespan's included),
  no deadline written, or one below the period (`timing.v1` §2.2, §2.5);
  the revision's `uses` is not read for it, as `timing.v1` reads values
  alone. A provider is missed once more than the deadline has passed since
  its last arrival, so it is still eligible at exactly `a` + deadline
  (`timing.v1` §2.5); the reference judges each provider's one key with
  `zenkey_model::timing`'s `Tracker`.
- **A delete sample is no arrival** (`timing.v1` §2.5), a seventh outcome,
  `deleted`, counted for its provider.
- **History.** A stream's history is replayed at a subscription's start,
  each sample an arrival (`timing.v1` §2.5), which would make a commander
  that stopped eligible for a deadline after a consumer's start: a
  contract SHOULD NOT declare `history` on an arbitrated role's stream.
- **`not_used`, `manifest_role`, `cardinality`, `resource`,
  `params_unbound`** name the defaults' conditions; `value` is
  `redundancy.v1`'s word for a value no text defines.
- **A shadowed entry** (an exact entry listed again, or one an earlier
  entry covers) is warned of with the wildcard, as its rank is not where
  it was written either.
- **A tool's view** is unobservable before it has listened one deadline,
  and whenever it knows it lost a sample within the last deadline; a role
  bound to nothing is no, `unbound`; a tool reads a manifest's role as not
  this profile's, since it sees no policy.
- **Which revision a tool reads the deadline from.** A descriptor names a
  role's interface and never the revision its consumer holds, so a tool
  reads every revision it holds, and answers `revisions_disagree` when they
  give two declarations.

**Deliberately not at 1.0** (#909's "Out at 1.0"): `freshest`, `lease`,
`acquire`/`release`, hysteresis, `clock.v1`'s simulated time, a deadline
tightened by the binding or the requirement, a selection contract, and
manifest-declared arbitrated roles (§2.15).

**Also deliberately not:**
- **No descriptor member** naming the revision of a required interface its
  consumer holds, which would let a tool read the consumer's own deadline:
  core §3.3 lists a role's interface only, and a tool reading every
  revision it holds answers the same where they agree.
- **No core rule for a consumer's switches:** they are the consumer's to
  tell (§2.10).
