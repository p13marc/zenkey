# `link.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

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
