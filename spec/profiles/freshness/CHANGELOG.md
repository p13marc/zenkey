# `freshness.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.2 — 2026-10-10: three questions a cold read asked (#720)

The Python implementation read 0.1 cold (PR #729): every fixture case
passed on its first run, and the Rust and Python runtimes agreed on every
freshness result checked. It found three places where the text and the
fixture, or the text and itself, left room (F-100 to F-102).

**Changed: rules stated.**
- **What a subscription's silence judges (§2.3, §2.5; F-100).** 0.1's "no
  value, no verdict" read as contradicting §2.5's rules 5 and 6, under
  which a subscriber that heard nothing is stale past ttl. Both hold, for
  different members. §2.3 is about a member no observation showed a value
  for, which is unobservable. §2.5's silence rules judge a member another
  observation (a GET's value) made known, whose silence since is the
  evidence. The fixture already modelled the second; the text now says
  so. No expected value changes.
- **No observation at all (§2.7; F-101).** The order's horizon steps need
  no observation, so they answer even then: no horizon is not asked, not
  unobservable. The reference's `judge_all` returned `no_observation` for
  every horizon and now follows the order. `judgements.json` gains four
  cases.
- **How long a measured clock is trusted (§2.6 ground 2; F-102).** Over
  one reading, a tool's window or a subscription's lifetime, the offset
  closest to zero decides, and any stamp ahead by more than the delta
  withdraws the trust for the rest of the reading, since no transit causes
  that. A later offset above the delta does not, because slow transit
  cannot be told from a clock falling behind. Trust lasts the reading,
  and a long-running reader re-measures per reading or rests on the
  deployment's word. Both runtimes kept the smallest offset for the
  reading's lifetime, so neither saw a stamp running ahead. The reference
  now records both ends (`ClockMeasure`), its consumer and `check conform`
  use it, and the new fixture `clock-trust.json` pins the rule.

**Deliberately not changed.**
- **No time limit inside a reading.** A reading's span is the reader's,
  and the text bounds trust by it rather than by a number of seconds no
  deployment could tune.
- **No re-measuring rule for subscriptions.** A subscription is one
  reading. A daemon that wants fresher trust opens readings of its own.

## 0.1 — 2026-10-09: the first text, draft (#720, PC)

`freshness.v1` is the second profile of #613's first tier. The maintainer
decided on 2026-10-09 to write it before `health.v1`, which uses it, as do
about 20 contracts of `examples/zk2/`: every one on a plain `state`
resource, with horizons of 0, 5, 60, 300, 900, 7,200, 86,400 and
31,536,000 s. It is written against core 0.21, which publishes its
vocabulary in place of Appendix D's interim row, and says that a re-put is
a mutation (S1, S2).

**Stated.**
- **The value** (§2.1): a whole number of seconds from 0 to 2^53−1, an
  integral float being that integer; anything else is no horizon, and its
  resource's freshness unobservable.
- **The kinds** (§2.2): state and streams carry it; events and operations
  ignore it.
- **No horizon** is not asked, and **0** is never stale (§2.3), closing
  what D16 left open.
- **The owner's obligation** (§2.4): a state member confirmed at least
  every ttl/2, a change or a re-put of the value unchanged under a fresh
  owner's stamp; a stream sample at least every ttl/2, never repeated by a
  runtime.
- **How age is measured** (§2.5, §2.6): a subscriber on its own monotonic
  receive clock; a GET reader from the reply's stamp, only against a clock
  it trusts to the delta, on the deployment's word or a measurement, with a
  band of the delta either side of the horizon.
- **Several observations** of one member, and the order of the checks
  (§2.7); archives not asked (§2.8); liveness across a constrained face
  (§2.9, core R7); an owner whose clock is ahead going stale by design
  (§2.10); stale as a finding about a value, never "down" (§2.11).
- **What a tool may conclude** (§5), per member, per resource, and for a
  provider across a face; the changes against v1's RFC 04 §1.2
  (Appendix A).
- **Evidence:** `conformance/horizons.json` (30 cases),
  `conformance/judgements.json` (57 cases), and six scenario sections.

**Decided here, where the record was silent.**
- **No fraction.** The examples are integers, and the model kept the value
  untyped. A sub-second horizon is `timing.v1`'s, and a GET reader could
  not tell it apart from its clocks' disagreement.
- **Streams carry it.** D16 names state only. zenoh-modem's RF face carries
  streams and no state (its README's gap 14), and core R7 judges liveness
  from what crosses.
- **A GET reader's clock is trusted, or the age unobservable.** D16 gives
  no clock. Architecture r4's `timing.v1` row records a sender-stamp check
  failing a healthy commander at ±250 ms of skew (spike S11).
- **The band.** A trusted clock is trusted to the delta, so an age within
  the delta of the horizon is unobservable rather than decided either way.
- **Combining.** A reader with a subscription and a GET takes the most
  recent confirmation: fresh wins, then stale.

**Deliberately not stated.**
- **No lint code** for a value that is no horizon. The core has no code for
  an annotation's value, and the core's own fixtures use
  `freshness.ttl_s` as a carrier for E000, E020 and E028. A linter SHOULD
  warn (§2.1).
- **No meaning on a requirement.** A consumer's own horizon for a role
  would tighten the provider's, as `timing.v1`'s deadline may; nothing asks
  for it yet.
- **No "suspect" state,** and no expiry by the owner of a population past
  its horizon (v1 RFC 04 §1.2): Appendix A.
- **No liveness from freshness where presence crosses.** Presence answers
  that, and this profile answers whether a value is current.
- **No `health.v1` document.** `health.v1` uses this profile; how it
  reports a stale input is its own text (chunk PD).
