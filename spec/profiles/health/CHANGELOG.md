# `health.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.5 — 2026-10-30: editorial: inline history taken out of the rules and long sentences split, no rule changed (#935)

The rules carried their own history, as the core's did before core 0.60
(#935, epic #921 Track B). A rule named the version that made it, as in
"(0.2)". This changelog and the core's already hold that history, so the
text now states each rule and cites its evidence, and nothing else. This
is #935's third part, beside `freshness.v1`, `hostid.v1`, `config.v1`
and `link.v1`.

**Editorial: inline history taken out of the rules, no rule changed.**
- **This text's versions.** "(0.2)" is gone from three bullets of §2.5:
  the fault's stamp, "An owner that sets the stamp itself", and "A
  simulated offset is not a drift".
- **The core's version.** §2.3's "the core keeps its SHOULD (core 0.23)"
  is "the core keeps its SHOULD".
- **An issue.** Appendix B's zenwatch row said that zenwatch implements
  `zenwatch.v1` "since #614's FL3". It now says that zenwatch implements
  it.
- **The scenarios.** Their conventions' "T's clock (0.2)" and §4's
  "Without root (0.2)" are run-in labels alone. §5 to §7 cite "the
  conventions" without "0.2", and the opening paragraph names the
  reference's tests without "#721, PE" and "#721, PF".

**The long sentence, split.** §2.2's SHOULD, that a tool reports a
status better than a check from two readings a grace apart, ended in its
reason after a colon. The reason is a sentence of its own, "In one
reading, a reader that pairs …". The SHOULD keeps its words.

**Nothing moved to an appendix.** §2.5's measurement, a stamp 2 s ahead
re-stamped on zenoh 1.10.1, carried no tag and states the ground of its
bullet, which holds no keyword. This text has no appendix of
measurements, so it stays where it is.

**How "no rule changed" was checked**, as for core 0.60 and 0.61. Every
sentence holding MUST, MUST NOT, SHOULD, SHOULD NOT, MAY or RECOMMENDED
was extracted before and after, keyed by section, and the two sets
diffed. Both hold 27. 24 are unchanged, two lost a tag only (§2.3's and
§2.5's MAY), and §2.2's SHOULD lost its reason to a sentence of its own.
The scenarios' 44 such sentences and Expected lines are unchanged. No
fixture moved.

**The figures.** This text's tags went from 5 to none, its sentences
over 50 words from 1 to none, and its mean from 15.02 words to 14.95.
`scenarios.md`'s tags went from 4 to none, and its mean from 13.75 to
13.70. `spec/.prose-baseline.json` is lowered to them.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history.
- **Written against core 0.23**, still, and the header's "uses
  `freshness.v1`, text 0.2".

## 0.4 — 2026-10-10: zenwatch's row cites its v1 source and its zk2 home (#614)

#614's FL3 made zenwatch a zk2 service: `zenwatch/src/publish.rs` on
`main` is now its zk2 self-publication, implementing this profile beside
an interface of its own. Appendix B's zenwatch row cited "its
`publish.rs`" for the v1 health document it maps.

**Changed: wording, in v1.md.**
- **Appendix B's zenwatch row** cites its v1 `zenwatch/src/publish.rs` on
  the repository's `v1` branch, and names the interface of its own the
  row foresaw: `zenwatch.v1` (`zenwatch/contracts/zenwatch.v1.toml`).

**Deliberately not changed.**
- **No rule.** The mapping is the one 0.1 wrote: the status, beside an
  interface of its own for its rules' state.
- **Written against core 0.23**, still: a citation reads nothing new
  from the core.

## 0.3 — 2026-10-10: §5's clock question has its fixture (#721, PF)

The reference tool (`zenctl health`, #721's PF) answers §5's last
question, "is this service's clock ahead?", and found it pinned by the
text and checked by no fixture: the runtime's §4 test computed it by hand.

**Changed.**
- **`conformance/clock.json`** (21 cases), cited from §5's row: what one
  subscriber heard of one service in its window, in arrival order (the
  status's puts and deletes, `clock_ahead` faults, other faults), and its
  presence as `judgements.json` writes it → yes, no, unobservable or not
  asked, with a reason. `zenkey_model::health::clock_ahead` runs it.

**Deliberately not changed.**
- **The rule.** Every case is §5's row as 0.2 states it. Where the row is
  silent the fixture reads it so, and says so in its notes:
  - a status delete is no confirmation (§2.3), so it neither answers no nor
    clears a fault;
  - a fault's level and stamp play no part, only its code (§2.5: nothing a
    reader concludes rests on the stamp);
  - "not asked, as the first row's" is absent and not listed. A service
    whose descriptor was not read, or whose token a read possibly
    incomplete missed, is still answered by a fault heard on its key: the
    fault is its own word.
- **§5's second question** ("does its status agree with its checks?")
  stays without a fixture of its own: its answer is the first question's
  reason for one reading (`inconsistent` is the no), and `judgements.json`
  pins that reason.

## 0.2 — 2026-10-10: a clock ahead without root, and the tool's clock (#721, PE)

Two findings of the Python implementation's cold read of 0.1 (#609),
resolved with the runtime that implements the text (`zenkey::health`, PE)
and measured where zenoh decides.

**Changed.**
- **F-103: what `scenarios.md` §4 checks without root.** Its expected 2
  wanted the `clock_ahead` fault re-stamped by R1, which needs the fault to
  reach R1 dated ahead. A runner without root cannot move a host's clock,
  and zenoh lets no program move its session's HLC, so the offset is
  simulated in the owner's runtime. Measured on zenoh 1.10.1, from Rust
  (`zenkey/tests/profile_health.rs`) and from Python: a session whose HLC
  runs re-stamps a put given a stamp 2 s ahead of that HLC before the put
  leaves it, so the fault reaches R1 under the owner's honest stamp, as an
  unstamped one does.
  - §2.5 now states that fact: a simulated offset is not a drift. It says
    that nothing a reader concludes rests on the fault's stamp, and that
    an owner MAY set the stamp itself from the clock it mints state stamps
    with, which the reference runtime does.
  - §4 marks the stamp check **[moved clock]**, with a new step 4 under
    `timestamping.drop_future_timestamp`: the fault and the status put
    while ahead are dropped, and the owner's GET still answers. Every
    other expectation of §4 holds without root.
  - A runner meets the [moved clock] tier by moving the owner host's
    clock, or with an owner that stamps from its offset clock on a session
    without an HLC. The reference does the second: in-process client
    owners, whose HLC is off by default, and its owner example under
    `--clock-offset-ms`.
- **F-104: the tool's clock in `scenarios.md` §5 to §7.** T reads by GET
  alone, and `freshness.v1` §2.6 ages a reply only with a trusted clock,
  so read literally every status there was unobservable,
  `clock_untrusted`. T now takes the deployment's word (`freshness.v1`
  §2.6, ground 1), stated in the conventions and in each setup. Every
  session of a scenario runs on one host, so the word holds.
- **The conventions' waits:** §4 waits about 100 s (65 s, then 35 s), not
  70 s.
- **Uses `freshness.v1` text 0.2.** §2.8 already cited its §2.5, "a
  member it knows", and F-104 reads its §2.6.

**Deliberately not changed.**
- **The rule.** An owner whose guard holds still publishes `clock_ahead`
  at `FAILED`, at once, then once per status interval, and none after
  release. Only what a runner can observe of the stamp moved.
- **Measuring T's clock** was the other way. T would subscribe to the
  statuses before the owners start, and measure from their first puts.
  It was not chosen: it ties §5–§7 to a subscription and a start order,
  and a tool reaching a running deployment would hear a re-put only every
  30 s. What those sections check is health's reading. The grounds of a
  clock's trust are `freshness.v1`'s, which its scenarios §4 and §6 check.
  G in §1 to §4 still measures from S's deliveries.
- **Core 0.23 §4.1 and Appendix B** are not amended here. The session's
  re-stamp is a zenoh fact, recorded in §2.5 where this text relies on it.
  The core may list it among its §4.1 facts at its next version.

## 0.1 — 2026-10-10: the first text, draft (#721, PD)

`health.v1` is the third profile of #613's first tier, after `hostid.v1`
and `freshness.v1`, which it uses. The maintainer decided on 2026-10-09
that it comes before zengui and zenwatch move onto zk2 (FL, #614). Its
standard contract was written with the walkthrough, published as revision
1.0 (`sha256:e9dbcdcb…`), and implemented by name in the tcgui, ZenSight
and zenoh-modem mappings. This text gives it rules, and is written against
core 0.23, which names the `faults` stream where §4.3 said to report a
clock ahead through `health.v1`.

**Stated.**
- **The contract moves** under the profile (§3), unchanged:
  `spec/profiles/health/health.v1.toml` and its proto, the same
  fingerprint before and after, revision 1.0 copied byte for byte into
  `spec/profiles/.history/health.v1/`. Doc strings and proto comments were
  added, and are outside the fingerprint.
- **The levels** (§2.1): `OK`, `DEGRADED`, `FAILED`, ordered; absent is
  presence's word, never `FAILED`.
- **Aggregation** (§2.2): the status never better than its worst current
  check, with the puts ordered so that no intermediate breaks it; no
  roll-up across services; a tool's roll-up is the worst established level,
  with stale, unobservable and not-asked services counted apart.
- **Who puts what, and when** (§2.3): the status before the tokens, a MUST
  where core §8.2 says SHOULD; re-put by `freshness.v1`; never deleted
  while the service runs. Checks put on change and deleted when retired.
  Faults are occurrences.
- **Freshness** (§2.4) by `freshness.v1`, and **a fresh status vouches for
  the owner's checks**, which carry no freshness of their own.
- **A clock ahead** (§2.5) is a `clock_ahead` fault at `FAILED`, and the
  status goes stale by design.
- **Unknown levels** (§2.6), **the tokenless set** (§2.7), **constrained
  faces** (§2.8), **device-as-service** (§2.9), **fault codes** (§2.10),
  and **the reader's procedure** (§2.11).
- **What a tool may conclude** (§5): healthy, unhealthy, stale,
  unobservable, not asked; the status against its checks; a roll-up; a
  clock ahead. The changes against v1's health document (Appendix A), and
  the adopters' mappings (Appendix B).
- **Evidence:** `conformance/judgements.json` (61 cases),
  `conformance/rollups.json` (8), `conformance/codes.json` (22), and eight
  scenario sections, five the runtime's and three a tool's.

**Decided here, where the record was silent.**
- **One heartbeat.** The contract gives `checks` no horizon. Rather than
  give each check a freshness of its own, a fresh status vouches for every
  check the owner holds, and the owner deletes what it retires and leaves no
  check it cannot run. 64 checks re-put every 30 s would cost 64 times the
  status's one put.
- **The order of the puts.** v1 said a writer orders a transition across
  keys so that every intermediate is safe (RFC 04 §1.2); here, the status
  first when a check gets worse, the check first when it gets better.
- **A *no* can rest on a check, a *yes* only on the status.** A status at
  an unknown level, with a current check `FAILED`, reads unhealthy: the
  check is vouched for by the fresh status. Checks all `OK` never make an
  unknown status healthy, since a service can fail in ways no check names.
- **A status better than a check reads at the check's level,** reason
  `inconsistent`, rather than unobservable: the evidence for *no* is there.
- **The fault for a clock ahead.** `docs/zk2/architecture.md`'s S7 row and
  core §4.3 said to report through `health.v1`, but the status is state,
  which an owner ahead stops writing. A `faults` sample is a stream sample,
  which the guard does not hold.
- **Fault codes are a vocabulary of the form `[a-z][a-z0-9_]*`,** the
  profile's table first, an application's codes beside it, prefixed by
  recommendation.
- **A deleted status reads unobservable,** reason `deleted`: the owner
  broke §2.3, and nothing answers whether it is healthy.
- **Silence ages a status only when one is known to exist.** A present,
  listed owner holds one (§2.3), so a subscriber's silence makes it stale.
  Across a constrained face nothing says so until a delivery crosses
  (`freshness.v1` §2.5, "a member it knows"), so a status of which nothing
  has crossed is unobservable, `nothing_crossed`, which is what core R7
  says of a face where nothing crosses.
- **zenoh-modem's RF face lets nothing of `health.v1` cross**: its
  `link.md` denies the device's state and `health.v1`'s streams. The text
  records it as a gap of that mapping (Appendix B) and leaves `link.md` as
  it is.

**Deliberately not stated.**
- **No application field on `Status`** (the maintainer's decision for
  #721): ZenSight's `HealthSnapshot` and zenwatch's counters stay in their
  own interfaces.
- **No horizon on checks or faults.** The contract is published, and
  adding one would be a new revision; the vouching rule makes one
  unnecessary.
- **No resolution of a fault.** A fault is an occurrence; a confirmed
  status is the evidence that a drift ended.
- **No `health.v1` in the descriptor's `profiles`.** Core §3.3 defines the
  list as the contracts' `uses` and the derivation-only profiles, and
  `health.v1` is neither: its `interfaces` carry it.
- **No runtime and no tool here.** `zenkey::health` and the owner example
  are #721's PE, the fleet's checks and `zenctl health` its PF. They run
  `scenarios.md` §1–§5 and §6–§8.
