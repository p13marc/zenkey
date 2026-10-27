# `arbitration.v1` changelog

Amendments to [`v1.md`](v1.md). Each entry records what changed, what
deliberately did not, and why.

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
  **the ranking** (§2.4); **samples and who sent them**, six outcomes in
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
- **Evidence:** `conformance/declarations.json` (36 cases),
  `conformance/rankings.json` (14), `conformance/arbitrations.json` (39),
  `conformance/conclusions.json` (19), 108 in all. Each expectation was
  computed by an independent transcription of this text before
  `zenkey_model::arbitration` ran it. Ten scenario sections, nine the
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
- **The miss's instant.** A provider whose last arrival was at `a` has
  missed at `a` + deadline, as `timing.v1`'s "from the deadline after its
  last arrival" reads; phase 2 aligns it with `timing.v1`'s text.
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
