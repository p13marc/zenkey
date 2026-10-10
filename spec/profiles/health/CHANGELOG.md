# `health.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

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
