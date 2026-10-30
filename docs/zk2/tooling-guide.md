# zk2 tooling guide

**Status: guidance.** The rules a tool follows are normative in
[`spec/observer.md`](../../spec/observer.md) since #925: the judgement
shape, silence never being a verdict, the reader's discipline (reading
presence, state and the admin space, windows and the population budget),
and the observer obligations that cite evidence. They came from this
guide and from the core. [`spec/core.md`](../../spec/core.md) binds what
participants put on the wire. Where this guide restates a rule,
`observer.md` or the core wins.

What stays here is guidance: how the reference tools (`zenkey-fleet`,
zenctl, zengui, zenwatch) spell those rules, the observer obligations
that no fixture or scenario checks yet, recording and replay, what did
not carry over from v1's observer conformance (RFC 13, frozen on the `v1`
branch at v1.50), how the reference tools test themselves (§7), and the
practices they keep from v1's operations cookbook, which zk2 states no rule
for (§8). An
obligation leaves this guide for `observer.md` once a fixture or a
scenario can check it. This is #612's tooling guide.

Contents:
1. [The judgement shape](#1-the-judgement-shape)
2. [Silence is never a verdict](#2-silence-is-never-a-verdict)
3. [The observer obligations](#3-the-observer-obligations)
4. [Tools do not write what they do not own](#4-tools-do-not-write-what-they-do-not-own)
5. [Recording and replay](#5-recording-and-replay)
6. [What does not carry over from v1](#6-what-does-not-carry-over-from-v1)
7. [Testing a tool against this guide](#7-testing-a-tool-against-this-guide)
8. [What the reference keeps from v1's operations cookbook](#8-what-the-reference-keeps-from-v1s-operations-cookbook)

---

## 1. The judgement shape

The shape itself is normative: [`observer.md`](../../spec/observer.md)
§1 (Established yes and no, NotAsked, Unobservable with its reason, the
two unestablished kinds kept apart, an empty scope unobservable). This
section keeps what no fixture checks: the polarity, the exit codes, and
the reference's mappings.

**Polarity.** Every verdict vocabulary documents its mapping onto the
shape, and names which Established pole is the *finding*. A vocabulary
named for the sought answer (`compatible`, `Met`) has its finding on *no*.
One named for a condition firing (`Explained`, `split-brain`) has its
finding on *yes*. A renderer or an exit map that assumes the wrong polarity
inverts every verdict, and still looks honest. A profile's §5 states its
polarity, as its template asks
([`spec/profiles/README.md`](../../spec/profiles/README.md)).

**Exit codes.** A judging tool projects the shape onto its exit code, after
the polarity mapping:

| Exit | Meaning |
|---|---|
| **0** | Established, and clean: no action is needed. |
| **1** | Established, and a finding. |
| **2** | Unestablished, either kind. The report says which. |

- **An empty scope** is Unobservable (observer §1), so the judgement exits
  2, never 0.
- **zenctl's spelling.** Its exit contract (`zenctl/src/exit.rs`) is this
  projection, and every verdict exits through
  `zenkey_fleet::judgement_exit_code`. Usage errors and refused input also
  exit 2: asking nothing is not a verdict.

Worked mappings in the zk2 tools (sections are the core's, or
`observer.md`'s where marked):

| Vocabulary (question) | yes | no | Unestablished |
|---|---|---|---|
| `compat`: is the new revision compatible with the old one? (§9.8) | `compatible` | `review` or `breaking`: the finding | a side that does not load or retrieve (Unobservable) |
| A presence read: does a token match? (observer §3) | the tokens | complete and empty, worded "no token visible to this reader" (below) | a read that ended at its timeout: possibly incomplete (Unobservable) |
| A state GET: what is the current value? (S4) | the owner's value, or its deletion | none: a GET answers or is silent | silence (Unobservable). An archive's answer is a *different* question (§2). |
| Split-brain: do two instances serve one exclusive operation at once? (observer §6.1) | `split-brain`: the finding | none seen | a holder whose descriptor or bundle could not be read (`undecided`) |
| A `doctor` check: does its condition fire? Every id names the condition, `split-brain` to `router-version-skew` (#612, FJ6), then `health-failed`, `health-degraded`, `health-stale`, `health-inconsistent` and `hostid-duplicate` (#721), then `population-over-bound` (#735, `--deep`) | a finding, with its severity, subject and evidence | clean, with the evidence that makes it clean as the reason | a subject it could not decide (Unobservable, each listed `unjudged` with why): for the `health-*` checks a status whose age no trusted clock reads (the doctor measures none: `--clocks-synced`), for `hostid-duplicate` an instance whose minting is unobservable, unless the counted ones establish it, for `population-over-bound` a population within its bound by a GET that ended at its timeout; every check that reads presence over an empty scope (Unobservable); a check the run did not ask (NotAsked) |
| A `why` rung: is the silence caused here? `namespace` to `last-known` (#702) | a cause: the finding, and the ladder stops | healthy, with its evidence | a rung whose read could not be had, the owner's silence included (Unobservable); every rung past the stop, and an operation's answer, never called (NotAsked) |
| A `check conform` case: does the service break this rule here? `contract-served` to `state-get` (#703), `freshness` (#720), `health` and `health-aggregation` (#721), `budget`, `rate` and `budget-window` (#735) | a violation, with what broke it; for `health`, unhealthy or stale, and for `health-aggregation`, a check worse than a fresh status in both readings; for `budget` and `budget-window`, more live members than the bound in any reading; for `rate`, n + 1 occurrences of one member less than the period apart | passed, with the evidence; for `budget`, only by the owner's GET of a state that ran to its final reply; for `budget-window`, only over a window of a whole liveness span, lossless, the owner present throughout; "lossless" established, never assumed (observer §5.2): the run lost nothing and the resource is reliable with `block` | a resource silent in the window, a stamp no `meta.zid` attributes, a present service's silent call without `--calls-granted` (observer §7.2), a status whose age no trusted clock reads, a population within its bound from an incomplete GET or an incomplete window, a `rate` window shorter than one period (Unobservable); an operation not idempotent without `--i-know`, a raw type, a service whose descriptor does not list `health.v1`, the no-ceiling bound, a case `--skip`ped (NotAsked) |
| `zenctl health`: is this service healthy? (`health.v1` §2.11, §5; #721) | `healthy`, at its level | `unhealthy` at its level (its status, a current check worse than it, or a check under an unknown status), and `stale`, its own verdict and never a level: the findings | `unobservable` (a reply's age with no trusted clock, a silent GET, an unknown level, a face closed to the status, nothing crossed yet); `not_asked` (absent: presence's word, never FAILED; not listing `health.v1`; only an archive's status, last-known). The run: 1 on a finding, 2 when a service is unobservable or none is asked |
| `zenctl health`: does its status agree with its checks? (§2.2) | `yes`: a fresh status, no current check worse | `no`, the finding about the owner, only when both readings a grace apart show it | one reading's `no`, a status not fresh, an unknown level, checks not read (Unobservable); as the first question's (NotAsked) |
| `zenctl health`: is its clock ahead? (§2.5) | `yes`, the finding: a `clock_ahead` fault no confirmation of the status followed | a confirmation heard after the last fault, or with none heard — never from the faults' silence | nothing heard in the window (Unobservable); no window (NotAsked) |

## 2. Silence is never a verdict

Normative: [`observer.md`](../../spec/observer.md) §2, and core O5. What
stays here is how the reference tools word it:
- **Presence.** A refused read is complete and empty (observer §3.4), so
  a tool words absence as what its reader could see: "no token visible to
  this reader". `zenctl acl gen` grants every Consume and Call those reads
  (core §11.1).
- **State.** A tool that reads an archive after the owner's silence
  presents the answer as **last-known**, never current (core S6), and
  says which archive answered.
- **Contracts.** A tool never decodes with an unverified bundle (core
  §8.4).

## 3. The observer obligations

An observer is any tool that reads the bus without owning keys on it. Its
verdicts reach a person through three media, a table cell, a report field
and a note ([`observer.md`](../../spec/observer.md) §1). The reference
tools render one report struct through all three. That is what keeps the
three in agreement.

The obligations that a fixture or a scenario checks are normative in
`observer.md`: **O1** (a key that is not zk2 is a fact) is its §8.1,
**O4** (not asked is not answered no) is in its §1, **O5** (a wildcard
scope is not total coverage, and everything is three selectors) is its
§8.2, and **O7** (a timestamp names who stamped it) is its §8.3. What
stays below is the rest, guidance for want of evidence, and how the
reference spells the promoted ones.

**O1, the report's shape.** In a report, a key that is not zk2 is a
partial report whose verdict says how far description got. The fields
below the failure point are absent, never defaulted. The fleet's spelling
is `Unresolved::NotZk2`.

**O2. Classify by degrading, in this order.** Each failed rung weakens the
claim. None discards the key.

| Rung | Question | On failure |
|---|---|---|
| 1 | Is it in this namespace? A resolved verb's session strips the namespace (§1.6); a raw verb sees full keys. | Not in this namespace. Terminal. |
| 2 | Does it fit one of the five key forms (§1.1)? | Not a zk2 key, with the parser's reason. |
| 3 | Does presence show a provider of that address and interface? | No provider. |
| 4 | Does a descriptor name the revision (the full fingerprint, §8.4)? | No revision, or ambiguous: two revisions, and a data key names no instance. |
| 5 | Is the bundle held, retrieved and verified? | Not held (NotAsked), unavailable (the refusals carried), or unreadable. |
| 6 | Does a resource template match, and does it have this member (§2.2)? | No resource, or no member. |
| 7 | Do the bytes decode as the declared type (§7.2)? | Undecodable: the declared type, the size, the reason. A raw type is shown opaque, as its media type and size. |

Every rung outcome has its own spelling in every medium, so a script
branches on the rung, never on prose. The fleet's `Unresolved` and
`Rendered` enums are that spelling. A type is named one way, decoded or
not (observer §7.3, 0.8). zenctl's raw observers (`echo`, `rate`, `field`,
`timeline`, `snapshot`, `watchdog`; #612, FJ8b) walk this ladder through
one session-free lens, so a `.zrec` read later resolves exactly as the
live window did; each key's outcome rides as its `identity` (`KeyGroup`
plus the rung it stopped at).

**O3. Do not guess another deployment's namespace.** A data key outside
the configured namespace is "not in this namespace". A tool does not
attribute it to a namespace by scanning for a `zk2` chunk: a namespace may
contain one. Naming namespaces is `namespace list`'s job. It reads instance
tokens, whose form has a fixed arity (`<ns>/zk2/<system>/<service>/@zk/instance/<id>`),
and so attributes from the right.

**O4, its spelling in each medium.**
- **Table cell:** a reserved unknown mark (`—`), distinct from the empty
  answer and every negative word.
- **Report field:** a not-asked field is *absent*, never a defaulted
  `false`, `0` or `null` that collides with an answered state.
- **Note:** names what was not asked and how to ask it ("contract not
  retrieved; pass `--contracts`, or let the tool fetch it").

**O5, its spelling.** The exact selectors ride the report (a `scopes`
field), and a recording's header carries them too (§5). zenctl's raw
observers opt in to observer §8.2's explicit two with `--explicit`:
`record` (with `--on` too), `echo` and `timeline` add both selectors
beside the default, and `snapshot` adds `@state` alone, since a stream
answers no GET. Beside a selector the operator typed, the flag is refused
(exit 2): it would add every service's explicit traffic to the scope
typed. zengui's `explicit` preset names the same two selectors.

**O6. A bounded observer reports what it dropped.** It counts three things
and never folds them into one number:
- *lagged:* samples missed while the tool was behind;
- *evicted:* keys or lines forgotten to stay within the bound;
- *coalesced:* a burst folded into one update.

A non-zero count makes every "observed N" in the same report a lower bound,
and the report says so. A view that shrinks silently cannot be told from a
bus that went quiet. In zk2, R6's discards are a fourth, separate count: a
sample put on a wildcard key, which a consumer drops by rule (core §3.2).
They are not losses. A window's own loss, which decides whether it is
complete, is normative in observer §5.2.

**O7, pooling.** A tool never pools measurements from different stampers
into one distribution: a pooled median describes neither clock.

## 4. Tools do not write what they do not own

P3 (core §6, §11.1): a key is written only by the service that owns it.
- A tool acts on a service through its operations (`call`, core §5). It
  never writes the service's keys.
- zenctl's `pub` refuses a zk2 data key, with exit 2, and still writes
  foreign keys, as the bytes it is given. A tombstone on a foreign key is
  the operator's act (`--i-know`): no contract says whose it is to send.
  v1's `retire` has no zk2 meaning, and is gone.
- **The one exception** is a replayer standing in for the owners it
  recorded, in a namespace of its own (§5).
- **A mock owner is not an exception**: it *is* an owner. zenctl's `gen` and
  `serve` (#612, FJ8a) bring up a real service at an address the operator
  names, through the runtime's `ServiceBuilder` (descriptor, tokens, writers,
  the core §8.2 order), so P3 holds by construction. They refuse an address
  whose instance token is already present, which would be a second writer of
  its keys, unless `--i-know`. Their descriptor's `meta` carries
  `synthetic`, which is informative (core §3.3): a sample's attachment is
  the contract's to type (core §2.3), so it cannot carry a marker.

## 5. Recording and replay

v1's `.zrec` format minimum (RFC 13 §4.1, on the `v1` branch) carries over:
- **The header** carries the selectors the recording watched (observer
  §8.2), the namespace the operator stated, and when it was captured.
- **Payloads** are recorded losslessly, as bytes. A decoded rendering
  beside them is not the payload.
- **A gap** is a drop record placed where it happened, never summed at the
  end (O6). A reader shows the drop total beside anything it renders from
  the file.
- **Timestamps** are re-stamped on replay, deliberately: a replay is new
  traffic, and says so. The recorded stamp rides along, informatively.

zenctl's `.zrec` is version 3 (#612, FJ8a). Its header adds `excluded`: the
verbatim chunks (`@stream`, `@state`, `@op`, `@zk`, `@adv`) that no selector
of the capture names, so the file cannot contain them whatever the bus
carried (observer §8.2). A capture taken with `--explicit` lists all three
of observer §8.2's selectors, and its `excluded` names only `@op`, `@zk`
and `@adv` (#924). Each row adds `qos_axes`, the sample's priority,
congestion control, reliability and express as received, which a replay
honours. Versions 1 and 2 still read; their rows' v1 QoS profile names do
not (#612, FJ9), so such a row replays on the axes the operator gives.

What version 2 added (#218), and version 3 keeps:
- **A state preamble.** Rows marked `"preamble": true`, at `t` 0, before
  the first observed row: state fetched when a triggered capture began.
  The header's `preamble` names the fetch: its row count, the span it was
  collected over, its selectors, what it is a snapshot *of* (only the keys
  the retained window cannot show, or the full state), and what it could
  not fetch. A reader counts preamble rows apart from observed rows (O6).
- **A trigger record**, `{"trigger": {…}}` with no key, placed where the
  capture's rule fired (`record --on`). The header's `pre_roll` names what
  the retained window was asked for and what it held, with its two
  eviction kinds apart (O6). The pre-roll covers only what was watched
  (observer §8.2).
- **An unknown version is refused,** never guessed at. A reader reads
  every version up to its own, and refuses the next.

**The replay etiquette**, which zenctl enforces:
- **A dry run first.** `replay --dry-run` lists every put a replay would
  make, and opens no session.
- **The header's base is a contract.** A replay onto another base than
  the capture's is refused without `--force-base`. So is one from the
  empty base onto the empty base, which cannot tell two deployments apart
  (#506). `--namespace` moves the keys on purpose (below).
- **Re-stamped state wins over live state.** A replayed put is new when it
  is made, so it overwrites what the owners hold. That is why a preamble
  is never replayed unless the operator passes `--seed-state`, and the
  replay says how many rows it skipped.
- **Two replays.** A re-publishing replay (zenctl's) makes puts. A pane
  replay (zengui's scrubber) feeds the tool's own views from the file and
  touches no session.
- **Two clocks.** A `.zrec` carries the observer's arrival offsets (`t`)
  and the samples' stamps. A time axis says which one it plots
  (observer §8.3); zengui's scrubber plots `t`.

**A snapshot** (`.zsnap`, version 2 since #612's FJ8b) is the owners'
current state kept on disk: one S4 GET per selector (core §4.2), and a row
per key with its identity, its payload's conformance, whose clock stamped
it, and who holds it.
- **It is collected over a span, never at an instant.** The header states
  the span (`collection_span_s`), and so does every rendering.
- **Its holder is evidence,** read from presence: an instance held the
  address's token, a complete read showed none, or no presence read was
  made, and the holder is unattributed (O4).
- **A diff** states both spans, aligns two snapshots by zk2 key, each
  file's namespace stripped, and keeps a row's value, conformance and
  holder apart.
- A snapshot is read, never published.

**A replayer stands in for the recorded owners** (r4 §4.1, spike S13). It
republishes under the original addresses in a namespace of its own, a
`replay` deployment, through a session in that namespace
(`replay --namespace`), where the owners are not running. Replaying into
the namespace where they run would write keys they own, against P3 (§4).
That deserves an explicit operator override. Without a namespace, a replay
lands at the bus root, where no consumer of the deployment listens.

## 6. What does not carry over from v1

- **The registry rungs** (`Registered`, `Unregistered`, no slice for this
  producer) are replaced by contract resolution (§3, O2's rungs 3–7).
- **The base sweep** (`base list`) is replaced by `namespace list`.
- **Cutover acceptance** (RFC 13 §6) proved a v1 keyspace migration done.
  zk2's revision safety is the classifier and the history (core §9.7–§9.8:
  `compat`, `zk2 contract check-history`). A move from v1 to zk2 is each
  adopter's own (#614).
- **Profile-backed checks** stay dark until their profiles exist (#613).
  These are blobs and export, and `field-stuck`, which judges against a
  declared freshness (`freshness.v1`). Configuration came back with
  `config.v1` (#768), and alerts with `alarms.v1` (#800): `zenctl alarms`,
  the watchdog's `alert-firing`, the doctor's `alarm-key` and `alarm-stale`,
  and `check conform`'s `alarm-key` and `alarm-document` ask its §5
  questions; zenwatch's `alerts` notifies the first per alarm key, as it
  moves, and its `alert-firing` the second, as the watchdog does (#800,
  AD). Kinds came back with `telemetry.v1` (#819, TC), in place of v1's
  `kind-mismatch`: `zenctl telemetry` reads each metric member's kind and
  unit, its samples' conformance and each pair's continuity, with how its
  epoch was had; the doctor's `telemetry-declaration` and
  `telemetry-sample` (`--deep`) and `check conform`'s
  `telemetry-declaration`, `telemetry-sample` and `telemetry-continuity`
  ask its §5 questions; and a metric sample that does not conform is
  `invalid` wherever a payload is checked, its reasons named. zengui's
  Inspector reads a metric member through the same book (#819, TD): its
  kind and unit beside its type, its value plotted in that unit, a
  counter's §2.11 rate with a gap at each reset, and a decrease within an
  in-band epoch as the finding.
  The population budget was among them until core 0.24 (#735) made it core
  (spec §2.7; what a tool reads and concludes is observer §5 since #925):
  `check conform`'s `budget`, `budget-window` and `rate` cases and the
  doctor's `population-over-bound` ask it.
  A tool says *not asked*, never *clean*. `freshness.v1` exists since #720:
  `check conform`'s `freshness` case asks it, and `field-stuck` is still to
  be ported onto it. `health.v1` exists since #721: `zenctl health`, the
  doctor's `health-*` checks and `check conform`'s `health` cases ask it,
  and `hostid.v1`'s cloned ids are the doctor's `hostid-duplicate`.
- **Origin alignment** (RFC 13 §4.4's `--normalize-origins`) has nothing to
  align: a zk2 key names its system and service, so two deployments compare
  by key, each file's namespace stripped. Their two clocks are still never
  compared.

## 7. Testing a tool against this guide

Two corpora, over shared fixtures:
- **A contract test** pins what each report family serializes to. In the
  fleet, every serde-pinned shape lives under `zenkey-fleet/src/report/`,
  with its pinned-shape test beside it.
- **A render test** pins how each report is drawn (zenctl's
  `tests/render.rs`). It is a checklist per report: the rule, the
  distinguishable states, and each medium's spelling of them.

**Pin the distinctions, not the wording.** For every question a report can
pose, assert that the not-asked spelling differs from every answered one.
Run that against a fixture where every count is non-zero, because a
renderer that sums the counts passes any fixture where two are zero.
Honesty tests that pin copy are the tests people delete.

`observer.md`'s rules are checked the other way, by the spec's fixtures
and scenarios, which every implementation runs; these two corpora are how
the reference tools hold to them and to this guide.

## 8. What the reference keeps from v1's operations cookbook

v1's operations cookbook (RFC 09, on the `v1` branch) held practices that
zk2's texts state no rule for, and the reference tools still follow them as
their own. They are guidance: another tool may differ, and says so.

### 8.1 The session posture

A tool that observes a deployment it does not belong to opens its session
in this posture, unless its user states otherwise (`zenkey_fleet`'s
`open_reporting`, which zenctl, zengui and zenwatch share):
- **A client.** An observer is not a routing node: a peer listens on every
  interface, gossips its locator, and is offered links that churn every
  time the tool exits. It is a peer only when the user gives a listen
  endpoint, or when the user's zenoh config states `mode`.
- **Multicast scouting off,** with or without a config file, and gossip
  left on. The two are independent switches: multicast joins any mesh it
  can reach, a colleague's or production's, and gossip carries
  reachability through a router to the peers behind it. Turning both off
  to isolate a test cuts spokes off from each other, silently. A key the
  user's file states is the user's choice; a key zenoh defaulted while
  loading the file is not, since zenoh's defaults are `peer` and multicast
  on. **Isolated verification** is multicast off, gossip on and explicit
  endpoints (zengui's contexts offer it in one click).
- **Endpoint errors refuse.** An endpoint that does not parse is refused by
  name before a session is attempted. A client whose endpoints do not
  answer fails to open, and the tool reports no verdict, never an empty
  bus (observer §2).
- **The wire in no namespace.** A namespace strips its prefix on ingress
  (core §1.6), so raw views, wire selectors and the admin space (`@/…`)
  are read through a session in none, and a config file that sets a
  namespace for that session is refused. Resolved reads open a second
  session in the deployment's namespace, as its services do (#612, FJ4).

### 8.2 `$*` in a selector

The reference refuses `$*` in a selector its user types: zenctl's raw
verbs, zengui's scopes and zenwatch's rules. `$*` matches inside a chunk,
which strains zenoh's matching, and a key that seems to need it holds a
chunk that should be two. The one `$*` the reference asks itself is
`redundancy.v1` §2.15's standby probe of a template-less operation.

### 8.3 Matching status

zenoh reports matching status, "a subscriber matches this publisher" or
"a queryable serves this querier", only on publishers and queriers, and
only for the ones a process declared itself. The reference shows it for
its own publications and repeating queries, worded as a routing fact
about that entity ("no subscriber currently matches this publisher"),
never as a fleet verdict ("nobody listens on this key"): another
process's publisher may have subscribers this one cannot see (observer
§2). Whether someone else's publisher has subscribers cannot be read from
outside without publishing on its key, and the reference does not try.

### 8.4 Router storages

`zenctl storage gen` writes a router's `storage_manager` block, and its
`--check` compares the plan with a router's config file or its admin
space. zk2 runs no
storage on an owner's state (core §4.2 S4) and derives the union storages
of events from the contracts (core §2.6). The rest is zenoh's storage
manager, which the plan holds to:
- **A key expression and its `strip_prefix`.** A storage's `key_expr` is a
  valid zenoh key expression, and its `strip_prefix` is a literal string
  prefix of it: the plan derives the leftmost run of chunks with no
  wildcard.
- **A volume's capability pair.** A backend advertises *persistence*
  (volatile or durable) and *history* (latest or all). `memory` is
  volatile and latest, `fs` and `rocksdb` durable and latest, `influxdb`
  durable and all. `redb` offers both histories, and the choice is the
  volume's, so a deployment declares one volume per mode from the same
  plugin. A plugin outside this table declares its history, or the plan
  refuses it.
- **Replication** aligns one value per key, so it runs only on a
  latest-mode volume: the storage manager refuses to start a replicated
  storage on an all-mode one, and the plan refuses it first. Every replica
  uses the same `key_expr`, `strip_prefix` and parameters, with
  `propagation_delay` below half the `interval`: divergent parameters
  cause digest storms, not errors.
- **`complete: true`** is right only on a replicated latest-mode storage,
  where it lets a router answer a GET from the nearest replica. The plan
  emits it false elsewhere, and says why.
- **`garbage_collection` is a tombstone lifetime, not a retention.** It
  prunes metadata, tombstones included, older than its `lifespan` (zenoh's
  default 24 h), and never drops a value (core Appendix B). Retention is
  the backend's: an InfluxDB retention policy, or `redb`'s own block,
  which an all-mode `redb` storage needs to start and a latest-mode one
  refuses.
- **Overlaps.** Two storages whose selectors intersect both answer a GET
  under both, with duplicate and possibly divergent replies. Accept it, or
  carve one selector out of the other.

### 8.5 The doctor's memlock floor

`zenctl doctor`'s `shm-memlock-low` reports a finding below 8 MiB
(`zenkey::shm::MEMLOCK_FLOOR`), the limit under which spike S9 saw shared
memory fall back to TCP. That floor is the reference's practice, not a rule:
`spec/observer.md` §7.4 asks only that a tool report the largest pool the
limit admits, which no tool can compare with a pool it does not know.
