# `redundancy.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.1 — 2026-10-24: the first text, draft (#841, RA)

`redundancy.v1` is the profile the core has left exclusivity to since its
0.1: O1's at most once holds only while one instance serves (core §5.1,
spike S6), and "election and fencing belong to `redundancy.v1`" (core §6).
Two places waited on it: `hostid.v1` §2.12 reads a standby on a minted
system as a finding "until `redundancy.v1` says how a tool recognises a
standby", and the reference doctor's `token-missing` read a standby's
listing as an exposed interface with no token. ZenSight runs a claim
protocol for its two singletons (`zensight-common/src/service_guard.rs`,
fixed in ZenSight #1104 and #1105), and `zs.catalog.v1` and
`zs.desired.v1` list the profile in `uses`. Core Appendix D's interim row,
`redundancy | election`, was all the core said; no contract wrote the
key. This text is written against core 0.39, which marks a standby in the
descriptor and publishes the vocabulary.

**Decided on #841, before the text** (the maintainer's defaults, not
reopened here).
- **What the profile contributes:** point 4, through `uses`: an interface
  whose contract uses `redundancy.v1` is **governed**; point 2, a
  published vocabulary with one key, `election`, on the interface's
  `[defaults]`: `claim`, the default, this profile's protocol, and
  `external`, an outside lock electing, the profile saying only how a
  standby shows and what a tool concludes. No standard contract or schema.
- **The claim set** is the address's live instance tokens whose
  descriptors mark the governed interface as standing by or serving. A
  pure consumer on the address is not a claim.
- **The incumbent** is the instance holding the governed interface's
  token.
- **Election (`claim`):** claim before reading, the instance's token and
  its descriptor, the interface marked standing by, before it reads
  anything; a live incumbent wins, and the instance stands by; with no
  incumbent, a complete read of the claim set (core 0.37), one not
  complete meaning stand by; the lowest instance id wins, the winner
  exposing the interface's resources, putting its descriptor as serving
  and declaring the interface token last (§8.2's order), a loser standing
  by.
- **A standby** keeps its instance token and its descriptor; exposes none
  of the interface's resources and declares none of its queryables (O3),
  so a call finds no standby; follows the incumbent's interface token by a
  liveliness subscriber on a session that holds no presence read (#830),
  not by polling.
- **A vacancy** is the incumbent's interface token withdrawn, then absent
  in a complete read (core 0.37). It makes every standby campaign again. A
  crashed incumbent is seen at zenoh's lease, 10 s by default.
- **Healing a split:** two incumbents seen in two reads a grace apart
  (§6's rule) make the one with the higher instance id step down: it
  withdraws its token, unexposes, and stands by. The rule is
  deterministic, so a healed partition ends with one incumbent without a
  vote.
- **Consumers** SHOULD accept state and stream samples only from the
  incumbent, the holder of the interface token, and during a split follow
  the lowest instance id, the one the heal keeps: the profile's fencing,
  by presence, not an epoch in the payload, which stops S7's flapping
  between two writers. Callers keep core O1, so a split still runs a call
  on each side, and that is stated, not hidden.
- **Actuators** keep an out-of-bus lock, described as `election =
  "external"` and not replaced.
- **The core first:** the descriptor's per-interface standby mark (§3.3);
  a standby exposing none of an interface's resources, required ones
  included (§8.2 step 2); a standby holding no interface token (§8.1); the
  standby defined, and the split-brain check reading the mark (§6); the
  tokenless set never naming a governed interface, as for `archive.v1`.
- **Not at 1.0:** quorum or consensus; fencing epochs in payloads; leases
  other than zenoh's liveliness; exclusivity across routers stronger than
  "eventual single writer" (ZenSight's own claim).

**Stated.**
- **Who implements it** (§2.1); **the declaration**, governed and the
  election, with a closed list of reasons a declaration is invalid
  (§2.2); **the claim set and the incumbent**, and how ids compare (§2.3);
  **the standby** (§2.4); **the election under `claim`** (§2.5); **taking
  over** (§2.6); **following the address** (§2.7); **a vacancy** (§2.8);
  **healing a split** (§2.9); **consumers, callers, and the stamps they
  read** (§2.10); **`election = "external"`** (§2.11); **population**
  (§2.12); **constrained faces** (§2.13); **the tokenless set** (§2.14);
  **what a tool concludes** (§2.15); **not at 1.0** (§2.16).
- **No contract** (§3), and **the vocabulary** (§4), `election`, the
  interim row's key unchanged.
- **What a tool may conclude** (§5): is the address served by exactly one
  incumbent; is there a vacancy; does any instance stand by, and how
  many; is this instance a standby, or a twin. The changes against v1's
  ownership protocol (Appendix A), and the adopters' mappings
  (Appendix B).
- **The core's part** is 0.39 ([`../../CHANGELOG.md`](../../CHANGELOG.md)):
  the mark, D012, a standby's tokens and exposure, serving later and
  stopping in §8.2's order, the tokenless refusal, the split-brain check
  reading the mark, Appendix D's published row, and W105's fixture
  `contracts/w105-redundancy-epoch`.
- **Evidence:** `conformance/declarations.json` (26 cases),
  `conformance/elections.json` (29), `conformance/heals.json` (16),
  `conformance/descriptors.json` (20), `conformance/lifecycle.json` (30),
  `conformance/conclusions.json` (39), 160 in all. Each expectation was
  computed by an independent transcription of this text before
  `zenkey_model::redundancy` ran it. Twelve scenario sections, eleven the
  runtime's and one a tool's, over two test contracts, `ledger.v1`
  (`claim`) and `valve.v1` (`external`), kept under
  `zenkey/tests/contracts/`. `zenkey_model::redundancy` runs every
  fixture.
- **`hostid.v1` text 0.6** points its §2.12 here: a recognised standby,
  which §2.15's fourth question answers `standby` of every interface it
  lists, all governed, is not counted (its CHANGELOG).
- **The examples' contracts need no new revision.** `zs.catalog.v1` and
  `zs.desired.v1` list `redundancy.v1` in `uses` and write no election,
  so each reads governed, `claim`, as the reference checks; their header
  comments, outside the fingerprint, were rewritten to this text.
  `modem.v3`, `tc.netif.v1` and `tc.netem.v1` do not list the profile,
  and read not this profile's (Appendix B).

**Decided here, where the record was silent.**
- **The election is read from the resources.** A contract writes
  `redundancy.election` on `[defaults]`, and the canonical form carries
  merged annotations per resource (core §9.5), so a reader of a bundle
  finds it on each resource, never on the interface. Every resource
  resolves to one election, or the declaration is invalid, a resource
  carrying none read as `claim`, and an interface with no resource is
  `claim`; a `[defaults.<kind>]` election
  that reaches some resources only is `disagrees`. The key on a
  requirement means nothing, and another `redundancy.<key>` is W105's
  and makes nothing invalid.
- **An invalid declaration** has four reasons, checked alone: `not_used`
  (core E020's, for a reader of annotations from elsewhere), `value`,
  `disagrees`, `tokenless`. A runtime refuses to start such an instance,
  as core §8.2 step 2 refuses, and a tool reports it.
- **Governance is a revision's.** Each instance is governed by the
  revision its descriptor names; adding or dropping `uses`, or changing
  the election, is a review (core §9.8). A tool reads an address whose
  candidates' revisions disagree as unobservable, `revisions_disagree`,
  never as one or the other.
- **A read is one liveliness read of the address's `@zk/**`,** instance
  and interface tokens together, with the descriptors of the instances
  whose ids are lower than the reader's own: a higher id cannot beat it,
  so its descriptor is not needed.
- **Its own claim must be in the read** (accepted by the maintainer on
  2026-10-24). ZenSight's check ("an answer that
  does not contain our own claim is an answer we did not really get")
  carried over: a refused read is complete and empty (core §8.1, 0.8), so
  completeness alone cannot tell it from a lone candidate's.
- **Which reads are complete, by core 0.37 and 0.38.** A read cut off
  while it ran (its session's links gone when it ended) is possibly
  incomplete, and a read not made is read as one (0.38's premise rule), so
  either makes a campaigning instance stand by and retry; the tokens a read
  not complete returned were seen, so an incumbent it shows still wins,
  and the holders it shows still count toward a split. A tool reads a read
  not made as one not complete, and reports its reason beside the answer.
- **"Claim before reading" across sessions.** The read goes on a session
  that has never held a liveliness subscriber (core 0.37, 0.38), which
  may not be the one holding the claim; then the instance makes one round trip on the claim's
  session first, since zenoh orders no two sessions' messages. The claim
  narrows the simultaneous-start race; the heal ends what it leaves.
- **The retry interval.** A standby that saw no incumbent (the read not
  complete, its own claim missing, a lower descriptor unread, or a lower
  candidate) campaigns again after an interval of its own, SHOULD be a few
  seconds, as ZenSight's 2 s poll was. One that saw an incumbent waits on
  its watch alone. A lower candidate is retried too, since it may never
  take over.
- **The watch covers the address's whole `@zk/**`,** held by every
  candidate from before its first read: the standby to hear its incumbent
  go, the incumbent to hear a second one come, which is the heal's
  trigger.
- **How the heal is seen.** An incumbent that hears another holder appear
  reads at once and again a grace later; its own token counted whether a
  read shows it or not; a read's completeness ignored, since a read cannot
  invent a holder; the second read's lowest holder deciding, so three
  incumbents leave one. A crashed lower holder may be stepped down to; the
  vacancy that follows is campaigned for.
- **The order of a takeover and a step-down** is core §8.2's and its
  reverse (core 0.39): resources, the descriptor put without the mark,
  then the token; then the tokens, the resources, the descriptor put
  marked. A takeover's validation (step 2) happens at the start, as a
  standby, so a takeover cannot fail it. A standby, and an instance
  stepping down, put and delete none of the interface's keys: a delete
  would read as the incumbent's.
- **A standby still holds the bundle.** Its contract queryable (core
  §8.4) is no resource of the interface, and a tool retrieves the contract
  from any holder.
- **A standby's `unavailable` and `cardinality`** state what it will
  expose once it serves (core 0.39): one member flips at a takeover, and a
  tool sees a standby's readiness before it.
- **Stamps the consumer reads.** A candidate MUST state `meta.zid` (a
  SHOULD in core §3.3) and enable its HLC whatever kinds it serves, so
  every sample names its writer. A following consumer discards a sample it
  cannot attribute, accepts nothing new while it sees no incumbent, and
  pays for a split's seed: `Latest` keeps the later stamp per key,
  whichever incumbent wrote it, so the key reads nothing until its
  incumbent puts again, which the text states.
- **`external`** (accepted by the maintainer on 2026-10-24): an instance
  without the lock refuses to start or stands by; with it, it takes over once a read shows no other holder, complete
  and holding its own claim, without the lowest-id step, since the lock
  decided, and waits for the vacancy rather than take over beside a
  crashed holder's token, which outlives the lock by up to a lease; it
  never steps down by ids, and steps down when it loses the lock; its
  anti-twin probe is an optional early refusal. A split under `external`
  is a tool's finding the profile does not heal.
- **Every interface at a governed address SHOULD be governed** (confirmed
  by the maintainer on 2026-10-24, for #841's record). One that is not is
  exposed by every instance at once, so an address with governed
  interfaces implements governed interfaces only, and a process's own
  `health.v1` and `alarms.v1`, whose contracts no adopter can make use the
  profile, go on an address of its own, as ZenSight's correlator does.
- **Several governed interfaces at one address** are elected apart.
- **Constrained faces:** presence does not cross, so a deployment SHOULD
  keep an address's candidates on one side; a tool across a face answers
  `across_face`.
- **A tool's questions** read two reads a grace apart (core §6, and §8.1's
  two-read rule for every presence shape), answer in five states, `not
  this profile's` among them, and count by token: a holder marked standby
  is an incumbent (`marked_holding` when it lasts), a candidate listing
  the interface unmarked with no token is `unmarked`, a holder not listing
  it `unlisted`. "Exactly one" and "standby" need a complete read; a split
  does not. The standby count is exact only from a complete read with
  every descriptor read, and a lower bound otherwise. `hostid.v1` §2.12's
  second process is the instance the fourth question answers `standby`.
- **The mark's reading** (`descriptors.json`): `malformed` first and
  alone; then `standby_tokenless` (core D012), `governed_tokenless`, and
  `not_governed` for a mark on an interface the profile does not govern.
- **A sixth fixture,** `lifecycle.json`, pins the state machine the
  runtime drives: events, steps, and the role after them, so that a second
  runtime reaches the same steps from the same events.
- **Uses no other profile.** Nothing here needs `freshness.v1`'s horizon:
  liveness is presence.

**Deliberately not stated.**
- **No quorum, epoch, claim id or lease of the profile's own,** and no
  exclusivity stronger than eventual single writer (§2.16): the
  maintainer's "not at 1.0".
- **No state handover, preferred incumbent, priority or switchover
  operation.** A clean stop of the incumbent is the switchover.
- **No reporting by a standby at its address.** Its `health.v1` would be
  a second writer of the incumbent's status; a process's own reporting
  goes on its own address.
- **No tool here.** `hostid.v1`'s text moves now (its 0.6); the doctor's
  `hostid-duplicate` reading it is #841's RC.
- **No timing numbers as rules.** The retry interval, the grace and the
  scenarios' 2 s are settings; the scenarios fix them for their runs.
- **No runtime and no tool here.** `zenkey`'s `ServiceBuilder::redundancy()`
  and the owner example's `--standby` are #841's RB, and run §1 to §11;
  zenctl's questions, the doctor reading the mark and zengui's standby are
  its RC, and run §12.
