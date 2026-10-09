# zk2 tooling guide

**Status: non-normative for the core.** [`spec/core.md`](../../spec/core.md)
binds what participants put on the wire. This guide binds what a **tool**
tells a person about the wire: zenctl, zengui, zenwatch, a probe, a
dashboard, a recording. Where it restates a core rule, the core wins.

It carries v1's observer conformance (RFC 13, frozen on the `v1` branch at
v1.50) over to zk2. Most of its rules answer a mistake a v1 tool actually
made; the zk2 additions answer what porting found. The reference tools (`zenkey-fleet` and zenctl) are
held to it by their tests (§7). This is #612's tooling guide.

Contents:
1. [The judgement shape](#1-the-judgement-shape)
2. [Silence is never a verdict](#2-silence-is-never-a-verdict)
3. [The observer obligations](#3-the-observer-obligations)
4. [Tools do not write what they do not own](#4-tools-do-not-write-what-they-do-not-own)
5. [Recording and replay](#5-recording-and-replay)
6. [What does not carry over from v1](#6-what-does-not-carry-over-from-v1)
7. [Testing a tool against this guide](#7-testing-a-tool-against-this-guide)

---

## 1. The judgement shape

Every verdict a tool presents fits one shape. That includes a badge, a
table cell, a report field and an exit code.

```text
Established(yes) | Established(no)
Unestablished(NotAsked) | Unestablished(Unobservable { reason })
```

- **Established:** the question was put to the bus, and the bus answered.
  Both poles are answers, and each claims evidence.
- **NotAsked:** nobody put the question. No contract was retrieved, no
  window ran, the flag was not passed. The tool has learned nothing, and
  renders nothing, never the negative answer.
- **Unobservable:** the question was put, and the observation could not be
  had. The session did not open, the bundle did not verify, the bytes did
  not decode. The `reason` is part of the verdict: it names what stood in
  the way.

The two Unestablished kinds stay apart in every medium. They call for
different actions: NotAsked is cured by asking, Unobservable by fixing the
named impediment.

**Polarity.** Every verdict vocabulary documents its mapping onto the
shape, and names which Established pole is the *finding*. A vocabulary
named for the sought answer (`compatible`, `Met`) has its finding on *no*.
One named for a condition firing (`Explained`, `split-brain`) has its
finding on *yes*. A renderer or an exit map that assumes the wrong polarity
inverts every verdict, and still looks honest.

**Exit codes.** A judging tool projects the shape onto its exit code, after
the polarity mapping:

| Exit | Meaning |
|---|---|
| **0** | Established, and clean: no action is needed. |
| **1** | Established, and a finding. |
| **2** | Unestablished, either kind. The report says which. |

- **An empty scope is Unobservable.** When nothing in scope was observed (no
  token, no reply, no sample), the judgement exits 2, never 0. A monitor
  pointed at the wrong namespace must not be green.
- **zenctl's spelling.** Its exit contract (`zenctl/src/exit.rs`) is this
  projection, and every verdict exits through
  `zenkey_fleet::judgement_exit_code`. Usage errors and refused input also
  exit 2: asking nothing is not a verdict.

Worked mappings in the zk2 tools:

| Vocabulary (question) | yes | no | Unestablished |
|---|---|---|---|
| `compat`: is the new revision compatible with the old one? (§9.8) | `compatible` | `review` or `breaking`: the finding | a side that does not load or retrieve (Unobservable) |
| A presence read: does a token match? (§8.1) | the tokens | complete and empty, worded "no token visible to this reader" (below) | a read that ended at its timeout: possibly incomplete (Unobservable) |
| A state GET: what is the current value? (S4) | the owner's value, or its deletion | none: a GET answers or is silent | silence (Unobservable). An archive's answer is a *different* question (§2). |
| Split-brain: do two instances serve one exclusive operation at once? (§6) | `split-brain`: the finding | none seen | a holder whose descriptor or bundle could not be read (`undecided`) |
| A `doctor` check: does its condition fire? Every id names the condition, `split-brain` to `router-version-skew` (#612, FJ6) | a finding, with its severity, subject and evidence | clean, with the evidence that makes it clean as the reason | a subject it could not decide (Unobservable, each listed `unjudged` with why); every check that reads presence over an empty scope (Unobservable); a check the run did not ask (NotAsked) |

## 2. Silence is never a verdict

**The absence of a signal is evidence about nothing in particular.** A
tool that heard nothing has established nothing (not absence, not health,
not death) unless its procedure made the silence *attributable*. That needs
three things:
- a bounded window that was provably open;
- a named party expected to speak within it;
- an independent check that speaking was possible.

Unattributable silence is Unestablished, never Established(no).

zk2's instances, each normative where it lives:
- **The reply set (core §5, O5).** An empty reply set is not "no such
  operation". A caller attributes silence through presence: the service
  was present and silent, or absent. Envelopes in a fan-out carry no key,
  so they are reported unattributed (§5.1).
- **Presence (core §8.1).**
  - A liveliness read that reached its timeout ends with an error reply. It
    is possibly incomplete, never a list of what exists.
  - A read that access control refuses is answered **complete and empty**,
    exactly like a selector no token matches (0.8). So a tool words absence
    as what its reader could see: "no token visible to this reader".
    Absence is a verdict only under grants that let the reader see presence
    (§11.1).
- **State (core §4.2, S4–S6).** Silence from the owner's GET is not "no
  value". A tool that then reads an archive presents the answer as
  **last-known**, never current, and says which archive answered.
- **Contracts (core §8.4, §9.6).** A bundle that no holder served, or that
  failed verification, is "contract unavailable", not "no contract". A tool
  never decodes with an unverified bundle.
- **Constrained faces (core §8.5, R7).** What `link.v1` does not expose
  across a face is *unobservable* from the far side, never absent.

## 3. The observer obligations

An observer is any tool that reads the bus without owning keys on it. A
verdict reaches a person through three media, and a conforming tool keeps
its claims straight in all three:
- a **table cell**, the aligned rendering a person scans;
- a **report field**, the serialized shape a script branches on;
- a **note**, a statement *about* a report: coverage, silence, bounds,
  caveats.

The reference tools render one report struct through all three. That is
what keeps the three in agreement.

**O1. A key that is not zk2 is a fact, not an error.** The bus is shared
(core §1.7): v1 keys, rmw_zenoh keys and other applications coexist.
- A tool never hides or refuses a foreign key. It shows what it can still
  establish: the chunks, the traffic, and a structural rendering of the
  payload (§7.2's last rung). Then it states what it could not.
- In a report, it is a partial report whose verdict says how far
  description got. The fields below the failure point are absent, never
  defaulted. The fleet's spelling is `Unresolved::NotZk2`.

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
not (core §7.2, 0.8).

**O3. Do not guess another deployment's namespace.** A data key outside
the configured namespace is "not in this namespace". A tool does not
attribute it to a namespace by scanning for a `zk2` chunk: a namespace may
contain one. Naming namespaces is `namespace list`'s job. It reads instance
tokens, whose form has a fixed arity (`<ns>/zk2/<system>/<service>/@zk/instance/<id>`),
and so attributes from the right.

**O4. Not asked is not answered no.** A tool distinguishes a question it
has not put from one answered negatively.
- **Table cell:** a reserved unknown mark (`—`), distinct from the empty
  answer and every negative word.
- **Report field:** a not-asked field is *absent*, never a defaulted
  `false`, `0` or `null` that collides with an answered state.
- **Note:** names what was not asked and how to ask it ("contract not
  retrieved; pass `--contracts`, or let the tool fetch it").

**O5. A wildcard scope is not total coverage.** `*` and `**` never match a
verbatim chunk. So an ambient selector (`zk2/**`, `zk2/<system>/**`) never
delivers:
- `@stream` or `@state` samples;
- `@op` queryables;
- `@adv` sidecars;
- control keys under `@zk`, contract bundles included (`zk2/@zk/contract/…`).

That guard is a gift, and a trap (core §1.3). A tool that offers a
wildcard scope either names the verbatim kinds it watches beside it, or
states that they are excluded. The exact selectors ride the report (a
`scopes` field), and a recording's header carries them too.

**O6. A bounded observer reports what it dropped.** It counts three things
and never folds them into one number:
- *lagged:* samples missed while the tool was behind;
- *evicted:* keys or lines forgotten to stay within the bound;
- *coalesced:* a burst folded into one update.

A non-zero count makes every "observed N" in the same report a lower bound,
and the report says so. A view that shrinks silently cannot be told from a
bus that went quiet. In zk2, R6's discards are a fourth, separate count: a
sample put on a wildcard key, which a consumer drops by rule (core §3.2).
They are not losses.

**O7. A tool that reports a timestamp names who stamped it.** A state
sample's stamp carries its clock's id (core §4.1, §4.3).
- **The owner's own stamp.** It is the owner's when S1 holds, and its id
  then matches the owner's session, which the descriptor's `meta.zid`
  names where present. Two zids compare by value, never by their text:
  zenoh drops leading zeros (core §3.3, 0.11).
- **A router's stamp.** A router stamps what arrives unstamped.
- **No stamp.** A stream sample may carry none.

A tool says whose clock a time, an age or a latency is on. It never pools
measurements from different stampers into one distribution: a pooled median
describes neither clock.

## 4. Tools do not write what they do not own

P3 (core §6, §11.1): a key is written only by the service that owns it.
- A tool acts on a service through its operations (`call`, core §5). It
  never writes the service's keys.
- zenctl's `pub` refuses a zk2 data key, with exit 2, and still writes
  foreign keys. v1's `retire` has no zk2 meaning, and is gone.
- **The one exception** is a replayer standing in for the owners it
  recorded, in a namespace of its own (§5).

## 5. Recording and replay

v1's `.zrec` format minimum (RFC 13 §4.1, on the `v1` branch) carries over:
- **The header** carries the selectors the recording watched (O5), the
  namespace the operator stated, and when it was captured.
- **Payloads** are recorded losslessly, as bytes. A decoded rendering
  beside them is not the payload.
- **A gap** is a drop record placed where it happened, never summed at the
  end (O6). A reader shows the drop total beside anything it renders from
  the file.
- **Timestamps** are re-stamped on replay, deliberately: a replay is new
  traffic, and says so. The recorded stamp rides along, informatively.

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
  These are kinds and budgets, alerts, configuration, blobs and export.

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
