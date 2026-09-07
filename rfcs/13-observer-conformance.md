# 13 — Observer Conformance

**Status: v1.24 (ratified)** · normative chapter · *created in v1.24 and
amended in v1.32, v1.34 and v1.35 — see [CHANGELOG.md](CHANGELOG.md)* — carved from chapter 09 §5.1–§5.3 and
§6; the moved material entered the set in v1.2, v1.9, v1.13 and v1.19
and was amended there in v1.18 and v1.21

> **Citation migration.** Before v1.24 everything in this chapter lived in
> chapter 09, and pre-v1.24 texts and code cite it there: "RFC 09 §5.1"
> (the observer obligations), "09 §5.2" (capture and replay), "09 §5.3"
> (the synthetic marker), "09 §6" (cutover acceptance). Those citations
> remain resolvable — [09](09-operations.md) keeps a tombstone at each old
> section naming its new home here — and the O-rules keep their numbers, so
> "RFC 09 §5.1 O4" and "13 §3 O4" name the same rule. New text cites this
> chapter.

Every chapter before this one specifies what a participant owes the bus.
This one specifies what a tool owes its **user**: an observer — `zenctl`,
`zengui`, a probe, a dashboard, a capture file — sits between the wire and
a person, and everything the person believes about the fleet passes through
what the tool chose to render. The convention can make keys parseable and
registries honest; it cannot stop a tool from rendering "I did not look" as
"it is not there". These rules can, and every one of them is a mistake that
was actually made (§3's provenance box).

The material graduated from a carve-out in the operations cookbook to a
chapter of its own when the reference tools were audited against it: seven
rules had become the contract for every report family in two explorers, the
same three-valued verdict had been reinvented five times under five names,
and the one thing the carve-out never said — what a conforming rendering
*looks like*, and how to test for it — was the thing every audit row
needed. This chapter says it.

Scope: the rules bind **tools** — observers, and any tool that renders a
verdict about the bus, including the judging halves of tools that also
publish (a replayer's dry-run, a generator's report, an acceptance probe).
Nothing here binds a producer or consumer, and nothing here makes
non-conformant traffic legitimate or illegitimate — the convention still
declines to govern foreign traffic, and this chapter only says how to
*report* it.

---

## 1. The judgment shape (normative)

Every verdict a tool presents — a badge, a table cell, a report field, an
exit code — MUST be expressible in one shape:

```
Established(yes) | Established(no)
Unestablished(NotAsked) | Unestablished(Unobservable { reason })
```

Three values, with the third split in two — because "I don't know" is two
different facts, and the difference is load-bearing:

- **Established** — the question was put to the world and the world
  answered. `yes` and `no` are both *answers*: each names evidence, and a
  tool that renders one is claiming it holds that evidence.
- **Unestablished(NotAsked)** — nobody put the question. No registry was
  loaded, no listen window ran, the flag was not passed, the pane was
  never opened. The tool has learned *nothing*, and must render nothing —
  never the negative answer (§3 O4).
- **Unestablished(Unobservable{reason})** — the question **was** put and
  the observation could not be obtained: the session would not open, the
  bytes did not decode, an input the judgment needed was itself
  unestablished. The `reason` is part of the verdict, not decoration — it
  names what was asked and what stood in the way, which is exactly what
  distinguishes this state from NotAsked at the moment a user (or a
  script) decides what to do next.

The two Unestablished kinds MUST be kept apart, in every medium where the
verdict appears. They license different actions — NotAsked is cured by
asking, Unobservable by fixing the named impediment — and conflating them
misleads in both directions: rendering Unobservable as NotAsked hides a
failure the user should see; rendering NotAsked as Unobservable claims an
attempt that never happened. (This split was rediscovered independently by
five verdict vocabularies in the reference implementation before it was
named here, and mis-folded by one — which is the empirical argument for
naming it once, normatively.)

Two boundary notes, so the shape is applied and not stretched:

- **A question that does not arise is not a verdict.** "Is this subject
  registered?" is not posed of an `@rpc` key — the registry has no surface
  there ([03 §1.4](03-grammar.md)). A tool states the *established* fact
  that makes the question moot ("a plane key; no registry surface"), and
  does not manufacture a fourth-state answer to a question it never posed.
- **Questions chain, and the shape composes.** An Unobservable reason is
  frequently the Established(no) of a *prerequisite* question: "payload
  conformance: Unobservable — no schema is served for this type" carries
  inside it the answered-no of "is a schema served?". A vocabulary maps
  cleanly only when it names *which* question each verdict answers — the
  reference implementation's `NoSchema` / `NoRegistry` split is exactly
  this rule: "asked the registry, none served" vs "no registry was ever
  consulted", two silences with distinct wire spellings.

### 1.1 Surface vocabularies, and the polarity trap

Domain vocabularies remain free — this chapter does not rename anybody's
verdicts. `OldStillSpeaks`, `Impaired`, `Unproven`, `Explained`, the
`NotValidated` reasons: each is a better word *in its context* than the
bare shape, because it carries the question with the answer. What is
required is the mapping:

**Every verdict vocabulary MUST document its mapping onto the shape** — in
its report contract, registry entry, or tool documentation — naming the
question it answers, which verdict lands on which of the four states, and,
as a required line of the mapping, **its polarity**: which Established pole
is the *finding*.

The polarity line is required because it is where mappings quietly go
wrong. A vocabulary named for the sought answer ("Met", "Pass", "Valid")
puts the finding on Established(no); a vocabulary named for the **condition
firing** ("OldStillSpeaks", "Explained", "over") puts the finding on
Established(yes) — the same shape, poles reversed. A renderer, a color
scheme, or an exit-code map that assumes one polarity and receives the
other inverts every verdict it touches while remaining perfectly
three-valued. The mapping is one sentence per vocabulary; the bug it
prevents survives code review, because both readings look honest.

Worked mappings from the reference implementation (informative):

| Vocabulary (question) | Established(yes) | Established(no) | Unestablished |
|---|---|---|---|
| `expect` — "did the window meet the spec?" | `Met` | `NotMet` | `Impaired` (Unobservable: the observation never stood up) |
| `cutover` — "did the migration finish?" (§6) | `Pass` | `OldStillSpeaks` — **condition-fires polarity: yes is the clean pole, no is the finding** | `Unproven` (Unobservable: everything silent — a dead fleet passes the silence half for free) |
| `why` — "is a cause for the silence established?" | `Explained` — **condition-fires polarity: yes is the finding** | `Healthy` (no cause, and everything checked looks healthy) | `Impaired` (Unobservable: a rung's input could not be obtained) · per-rung `NotAsked` |
| schema validation — "does the payload conform?" | `Valid`* | `Invalid`* | `NotValidated(NoRegistry)` (NotAsked) · `NotValidated(NoSchema \| Undecodable \| …)` (Unobservable, reason carried) |
| registration — "does a slice declare this subject?" | `Registered` | `Unregistered` · `NoSliceForProducer` | `Unknown` (NotAsked: no slice set loaded) |

\* polarity as named; the finding is Established(no).

### 1.2 The exit-code projection

A judging tool that exits with a verdict MUST project the shape onto its
exit code as:

| Exit | Meaning |
|---|---|
| **0** | Established, clean — the answer that requires no action |
| **1** | Established, finding — the answer that does |
| **2** | Unestablished — *either* kind; the report distinguishes which |

The projection is applied **after** the polarity mapping — exit 1 is the
finding, whichever Established pole the vocabulary spells it on — which is
the polarity trap's cash value: a condition-fires vocabulary that projects
its poles positionally instead of by the mapping ships inverted exit
codes, and CI built on them enforces the opposite of the intent.

Both Unestablished kinds project to 2 because a script must not *act* on
nothing either way — but the report behind the exit distinguishes them
(§1's MUST), so a caller that needs to know whether to ask again or to fix
something reads the report, not the code. And 2 is never a substitute for
1: "could not check" exiting like "checked and failed" would let a broken
observation masquerade as a finding — the exact confusion
[05 §3.1](05-control-rpc.md) forbids on the wire, at the process boundary.

## 2. Silence is never a verdict (normative)

The general rule, of which this convention states several instances:

**The absence of a signal is evidence about nothing in particular.** An
observer that heard nothing has established nothing — not absence, not
health, not death — unless the observation procedure made that silence
*attributable*: a bounded window that was provably open, a named party
that was expected to speak within it, and an independent check that
speaking was possible. Unattributable silence maps to **Unestablished**,
never to Established(no).

The instances, each normative where it lives:

- **The reply set** — [05 §3.1](05-control-rpc.md), the rule's original
  home, which stays there: an empty reply set conflates "no queryable
  matches" with "session dying mid-boot", and callers MUST NOT treat it as
  a verdict about any specific host. The liveliness roster is what makes
  RPC silence attributable (*alive ⇒ callable*).
- **The badge** — §3 O4: a question not yet asked must not render as a
  negative answer.
- **The window** — §3 O6: a bounded observer's silence is only as good as
  its accounting; a dropped sample is a hole in the window, and an
  unreported hole converts "I missed it" into "it never happened".
- **The acceptance run** — §6: a migration is proven finished by silence
  *on a fleet that is provably speaking*; everything-silent is `Unproven`,
  a non-verdict.

## 3. The observer obligations (moved from 09 §5.1; added v1.9, amended v1.18, v1.21)

*Added in v1.9. The convention told tools how to read a conformant key and
said nothing about the other three cases an explorer actually meets: a key
under a different base, a key that is not this convention at all, and a
question the tool has not yet asked. Each was left to be invented per tool,
and the obvious inventions are the dishonest ones.*

An **observer** is any tool that reads the bus without publishing on it —
`zenctl`, `zengui`, a probe, a dashboard. Observers run un-namespaced
([09 §4](09-operations.md)), so they see everything on the wire, including
traffic this convention does not govern. The rules below are what keeps
that honest.

Promotion to a chapter adds, for each rule, the two things the carve-out
never carried: what conformance observably *looks like*, and how it is
tested. A verdict reaches a person through three media, and a conforming
tool keeps its claims straight in all three:

- a **table cell** — the aligned rendering a person scans;
- a **report field** — the serialized shape a script branches on;
- a **note** — a statement *about* a report that must survive every
  format: coverage, silence, bounds, caveats.

The reference implementation renders one report struct through all three
media, which is what keeps them in agreement; the per-rule tables below
state the obligation per medium, and the **checklist row** under each rule
states what a conformance suite asserts. The test shape is two corpora
over shared fixtures: a *contract* test pinning what each report family
serializes to, and a *render* test pinning how it is drawn — a per-report
checklist where each row names the rule, the report family, the
distinguishable states, and each medium's spelling of them. Pin the
*distinctions*, not the wording: honesty tests that pin copy are the tests
people delete.

**O1 — A key that does not parse is a fact, not an error.** An observer
**MUST NOT** discard, hide, or refuse a key merely because it is not
conformant. Non-conformant traffic is not prohibited — nothing in this
convention binds a foreign publisher, and [03 §1.2](03-grammar.md) is a rule
about where the *convention's* keyspace lives, not a claim on the bus. An
observer **SHOULD** present such a key with whatever it can still establish
(its chunks, its traffic, its payload rendered structurally per
[08 §7](08-registry.md)) and state what it could not.

| Medium | Conforming consequence |
|---|---|
| table cell | the key appears as a row like any other — chunks, traffic, structural payload — with the verdict word in its column; never a filtered row, never an error line |
| report field | a **partial** report: every key yields one, a verdict field says how far description got, and fields below the failure point are absent — never defaulted |
| note | states what could not be established ("not a v1 key — rendered structurally"), not why the key is "wrong" |

**Checklist row.** Feeding the tool a non-conformant key yields a report,
not an error — pinned by fixture, because the natural implementation (a
builder that hard-errors on anything unregistered) is an O1 violation the
reference implementation actually shipped once. Every partial verdict has
a distinct serialization, and no medium drops the row.

**O2 — Classify by degrading, in this order.** Each rung that fails weakens
the claim rather than discarding the key:

| Rung | Question | On failure |
|---|---|---|
| 1 | Does it sit under the configured base (`strip_base`)? | **Not under this base.** Terminal. |
| 2 | Does the remainder parse as `v1/…` ([03 §1](03-grammar.md))? | **Unparsed**, carrying the reason. |
| 3 | Does a registry slice refine the subject ([08 §2](08-registry.md) precedence)? | **Unregistered**, or **no slice for this producer**. |

| Medium | Conforming consequence |
|---|---|
| table cell | one word per key, drawn from a closed set — *registered · unregistered · no slice for this producer · not a data class · not v1 · not under this base* — plus the not-asked state of rung 3 (§1: *registry not loaded* is NotAsked, not a rung failure) |
| report field | a machine-stable verdict enum: one variant per rung outcome **and** per not-asked state, so a script branches on the rung, never on prose |
| note | the failing rung's reason, verbatim from the grammar — it already cites its RFC section |

**Checklist row.** Every rung outcome has a fixture, and no two outcomes
share a spelling in any medium — in particular, rung 3's "slices loaded,
none declares this producer" and "no slices loaded" (O4) serialize
differently, because they answer differently.

**O3 — Do not guess another deployment's base.** An observer that finds a key
outside its configured base **MUST NOT** attribute it to a base by scanning
left-to-right for a `v1` chunk: a subject tail has no fixed arity, and a base
may itself contain a literal `v1`. Naming other bases is the base-discovery
sweep's job ([09 §5](09-operations.md)), which attributes fixed-arity *from
the right*. "Not under this base" is the honest terminal answer.

| Medium | Conforming consequence |
|---|---|
| table cell | **not under this base** as the terminal cell — never a guessed base name in a base column |
| report field | the not-under-base variant carries **no** inferred-base field at all; base names appear only in the sweep's own report, where they were established |
| note | a next-step pointing at the sweep ("`base list` names the bases in use"), not a claim about this key |

**Checklist row.** Asserts the *absence* of a field: the not-under-base
report shape has nowhere to put a guessed base, so the guess cannot ship.
The sweep's fixture includes a base containing a literal `v1` chunk and
asserts it attributes correctly.

**O4 — "Not asked" is not "answered no".** An observer **MUST** distinguish a
question it has not put to the bus from one that was answered negatively. A
tool with no registry loaded has learned nothing about whether a subject is
registered, and rendering that identically to "this subject is not registered"
reports a verdict it never obtained — [05 §3.1](05-control-rpc.md)'s rule
applied to a badge rather than to a reply set (§2). The same holds for a
roster not yet seeded and a schema not yet fetched.

| Medium | Conforming consequence |
|---|---|
| table cell | a reserved unknown mark, distinct from both the empty answer and every negative word — the reference tables draw `—` (and `—` is not the empty string), the rung ladders draw `?`, dim rather than red: the absence of an answer, not a milder failure |
| report field | not-asked serializes as the field's **absence** (skip-if-not-asked), never as a defaulted `false`/`0`/`null` that collides with an answered state; where a bare absence would be ambiguous, the field is present-only-when-asked and always-present-when-asked — "valid" and "not checked" must never share a spelling |
| note | a coverage note names what was not asked and what would ask it ("no registry loaded — pass `--registry <dir>`; not asked is not answered no") |

**Checklist row.** For every question a report family can pose, the row
asserts the not-asked serialization differs from every answered
serialization — in both corpora, since the collision can be introduced in
either: the contract test pins the JSON absence, the render test pins that
`—` and empty draw differently.

**O5 — A wildcard scope is not total coverage, and must not be presented as
such.** `*` and `**` never match a chunk beginning with `@` (this is what
makes [03 §4](03-grammar.md) D2 and D4 true). A `**` subscription therefore
cannot reach `@rpc`, `@media`, `@blob`, `@adv` sidecars, the admin space,
**or any service origin**. The first half is a gift — a firehose subscriber
cannot accidentally pull video frames or bulk objects. The second is a trap:
an observer scoped `**` sees no `@catalog` traffic *by construction*, and if
it labels that scope "everything" then a healthy catalog and a dead one look
identical. An observer offering a wildcard scope **MUST** either name the
verbatim planes it wants alongside it, or state that they are excluded.

| Medium | Conforming consequence |
|---|---|
| table cell | the scope headline names what rides alongside or is excluded ("`**` — @rpc/@media/@blob and service origins excluded"), where the user reads the scope, not in documentation |
| report field | the exact selectors watched ride the report (a `scopes` field) — coverage is data a script can check, not prose; a capture header carries its selectors for the same reason (§4.1) |
| note | a coverage note, citing this rule, stating the exclusion or the named planes |

**Checklist row.** Any report produced under a wildcard scope carries its
scopes and a coverage statement; the fixture pins that the verbatim planes
are named or declared excluded — never implied by the selector alone,
which is precisely what a reader cannot be assumed to compute.

**O6 — A bounded observer reports what it dropped.** Any long-running observer
bounds something — a buffer, a scrollback, a key table — and an unbounded one
is merely a leak with better manners. Whatever the bound, the tool **MUST**
report what it cost, and MUST NOT fold the kinds into one number, because
they are different facts about the same window: samples missed while it was
behind ("we could not keep up"), keys or lines retired to stay within the
bound ("we chose to forget"), and — where the tool folds a burst into one
rendered update — samples **coalesced** ("we kept the newest and summarized
the rest"; amended at ratification, v1.18). Coalescing is neither of the
first two: nothing the view promised to show was lost, but a consumer that
assumes it saw every sample individually is wrong. A view that silently
shrinks is indistinguishable from a bus that went quiet.

| Medium | Conforming consequence |
|---|---|
| table cell | three counters, three cells — *lagged*, *evicted*, *coalesced* — never a combined "dropped" total |
| report field | three fields, each absent only when zero **of that kind**, never summed into one; a non-zero count weakens every "observed N" in the same report to a lower bound, and the report says so where the N is |
| note | a bound note per non-zero kind, citing this rule, stating what the bound cost in that kind's own words |

**Checklist row.** The three kinds serialize distinctly and no rendering
sums them — asserted against a fixture with all three non-zero, because a
renderer that adds them up passes any fixture where two are zero.

**O7 — A tool that reports a timestamp names who stamped it.** An observer
that surfaces a sample's HLC — as a time, as an age, or as a latency —
**MUST NOT** describe it as the publisher's clock unless it established that
the publisher stamped it. Zenoh timestamps at the **first node with
timestamping enabled**, which on a deployment configured
`timestamping: { enabled: { router: true } }` is a router rather than the
producer; the stamping node's identity rides on every stamped sample, and can
be compared against the publisher's whenever `SourceInfo` is present. Three
cases, and they are three: **self-stamped** (the HLC is the publisher's
clock), **stamped elsewhere** (it is that node's clock, and a latency computed
from it measures stamper → observer), and **unattributable** (stamped, but
nothing said by whom — O4 applies: unknown, not foreign). An observer
**MUST NOT** pool measurements taken from different stampers into one
distribution: they measure from different clocks, and a combined median
describes neither. Naming the stamper is the whole obligation — this rule
asks for nothing new on the wire.

| Medium | Conforming consequence |
|---|---|
| table cell | every time, age, or latency column is labeled with its clock's owner, and latency distributions are keyed by stamper class — three populations or fewer, never one pooled column |
| report field | the stamper classification rides beside every derived latency; the raw HLC field documents only that a stamp exists, and whose clock it is stays a separate, answerable question |
| note | a caveat note stating which clock a number is on — arrival vs HLC — and, for an HLC, which of the three cases holds |

**Checklist row.** A latency figure without its stamper classification
fails the row; a fixture with mixed stampers asserts per-class populations
and the absence of a pooled aggregate. The reference engine's integration
test is the model: it proves the classifier *conservative* rather than
wrong — verifying independently that the stamper really was the publisher
while the rendered verdict stays "cannot establish".

*Practical note (measured against zenoh 1.9).* A subscriber is not currently
delivered `SourceInfo` — not from a plain publisher and not from an
AdvancedPublisher — so the id-to-id comparison is usually **unavailable**, and
**unattributable** is the ordinary answer rather than the exceptional one. That
is not a reason to guess. An observer that names the stamping node and says it
cannot attribute it has told the truth; one that calls the same number "the
publisher's HLC" has not, whatever the deployment happens to be doing. The rule
is written against what the wire carries, not against what one release
propagates, so it needs no revision if that changes.

**Declared versus observed (v1.32).** Two registry declarations exist so
that a judge can compare them with the wire, and the four poles of §1 do
the work in both; they are named here by obligation, their stable check
ids being the reference engine's:

- **A subject's `kind`** ([08 §2](08-registry.md)). *Not asked* when the
  entry declares none. *Established(no)* when a self-describing payload's
  tag disagrees with the declared kind, or when a `counter` decreased
  between two samples of one origin with no `alive` cycle of that origin in
  between — the restart is the one sanctioned reset, and it is on the wire.
  *Unobservable* when the payload could not be decoded, said with the
  reason. Per origin, never pooled across hosts, with the window stated
  (O5, O6) — a window in which a counter did not decrease has not
  established that it is one.
- **A producer's `[budget]`** ([08 §2](08-registry.md)). *Not asked* when
  the file declares none. *Unobservable* when no health document carrying
  `self_stats` ([04 §1.2](04-planes.md)) was seen — and the reason reads
  "this producer does not say how big it is", because that is itself the
  finding an operator wants. *Established(no)*, per origin, when
  `rss_bytes` exceeds `rss_mb` or a named table exceeds its bound;
  *Established(yes)* otherwise, for the sample read. A fetch of health
  documents costs the data plane, so it is asked for explicitly (the
  frugality note below), never folded into an ambient render.

**A surface's `when` (v1.35).** [08 §2](08-registry.md)'s `when` column
is a declaration a judge can hold a producer to, on the same four poles:

- A judge that sees `error/unsupported` or `error/gated` from a procedure
  declared `when` files *exempt: <predicates>* — the same
  Established(no)-of-the-gate shape as the rest-variable exemption above,
  said out loud, never a pass by omission. The same error from a procedure
  **not** declared `when` is Established(no) of [08 §6.1](08-registry.md):
  a conditional surface declares its condition. The kind binding is judged
  too — `unsupported` from a procedure with no `feature:` predicate, or
  `gated` from one with only `feature:` predicates, is a finding.
- A `when` subject unobserved in a window is exempt and says so; observed,
  it is judged like any other subject.
- A predicate kind this build does not know makes the entry conditional
  all the same, and its binding is *not asked*.

**A conformance suite over the registry (v1.35).** A tool that executes
the registry as a test suite asserts, per declared surface, one of three
states — *met*, *not met*, and *unknowable* with its reason — and never
folds the third into the second: a window proves presence, never absence,
so a declared subject that did not speak is unknowable, not failed. In
any test-report medium an unknowable assertion is *skipped*, not failed,
and a build MUST NOT go red on one. The suite is a projection of the
observer's own checks — it can find nothing the observer cannot — and
silence from a procedure whose origin the roster shows alive is a finding
under §2 (*alive ⇒ callable*), not an unknowable.

**Exporter obligations (v1.34).** A tool that re-publishes its
observations as a metrics surface — a Prometheus exposition, a status page
— is an observer whose reader is another machine, and the rules above bind
it unchanged; three follow from O4–O6 in that medium, where the natural
encodings are all dishonest.

- **A series is never a verdict about the wire.** For every observer
  bound the surface exposes the same kinds O6 names as separate series —
  samples missed, keys or bytes retired, samples coalesced between
  scrapes — never a sum; and while the missed kind moves, every value
  exposed is a lower bound and the surface says so.
- **A series that stopped is not a series that went quiet.** A key the
  observer chose to forget (its bound), a producer whose liveliness ended,
  and a key that simply has not spoken are three states, each exposed by
  name. An exporter MUST NOT let a series silently disappear: to a scraper,
  absence and silence are the same byte.
- **Scope and provenance ride the surface.** The selectors watched and the
  planes a wildcard cannot reach (O5) are exposed as data on the same
  surface, and any derived value states what it derives from: names and
  units come from the registry's declarations, never from the leaf's
  spelling, and a payload verdict is exposed as three counted populations
  — valid, invalid, *not validated* — never as a ratio that hides the
  third.

Histograms, remote write and push are neither obligations nor forbidden;
the reference exporter refuses them, a tool decision this chapter records
rather than makes.

*Informative — frugality (added at ratification, v1.18).* The obligations
above are about honesty, not thrift, but one habit keeps both cheap: an
observer SHOULD retrieve only what its user asked to see. Rendering what is
already in hand — registry slices, a seeded roster — is ambient and free;
anything that costs the data plane (a probe, a fetch, a subscription) is
asked for explicitly, once per ask. Guidance rather than obligation, because
cost is a design budget rather than a truth condition — recorded because the
reference explorers hold to it (data movement costs exactly one deliberate
action; zenkey #84/#85), and because a tool that ignores it tends to violate
O5 by accident: a pane that quietly fans out to keep itself fresh is
claiming coverage nobody asked it to have. Cost stays a design budget for the
*observer*; a budget the *producer* declared in its own registry file
([08 §2](08-registry.md) `[budget]`, v1.32) is a truth condition of that
declaration, and judging it is the paragraph above, not this note.

> **Where this came from.** Every rule above is a mistake that was made and
> caught while building `zengui` against this convention, not a hypothetical.
> O2 and O4 replaced a boolean "registered" flag that rendered "no registry
> loaded" as "unregistered". O3 replaced a base guess. O5 replaced a design
> that had defended against `@media` frames arriving through a `**` scope —
> which cannot happen — while missing that the same scope silently hid
> `@catalog`. O6's third kind arrived the same way, at ratification: the
> reference GUI's link layer batches bursts under a cap and counts the
> overflow (`coalesced`) beside its broadcast lag (`lagged`) — an honest
> number the two-counter wording could only misfile. O7 is the first that came
> from the *engine* rather than the GUI: `zenkey-fleet` documented its HLC as
> "the publisher's clock" and computed a latency from it for as long as the
> measurement had shipped, while never once reading the stamper id that rode
> beside it (zenkey #213).

## 4. Capture and replay — `.zrec` (moved from 09 §5.2; added v1.13)

*Added in v1.13. The v1.5 amendment slate's H7 promised cookbook material
on "record/replay etiquette" and never delivered it — nothing existed to
document. With the `record` module in `zenkey-fleet` (zenkey #39) and the
`zenctl record` / `zenctl replay` commands (zenkey #53), the etiquette half
matters, because replay is **publishing**, and publishing a capture onto a
live fleet base is exactly the accident this convention exists to prevent.
The obligations of §3 apply throughout — a capture file is an observer
whose window happens to be on disk.*

A `.zrec` file is newline-delimited JSON — deliberately the *same row
dialect* the explorers already emit (`zenctl echo --format ndjson`)
and read back (`zenctl pub --from ndjson`), not a second format —
upgraded with what a pipe does not need but a capture does. The file
format is §3 applied to disk: the header names what was asked (O4) and
states the coverage of a wildcard scope (O5), non-conformant keys are
recorded verbatim (O1) — a capture curates nothing — and drop records are
interleaved where the gap happened (O6).

### 4.1 The format minimum (normative since v1.24)

Before v1.24 the format was documented informatively, with the reference
implementation's code normative for the details. Interchange needs more
than that — a capture outlives the build that wrote it, and two tools that
exchange files need a floor neither can silently move — so the floor below
is now normative. Everything **not** listed here (QoS fields, decode
renderings, source attribution, size accounting, …) remains
reference-implementation-defined, and this section says so rather than
freezing it.

- **Line 1 is the header**: a JSON object carrying at least `zrec` (an
  integer format version), `selectors` (the full wire selectors the
  capture watched — its O4/O5 coverage statement), `base` (the operator's
  *stated* deployment base at capture time; MAY be empty, the base-less
  deployment), and `captured_at` (RFC 3339). Recorded keys are full wire
  keys and are never re-derived from `base` (O3). A reader that does not
  know the version MUST refuse the file rather than guess.
- **Every further line is a sample row or a drop record.**
- **A sample row** carries at least: `key` (the full wire key, verbatim —
  conformant or not); `delete` (boolean, always present — every sample is
  a put or a tombstone, and that is a fact rather than an unanswered
  question); `t` (integer microseconds since capture start, on the
  **observer's arrival clock** — the pacing clock). A non-delete row
  carries the exact wire payload losslessly in `bytes` (base64); a
  `value` field is a decoded *rendering*, is not round-trippable, and
  MUST NOT be treated as the payload. A delete row carries no payload —
  the tombstone is the whole fact ([04 §1.2](04-planes.md)). The sample's
  HLC rides `timestamp` **informatively** (§4.2), and an attachment rides
  losslessly in `attachment_b64`.
- **A drop record** is `{"dropped": n}`, placed **where the gap
  happened** — never aggregated to the tail. O6 applied to a file: a
  capture taken while the observer was behind is a partial view, and the
  file itself says so, at the position of the loss.
- **Reader and replayer obligations.** A reader MUST ignore row fields it
  does not know (the dialect evolves additively) and MUST surface the
  drop-record sum beside anything it renders from the file. A replayer
  MUST re-stamp (§4.2), MUST route every `delete` row through the
  class-conscious retire gate ([04 §1.2](04-planes.md), the v1.12
  bullet) — confirmation off the `state` class, wildcards impossible by
  construction since rows carry concrete keys — and MUST refuse a base
  other than the header's without an explicit operator override (§4.3).

**Version 2 (v1.34).** A capture that begins mid-story needs the story's
start: a thirty-second pre-roll of `state` keys is deltas with no base,
because the value that explains them was published an hour ago. Version 2
lets a file carry that base and the moment it was taken for.

- The header carries `zrec: 2` and MAY carry `preamble` — `{count,
  collected_over_s, selectors, semantics, incomplete, failed}`, the bounded
  fetch that produced the preamble rows and what it could not fetch — and
  `pre_roll` — `{asked_s, covered_s, watched, evicted, expired}`, what the
  retained window was asked for and what it could give, with the ring's two
  eviction kinds kept apart (O6).
- A **preamble row** is a sample row with `"preamble": true` and `t: 0`,
  emitted before the first observed row. Its `timestamp` is the HLC the
  fetched value carried — provenance, not pacing. A reader MUST count
  preamble rows apart from observed rows: O6's "kinds never folded",
  applied to rows.
- A **trigger record** is `{"trigger": {rule, from, to, at, evidence}}` — no
  `key`, like a drop record — placed where the transition was observed, so
  a reader can say what fired and where in the file it did.
- A version-2 reader MUST read version 1; a version-1 reader MUST refuse
  version 2 — the unknown-version rule above, doing its job.
- A replayer MUST NOT publish preamble rows unless the operator opts in,
  and MUST report how many it skipped and why (§4.2).

### 4.2 Timestamps are re-stamped on replay, deliberately

Replayed samples go through declared publishers and receive the
*replaying* session's HLC; the capture's `timestamp` field is provenance,
not a value to reproduce. Reconciliation is by HLC — newer value wins,
newer delete wins, and an untimestamped sample cannot be reconciled at all
([04 §3.2](04-planes.md)) — so carrying a foreign, hours-old HLC back onto
the wire would make every replayed sample silently lose LWW against
anything live, which turns "replay onto a quiet base" into a no-op that
*looks* like a replay. Re-stamping keeps replay legible: what you
published now is newest now. The cost is the inverse hazard, and it is the
whole reason this section exists: **re-stamped old data wins LWW against a
live fleet** ([04 §1.2](04-planes.md)) — a replayed capture can overwrite
current state with last Tuesday. The hazard is sharpest on a version-2
preamble (§4.1): those rows are *state at capture start*, and re-stamping
them republishes a whole snapshot over the live fleet with no pacing
between the rows. The reference replayer skips them unless told
`--seed-state`, and says how many it skipped.

### 4.3 The etiquette

- **Dry-run first.** A replay whose puts you have not previewed is a write
  you have not reviewed. `zenctl replay --dry-run` lists every would-be
  put and performs none.
- **The header base is a contract.** Replaying under a base other than the
  one in the capture header is refused unless explicitly forced
  (`--force-base`) — the tool never re-derives a base from the recorded
  keys (O3), and "same keys, different deployment" is presumed to be a
  mistake until the operator says otherwise.
- **Tombstone rows are operator deletes.** A recorded delete replays
  through the same class-conscious retire gate as a live one
  ([04 §1.2](04-planes.md), the v1.12 bullet): confirmation off the
  `state` class, wildcards impossible by construction (rows carry
  concrete keys).
- **Two replays exist; do not confuse them.** *Re-publishing* replay (the
  CLI) makes real puts through declared publishers and is governed by
  everything above. *Pane* replay (the GUI's scrubber) feeds a tool's own
  views from the file and never touches a session — it is reading, not
  publishing, and needs no etiquette beyond honesty about its mode.
- **A time scrubber states its clock.** A `.zrec` carries two clocks — the
  observer's arrival offsets (`t`) and the publishers' HLCs — and a
  consumer plotting a time axis says which one it plotted (O7 names the
  HLC's owner, or says it cannot). The reference scrubber plots `t`.
- **A pre-roll covers only what was watched.** A capture taken from a
  retained window (`--pre`) holds only the keys the observer was watching
  when the trigger fired (O5); the header names the watch set, and the
  ring's two eviction kinds ride beside it (O6). The preamble states its
  `semantics` — what it is a snapshot *of* — because the values at the
  moment the ring began are not recoverable, and the honest substitute
  ("the current state of what the ring cannot show", or the full current
  state) has to be named rather than implied.

### 4.4 Snapshots — `.zsnap` (v1.34)

A snapshot is an observer's fan-in GET kept on disk: one row per key,
with what the observer could establish about each. It is the sibling of
a capture, and it carries the same obligations, with one that captures do
not need: **a fan-in GET is collected *over* a span, never *at* an
instant**, and every rendering of a snapshot states the span.

- **Line 1 is the header**: `{zsnap: 1, selectors, base, collected_at,
  collection_span_s, asked, answered}`, optionally `elided` (replies not
  kept past the observer's bound), `errors` and `superseded` (answers that
  lost last-writer-wins to a newer reply on the same key), and `roster`
  — how many origins the liveliness roster reported, or absent when it
  was not asked.
- **A row** carries `key`, `delete`, `bytes`, `encoding`, `timestamp`,
  `stamper` (O7's classification of the HLC), `source_zid` where a reply
  named its replier, `registration` (O2's rung — including "registry not
  loaded", which is not "unregistered"), `verdict` (the three-valued
  payload verdict, never a boolean), and `holder`.
- **`holder` is evidence, not inference.** `live {origin, answered_by}`
  means the key's origin held an `alive` token during the collection —
  and `answered_by` says whether the replier was the stamping entity,
  another party, or unknown; `storage_only {origin}` means a value
  answered and no token was held — a storage remembers it, nobody is
  saying it now; `unattributed {reason}` means the roster was not asked,
  or the key names no origin (O1). `live` says alive at collection, not
  that the value is fresh — freshness stays [04 §4](04-planes.md)'s.
- **A diff of two snapshots** MUST state both spans, MUST keep the three
  facets (value, verdict, registration, holder) apart, and — when it
  aligns origins across deployments — MUST list, never drop, an origin it
  could not pair.

What deliberately does not exist: a *replay* of a snapshot. A snapshot is
read, never published; seeding a fleet's state from a file is
`--seed-state` over a version-2 preamble, the same act under the same
guard as §4.2.

## 5. Synthetic traffic — the generator's etiquette (moved from 09 §5.3; added v1.19)

*Added in v1.19, alongside `zenctl gen` (zenkey #162). Replay got its
etiquette in §4 because replay is publishing; a **generator** is
publishing with one fewer excuse — the bytes never even happened. The
marker below is normative; the generator's own behavior (guards, plan
preview) is tool documentation.*

Synthetic traffic is any sample published to exercise or test a consumer
rather than to report a fact about the world: generated payloads, fault
injections, load patterns. The keyspace cannot distinguish it — that is
the point of generating conforming traffic — so the **attachment** must:

- A tool that publishes synthetic traffic MUST attach, to every synthetic
  sample, a JSON object attachment carrying at least
  `{"synthetic": true, "tool": "<name>", "origin": "<generating origin>"}`.
  A fault injector additionally carries `"fault": "<kind>"`
  (zenkey #163). Attachments on the data classes are otherwise free-form —
  the registry types media-frame attachments ([08 §2](08-registry.md)'s
  `[[media]]` `attachment` field) and declares nothing about data-class
  attachments; this marker is the one reserved shape among them.
- An observer that judges traffic (a doctor listen window, an `expect`
  verdict, a capture reader) SHOULD count marked samples separately and
  say so — generated traffic judged as real is a self-inflicted finding.
- A `.zrec` capture records the marker like any attachment (the row
  dialect already round-trips attachments verbatim, §4.1), so a **replay
  of synthetic traffic stays marked** with no extra rule.
- Deliberately **not** changed: replayed-but-originally-real traffic is
  not marked — replay provenance stays in the capture header and §4.2's
  re-stamping rules; inventing a marker for it would rewrite recorded
  bytes. The `spray` demo is likewise unmarked: it exists to be a
  self-contained adversarial bus, runs against no fleet but its own, and
  marking it would defeat the negative cases it stages.

## 6. Cutover acceptance (moved from 09 §6; added v1.2)

*Added in v1.2. Nothing in the convention said how you **prove** a
migration finished. [11](11-zensight-profile.md) scopes verification out
explicitly, so it belongs with the other judgment procedures: an
acceptance run is an observer whose verdict gates a decision, and §1's
shape reads on it directly — the reference verdict vocabulary is
`Pass | OldStillSpeaks | Unproven`, mapped in §1.1.*

A cutover to (or between majors of) this convention is **not done** until
an isolated run demonstrates **both** of the following. One without the
other is not evidence.

**1. The retired key family is silent.**

Stand up the deployment, subscribe to the *whole* old root, and assert an
empty result set while the new planes carry traffic. A migration you can
assert the *absence* of is a migration you can finish; one you cannot is a
migration you merely believe in. (§2 governs the silence half: everything
silent proves nothing — the old root's silence is evidence only beside a
new plane that is provably speaking. The three-valued verdict is forced,
not stylistic.)

Note what this costs you if the version chunk is plain (`v1`, not `@v1`):
`<base>/**` reaches the new keys too, so the subscriber sees your own
traffic and the leak check must state its meaning explicitly — *anything
outside `<base>/v1/`* — rather than riding on key algebra. Same guarantee,
stated rather than inferred. (A verbatim version chunk gives the algebraic
version for free, and costs you zenoh-ext's `@adv` sidecars, which is a bad
trade — [03-grammar.md §1.2](03-grammar.md).)

**2. A consumer-shaped, concrete-key probe passes.**

> **A probe MUST build its keys the way the product builds them.**

A fleet-selector probe (`<base>/v1/*/@rpc/…`) **cannot** catch a broken
origin path: the `*` matches *any* origin, so a caller whose origin concept
is complete garbage still gets replies. The reference implementation's
smoke was green — subscribing on wildcards, asserting replies arrived —
while **every drill-down in the product was broken**, because the product
used concrete origins and the probe did not
([06-identity.md §6.3](06-identity.md)).

> **A test that uses a wildcard where the product uses a concrete value is
> testing a different program.**

So the probe MUST resolve an origin through the same bridge the consumer
uses ([06-identity.md §6](06-identity.md)) and then issue **origin-scoped**
calls — and it MUST fail if the bridge yields nothing. Absence of replies
and absence of *callers* must not look alike.

Recommended shape (both halves, one run): multicast off / gossip on
([09 §0.1](09-operations.md)), an explicit endpoint, the full producer
set, one un-namespaced observer for the honest wire view
([09 §5](09-operations.md)), and a consumer-shaped phase that resolves
origins and drills down exactly as the UI does.
