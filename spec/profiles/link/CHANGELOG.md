# `link.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.2 — 2026-10-18: what compiling it into live routers found (#769, LB)

The reference generator (`zenctl acl gen --face-policy`, LB) and the
spec-only implementation (zk2py, round 30) each compiled the profile into
zenoh 1.10.1 routers and ran the six scenario sections. zk2py passed every
fixture (119) and filed two findings, F-116 and F-117; the reference
confirmed F-117 in its own §2. Written against core 0.30, which corrects
§8.5 for F-117. No fixture case changed.

**Changed: readers subscribe to what they are granted (F-116;
scenarios.md Conventions, §5).**
- **The finding.** G and L subscribed to `zk2/lab/*/**`, and §5's G
  subscribed to and GOT `…/radio.v1/state/**`, under a `deny` block
  compiled from per-binding grants. zenoh refuses a subscription or a GET
  that no grant includes (core §11.3): zk2py measured a subscriber on
  `a/**` with allows on `a/b` and `a/c` receiving nothing, while a GET of
  `a/b` is answered. As written, the scenarios' readers received nothing.
- **The fix.** Each reader subscribes to, and GETs, the selectors its
  grants name, one per resource, and the Conventions say why. The
  enrollment is the reference's (`examples/zk2/link/radio.enrollment.toml`):
  one tool bound on `lab/*`, run by `ground`, `ground-router` and `local`.
- **The text** implied nothing wider: no rule of v1.md lets a reader
  subscribe beyond its grants, and §2.7's crossing is about an owner's
  pushes. It is unchanged.
- **No fixture:** a subscription's admission is a router's, which only a
  scenario runs.

**Changed: what the south region's declaration grant covers (F-117;
§2.5, scenarios.md §2).**
- **The finding.** §2.5, restating core §8.5, required the far principal
  to be granted `declare_queryable` on egress "for what its side may
  query". Granted on its bindings' selectors only, zk2py's G had its call
  answered and its state GET not; granted on the owners' `state/**` and
  `@state/**` as well, both; with no grant, neither. The reference
  measured the same in its live §2.
- **Why.** An owner's state queryables are declared over its interface's
  `state/**` and `@state/**` (core S2), and zenoh checks a declaration by
  inclusion against that key expression (core §11.3). A selector over one
  resource, `…/state/peers/*`, does not include `…/state/**`, so the
  declaration never reaches the far router, which then routes no GET.
- **The rule.** The grant covers every queryable the far side may query,
  **as its owner declares it**: the state queryables for a state GET, an
  operation's queryable over its template's pattern for a call (core §5:
  `@op/set/*` for a call of `set/duty`, the same inclusion), and an
  archive's over its `@state` forms. It is still a grant, not a deny: a
  declaration carries a key expression and no value, and what crosses in
  the replies is still the Consume and Call grants'.
- **Scenarios §2** say how the owner serves, and add the control F-117
  measured: the grant over G's own selectors answers the call and not the
  GET.
- **No fixture:** the grant depends on the enrollment's bindings and calls
  (core §11.1's input), which no fixture of this profile carries; the
  plan's shape is unchanged. The reference pins it in its generator's
  fixtures (`walkthrough.face-south-deny.json5`) and runs it in §2.

**Changed: the owner's traffic fits its 5 s (scenarios.md Conventions).**
0.1 had `oper` put 10 times "one a second apart" within 5 s, which
takes 9 s. It is one every half second, as the reference runs it.

**Changed: pointers.** §2.15 names `zenctl check face` beside `acl gen
--check`; Appendix B names the walkthrough's face policy and the radio
example (`examples/zk2/link/`).

**Deliberately not changed.**
- **The plan's shape** (§2.3) and every fixture: neither finding is the
  plan's. A plan member listing the declarations a south region needs was
  considered and left out: they follow from grants, not from the policy
  and the contracts, and the plan would carry an enrollment's facts.
- **The far principal's withheld grants.** The reference withholds the far
  principal's presence, history and admin-space grants, which the plan's
  `zk`, `adv` and `admin` denies would cut anyway (§2.4: deny is evaluated
  first within a policy). That is a generator's tidiness, with the same
  effect, and no rule here asks for it.
- **The core's constrained scenarios** (`constrained.md` §6) keep their
  words: a ground client's state GET "works" there with the declaration
  grant core 0.30 now spells out.

## 0.1 — 2026-10-18: the first text, draft (#769, LA)

`link.v1` is a profile of #613. Its record was zenoh-modem's sketch
(`examples/zk2/zenoh-modem/link.md`, #623), with the README's gaps 1, 10
to 14 and 20, architecture D14, D15, D9c and E3, spike S3, and core §8.5,
which names `link.v1` as the standard face configuration and needs only
two facts of it: that a face is constrained, and its maximum outage. The
maintainer decided on #769 that the face policy is a deployment file, that
the rate lives in it, that the contract keeps only `link.exposure`, that
exposure per principal is not in 0.1, that a crossing counter SHOULD carry
its epoch, and that core §8.5's "SHOULD … a third of the lease" wins over
architecture r4's "MUST cross well within". It is written against core
0.29, which publishes its vocabulary in place of Appendix D's interim row
and drops `link.downsample_ms`.

**Stated.**
- **The face policy** (§2.1): a TOML 1.0 file per router, its fields one
  by one (`system`, `services`, and per face `far`, `attach`, `region`,
  `select.link_protocols`, `class`, `expose`, `interval_s`,
  `max_outage_s`, `batch_bytes`, `link_bps`, `lease_ms`), and the findings
  a linter reports, each with a code, a severity, a face and a key.
- **`link.exposure`** (§2.2, §4): `host` or `link`, `host` when absent,
  any other value read as `host`; ignored on an operation.
- **The plan** (§2.3): the denies, the downsampling block, the
  attachment, the window floors and the liveness evidence of each face,
  in an order a second implementation reproduces.
- **Where the denies go** (§2.4): into every policy that can match a
  session on the face, since any allow wins across subjects; no subject
  of the face's own.
- **The attachment** (§2.5), router to router refused; **the `@zk` deny**,
  never omitted, beside `@adv` and the admin space (§2.6).
- **What crosses** (§2.7): the pushes of crossing resources; every other
  push denied, wholesale per kind token when nothing of it crosses, else
  per interface, carved into subtrees and names, template by template
  below a literal beside a parameter, with an overlap reported where
  zenoh cannot carve.
- **Downsampling** (§2.8): one rule per concrete key of a listed service,
  egress, `put`, 1/`interval_s` Hz; streams only; a templated exposed
  stream denied; every service whose exposed streams reach the face
  listed.
- **Events and requests** (§2.9): an exposed event whole, never
  downsampled; requests by grant, never by exposure.
- **The batch against the lease** (§2.10, a SHOULD), **the outage as every
  listed owner of state's window floor** (§2.11), **liveness from the
  plan's evidence** (§2.12), **counters' epochs** (§2.13, a SHOULD), what
  a deployment must not expect (§2.14), and the checks of a router's block
  (§2.15).
- **Evidence:** `conformance/exposure.json` (31 cases),
  `conformance/plan.json` (11 cases, two of them zenoh-modem's faces from
  its own contracts) and `conformance/lints.json` (77 cases), and six
  scenario sections for the generator and live routers (LB).

**How the fixtures' expectations were decided.** Each was written by hand
from this text before the model existed, and the model (`zenkey-model`'s
`link`) was then written to the text. It agreed with every expectation on
its first run.

**Decided here, where the record was silent.**
- **`far` in the policy.** The record's face file named no principal.
  §2.4 puts the denies in the far principal's policy, as the reference
  generator does for `--far`, so the policy names it.
- **No subject of the face's own** (§2.4). The sketch, and v1's RFC 09
  §4, selected the face by a link-protocol subject and repeated its denies
  in each operator's own. Under `allow`, such a subject's allows undo the
  far principal's denies (core §11.2, F-87), and core §11.3 already makes
  every session on a face a principal's.
- **Requests by grant, not exposure** (§2.9). The sketch denied every
  operation on the face and carved an operator's calls out of its own
  policy. Exposure is what crosses unasked; a request is asked once, and
  §11's Call and Consume grants say who may ask. This is also what
  `config.v1` §2.19 expects: its status GET crosses, its subscription not.
- **Only streams are downsampled** (§2.8). A downsampled state can leave
  the far side holding a superseded value as current, and a downsampled
  event loses occurrences no reader can tell from silence.
- **By interface, not by service** (§2.7). The sketch denied its
  companions' resources by service. Exposure is the contract's, for every
  service that implements it.
- **Wholesale per kind token** when nothing of that token crosses (§2.7),
  which fails closed for interfaces the policy does not know.
- **A rest parameter's pattern is `*/**`** (§0), so that a deny of
  `x/{p...}` leaves the key `x`, as the template does.
- **The window floor for every listed owner of state** (§2.11), not only
  for archived state as core S3 says: a far late joiner seeds by a GET
  after each outage, and a delete the window lost leaves its key standing.
- **The interval against the horizon** (§2.12): a crossing stream is fresh
  across the face only when `interval_s` ≤ ttl/2.

**Deliberately not stated.**
- **No exposure per principal** (the sketch's gap 20): deferred, as
  decided. Requests by grant cover the operator's calls and GETs.
- **No standard contract.** Nothing of the profile is on the bus.
- **No face subject, no `@op` deny, no `@media` or `@blob` deny**: §2.4,
  §2.9, and Appendix A.
- **No comparison of a block with the whole plan** in the model: it is a
  generator's check of its own output (`zenctl acl gen --check`, LB).
- **No counter type.** It is `telemetry.v1`'s (§2.13).
- **zenoh-modem's contract is unchanged.** Its four link streams declare
  no horizon, so the plan lists no liveness evidence for its RF face
  (Appendix B); its sketch is marked superseded and kept as the record.
