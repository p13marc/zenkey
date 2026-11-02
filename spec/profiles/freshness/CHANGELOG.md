# `freshness` changelog

Versions of the texts of [`v1.md`](v1.md) and [`v2.md`](v2.md), each
entry naming its text. Each entry records what changed, what deliberately
did not, and why. A breaking change is a new major, a new file, never an
entry here ([`../README.md`](../README.md)).

## `v2.md` 0.6 — 2026-11-02: a GET reader ages a snapshot's entry (#977)

Core 0.97 (#977's E3) answers a state GET by snapshot, one collection per
reply, in place of a reply per key. A member of this text then carries
its confirmation in band, in its entry, where its reply carried the
26-byte Zenoh timestamp of its last confirmation. Core §4.3's table drops
its row. This text, written against core 0.97, follows.

**Changed: §2.4's "Its stamps".**
- **Before:** a GET reply carries the member's last confirmation as its
  Zenoh timestamp.
- **Now:** each live entry carries a confirmation. It is 1, vouched for
  at the snapshot's time, while the owner vouches for the member, as a
  heartbeat sent then would. Otherwise it is 2, with the member's last
  confirmation. Both times are minted as before, and kept to the
  millisecond from the deployment epoch.
- **Why one per entry.** A collection has no single confirmation once
  one member's bit is clear. So the snapshot's time is when it was
  built, and a value the owner stopped vouching for never reads fresh
  from a snapshot.

**Changed: §2.6.** A GET reader MUST age an entry from its confirmation
as v1 §2.6 ages a reply from its stamp: its trust rules, its band and its
order of checks. `[F: judgements.json]` `[Sc: §3]`
- **The time** is the snapshot's for confirmation 1, the entry's conf
  for 2. An unknown time is no stamp.
- **The clock** is the owner's, which built the snapshot.
- **Confirmation 0** on a member with a horizon breaks core S2: no stamp.
- **A tombstone** is a `reply_del`, deleted. A member a complete
  snapshot omits is deleted too, by core §4.2's omission rule. One an
  incomplete snapshot omits is silent.

**Changed: §2.5's re-read.** It is a collection read, the snapshot key
followed by `/**`. A member a complete snapshot left out leaves the view
by core §4.2's omission rule, and an incomplete one removes nothing. A
re-read costs a reply per page, no longer one per member.

**Changed: §2.10 and §5.** A snapshot's times come from the clock ahead,
as its replies' stamps did, and no router reads them. A tool shows a
snapshot's confirmation as its time.

**The evidence.**
- **`judgements.json`:** 88 cases, 27 new over a snapshot's entry: each
  confirmation, its band's edges, an unknown time, confirmation 0, a
  tombstone, a member a complete or an incomplete snapshot omits, ttl 0,
  and three combinations. The observation is `{"via": "snapshot",
  "complete", "entry", "clock"}`. Written by an independent reading of
  the text, which also gives the 61 earlier cases, checked, never
  blessed.
- **`scenarios-v2.md`:** §3 reads both confirmations, and a collection
  read with one of each. §2 step 4's reply is a snapshot's entry, and §5
  seeds by a collection read. §6's leftover leaves by the omission rule.

**Deliberately not changed.**
- **v1 §2.6** and every `freshness.v1` text. A state of a contract that
  uses `freshness.v1` keeps a reply per key, with its Zenoh timestamp
  (core §4.2's per-key resources).
- **The heartbeat**, its body, the subscriber's age (§2.5's rule) and the
  digest. An entry carries each member's run where it is not the
  header's, so a reader rebuilds its digest from a snapshot.
- **The trust grounds.** The deployment's word, or a measurement of the
  owner's clock, as v1 §2.6 states them.
- **No contract or fingerprint moves.**

## `v2.md` 0.5 — 2026-11-02: a candidate's held write clears its bit (#978)

zk2py's round 98 found §2.10 contradicting core §4.3 for a
`redundancy.v1` candidate (F-461). A candidate's session stamps every
write (core §8.2), so its guard holds every write of a member of this
text, while §2.10 said it holds none and the heartbeats go on vouching.
A held put of a new value left the owner holding a value it could no
longer make current, and the heartbeat turned it fresh. `zs.catalog.v1`
and `zs.desired.v1` use both profiles. This text, written against core
0.90, answers it with core 0.90's "Ahead".

**Changed: §2.10.**
- **Before:** "The puts and deletes of a member of this profile carry
  none, so the guard holds none of them", and the heartbeats go on
  vouching as at any other time.
- **Now:** that holds on a session without timestamping. A new bullet,
  "A held write": where the guard holds a member's write (a candidate's
  session stamps every write), the owner clears its bit until a write of
  it goes on. `[Sc: §2]` By §2.4's "Only a value it holds current", the
  value it still holds is one it can no longer vouch for.
- **§2.4's "Its stamps"** says the puts carry no Zenoh timestamp on a
  session without timestamping (core §8.2), and Appendix A's row names
  the member whose write the guard held.

**`scenarios-v2.md` §2, step 6.** A candidate, `lab/candidate`, serves
`beacon.v2` at minor 1, governed by `redundancy.v1`, its clock 2 s ahead.
Before any write is held, its heartbeats vouch for `status`, the time
unknown. Its held put of `still up` clears the bit, the digest still over
the first put. The guard's release alone sets nothing, and the next put
that goes through sets it again. The reference runs it as
`s2_a_candidates_held_put_clears_its_bit`
(`zenkey/tests/profile_freshness_v2.rs`, its contract
`zenkey/tests/contracts/beacon-governed.v2.toml`).

**Deliberately not changed.**
- **No keyword rule.** §2.4's MUST, to set a bit only while the owner
  vouches, binds the new bullet, which states none.
- **An owner whose session runs without timestamping,** as core §8.2
  asks of any but a candidate, sees no change: §2.10's time of all ones
  and its replies are 0.4's.
- **The body:** no fixture, contract or fingerprint moves.

## `v2.md` 0.4 — 2026-11-02: a clock ahead holds no confirmation (#978)

Core 0.88 (#978's F6) narrows its clock guard. An owner whose clock is
ahead of its router stops only the state writes that carry a Zenoh
timestamp. A member of this text carries none on its puts (§2.4), so the
guard holds none of its writes. This text, written against core 0.88,
follows.

**Changed: §2.10.**
- **Before:** while the guard holds, an owner MUST NOT vouch for any
  member, and its heartbeat's time is unknown. Every bit is clear, and
  the members go stale, by design.
- **Now:** its heartbeats go on vouching as at any other time, and it
  MUST write all ones as their time while the guard holds. `[Sc: §2]`
  A subscriber ages a member on its own clock, which the owner's does
  not move.
- **Its replies** carry a last confirmation from the clock ahead. A
  GET reader whose clock is right reads a fresh one as ahead of its
  clock beyond the delta: unobservable, by v1 §2.6's band.
- **When the guard releases,** the next heartbeat's time is known again,
  where the next heartbeat vouched again.
- **§2.4's time** says all ones are what a held guard writes, and
  Appendix A gains the row.

**`scenarios-v2.md` §2, step 4.** The owner `lab/ahead` puts `status`
again once its guard holds, and the put goes on. H2 hears each heartbeat
vouch for `status`, its time unknown, its digest over that put. S2 judges
`status` fresh 4 s on, past its horizon. G, on the deployment's word,
judges the reply unobservable, its stamp ahead of G's clock.

**Deliberately not changed.**
- **The body:** a time of all ones is §7.5's unknown, as before. No
  fixture, contract or fingerprint moves.
- **`freshness.v1` §2.10:** a v1 member is re-put with a Zenoh timestamp,
  which the guard holds, so it goes stale as before.
- **The reply's stamp** is not held: holding it would break core S2.

## `v1.md` experimental — 2026-11-02: no user left (#978)

The text's status moves from `draft` to `experimental`, the status of a
profile that no adopter's mapping and no shipped service uses
([`../README.md`](../README.md), "The process"). #978's F5 moves the
adopters' contracts onto `freshness.v2`: ZenSight's, zenoh-modem's
`modem.v3` and tcgui's, with the walkthrough's `nav.v2`, each a
review-class `uses_changed` revision published in `examples/zk2/.history`.
F4 had moved `health.v1`, `alarms.v1`, `config.v1` and zenwatch. No
contract of an adopter's mapping lists `freshness.v1` any longer, and
zenwatch does not follow it.

**Changed: the status alone.** The index lists `freshness.v1` in its
second table, "Experimental: no adopter yet", and the header's status
reads `experimental`.

**Deliberately not changed.**
- **No rule.** The text stays 0.5, written against core 0.22, and its
  fixtures, scenarios, runtime and tools are as they were.
- **It stays defined.** `freshness.v2` restates its rules by reference,
  and an owner still re-puts a contract that lists it.
- **The contracts that keep it,** each stated by a text that reads it so:
  its own scenarios' `beacon.v1`, the `status` of the core's state
  scenarios (`../../scenarios/state.md` §1), `link.v1`'s scenarios'
  `radio.v1`, whose §6 judges liveness by its re-puts, and `desired.v1`'s
  scenarios' `tasking.v1`. So do the conformance corpus's contracts,
  the core's lint cases and `link.v1`'s `sampler.v1`, whose horizons are
  on streams alone, confirmed by sampling under either major. zk2py's
  interop contracts move in its round.

## `v2.md` 0.3 — 2026-11-02: draft, used by the adopters and zenwatch (#978)

#978's F4 moves `health.v1`, `alarms.v1` and `config.v1` onto this text,
and zenwatch's own `zenwatch.v1` with them. The adopters' mappings use
those profiles, and zenwatch ships, so this text is no longer
experimental. Its status is `draft`, and the index lists it in its first
table.

**Deliberately not changed.** No rule, fixture or scenario. Written
against core 0.85, as 0.2 was.

## `v2.md` 0.2 — 2026-11-01: zk2py round 96's findings answered (#978)

zk2py's round 96 read text 0.1 cold and filed six findings on it and
the core's half, none a blocker. This version answers them, written
against core 0.85. Each reading taken is the one the reference and zk2py
already ran. Only the scenarios' contract changes, in both.

**Changed: §2.4, its stamps (F-445, F-446).**
- **Every member's replies** carry its last confirmation. For a member no
  heartbeat vouched for since its put, that is the put's.
- **The timestamps** are minted at each put of the member and at each
  heartbeat, as core 0.85's §4.3 "Minting" says. Only the replies carry
  them.
- **A heartbeat's time** is the instant of the timestamp minted as it is
  written, to the millisecond.

**Changed: §2.5, what counts (F-448).** Only a **read heartbeat**, one
read at step 4, counts.
- One stopped at step 1, 2 or 3 moves neither the sequence nor a count
  of divergences in a row. Step 3, the gap and the re-read say "read".
- `apply.json`'s case of another revision's tag and a refused body
  already read so. A new case shows it whole: a refused 6 between 5 and
  7 leaves a gap, and another revision's tag between two divergences
  leaves them in a row, so a read is due. It is written by hand from the
  text, and never blessed.

**Changed: §2.10, a guarded heartbeat (F-447).** "Every bit clear" is
every vouched bit. The map, the live counts and the digests stay as the
owner holds them, and the guard stops every write that would move them.
The reference already sent them so. Scenarios §2 step 4 now checks the
map and `status`'s digest.

**Changed: the scenarios' contract (F-443).** §7 read `link.exposure =
"link"` on `beacon.v2`'s `status` and `level`, which no section wrote.
- A contract that writes it lists `link.v1` in `uses` (core E020), so
  §7's was another revision.
- `beacon.v2` now carries both marks, and `link.v1` in `uses`. A face
  that exposes none ignores them, and only §7 runs a face.
- The reference's `zenkey/tests/contracts/beacon.v2.toml` follows, and
  its §7 test no longer marks a copy.

**Changed: scenarios §3 (F-446).** H listens too. G's reply's instant,
to the millisecond, is the time H read in the heartbeat that vouched for
`status`.

**Deliberately not changed.**
- **No rule's keyword.** §2.4's, §2.5's and §2.10's are 0.1's.
- **`beacon.v2`'s fingerprint moves.** It is a test contract, and no
  published history holds it.

## `v2.md` 0.1 — 2026-11-01: `freshness.v2`, confirmed by a heartbeat (#978)

Epic #975, zk2c, makes the wire bandwidth-first. `freshness.v1` confirms
a state member by putting it again, whole and freshly stamped, at most
every ttl/2. On W2 that is about 6.2 MB a day of unchanged documents,
config.v1's view and status most of it (#989). Core 0.81 defined a
heartbeat, §7.5's form 6, that vouches for each member by a bit. This
text, #978's F3, adopts it, as a new major: `freshness.v1` stays defined
for the contracts that use it. It is experimental: no adopter's mapping
uses it yet. F4 moves health.v1, alarms.v1 and config.v1 onto it.

**What it restates.** v1 §2.1–§2.3 and §2.5–§2.11 by reference, word for
word: the horizon, its kinds, the judgements, the combination, archives,
the face and "stale is never down". Only what counts as a confirmation
grows.

**What it replaces: v1 §2.4.**
- **The heartbeat.** An owner MUST publish one per interface it serves
  whose contract uses `freshness.v2` and has a collection, at most half
  the shortest horizon among its collections apart. `[Sc: §1]`
- **The bits.** It MUST set a member's bit only while it vouches for the
  value, and a member that is not live has none. `[Sc: §1, §2]`
- **No re-put.** It SHOULD NOT put a member again to confirm it.
  `[Sc: §1]`
- **Its stamps.** Its puts carry no Zenoh timestamp, and its replies the
  last confirmation (core §4.3's table and S2, 0.82). `[Sc: §3]`
- **When.** From the owner's tokens on, never from a standby, with no
  member live too.

**Added: §2.5, the reader's rule.** A subscriber MUST confirm a member by
a heartbeat only as the rule says: another revision's tag, a refused
body and an older seq confirm nothing; then, per collection, a map bit of
0 nothing, a digest or live count not the view's nothing, and otherwise
each member vouched for. A gap in the heartbeat key's seqs MUST be read
as a lost delivery until one arrives in sequence. After two divergences
in a row a subscriber SHOULD re-read the collection, and the answer MUST
replace its view. `[F: apply.json]` `[Sc: §4, §5, §6]`

**Added: §2.10, the clock guard.** While it holds, an owner MUST NOT
vouch for any member, and its heartbeat's time is unknown. `[Sc: §2]`

**Added: §3, the standard contract.** `freshness.v2.toml`, one stream,
`heartbeat/{iface}`: reliable, `drop`, priority `data`, cardinality 256.
Published in `../.history/freshness.v2/`, fingerprint `8756e6a6…`.

**Fixtures and scenarios.** `conformance/apply.json`, written by an
independent reading of §2.5 (Python, standard library), checked and never
blessed: 14 cases over the core's `heartbeat/layouts.json`.
`scenarios-v2.md` §1–§7, which the reference runs in process.

**The defaults #978's design took.** A new id, not `freshness.v1`
amended in place. One heartbeat per interface, not per owner. A digest per
collection, not one per exposure group. A cleared bit is no confirmation,
not stale at once. A re-read after two divergences, not one plus a grace.
The key a resource of the standard contract, not a verbatim kind.

## 0.5 — 2026-10-30: editorial: inline history taken out of the rules and long sentences split, no rule changed (#935)

The rules carried their own history, as the core's did before core 0.60
(#935, epic #921 Track B). A rule named the version that made it, as in
"(0.2)" or "(core S1, 0.21)". This changelog and the core's already hold
that history, so the text now states each rule and cites its evidence,
and nothing else. This is #935's third part, beside `health.v1`,
`hostid.v1`, `config.v1` and `link.v1`.

**Editorial: inline history taken out of the rules, no rule changed.**
Nine tags are gone, and none carried a reason that needed words:
- **This text's versions:** "(0.2)" in §2.3's "No value, no verdict",
  §2.5's "a member the reader knows", §2.6's measurement ("so (0.2):")
  and §2.7's "With no observation at all".
- **The core's versions beside a core citation:** "(core S1, 0.21)" in
  §2.4, "(core §4.1, 0.23)" in §2.2's stream row and "(core R7, 0.21)"
  in §2.9 now cite the core section alone.
- **"(core 0.21)" after "the interim row of core Appendix D"**, in §1's
  table and under §4's. The row is core Appendix D's, which says that a
  profile's own table replaces it.

**Nothing moved to an appendix.** No rule held a measurement.

**No sentence to split.** None was over 50 words.

**How "no rule changed" was checked**, as for core 0.60 and 0.61. Every
sentence holding MUST, MUST NOT, SHOULD, SHOULD NOT, MAY or RECOMMENDED
was extracted before and after, keyed by section, and the two sets
diffed. Both hold 16, and all 16 are unchanged: each tag sat in a
sentence with no keyword, or in a run-in label. No fixture moved, and no
scenario.

**The figures.** This text's tags went from 9 to none, and its mean from
16.45 words to 16.42. `spec/.prose-baseline.json` is lowered to them.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history: its "since 0.24" and "(core §4.2,
  0.26)" stay.
- **`scenarios.md`**, which held no tag and no sentence over 50 words.
- **Written against core 0.22**, still.

## 0.4 — 2026-10-27: two sentences that had fallen behind, non-normative (#908)

Writing `timing.v1` (#908, MA) found two sentences of this text that no
longer said what is true.
- **§2.2's stream row** gave as the reason a stream is judged by a
  subscriber only that "its samples carry no owner's stamp (core §4.1)".
  Core 0.23 says otherwise: a session whose HLC is enabled, as a serving
  session's is, stamps every put it is not given a stamp for, a stream
  sample included, and `timing.v1` §2.6 now requires an owner of a
  lifespan resource to stamp its samples.
- **§2.1's "why seconds"** said `timing.v1`'s `lifespan_ms` is judged on a
  receiver's clock. Its deadline is; its lifespan is judged against the
  owner's stamp, and only on a bound the deployment states (`timing.v1`
  §2.6).

**Changed: wording, in §2.1 and §2.2's table.**
- **§2.2:** a stream is judged by a subscriber only because it has no
  queryable, so no reply to age, and a subscriber ages a delivery by its
  arrival, never its stamp (§2.5), whichever clock set it.
- **§2.1:** sub-second staleness is `timing.v1`'s, its deadline on a
  receiver's clock and its lifespan on a stated bound.

**Deliberately not changed.**
- **No rule and no fixture.** Who judges a stream, and how, is as before:
  a subscriber, on its receive clock (§2.5). No case of `conformance/`
  moves.
- **Written against core 0.22**, still. No rule of this text reads what a
  later core changed; the two sentences name 0.23 and `timing.v1` for
  facts about them.

## 0.3 — 2026-10-18: an appendix corrected, non-normative (#614)

Two sentences of Appendix A, under "What is not ported", had fallen behind
the core. The migration note (`docs/zk2/migration.md`) and its follow-up
found them.
- It said that a population budget is a profile's (#613). Core 0.24 (#735)
  made it core: §2.7 bounds a templated resource's live members, a budget
  the owner keeps and a tool can judge.
- It said that zk2 has no seed. Core 0.26 (#614) gave a late joiner one:
  it subscribes, then makes S4's GET to the owner, and merges the two by
  the owner's stamp (§4.2).

**Changed: wording, in v1.md's Appendix A, which states no rule.**
- **"Aging out a population"** cites core §2.7 for the budget, and says
  that a horizon ages no member out. An owner still deletes when it
  decides (core S3), as before.
- **"Seeding by re-publishing"** keeps its point, that the owner is the
  only seed source and a re-put is a confirmation, not a seed. It now
  says how a late joiner seeds (core §4.2, 0.26), and that an archive's
  answer is last-known, never a seed (core S5, S6).

**Deliberately not changed.**
- **No rule and no fixture.** §2, §5 and every case of `conformance/` are
  as before.
- **Written against core 0.22**, still. No rule of this text reads
  anything a later core changed; the appendix names 0.24 and 0.26 for
  facts about the core.
- **`health.v1`** names the text 0.2 it was written against, and keeps it:
  nothing it uses moved.

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
